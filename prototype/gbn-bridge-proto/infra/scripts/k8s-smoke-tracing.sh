#!/usr/bin/env bash
# Validate Conduit local-k8s distributed tracing/log instrumentation across all nodes.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
OBS_NS="${VERITAS_OBS_NAMESPACE:-observability}"
EXPECTED_BRIDGES="${VERITAS_K8S_EXPECTED_BRIDGES:-3}"
ADMIN_PORT="${VERITAS_K8S_ADMIN_PORT:-9090}"
MESSAGE_SIZE="${VERITAS_K8S_SMOKE_MESSAGE_SIZE:-128}"
ARTIFACT_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --namespace) NAMESPACE="$2"; shift 2 ;;
    --observability-namespace) OBS_NS="$2"; shift 2 ;;
    --expected-bridges) EXPECTED_BRIDGES="$2"; shift 2 ;;
    --message-size) MESSAGE_SIZE="$2"; shift 2 ;;
    --artifact-dir) ARTIFACT_DIR="$2"; shift 2 ;;
    *) echo "ERROR: unknown argument '$1'." >&2; exit 2 ;;
  esac
done

cd "$ROOT_DIR"
source "$SCRIPT_DIR/k8s-smoke-common.sh"
smoke_require_deps
smoke_artifact_dir tracing >/dev/null
trap 'status=$?; if [[ $status -ne 0 ]]; then smoke_collect_diagnostics; fi; smoke_stop_observability; echo "Artifacts: $ARTIFACT_DIR"; exit $status' EXIT

smoke_check_rollouts
smoke_discover_nodes
smoke_check_admin_metrics
smoke_wait_for_bridge_registry
smoke_start_observability

echo "Generating trace traffic from every Conduit node..."
: >"$ARTIFACT_DIR/send-dummy-results.jsonl"
declare -a CHAIN_IDS=()
declare -A ACTOR_CHAIN_BY_POD=()
for check in "${NODE_CHECKS[@]}"; do
  pod="${check%%:*}"
  rest="${check#*:}"
  container="${rest%%:*}"
  role="${check##*:}"
  result="$(smoke_admin_curl "$pod" "$container" POST /v1/admin/send-dummy "{\"size\":${MESSAGE_SIZE}}")"
  chain_id="$(printf '%s' "$result" | smoke_json_field chain_id)"
  assigned="$(printf '%s' "$result" | smoke_json_field assigned_bridge_id)"
  if [[ -z "$chain_id" || -z "$assigned" ]]; then
    echo "ERROR: send-dummy on $pod did not return chain_id and assigned_bridge_id." >&2
    printf '%s\n' "$result" >&2
    exit 1
  fi
  CHAIN_IDS+=("$chain_id")
  ACTOR_CHAIN_BY_POD["$pod"]="$chain_id"
  python3 - "$pod" "$container" "$role" "$result" >>"$ARTIFACT_DIR/send-dummy-results.jsonl" <<'PY'
import json
import sys

pod, container, role, raw = sys.argv[1:5]
print(json.dumps({
    "pod": pod,
    "container": container,
    "role": role,
    "result": json.loads(raw),
}, sort_keys=True))
PY
  echo "  $pod -> chain_id=$chain_id assigned_bridge_id=$assigned"
done

sleep 5
smoke_wait_tempo_tag chain_id "$ARTIFACT_DIR/tempo-tags.json"
smoke_wait_tempo_tag service.name "$ARTIFACT_DIR/tempo-tags.json"
smoke_wait_prom_result_count 'up{namespace="veritas"}' "$ARTIFACT_DIR/prometheus-up.json" 5 ||
  smoke_fail "Prometheus did not report at least 5 Conduit targets."
up_count="$(smoke_json_result_count "$ARTIFACT_DIR/prometheus-up.json")"
if [[ "$up_count" -lt 5 ]]; then
  smoke_fail "expected at least 5 Prometheus Conduit targets, got $up_count."
fi

echo "Checking Loki and Tempo evidence for generated chain_id values..."
mkdir -p "$ARTIFACT_DIR/loki" "$ARTIFACT_DIR/tempo" "$ARTIFACT_DIR/kubectl-logs"
for chain_id in "${CHAIN_IDS[@]}"; do
  smoke_wait_loki_hits "$chain_id" "$ARTIFACT_DIR/loki/${chain_id}.json" 1 ||
    smoke_fail "Loki returned no log hits for chain_id=$chain_id."
  smoke_wait_tempo_hits "$chain_id" "$ARTIFACT_DIR/tempo/${chain_id}.json" 1 ||
    smoke_fail "Tempo returned no trace hits for chain_id=$chain_id."
done

for check in "${NODE_CHECKS[@]}"; do
  pod="${check%%:*}"
  rest="${check#*:}"
  container="${rest%%:*}"
  chain_id="${ACTOR_CHAIN_BY_POD[$pod]}"
  kubectl -n "$NAMESPACE" logs --since=15m "$pod" -c "$container" \
    --insecure-skip-tls-verify-backend=true >"$ARTIFACT_DIR/kubectl-logs/${pod}.log" 2>/dev/null || true
  if ! grep -F "$chain_id" "$ARTIFACT_DIR/kubectl-logs/${pod}.log" >/dev/null 2>&1; then
    smoke_fail "creator pod $pod did not log its own chain_id=$chain_id."
  fi
done

cat >"$ARTIFACT_DIR/trace-summary.md" <<EOF
# Conduit Tracing Smoke Summary

- namespace: $NAMESPACE
- generated chains: ${#CHAIN_IDS[@]}
- prometheus up series: $up_count
- result: passed
EOF

echo "Conduit tracing smoke validation passed."
