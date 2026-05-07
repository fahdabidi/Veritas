#!/usr/bin/env bash
# Bring up the local Conduit topology in k3d.
set -euo pipefail

CLUSTER_NAME="${VERITAS_K3D_CLUSTER:-veritas}"
NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
SERVERS="${VERITAS_K3D_SERVERS:-1}"
AGENTS="${VERITAS_K3D_AGENTS:-2}"
RUN_SMOKE="${VERITAS_K8S_RUN_SMOKE:-1}"
RUN_CARGO_PERSISTENCE="${VERITAS_K8S_RUN_CARGO_PERSISTENCE:-1}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
OVERLAY_DIR="$ROOT_DIR/infra/k8s/conduit/overlays/dev"

for dep in docker k3d kubectl python3; do
  command -v "$dep" >/dev/null 2>&1 || {
    echo "ERROR: '$dep' is required. Run infra/scripts/bootstrap-k8s.sh inside WSL2 first." >&2
    exit 1
  }
done

docker version >/dev/null

if [[ ! -f "$OVERLAY_DIR/password.txt" ]]; then
  echo "Generating dev-only Postgres password at $OVERLAY_DIR/password.txt"
  umask 077
  python3 - <<'PY' > "$OVERLAY_DIR/password.txt"
import secrets
import string

alphabet = string.ascii_letters + string.digits
print("".join(secrets.choice(alphabet) for _ in range(32)))
PY
fi

if ! k3d cluster get "$CLUSTER_NAME" >/dev/null 2>&1; then
  echo "Creating k3d cluster '$CLUSTER_NAME' (${SERVERS} server, ${AGENTS} agents)..."
  k3d cluster create "$CLUSTER_NAME" \
    --servers "$SERVERS" \
    --agents "$AGENTS" \
    --port "30030:30030@loadbalancer" \
    --wait
else
  echo "Using existing k3d cluster '$CLUSTER_NAME'."
fi

echo "Building local Conduit images..."
docker build -f "$ROOT_DIR/Dockerfile.publisher-authority" \
  -t veritas/publisher-authority:dev "$ROOT_DIR"
docker build -f "$ROOT_DIR/Dockerfile.publisher-receiver" \
  -t veritas/publisher-receiver:dev "$ROOT_DIR"
docker build -f "$ROOT_DIR/Dockerfile.bridge" \
  -t veritas/exit-bridge:dev "$ROOT_DIR"

echo "Importing local images into k3d..."
k3d image import \
  veritas/publisher-authority:dev \
  veritas/publisher-receiver:dev \
  veritas/exit-bridge:dev \
  -c "$CLUSTER_NAME"

echo "Applying Conduit manifests..."
kubectl apply -k "$OVERLAY_DIR"

echo "Waiting for Conduit pods..."
kubectl -n "$NAMESPACE" rollout status statefulset/postgres --timeout=180s
kubectl -n "$NAMESPACE" rollout status deployment/publisher-authority --timeout=180s
kubectl -n "$NAMESPACE" rollout status deployment/publisher-receiver --timeout=180s
kubectl -n "$NAMESPACE" rollout status deployment/exit-bridge --timeout=180s

kubectl -n "$NAMESPACE" get pods,svc,statefulset,deployment

if [[ "$RUN_SMOKE" == "1" ]]; then
  "$SCRIPT_DIR/k8s-smoke.sh" --send-dummy
else
  echo "Skipping k8s smoke validation because VERITAS_K8S_RUN_SMOKE=$RUN_SMOKE."
fi

if [[ "$RUN_CARGO_PERSISTENCE" == "1" ]]; then
  "$SCRIPT_DIR/k8s-test-publisher-postgres.sh"
else
  echo "Skipping Cargo Postgres validation because VERITAS_K8S_RUN_CARGO_PERSISTENCE=$RUN_CARGO_PERSISTENCE."
fi

echo "Conduit local Kubernetes topology is up."
echo "Cluster:   $CLUSTER_NAME"
echo "Namespace: $NAMESPACE"
