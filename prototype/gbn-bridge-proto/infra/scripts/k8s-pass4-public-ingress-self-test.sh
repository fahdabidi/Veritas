#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "$SCRIPT_DIR/../.." && pwd)"

if [[ "${VERITAS_ALLOW_NON_WSL:-0}" != "1" ]]; then
  uname -a | grep -i microsoft >/dev/null || {
    echo "Pass 4 tooling requires WSL2 Ubuntu" >&2
    exit 1
  }
fi

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/pass4-public-ingress-test.XXXXXX")"
trap 'rm -rf "$tmp_dir"' EXIT

valid_config="$ROOT_DIR/infra/pass4/public-ingress/run-profile.local-k8s-public.example.json"
artifact_dir="$tmp_dir/artifacts"

"$SCRIPT_DIR/k8s-pass4-public-ingress-prepare.sh" \
  --config "$valid_config" \
  --run-id pass4-public-ingress-self-test \
  --artifact-dir "$artifact_dir" \
  --skip-k8s-check \
  --skip-network-checks

"$SCRIPT_DIR/k8s-pass4-public-ingress-verify.sh" \
  --artifact-dir "$artifact_dir" \
  --require-no-public-admin \
  --require-hostcreator-qr \
  --require-public-dht-endpoints \
  --skip-network-checks

"$SCRIPT_DIR/k8s-pass4-public-ingress-down.sh" \
  --artifact-dir "$artifact_dir" \
  --run-id pass4-public-ingress-self-test

python3 - "$artifact_dir" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
required = [
    "public_endpoint_map.json",
    "publisher_public_dht_snapshot.json",
    "hostcreator_bootstrap_qr.png",
    "hostcreator_bootstrap_seed.redacted.json",
    "public_ingress_evidence.json",
    "public_reachability_transcript.txt",
    "admin_denial_transcript.txt",
    "teardown_transcript.txt",
]
missing = [name for name in required if not (root / name).exists()]
if missing:
    raise SystemExit(f"missing expected artifacts: {missing}")

seed = json.loads((root / "hostcreator_bootstrap_seed.redacted.json").read_text())
text = json.dumps(seed).lower()
for forbidden in ("publisher-authority", "publisher-receiver", "exit-bridge", "/v1/admin", "localhost", ".svc"):
    if forbidden in text:
        raise SystemExit(f"forbidden seed content found: {forbidden}")

teardown = json.loads((root / "public_endpoint_map.invalidated.json").read_text())
if teardown.get("invalidated") is not True:
    raise SystemExit("endpoint map was not invalidated")
PY

invalid_private="$tmp_dir/invalid-private.json"
python3 - "$valid_config" "$invalid_private" <<'PY'
import json
import pathlib
import sys

source = pathlib.Path(sys.argv[1])
target = pathlib.Path(sys.argv[2])
data = json.loads(source.read_text())
data["endpoints"][0]["public_host"] = "127.0.0.1"
target.write_text(json.dumps(data, indent=2) + "\n")
PY

if "$SCRIPT_DIR/k8s-pass4-public-ingress-prepare.sh" \
  --config "$invalid_private" \
  --run-id pass4-public-ingress-invalid-private \
  --artifact-dir "$tmp_dir/invalid-private-artifacts" \
  --skip-k8s-check \
  --skip-network-checks >"$tmp_dir/invalid-private.log" 2>&1; then
  echo "expected private host validation to fail" >&2
  exit 1
fi

invalid_admin="$tmp_dir/invalid-admin.json"
python3 - "$valid_config" "$invalid_admin" <<'PY'
import json
import pathlib
import sys

source = pathlib.Path(sys.argv[1])
target = pathlib.Path(sys.argv[2])
data = json.loads(source.read_text())
data["endpoints"][0]["tcp_port"] = 9090
target.write_text(json.dumps(data, indent=2) + "\n")
PY

if "$SCRIPT_DIR/k8s-pass4-public-ingress-prepare.sh" \
  --config "$invalid_admin" \
  --run-id pass4-public-ingress-invalid-admin \
  --artifact-dir "$tmp_dir/invalid-admin-artifacts" \
  --skip-k8s-check \
  --skip-network-checks >"$tmp_dir/invalid-admin.log" 2>&1; then
  echo "expected admin port validation to fail" >&2
  exit 1
fi

echo "Pass 4 public ingress self-test passed"
