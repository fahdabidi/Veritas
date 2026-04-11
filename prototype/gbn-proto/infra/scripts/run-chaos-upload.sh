#!/usr/bin/env bash
# run-chaos-upload.sh — Wait for stabilization at full scale, enable chaos rule, then trigger creator upload.
#
# Usage: ./run-chaos-upload.sh <stack-name> [region] [upload-command]

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

STACK_NAME="${1:?Usage: $0 <stack-name> [region] [upload-command]}"
REGION="${2:-us-east-1}"
UPLOAD_COMMAND="${3:-gbn-proto --help}"
POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-30}"
POLL_TIMEOUT_SECONDS="${POLL_TIMEOUT_SECONDS:-600}"

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

  local val
  val="$(aws cloudwatch get-metric-data \
    --region "$REGION" \
    --start-time "$start" \
    --end-time "$end" \
    --scan-by TimestampDescending \
    --metric-data-queries '[{"Id":"boot","Expression":"SUM(SEARCH('"'"'{GBN/ScaleTest,Scale,Subnet,NodeId} MetricName=\"BootstrapResult\"'"'"', '"'"'Sum'"'"', 60))","ReturnData":true}]' \
    --query 'MetricDataResults[0].Values[0]' \
    --output text 2>/dev/null || true)"

  if [ -z "$val" ] || [ "$val" = "None" ] || [ "$val" = "null" ]; then
    echo "0"
  else
    echo "$val"
  fi
}

echo "============================================"
echo "  GBN Phase 1 — Run Chaos Upload"
echo "  Stack:  $STACK_NAME"
echo "  Region: $REGION"
echo "============================================"

echo "[1/5] Resolving stack resource IDs..."
CLUSTER_NAME="$(cf_resource_id ECSCluster)"
CHAOS_RULE_NAME="$(cf_resource_id ChaosEngineRule)"
CREATOR_SERVICE_NAME="$(cf_resource_id CreatorService)"
HOSTILE_SERVICE_NAME="$(cf_resource_id HostileRelayService)"
FREE_SERVICE_NAME="$(cf_resource_id FreeRelayService)"

if [ -z "$CLUSTER_NAME" ] || [ -z "$CHAOS_RULE_NAME" ] || [ -z "$CREATOR_SERVICE_NAME" ]; then
  echo "ERROR: Missing required stack resources (ECSCluster/ChaosEngineRule/CreatorService)."
  exit 1
fi

echo "[2/5] Stabilization Gate 2 (full scale bootstrap >90%)..."
HOSTILE_DESIRED="$(aws ecs describe-services --cluster "$CLUSTER_NAME" --services "$HOSTILE_SERVICE_NAME" --region "$REGION" --query 'services[0].desiredCount' --output text)"
FREE_DESIRED="$(aws ecs describe-services --cluster "$CLUSTER_NAME" --services "$FREE_SERVICE_NAME" --region "$REGION" --query 'services[0].desiredCount' --output text)"
TOTAL_DESIRED=$((HOSTILE_DESIRED + FREE_DESIRED))
THRESHOLD=$((TOTAL_DESIRED * 90 / 100))
if [ "$THRESHOLD" -lt 1 ]; then THRESHOLD=1; fi

start_ts=$(date +%s)
while true; do
  latest_sum="$(bootstrap_sum_latest)"
  latest_int="$(printf '%.0f' "$latest_sum" 2>/dev/null || echo 0)"
  now_ts=$(date +%s)
  elapsed=$((now_ts - start_ts))

  echo "  - BootstrapResult(sum latest)=$latest_sum threshold=$THRESHOLD elapsed=${elapsed}s"
  if [ "$latest_int" -ge "$THRESHOLD" ]; then
    break
  fi

  if [ "$elapsed" -ge "$POLL_TIMEOUT_SECONDS" ]; then
    echo "ERROR: Stabilization Gate 2 timeout after ${elapsed}s"
    exit 1
  fi
  sleep "$POLL_INTERVAL_SECONDS"
done

echo "[3/5] Enabling chaos rule: $CHAOS_RULE_NAME"
aws events enable-rule --name "$CHAOS_RULE_NAME" --region "$REGION"

echo "[4/5] Waiting 60s for churn to take effect..."
sleep 60

echo "[5/5] Executing upload command in creator task..."
CREATOR_TASK_ARN="$(aws ecs list-tasks --cluster "$CLUSTER_NAME" --service-name "$CREATOR_SERVICE_NAME" --desired-status RUNNING --region "$REGION" --query 'taskArns[0]' --output text)"
if [ -z "$CREATOR_TASK_ARN" ] || [ "$CREATOR_TASK_ARN" = "None" ]; then
  echo "ERROR: No running task found for creator service: $CREATOR_SERVICE_NAME"
  exit 1
fi

aws ecs execute-command \
  --cluster "$CLUSTER_NAME" \
  --task "$CREATOR_TASK_ARN" \
  --container creator \
  --interactive \
  --command "sh -lc '$UPLOAD_COMMAND'" \
  --region "$REGION"

echo ""
echo "✅ Chaos enabled and creator upload command executed."
echo "   Cluster: $CLUSTER_NAME"
echo "   Chaos rule: $CHAOS_RULE_NAME"
echo "   Creator task: $CREATOR_TASK_ARN"
