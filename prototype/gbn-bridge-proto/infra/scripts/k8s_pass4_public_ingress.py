#!/usr/bin/env python3
"""Pass 4 local-k8s public ingress artifact and validation helper.

The shell wrappers are the operator entrypoints. This module keeps the JSON and
endpoint validation logic in one place so prepare/verify/down agree on the same
contract.
"""

from __future__ import annotations

import argparse
import base64
import datetime as dt
import hashlib
import ipaddress
import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.request
import zlib
from pathlib import Path
from typing import Any


ALLOWED_ROLES = {
    "publisher_authority",
    "publisher_receiver",
    "host_creator_bootstrap",
    "exit_bridge",
}
ADMIN_PORTS = {9090, 9100, 10250, 10255, 6443, 2379, 2380, 5432}
FORBIDDEN_SEED_KEYS = {
    "admin_url",
    "admin_urls",
    "publisher_dht",
    "publisher_entry",
    "publisher_public_key",
    "publisher_trust_root",
    "bridge_dht",
    "bridge_catalog",
    "exit_bridge_dht",
    "exit_bridges",
    "private_key",
    "secret_key",
    "aws_access_key_id",
    "aws_secret_access_key",
    "aws_session_token",
}


class ValidationError(RuntimeError):
    pass


def now_ms() -> int:
    return int(time.time() * 1000)


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()


def stable_json(data: Any) -> str:
    return json.dumps(data, indent=2, sort_keys=True)


def compact_json(data: Any) -> str:
    return json.dumps(data, separators=(",", ":"), sort_keys=True)


def write_json(path: Path, data: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(stable_json(data) + "\n", encoding="utf-8")


def read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        raise ValidationError(f"{path} is not valid JSON: {error}") from error


def sha256_hex_bytes(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def sha256_hex_text(payload: str) -> str:
    return sha256_hex_bytes(payload.encode("utf-8"))


def ensure_wsl(force: bool = False) -> None:
    if force:
        return
    kernel = os.uname().release.lower() if hasattr(os, "uname") else ""
    if "microsoft" not in kernel:
        raise ValidationError("Pass 4 tooling requires WSL2 Ubuntu")


def is_forbidden_public_host(host: str) -> str | None:
    host = host.strip().lower().rstrip(".")
    if not host:
        return "public_host is required"
    if host in {"localhost", "localhost.localdomain"}:
        return "localhost is not a public mobile-reachable host"
    if host.endswith(".local") or host.endswith(".internal"):
        return "local/internal hostnames are not public mobile-reachable hosts"
    if ".svc" in host or host.endswith(".cluster.local"):
        return "Kubernetes service DNS names are not public mobile-reachable hosts"
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        return None
    if not ip.is_global:
        return f"{host} is not a globally routable public address"
    return None


def endpoint_ports(endpoint: dict[str, Any]) -> list[tuple[str, int]]:
    ports: list[tuple[str, int]] = []
    for key in ("tcp_port", "udp_port"):
        value = endpoint.get(key)
        if value is not None:
            try:
                ports.append((key, int(value)))
            except (TypeError, ValueError):
                ports.append((key, -1))
    return ports


def endpoint_url(endpoint: dict[str, Any]) -> str:
    protocol = endpoint.get("protocol")
    host = endpoint.get("public_host")
    tcp_port = endpoint.get("tcp_port")
    udp_port = endpoint.get("udp_port")
    if protocol in {"http", "https", "ws", "wss", "tcp", "tls"} and tcp_port is not None:
        return f"{protocol}://{host}:{tcp_port}"
    if udp_port is not None:
        return f"udp://{host}:{udp_port}"
    return f"{protocol or 'unknown'}://{host or 'missing'}"


def validate_endpoint(endpoint: dict[str, Any], run_profile: str, current_ms: int) -> list[str]:
    errors: list[str] = []
    endpoint_id = str(endpoint.get("endpoint_id", "")).strip()
    actor_id = str(endpoint.get("actor_id", "")).strip()
    role = str(endpoint.get("role", "")).strip()
    host = str(endpoint.get("public_host", "")).strip()

    if not endpoint_id:
        errors.append("endpoint_id is required")
    if not actor_id:
        errors.append(f"{endpoint_id or '<missing>'}: actor_id is required")
    if role not in ALLOWED_ROLES:
        errors.append(f"{endpoint_id or '<missing>'}: role must be one of {sorted(ALLOWED_ROLES)}")
    if endpoint.get("profile") != run_profile:
        errors.append(f"{endpoint_id or '<missing>'}: profile must be {run_profile}")

    forbidden_reason = is_forbidden_public_host(host)
    if forbidden_reason:
        errors.append(f"{endpoint_id or '<missing>'}: {forbidden_reason}")

    ports = endpoint_ports(endpoint)
    if not ports:
        errors.append(f"{endpoint_id or '<missing>'}: tcp_port or udp_port is required")
    for port_name, port in ports:
        if port <= 0 or port > 65535:
            errors.append(f"{endpoint_id or '<missing>'}: {port_name} must be 1..65535")
        if port in ADMIN_PORTS:
            errors.append(f"{endpoint_id or '<missing>'}: {port_name} {port} is an admin/private port")

    if role in {"publisher_authority", "publisher_receiver", "host_creator_bootstrap"}:
        if endpoint.get("tcp_port") is None:
            errors.append(f"{endpoint_id or '<missing>'}: role {role} requires tcp_port")
        if not (endpoint.get("tls_sni") or endpoint.get("certificate_fingerprint")):
            errors.append(
                f"{endpoint_id or '<missing>'}: TLS SNI or certificate fingerprint is required"
            )

    if role == "exit_bridge" and endpoint.get("udp_port") is None:
        errors.append(f"{endpoint_id or '<missing>'}: exit_bridge requires udp_port")

    expires_at_ms = endpoint.get("expires_at_ms")
    try:
        expires_at_ms_int = int(expires_at_ms)
    except (TypeError, ValueError):
        errors.append(f"{endpoint_id or '<missing>'}: expires_at_ms is required")
    else:
        if expires_at_ms_int <= current_ms:
            errors.append(f"{endpoint_id or '<missing>'}: descriptor is expired")

    chain_id = str(endpoint.get("chain_id", "")).strip()
    if not chain_id:
        errors.append(f"{endpoint_id or '<missing>'}: chain_id is required")
    if any(part in endpoint_url(endpoint).lower() for part in ("/v1/admin", "admin.")):
        errors.append(f"{endpoint_id or '<missing>'}: admin URL/path must not be public")

    return errors


def validate_endpoint_map(endpoint_map: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    profile = str(endpoint_map.get("profile", "")).strip()
    current_ms = now_ms()
    endpoints = endpoint_map.get("endpoints")
    if not isinstance(endpoints, list) or not endpoints:
        return ["endpoint map must contain at least one endpoint"]
    seen: set[str] = set()
    required_roles = {"publisher_authority", "publisher_receiver", "host_creator_bootstrap", "exit_bridge"}
    seen_roles: set[str] = set()
    for endpoint in endpoints:
        if not isinstance(endpoint, dict):
            errors.append("endpoint entries must be objects")
            continue
        endpoint_id = str(endpoint.get("endpoint_id", "")).strip()
        if endpoint_id in seen:
            errors.append(f"{endpoint_id}: endpoint_id is duplicated")
        seen.add(endpoint_id)
        seen_roles.add(str(endpoint.get("role", "")).strip())
        errors.extend(validate_endpoint(endpoint, profile, current_ms))
    missing_roles = sorted(required_roles - seen_roles)
    if missing_roles:
        errors.append(f"endpoint map is missing required roles: {', '.join(missing_roles)}")
    return errors


def run_command(command: list[str], timeout_seconds: int = 30) -> tuple[int, str]:
    try:
        completed = subprocess.run(
            command,
            check=False,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=timeout_seconds,
        )
        return completed.returncode, completed.stdout.strip()
    except FileNotFoundError:
        return 127, f"{command[0]} not found"
    except subprocess.TimeoutExpired as error:
        output = (error.stdout or "") if isinstance(error.stdout, str) else ""
        return 124, f"timed out after {timeout_seconds}s\n{output}".strip()


def check_k8s(namespace: str) -> dict[str, Any]:
    checks: list[dict[str, Any]] = []
    for kind_name in (
        "deployment/publisher-authority",
        "deployment/publisher-receiver",
        "deployment/creator-host",
        "statefulset/exit-bridge",
    ):
        code, output = run_command(["kubectl", "-n", namespace, "rollout", "status", kind_name, "--timeout=5s"])
        checks.append(
            {
                "target": kind_name,
                "status": "pass" if code == 0 else "fail",
                "exit_code": code,
                "output": output,
            }
        )
    return {
        "namespace": namespace,
        "status": "pass" if all(check["status"] == "pass" for check in checks) else "fail",
        "checks": checks,
    }


def resolve_host(host: str) -> tuple[str, list[str]]:
    records = sorted({item[4][0] for item in socket.getaddrinfo(host, None)})
    return "pass", records


def tcp_check(host: str, port: int, timeout_seconds: float = 5.0) -> tuple[str, str]:
    try:
        with socket.create_connection((host, port), timeout=timeout_seconds):
            return "pass", "tcp connection established"
    except OSError as error:
        return "fail", str(error)


def udp_check(host: str, port: int, timeout_seconds: float = 2.0) -> tuple[str, str]:
    try:
        addrinfo = socket.getaddrinfo(host, port, socket.AF_UNSPEC, socket.SOCK_DGRAM)[0]
        with socket.socket(addrinfo[0], socket.SOCK_DGRAM) as sock:
            sock.settimeout(timeout_seconds)
            sock.sendto(b"veritas-pass4-public-ingress-probe", addrinfo[4])
        return "sent", "udp probe datagram sent; protocol-specific ACK is validated in Phase 5"
    except OSError as error:
        return "fail", str(error)


def reachability_checks(endpoints: list[dict[str, Any]], skip: bool) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    for endpoint in endpoints:
        host = str(endpoint["public_host"])
        endpoint_result: dict[str, Any] = {
            "endpoint_id": endpoint["endpoint_id"],
            "role": endpoint["role"],
            "url": endpoint_url(endpoint),
        }
        if skip:
            endpoint_result["status"] = "skipped"
            endpoint_result["reason"] = "network checks skipped by operator flag"
            results.append(endpoint_result)
            continue
        try:
            dns_status, records = resolve_host(host)
            endpoint_result["dns"] = {"status": dns_status, "records": records}
        except OSError as error:
            endpoint_result["dns"] = {"status": "fail", "error": str(error)}
        if endpoint.get("tcp_port") is not None:
            status, message = tcp_check(host, int(endpoint["tcp_port"]))
            endpoint_result["tcp"] = {"status": status, "message": message}
        if endpoint.get("udp_port") is not None:
            status, message = udp_check(host, int(endpoint["udp_port"]))
            endpoint_result["udp"] = {"status": status, "message": message}
        endpoint_result["status"] = "pass" if endpoint_result.get("tcp", {}).get("status") in {None, "pass"} else "fail"
        if endpoint.get("udp_port") is not None and endpoint_result.get("udp", {}).get("status") == "fail":
            endpoint_result["status"] = "fail"
        if endpoint_result.get("dns", {}).get("status") == "fail":
            endpoint_result["status"] = "fail"
        results.append(endpoint_result)
    return results


def admin_denial_checks(admin_checks: list[dict[str, Any]], skip: bool) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    for check in admin_checks:
        url = str(check.get("url", "")).strip()
        item: dict[str, Any] = {"name": check.get("name", "admin-check"), "url": url}
        if skip:
            item.update({"status": "skipped", "reason": "network checks skipped by operator flag"})
            results.append(item)
            continue
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "veritas-pass4-ingress-verify"})
            with urllib.request.urlopen(req, timeout=5) as response:
                body = response.read(256)
                item.update(
                    {
                        "status": "fail",
                        "http_status": response.status,
                        "message": f"admin endpoint returned HTTP {response.status}",
                        "sample_sha256": sha256_hex_bytes(body),
                    }
                )
        except urllib.error.HTTPError as error:
            # Any HTTP response means the admin surface is publicly reachable. Even 401/403
            # must be treated as a Phase 4 failure; admin has to stay off public ingress.
            item.update(
                {
                    "status": "fail",
                    "http_status": error.code,
                    "message": f"admin endpoint returned HTTP {error.code}",
                }
            )
        except (urllib.error.URLError, TimeoutError, OSError) as error:
            item.update({"status": "pass", "message": f"not reachable: {error}"})
        results.append(item)
    return results


def find_endpoint(endpoints: list[dict[str, Any]], role: str) -> dict[str, Any]:
    matches = [endpoint for endpoint in endpoints if endpoint.get("role") == role]
    if not matches:
        raise ValidationError(f"run profile is missing endpoint role {role}")
    return matches[0]


def public_key_bytes(hex_value: str) -> list[int]:
    normalized = hex_value.strip().lower()
    if normalized.startswith("0x"):
        normalized = normalized[2:]
    if len(normalized) != 64:
        raise ValidationError("host_creator.public_key_hex must be 32 bytes encoded as 64 hex chars")
    try:
        raw = bytes.fromhex(normalized)
    except ValueError as error:
        raise ValidationError("host_creator.public_key_hex must be valid hex") from error
    return list(raw)


def placeholder_signature(seed: str) -> list[int]:
    digest = hashlib.sha512(seed.encode("utf-8")).digest()
    return list(digest[:64])


def build_host_creator_seed(
    config: dict[str, Any],
    endpoints: list[dict[str, Any]],
    run_id: str,
    chain_id: str,
) -> dict[str, Any]:
    host_config = config.get("host_creator")
    if not isinstance(host_config, dict):
        raise ValidationError("host_creator object is required")
    host_endpoint = find_endpoint(endpoints, "host_creator_bootstrap")
    actor_id = str(host_config.get("actor_id") or host_endpoint.get("actor_id") or "creator-host")
    key_bytes = public_key_bytes(str(host_config.get("public_key_hex", "")))
    udp_port = int(host_endpoint.get("udp_port") or host_endpoint.get("tcp_port") or 443)
    expires_at_ms = int(host_endpoint["expires_at_ms"])
    endpoint_metadata = {
        "endpoint_id": host_endpoint["endpoint_id"],
        "protocol": host_endpoint.get("protocol", "https"),
        "public_host": host_endpoint["public_host"],
        "tcp_port": host_endpoint.get("tcp_port"),
        "udp_port": host_endpoint.get("udp_port"),
        "tls_sni": host_endpoint.get("tls_sni"),
        "certificate_fingerprint": host_endpoint.get("certificate_fingerprint"),
        "reachability_class": host_endpoint.get("reachability_class", "direct"),
    }
    unsigned_entry = {
        "node_id": actor_id,
        "ip_addr": host_endpoint["public_host"],
        "pub_key": key_bytes,
        "udp_punch_port": udp_port,
        "entry_expiry_ms": expires_at_ms,
    }
    seed = {
        "schema": "veritas.pass4.host_creator_dht_seed.v1",
        "run_id": run_id,
        "chain_id": chain_id,
        "issued_at_ms": now_ms(),
        "expires_at_ms": expires_at_ms,
        "host_creator_public_key": key_bytes,
        "host_creator_public_key_fingerprint": sha256_hex_bytes(bytes(key_bytes))[:32],
        "host_creator_entry": {
            **unsigned_entry,
            "publisher_sig": placeholder_signature(compact_json(unsigned_entry)),
            "active": True,
        },
        "host_creator_bootstrap_endpoint": endpoint_metadata,
        "payload_hash": "",
        "signature_metadata": {
            "algorithm": "ed25519",
            "signature_source": "publisher_or_hostcreator_required_for_live_run",
            "signature_present": False,
        },
    }
    payload_without_hash = compact_json({**seed, "payload_hash": ""})
    seed["payload_hash"] = "sha256:" + sha256_hex_text(payload_without_hash)
    return seed


def validate_host_seed(seed: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if seed.get("schema") != "veritas.pass4.host_creator_dht_seed.v1":
        errors.append("HostCreator seed schema is invalid")
    entry = seed.get("host_creator_entry")
    endpoint = seed.get("host_creator_bootstrap_endpoint")
    if not isinstance(entry, dict):
        errors.append("HostCreator seed is missing host_creator_entry")
    if not isinstance(endpoint, dict):
        errors.append("HostCreator seed is missing host_creator_bootstrap_endpoint")
    forbidden = sorted(FORBIDDEN_SEED_KEYS & set(seed.keys()))
    if forbidden:
        errors.append(f"HostCreator seed contains forbidden top-level fields: {', '.join(forbidden)}")
    serialized = compact_json(seed).lower()
    forbidden_fragments = [
        "publisher-authority",
        "publisher-receiver",
        "exit-bridge",
        "/v1/admin",
        "cluster.local",
        ".svc",
        "localhost",
        "127.0.0.1",
    ]
    for fragment in forbidden_fragments:
        if fragment in serialized:
            errors.append(f"HostCreator seed contains forbidden fragment: {fragment}")
    return errors


def render_placeholder_png(path: Path, payload: str) -> None:
    # Deterministic QR-like artifact for environments without qrencode. The
    # payload text next to it remains the scannable source for real QR generation.
    size = 256
    cell = 8
    digest = hashlib.sha256(payload.encode("utf-8")).digest()
    rows = bytearray()
    for y in range(size):
        rows.append(0)
        for x in range(size):
            cell_x = x // cell
            cell_y = y // cell
            idx = (cell_x + cell_y * (size // cell)) % len(digest)
            bit = (digest[idx] >> ((cell_x + cell_y) % 8)) & 1
            pixel = 0 if bit else 255
            if x < 24 and y < 24:
                pixel = 0
            elif x < 20 and y < 20:
                pixel = 255
            rows.extend([pixel, pixel, pixel])
    compressor = zlib.compressobj()
    data = compressor.compress(bytes(rows)) + compressor.flush()

    def chunk(kind: bytes, chunk_data: bytes) -> bytes:
        return (
            struct.pack(">I", len(chunk_data))
            + kind
            + chunk_data
            + struct.pack(">I", zlib.crc32(kind + chunk_data) & 0xFFFFFFFF)
        )

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", data)
    png += chunk(b"IEND", b"")
    path.write_bytes(png)


def render_qr_artifacts(artifact_dir: Path, payload: str) -> dict[str, Any]:
    payload_path = artifact_dir / "hostcreator_bootstrap_qr_payload.txt"
    payload_path.write_text(payload + "\n", encoding="utf-8")
    encoded_payload = base64.urlsafe_b64encode(payload.encode("utf-8")).decode("ascii")

    svg_path = artifact_dir / "hostcreator_bootstrap_qr.svg"
    svg_path.write_text(
        "\n".join(
            [
                '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="320" viewBox="0 0 320 320">',
                '<rect width="320" height="320" fill="white"/>',
                '<rect x="16" y="16" width="288" height="288" fill="none" stroke="black" stroke-width="2"/>',
                '<text x="24" y="42" font-size="12" font-family="monospace">Pass 4 HostCreator seed</text>',
                '<text x="24" y="64" font-size="10" font-family="monospace">Use payload text when qrencode is absent.</text>',
                f'<desc>{encoded_payload}</desc>',
                "</svg>",
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    png_path = artifact_dir / "hostcreator_bootstrap_qr.png"
    qrencode = shutil.which("qrencode")
    if qrencode:
        code, output = run_command([qrencode, "-o", str(png_path), payload])
        if code != 0:
            render_placeholder_png(png_path, payload)
            return {
                "png": str(png_path),
                "svg": str(svg_path),
                "payload": str(payload_path),
                "mode": "placeholder_png_after_qrencode_failure",
                "qrencode_output": output,
            }
        return {
            "png": str(png_path),
            "svg": str(svg_path),
            "payload": str(payload_path),
            "mode": "qrencode_png",
        }
    render_placeholder_png(png_path, payload)
    return {
        "png": str(png_path),
        "svg": str(svg_path),
        "payload": str(payload_path),
        "mode": "placeholder_png_no_qrencode",
    }


def endpoint_to_publisher_public_entry(endpoint: dict[str, Any]) -> dict[str, Any]:
    protocol = endpoint.get("protocol", "https")
    return {
        "endpoint_id": endpoint["endpoint_id"],
        "actor_id": endpoint["actor_id"],
        "role": endpoint["role"],
        "url": endpoint_url(endpoint),
        "protocol": protocol,
        "public_host": endpoint["public_host"],
        "tcp_port": endpoint.get("tcp_port"),
        "udp_port": endpoint.get("udp_port"),
        "tls_sni": endpoint.get("tls_sni"),
        "certificate_fingerprint": endpoint.get("certificate_fingerprint"),
        "reachability_class": endpoint.get("reachability_class", "direct"),
        "expires_at_ms": endpoint["expires_at_ms"],
        "chain_id": endpoint["chain_id"],
    }


def build_publisher_public_dht_snapshot(endpoint_map: dict[str, Any]) -> dict[str, Any]:
    endpoints = endpoint_map["endpoints"]
    publisher_authority = find_endpoint(endpoints, "publisher_authority")
    publisher_receiver = find_endpoint(endpoints, "publisher_receiver")
    bridge_entries = [
        endpoint_to_publisher_public_entry(endpoint)
        for endpoint in endpoints
        if endpoint.get("role") == "exit_bridge"
    ]
    return {
        "schema": "veritas.pass4.publisher_public_dht_snapshot.v1",
        "generated_at": utc_now(),
        "run_id": endpoint_map["run_id"],
        "profile": endpoint_map["profile"],
        "chain_id": endpoint_map["chain_id"],
        "publisher_entry": {
            "node_id": "publisher",
            "authority_url": endpoint_url(publisher_authority),
            "receiver_url": endpoint_url(publisher_receiver),
            "authority_endpoint_id": publisher_authority["endpoint_id"],
            "receiver_endpoint_id": publisher_receiver["endpoint_id"],
        },
        "bridge_dht_entries": bridge_entries,
        "publisher_signing_status": "requires_live_publisher_initialize_after_public_endpoint_apply",
        "source": "pass4_public_endpoint_map",
    }


def transcript(path: Path, title: str, rows: list[dict[str, Any]]) -> None:
    lines = [f"# {title}", f"generated_at={utc_now()}", ""]
    for row in rows:
        lines.append(json.dumps(row, sort_keys=True))
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def prepare(args: argparse.Namespace) -> int:
    ensure_wsl(args.allow_non_wsl)
    config_path = Path(args.config)
    config = read_json(config_path)
    profile = args.profile or config.get("profile") or "local_k8s_public"
    run_id = args.run_id or config.get("run_id") or f"pass4-local-public-{time.strftime('%Y%m%d-%H%M%S')}"
    chain_id = config.get("chain_id") or run_id
    artifact_dir = Path(args.artifact_dir or Path("target/pass4-public-ingress") / run_id)
    artifact_dir.mkdir(parents=True, exist_ok=True)

    endpoints = config.get("endpoints")
    if not isinstance(endpoints, list):
        raise ValidationError("run profile must contain endpoints array")
    normalized_endpoints = []
    for endpoint in endpoints:
        if not isinstance(endpoint, dict):
            raise ValidationError("endpoint entries must be objects")
        next_endpoint = dict(endpoint)
        next_endpoint.setdefault("profile", profile)
        next_endpoint.setdefault("chain_id", chain_id)
        normalized_endpoints.append(next_endpoint)

    endpoint_map = {
        "schema": "veritas.pass4.public_endpoint_map.v1",
        "generated_at": utc_now(),
        "run_id": run_id,
        "profile": profile,
        "chain_id": chain_id,
        "source_config": str(config_path),
        "network_check_mode": "skipped" if args.skip_network_checks else "live",
        "endpoints": normalized_endpoints,
    }

    errors = validate_endpoint_map(endpoint_map)
    if errors:
        write_json(artifact_dir / "validation_errors.json", {"errors": errors})
        raise ValidationError("endpoint validation failed:\n- " + "\n- ".join(errors))

    k8s_status = (
        {"status": "skipped", "reason": "skipped by --skip-k8s-check", "namespace": args.namespace}
        if args.skip_k8s_check
        else check_k8s(args.namespace)
    )
    if k8s_status["status"] == "fail" and not args.allow_k8s_not_ready:
        write_json(artifact_dir / "k8s_readiness.json", k8s_status)
        raise ValidationError("local k8s Pass 3 topology is not ready")

    reachability = reachability_checks(normalized_endpoints, args.skip_network_checks)
    if not args.skip_network_checks and any(item.get("status") == "fail" for item in reachability):
        transcript(artifact_dir / "public_reachability_transcript.txt", "Public Reachability", reachability)
        raise ValidationError("one or more public endpoint reachability checks failed")

    admin_checks = config.get("admin_checks") or []
    if not isinstance(admin_checks, list):
        raise ValidationError("admin_checks must be an array when present")
    admin_denial = admin_denial_checks(admin_checks, args.skip_network_checks)
    if not args.skip_network_checks and any(item.get("status") == "fail" for item in admin_denial):
        transcript(artifact_dir / "admin_denial_transcript.txt", "Admin Denial", admin_denial)
        raise ValidationError("one or more public admin denial checks failed")

    seed = build_host_creator_seed(config, normalized_endpoints, run_id, chain_id)
    seed_errors = validate_host_seed(seed)
    if seed_errors:
        write_json(artifact_dir / "hostcreator_bootstrap_seed_errors.json", {"errors": seed_errors})
        raise ValidationError("HostCreator seed validation failed:\n- " + "\n- ".join(seed_errors))

    public_endpoint_map_path = artifact_dir / "public_endpoint_map.json"
    publisher_snapshot = build_publisher_public_dht_snapshot(endpoint_map)
    write_json(public_endpoint_map_path, endpoint_map)
    write_json(artifact_dir / "publisher_public_dht_snapshot.json", publisher_snapshot)
    write_json(artifact_dir / "hostcreator_bootstrap_seed.json", seed)
    write_json(
        artifact_dir / "hostcreator_bootstrap_seed.redacted.json",
        {
            **seed,
            "redaction": {
                "private_fields_present": False,
                "admin_fields_present": False,
                "publisher_or_bridge_shortcut_fields_present": False,
            },
        },
    )
    qr = render_qr_artifacts(artifact_dir, compact_json(seed))
    transcript(artifact_dir / "public_reachability_transcript.txt", "Public Reachability", reachability)
    transcript(artifact_dir / "admin_denial_transcript.txt", "Admin Denial", admin_denial)
    write_json(artifact_dir / "k8s_readiness.json", k8s_status)
    evidence = {
        "schema": "veritas.pass4.public_ingress_evidence.v1",
        "generated_at": utc_now(),
        "result": "pass",
        "run_id": run_id,
        "profile": profile,
        "chain_id": chain_id,
        "artifact_dir": str(artifact_dir),
        "endpoint_validation": "pass",
        "k8s_readiness": k8s_status["status"],
        "network_check_mode": "skipped" if args.skip_network_checks else "live",
        "reachability_summary": reachability,
        "admin_denial_summary": admin_denial,
        "hostcreator_qr": qr,
        "public_endpoint_map_sha256": sha256_hex_bytes(public_endpoint_map_path.read_bytes()),
        "publisher_public_dht_snapshot_sha256": sha256_hex_bytes(
            (artifact_dir / "publisher_public_dht_snapshot.json").read_bytes()
        ),
        "hostcreator_seed_sha256": sha256_hex_bytes((artifact_dir / "hostcreator_bootstrap_seed.json").read_bytes()),
        "live_signing_note": "Publisher reinitialization/signing must be rerun against the live public endpoint config before Phase 5 mobile validation.",
    }
    write_json(artifact_dir / "public_ingress_evidence.json", evidence)
    print(f"Pass 4 public ingress artifacts written to {artifact_dir}")
    return 0


def verify(args: argparse.Namespace) -> int:
    ensure_wsl(args.allow_non_wsl)
    artifact_dir = Path(args.artifact_dir)
    endpoint_map_path = artifact_dir / "public_endpoint_map.json"
    seed_path = artifact_dir / "hostcreator_bootstrap_seed.redacted.json"
    evidence_path = artifact_dir / "public_ingress_evidence.json"

    missing = [
        str(path)
        for path in (
            endpoint_map_path,
            artifact_dir / "publisher_public_dht_snapshot.json",
            seed_path,
            evidence_path,
        )
        if not path.exists()
    ]
    if args.require_hostcreator_qr:
        for path in (
            artifact_dir / "hostcreator_bootstrap_qr.png",
            artifact_dir / "hostcreator_bootstrap_qr_payload.txt",
        ):
            if not path.exists():
                missing.append(str(path))
    if missing:
        raise ValidationError("missing required artifacts:\n- " + "\n- ".join(missing))

    endpoint_map = read_json(endpoint_map_path)
    errors = validate_endpoint_map(endpoint_map)
    if errors:
        raise ValidationError("endpoint map validation failed:\n- " + "\n- ".join(errors))

    seed = read_json(seed_path)
    seed_errors = validate_host_seed(seed)
    if seed_errors:
        raise ValidationError("HostCreator seed validation failed:\n- " + "\n- ".join(seed_errors))

    endpoints = endpoint_map["endpoints"]
    role_counts = {role: sum(1 for endpoint in endpoints if endpoint.get("role") == role) for role in ALLOWED_ROLES}
    if args.require_public_dht_endpoints:
        if role_counts["publisher_authority"] < 1 or role_counts["publisher_receiver"] < 1:
            raise ValidationError("publisher authority and receiver public endpoints are required")
        if role_counts["exit_bridge"] < 1:
            raise ValidationError("at least one public ExitBridge endpoint is required")

    if args.require_no_public_admin:
        admin_checks = read_json(evidence_path).get("admin_denial_summary", [])
        if any(item.get("status") == "fail" for item in admin_checks):
            raise ValidationError("public admin denial evidence contains failures")
        for endpoint in endpoints:
            for _, port in endpoint_ports(endpoint):
                if port in ADMIN_PORTS:
                    raise ValidationError(f"admin/private port {port} appears in endpoint map")

    reachability = reachability_checks(endpoints, args.skip_network_checks)
    admin_denial = admin_denial_checks(read_json(evidence_path).get("admin_denial_summary", []), args.skip_network_checks)
    if not args.skip_network_checks and any(item.get("status") == "fail" for item in reachability):
        raise ValidationError("live reachability recheck failed")
    if not args.skip_network_checks and any(item.get("status") == "fail" for item in admin_denial):
        raise ValidationError("live admin-denial recheck failed")

    report = {
        "schema": "veritas.pass4.public_ingress_verify.v1",
        "generated_at": utc_now(),
        "result": "pass",
        "artifact_dir": str(artifact_dir),
        "endpoint_count": len(endpoints),
        "role_counts": role_counts,
        "network_check_mode": "skipped" if args.skip_network_checks else "live",
    }
    write_json(artifact_dir / "public_ingress_verify.json", report)
    print(f"Pass 4 public ingress verification passed for {artifact_dir}")
    return 0


def down(args: argparse.Namespace) -> int:
    ensure_wsl(args.allow_non_wsl)
    artifact_dir = Path(args.artifact_dir)
    endpoint_map_path = artifact_dir / "public_endpoint_map.json"
    if not endpoint_map_path.exists():
        raise ValidationError(f"missing endpoint map: {endpoint_map_path}")
    endpoint_map = read_json(endpoint_map_path)
    invalidated = {
        **endpoint_map,
        "invalidated_at": utc_now(),
        "invalidated": True,
        "teardown_mode": "artifact_invalidation",
        "teardown_note": "Router/firewall/tunnel teardown is operator-owned; this script invalidates the run artifacts and descriptors.",
    }
    invalidated_path = artifact_dir / "public_endpoint_map.invalidated.json"
    write_json(invalidated_path, invalidated)
    teardown = {
        "schema": "veritas.pass4.public_ingress_teardown.v1",
        "generated_at": utc_now(),
        "result": "pass",
        "artifact_dir": str(artifact_dir),
        "run_id": args.run_id or endpoint_map.get("run_id"),
        "invalidated_endpoint_map": str(invalidated_path),
    }
    write_json(artifact_dir / "public_ingress_teardown.json", teardown)
    transcript(
        artifact_dir / "teardown_transcript.txt",
        "Public Ingress Teardown",
        [
            {
                "status": "pass",
                "message": "Endpoint descriptors invalidated. Remove external router/firewall/tunnel rules for this run id.",
                "run_id": teardown["run_id"],
            }
        ],
    )
    print(f"Pass 4 public ingress artifacts invalidated in {artifact_dir}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Pass 4 public ingress helper")
    subparsers = parser.add_subparsers(dest="command", required=True)

    prepare_parser = subparsers.add_parser("prepare")
    prepare_parser.add_argument("--profile", default=None)
    prepare_parser.add_argument("--run-id", default=None)
    prepare_parser.add_argument("--config", required=True)
    prepare_parser.add_argument("--artifact-dir", default=None)
    prepare_parser.add_argument("--namespace", default="veritas")
    prepare_parser.add_argument("--skip-network-checks", action="store_true")
    prepare_parser.add_argument("--skip-k8s-check", action="store_true")
    prepare_parser.add_argument("--allow-k8s-not-ready", action="store_true")
    prepare_parser.add_argument("--allow-non-wsl", action="store_true")
    prepare_parser.set_defaults(func=prepare)

    verify_parser = subparsers.add_parser("verify")
    verify_parser.add_argument("--artifact-dir", required=True)
    verify_parser.add_argument("--require-no-public-admin", action="store_true")
    verify_parser.add_argument("--require-hostcreator-qr", action="store_true")
    verify_parser.add_argument("--require-public-dht-endpoints", action="store_true")
    verify_parser.add_argument("--skip-network-checks", action="store_true")
    verify_parser.add_argument("--allow-non-wsl", action="store_true")
    verify_parser.set_defaults(func=verify)

    down_parser = subparsers.add_parser("down")
    down_parser.add_argument("--artifact-dir", required=True)
    down_parser.add_argument("--run-id", default=None)
    down_parser.add_argument("--allow-non-wsl", action="store_true")
    down_parser.set_defaults(func=down)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except ValidationError as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
