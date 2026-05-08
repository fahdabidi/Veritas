#!/usr/bin/env bash
# Validate Conduit local-k8s discovery state without sending payload frames.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
OBS_NS="${VERITAS_OBS_NAMESPACE:-observability}"
EXPECTED_BRIDGES="${VERITAS_K8S_EXPECTED_BRIDGES:-3}"
ADMIN_PORT="${VERITAS_K8S_ADMIN_PORT:-9090}"
REQUIRE_OBSERVABILITY=0
ARTIFACT_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --namespace) NAMESPACE="$2"; shift 2 ;;
    --observability-namespace) OBS_NS="$2"; shift 2 ;;
    --expected-bridges) EXPECTED_BRIDGES="$2"; shift 2 ;;
    --require-observability) REQUIRE_OBSERVABILITY=1; shift ;;
    --artifact-dir) ARTIFACT_DIR="$2"; shift 2 ;;
    *) echo "ERROR: unknown argument '$1'." >&2; exit 2 ;;
  esac
done

cd "$ROOT_DIR"
source "$SCRIPT_DIR/k8s-smoke-common.sh"
smoke_require_deps
smoke_artifact_dir discovery >/dev/null
trap 'status=$?; if [[ $status -ne 0 ]]; then smoke_collect_diagnostics; fi; smoke_stop_observability; echo "Artifacts: $ARTIFACT_DIR"; exit $status' EXIT

smoke_check_rollouts
smoke_discover_nodes
smoke_check_admin_metrics
smoke_wait_for_bridge_registry

if [[ "$REQUIRE_OBSERVABILITY" == "1" ]]; then
  smoke_start_observability
fi

echo "Validating authority bridge registry against running bridge pods..."
python3 - "$ARTIFACT_DIR/bridges.json" "${BRIDGE_PODS[@]}" <<'PY'
import json
import sys
import time

path, *expected = sys.argv[1:]
data = json.load(open(path))
records = {item["bridge_id"]: item for item in data.get("bridges", [])}
now_ms = int(time.time() * 1000)
missing = [pod for pod in expected if pod not in records]
if missing:
    raise SystemExit(f"missing bridge record(s): {', '.join(missing)}")
bad = []
for pod in expected:
    record = records[pod]
    if record.get("revoked_reason") is not None:
        bad.append(f"{pod}: revoked_reason={record.get('revoked_reason')}")
    if record.get("current_lease", {}).get("lease_expiry_ms", 0) < now_ms:
        bad.append(f"{pod}: expired lease")
    if not record.get("ingress_endpoints"):
        bad.append(f"{pod}: no ingress_endpoints")
    if "session_relay" not in record.get("capabilities", []):
        bad.append(f"{pod}: missing session_relay capability")
if bad:
    raise SystemExit("; ".join(bad))
PY

echo "Running discovery probes from every node..."
: >"$ARTIFACT_DIR/discovery-probes.jsonl"
declare -a CHAIN_IDS=()
for check in "${NODE_CHECKS[@]}"; do
  pod="${check%%:*}"
  rest="${check#*:}"
  container="${rest%%:*}"
  role="${check##*:}"
  result="$(smoke_admin_curl "$pod" "$container" POST /v1/admin/discovery-probe '{}')"
  result_file="$ARTIFACT_DIR/discovery-${pod}.json"
  printf '%s\n' "$result" >"$result_file"
  chain_id="$(printf '%s' "$result" | smoke_json_field chain_id)"
  if [[ -z "$chain_id" ]]; then
    echo "ERROR: discovery-probe on $pod did not return chain_id." >&2
    cat "$result_file" >&2
    exit 1
  fi
  CHAIN_IDS+=("$chain_id")
  python3 - "$result_file" "$EXPECTED_BRIDGES" "${BRIDGE_PODS[@]}" <<'PY'
import json
import sys

path, expected_count, *bridge_pods = sys.argv[1:]
data = json.load(open(path))
known = set(data.get("known_bridge_ids", []))
missing = [pod for pod in bridge_pods if pod not in known]
if int(data.get("known_bridge_count", 0)) < int(expected_count):
    raise SystemExit(f"known_bridge_count too low: {data.get('known_bridge_count')}")
if missing:
    raise SystemExit(f"discovery result missing bridge(s): {', '.join(missing)}")
if data.get("assigned_bridge_id") not in known:
    raise SystemExit(f"assigned_bridge_id {data.get('assigned_bridge_id')!r} not in known bridge set")
if not data.get("bridge_address"):
    raise SystemExit("bridge_address is empty")
PY
  frames="$(
    smoke_admin_curl "$AUTHORITY_POD" publisher-authority GET "/v1/admin/frames?chain_id=${chain_id}" |
      python3 -c 'import json,sys; print(len(json.load(sys.stdin).get("frames", [])))'
  )"
  if [[ "$frames" -ne 0 ]]; then
    smoke_fail "discovery-probe chain_id=$chain_id unexpectedly persisted $frames frame(s)."
  fi
  python3 - "$pod" "$container" "$role" "$result_file" >>"$ARTIFACT_DIR/discovery-probes.jsonl" <<'PY'
import json
import sys

pod, container, role, path = sys.argv[1:5]
print(json.dumps({
    "pod": pod,
    "container": container,
    "role": role,
    "result": json.load(open(path)),
}, sort_keys=True))
PY
  echo "  $pod -> chain_id=$chain_id"
done

if [[ "$REQUIRE_OBSERVABILITY" == "1" ]]; then
  sleep 5
  smoke_wait_tempo_tag chain_id "$ARTIFACT_DIR/tempo-tags.json"
  mkdir -p "$ARTIFACT_DIR/loki" "$ARTIFACT_DIR/tempo"
  for chain_id in "${CHAIN_IDS[@]}"; do
    smoke_wait_loki_hits "$chain_id" "$ARTIFACT_DIR/loki/${chain_id}.json" 1 ||
      smoke_fail "Loki returned no discovery log hits for chain_id=$chain_id."
    smoke_wait_tempo_hits "$chain_id" "$ARTIFACT_DIR/tempo/${chain_id}.json" 1 ||
      smoke_fail "Tempo returned no discovery trace hits for chain_id=$chain_id."
  done
fi

cat >"$ARTIFACT_DIR/trace-summary.md" <<EOF
# Conduit Discovery Smoke Summary

- namespace: $NAMESPACE
- bridge pods: ${#BRIDGE_PODS[@]}
- discovery probes: ${#CHAIN_IDS[@]}
- observability required: $REQUIRE_OBSERVABILITY
- result: passed
EOF

echo "Conduit discovery smoke validation passed."
