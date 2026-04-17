#!/usr/bin/env bash
# deploy-smoke-n5.sh - dedicated 5-node smoke bring-up wrapper
#
# Topology intent:
# - ECS relays: 2 hostile + 1 free
# - ECS creator: 1
# - Static publisher EC2: 1
#
# This wrapper hard-pins smoke mode and prevents accidental full-scale expansion.
#
# Usage:
#   ./deploy-smoke-n5.sh [stack-name] [region]

set -euo pipefail
export AWS_PAGER=""

STACK_NAME="${1:-gbn-proto-phase1-scale-n100}"
REGION="${2:-us-east-1}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Force smoke behavior regardless of caller shell state.
export SMOKE_TOPOLOGY=1
export SEED_PERCENT=30
export RESTART_STATIC_NODES=1

echo "============================================"
echo "  GBN Dedicated Smoke Deploy (n5)"
echo "  Stack:  $STACK_NAME"
echo "  Region: $REGION"
echo "============================================"
echo "  Enforced: SMOKE_TOPOLOGY=1 (no full-scale expansion)"
echo ""

# Template currently restricts ScaleTarget to AllowedValues [100, 500, 1000].
# We pass 100 and rely on SMOKE_TOPOLOGY=1 to pin runtime topology to smoke size.
exec bash "$SCRIPT_DIR/deploy-scale-test.sh" "$STACK_NAME" 100 "$REGION"
