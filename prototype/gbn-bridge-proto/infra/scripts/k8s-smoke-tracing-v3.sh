#!/usr/bin/env bash
# Pass 3 Smoke 1: validate ChainID logs, traces, and Prometheus scrape coverage.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
OBS_NS="${VERITAS_OBS_NAMESPACE:-observability}"
EXPECTED_BRIDGES="${VERITAS_K8S_EXPECTED_BRIDGES:-10}"
ADMIN_PORT="${VERITAS_K8S_ADMIN_PORT:-9090}"
TIMEOUT_SECONDS="${VERITAS_K8S_SMOKE_TRACE_TIMEOUT:-120}"
CHAIN_ID_PREFIX="${VERITAS_K8S_SMOKE_CHAIN_PREFIX:-smoke-1-}"
REQUIRE_OBSERVABILITY=1
ARTIFACT_DIR=""

usage() {
  cat <<'EOF'
Usage: k8s-smoke-tracing-v3.sh [options]

Options:
  --namespace NAME                    Kubernetes namespace for Conduit pods.
  --observability-namespace NAME      Kubernetes namespace for Prometheus/Loki/Tempo.
  --expected-bridges N                Expected exit-bridge pod count. Default: 10.
  --timeout N                         Per actor Loki/Tempo wait timeout in seconds.
  --chain-id-prefix PREFIX            Prefix for the generated smoke ChainID.
  --artifact-dir DIR                  Artifact output directory.
  --require-observability             Require Prometheus, Loki, and Tempo. Default.
  --no-require-observability          Only validate echo-chain admin responses.
  -h, --help                          Show this help.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --namespace) NAMESPACE="$2"; shift 2 ;;
    --observability-namespace) OBS_NS="$2"; shift 2 ;;
    --expected-bridges) EXPECTED_BRIDGES="$2"; shift 2 ;;
    --timeout) TIMEOUT_SECONDS="$2"; shift 2 ;;
    --chain-id-prefix) CHAIN_ID_PREFIX="$2"; shift 2 ;;
    --artifact-dir) ARTIFACT_DIR="$2"; shift 2 ;;
    --require-observability) REQUIRE_OBSERVABILITY=1; shift ;;
    --no-require-observability) REQUIRE_OBSERVABILITY=0; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument '$1'." >&2; usage >&2; exit 2 ;;
  esac
done

uname -a | grep -i microsoft >/dev/null || {
  echo "Pass 3 tooling requires WSL2 Ubuntu" >&2
  exit 1
}

cd "$ROOT_DIR"
source "$SCRIPT_DIR/k8s-smoke-common.sh"

smoke_require_deps
smoke_artifact_dir smoke-1-tracing >/dev/null
trap 'status=$?; if [[ $status -ne 0 ]]; then smoke_collect_diagnostics; fi; smoke_stop_observability; echo "Artifacts: $ARTIFACT_DIR"; exit $status' EXIT

mkdir -p "$ARTIFACT_DIR/loki" "$ARTIFACT_DIR/tempo" "$ARTIFACT_DIR/kubectl-logs"

json_string() {
  python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$1"
}

json_field_from_arg() {
  local raw="$1" field="$2"
  python3 -c 'import json,sys; print(json.loads(sys.argv[1]).get(sys.argv[2], ""))' "$raw" "$field"
}

wait_loki_actor() {
  local chain_id="$1" actor_id="$2" output="$3" deadline count
  deadline=$((SECONDS + TIMEOUT_SECONDS))
  while ((SECONDS <= deadline)); do
    smoke_loki_query_chain_actor "$chain_id" "$actor_id" "$output"
    count="$(smoke_loki_hit_count "$output")"
    if [[ "$count" -ge 1 ]]; then
      printf '%s\n' "$count"
      return 0
    fi
    sleep 2
  done
  printf '0\n'
  return 1
}

wait_tempo_service() {
  local chain_id="$1" service_name="$2" output="$3" deadline count
  deadline=$((SECONDS + TIMEOUT_SECONDS))
  while ((SECONDS <= deadline)); do
    smoke_tempo_query_chain_service "$chain_id" "$service_name" "$output"
    count="$(smoke_tempo_hit_count "$output")"
    if [[ "$count" -ge 1 ]]; then
      printf '%s\n' "$count"
      return 0
    fi
    sleep 2
  done
  printf '0\n'
  return 1
}

wait_tempo_all_services() {
  local chain_id="$1" actors_file="$2" output="$3" deadline missing_output
  missing_output="${output%.json}.missing.txt"
  deadline=$((SECONDS + TIMEOUT_SECONDS))
  while ((SECONDS <= deadline)); do
    smoke_tempo_query_chain "$chain_id" "$output"
    if python3 - "$actors_file" "$output" "$missing_output" <<'PY'
import json
import sys

actors_path, tempo_path, missing_path = sys.argv[1:4]
expected = set()
with open(actors_path, encoding="utf-8") as handle:
    for line in handle:
        fields = line.rstrip("\n").split("\t")
        if len(fields) >= 6:
            expected.add(fields[5])

data = json.load(open(tempo_path, encoding="utf-8"))
seen = set()
for trace in data.get("traces", data.get("data", {}).get("traces", [])):
    service = trace.get("rootServiceName")
    if service:
        seen.add(service)
    for service in trace.get("serviceStats", {}).keys():
        seen.add(service)

missing = sorted(expected - seen)
if missing:
    with open(missing_path, "w", encoding="utf-8") as handle:
        handle.write("\n".join(missing) + "\n")
    raise SystemExit(1)
try:
    import os
    os.remove(missing_path)
except FileNotFoundError:
    pass
PY
    then
      return 0
    fi
    sleep 5
  done
  return 1
}

echo "Checking Pass 3 Conduit rollout in namespace '$NAMESPACE'..."
smoke_check_rollouts
smoke_discover_nodes
smoke_check_admin_metrics

CHAIN_ID="${CHAIN_ID_PREFIX}$(python3 -c 'import uuid; print(uuid.uuid4().hex)')"
SMOKE_LOKI_QUERY_START_NS="$(date +%s%N)"
printf '%s\n' "$CHAIN_ID" >"$ARTIFACT_DIR/chain-id.txt"
: >"$ARTIFACT_DIR/echo-responses.jsonl"
: >"$ARTIFACT_DIR/actors.tsv"

echo "Emitting echo-chain-id probe on ${#NODE_CHECKS[@]} Conduit actors with chain_id=$CHAIN_ID..."
for check in "${NODE_CHECKS[@]}"; do
  pod="${check%%:*}"
  rest="${check#*:}"
  container="${rest%%:*}"
  expected_role="${check##*:}"
  body="{\"chain_id\":$(json_string "$CHAIN_ID")}"
  response="$(smoke_admin_curl "$pod" "$container" POST /v1/admin/echo-chain-id "$body")"
  response_chain_id="$(json_field_from_arg "$response" chain_id)"
  actor_id="$(json_field_from_arg "$response" actor_id)"
  response_role="$(json_field_from_arg "$response" role)"
  service_name="$(json_field_from_arg "$response" service_name)"
  if [[ "$response_chain_id" != "$CHAIN_ID" || -z "$actor_id" || -z "$response_role" || -z "$service_name" ]]; then
    printf '%s\n' "$response" >&2
    smoke_fail "echo-chain-id response from $pod did not echo chain_id, actor_id, role, and service_name."
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$pod" "$container" "$expected_role" "$actor_id" "$response_role" "$service_name" \
    >>"$ARTIFACT_DIR/actors.tsv"
  python3 - "$pod" "$container" "$expected_role" "$response" >>"$ARTIFACT_DIR/echo-responses.jsonl" <<'PY'
import json
import sys

pod, container, expected_role, raw = sys.argv[1:5]
print(json.dumps({
    "pod": pod,
    "container": container,
    "expected_role": expected_role,
    "response": json.loads(raw),
}, sort_keys=True))
PY
  echo "  $pod/$container actor_id=$actor_id role=$response_role service_name=$service_name"
done

if [[ "$REQUIRE_OBSERVABILITY" -eq 0 ]]; then
  cat >"$ARTIFACT_DIR/summary.md" <<EOF
# Conduit Smoke 1 Tracing Summary

- namespace: $NAMESPACE
- chain_id: $CHAIN_ID
- actors probed: ${#NODE_CHECKS[@]}
- observability: skipped by --no-require-observability
- result: echo-chain admin responses passed
EOF
  echo "Smoke 1 echo-chain admin validation passed without observability assertions."
  exit 0
fi

echo "Starting Prometheus/Loki/Tempo port-forwards..."
smoke_start_observability

echo "Checking Prometheus scrape coverage..."
smoke_wait_prom_result_count "up{namespace=\"$NAMESPACE\"}" "$ARTIFACT_DIR/prometheus-up.json" "${#NODE_CHECKS[@]}" ||
  smoke_fail "Prometheus did not report at least ${#NODE_CHECKS[@]} Conduit scrape targets."
metric_query="{__name__=~\"conduit_(authority|receiver|bridge|creator)_.*\",namespace=\"$NAMESPACE\"}"
smoke_wait_prom_result_count "$metric_query" "$ARTIFACT_DIR/prometheus-counter-samples.json" 4 ||
  smoke_fail "Prometheus did not report fresh Conduit metric samples."

python3 - "$ARTIFACT_DIR/prometheus-up.json" "${#NODE_CHECKS[@]}" <<'PY'
import json
import sys

path, expected = sys.argv[1], int(sys.argv[2])
data = json.load(open(path))
results = data.get("data", {}).get("result", [])
up = [item for item in results if float(item.get("value", [0, "0"])[1]) == 1.0]
if len(up) < expected:
    raise SystemExit(f"expected at least {expected} up scrape targets, got {len(up)}")
PY

echo "Checking Loki and Tempo evidence per actor..."
: >"$ARTIFACT_DIR/actor-observability.tsv"
wait_tempo_all_services "$CHAIN_ID" "$ARTIFACT_DIR/actors.tsv" "$ARTIFACT_DIR/tempo/chain-all-services.json" ||
  smoke_fail "Tempo did not report all expected service names for chain_id=$CHAIN_ID within ${TIMEOUT_SECONDS}s."
while IFS=$'\t' read -r pod container expected_role actor_id response_role service_name; do
  loki_file="$ARTIFACT_DIR/loki/${actor_id}.json"
  tempo_file="$ARTIFACT_DIR/tempo/${service_name}.json"
  loki_hits="$(wait_loki_actor "$CHAIN_ID" "$actor_id" "$loki_file")" ||
    smoke_fail "Loki returned no hits for actor_id=$actor_id chain_id=$CHAIN_ID."
  tempo_hits="$(wait_tempo_service "$CHAIN_ID" "$service_name" "$tempo_file")" ||
    smoke_fail "Tempo returned no hits for service_name=$service_name chain_id=$CHAIN_ID."
  kubectl -n "$NAMESPACE" logs --since=15m "$pod" -c "$container" \
    --insecure-skip-tls-verify-backend=true >"$ARTIFACT_DIR/kubectl-logs/${pod}.log" 2>/dev/null || true
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$pod" "$container" "$expected_role" "$actor_id" "$response_role" "$service_name" "$loki_hits" "$tempo_hits" \
    >>"$ARTIFACT_DIR/actor-observability.tsv"
  echo "  $actor_id service=$service_name loki_hits=$loki_hits tempo_hits=$tempo_hits"
done <"$ARTIFACT_DIR/actors.tsv"

python3 - "$ARTIFACT_DIR/actor-observability.tsv" "$ARTIFACT_DIR/loki-hits-by-actor.json" "$ARTIFACT_DIR/tempo-spans-by-actor.json" <<'PY'
import json
import sys

rows_path, loki_path, tempo_path = sys.argv[1:4]
loki = []
tempo = []
with open(rows_path, encoding="utf-8") as handle:
    for line in handle:
        pod, container, expected_role, actor_id, response_role, service_name, loki_hits, tempo_hits = line.rstrip("\n").split("\t")
        loki.append({
            "actor_id": actor_id,
            "pod": pod,
            "role": response_role,
            "hits": int(loki_hits),
        })
        tempo.append({
            "actor_id": actor_id,
            "service_name": service_name,
            "pod": pod,
            "spans": int(tempo_hits),
        })
json.dump(loki, open(loki_path, "w", encoding="utf-8"), indent=2, sort_keys=True)
json.dump(tempo, open(tempo_path, "w", encoding="utf-8"), indent=2, sort_keys=True)
PY

up_count="$(smoke_json_result_count "$ARTIFACT_DIR/prometheus-up.json")"
metric_count="$(smoke_json_result_count "$ARTIFACT_DIR/prometheus-counter-samples.json")"
{
  echo "# Conduit Smoke 1 Tracing Summary"
  echo
  echo "- namespace: $NAMESPACE"
  echo "- observability namespace: $OBS_NS"
  echo "- chain_id: $CHAIN_ID"
  echo "- actors probed: ${#NODE_CHECKS[@]}"
  echo "- prometheus up series: $up_count"
  echo "- prometheus Conduit metric samples: $metric_count"
  echo "- result: passed"
  echo
  echo "| Pod | Actor | Role | Service Name | Loki Hits | Tempo Spans |"
  echo "|---|---|---|---|---:|---:|"
  while IFS=$'\t' read -r pod _container _expected_role actor_id response_role service_name loki_hits tempo_hits; do
    echo "| $pod | $actor_id | $response_role | $service_name | $loki_hits | $tempo_hits |"
  done <"$ARTIFACT_DIR/actor-observability.tsv"
} >"$ARTIFACT_DIR/summary.md"

echo "Conduit Smoke 1 tracing validation passed."
