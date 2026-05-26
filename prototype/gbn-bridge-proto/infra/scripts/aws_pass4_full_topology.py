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
DEPLOY_PREREQS_SCHEMA = "veritas.pass4.aws_public_deploy_prereqs.v1"
DEPLOY_PREREQ_ENV = {
    "vpc_id": "VPC_ID",
    "service_subnet_ids": "SERVICE_SUBNET_IDS",
    "database_subnet_ids": "DATABASE_SUBNET_IDS",
    "authority_image": "AUTHORITY_IMAGE",
    "receiver_image": "RECEIVER_IMAGE",
    "bridge_image": "BRIDGE_IMAGE",
    "creator_image": "CREATOR_IMAGE",
    "publisher_signing_key_secret_arn": "PUBLISHER_SIGNING_KEY_SECRET_ARN",
    "bridge_signing_seed_secret_arn": "BRIDGE_SIGNING_SEED_SECRET_ARN",
    "publisher_public_key_hex": "PUBLISHER_PUBLIC_KEY_HEX",
}
ECR_REPO_CANDIDATES = {
    "authority_image": [
        "gbn-conduit-full-authority",
        "gbn-publisher-authority",
        "publisher-authority",
    ],
    "receiver_image": [
        "gbn-conduit-full-receiver",
        "gbn-publisher-receiver",
        "publisher-receiver",
    ],
    "bridge_image": [
        "gbn-conduit-full-bridge",
        "gbn-exit-bridge",
        "exit-bridge",
    ],
    "creator_image": [
        "gbn-conduit-full-creator",
        "gbn-creator-runner",
        "creator-runner",
    ],
}
PUBLIC_KEY_FIELDS = [
    "publisher_public_key_hex",
    "public_key_hex",
    "publisher_public_key",
    "public_key",
    "GBN_BRIDGE_PUBLISHER_PUBLIC_KEY_HEX",
]
SIGNING_KEY_FIELDS = [
    "publisher_signing_key_hex",
    "signing_key_hex",
    "GBN_BRIDGE_PUBLISHER_SIGNING_KEY_HEX",
]


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


def is_hex_32(value: str) -> bool:
    value = value.strip()
    if len(value) != 64:
        return False
    try:
        bytes.fromhex(value)
    except ValueError:
        return False
    return True


def value_from_arg_or_env(args: argparse.Namespace, name: str) -> tuple[str | None, str | None]:
    value = getattr(args, name, None)
    if value:
        return str(value), "cli"
    env_name = DEPLOY_PREREQ_ENV[name]
    value = os.environ.get(env_name)
    if value:
        return value, f"env:{env_name}"
    return None, None


def optional_aws_json(region: str, args: list[str], warnings: list[str]) -> Any | None:
    try:
        return aws_json(region, args)
    except SystemExit as error:
        warnings.append(str(error))
        return None


def try_aws_json(region: str, args: list[str]) -> Any | None:
    result = run(["aws", "--region", region, *args, "--output", "json"], check=False)
    if result.returncode != 0:
        return None
    try:
        return json.loads(result.stdout or "{}")
    except json.JSONDecodeError:
        return None


def discover_account_id(region: str, warnings: list[str]) -> str | None:
    response = optional_aws_json(region, ["sts", "get-caller-identity"], warnings)
    if not response:
        return None
    return response.get("Account")


def discover_vpc(region: str, warnings: list[str]) -> tuple[str | None, str | None]:
    response = optional_aws_json(
        region,
        ["ec2", "describe-vpcs", "--filters", "Name=isDefault,Values=true"],
        warnings,
    )
    vpcs = (response or {}).get("Vpcs", [])
    if vpcs:
        return vpcs[0].get("VpcId"), "aws:ec2.default_vpc"
    response = optional_aws_json(region, ["ec2", "describe-vpcs"], warnings)
    vpcs = sorted((response or {}).get("Vpcs", []), key=lambda item: item.get("VpcId", ""))
    if vpcs:
        return vpcs[0].get("VpcId"), "aws:ec2.first_vpc"
    return None, None


def select_subnet_ids(subnets: list[dict[str, Any]], *, prefer_public: bool) -> list[str]:
    filtered = [
        item
        for item in subnets
        if bool(item.get("MapPublicIpOnLaunch")) is prefer_public and item.get("SubnetId")
    ]
    if len(filtered) < 2:
        filtered = [item for item in subnets if item.get("SubnetId")]
    filtered = sorted(filtered, key=lambda item: (item.get("AvailabilityZone", ""), item.get("SubnetId", "")))
    by_az: dict[str, dict[str, Any]] = {}
    for subnet in filtered:
        by_az.setdefault(subnet.get("AvailabilityZone", ""), subnet)
    selected = list(by_az.values())[:2]
    if len(selected) < 2:
        selected = filtered[:2]
    return [str(item["SubnetId"]) for item in selected]


def discover_subnet_sets(region: str, vpc_id: str, warnings: list[str]) -> dict[str, tuple[str | None, str | None]]:
    response = optional_aws_json(
        region,
        ["ec2", "describe-subnets", "--filters", f"Name=vpc-id,Values={vpc_id}"],
        warnings,
    )
    subnets = (response or {}).get("Subnets", [])
    public_ids = select_subnet_ids(subnets, prefer_public=True)
    private_ids = select_subnet_ids(subnets, prefer_public=False)
    values: dict[str, tuple[str | None, str | None]] = {
        "service_subnet_ids": (",".join(public_ids), "aws:ec2.public_subnets") if len(public_ids) >= 2 else (None, None),
        "database_subnet_ids": (",".join(private_ids), "aws:ec2.private_subnets") if len(private_ids) >= 2 else (None, None),
    }
    if values["database_subnet_ids"][0] is None and len(public_ids) >= 2:
        values["database_subnet_ids"] = (",".join(public_ids), "aws:ec2.public_subnets_fallback")
    return values


def discover_latest_ecr_image(region: str, repo_candidates: list[str], warnings: list[str]) -> tuple[str | None, str | None]:
    for repo in repo_candidates:
        repository = try_aws_json(region, ["ecr", "describe-repositories", "--repository-names", repo])
        repositories = (repository or {}).get("repositories", [])
        if not repositories:
            continue
        images = optional_aws_json(region, ["ecr", "describe-images", "--repository-name", repo], warnings)
        image_details = [
            item for item in (images or {}).get("imageDetails", []) if item.get("imageTags")
        ]
        if not image_details:
            warnings.append(f"ECR repository {repo} exists but has no tagged images.")
            continue
        image_details.sort(key=lambda item: str(item.get("imagePushedAt", "")), reverse=True)
        preferred = next((item for item in image_details if "latest" in item.get("imageTags", [])), image_details[0])
        tags = sorted(preferred.get("imageTags", []), key=lambda tag: (tag != "latest", tag))
        image_uri = f"{repositories[0]['repositoryUri']}:{tags[0]}"
        return image_uri, f"aws:ecr:{repo}:{tags[0]}"
    return None, None


def list_secrets(region: str, warnings: list[str]) -> list[dict[str, Any]]:
    secrets: list[dict[str, Any]] = []
    token: str | None = None
    while True:
        command = ["secretsmanager", "list-secrets", "--max-results", "100"]
        if token:
            command.extend(["--next-token", token])
        response = optional_aws_json(region, command, warnings)
        if not response:
            return secrets
        secrets.extend(response.get("SecretList", []))
        token = response.get("NextToken")
        if not token:
            return secrets


def secret_text(secret: dict[str, Any]) -> str:
    return f"{secret.get('Name', '')} {secret.get('ARN', '')}".lower()


def choose_secret_arn(
    secrets: list[dict[str, Any]],
    candidates: list[tuple[str, ...]],
    *,
    exclude: tuple[str, ...] = (),
) -> tuple[str | None, str | None]:
    scored: list[tuple[int, str, str]] = []
    for secret in secrets:
        text = secret_text(secret)
        if any(term in text for term in exclude):
            continue
        for index, terms in enumerate(candidates):
            if all(term in text for term in terms):
                score = 100 - index
                if "pass4" in text or "conduit-full" in text:
                    score += 10
                scored.append((score, secret.get("Name", ""), secret.get("ARN", "")))
                break
    scored.sort(reverse=True)
    if scored and scored[0][2]:
        return scored[0][2], f"aws:secretsmanager:{scored[0][1]}"
    return None, None


def read_secret_payload(region: str, secret_id: str, warnings: list[str]) -> Any | None:
    response = optional_aws_json(region, ["secretsmanager", "get-secret-value", "--secret-id", secret_id], warnings)
    if not response:
        return None
    raw = response.get("SecretString")
    if not raw and response.get("SecretBinary"):
        try:
            raw = base64.b64decode(response["SecretBinary"]).decode("utf-8")
        except Exception:
            return None
    if not raw:
        return None
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return raw.strip()


def field_from_secret_payload(payload: Any, fields: list[str], *, allow_plain_hex: bool = False) -> str | None:
    if isinstance(payload, str):
        return payload.strip() if allow_plain_hex and is_hex_32(payload.strip()) else None
    if not isinstance(payload, dict):
        return None
    for field in fields:
        value = payload.get(field)
        if isinstance(value, str) and is_hex_32(value.strip()):
            return value.strip()
    return None


def derive_ed25519_public_from_seed(seed_hex: str, warnings: list[str]) -> str | None:
    if not is_hex_32(seed_hex):
        return None
    try:
        from cryptography.hazmat.primitives import serialization
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    except Exception as error:
        warnings.append(
            "Publisher signing secret is readable, but Python cryptography is unavailable; "
            f"cannot derive publisher public key automatically ({error})."
        )
        return None
    try:
        public = Ed25519PrivateKey.from_private_bytes(bytes.fromhex(seed_hex)).public_key()
        return public.public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw,
        ).hex()
    except Exception as error:
        warnings.append(f"Failed to derive publisher public key from signing secret: {error}")
        return None


def resolve_deploy_prereqs(args: argparse.Namespace, *, auto_discover: bool = True) -> dict[str, Any]:
    warnings: list[str] = []
    values: dict[str, str | None] = {}
    sources: dict[str, str] = {}
    for name in DEPLOY_PREREQ_ENV:
        value, source = value_from_arg_or_env(args, name)
        values[name] = value
        if value and source:
            sources[name] = source

    if auto_discover:
        account_id = discover_account_id(args.region, warnings)
        if account_id:
            sources["aws_account_id"] = f"aws:sts:{account_id}"
        if not values["vpc_id"]:
            value, source = discover_vpc(args.region, warnings)
            if value:
                values["vpc_id"] = value
                sources["vpc_id"] = source or "aws:ec2"
        if values["vpc_id"] and (not values["service_subnet_ids"] or not values["database_subnet_ids"]):
            subnet_sets = discover_subnet_sets(args.region, values["vpc_id"], warnings)
            for name, (value, source) in subnet_sets.items():
                if value and not values[name]:
                    values[name] = value
                    sources[name] = source or "aws:ec2.subnets"
        for name, candidates in ECR_REPO_CANDIDATES.items():
            if values[name]:
                continue
            value, source = discover_latest_ecr_image(args.region, candidates, warnings)
            if value:
                values[name] = value
                sources[name] = source or "aws:ecr"

        secrets = list_secrets(args.region, warnings)
        if secrets:
            if not values["publisher_signing_key_secret_arn"]:
                value, source = choose_secret_arn(
                    secrets,
                    [("publisher", "signing"), ("publisher", "sign"), ("publisher", "key")],
                    exclude=("public",),
                )
                if value:
                    values["publisher_signing_key_secret_arn"] = value
                    sources["publisher_signing_key_secret_arn"] = source or "aws:secretsmanager"
            if not values["bridge_signing_seed_secret_arn"]:
                value, source = choose_secret_arn(
                    secrets,
                    [("bridge", "signing", "seed"), ("bridge", "seed"), ("bridge", "signing"), ("bridge", "key")],
                    exclude=("public",),
                )
                if value:
                    values["bridge_signing_seed_secret_arn"] = value
                    sources["bridge_signing_seed_secret_arn"] = source or "aws:secretsmanager"
            if not values["publisher_public_key_hex"]:
                public_arn, public_source = choose_secret_arn(
                    secrets,
                    [("publisher", "public", "key"), ("publisher", "trust"), ("publisher", "public")],
                )
                if public_arn:
                    payload = read_secret_payload(args.region, public_arn, warnings)
                    value = field_from_secret_payload(payload, PUBLIC_KEY_FIELDS, allow_plain_hex=True)
                    if value:
                        values["publisher_public_key_hex"] = value
                        sources["publisher_public_key_hex"] = public_source or "aws:secretsmanager"

        signing_arn = values.get("publisher_signing_key_secret_arn")
        if signing_arn and not values["publisher_public_key_hex"]:
            payload = read_secret_payload(args.region, signing_arn, warnings)
            value = field_from_secret_payload(payload, PUBLIC_KEY_FIELDS)
            if value:
                values["publisher_public_key_hex"] = value
                sources["publisher_public_key_hex"] = "aws:secretsmanager.publisher_signing_secret.public_field"
            else:
                seed = field_from_secret_payload(payload, SIGNING_KEY_FIELDS, allow_plain_hex=True)
                if seed:
                    derived = derive_ed25519_public_from_seed(seed, warnings)
                    if derived:
                        values["publisher_public_key_hex"] = derived
                        sources["publisher_public_key_hex"] = "aws:secretsmanager.publisher_signing_secret.derived"

    missing = [name for name, value in values.items() if not value]
    summary = {
        "schema": DEPLOY_PREREQS_SCHEMA,
        "run_id": args.run_id,
        "region": args.region,
        "ok": not missing,
        "auto_discover": auto_discover,
        "values": values,
        "sources": sources,
        "missing": missing,
        "warnings": warnings,
        "notes": [
            "CLI args override environment variables; environment variables override AWS discovery.",
            "Secret values are never written to this artifact; only ARNs and public key material are recorded.",
            "If publisher_public_key_hex cannot be discovered, store it in a Secrets Manager field named publisher_public_key_hex or pass PUBLISHER_PUBLIC_KEY_HEX.",
        ],
    }
    return summary


def write_prereq_summary(args: argparse.Namespace, summary: dict[str, Any]) -> Path:
    out_dir = artifact_dir(args)
    path = out_dir / "aws-deploy-prerequisites.json"
    write_json(path, summary)
    return path


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
    prereqs = None
    if args.discover_prereqs:
        require_aws()
        prereqs = resolve_deploy_prereqs(args, auto_discover=not args.no_auto_discover_prereqs)
        write_prereq_summary(args, prereqs)
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
        "deploy_inputs": [
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
        "deploy_input_resolution": "aws-pass4-full-topology-up.sh resolves these from CLI args, environment variables, or AWS discovery.",
        "prerequisites": prereqs,
    }
    write_json(out_dir / "aws-public-plan.json", plan)
    print(json.dumps(plan, indent=2, sort_keys=True))


def command_prereqs(args: argparse.Namespace) -> None:
    require_aws()
    summary = resolve_deploy_prereqs(args, auto_discover=not args.no_auto_discover_prereqs)
    write_prereq_summary(args, summary)
    print(json.dumps(summary, indent=2, sort_keys=True))
    if not summary["ok"]:
        raise SystemExit(1)


def command_up(args: argparse.Namespace) -> None:
    require_aws()
    if not args.discover_existing:
        prereqs = resolve_deploy_prereqs(args, auto_discover=not args.no_auto_discover_prereqs)
        prereq_path = write_prereq_summary(args, prereqs)
        missing = [name.replace("_", "-") for name in prereqs["missing"]]
        if missing:
            raise SystemExit(
                "missing required deploy args after env/AWS discovery: "
                + ", ".join(missing)
                + f"\nSee {prereq_path}"
            )
        resolved = prereqs["values"]
        deploy = [
            str(SCRIPT_DIR / "deploy-conduit-full.sh"),
            "--stack-name",
            args.stack_name,
            "--region",
            args.region,
            "--environment",
            args.environment,
            "--vpc-id",
            resolved["vpc_id"],
            "--service-subnet-ids",
            resolved["service_subnet_ids"],
            "--database-subnet-ids",
            resolved["database_subnet_ids"],
            "--authority-image",
            resolved["authority_image"],
            "--receiver-image",
            resolved["receiver_image"],
            "--bridge-image",
            resolved["bridge_image"],
            "--creator-image",
            resolved["creator_image"],
            "--publisher-signing-key-secret-arn",
            resolved["publisher_signing_key_secret_arn"],
            "--bridge-signing-seed-secret-arn",
            resolved["bridge_signing_seed_secret_arn"],
            "--publisher-public-key-hex",
            resolved["publisher_public_key_hex"],
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
            "--postgres-tls-accept-invalid-certs",
            args.postgres_tls_accept_invalid_certs,
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


def add_deploy_prereq_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--vpc-id")
    parser.add_argument("--service-subnet-ids")
    parser.add_argument("--database-subnet-ids")
    parser.add_argument("--authority-image")
    parser.add_argument("--receiver-image")
    parser.add_argument("--bridge-image")
    parser.add_argument("--creator-image")
    parser.add_argument("--publisher-signing-key-secret-arn")
    parser.add_argument("--bridge-signing-seed-secret-arn")
    parser.add_argument("--publisher-public-key-hex")
    parser.add_argument(
        "--no-auto-discover-prereqs",
        action="store_true",
        help="Use only CLI args and environment variables for deploy prerequisites.",
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)

    plan = sub.add_parser("plan")
    add_common(plan)
    add_deploy_prereq_args(plan)
    plan.add_argument("--bridge-count", type=int, default=3)
    plan.add_argument("--discover-prereqs", action="store_true")
    plan.set_defaults(func=command_plan)

    prereqs = sub.add_parser("prereqs")
    add_common(prereqs)
    add_deploy_prereq_args(prereqs)
    prereqs.set_defaults(func=command_prereqs)

    up = sub.add_parser("up")
    add_common(up)
    up.add_argument("--environment", default=os.environ.get("GBN_BRIDGE_ENVIRONMENT", "pass4"))
    up.add_argument("--bridge-count", type=int, default=3)
    up.add_argument("--expires-hours", type=int, default=24)
    up.add_argument("--evidence-bucket", default=os.environ.get("PASS4_MOBILE_EVIDENCE_BUCKET", DEFAULT_BUCKET))
    up.add_argument("--evidence-prefix")
    up.add_argument("--mobile-ingress-cidr", default=os.environ.get("PASS4_MOBILE_INGRESS_CIDR", "0.0.0.0/0"))
    up.add_argument(
        "--postgres-tls-accept-invalid-certs",
        default=os.environ.get("GBN_BRIDGE_POSTGRES_TLS_ACCEPT_INVALID_CERTS", "true"),
    )
    up.add_argument("--discover-existing", action="store_true")
    add_deploy_prereq_args(up)
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
