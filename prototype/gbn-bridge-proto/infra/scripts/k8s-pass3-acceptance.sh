#!/usr/bin/env bash
# Run the Pass 3 local Kubernetes smoke gates in dependency order.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
REQUIRE_OBSERVABILITY=1

usage() {
  cat <<'EOF'
Usage: k8s-pass3-acceptance.sh [--namespace NAME] [--require-observability|--no-require-observability]

Runs the Pass 3 smoke gates in order:
  1. k8s-smoke-tracing-v3.sh
  2. k8s-smoke-discovery-v3.sh
  3. k8s-smoke-route-v3.sh
  4. k8s-smoke-upload-v3.sh
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --namespace)
      NAMESPACE="$2"
      shift 2
      ;;
    --require-observability)
      REQUIRE_OBSERVABILITY=1
      shift
      ;;
    --no-require-observability)
      REQUIRE_OBSERVABILITY=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "ERROR: unknown argument '$1'." >&2
      usage >&2
      exit 2
      ;;
  esac
done

uname -a | grep -i microsoft >/dev/null || {
  echo "Pass 3 tooling requires WSL2 Ubuntu" >&2
  exit 1
}

args=(--namespace "$NAMESPACE")
if [[ "$REQUIRE_OBSERVABILITY" == "1" ]]; then
  args+=(--require-observability)
fi

run_gate() {
  local label="$1" script="$2"
  echo ""
  echo "=== ${label}: ${script} ==="
  bash "$SCRIPT_DIR/$script" "${args[@]}"
}

run_gate "Smoke 1 / Tracing" "k8s-smoke-tracing-v3.sh"
run_gate "Smoke 2 / Discovery" "k8s-smoke-discovery-v3.sh"
run_gate "Smoke 3 / Route" "k8s-smoke-route-v3.sh"
run_gate "Smoke 4 / Upload" "k8s-smoke-upload-v3.sh"

echo ""
echo "Pass 3 local Kubernetes acceptance passed."
