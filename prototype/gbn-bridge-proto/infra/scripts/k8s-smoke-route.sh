#!/usr/bin/env bash
# Validate creator-to-publisher dummy route delivery in local Kubernetes.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
OBS_NS="${VERITAS_OBS_NAMESPACE:-observability}"
EXPECTED_BRIDGES="${VERITAS_K8S_EXPECTED_BRIDGES:-3}"
ADMIN_PORT="${VERITAS_K8S_ADMIN_PORT:-9090}"
MESSAGE_SIZE="${VERITAS_K8S_SMOKE_MESSAGE_SIZE:-256}"
CREATOR_SELECTOR="veritas-role=authority"
ALL_CREATORS=0
REQUIRE_OBSERVABILITY=0
ARTIFACT_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --namespace) NAMESPACE="$2"; shift 2 ;;
    --observability-namespace) OBS_NS="$2"; shift 2 ;;
    --expected-bridges) EXPECTED_BRIDGES="$2"; shift 2 ;;
    --creator-selector) CREATOR_SELECTOR="$2"; shift 2 ;;
    --all-creators) ALL_CREATORS=1; shift ;;
    --message-size) MESSAGE_SIZE="$2"; shift 2 ;;
    --require-observability) REQUIRE_OBSERVABILITY=1; shift ;;
    --artifact-dir) ARTIFACT_DIR="$2"; shift 2 ;;
    *) echo "ERROR: unknown argument '$1'." >&2; exit 2 ;;
  esac
done

cd "$ROOT_DIR"
source "$SCRIPT_DIR/k8s-smoke-common.sh"
smoke_require_deps
smoke_artifact_dir route >/dev/null
trap 'status=$?; if [[ $status -ne 0 ]]; then smoke_collect_diagnostics; fi; smoke_stop_observability; echo "Artifacts: $ARTIFACT_DIR"; exit $status' EXIT

smoke_check_rollouts
smoke_discover_nodes
smoke_check_admin_metrics
smoke_wait_for_bridge_registry

if [[ "$REQUIRE_OBSERVABILITY" == "1" ]]; then
  smoke_start_observability
fi

declare -a CREATOR_CHECKS=()
if [[ "$ALL_CREATORS" == "1" ]]; then
  CREATOR_CHECKS=("${NODE_CHECKS[@]}")
else
  mapfile -t selected_pods < <(
    kubectl -n "$NAMESPACE" get pods -l "$CREATOR_SELECTOR" -o json |
      python3 -c 'import json,sys
data=json.load(sys.stdin)
names=[]
for item in data.get("items", []):
    if item.get("metadata", {}).get("deletionTimestamp"):
        continue
    if item.get("status", {}).get("phase") == "Running":
        names.append(item.get("metadata", {}).get("name", ""))
print("\n".join(names))'
  )
  for selected in "${selected_pods[@]}"; do
    for check in "${NODE_CHECKS[@]}"; do
      [[ "${check%%:*}" == "$selected" ]] && CREATOR_CHECKS+=("$check")
    done
  done
fi

if [[ "${#CREATOR_CHECKS[@]}" -eq 0 ]]; then
  smoke_fail "no creator pods matched selector '$CREATOR_SELECTOR'."
fi

declare -A BRIDGE_CONTAINER_BY_POD=()
for pod in "${BRIDGE_PODS[@]}"; do
  BRIDGE_CONTAINER_BY_POD["$pod"]="exit-bridge"
done

echo "Running route smoke for ${#CREATOR_CHECKS[@]} creator node(s)..."
: >"$ARTIFACT_DIR/send-dummy-results.jsonl"
declare -a CHAIN_IDS=()
for check in "${CREATOR_CHECKS[@]}"; do
  pod="${check%%:*}"
  rest="${check#*:}"
  container="${rest%%:*}"
  role="${check##*:}"

  authority_before="$(smoke_admin_curl "$AUTHORITY_POD" publisher-authority GET /v1/admin/metrics)"
  receiver_before="$(smoke_admin_curl "$RECEIVER_POD" publisher-receiver GET /v1/admin/metrics)"
  receiver_frames_before="$(printf '%s' "$receiver_before" | smoke_json_metric_field frames_accepted)"
  receiver_bytes_before="$(printf '%s' "$receiver_before" | smoke_json_metric_field bytes_ingested)"

  result="$(smoke_admin_curl "$pod" "$container" POST /v1/admin/send-dummy "{\"size\":${MESSAGE_SIZE}}")"
  result_file="$ARTIFACT_DIR/send-dummy-${pod}.json"
  printf '%s\n' "$result" >"$result_file"
  chain_id="$(printf '%s' "$result" | smoke_json_field chain_id)"
  assigned="$(printf '%s' "$result" | smoke_json_field assigned_bridge_id)"
  if [[ -z "$chain_id" || -z "$assigned" ]]; then
    echo "ERROR: send-dummy on $pod did not return chain_id and assigned_bridge_id." >&2
    cat "$result_file" >&2
    smoke_fail "send-dummy on $pod did not return chain_id and assigned_bridge_id."
  fi
  CHAIN_IDS+=("$chain_id")

  frames_file="$ARTIFACT_DIR/frames-${chain_id}.json"
  smoke_admin_curl "$AUTHORITY_POD" publisher-authority GET "/v1/admin/frames?chain_id=${chain_id}" \
    >"$frames_file"
  python3 - "$frames_file" "$chain_id" "$assigned" <<'PY'
import json
import sys

path, chain_id, assigned = sys.argv[1:4]
frames = json.load(open(path)).get("frames", [])
if not frames:
    raise SystemExit(f"no frames persisted for chain_id={chain_id}")
bad_chain = [frame for frame in frames if frame.get("chain_id") != chain_id and frame.get("frame", {}).get("chain_id") != chain_id]
if bad_chain:
    raise SystemExit(f"found frame with mismatched chain_id for {chain_id}")
if not any(frame.get("via_bridge_id") == assigned for frame in frames):
    raise SystemExit(f"no persisted frame used assigned bridge {assigned}")
PY

  receiver_after=""
  for _ in {1..12}; do
    receiver_after="$(smoke_admin_curl "$RECEIVER_POD" publisher-receiver GET /v1/admin/metrics)"
    receiver_frames_after="$(printf '%s' "$receiver_after" | smoke_json_metric_field frames_accepted)"
    receiver_bytes_after="$(printf '%s' "$receiver_after" | smoke_json_metric_field bytes_ingested)"
    if ((receiver_frames_after >= receiver_frames_before + 1 && receiver_bytes_after >= receiver_bytes_before + MESSAGE_SIZE)); then
      break
    fi
    sleep 5
  done
  if ((receiver_frames_after < receiver_frames_before + 1)); then
    smoke_fail "receiver frames_accepted did not increase for chain_id=$chain_id."
  fi
  if ((receiver_bytes_after < receiver_bytes_before + MESSAGE_SIZE)); then
    smoke_fail "receiver bytes_ingested did not increase by message size for chain_id=$chain_id."
  fi

  if [[ -n "${BRIDGE_CONTAINER_BY_POD[$assigned]:-}" ]]; then
    bridge_forwarded=0
    for _ in {1..12}; do
      bridge_metrics="$(smoke_admin_curl "$assigned" "${BRIDGE_CONTAINER_BY_POD[$assigned]}" GET /v1/admin/metrics)"
      bridge_forwarded="$(printf '%s' "$bridge_metrics" | smoke_json_metric_field frames_forwarded)"
      ((bridge_forwarded >= 1)) && break
      sleep 5
    done
    if ((bridge_forwarded < 1)); then
      smoke_fail "assigned bridge $assigned has no forwarded frames after chain_id=$chain_id."
    fi
  else
    smoke_fail "assigned bridge $assigned is not one of the running bridge pods."
  fi

  python3 - "$pod" "$container" "$role" "$assigned" "$result_file" "$frames_file" >>"$ARTIFACT_DIR/send-dummy-results.jsonl" <<'PY'
import json
import sys

pod, container, role, assigned, result_path, frames_path = sys.argv[1:7]
print(json.dumps({
    "pod": pod,
    "container": container,
    "role": role,
    "assigned_bridge_id": assigned,
    "result": json.load(open(result_path)),
    "frames": json.load(open(frames_path)).get("frames", []),
}, sort_keys=True))
PY
  printf '%s\n' "$authority_before" >"$ARTIFACT_DIR/authority-metrics-before-${pod}.json"
  printf '%s\n' "$receiver_before" >"$ARTIFACT_DIR/receiver-metrics-before-${pod}.json"
  printf '%s\n' "$receiver_after" >"$ARTIFACT_DIR/receiver-metrics-after-${pod}.json"
  echo "  $pod -> chain_id=$chain_id assigned_bridge_id=$assigned"
done

if [[ "$REQUIRE_OBSERVABILITY" == "1" ]]; then
  sleep 5
  smoke_wait_tempo_tag chain_id "$ARTIFACT_DIR/tempo-tags.json"
  smoke_prom_query 'conduit_receiver_frames_accepted_total' "$ARTIFACT_DIR/prometheus-receiver-frames.json"
  smoke_prom_query 'conduit_bridge_frames_forwarded_total' "$ARTIFACT_DIR/prometheus-bridge-forwarded.json"
  mkdir -p "$ARTIFACT_DIR/loki" "$ARTIFACT_DIR/tempo"
  for chain_id in "${CHAIN_IDS[@]}"; do
    smoke_wait_loki_hits "$chain_id" "$ARTIFACT_DIR/loki/${chain_id}.json" 1 ||
      smoke_fail "Loki returned no route log hits for chain_id=$chain_id."
    smoke_wait_tempo_hits "$chain_id" "$ARTIFACT_DIR/tempo/${chain_id}.json" 1 ||
      smoke_fail "Tempo returned no route trace hits for chain_id=$chain_id."
  done
fi

cat >"$ARTIFACT_DIR/trace-summary.md" <<EOF
# Conduit Route Smoke Summary

- namespace: $NAMESPACE
- creators: ${#CREATOR_CHECKS[@]}
- message_size: $MESSAGE_SIZE
- observability required: $REQUIRE_OBSERVABILITY
- result: passed
EOF

echo "Conduit route smoke validation passed."
