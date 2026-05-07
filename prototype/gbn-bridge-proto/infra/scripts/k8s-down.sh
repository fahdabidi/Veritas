#!/usr/bin/env bash
# Delete the local Conduit k3d cluster.
set -euo pipefail

CLUSTER_NAME="${VERITAS_K3D_CLUSTER:-veritas}"
ASSUME_YES="${VERITAS_K8S_ASSUME_YES:-0}"

command -v k3d >/dev/null 2>&1 || {
  echo "ERROR: k3d is not installed." >&2
  exit 1
}

if ! k3d cluster get "$CLUSTER_NAME" >/dev/null 2>&1; then
  echo "No k3d cluster named '$CLUSTER_NAME' found."
  exit 0
fi

if [[ "$ASSUME_YES" != "1" ]]; then
  read -r -p "Delete k3d cluster '$CLUSTER_NAME'? [y/N]: " confirm
  if [[ "${confirm,,}" != "y" ]]; then
    echo "Not deleting '$CLUSTER_NAME'."
    exit 0
  fi
fi

k3d cluster delete "$CLUSTER_NAME"
echo "Deleted k3d cluster '$CLUSTER_NAME'."
