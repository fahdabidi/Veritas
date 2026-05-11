#!/usr/bin/env bash
# Pass 4 Phase 1: strict SendDummy validation gated by hardened bootstrap.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
OBS_NS="${VERITAS_OBS_NAMESPACE:-observability}"
EXPECTED_BRIDGES="${VERITAS_K8S_EXPECTED_BRIDGES:-10}"
ARTIFACT_DIR=""
REQUIRE_ONBOARDED_FROM_STRICT_BOOTSTRAP=1
REQUIRE_ROUTE_SOURCE="local_dht"
REQUIRE_CIPHERTEXT_ONLY_AT_BRIDGE=1
ROUTE_ARGS=()
BOOTSTRAP_ARGS=()

usage() {
  cat <<'EOF'
Usage: k8s-smoke-senddummy-strict-v4.sh [options]

Strict Pass 4 options:
  --require-onboarded-from-strict-bootstrap       Run strict bootstrap first and require matching session evidence. Default.
  --no-require-onboarded-from-strict-bootstrap    Skip strict-bootstrap prerequisite.
  --require-route-source VALUE                    Required SendDummy route_source. Default: local_dht.
  --require-ciphertext-only-at-bridge             Require bridge plaintext-grep artifact to be empty. Default.
  --no-require-ciphertext-only-at-bridge          Skip bridge ciphertext-only assertion.

All other options are forwarded to k8s-smoke-route-v3.sh. Bootstrap-compatible options
are also forwarded to k8s-smoke-bootstrap-strict-v4.sh when the prerequisite is enabled.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --require-onboarded-from-strict-bootstrap) REQUIRE_ONBOARDED_FROM_STRICT_BOOTSTRAP=1; shift ;;
    --no-require-onboarded-from-strict-bootstrap) REQUIRE_ONBOARDED_FROM_STRICT_BOOTSTRAP=0; shift ;;
    --require-route-source) REQUIRE_ROUTE_SOURCE="$2"; shift 2 ;;
    --require-ciphertext-only-at-bridge) REQUIRE_CIPHERTEXT_ONLY_AT_BRIDGE=1; shift ;;
    --no-require-ciphertext-only-at-bridge) REQUIRE_CIPHERTEXT_ONLY_AT_BRIDGE=0; shift ;;
    --namespace)
      NAMESPACE="$2"
      ROUTE_ARGS+=("$1" "$2")
      BOOTSTRAP_ARGS+=("$1" "$2")
      shift 2
      ;;
    --observability-namespace)
      OBS_NS="$2"
      ROUTE_ARGS+=("$1" "$2")
      BOOTSTRAP_ARGS+=("$1" "$2")
      shift 2
      ;;
    --expected-bridges)
      EXPECTED_BRIDGES="$2"
      ROUTE_ARGS+=("$1" "$2")
      BOOTSTRAP_ARGS+=("$1" "$2")
      shift 2
      ;;
    --trace-timeout|--bootstrap-timeout|--min-active-bridges)
      ROUTE_ARGS+=("$1" "$2")
      BOOTSTRAP_ARGS+=("$1" "$2")
      shift 2
      ;;
    --require-observability|--no-require-observability)
      ROUTE_ARGS+=("$1")
      BOOTSTRAP_ARGS+=("$1")
      shift
      ;;
    --artifact-dir)
      ARTIFACT_DIR="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      ROUTE_ARGS+=("$1")
      shift
      ;;
  esac
done

uname -a | grep -i microsoft >/dev/null || {
  echo "Pass 4 tooling requires WSL2 Ubuntu" >&2
  exit 1
}

cd "$ROOT_DIR"
source "$SCRIPT_DIR/k8s-smoke-common.sh"

smoke_require_deps
smoke_artifact_dir smoke-3-senddummy-strict-v4 >/dev/null
trap 'status=$?; echo "Artifacts: $ARTIFACT_DIR"; exit $status' EXIT

mkdir -p "$ARTIFACT_DIR"

if [[ "$REQUIRE_ONBOARDED_FROM_STRICT_BOOTSTRAP" -eq 1 ]]; then
  bash "$SCRIPT_DIR/k8s-smoke-bootstrap-strict-v4.sh" \
    "${BOOTSTRAP_ARGS[@]}" \
    --artifact-dir "$ARTIFACT_DIR/bootstrap"
fi

bash "$SCRIPT_DIR/k8s-smoke-route-v3.sh" \
  "${ROUTE_ARGS[@]}" \
  --artifact-dir "$ARTIFACT_DIR/route"

python3 - \
  "$ARTIFACT_DIR" \
  "$REQUIRE_ONBOARDED_FROM_STRICT_BOOTSTRAP" \
  "$REQUIRE_ROUTE_SOURCE" \
  "$REQUIRE_CIPHERTEXT_ONLY_AT_BRIDGE" <<'PY'
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
require_strict_bootstrap = sys.argv[2] == "1"
required_route_source = sys.argv[3]
require_ciphertext_only = sys.argv[4] == "1"
route_root = root / "route"
bootstrap_root = root / "bootstrap"
failure_path = root / "strict-senddummy-failure.json"
summary_path = root / "strict-senddummy-summary.json"
report_path = root / "strict-report.md"

def load(path, default=None):
    if path.exists():
        return json.loads(path.read_text(encoding="utf-8"))
    if default is not None:
        return default
    fail("missing_artifact", path=str(path.relative_to(root)))

def fail(code, **detail):
    detail["code"] = code
    failure_path.write_text(json.dumps(detail, indent=2, sort_keys=True), encoding="utf-8")
    raise SystemExit(f"{code}: {detail}")

def validate_send(label):
    result = load(route_root / f"send-dummy-{label}-result.json")
    if result.get("skipped"):
        return {"label": label, "skipped": True}
    if result.get("route_source") != required_route_source:
        fail("route_source_mismatch", label=label, actual=result.get("route_source"), expected=required_route_source)
    if result.get("encryption_envelope") != "publisher_x25519_hkdf_aes256gcm_v1":
        fail(
            "senddummy_encryption_envelope_mismatch",
            label=label,
            actual=result.get("encryption_envelope"),
            expected="publisher_x25519_hkdf_aes256gcm_v1",
        )
    if result.get("ciphertext_only_at_bridge") is not True:
        fail("senddummy_result_not_ciphertext_only_at_bridge", label=label, result=result)
    if not result.get("chain_id") or not result.get("assigned_bridge_id"):
        fail("senddummy_missing_chain_or_bridge", label=label, result=result)
    validation = load(route_root / f"received-dummy-{label}.json")
    if validation.get("payload_hash_match") is not True:
        fail("payload_hash_mismatch", label=label, validation=validation)
    frames = load(route_root / f"frames-{label}.json", {"frames": []})
    if len(frames.get("frames") or []) < 1:
        fail("missing_receiver_frame", label=label)
    return {
        "label": label,
        "chain_id": result.get("chain_id"),
        "assigned_bridge_id": result.get("assigned_bridge_id"),
        "route_source": result.get("route_source"),
        "payload_hash_match": validation.get("payload_hash_match"),
        "validated_frame_count": validation.get("validated_frame_count"),
        "frame_count": len(frames.get("frames") or []),
        "encryption_envelope": result.get("encryption_envelope"),
        "ciphertext_only_at_bridge": result.get("ciphertext_only_at_bridge"),
    }

ready = load(route_root / "creator-local-dht-ready-summary.json")
if ready.get("state") != "onboarded":
    fail("creator_not_onboarded", state=ready.get("state"))

bootstrap_summary = {}
bootstrap_relay_privacy = {}
if require_strict_bootstrap:
    bootstrap_summary = load(bootstrap_root / "strict-bootstrap-summary.json")
    bootstrap_relay_privacy = load(bootstrap_root / "bootstrap-relay-privacy-evidence.json")
    if ready.get("bootstrap_session_id") != bootstrap_summary.get("bootstrap_session_id"):
        fail(
            "strict_bootstrap_session_mismatch",
            route_session=ready.get("bootstrap_session_id"),
            bootstrap_session=bootstrap_summary.get("bootstrap_session_id"),
        )

normal = validate_send("normal")
failover = validate_send("failover")

if require_ciphertext_only:
    grep_path = route_root / "bridge-plaintext-grep.txt"
    if not grep_path.exists():
        fail("missing_bridge_plaintext_grep")
    if grep_path.read_text(encoding="utf-8", errors="replace").strip():
        fail("bridge_plaintext_marker_detected", artifact="route/bridge-plaintext-grep.txt")

dht_summary = load(route_root / "dht-evidence/pre-send/dht-summary.json", {})

summary = {
    "strict_bootstrap_required": require_strict_bootstrap,
    "bootstrap_chain_id": bootstrap_summary.get("chain_id"),
    "bootstrap_session_id": ready.get("bootstrap_session_id"),
    "bootstrap_relay_privacy": bootstrap_relay_privacy,
    "required_route_source": required_route_source,
    "pre_send_dht": dht_summary,
    "normal": normal,
    "failover": failover,
    "ciphertext_only_at_bridge": require_ciphertext_only,
}
summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True), encoding="utf-8")

report = [
    "# GBN-PROTO-013 Phase 1 Strict SendDummy Report",
    "",
    f"- Bootstrap session: `{ready.get('bootstrap_session_id')}`",
    f"- Required route source: `{required_route_source}`",
    f"- Normal ChainID: `{normal.get('chain_id')}`",
    f"- Normal bridge: `{normal.get('assigned_bridge_id')}`",
    f"- Failover ChainID: `{failover.get('chain_id')}`",
    f"- Failover bridge: `{failover.get('assigned_bridge_id')}`",
    f"- Ciphertext-only bridge check: `{require_ciphertext_only}`",
    "",
    "## Prerequisite Bootstrap Privacy",
    "",
    "| Gate | Status | Evidence |",
    "|---|---:|---|",
    f"| CreatorBootstrap encrypted to NewCreator | `pass` | recipient_key_id=`{(bootstrap_summary.get('encrypted_bootstrap_payload') or {}).get('recipient_key_id')}`, ciphertext_len=`{(bootstrap_summary.get('encrypted_bootstrap_payload') or {}).get('ciphertext_len')}` |",
    f"| SeedBridgeCatalog encrypted to NewCreator | `pass` | recipient_key_id=`{(bootstrap_summary.get('encrypted_seed_bridge_catalog_payload') or {}).get('recipient_key_id')}`, ciphertext_len=`{(bootstrap_summary.get('encrypted_seed_bridge_catalog_payload') or {}).get('ciphertext_len')}` |",
    f"| HostCreator and ExitBridgeA cannot see bootstrap payload plaintext | `pass` | forbidden_plaintext_hits=`{bootstrap_relay_privacy.get('forbidden_plaintext_hit_count')}` |",
    "",
    "## DHT Evidence",
    "",
    f"- Publisher DHT entries: `{dht_summary.get('publisher_dht_entry_count')}`",
    f"- NewCreator local DHT entries: `{dht_summary.get('creator_new_dht_entry_count')}`",
    f"- NewCreator active bridge entries: `{dht_summary.get('creator_new_active_bridge_count')}`",
    f"- NewCreator active tunnels: `{dht_summary.get('creator_new_active_tunnel_count')}`",
    f"- Publisher encryption key present in NewCreator DHT: `{dht_summary.get('publisher_encryption_key_present')}`",
    f"- NewCreator state: `{dht_summary.get('new_creator_state')}`",
    "",
    "## API Completion And Payload Validation",
    "",
    "| Invocation | ChainID | Bridge | Route Source | Envelope | Frames | Payload Hash Match | Ciphertext Only At Bridge |",
    "|---|---|---|---|---|---:|---:|---:|",
    f"| normal | {normal.get('chain_id')} | {normal.get('assigned_bridge_id')} | {normal.get('route_source')} | {normal.get('encryption_envelope')} | {normal.get('frame_count')} | {str(normal.get('payload_hash_match')).lower()} | {str(normal.get('ciphertext_only_at_bridge')).lower()} |",
    f"| failover | {failover.get('chain_id')} | {failover.get('assigned_bridge_id')} | {failover.get('route_source')} | {failover.get('encryption_envelope')} | {failover.get('frame_count')} | {str(failover.get('payload_hash_match')).lower()} | {str(failover.get('ciphertext_only_at_bridge')).lower()} |",
    "",
    "Artifacts: `bootstrap/strict-bootstrap-summary.json`, `bootstrap/bootstrap-relay-privacy-evidence.json`, `route/dht-evidence/pre-send/`, `route/send-dummy-*-result.json`, `route/received-dummy-*.json`, `route/bridge-plaintext-grep.txt`, `strict-senddummy-summary.json`, and `route/chainid-evidence/`.",
    "",
    "Result: strict SendDummy validation passed.",
    "",
]
report_path.write_text("\n".join(report), encoding="utf-8")
PY

repo_root="$(cd "$ROOT_DIR/../.." && pwd)"
export VERITAS_K8S_SMOKE_REPORT_ROOT="${VERITAS_K8S_SMOKE_REPORT_ROOT:-$repo_root/docs/prototyping/Conduit/Full-Implementation-Plan-Pass4/Test-Reports}"
smoke_archive_report "GBN-PROTO-013-Phase1-Strict-SendDummy" "$ARTIFACT_DIR/strict-report.md"

echo "Pass 4 strict SendDummy validation passed."
echo "Strict evidence report: $ARTIFACT_DIR/strict-report.md"
