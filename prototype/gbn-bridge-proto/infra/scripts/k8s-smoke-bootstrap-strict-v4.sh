#!/usr/bin/env bash
# Pass 4 Phase 1: strict bootstrap hardening validation over the Pass 3 k8s smoke.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
OBS_NS="${VERITAS_OBS_NAMESPACE:-observability}"
EXPECTED_BRIDGES="${VERITAS_K8S_EXPECTED_BRIDGES:-10}"
ARTIFACT_DIR=""
REQUIRE_ENCRYPTED_BOOTSTRAP_PAYLOAD=1
REQUIRE_SEED_BRIDGE_CATALOG_HANDOFF=1
REQUIRE_REAL_FANOUT_PROGRESS=1
PASS_THROUGH=()

usage() {
  cat <<'EOF'
Usage: k8s-smoke-bootstrap-strict-v4.sh [options]

Strict Pass 4 options:
  --require-encrypted-bootstrap-payload       Require encrypted CreatorBootstrap evidence. Default.
  --no-require-encrypted-bootstrap-payload    Skip encrypted CreatorBootstrap evidence assertion.
  --require-seed-bridge-catalog-handoff       Require encrypted SeedBridgeCatalog evidence. Default.
  --no-require-seed-bridge-catalog-handoff    Skip encrypted SeedBridgeCatalog evidence assertion.
  --require-real-fanout-progress              Require per-bridge fanout progress before completion. Default.
  --no-require-real-fanout-progress           Skip per-bridge progress assertion.

All other options are forwarded to k8s-smoke-discovery-v3.sh.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --require-encrypted-bootstrap-payload) REQUIRE_ENCRYPTED_BOOTSTRAP_PAYLOAD=1; shift ;;
    --no-require-encrypted-bootstrap-payload) REQUIRE_ENCRYPTED_BOOTSTRAP_PAYLOAD=0; shift ;;
    --require-seed-bridge-catalog-handoff) REQUIRE_SEED_BRIDGE_CATALOG_HANDOFF=1; shift ;;
    --no-require-seed-bridge-catalog-handoff) REQUIRE_SEED_BRIDGE_CATALOG_HANDOFF=0; shift ;;
    --require-real-fanout-progress) REQUIRE_REAL_FANOUT_PROGRESS=1; shift ;;
    --no-require-real-fanout-progress) REQUIRE_REAL_FANOUT_PROGRESS=0; shift ;;
    --expected-bridges)
      EXPECTED_BRIDGES="$2"
      PASS_THROUGH+=("$1" "$2")
      shift 2
      ;;
    --namespace)
      NAMESPACE="$2"
      PASS_THROUGH+=("$1" "$2")
      shift 2
      ;;
    --observability-namespace)
      OBS_NS="$2"
      PASS_THROUGH+=("$1" "$2")
      shift 2
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
      PASS_THROUGH+=("$1")
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
smoke_artifact_dir smoke-2-bootstrap-strict-v4 >/dev/null
trap 'status=$?; echo "Artifacts: $ARTIFACT_DIR"; exit $status' EXIT

bash "$SCRIPT_DIR/k8s-smoke-discovery-v3.sh" "${PASS_THROUGH[@]}" --artifact-dir "$ARTIFACT_DIR"

python3 - \
  "$ARTIFACT_DIR" \
  "$EXPECTED_BRIDGES" \
  "$REQUIRE_ENCRYPTED_BOOTSTRAP_PAYLOAD" \
  "$REQUIRE_SEED_BRIDGE_CATALOG_HANDOFF" \
  "$REQUIRE_REAL_FANOUT_PROGRESS" <<'PY'
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
expected_bridges = int(sys.argv[2])
require_encrypted = sys.argv[3] == "1"
require_catalog = sys.argv[4] == "1"
require_fanout = sys.argv[5] == "1"
failure_path = root / "strict-bootstrap-failure.json"
summary_path = root / "strict-bootstrap-summary.json"
report_path = root / "strict-report.md"

def load(name):
    return json.loads((root / name).read_text(encoding="utf-8"))

def fail(code, **detail):
    detail["code"] = code
    failure_path.write_text(json.dumps(detail, indent=2, sort_keys=True), encoding="utf-8")
    raise SystemExit(f"{code}: {detail}")

def norm(value):
    return str(value or "").lower()

seed_result = load("seed-new-creator-result.json")
session_response = load("bootstrap-session.json")
assertion_summary = load("bootstrap-assertion-summary.json")
local_dht = load("local-dht-final.json")
evidence = seed_result.get("strict_bootstrap_evidence")
session = session_response.get("bootstrap_session") or {}
chain_id = seed_result.get("chain_id")
bootstrap_session_id = seed_result.get("bootstrap_session_id")

if not evidence:
    fail("missing_strict_bootstrap_evidence")
if not chain_id or not bootstrap_session_id:
    fail("missing_seed_result_chain_or_session", chain_id=chain_id, bootstrap_session_id=bootstrap_session_id)
if evidence.get("initial_plaintext_bridge_set_present") is not False:
    fail("initial_plaintext_bridge_set_present", value=evidence.get("initial_plaintext_bridge_set_present"))

def assert_payload(name, expected_kind):
    payload = evidence.get(name) or {}
    if norm(payload.get("payload_kind")) != expected_kind:
        fail("payload_kind_mismatch", payload=name, actual=payload.get("payload_kind"), expected=expected_kind)
    if payload.get("chain_id") != chain_id:
        fail("payload_chain_mismatch", payload=name, actual=payload.get("chain_id"), expected=chain_id)
    if payload.get("bootstrap_session_id") != bootstrap_session_id:
        fail(
            "payload_session_mismatch",
            payload=name,
            actual=payload.get("bootstrap_session_id"),
            expected=bootstrap_session_id,
        )
    if int(payload.get("ciphertext_len") or 0) <= 0:
        fail("payload_missing_ciphertext", payload=name)
    if int(payload.get("auth_tag_len") or 0) != 16:
        fail("payload_auth_tag_len_mismatch", payload=name, actual=payload.get("auth_tag_len"))
    if len(payload.get("plaintext_sha256") or []) != 32:
        fail("payload_hash_len_mismatch", payload=name, actual=len(payload.get("plaintext_sha256") or []))
    return payload

bootstrap_payload = None
catalog_payload = None
if require_encrypted:
    bootstrap_payload = assert_payload("encrypted_bootstrap_payload", "creator_bootstrap")
if require_catalog:
    catalog_payload = assert_payload("encrypted_seed_bridge_catalog_payload", "seed_bridge_catalog")

bridge_ids = session.get("bridge_ids") or []
if len(bridge_ids) != expected_bridges:
    fail("session_bridge_count_mismatch", actual=len(bridge_ids), expected=expected_bridges)
if evidence.get("seed_catalog_bridge_count") != expected_bridges:
    fail("seed_catalog_bridge_count_mismatch", actual=evidence.get("seed_catalog_bridge_count"), expected=expected_bridges)
if sorted(evidence.get("seed_catalog_bridge_ids") or []) != sorted(bridge_ids):
    fail("seed_catalog_bridge_ids_mismatch", evidence=sorted(evidence.get("seed_catalog_bridge_ids") or []), session=sorted(bridge_ids))

state = norm(session.get("state"))
if state != "completed":
    fail("bootstrap_session_not_completed", state=session.get("state"))
if assertion_summary.get("state") != "onboarded":
    fail("local_dht_not_onboarded", state=assertion_summary.get("state"))
if local_dht.get("self_onboarding_state") != "onboarded":
    fail("local_dht_final_not_onboarded", state=local_dht.get("self_onboarding_state"))

progress = session.get("progress_events") or []
for event in progress:
    if event.get("chain_id") != chain_id:
        fail("progress_chain_mismatch", event=event)
    if event.get("bootstrap_session_id") != bootstrap_session_id:
        fail("progress_session_mismatch", event=event)

seed_bridge_id = session.get("seed_bridge_id") or evidence.get("seed_bridge_id")
seed_payload_reporters = {
    event.get("reporter_id")
    for event in progress
    if norm(event.get("stage")) == "seed_payload_received"
}
seed_tunnel_reporters = {
    event.get("reporter_id")
    for event in progress
    if norm(event.get("stage")) == "seed_tunnel_established"
}
if seed_bridge_id not in seed_payload_reporters:
    fail("seed_payload_progress_missing", seed_bridge_id=seed_bridge_id, reporters=sorted(seed_payload_reporters))
if seed_bridge_id not in seed_tunnel_reporters or "new-creator" not in seed_tunnel_reporters:
    fail("seed_tunnel_progress_missing", seed_bridge_id=seed_bridge_id, reporters=sorted(seed_tunnel_reporters))

fanout_reporters = {
    event.get("reporter_id")
    for event in progress
    if norm(event.get("stage")) == "bridge_tunnel_established"
}
missing_fanout = sorted(set(bridge_ids) - fanout_reporters)
if require_fanout and missing_fanout:
    fail("bridge_tunnel_progress_missing", bridge_ids=missing_fanout)
if require_fanout and not any(norm(event.get("stage")) == "bridge_set_complete" for event in progress):
    fail("bridge_set_complete_progress_missing")

summary = {
    "chain_id": chain_id,
    "bootstrap_session_id": bootstrap_session_id,
    "new_creator_encryption_pub_key_present": bool(evidence.get("new_creator_encryption_pub_key")),
    "encrypted_bootstrap_payload": bootstrap_payload,
    "encrypted_seed_bridge_catalog_payload": catalog_payload,
    "seed_bridge_id": seed_bridge_id,
    "bridge_count": len(bridge_ids),
    "fanout_progress_count": len(fanout_reporters),
    "progress_event_count": len(progress),
    "local_dht_state": local_dht.get("self_onboarding_state"),
}
summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True), encoding="utf-8")

report = [
    "# GBN-PROTO-013 Phase 1 Strict Bootstrap Report",
    "",
    f"- ChainID: `{chain_id}`",
    f"- Bootstrap session: `{bootstrap_session_id}`",
    f"- Seed bridge: `{seed_bridge_id}`",
    f"- Bridge count: `{len(bridge_ids)}`",
    f"- Per-bridge fanout progress events: `{len(fanout_reporters)}`",
    f"- Initial plaintext bridge set present: `{evidence.get('initial_plaintext_bridge_set_present')}`",
    f"- CreatorBootstrap ciphertext bytes: `{(bootstrap_payload or {}).get('ciphertext_len')}`",
    f"- SeedBridgeCatalog ciphertext bytes: `{(catalog_payload or {}).get('ciphertext_len')}`",
    "",
    "Artifacts: `seed-new-creator-result.json`, `bootstrap-session.json`, `local-dht-final.json`, `strict-bootstrap-summary.json`, `pod-log-events.json`, and `pod-logs/`.",
    "",
    "Result: strict bootstrap hardening validation passed.",
    "",
]
report_path.write_text("\n".join(report), encoding="utf-8")
PY

repo_root="$(cd "$ROOT_DIR/../.." && pwd)"
export VERITAS_K8S_SMOKE_REPORT_ROOT="${VERITAS_K8S_SMOKE_REPORT_ROOT:-$repo_root/docs/prototyping/Conduit/Full-Implementation-Plan-Pass4/Test-Reports}"
smoke_archive_report "GBN-PROTO-013-Phase1-Strict-Bootstrap" "$ARTIFACT_DIR/strict-report.md"

echo "Pass 4 strict bootstrap validation passed."
echo "Strict evidence report: $ARTIFACT_DIR/strict-report.md"
