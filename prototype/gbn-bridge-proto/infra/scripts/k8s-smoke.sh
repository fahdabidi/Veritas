#!/usr/bin/env bash
# Validate the local Conduit Kubernetes topology and the GBN-PROTO-007 admin surfaces.
set -euo pipefail

NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
EXPECTED_BRIDGES="${VERITAS_K8S_EXPECTED_BRIDGES:-10}"
ADMIN_PORT="${VERITAS_K8S_ADMIN_PORT:-9090}"
SEND_DUMMY=0
CHECK_CREATOR_RESTART_PERSISTENCE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --send-dummy)
      SEND_DUMMY=1
      shift
      ;;
    --check-creator-restart-persistence)
      CHECK_CREATOR_RESTART_PERSISTENCE=1
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

assert_not_applicable_local_dht() {
  local pod="$1" container="$2" expected_role="$3" expected_surface="${4:-}"
  admin_curl "$pod" "$container" GET /v1/admin/local-dht | python3 -c '
import json
import sys

expected_role, expected_surface = sys.argv[1:3]
data = json.load(sys.stdin)
role = data.get("role")
state = data.get("state")
publisher_surface = data.get("publisher_surface")
if role != expected_role:
    raise SystemExit(f"expected local DHT role {expected_role!r}, got {role!r}")
if state != "not_applicable":
    raise SystemExit(f"expected state not_applicable, got {state!r}")
if expected_surface and publisher_surface != expected_surface:
    raise SystemExit(f"expected publisher_surface {expected_surface!r}, got {publisher_surface!r}")
if not data.get("reason"):
    raise SystemExit(f"expected a not_applicable reason, got {data!r}")
' "$expected_role" "$expected_surface"
}

assert_publisher_metadata() {
  local pod="$1" container="$2" expected_surface="$3" expected_url_field="$4"
  admin_curl "$pod" "$container" GET /v1/admin/node-metadata | python3 -c '
import json
import sys

expected_surface, expected_url_field = sys.argv[1:3]
data = json.load(sys.stdin)
role = data.get("role")
publisher_surface = data.get("publisher_surface")
if role != "publisher":
    raise SystemExit(f"expected publisher role, got {role!r}")
if publisher_surface != expected_surface:
    raise SystemExit(f"expected publisher_surface {expected_surface!r}, got {publisher_surface!r}")
if not data.get(expected_url_field):
    raise SystemExit(f"expected {expected_url_field} in metadata, got {data!r}")
if not data.get("public_key") or not data.get("publisher_public_key"):
    raise SystemExit(f"expected public_key and publisher_public_key, got {data!r}")
' "$expected_surface" "$expected_url_field"
}

assert_bridge_metadata() {
  local pod="$1"
  admin_curl "$pod" exit-bridge GET /v1/admin/node-metadata | python3 -c '
import json
import sys

data = json.load(sys.stdin)
role = data.get("role")
if role != "exit_bridge":
    raise SystemExit(f"expected exit_bridge role, got {role!r}")
if not data.get("ingress_endpoints"):
    raise SystemExit(f"expected ingress_endpoints, got {data!r}")
if not data.get("udp_punch_port"):
    raise SystemExit(f"expected udp_punch_port, got {data!r}")
if data.get("reachability_class") not in {"direct", "brokered", "relay_only"}:
    raise SystemExit(f"unexpected reachability_class, got {data!r}")
if not data.get("capabilities"):
    raise SystemExit(f"expected capabilities, got {data!r}")
if not data.get("public_key") or not data.get("publisher_public_key"):
    raise SystemExit(f"expected public_key and publisher_public_key, got {data!r}")
'
}

assert_creator_restart_persistence() {
  local before_file after_file old_pod new_pod
  before_file="$(mktemp)"
  after_file="$(mktemp)"
  old_pod="$creator_new_pod"
  admin_curl "$old_pod" creator-runner GET /v1/admin/local-dht >"$before_file"

  echo "Checking creator-new local DHT persistence across pod restart..."
  kubectl -n "$NAMESPACE" delete pod "$old_pod" --wait=true --timeout=120s >/dev/null
  kubectl -n "$NAMESPACE" rollout status deployment/creator-new --timeout=180s >/dev/null
  new_pod="$(pod_for_selector 'app.kubernetes.io/name=creator-new')"
  if [[ -z "$new_pod" || "$new_pod" == "$old_pod" ]]; then
    echo "ERROR: creator-new pod did not restart cleanly (old=$old_pod new=${new_pod:-none})." >&2
    exit 1
  fi
  creator_new_pod="$new_pod"
  admin_curl "$new_pod" creator-runner GET /v1/admin/local-dht >"$after_file"

  python3 - "$before_file" "$after_file" <<'PY'
import json
import sys

before = json.load(open(sys.argv[1]))
after = json.load(open(sys.argv[2]))
for field in ("last_update_ms",):
    before.pop(field, None)
    after.pop(field, None)
if before != after:
    raise SystemExit(f"creator local DHT did not persist across restart\nbefore={before!r}\nafter={after!r}")
PY
  rm -f "$before_file" "$after_file"
}

reset_creator_state() {
  local pod="$1" actor="$2" chain_id
  chain_id="k8s-smoke-reset-${actor}-$(date +%s%N)"
  admin_curl "$pod" creator-runner POST "/v1/admin/reset-creator-state?chain_id=${chain_id}" "{}" >/dev/null
}

echo "Checking namespace '$NAMESPACE'..."
kubectl get namespace "$NAMESPACE" >/dev/null

echo "Checking rollout status..."
kubectl -n "$NAMESPACE" rollout status statefulset/postgres --timeout=30s
kubectl -n "$NAMESPACE" rollout status deployment/publisher-authority --timeout=30s
kubectl -n "$NAMESPACE" rollout status deployment/publisher-receiver --timeout=30s
kubectl -n "$NAMESPACE" rollout status statefulset/exit-bridge --timeout=90s
kubectl -n "$NAMESPACE" rollout status deployment/creator-host --timeout=30s
kubectl -n "$NAMESPACE" rollout status deployment/creator-new --timeout=30s

authority_pod="$(pod_for_selector 'veritas-role=authority')"
receiver_pod="$(pod_for_selector 'veritas-role=receiver')"
creator_host_pod="$(pod_for_selector 'app.kubernetes.io/name=creator-host')"
creator_new_pod="$(pod_for_selector 'app.kubernetes.io/name=creator-new')"
mapfile -t bridge_pods < <(
  kubectl -n "$NAMESPACE" get pods -l veritas-role=bridge \
    -o jsonpath='{range .items[?(@.status.phase=="Running")]}{.metadata.name}{"\n"}{end}'
)

if [[ -z "$authority_pod" || -z "$receiver_pod" || -z "$creator_host_pod" || -z "$creator_new_pod" || "${#bridge_pods[@]}" -lt "$EXPECTED_BRIDGES" ]]; then
  echo "ERROR: expected authority, receiver, creator-host, creator-new, and $EXPECTED_BRIDGES bridge pods." >&2
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

echo "Checking node metadata and empty creator local DHT endpoints..."
assert_publisher_metadata "$authority_pod" publisher-authority authority authority_url
assert_publisher_metadata "$receiver_pod" publisher-receiver receiver receiver_url
assert_not_applicable_local_dht "$authority_pod" publisher-authority publisher authority
assert_not_applicable_local_dht "$receiver_pod" publisher-receiver publisher receiver
for pod in "${bridge_pods[@]}"; do
  assert_bridge_metadata "$pod"
  assert_not_applicable_local_dht "$pod" exit-bridge exit_bridge
done
echo "Resetting creator local DHT baseline for repeatable smoke validation..."
reset_creator_state "$creator_host_pod" host-creator
reset_creator_state "$creator_new_pod" new-creator
for check in "$creator_host_pod:host-creator" "$creator_new_pod:new-creator"; do
  pod="${check%%:*}"
  actor="${check##*:}"
  metadata="$(admin_curl "$pod" creator-runner GET /v1/admin/node-metadata)"
  local_dht="$(admin_curl "$pod" creator-runner GET /v1/admin/local-dht)"
  printf '%s' "$metadata" | python3 -c '
import json
import sys

actor = sys.argv[1]
data = json.load(sys.stdin)
role = data.get("role")
conduit_actor = data.get("conduit_actor")
if role != "creator":
    raise SystemExit(f"expected creator role, got {role!r}")
if conduit_actor != actor:
    raise SystemExit(f"expected actor {actor!r}, got {conduit_actor!r}")
' "$actor"
  printf '%s' "$local_dht" | python3 -c '
import json
import sys

data = json.load(sys.stdin)
role = data.get("role")
actor_id = data.get("actor_id")
self_onboarding_state = data.get("self_onboarding_state")
host_role_state = data.get("host_role_state")
if role != "creator":
    raise SystemExit(f"expected creator local DHT role, got {role!r}")
if actor_id != sys.argv[1]:
    raise SystemExit(f"expected actor_id {sys.argv[1]!r}, got {actor_id!r}")
if self_onboarding_state != "none":
    raise SystemExit(f"expected self_onboarding_state none, got {self_onboarding_state!r}")
if host_role_state != "not_host":
    raise SystemExit(f"expected host_role_state not_host, got {host_role_state!r}")
if data.get("bridge_entries") != [] or data.get("active_tunnels") != []:
    raise SystemExit(f"expected empty bridge/tunnel state, got {data!r}")
' "$actor"
done

if [[ "$CHECK_CREATOR_RESTART_PERSISTENCE" == "1" ]]; then
  assert_creator_restart_persistence
fi

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
  echo "Checking legacy SendDummy surfaces reject non-onboarded/non-creator nodes..."
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
    code="$(printf '%s' "$result" | python3 -c 'import json,sys; data=json.load(sys.stdin); print((data.get("error") or {}).get("code", ""))')"
    if [[ "$code" != "creator_not_onboarded" && "$code" != "not_supported" ]]; then
      echo "ERROR: send-dummy on non-creator pod $pod should be rejected after Pass 3 local-DHT routing." >&2
      printf '%s\n' "$result" >&2
      exit 1
    fi
    echo "  $pod -> rejected as expected with code=$code"
  done
  echo "  Full SendDummy success is covered by Pass 3 Smoke 3 after creator onboarding."
fi

echo "Local Conduit Kubernetes smoke validation passed."
