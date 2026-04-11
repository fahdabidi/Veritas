#!/usr/bin/env bash
# teardown-scale-test.sh — Disable chaos, dump metrics, scale services to zero, and delete scale stack.
#
# Usage: ./teardown-scale-test.sh <stack-name> [region]

set -euo pipefail
export AWS_PAGER=""

if ! command -v aws >/dev/null 2>&1; then
  if command -v aws.exe >/dev/null 2>&1; then
    aws() { aws.exe "$@"; }
  else
    echo "ERROR: aws CLI not found in PATH (tried aws and aws.exe)."
    exit 1
  fi
fi

STACK_NAME="${1:?Usage: $0 <stack-name> [region]}"
REGION="${2:-us-east-1}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROTO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
RESULTS_DIR="$PROTO_ROOT/results"
mkdir -p "$RESULTS_DIR"

cf_resource_id() {
  local logical_id="$1"
  aws cloudformation describe-stack-resources \
    --stack-name "$STACK_NAME" \
    --region "$REGION" \
    --logical-resource-id "$logical_id" \
    --query 'StackResources[0].PhysicalResourceId' \
    --output text 2>/dev/null || true
}

echo "============================================"
echo "  GBN Phase 1 — Teardown Scale Test"
echo "  Stack:  $STACK_NAME"
echo "  Region: $REGION"
echo "============================================"

CLUSTER_NAME="$(cf_resource_id ECSCluster)"
CHAOS_RULE_NAME="$(cf_resource_id ChaosEngineRule)"
HOSTILE_SERVICE_NAME="$(cf_resource_id HostileRelayService)"
FREE_SERVICE_NAME="$(cf_resource_id FreeRelayService)"
CREATOR_SERVICE_NAME="$(cf_resource_id CreatorService)"
PUBLISHER_SERVICE_NAME="$(cf_resource_id PublisherService)"

TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
METRICS_FILE="$RESULTS_DIR/scale-${STACK_NAME}-${TIMESTAMP}-metrics.json"

echo "[1/4] Disabling chaos rule (if present)..."
if [ -n "$CHAOS_RULE_NAME" ] && [ "$CHAOS_RULE_NAME" != "None" ]; then
  aws events disable-rule --name "$CHAOS_RULE_NAME" --region "$REGION" || true
  echo "  Chaos disabled: $CHAOS_RULE_NAME"
else
  echo "  Chaos rule not found (already removed or stack not active)."
fi

echo "[2/4] Dumping CloudWatch metrics..."
END_TIME="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || python - <<'PY'
from datetime import datetime, timezone
print(datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'))
PY
)"
START_TIME="$(date -u -d '4 hours ago' +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || python - <<'PY'
from datetime import datetime, timedelta, timezone
print((datetime.now(timezone.utc) - timedelta(hours=4)).strftime('%Y-%m-%dT%H:%M:%SZ'))
PY
)"

aws cloudwatch get-metric-data \
  --region "$REGION" \
  --start-time "$START_TIME" \
  --end-time "$END_TIME" \
  --scan-by TimestampAscending \
  --metric-data-queries '[
    {"Id":"bootstrap","Expression":"SUM(SEARCH('"'"'{GBN/ScaleTest,Scale,Subnet,NodeId} MetricName=\"BootstrapResult\"'"'"', '"'"'Sum'"'"', 60))","ReturnData":true},
    {"Id":"gossipbw","Expression":"SUM(SEARCH('"'"'{GBN/ScaleTest,Scale,Subnet,NodeId} MetricName=\"GossipBandwidthBytes\"'"'"', '"'"'Sum'"'"', 60))","ReturnData":true},
    {"Id":"circuit","Expression":"SUM(SEARCH('"'"'{GBN/ScaleTest,Scale,Subnet,NodeId} MetricName=\"CircuitBuildResult\"'"'"', '"'"'Sum'"'"', 60))","ReturnData":true},
    {"Id":"chunks","Expression":"SUM(SEARCH('"'"'{GBN/ScaleTest,Scale,Subnet,NodeId} MetricName=\"ChunksDelivered\"'"'"', '"'"'Sum'"'"', 60))","ReturnData":true}
  ]' \
  --output json > "$METRICS_FILE"

echo "  Metrics saved: $METRICS_FILE"

echo "[3/4] Scaling ECS services to 0 (fast cost cutoff)..."
if [ -n "$CLUSTER_NAME" ] && [ "$CLUSTER_NAME" != "None" ]; then
  for svc in "$HOSTILE_SERVICE_NAME" "$FREE_SERVICE_NAME" "$CREATOR_SERVICE_NAME" "$PUBLISHER_SERVICE_NAME"; do
    if [ -n "$svc" ] && [ "$svc" != "None" ]; then
      aws ecs update-service --cluster "$CLUSTER_NAME" --service "$svc" --desired-count 0 --region "$REGION" >/dev/null || true
      echo "  Scaled to zero: $svc"
    fi
  done
else
  echo "  Cluster not found; skipping ECS scale-down."
fi

echo "[4/4] Deleting CloudFormation stack..."
aws cloudformation delete-stack --stack-name "$STACK_NAME" --region "$REGION"
echo "  Waiting for stack delete completion..."
aws cloudformation wait stack-delete-complete --stack-name "$STACK_NAME" --region "$REGION"

echo ""
echo "✅ Scale test stack teardown complete."
echo "   Stack:   $STACK_NAME"
echo "   Metrics: $METRICS_FILE"
