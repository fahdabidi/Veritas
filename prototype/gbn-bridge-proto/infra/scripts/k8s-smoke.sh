#!/usr/bin/env bash
# Validate the local Conduit Kubernetes topology and the GBN-PROTO-007 admin surfaces.
set -euo pipefail

NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
EXPECTED_BRIDGES="${VERITAS_K8S_EXPECTED_BRIDGES:-3}"
ADMIN_PORT="${VERITAS_K8S_ADMIN_PORT:-9090}"
SEND_DUMMY=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --send-dummy)
      SEND_DUMMY=1
      shift
      ;;
    --namespace)
      NAMESPACE="$2"
      shift 2
      ;;
    --expected-bridges)
      EXPECTED_BRIDGES="$2"
      shift 2
      ;;
    *)
      echo "ERROR: unknown argument '$1'." >&2
      exit 2
      ;;
  esac
done

for dep in kubectl python3; do
  command -v "$dep" >/dev/null 2>&1 || {
    echo "ERROR: '$dep' is required." >&2
    exit 1
  }
done

json_field() {
  local field="$1"
  python3 -c "import json,sys; print(json.load(sys.stdin).get('$field',''))"
}

admin_curl() {
  local pod="$1" container="$2" method="$3" path="$4" body="${5:-}"
  if [[ -n "$body" ]]; then
    kubectl -n "$NAMESPACE" exec "$pod" -c "$container" -- \
      sh -lc "curl -sS -X $method -H 'Content-Type: application/json' http://127.0.0.1:${ADMIN_PORT}${path} -d '$body'"
  else
    kubectl -n "$NAMESPACE" exec "$pod" -c "$container" -- \
      sh -lc "curl -sS -X $method http://127.0.0.1:${ADMIN_PORT}${path}"
  fi
}

pod_for_selector() {
  local selector="$1"
  kubectl -n "$NAMESPACE" get pod -l "$selector" \
    -o jsonpath='{.items[?(@.status.phase=="Running")].metadata.name}' |
    awk '{print $1}'
}

echo "Checking namespace '$NAMESPACE'..."
kubectl get namespace "$NAMESPACE" >/dev/null

echo "Checking rollout status..."
kubectl -n "$NAMESPACE" rollout status statefulset/postgres --timeout=30s
kubectl -n "$NAMESPACE" rollout status deployment/publisher-authority --timeout=30s
kubectl -n "$NAMESPACE" rollout status deployment/publisher-receiver --timeout=30s
kubectl -n "$NAMESPACE" rollout status deployment/exit-bridge --timeout=30s

authority_pod="$(pod_for_selector 'veritas-role=authority')"
receiver_pod="$(pod_for_selector 'veritas-role=receiver')"
mapfile -t bridge_pods < <(
  kubectl -n "$NAMESPACE" get pods -l veritas-role=bridge \
    -o jsonpath='{range .items[?(@.status.phase=="Running")]}{.metadata.name}{"\n"}{end}'
)

if [[ -z "$authority_pod" || -z "$receiver_pod" || "${#bridge_pods[@]}" -lt "$EXPECTED_BRIDGES" ]]; then
  echo "ERROR: expected authority, receiver, and $EXPECTED_BRIDGES bridge pods." >&2
  kubectl -n "$NAMESPACE" get pods -o wide >&2
  exit 1
fi

echo "Checking Postgres readiness..."
postgres_pod="$(pod_for_selector 'app.kubernetes.io/name=postgres')"
kubectl -n "$NAMESPACE" exec "$postgres_pod" -c postgres -- \
  pg_isready -h postgres -U veritas -d veritas_conduit >/dev/null

echo "Checking public health endpoints..."
kubectl -n "$NAMESPACE" exec "$authority_pod" -c publisher-authority -- \
  curl -fsS http://127.0.0.1:8080/readyz >/dev/null
kubectl -n "$NAMESPACE" exec "$receiver_pod" -c publisher-receiver -- \
  curl -fsS http://127.0.0.1:8081/readyz >/dev/null

echo "Checking admin metrics endpoints..."
admin_curl "$authority_pod" publisher-authority GET /v1/admin/metrics >/dev/null
admin_curl "$receiver_pod" publisher-receiver GET /v1/admin/metrics >/dev/null
for pod in "${bridge_pods[@]}"; do
  admin_curl "$pod" exit-bridge GET /v1/admin/metrics >/dev/null
done

echo "Waiting for bridge registration..."
for _ in {1..36}; do
  bridge_count="$(
    admin_curl "$authority_pod" publisher-authority GET /v1/admin/bridges |
      python3 -c 'import json,sys; print(len(json.load(sys.stdin).get("bridges", [])))'
  )"
  if [[ "$bridge_count" -ge "$EXPECTED_BRIDGES" ]]; then
    break
  fi
  sleep 5
done

if [[ "${bridge_count:-0}" -lt "$EXPECTED_BRIDGES" ]]; then
  echo "ERROR: only ${bridge_count:-0} bridge(s) registered; expected $EXPECTED_BRIDGES." >&2
  admin_curl "$authority_pod" publisher-authority GET /v1/admin/bridges >&2 || true
  exit 1
fi

echo "Registered bridges: $bridge_count"

if [[ "$SEND_DUMMY" == "1" ]]; then
  echo "Running SendDummy through authority, receiver, and bridge pods..."
  declare -a checks=(
    "$authority_pod:publisher-authority"
    "$receiver_pod:publisher-receiver"
  )
  for pod in "${bridge_pods[@]}"; do
    checks+=("$pod:exit-bridge")
  done

  for check in "${checks[@]}"; do
    pod="${check%%:*}"
    container="${check##*:}"
    result="$(admin_curl "$pod" "$container" POST /v1/admin/send-dummy '{"size":256}')"
    chain_id="$(printf '%s' "$result" | json_field chain_id)"
    assigned="$(printf '%s' "$result" | json_field assigned_bridge_id)"
    if [[ -z "$chain_id" || -z "$assigned" ]]; then
      echo "ERROR: send-dummy on $pod did not return chain_id and assigned_bridge_id." >&2
      printf '%s\n' "$result" >&2
      exit 1
    fi
    frames="$(
      admin_curl "$authority_pod" publisher-authority GET "/v1/admin/frames?chain_id=${chain_id}" |
        python3 -c 'import json,sys; print(len(json.load(sys.stdin).get("frames", [])))'
    )"
    if [[ "$frames" -lt 1 ]]; then
      echo "ERROR: chain_id $chain_id from $pod was not persisted in authority frames." >&2
      exit 1
    fi
    log_found=0
    for _ in {1..10}; do
      recent_logs="$(
        kubectl -n "$NAMESPACE" logs --tail=2000 \
        --insecure-skip-tls-verify-backend=true \
        -l app.kubernetes.io/part-of=veritas-conduit \
        --all-containers=true
      )"
      if [[ "$recent_logs" == *"$chain_id"* ]]; then
        log_found=1
        break
      fi
      sleep 2
    done
    if [[ "$log_found" != "1" ]]; then
      echo "ERROR: chain_id $chain_id from $pod did not appear in recent pod logs." >&2
      exit 1
    fi
    echo "  $pod -> chain_id=$chain_id assigned_bridge_id=$assigned frames=$frames"
  done
fi

echo "Local Conduit Kubernetes smoke validation passed."
