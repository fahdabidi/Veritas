#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

if [[ "${VERITAS_ALLOW_NON_WSL:-0}" != "1" ]]; then
  uname -a | grep -i microsoft >/dev/null || {
    echo "Pass 4 tooling requires WSL2 Ubuntu" >&2
    exit 1
  }
fi

exec python3 "$SCRIPT_DIR/k8s_pass4_public_ingress.py" verify "$@"
