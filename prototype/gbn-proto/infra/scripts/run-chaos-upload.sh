#!/usr/bin/env bash
# run-chaos-upload.sh — Wait for stabilization at full scale, optionally enable chaos,
# then trigger creator upload.
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
UPLOAD_COMMAND="${3:-echo 'gbn-creator-healthy'}"
ENABLE_CHAOS="${ENABLE_CHAOS:-1}"
CHAOS_NORMALIZED="$(printf "%s" "$ENABLE_CHAOS" | tr '[:upper:]' '[:lower:]')"
POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-10}"
POLL_TIMEOUT_SECONDS="${POLL_TIMEOUT_SECONDS:-1200}"
# How long to let the gossip network run under chaos before tearing down (seconds).
# Must be long enough for: creator to detect peers, publish messages, gossip to propagate,
# and CloudWatch to receive at least one full 60-second metric window.
# 600s = 10 min: first ~4-5 min for the gossip network to form (register + jitter + bootstrap
# + re-bootstrap), then 5+ min of chaos churn to observe the network under failure.
CHAOS_OBSERVE_SECONDS="${CHAOS_OBSERVE_SECONDS:-600}"

cf_resource_id() {
  local logical_id="$1"
  aws cloudformation describe-stack-resources \
    --stack-name "$STACK_NAME" \
    --region "$REGION" \
    --logical-resource-id "$logical_id" \
    --query 'StackResources[0].PhysicalResourceId' \
    --output text | tr -d '\r'
}

# Derive numeric scale from stack name suffix (e.g. gbn-proto-phase1-scale-n100 → 100).
# Used to scope the CloudWatch SEARCH to the current run and avoid MaxMetricsExceeded.
SCALE_HINT="$(echo "$STACK_NAME" | grep -oE 'n[0-9]+$' | tr -d 'n' || true)"

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

  # Filter by Scale=\"$SCALE_HINT\" when available to avoid matching NodeId series
  # from past runs, which can exceed CloudWatch's 500-series-per-request limit.
  local scale_filter=""
  if [ -n "$SCALE_HINT" ]; then
    scale_filter=" Scale=\\\"$SCALE_HINT\\\""
  fi

  local val
  val="$(aws cloudwatch get-metric-data \
    --region "$REGION" \
    --start-time "$start" \
    --end-time "$end" \
    --scan-by TimestampDescending \
    --metric-data-queries "[{\"Id\":\"boot\",\"Expression\":\"SUM(SEARCH('{GBN/ScaleTest,Scale,Subnet,NodeId}${scale_filter} MetricName=\\\"BootstrapResult\\\"', 'SampleCount', 300))\",\"ReturnData\":true}]" \
    --query 'max(MetricDataResults[0].Values)' \
    --output text 2>/dev/null || true)"

  if [ -z "$val" ] || [ "$val" = "None" ] || [ "$val" = "null" ]; then
    echo "0"
  else
    echo "$val"
  fi
}

running_sum_latest() {
  local hostile_running free_running
  hostile_running="$(aws ecs describe-services --cluster "$CLUSTER_NAME" --services "$HOSTILE_SERVICE_NAME" --region "$REGION" --query 'services[0].runningCount' --output text 2>/dev/null | tr -d '\r' | sed 's/[^0-9]//g' || echo 0)"
  free_running="$(aws ecs describe-services --cluster "$CLUSTER_NAME" --services "$FREE_SERVICE_NAME" --region "$REGION" --query 'services[0].runningCount' --output text 2>/dev/null | tr -d '\r' | sed 's/[^0-9]//g' || echo 0)"
  hostile_running="${hostile_running:-0}"
  free_running="${free_running:-0}"
  echo $((hostile_running + free_running))
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

echo "[2/5] Stabilization Gate 2 (ECS running tasks >90% full scale; BootstrapResult for diagnostics)..."
HOSTILE_DESIRED="$(aws ecs describe-services --cluster "$CLUSTER_NAME" --services "$HOSTILE_SERVICE_NAME" --region "$REGION" --query 'services[0].desiredCount' --output text | tr -d '\r' | sed 's/[^0-9]//g')"
FREE_DESIRED="$(aws ecs describe-services --cluster "$CLUSTER_NAME" --services "$FREE_SERVICE_NAME" --region "$REGION" --query 'services[0].desiredCount' --output text | tr -d '\r' | sed 's/[^0-9]//g')"
HOSTILE_DESIRED="${HOSTILE_DESIRED:-0}"
FREE_DESIRED="${FREE_DESIRED:-0}"
TOTAL_DESIRED=$((HOSTILE_DESIRED + FREE_DESIRED))
THRESHOLD=$((TOTAL_DESIRED * 90 / 100))
if [ "$THRESHOLD" -lt 1 ]; then THRESHOLD=1; fi
echo "  Target: $THRESHOLD/$TOTAL_DESIRED tasks running  (timeout: ${POLL_TIMEOUT_SECONDS}s)"

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

  printf "  [%4ds] ECS: %d/%d running  |  CW BootstrapResult(15m sum): %s  (next CW in %ds)\n" \
    "$elapsed" "$running_total" "$THRESHOLD" "$cw_val" "$cw_next"

  # Gate condition: ECS running count only (reliable; CW is diagnostic)
  if [ "$running_total" -ge "$THRESHOLD" ]; then
    echo "  ✅ Gate 2 passed: ECS running=$running_total >= threshold=$THRESHOLD  (CW bootstrap=$cw_val)"
    break
  fi

  if [ "$elapsed" -ge "$POLL_TIMEOUT_SECONDS" ]; then
    echo "ERROR: Stabilization Gate 2 timeout after ${elapsed}s  (ECS running=$running_total/$THRESHOLD  CW bootstrap=$cw_val)"
    exit 1
  fi
  sleep "$POLL_INTERVAL_SECONDS"
done

if [ "$CHAOS_NORMALIZED" = "1" ] || [ "$CHAOS_NORMALIZED" = "true" ] || [ "$CHAOS_NORMALIZED" = "yes" ] || [ "$CHAOS_NORMALIZED" = "on" ]; then
  echo "[3/5] Enabling chaos rule: $CHAOS_RULE_NAME"
  aws events enable-rule --name "$CHAOS_RULE_NAME" --region "$REGION"
else
  echo "[3/5] Chaos disabled (ENABLE_CHAOS=$ENABLE_CHAOS) - skipping chaos engine enable"
fi

if [ "$CHAOS_NORMALIZED" = "1" ] || [ "$CHAOS_NORMALIZED" = "true" ] || [ "$CHAOS_NORMALIZED" = "yes" ] || [ "$CHAOS_NORMALIZED" = "on" ]; then
  echo "[4/5] Waiting ${CHAOS_OBSERVE_SECONDS}s for chaos churn + gossip propagation..."
  sleep "$CHAOS_OBSERVE_SECONDS"
else
  echo "[4/5] Skipping chaos wait window (ENABLE_CHAOS=$ENABLE_CHAOS)"
fi

echo "[5/5] Executing upload command in creator task..."
CREATOR_TASK_ARN="$(aws ecs list-tasks --cluster "$CLUSTER_NAME" --service-name "$CREATOR_SERVICE_NAME" --desired-status RUNNING --region "$REGION" --query 'taskArns[0]' --output text)"
if [ -z "$CREATOR_TASK_ARN" ] || [ "$CREATOR_TASK_ARN" = "None" ]; then
  echo "ERROR: No running task found for creator service: $CREATOR_SERVICE_NAME"
  exit 1
fi

if aws ecs execute-command \
    --cluster "$CLUSTER_NAME" \
    --task "$CREATOR_TASK_ARN" \
    --container creator \
    --interactive \
    --command "sh -lc '$UPLOAD_COMMAND'" \
    --region "$REGION"; then
  echo "  execute-command completed."
else
  echo "  [WARN] execute-command unavailable (SSM plugin not installed or ECS Exec not yet ready)."
  echo "  Falling back to a 480s wait so the creator can complete circuit build + upload..."
  sleep 480
fi

echo ""
if [ "$CHAOS_NORMALIZED" = "1" ] || [ "$CHAOS_NORMALIZED" = "true" ] || [ "$CHAOS_NORMALIZED" = "yes" ] || [ "$CHAOS_NORMALIZED" = "on" ]; then
  echo "✅ Chaos enabled and creator upload command executed."
  echo "   Cluster: $CLUSTER_NAME"
  echo "   Chaos rule: $CHAOS_RULE_NAME"
else
  echo "✅ Creator upload command executed in stable (no-chaos) mode."
  echo "   Cluster: $CLUSTER_NAME"
fi
echo "   Creator task: $CREATOR_TASK_ARN"
