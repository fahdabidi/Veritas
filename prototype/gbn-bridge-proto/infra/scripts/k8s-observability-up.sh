#!/usr/bin/env bash
# Install the local Conduit observability stack into k3d.
set -euo pipefail

OBS_NS="${VERITAS_OBS_NAMESPACE:-observability}"
CONDUIT_NS="${VERITAS_K8S_NAMESPACE:-veritas}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
OBS_DIR="$ROOT_DIR/infra/k8s/observability"
VALUES_DIR="$OBS_DIR/values"
DASHBOARD_CM="$OBS_DIR/dashboards/conduit-overview-cm.yaml"

for dep in kubectl helm; do
  command -v "$dep" >/dev/null 2>&1 || {
    echo "ERROR: '$dep' is required. Run infra/scripts/bootstrap-k8s.sh inside WSL2 first." >&2
    exit 1
  }
done

if ! kubectl get namespace "$CONDUIT_NS" >/dev/null 2>&1; then
  echo "ERROR: Conduit namespace '$CONDUIT_NS' does not exist. Run infra/scripts/k8s-up.sh first." >&2
  exit 1
fi

echo "Adding Helm repositories..."
helm repo add prometheus-community https://prometheus-community.github.io/helm-charts --force-update >/dev/null
helm repo add grafana https://grafana.github.io/helm-charts --force-update >/dev/null
helm repo update >/dev/null

kubectl create namespace "$OBS_NS" --dry-run=client -o yaml | kubectl apply -f -

echo "Installing Loki + Promtail..."
helm upgrade --install loki grafana/loki-stack \
  --namespace "$OBS_NS" \
  --values "$VALUES_DIR/loki-stack.values.yaml" \
  --wait \
  --timeout 10m

echo "Installing Tempo..."
helm upgrade --install tempo grafana/tempo \
  --namespace "$OBS_NS" \
  --values "$VALUES_DIR/tempo.values.yaml" \
  --wait \
  --timeout 10m

echo "Installing Prometheus + Grafana..."
helm upgrade --install kube-prom prometheus-community/kube-prometheus-stack \
  --namespace "$OBS_NS" \
  --values "$VALUES_DIR/kube-prometheus-stack.values.yaml" \
  --wait \
  --timeout 10m

echo "Applying Conduit Grafana dashboard..."
kubectl apply -f "$DASHBOARD_CM"

echo "Waiting for observability pods to be Ready..."
kubectl -n "$OBS_NS" wait --for=condition=Ready pod --all --timeout=300s

kubectl -n "$OBS_NS" get pods,svc

echo ""
echo "Observability stack ready."
echo "Namespace:  $OBS_NS"
echo "Grafana:    http://localhost:30030  (admin/admin; local only)"
echo "Prometheus: kubectl -n $OBS_NS port-forward svc/kube-prom-prometheus 9090:9090"
echo "Tempo:      kubectl -n $OBS_NS port-forward svc/tempo 3200:3200"
echo ""
echo "Open Grafana, then go to Dashboards > Conduit > Conduit V2 Overview."
