#!/usr/bin/env bash
# Placeholder for GBN-PROTO-012 Phase 8.

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: k8s-smoke-discovery-v3.sh [--namespace NAME] [--require-observability] [--bootstrap-timeout N]

Phase 8 owns the implementation of this script. Phase 6 installs the command
name so operator tooling and acceptance ordering cannot fall back to Pass 2
discovery-probe shortcuts.
EOF
}

for arg in "$@"; do
  case "$arg" in
    -h|--help)
      usage
      exit 0
      ;;
  esac
done

uname -a | grep -i microsoft >/dev/null || {
  echo "Pass 3 tooling requires WSL2 Ubuntu" >&2
  exit 1
}

echo "GBN-PROTO-012 Phase 8 has not implemented k8s-smoke-discovery-v3.sh yet." >&2
exit 64
