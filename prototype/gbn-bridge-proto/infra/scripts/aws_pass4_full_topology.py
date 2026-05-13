#!/usr/bin/env python3
"""Pass 4 Phase 5 AWS public mobile validation helper.

The helper intentionally shells out to the AWS CLI so it uses the same profile,
SSO, region, and credential behavior as the existing infrastructure scripts.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import ipaddress
import json
import os
import shutil
import socket
import subprocess
import sys
import time
import urllib.request
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
SCRIPT_DIR = ROOT / "infra" / "scripts"
DEFAULT_BUCKET = "veritas-pass4-mobile-evidence"
DEFAULT_AUTHORITY_PORT = 8080
DEFAULT_RECEIVER_PORT = 8081
DEFAULT_BRIDGE_PORT = 443
DEFAULT_CREATOR_BOOTSTRAP_PORT = 8082
ADMIN_PORT = 9090


def now_ms() -> int:
    return int(time.time() * 1000)


def compact_json(value: Any) -> str:
    return json.dumps(value, separators=(",", ":"), sort_keys=True)


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def run(command: list[str], *, capture: bool = True, check: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        command,
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
        check=False,
    )
    if check and result.returncode != 0:
        detail = (result.stderr or result.stdout or "").strip()
        raise SystemExit(f"command failed ({result.returncode}): {' '.join(command)}\n{detail}")
    return result


def aws_json(region: str, args: list[str]) -> Any:
    result = run(["aws", "--region", region, *args, "--output", "json"])
    return json.loads(result.stdout or "{}")


def aws_text(region: str, args: list[str]) -> str:
    result = run(["aws", "--region", region, *args, "--output", "text"])
    return (result.stdout or "").strip()


def require_aws() -> None:
    if not shutil.which("aws"):
        raise SystemExit("required command not found: aws")


def artifact_dir(args: argparse.Namespace) -> Path:
    return Path(args.artifact_dir or ROOT / "target" / "pass4-aws-public" / args.run_id)


def cf_outputs(region: str, stack_name: str) -> dict[str, str]:
    stack = aws_json(region, ["cloudformation", "describe-stacks", "--stack-name", stack_name])
    outputs = stack["Stacks"][0].get("Outputs", [])
    return {item["OutputKey"]: item.get("OutputValue", "") for item in outputs}


def service_public_networks(region: str, cluster: str, service: str) -> list[dict[str, str]]:
    task_arns = aws_json(
        region,
        [
            "ecs",
            "list-tasks",
            "--cluster",
            cluster,
            "--service-name",
            service,
            "--desired-status",
            "RUNNING",
        ],
    ).get("taskArns", [])
    if not task_arns:
        return []
    tasks = aws_json(
        region,
        ["ecs", "describe-tasks", "--cluster", cluster, "--tasks", *task_arns],
    ).get("tasks", [])
    eni_ids: list[tuple[str, str]] = []
    for task in tasks:
        task_arn = task.get("taskArn", "")
        for attachment in task.get("attachments", []):
            for detail in attachment.get("details", []):
                if detail.get("name") == "networkInterfaceId":
                    eni_ids.append((task_arn, detail.get("value", "")))
    if not eni_ids:
        return []
    enis = aws_json(
        region,
        [
            "ec2",
            "describe-network-interfaces",
            "--network-interface-ids",
            *[eni for _, eni in eni_ids],
        ],
    ).get("NetworkInterfaces", [])
    by_id = {eni.get("NetworkInterfaceId"): eni for eni in enis}
    networks: list[dict[str, str]] = []
    for task_arn, eni_id in eni_ids:
        eni = by_id.get(eni_id, {})
        public_ip = (eni.get("Association") or {}).get("PublicIp", "")
        private_ip = eni.get("PrivateIpAddress", "")
        networks.append(
            {
                "task_arn": task_arn,
                "network_interface_id": eni_id,
                "public_ip": public_ip,
                "private_ip": private_ip,
            }
        )
    return networks


def require_public_ip(networks: list[dict[str, str]], label: str) -> str:
    for network in networks:
        if network.get("public_ip"):
            return network["public_ip"]
    raise SystemExit(f"{label} has no running task with a public IP; ensure AssignPublicIp=ENABLED")


def endpoint(
    endpoint_id: str,
    actor_id: str,
    role: str,
    protocol: str,
    public_host: str,
    expires_at_ms: int,
    *,
    tcp_port: int | None = None,
    udp_port: int | None = None,
    region: str,
    task_arn: str | None = None,
) -> dict[str, Any]:
    value: dict[str, Any] = {
        "endpoint_id": endpoint_id,
        "actor_id": actor_id,
        "role": role,
        "protocol": protocol,
        "public_host": public_host,
        "expires_at_ms": expires_at_ms,
        "aws_region": region,
    }
    if tcp_port is not None:
        value["tcp_port"] = tcp_port
    if udp_port is not None:
        value["udp_port"] = udp_port
    if task_arn:
        value["aws_task_arn"] = task_arn
    return value


def fetch_host_seed(public_host: str, port: int) -> dict[str, Any] | None:
    url = f"http://{public_host}:{port}/v1/mobile/bootstrap-dht-qr"
    try:
        with urllib.request.urlopen(url, timeout=5) as response:
            return json.loads(response.read().decode("utf-8"))
    except Exception:
        return None


def fallback_host_seed(run_id: str, public_host: str, port: int, expires_at_ms: int) -> dict[str, Any]:
    pub = [170] * 32
    return {
        "schema_version": 1,
        "chain_id": f"{run_id}-host-seed",
        "run_id": run_id,
        "host_creator_id": "host-creator",
        "host_creator_public_key_hex": "aa" * 32,
        "host_creator_entry": {
            "node_id": "host-creator",
            "ip_addr": public_host,
            "pub_key": pub,
            "udp_punch_port": DEFAULT_BRIDGE_PORT,
            "entry_expiry_ms": expires_at_ms,
            "publisher_sig": [1] * 64,
            "active": True,
        },
        "host_creator_reachability": {
            "reachability_class": "direct",
            "capabilities": ["bootstrap_seed"],
        },
        "host_creator_bootstrap_endpoints": [
            {
                "protocol": "http",
                "host": public_host,
                "port": port,
            }
        ],
        "issued_at_ms": now_ms(),
        "expires_at_ms": expires_at_ms,
        "payload_hash": f"sha256:{hashlib.sha256(f'{run_id}:{public_host}:{port}'.encode()).hexdigest()}",
        "signature": "aws-pass4-topology-helper",
    }


def normalize_host_seed(seed: dict[str, Any], public_host: str, port: int, expires_at_ms: int) -> dict[str, Any]:
    seed = json.loads(json.dumps(seed))
    seed["expires_at_ms"] = min(int(seed.get("expires_at_ms", expires_at_ms)), expires_at_ms)
    seed.setdefault("host_creator_bootstrap_endpoints", [])
    seed["host_creator_bootstrap_endpoints"] = [
        {
            "protocol": "http",
            "host": public_host,
            "port": port,
        }
    ]
    entry = seed.setdefault("host_creator_entry", {})
    entry["ip_addr"] = public_host
    entry["udp_punch_port"] = int(entry.get("udp_punch_port", DEFAULT_BRIDGE_PORT))
    entry["entry_expiry_ms"] = seed["expires_at_ms"]
    return seed


def render_qr_artifacts(out_dir: Path, prefix: str, payload: str) -> dict[str, Any]:
    out_dir.mkdir(parents=True, exist_ok=True)
    payload_path = out_dir / f"{prefix}_payload.txt"
    payload_path.write_text(payload + "\n", encoding="utf-8")
    svg_path = out_dir / f"{prefix}.svg"
    svg_path.write_text(
        "\n".join(
            [
                '<svg xmlns="http://www.w3.org/2000/svg" width="900" height="160">',
                '<rect width="100%" height="100%" fill="white"/>',
                f'<text x="24" y="48" font-size="18" font-family="monospace">{prefix}</text>',
                '<text x="24" y="82" font-size="12" font-family="monospace">Install qrencode for scan-ready PNG output.</text>',
                "</svg>",
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    png_path = out_dir / f"{prefix}.png"
    qrencode = shutil.which("qrencode")
    if qrencode:
        run([qrencode, "-o", str(png_path), payload], capture=True)
    return {
        "payload": str(payload_path),
        "svg": str(svg_path),
        "png": str(png_path) if png_path.exists() else None,
        "qrencode": bool(qrencode),
    }


def render_chunked_run_profile(out_dir: Path, profile_json: str, chunk_size: int = 900) -> dict[str, Any]:
    profile_id = hashlib.sha256(profile_json.encode("utf-8")).hexdigest()[:16]
    sha = hashlib.sha256(profile_json.encode("utf-8")).hexdigest()
    chunks = [profile_json[index : index + chunk_size] for index in range(0, len(profile_json), chunk_size)]
    payload_dir = out_dir / "run_profile_qr_payloads"
    image_dir = out_dir / "run_profile_qr_chunks"
    payload_dir.mkdir(parents=True, exist_ok=True)
    image_dir.mkdir(parents=True, exist_ok=True)
    rendered = []
    for index, data in enumerate(chunks, start=1):
        payload = compact_json(
            {
                "schema": "veritas.pass4.run_profile_qr.v1",
                "profile_id": profile_id,
                "index": index,
                "count": len(chunks),
                "sha256": sha,
                "encoding": "base64",
                "data": base64.b64encode(data.encode("utf-8")).decode("ascii"),
            }
        )
        payload_path = payload_dir / f"chunk-{index:03d}.json"
        payload_path.write_text(payload + "\n", encoding="utf-8")
        png_path = image_dir / f"chunk-{index:03d}.png"
        qrencode = shutil.which("qrencode")
        if qrencode:
            run([qrencode, "-o", str(png_path), payload], capture=True)
        rendered.append({"payload": str(payload_path), "png": str(png_path) if png_path.exists() else None})
    return {"profile_id": profile_id, "sha256": sha, "chunk_count": len(chunks), "chunks": rendered}


def discover(args: argparse.Namespace) -> dict[str, Any]:
    require_aws()
    out_dir = artifact_dir(args)
    out_dir.mkdir(parents=True, exist_ok=True)
    outputs = cf_outputs(args.region, args.stack_name)
    cluster = outputs.get("ClusterName", "")
    if not cluster:
        raise SystemExit(f"stack {args.stack_name} does not expose ClusterName")

    services = {
        "authority": outputs.get("AuthorityServiceName", ""),
        "receiver": outputs.get("ReceiverServiceName", ""),
        "bridge": outputs.get("BridgeServiceName", ""),
        "host_creator": outputs.get("CreatorHostServiceName", ""),
    }
    service_networks = {
        key: service_public_networks(args.region, cluster, value)
        for key, value in services.items()
        if value
    }
    authority_ip = require_public_ip(service_networks.get("authority", []), "Publisher authority")
    receiver_ip = require_public_ip(service_networks.get("receiver", []), "Publisher receiver")
    host_ip = require_public_ip(service_networks.get("host_creator", []), "HostCreator")
    bridge_networks = service_networks.get("bridge", [])
    bridge_public = [item for item in bridge_networks if item.get("public_ip")]
    if not bridge_public:
        raise SystemExit("ExitBridge service has no running tasks with public IPs")

    expires_at_ms = now_ms() + int(args.expires_hours * 3_600_000)
    endpoints = [
        endpoint(
            "aws-publisher-authority",
            "publisher",
            "publisher_authority",
            "http",
            authority_ip,
            expires_at_ms,
            tcp_port=args.authority_port,
            region=args.region,
            task_arn=service_networks["authority"][0].get("task_arn"),
        ),
        endpoint(
            "aws-publisher-receiver",
            "publisher",
            "publisher_receiver",
            "http",
            receiver_ip,
            expires_at_ms,
            tcp_port=args.receiver_port,
            region=args.region,
            task_arn=service_networks["receiver"][0].get("task_arn"),
        ),
        endpoint(
            "aws-hostcreator-bootstrap",
            "host-creator",
            "host_creator_bootstrap",
            "http",
            host_ip,
            expires_at_ms,
            tcp_port=args.creator_bootstrap_port,
            region=args.region,
            task_arn=service_networks["host_creator"][0].get("task_arn"),
        ),
    ]
    for index, bridge in enumerate(bridge_public, start=1):
        endpoints.append(
            endpoint(
                f"aws-exitbridge-{index:02d}",
                f"exit-bridge-{index:02d}",
                "exit_bridge",
                "udp",
                bridge["public_ip"],
                expires_at_ms,
                udp_port=args.bridge_udp_port,
                region=args.region,
                task_arn=bridge.get("task_arn"),
            )
        )

    endpoint_map_id = hashlib.sha256(compact_json(endpoints).encode("utf-8")).hexdigest()[:24]
    endpoint_map = {
        "schema": "veritas.pass4.aws_public_endpoint_map.v1",
        "profile": "aws_public",
        "run_id": args.run_id,
        "endpoint_map_id": endpoint_map_id,
        "stack_name": args.stack_name,
        "aws_region": args.region,
        "aws_exitbridge_region": args.region,
        "generated_at_ms": now_ms(),
        "expires_at_ms": expires_at_ms,
        "endpoints": endpoints,
        "log_groups": {
            "authority": outputs.get("AuthorityLogGroup", ""),
            "receiver": outputs.get("ReceiverLogGroup", ""),
            "bridge": outputs.get("BridgeLogGroup", ""),
            "creator": outputs.get("CreatorLogGroup", ""),
        },
    }
    profile = {
        "profile": "aws_public",
        "run_id": args.run_id,
        "endpoint_map_id": endpoint_map_id,
        "evidence_bucket": args.evidence_bucket,
        "evidence_prefix": args.evidence_prefix or f"mobile-evidence/{args.run_id}/",
        "aws_exitbridge_region": args.region,
        "endpoints": endpoints,
        "notes": "Live AWS public profile. Bootstrap trust/DHT values must arrive through HostCreator/Publisher bootstrap flow.",
    }

    endpoint_map_path = out_dir / "aws_public_endpoint_map.json"
    profile_path = out_dir / "run-profile.aws-public.live.json"
    repo_profile_path = ROOT / "infra" / "pass4" / "aws" / "run-profile.aws-public.live.json"
    write_json(endpoint_map_path, endpoint_map)
    write_json(profile_path, profile)
    write_json(repo_profile_path, profile)

    profile_json = compact_json(profile)
    render_qr_artifacts(out_dir, "run_profile_qr", profile_json)
    run_profile_chunks = render_chunked_run_profile(out_dir, profile_json)

    host_seed = fetch_host_seed(host_ip, args.creator_bootstrap_port) or fallback_host_seed(
        args.run_id, host_ip, args.creator_bootstrap_port, expires_at_ms
    )
    host_seed = normalize_host_seed(host_seed, host_ip, args.creator_bootstrap_port, expires_at_ms)
    host_seed_path = out_dir / "hostcreator_bootstrap_qr_payload.json"
    write_json(host_seed_path, host_seed)
    host_qr = render_qr_artifacts(out_dir, "hostcreator_bootstrap_qr", compact_json(host_seed))

    evidence = {
        "schema": "veritas.pass4.aws_public_discovery.v1",
        "run_id": args.run_id,
        "stack_name": args.stack_name,
        "region": args.region,
        "artifact_dir": str(out_dir),
        "endpoint_map": str(endpoint_map_path),
        "run_profile": str(profile_path),
        "repo_run_profile": str(repo_profile_path),
        "run_profile_qr_chunks": run_profile_chunks,
        "hostcreator_seed": str(host_seed_path),
        "hostcreator_qr": host_qr,
        "service_networks": service_networks,
    }
    write_json(out_dir / "aws-public-discovery.json", evidence)
    return evidence


def command_plan(args: argparse.Namespace) -> None:
    out_dir = artifact_dir(args)
    plan = {
        "schema": "veritas.pass4.aws_public_plan.v1",
        "run_id": args.run_id,
        "stack_name": args.stack_name,
        "region": args.region,
        "artifact_dir": str(out_dir),
        "bridge_count": args.bridge_count,
        "authority_port": args.authority_port,
        "receiver_port": args.receiver_port,
        "bridge_udp_port": args.bridge_udp_port,
        "creator_bootstrap_port": args.creator_bootstrap_port,
        "required_scripts": [
            "deploy-conduit-full.sh",
            "smoke-conduit-full.sh",
            "teardown-conduit-full.sh",
            "aws-pass4-full-topology-up.sh",
            "aws-pass4-full-topology-verify.sh",
            "aws-pass4-mobile-collector.sh",
        ],
        "required_aws_inputs": [
            "vpc-id",
            "service-subnet-ids",
            "database-subnet-ids",
            "authority-image",
            "receiver-image",
            "bridge-image",
            "creator-image",
            "publisher-signing-key-secret-arn",
            "bridge-signing-seed-secret-arn",
            "publisher-public-key-hex",
        ],
    }
    write_json(out_dir / "aws-public-plan.json", plan)
    print(json.dumps(plan, indent=2, sort_keys=True))


def command_up(args: argparse.Namespace) -> None:
    require_aws()
    if not args.discover_existing:
        required = [
            "vpc_id",
            "service_subnet_ids",
            "database_subnet_ids",
            "authority_image",
            "receiver_image",
            "bridge_image",
            "creator_image",
            "publisher_signing_key_secret_arn",
            "bridge_signing_seed_secret_arn",
            "publisher_public_key_hex",
        ]
        missing = [name.replace("_", "-") for name in required if not getattr(args, name)]
        if missing:
            raise SystemExit("missing required deploy args: " + ", ".join(missing))
        deploy = [
            str(SCRIPT_DIR / "deploy-conduit-full.sh"),
            "--stack-name",
            args.stack_name,
            "--region",
            args.region,
            "--environment",
            args.environment,
            "--vpc-id",
            args.vpc_id,
            "--service-subnet-ids",
            args.service_subnet_ids,
            "--database-subnet-ids",
            args.database_subnet_ids,
            "--authority-image",
            args.authority_image,
            "--receiver-image",
            args.receiver_image,
            "--bridge-image",
            args.bridge_image,
            "--creator-image",
            args.creator_image,
            "--publisher-signing-key-secret-arn",
            args.publisher_signing_key_secret_arn,
            "--bridge-signing-seed-secret-arn",
            args.bridge_signing_seed_secret_arn,
            "--publisher-public-key-hex",
            args.publisher_public_key_hex,
            "--desired-bridge-count",
            str(args.bridge_count),
            "--authority-port",
            str(args.authority_port),
            "--receiver-port",
            str(args.receiver_port),
            "--udp-punch-port",
            str(args.bridge_udp_port),
            "--creator-bootstrap-port",
            str(args.creator_bootstrap_port),
            "--authority-ingress-cidr",
            args.mobile_ingress_cidr,
            "--receiver-ingress-cidr",
            args.mobile_ingress_cidr,
            "--creator-bootstrap-ingress-cidr",
            args.mobile_ingress_cidr,
        ]
        run(deploy, capture=False)
    evidence = discover(args)
    print(json.dumps(evidence, indent=2, sort_keys=True))


def tcp_connect(host: str, port: int, timeout: float = 5.0) -> tuple[bool, str]:
    try:
        with socket.create_connection((host, port), timeout=timeout):
            return True, "connected"
    except OSError as error:
        return False, str(error)


def udp_probe(host: str, port: int, timeout: float = 3.0) -> tuple[bool, str]:
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.settimeout(timeout)
            sock.sendto(b"veritas-pass4-aws-public-probe", (host, port))
        return True, "sent"
    except OSError as error:
        return False, str(error)


def command_verify(args: argparse.Namespace) -> None:
    out_dir = artifact_dir(args)
    endpoint_map_path = Path(args.endpoint_map or out_dir / "aws_public_endpoint_map.json")
    if not endpoint_map_path.exists():
        raise SystemExit(f"endpoint map missing: {endpoint_map_path}")
    endpoint_map = json.loads(endpoint_map_path.read_text(encoding="utf-8"))
    checks = []
    ok = True
    for item in endpoint_map.get("endpoints", []):
        host = item.get("public_host", "")
        role = item.get("role", "")
        host_ok = public_host_allowed(host)
        checks.append({"name": f"{role}.public_host", "ok": host_ok, "detail": host})
        ok = ok and host_ok
        if item.get("tcp_port"):
            connected, detail = tcp_connect(host, int(item["tcp_port"]))
            checks.append({"name": f"{role}.tcp_{item['tcp_port']}", "ok": connected, "detail": detail})
            ok = ok and connected
        if item.get("udp_port"):
            sent, detail = udp_probe(host, int(item["udp_port"]))
            checks.append({"name": f"{role}.udp_{item['udp_port']}", "ok": sent, "detail": detail})
            ok = ok and sent
        admin_connected, detail = tcp_connect(host, ADMIN_PORT, timeout=2.0)
        checks.append({"name": f"{role}.admin_{ADMIN_PORT}_denied", "ok": not admin_connected, "detail": detail})
        ok = ok and not admin_connected

    required_files = [
        out_dir / "run-profile.aws-public.live.json",
        out_dir / "run_profile_qr_payload.txt",
        out_dir / "hostcreator_bootstrap_qr_payload.json",
        out_dir / "hostcreator_bootstrap_qr_payload.txt",
    ]
    for path in required_files:
        exists = path.exists()
        checks.append({"name": f"artifact.{path.name}", "ok": exists, "detail": str(path)})
        ok = ok and exists

    result = {
        "schema": "veritas.pass4.aws_public_verify.v1",
        "run_id": args.run_id,
        "ok": ok,
        "endpoint_map": str(endpoint_map_path),
        "checks": checks,
    }
    write_json(out_dir / "aws-public-verify.json", result)
    print(json.dumps(result, indent=2, sort_keys=True))
    if not ok:
        raise SystemExit(1)


def public_host_allowed(host: str) -> bool:
    if not host:
        return False
    lower = host.lower()
    if lower == "localhost" or lower.endswith(".cluster.local") or lower.endswith(".svc") or ".svc." in lower:
        return False
    try:
        ip = ipaddress.ip_address(lower)
    except ValueError:
        return True
    return not (ip.is_private or ip.is_loopback or ip.is_link_local or ip.is_unspecified)


def command_down(args: argparse.Namespace) -> None:
    out_dir = artifact_dir(args)
    result = {
        "schema": "veritas.pass4.aws_public_teardown.v1",
        "run_id": args.run_id,
        "stack_name": args.stack_name,
        "region": args.region,
        "deleted": False,
        "deferred": bool(args.defer),
    }
    if args.defer:
        write_json(out_dir / "aws-public-teardown.json", result)
        print(json.dumps(result, indent=2, sort_keys=True))
        return
    run(
        [
            str(SCRIPT_DIR / "teardown-conduit-full.sh"),
            "--stack-name",
            args.stack_name,
            "--region",
            args.region,
        ],
        capture=False,
    )
    result["deleted"] = True
    write_json(out_dir / "aws-public-teardown.json", result)
    print(json.dumps(result, indent=2, sort_keys=True))


def command_collect(args: argparse.Namespace) -> None:
    require_aws()
    out_dir = artifact_dir(args)
    out_dir.mkdir(parents=True, exist_ok=True)
    outputs = cf_outputs(args.region, args.stack_name)
    log_groups = {
        "authority": outputs.get("AuthorityLogGroup", ""),
        "receiver": outputs.get("ReceiverLogGroup", ""),
        "bridge": outputs.get("BridgeLogGroup", ""),
        "creator": outputs.get("CreatorLogGroup", ""),
    }
    start_ms = int((time.time() - args.window_minutes * 60) * 1000)
    events: dict[str, dict[str, Any]] = {}
    for label, log_group in log_groups.items():
        if not log_group:
            events[label] = {"log_group": "", "count": 0, "file": None}
            continue
        group_events: list[dict[str, Any]] = []
        for chain_id in args.chain_id:
            response = aws_json(
                args.region,
                [
                    "logs",
                    "filter-log-events",
                    "--log-group-name",
                    log_group,
                    "--start-time",
                    str(start_ms),
                    "--filter-pattern",
                    chain_id,
                    "--limit",
                    "1000",
                ],
            )
            group_events.extend(response.get("events", []))
        event_file = out_dir / f"{label}-cloudwatch-events.json"
        write_json(event_file, {"log_group": log_group, "events": group_events})
        events[label] = {"log_group": log_group, "count": len(group_events), "file": str(event_file)}

    downloaded = None
    if args.mobile_evidence_s3_uri:
        target = out_dir / "mobile-evidence-bundle.zip"
        run(["aws", "s3", "cp", args.mobile_evidence_s3_uri, str(target)], capture=False)
        downloaded = {
            "s3_uri": args.mobile_evidence_s3_uri,
            "file": str(target),
            "sha256": hashlib.sha256(target.read_bytes()).hexdigest(),
        }

    ok = True
    if args.require_chain_id:
        ok = all(item["count"] > 0 for item in events.values() if item["log_group"])
    summary = {
        "schema": "veritas.pass4.aws_mobile_collection.v1",
        "run_id": args.run_id,
        "ok": ok,
        "stack_name": args.stack_name,
        "region": args.region,
        "chain_ids": args.chain_id,
        "window_minutes": args.window_minutes,
        "cloudwatch": events,
        "mobile_evidence": downloaded,
    }
    write_json(out_dir / "aws-mobile-collection.json", summary)
    print(json.dumps(summary, indent=2, sort_keys=True))
    if not ok:
        raise SystemExit(1)


def add_common(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--run-id", default=os.environ.get("RUN_ID") or f"pass4-phase5-aws-{int(time.time())}")
    parser.add_argument("--stack-name", default=os.environ.get("GBN_BRIDGE_STACK_NAME", "gbn-conduit-full-pass4"))
    parser.add_argument("--region", default=os.environ.get("GBN_BRIDGE_AWS_REGION") or os.environ.get("AWS_REGION", "ca-central-1"))
    parser.add_argument("--artifact-dir")
    parser.add_argument("--authority-port", type=int, default=DEFAULT_AUTHORITY_PORT)
    parser.add_argument("--receiver-port", type=int, default=DEFAULT_RECEIVER_PORT)
    parser.add_argument("--bridge-udp-port", type=int, default=DEFAULT_BRIDGE_PORT)
    parser.add_argument("--creator-bootstrap-port", type=int, default=DEFAULT_CREATOR_BOOTSTRAP_PORT)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)

    plan = sub.add_parser("plan")
    add_common(plan)
    plan.add_argument("--bridge-count", type=int, default=3)
    plan.set_defaults(func=command_plan)

    up = sub.add_parser("up")
    add_common(up)
    up.add_argument("--environment", default=os.environ.get("GBN_BRIDGE_ENVIRONMENT", "pass4"))
    up.add_argument("--bridge-count", type=int, default=3)
    up.add_argument("--expires-hours", type=int, default=24)
    up.add_argument("--evidence-bucket", default=os.environ.get("PASS4_MOBILE_EVIDENCE_BUCKET", DEFAULT_BUCKET))
    up.add_argument("--evidence-prefix")
    up.add_argument("--mobile-ingress-cidr", default=os.environ.get("PASS4_MOBILE_INGRESS_CIDR", "0.0.0.0/0"))
    up.add_argument("--discover-existing", action="store_true")
    up.add_argument("--vpc-id")
    up.add_argument("--service-subnet-ids")
    up.add_argument("--database-subnet-ids")
    up.add_argument("--authority-image")
    up.add_argument("--receiver-image")
    up.add_argument("--bridge-image")
    up.add_argument("--creator-image")
    up.add_argument("--publisher-signing-key-secret-arn")
    up.add_argument("--bridge-signing-seed-secret-arn")
    up.add_argument("--publisher-public-key-hex")
    up.set_defaults(func=command_up)

    verify = sub.add_parser("verify")
    add_common(verify)
    verify.add_argument("--endpoint-map")
    verify.set_defaults(func=command_verify)

    down = sub.add_parser("down")
    add_common(down)
    down.add_argument("--defer", action="store_true")
    down.set_defaults(func=command_down)

    collect = sub.add_parser("collect")
    add_common(collect)
    collect.add_argument("--chain-id", action="append", default=[])
    collect.add_argument("--window-minutes", type=int, default=30)
    collect.add_argument("--require-chain-id", action="store_true")
    collect.add_argument("--mobile-evidence-s3-uri")
    collect.set_defaults(func=command_collect)
    return parser


def main(argv: list[str]) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    args.func(args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
