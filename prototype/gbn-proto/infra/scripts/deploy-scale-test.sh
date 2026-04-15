#!/usr/bin/env bash
# deploy-scale-test.sh — Deploy Phase 1 scale stack, seed at configurable percentage,
# wait for bootstrap, then scale to full target.
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

# Convert a Linux/WSL path to a Windows path when wslpath is available.
# This is required because aws.exe (Windows-native CLI) cannot resolve /mnt/* paths.
convert_path() {
  # Only convert to Windows paths when the effective AWS CLI is aws.exe.
  # If running native Linux aws (WSL/Git Bash with GNU aws), keep POSIX paths.
  if command -v aws.exe >/dev/null 2>&1 && ! command -v aws >/dev/null 2>&1; then
    if command -v wslpath >/dev/null 2>&1; then
      wslpath -w "$1"
      return
    fi
  fi
  echo "$1"
}

STACK_NAME="${1:?Usage: $0 <stack-name> [scale-target] [region]}"
SCALE_TARGET="${2:-100}"
REGION="${3:-us-east-1}"
POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-10}"
POLL_TIMEOUT_SECONDS="${POLL_TIMEOUT_SECONDS:-1200}"
SEED_PERCENT="${SEED_PERCENT:-30}"

if [ "$SEED_PERCENT" -lt 1 ] || [ "$SEED_PERCENT" -gt 99 ]; then
  echo "ERROR: SEED_PERCENT must be between 1 and 99."
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROTO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TEMPLATE_PATH="$(convert_path "$PROTO_ROOT/infra/cloudformation/phase1-scale-stack.yaml")"

cf_resource_id() {
  local logical_id="$1"
  aws cloudformation describe-stack-resources \
    --stack-name "$STACK_NAME" \
    --region "$REGION" \
    --logical-resource-id "$logical_id" \
    --query 'StackResources[0].PhysicalResourceId' \
    --output text
}

running_sum_latest() {
  local hostile_running free_running
  hostile_running="$(aws ecs describe-services --cluster "$CLUSTER_NAME" --services "$HOSTILE_SERVICE_NAME" --region "$REGION" --query 'services[0].runningCount' --output text 2>/dev/null || echo 0)"
  free_running="$(aws ecs describe-services --cluster "$CLUSTER_NAME" --services "$FREE_SERVICE_NAME" --region "$REGION" --query 'services[0].runningCount' --output text 2>/dev/null || echo 0)"
  hostile_running="${hostile_running:-0}"
  free_running="${free_running:-0}"
  echo $((hostile_running + free_running))
}

bootstrap_sum_latest() {
  local start end
  start="$(date -u -d '15 minutes ago' +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || python3 - <<'PY'
from datetime import datetime, timedelta, timezone
print((datetime.now(timezone.utc) - timedelta(minutes=15)).strftime('%Y-%m-%dT%H:%M:%SZ'))
PY
)"
  end="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || python3 - <<'PY'
from datetime import datetime, timezone
print(datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'))
PY
)"

  # Filter by Scale=\"$SCALE_TARGET\" to avoid matching NodeId series from past runs
  # which can push the total series count past CloudWatch's 500-per-request limit.
  local json
  json="$(aws cloudwatch get-metric-data \
    --region "$REGION" \
    --start-time "$start" \
    --end-time "$end" \
    --scan-by TimestampDescending \
    --metric-data-queries "[{\"Id\":\"boot\",\"Expression\":\"SUM(SEARCH('{GBN/ScaleTest,Scale,Subnet,NodeId} Scale=\\\"$SCALE_TARGET\\\" MetricName=\\\"BootstrapResult\\\"', 'SampleCount', 300))\",\"ReturnData\":true}]" \
    --query 'max(MetricDataResults[0].Values)' \
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
  --no-fail-on-empty-changeset \
  --parameter-overrides "ScaleTarget=$SCALE_TARGET" \
  --region "$REGION"

echo "[2/5] Resolving ECS services..."
CLUSTER_NAME="$(cf_resource_id ECSCluster)"
HOSTILE_SERVICE_NAME="$(cf_resource_id HostileRelayService)"
FREE_SERVICE_NAME="$(cf_resource_id FreeRelayService)"
CREATOR_SERVICE_NAME="$(cf_resource_id CreatorService)"
PUBLISHER_SERVICE_NAME="$(cf_resource_id PublisherService)"
ECR_RELAY_REPO="$(aws cloudformation describe-stacks --stack-name "$STACK_NAME" --region "$REGION" --output json | \
  python3 -c "import json,sys; d=json.load(sys.stdin); o=d['Stacks'][0].get('Outputs',[]); print(next((x['OutputValue'] for x in o if x.get('OutputKey')=='ECRUriRelay'), ''))" 2>/dev/null || true)"
RELAY_REPO_NAME="${ECR_RELAY_REPO##*/}"

if [ -z "$CLUSTER_NAME" ] || [ -z "$HOSTILE_SERVICE_NAME" ] || [ -z "$FREE_SERVICE_NAME" ]; then
  echo "ERROR: Failed to resolve ECS cluster/service resource IDs."
  exit 1
fi

# Check if ECR has images before attempting to scale (tasks will fail image pull otherwise).
ECR_IMAGE_COUNT=0
if [ -n "$RELAY_REPO_NAME" ]; then
  ECR_IMAGE_COUNT="$(aws ecr list-images --repository-name "$RELAY_REPO_NAME" --region "$REGION" \
    --query 'length(imageIds)' --output text 2>/dev/null || echo 0)"
  ECR_IMAGE_COUNT="${ECR_IMAGE_COUNT:-0}"
fi
if [ "$ECR_IMAGE_COUNT" = "0" ] || [ "$ECR_IMAGE_COUNT" = "None" ]; then
  echo ""
  echo "⚠️  ECR repository '$RELAY_REPO_NAME' has no images yet."
  echo "   Run build-and-push.sh first, then re-run this script:"
  echo "   bash infra/scripts/build-and-push.sh $STACK_NAME $REGION"
  echo "   bash infra/scripts/deploy-scale-test.sh $STACK_NAME $SCALE_TARGET $REGION"
  echo ""
  echo "   Stack created successfully — ECR repos and all other resources are ready."
  exit 0
fi

SEED_COUNT=$((SCALE_TARGET * SEED_PERCENT / 100))
if [ "$SEED_COUNT" -lt 1 ]; then SEED_COUNT=1; fi
HOSTILE_SEED=$((SEED_COUNT * 9 / 10))
if [ "$HOSTILE_SEED" -lt 1 ]; then HOSTILE_SEED=1; fi
FREE_SEED=$((SEED_COUNT - HOSTILE_SEED))
if [ "$FREE_SEED" -lt 1 ]; then FREE_SEED=1; fi

FULL_HOSTILE=$((SCALE_TARGET * 9 / 10))
FULL_FREE=$((SCALE_TARGET - FULL_HOSTILE))

echo "[3/5] Scaling to seed fleet (${SEED_PERCENT}%) + creator + publisher..."
echo "  Hostile seed: $HOSTILE_SEED"
echo "  Free seed:    $FREE_SEED"
aws ecs update-service --cluster "$CLUSTER_NAME" --service "$HOSTILE_SERVICE_NAME" --desired-count "$HOSTILE_SEED" --region "$REGION" >/dev/null
aws ecs update-service --cluster "$CLUSTER_NAME" --service "$FREE_SERVICE_NAME" --desired-count "$FREE_SEED" --region "$REGION" >/dev/null
if [ -n "$CREATOR_SERVICE_NAME" ] && [ "$CREATOR_SERVICE_NAME" != "None" ]; then
  aws ecs update-service --cluster "$CLUSTER_NAME" --service "$CREATOR_SERVICE_NAME" --desired-count 1 --region "$REGION" >/dev/null
  echo "  Creator: 1"
fi
if [ -n "$PUBLISHER_SERVICE_NAME" ] && [ "$PUBLISHER_SERVICE_NAME" != "None" ]; then
  aws ecs update-service --cluster "$CLUSTER_NAME" --service "$PUBLISHER_SERVICE_NAME" --desired-count 1 --region "$REGION" >/dev/null
  echo "  Publisher: 1"
fi

echo "[4/5] Stabilization Gate 1 (ECS running tasks >= 90% seed; BootstrapResult for diagnostics)..."
SEED_THRESHOLD=$((SEED_COUNT * 90 / 100))
if [ "$SEED_THRESHOLD" -lt 1 ]; then SEED_THRESHOLD=1; fi
echo "  Target: $SEED_THRESHOLD/$SEED_COUNT tasks running  (timeout: ${POLL_TIMEOUT_SECONDS}s)"

start_ts=$(date +%s)
last_cw_ts=0
cw_val="--"

while true; do
  now_ts=$(date +%s)
  elapsed=$((now_ts - start_ts))
  running_total="$(running_sum_latest)"

  # Query CloudWatch every 30s (rate-limit; diagnostic only — not a gate condition)
  cw_next=$(( 30 - (now_ts - last_cw_ts) ))
  if [ "$cw_next" -le 0 ]; then
    cw_val="$(bootstrap_sum_latest)"
    last_cw_ts=$now_ts
    cw_next=30
  fi

  printf "  [%4ds] ECS: %d/%d running  |  CW BootstrapResult(10m sum): %s  (next CW in %ds)\n" \
    "$elapsed" "$running_total" "$SEED_THRESHOLD" "$cw_val" "$cw_next"

  # Gate condition: ECS running count only (reliable; CW is diagnostic)
  if [ "$running_total" -ge "$SEED_THRESHOLD" ]; then
    echo "  ✅ Gate 1 passed: ECS running=$running_total >= threshold=$SEED_THRESHOLD  (CW bootstrap=$cw_val)"
    break
  fi

  if [ "$elapsed" -ge "$POLL_TIMEOUT_SECONDS" ]; then
    echo "ERROR: Stabilization Gate 1 timeout after ${elapsed}s  (ECS running=$running_total/$SEED_THRESHOLD  CW bootstrap=$cw_val)"
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
