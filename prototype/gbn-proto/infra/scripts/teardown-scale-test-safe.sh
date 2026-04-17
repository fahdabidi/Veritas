#!/usr/bin/env bash
# teardown-scale-test-safe.sh - path-safe wrapper for teardown-scale-test.sh
#
# Usage:
#   ./teardown-scale-test-safe.sh <stack-name> [region]
#   STACK_NAME=<stack-name> AWS_REGION=<region> ./teardown-scale-test-safe.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET_SCRIPT="$SCRIPT_DIR/teardown-scale-test.sh"

if [ ! -f "$TARGET_SCRIPT" ]; then
  echo "ERROR: teardown script not found at: $TARGET_SCRIPT"
  exit 1
fi

STACK_NAME="${1:-${STACK_NAME:-}}"
REGION="${2:-${AWS_REGION:-us-east-1}}"

if [ -z "$STACK_NAME" ]; then
  echo "Usage: $0 <stack-name> [region]"
  echo "Or set STACK_NAME environment variable."
  exit 1
fi

exec bash "$TARGET_SCRIPT" "$STACK_NAME" "$REGION"

