#!/usr/bin/env bash
set -euo pipefail

GRANT_JSON=""
OUT_DIR=""
CHUNK_SIZE="900"

usage() {
  cat <<'USAGE'
Usage: pass4-s3-grant-qr.sh --grant-json FILE --out-dir DIR [--chunk-size BYTES]

Create standalone-phone S3 evidence grant QR payloads for Pass 4 Phase 5.

Outputs:
  manifest.json
  grant.redacted.json
  chunks/chunk-NNN.json
  chunks/chunk-NNN.txt
  chunks/chunk-NNN.png when qrencode is installed
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --grant-json) GRANT_JSON="$2"; shift 2 ;;
    --out-dir) OUT_DIR="$2"; shift 2 ;;
    --chunk-size) CHUNK_SIZE="$2"; shift 2 ;;
    --help|-h) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$GRANT_JSON" ]] || { echo "--grant-json is required" >&2; exit 2; }
[[ -n "$OUT_DIR" ]] || { echo "--out-dir is required" >&2; exit 2; }
[[ "$CHUNK_SIZE" =~ ^[0-9]+$ ]] || { echo "--chunk-size must be an integer" >&2; exit 2; }

python3 - "$GRANT_JSON" "$OUT_DIR" "$CHUNK_SIZE" <<'PY'
import base64
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

grant_path = Path(sys.argv[1])
out_dir = Path(sys.argv[2])
chunk_size = int(sys.argv[3])
if chunk_size < 128:
    raise SystemExit("--chunk-size must be at least 128")

raw = grant_path.read_text(encoding="utf-8")
grant = json.loads(raw)
if grant.get("upload_mode") != "s3_presigned_put":
    raise SystemExit("grant upload_mode must be s3_presigned_put")
if not str(grant.get("object_key", "")).startswith("mobile-evidence/"):
    raise SystemExit("grant object_key must stay under mobile-evidence/")
for forbidden in (
    "aws_access_key_id",
    "aws_secret_access_key",
    "aws_session_token",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
):
    if forbidden in grant:
        raise SystemExit(f"raw credential field is forbidden: {forbidden}")

canonical = json.dumps(grant, separators=(",", ":"), sort_keys=True)
sha = hashlib.sha256(canonical.encode("utf-8")).hexdigest()
grant_id = (
    str(grant.get("object_key", "mobile-evidence"))
    .replace("/", "-")
    .replace(".", "-")[:96]
)
chunks = [
    canonical[idx : idx + chunk_size]
    for idx in range(0, len(canonical), chunk_size)
]
out_dir.mkdir(parents=True, exist_ok=True)
chunk_dir = out_dir / "chunks"
chunk_dir.mkdir(parents=True, exist_ok=True)

qrencode = shutil.which("qrencode")
payload_paths = []
for idx, chunk in enumerate(chunks, start=1):
    payload = {
        "type": "gbn.s3_grant.chunk",
        "version": 1,
        "grant_id": grant_id,
        "index": idx,
        "count": len(chunks),
        "sha256": sha,
        "data": base64.urlsafe_b64encode(chunk.encode("utf-8")).decode("ascii").rstrip("="),
    }
    payload_json = json.dumps(payload, separators=(",", ":"), sort_keys=True)
    json_path = chunk_dir / f"chunk-{idx:03d}.json"
    txt_path = chunk_dir / f"chunk-{idx:03d}.txt"
    png_path = chunk_dir / f"chunk-{idx:03d}.png"
    json_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    txt_path.write_text(payload_json + "\n", encoding="utf-8")
    payload_paths.append(str(txt_path))
    if qrencode:
        subprocess.run(["qrencode", "-o", str(png_path), payload_json], check=True)

redacted = dict(grant)
if redacted.get("presigned_put_url"):
    redacted["presigned_put_url"] = "redacted"
manifest = {
    "schema": "veritas.pass4.s3_grant_qr.v1",
    "grant_id": grant_id,
    "bucket": grant.get("bucket"),
    "object_key": grant.get("object_key"),
    "expires_at_ms": grant.get("expires_at_ms"),
    "sha256": sha,
    "chunk_count": len(chunks),
    "chunk_size": chunk_size,
    "qr_png_generated": bool(qrencode),
    "payloads": payload_paths,
}
(out_dir / "grant.redacted.json").write_text(json.dumps(redacted, indent=2, sort_keys=True) + "\n", encoding="utf-8")
(out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
print(json.dumps(manifest, indent=2, sort_keys=True))
PY
