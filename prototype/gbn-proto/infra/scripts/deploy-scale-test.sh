#!/usr/bin/env bash
# deploy-scale-test.sh — Deploy Phase 1 scale stack, seed at 33%, wait for bootstrap, then scale to full target.
#
# Usage: ./deploy-scale-test.sh <stack-name> [scale-target] [region]

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

STACK_NAME="${1:?Usage: $0 <stack-name> [scale-target] [region]}"
SCALE_TARGET="${2:-100}"
REGION="${3:-us-east-1}"
POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-30}"
POLL_TIMEOUT_SECONDS="${POLL_TIMEOUT_SECONDS:-600}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROTO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TEMPLATE_PATH="$PROTO_ROOT/infra/cloudformation/phase1-scale-stack.yaml"

cf_resource_id() {
  local logical_id="$1"
  aws cloudformation describe-stack-resources \
    --stack-name "$STACK_NAME" \
    --region "$REGION" \
    --logical-resource-id "$logical_id" \
    --query 'StackResources[0].PhysicalResourceId' \
    --output text
}

bootstrap_sum_latest() {
  local start end
  start="$(date -u -d '10 minutes ago' +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || python - <<'PY'
from datetime import datetime, timedelta, timezone
print((datetime.now(timezone.utc) - timedelta(minutes=10)).strftime('%Y-%m-%dT%H:%M:%SZ'))
PY
)"
  end="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || python - <<'PY'
from datetime import datetime, timezone
print(datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'))
PY
)"

  local json
  json="$(aws cloudwatch get-metric-data \
    --region "$REGION" \
    --start-time "$start" \
    --end-time "$end" \
    --scan-by TimestampDescending \
    --metric-data-queries '[{"Id":"boot","Expression":"SUM(SEARCH('"'"'{GBN/ScaleTest,Scale,Subnet,NodeId} MetricName=\"BootstrapResult\"'"'"', '"'"'Sum'"'"', 60))","ReturnData":true}]' \
    --query 'MetricDataResults[0].Values[0]' \
    --output text 2>/dev/null || true)"

  if [ -z "$json" ] || [ "$json" = "None" ] || [ "$json" = "null" ]; then
    echo "0"
  else
    printf '%s\n' "$json"
  fi
}

echo "============================================"
echo "  GBN Phase 1 — Deploy Scale Test"
echo "  Stack:  $STACK_NAME"
echo "  Scale:  $SCALE_TARGET"
echo "  Region: $REGION"
echo "============================================"

echo "[1/5] Deploying CloudFormation stack..."
aws cloudformation deploy \
  --stack-name "$STACK_NAME" \
  --template-file "$TEMPLATE_PATH" \
  --capabilities CAPABILITY_IAM \
  --parameter-overrides "ScaleTarget=$SCALE_TARGET" \
  --region "$REGION"

echo "[2/5] Resolving ECS services..."
CLUSTER_NAME="$(cf_resource_id ECSCluster)"
HOSTILE_SERVICE_NAME="$(cf_resource_id HostileRelayService)"
FREE_SERVICE_NAME="$(cf_resource_id FreeRelayService)"

if [ -z "$CLUSTER_NAME" ] || [ -z "$HOSTILE_SERVICE_NAME" ] || [ -z "$FREE_SERVICE_NAME" ]; then
  echo "ERROR: Failed to resolve ECS cluster/service resource IDs."
  exit 1
fi

SEED_COUNT=$((SCALE_TARGET / 3))
if [ "$SEED_COUNT" -lt 1 ]; then SEED_COUNT=1; fi
HOSTILE_SEED=$((SEED_COUNT * 9 / 10))
if [ "$HOSTILE_SEED" -lt 1 ]; then HOSTILE_SEED=1; fi
FREE_SEED=$((SEED_COUNT - HOSTILE_SEED))
if [ "$FREE_SEED" -lt 1 ]; then FREE_SEED=1; fi

FULL_HOSTILE=$((SCALE_TARGET * 9 / 10))
FULL_FREE=$((SCALE_TARGET - FULL_HOSTILE))

echo "[3/5] Scaling to seed fleet (33%)..."
echo "  Hostile seed: $HOSTILE_SEED"
echo "  Free seed:    $FREE_SEED"
aws ecs update-service --cluster "$CLUSTER_NAME" --service "$HOSTILE_SERVICE_NAME" --desired-count "$HOSTILE_SEED" --region "$REGION" >/dev/null
aws ecs update-service --cluster "$CLUSTER_NAME" --service "$FREE_SERVICE_NAME" --desired-count "$FREE_SEED" --region "$REGION" >/dev/null

echo "[4/5] Stabilization Gate 1 (BootstrapResult >= 90% seed)..."
SEED_THRESHOLD=$((SEED_COUNT * 90 / 100))
if [ "$SEED_THRESHOLD" -lt 1 ]; then SEED_THRESHOLD=1; fi
echo "  Threshold: $SEED_THRESHOLD"

start_ts=$(date +%s)
while true; do
  latest_sum="$(bootstrap_sum_latest)"
  latest_int="$(printf '%.0f' "$latest_sum" 2>/dev/null || echo 0)"
  now_ts=$(date +%s)
  elapsed=$((now_ts - start_ts))

  echo "  - BootstrapResult(sum latest)=$latest_sum elapsed=${elapsed}s"
  if [ "$latest_int" -ge "$SEED_THRESHOLD" ]; then
    break
  fi

  if [ "$elapsed" -ge "$POLL_TIMEOUT_SECONDS" ]; then
    echo "ERROR: Stabilization Gate 1 timeout after ${elapsed}s"
    exit 1
  fi
  sleep "$POLL_INTERVAL_SECONDS"
done

echo "[5/5] Scaling to full target..."
echo "  Hostile full: $FULL_HOSTILE"
echo "  Free full:    $FULL_FREE"
aws ecs update-service --cluster "$CLUSTER_NAME" --service "$HOSTILE_SERVICE_NAME" --desired-count "$FULL_HOSTILE" --region "$REGION" >/dev/null
aws ecs update-service --cluster "$CLUSTER_NAME" --service "$FREE_SERVICE_NAME" --desired-count "$FULL_FREE" --region "$REGION" >/dev/null

echo ""
echo "✅ Scale test stack deployed and scaled to full target."
echo "   Cluster: $CLUSTER_NAME"
echo "   Hostile service: $HOSTILE_SERVICE_NAME"
echo "   Free service:    $FREE_SERVICE_NAME"
