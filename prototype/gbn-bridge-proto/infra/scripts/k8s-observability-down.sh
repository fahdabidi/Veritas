#!/usr/bin/env bash
# Remove the local Conduit observability stack.
set -euo pipefail

OBS_NS="${VERITAS_OBS_NAMESPACE:-observability}"
ASSUME_YES="${VERITAS_K8S_ASSUME_YES:-0}"

for dep in kubectl helm; do
  command -v "$dep" >/dev/null 2>&1 || {
    echo "ERROR: '$dep' is required." >&2
    exit 1
  }
done

if ! kubectl get namespace "$OBS_NS" >/dev/null 2>&1; then
  echo "No observability namespace '$OBS_NS' found."
  exit 0
fi

if [[ "$ASSUME_YES" != "1" ]]; then
  read -r -p "Uninstall observability stack from namespace '$OBS_NS'? [y/N]: " confirm
  if [[ "${confirm,,}" != "y" ]]; then
    echo "Not uninstalling observability stack."
    exit 0
  fi
fi

helm uninstall kube-prom --namespace "$OBS_NS" || true
helm uninstall loki --namespace "$OBS_NS" || true
helm uninstall tempo --namespace "$OBS_NS" || true
kubectl delete namespace "$OBS_NS" --ignore-not-found
echo "Observability stack removed."
