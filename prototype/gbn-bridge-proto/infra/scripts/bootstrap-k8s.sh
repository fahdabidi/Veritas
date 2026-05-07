#!/usr/bin/env bash
# Install local Kubernetes tooling for Conduit k3d validation on WSL2 Ubuntu.
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "ERROR: bootstrap-k8s.sh is intended for WSL2/Linux. Run it inside the WSL distro." >&2
  exit 1
fi

for dep in curl sudo; do
  command -v "$dep" >/dev/null 2>&1 || {
    echo "ERROR: '$dep' is required before bootstrap can continue." >&2
    exit 1
  }
done

if ! command -v docker >/dev/null 2>&1; then
  echo "ERROR: docker is not installed or not on PATH. Enable Docker Desktop WSL integration first." >&2
  exit 1
fi

docker version >/dev/null

if ! command -v k3d >/dev/null 2>&1; then
  echo "Installing k3d..."
  curl -fsSL https://raw.githubusercontent.com/k3d-io/k3d/main/install.sh | bash
fi

if ! command -v kubectl >/dev/null 2>&1; then
  echo "Installing kubectl..."
  tmp="$(mktemp)"
  curl -fsSLo "$tmp" "https://dl.k8s.io/release/$(curl -fsSL https://dl.k8s.io/release/stable.txt)/bin/linux/amd64/kubectl"
  chmod +x "$tmp"
  sudo mv "$tmp" /usr/local/bin/kubectl
fi

if ! command -v helm >/dev/null 2>&1; then
  echo "Installing helm..."
  curl -fsSL https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 | bash
fi

k3d version
kubectl version --client
helm version
echo "Local Kubernetes bootstrap complete."
