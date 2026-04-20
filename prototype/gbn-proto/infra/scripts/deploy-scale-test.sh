#!/usr/bin/env bash
# deploy-scale-test.sh — Deploy Phase 1 scale stack, seed at configurable percentage,
# wait for bootstrap, then scale to full target.
#
# Usage: ./deploy-scale-test.sh <stack-name> [scale-target] [region]
#        ENABLE_CHAOS=1 bash deploy-scale-test.sh gbn-proto-phase1-scale-n100 100 us-east-1
# ENABLE_CHAOS=1 CHAOS_ENABLE_DELAY_SECONDS=180 CHAOS_HOSTILE_CHURN_RATE=0.4 CHAOS_FREE_CHURN_RATE=0.2 bash prototype/gbn-proto/infra/scripts/deploy-scale-test.sh gbn-proto-phase1-scale-n100 100 us-east-1

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

format_duration() {
  local total="${1:-0}"
  if [ "$total" -lt 0 ]; then
    total=0
  fi
  local minutes=$((total / 60))
  local seconds=$((total % 60))
  printf "%02dm%02ds" "$minutes" "$seconds"
}

show_countdown() {
  local elapsed="$1"
  local budget="$2"
  local remaining=$((budget - elapsed))
  if [ "$remaining" -lt 0 ]; then
    remaining=0
  fi
  printf "elapsed=%s eta~%s" "$(format_duration "$elapsed")" "$(format_duration "$remaining")"
}

latest_stack_progress() {
  local stack_status latest_event
  stack_status="$(aws cloudformation describe-stacks \
    --stack-name "$STACK_NAME" \
    --region "$REGION" \
    --query 'Stacks[0].StackStatus' \
    --output text 2>/dev/null || echo unknown)"
  latest_event="$(aws cloudformation describe-stack-events \
    --stack-name "$STACK_NAME" \
    --region "$REGION" \
    --query "StackEvents[?contains(ResourceStatus, 'IN_PROGRESS')]|[0].[LogicalResourceId,ResourceType,ResourceStatus,ResourceStatusReason]" \
    --output text 2>/dev/null || true)"
  if [ -n "$latest_event" ] && [ "$latest_event" != "None" ]; then
    printf "stack=%s current=%s" "$stack_status" "$latest_event"
  else
    printf "stack=%s" "$stack_status"
  fi
}

run_cloudformation_deploy_with_progress() {
  local log_file pid start_ts elapsed rc
  log_file="$(mktemp)"

  aws cloudformation deploy \
    --stack-name "$STACK_NAME" \
    --template-file "$TEMPLATE_PATH" \
    --capabilities CAPABILITY_IAM \
    --no-fail-on-empty-changeset \
    --parameter-overrides "ScaleTarget=$SCALE_TARGET" \
                          "PublisherPrivKeyHex=$PUBLISHER_KEY_HEX" \
                          "PublisherPubKeyHex=$PUBLISHER_PUB_HEX" \
                          "SeedRelayNoisePrivKey=$SEED_RELAY_NOISE_PRIVKEY" \
                          "SeedRelayKeyName=$SEED_RELAY_KEY_NAME" \
                          "AdminCidr=$ADMIN_CIDR" \
    --region "$REGION" >"$log_file" 2>&1 &
  pid=$!
  start_ts=$(date +%s)

  while kill -0 "$pid" >/dev/null 2>&1; do
    elapsed=$(( $(date +%s) - start_ts ))
    printf "  [CFN %s] %s\n" "$(show_countdown "$elapsed" 900)" "$(latest_stack_progress)"
    sleep 15
  done

  if wait "$pid"; then
    rm -f "$log_file"
    return 0
  fi
  rc=$?
  cat "$log_file"
  rm -f "$log_file"
  return "$rc"
}

STACK_NAME="${1:?Usage: $0 <stack-name> [scale-target] [region]}"
SCALE_TARGET="${2:-100}"
REGION="${3:-us-east-1}"
POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-10}"
POLL_TIMEOUT_SECONDS="${POLL_TIMEOUT_SECONDS:-1200}"
SEED_PERCENT="${SEED_PERCENT:-30}"
SMOKE_TOPOLOGY="${SMOKE_TOPOLOGY:-0}"
ENABLE_CHAOS="${ENABLE_CHAOS:-0}"
CHAOS_ENABLE_DELAY_SECONDS="${CHAOS_ENABLE_DELAY_SECONDS:-0}"
CHAOS_HOSTILE_CHURN_RATE="${CHAOS_HOSTILE_CHURN_RATE:-0.4}"
CHAOS_FREE_CHURN_RATE="${CHAOS_FREE_CHURN_RATE:-0.2}"
SEED_RELAY_KEY_NAME="${SEED_RELAY_KEY_NAME:-}"
ADMIN_CIDR="${ADMIN_CIDR:-0.0.0.0/0}"
RESTART_STATIC_NODES="${RESTART_STATIC_NODES:-1}"
AUTO_BUILD_PUSH_IF_ECR_EMPTY="${AUTO_BUILD_PUSH_IF_ECR_EMPTY:-1}"
STOP_ECS_TASKS_BEFORE_DEPLOY="${STOP_ECS_TASKS_BEFORE_DEPLOY:-0}"

if [ "$SEED_PERCENT" -lt 1 ] || [ "$SEED_PERCENT" -gt 99 ]; then
  echo "ERROR: SEED_PERCENT must be between 1 and 99."
  exit 1
fi

if ! [[ "$CHAOS_ENABLE_DELAY_SECONDS" =~ ^[0-9]+$ ]]; then
  echo "ERROR: CHAOS_ENABLE_DELAY_SECONDS must be a non-negative integer."
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

validate_rate() {
  local name="$1"
  local value="$2"
  python3 - "$name" "$value" <<'PY'
import sys
name, raw = sys.argv[1], sys.argv[2]
try:
    value = float(raw)
except ValueError:
    print(f"ERROR: {name} must be a float between 0.0 and 1.0.")
    sys.exit(1)
if not (0.0 <= value <= 1.0):
    print(f"ERROR: {name} must be between 0.0 and 1.0.")
    sys.exit(1)
PY
}

validate_rate "CHAOS_HOSTILE_CHURN_RATE" "$CHAOS_HOSTILE_CHURN_RATE"
validate_rate "CHAOS_FREE_CHURN_RATE" "$CHAOS_FREE_CHURN_RATE"

configure_chaos_lambda() {
  local function_name="$1"
  local current_env_json merged_env_json

  if [ -z "$function_name" ] || [ "$function_name" = "None" ]; then
    echo "  Chaos lambda not found; skipping configuration."
    return 0
  fi

  current_env_json="$(aws lambda get-function-configuration \
    --function-name "$function_name" \
    --region "$REGION" \
    --query 'Environment.Variables' \
    --output json 2>/dev/null || echo '{}')"

  merged_env_json="$(
    CURRENT_ENV_JSON="$current_env_json" \
    CHAOS_HOSTILE_CHURN_RATE="$CHAOS_HOSTILE_CHURN_RATE" \
    CHAOS_FREE_CHURN_RATE="$CHAOS_FREE_CHURN_RATE" \
    python3 -c 'import json, os
try:
    env = json.loads(os.environ.get("CURRENT_ENV_JSON", "{}") or "{}")
except Exception:
    env = {}
env["HOSTILE_CHURN_RATE"] = os.environ["CHAOS_HOSTILE_CHURN_RATE"]
env["FREE_CHURN_RATE"] = os.environ["CHAOS_FREE_CHURN_RATE"]
print(json.dumps({"Variables": env}, separators=(",", ":")))' 
  )"

  if [ -z "$merged_env_json" ] || [ "$merged_env_json" = "None" ]; then
    echo "ERROR: Failed to build merged lambda environment for $function_name."
    return 1
  fi

  aws lambda update-function-configuration \
    --function-name "$function_name" \
    --region "$REGION" \
    --environment "$merged_env_json" >/dev/null
  aws lambda wait function-updated \
    --function-name "$function_name" \
    --region "$REGION"

  echo "  Chaos lambda configured: $function_name"
  echo "    Hostile churn rate: $CHAOS_HOSTILE_CHURN_RATE"
  echo "    Free churn rate:    $CHAOS_FREE_CHURN_RATE"
}

set_chaos_rule_state() {
  local rule_name="$1"
  local enable_flag="$2"

  if [ -z "$rule_name" ] || [ "$rule_name" = "None" ]; then
    echo "  Chaos rule not found; skipping state change."
    return 0
  fi

  if [ "$enable_flag" = "1" ]; then
    aws events enable-rule --name "$rule_name" --region "$REGION"
    echo "  Chaos enabled: $rule_name"
  else
    aws events disable-rule --name "$rule_name" --region "$REGION" || true
    echo "  Chaos disabled: $rule_name"
  fi
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

wait_for_seed_relay_container() {
  local instance_id="$1"
  local max_wait="${2:-300}"
  local interval=10
  local elapsed=0

  while [ "$elapsed" -lt "$max_wait" ]; do
    local cmd_id status out
    cmd_id="$(aws ssm send-command \
      --region "$REGION" \
      --instance-ids "$instance_id" \
      --document-name "AWS-RunShellScript" \
      --parameters 'commands=["docker ps --filter name=gbn-seed-relay --format \"{{.Names}}\""]' \
      --query 'Command.CommandId' \
      --output text 2>/dev/null || true)"

    if [ -n "$cmd_id" ] && [ "$cmd_id" != "None" ]; then
      aws ssm wait command-executed \
        --region "$REGION" \
        --command-id "$cmd_id" \
        --instance-id "$instance_id" 2>/dev/null || true

      status="$(aws ssm get-command-invocation \
        --region "$REGION" \
        --command-id "$cmd_id" \
        --instance-id "$instance_id" \
        --query 'Status' \
        --output text 2>/dev/null || true)"
      out="$(aws ssm get-command-invocation \
        --region "$REGION" \
        --command-id "$cmd_id" \
        --instance-id "$instance_id" \
        --query 'StandardOutputContent' \
        --output text 2>/dev/null || true)"

      if [ "$status" = "Success" ] && printf '%s' "$out" | grep -q "gbn-seed-relay"; then
        return 0
      fi
    fi

    printf "  [SeedRelay readiness %s] docker ps has not reported gbn-seed-relay yet\n" \
      "$(show_countdown "$elapsed" "$max_wait")"
    sleep "$interval"
    elapsed=$((elapsed + interval))
  done

  return 1
}

list_service_tasks() {
  local cluster_name="$1"
  local service_name="$2"
  aws ecs list-tasks \
    --cluster "$cluster_name" \
    --service-name "$service_name" \
    --desired-status RUNNING \
    --region "$REGION" \
    --query 'taskArns[]' \
    --output text 2>/dev/null || true
}

stop_service_tasks() {
  local cluster_name="$1"
  local service_name="$2"
  local reason="$3"
  local tasks task_count=0
  local max_parallel="${ECS_STOP_PARALLELISM:-20}"
  local -a pids=()
  local failures=0

  tasks="$(list_service_tasks "$cluster_name" "$service_name")"
  if [ -z "$tasks" ] || [ "$tasks" = "None" ]; then
    echo "  $service_name: no running tasks to stop"
    return 0
  fi

  echo "  $service_name: stopping running tasks before force-new-deployment..."
  while IFS= read -r task_arn; do
    [ -n "$task_arn" ] || continue
    (
      aws ecs stop-task \
        --cluster "$cluster_name" \
        --task "$task_arn" \
        --reason "$reason" \
        --region "$REGION" >/dev/null
      echo "    stopped ${task_arn##*/}"
    ) &
    pids+=("$!")
    task_count=$((task_count + 1))
    if [ "${#pids[@]}" -ge "$max_parallel" ]; then
      if ! wait "${pids[0]}"; then
        failures=$((failures + 1))
      fi
      pids=("${pids[@]:1}")
    fi
  done < <(printf '%s\n' "$tasks" | tr '\t' '\n')

  for pid in "${pids[@]}"; do
    if ! wait "$pid"; then
      failures=$((failures + 1))
    fi
  done

  echo "  $service_name: stopped $task_count task(s)"
  if [ "$failures" -gt 0 ]; then
    echo "ERROR: $service_name failed to stop $failures task(s)"
    return 1
  fi
}

redeploy_service() {
  local cluster_name="$1"
  local service_name="$2"
  local desired_count="$3"

  if [ "$STOP_ECS_TASKS_BEFORE_DEPLOY" = "1" ]; then
    stop_service_tasks "$cluster_name" "$service_name" "GBN stop-before-redeploy"
  fi

  aws ecs update-service \
    --cluster "$cluster_name" \
    --service "$service_name" \
    --desired-count "$desired_count" \
    --enable-execute-command \
    --force-new-deployment \
    --region "$REGION" >/dev/null
}

echo "============================================"
echo "  GBN Phase 1 — Deploy Scale Test"
echo "  Stack:  $STACK_NAME"
echo "  Scale:  $SCALE_TARGET"
echo "  Region: $REGION"
if [ "$SMOKE_TOPOLOGY" = "1" ]; then
  echo "  Mode:   smoke topology override"
else
  echo "  Mode:   seeded scale deployment"
fi
if [ "$ENABLE_CHAOS" = "1" ]; then
  echo "  Chaos:  enable scheduled churn after deploy"
else
  echo "  Chaos:  leave scheduled churn disabled"
fi
echo "  Chaos enable delay:   ${CHAOS_ENABLE_DELAY_SECONDS}s"
echo "  Hostile churn rate:   $CHAOS_HOSTILE_CHURN_RATE"
echo "  Free churn rate:      $CHAOS_FREE_CHURN_RATE"
echo "============================================"

echo "[1/6] Generating static cryptographic keys..."
cd "$PROTO_ROOT"
if [ ! -f "publisher.key" ]; then
    echo "  Generating new Publisher keys..."
    cargo run --release --bin proto-cli -- keygen > /dev/null 2>&1
fi
PUBLISHER_KEY_HEX="$(xxd -p -c 32 publisher.key)"
PUBLISHER_PUB_HEX="$(xxd -p -c 32 publisher.pub)"
SEED_RELAY_NOISE_PRIVKEY="$(openssl rand -hex 32)"
cd "$SCRIPT_DIR"

echo "[2/6] Deploying CloudFormation stack..."
run_cloudformation_deploy_with_progress

# Ensure relay image exists before restarting static EC2 nodes that docker-pull :latest.
ECR_RELAY_REPO_PRE="$(aws cloudformation describe-stacks --stack-name "$STACK_NAME" --region "$REGION" --output json | \
  python3 -c "import json,sys; d=json.load(sys.stdin); o=d['Stacks'][0].get('Outputs',[]); print(next((x['OutputValue'] for x in o if x.get('OutputKey')=='ECRUriRelay'), ''))" 2>/dev/null || true)"
RELAY_REPO_NAME_PRE="${ECR_RELAY_REPO_PRE##*/}"
ECR_IMAGE_COUNT_PRE=0
if [ -n "$RELAY_REPO_NAME_PRE" ]; then
  ECR_IMAGE_COUNT_PRE="$(aws ecr list-images --repository-name "$RELAY_REPO_NAME_PRE" --region "$REGION" \
    --query 'length(imageIds)' --output text 2>/dev/null || echo 0)"
  ECR_IMAGE_COUNT_PRE="${ECR_IMAGE_COUNT_PRE:-0}"
fi
if [ "$ECR_IMAGE_COUNT_PRE" = "0" ] || [ "$ECR_IMAGE_COUNT_PRE" = "None" ]; then
  if [ "$AUTO_BUILD_PUSH_IF_ECR_EMPTY" = "1" ]; then
    echo "[2.5/6] ECR relay repo empty pre-static-restart; running build-and-push..."
    bash "$SCRIPT_DIR/build-and-push.sh" "$STACK_NAME" "$REGION"
  else
    echo "ERROR: ECR relay repo empty before static node restart and AUTO_BUILD_PUSH_IF_ECR_EMPTY=0"
    exit 1
  fi
fi

if [ "$RESTART_STATIC_NODES" = "1" ]; then
  echo "[3/7] Ensuring static EC2 SeedRelay/Publisher run with host networking..."
  echo "  Static-node recovery policy: reboot when SSM is Online, stop/start when SSM is ConnectionLost/disconnected."
  bash "$SCRIPT_DIR/restart-static-nodes.sh" "$STACK_NAME" "$REGION"
else
  echo "[3/7] Skipping static node host-network restart (RESTART_STATIC_NODES=$RESTART_STATIC_NODES)."
fi

SEED_RELAY_INSTANCE_ID="$(cf_resource_id SeedRelayInstance)"
if [ -z "$SEED_RELAY_INSTANCE_ID" ] || [ "$SEED_RELAY_INSTANCE_ID" = "None" ]; then
  echo "ERROR: Failed to resolve SeedRelayInstance resource ID."
  exit 1
fi
echo "[3.5/7] Waiting for SeedRelay container readiness before scaling ECS services..."
if ! wait_for_seed_relay_container "$SEED_RELAY_INSTANCE_ID" "$POLL_TIMEOUT_SECONDS"; then
  echo "ERROR: Seed relay container did not become ready within ${POLL_TIMEOUT_SECONDS}s ($SEED_RELAY_INSTANCE_ID)."
  exit 1
fi

echo "[4/7] Resolving ECS services..."
CLUSTER_NAME="$(cf_resource_id ECSCluster)"
HOSTILE_SERVICE_NAME="$(cf_resource_id HostileRelayService)"
FREE_SERVICE_NAME="$(cf_resource_id FreeRelayService)"
CREATOR_SERVICE_NAME="$(cf_resource_id CreatorService)"
CHAOS_LAMBDA_NAME="$(cf_resource_id ChaosControllerLambda)"
CHAOS_RULE_NAME="$(cf_resource_id ChaosEngineRule)"
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
  if [ "$AUTO_BUILD_PUSH_IF_ECR_EMPTY" = "1" ]; then
    echo ""
    echo "⚠️  ECR repository '$RELAY_REPO_NAME' has no images yet."
    echo "   AUTO_BUILD_PUSH_IF_ECR_EMPTY=1, running build-and-push automatically..."
    bash "$SCRIPT_DIR/build-and-push.sh" "$STACK_NAME" "$REGION"
    ECR_IMAGE_COUNT="$(aws ecr list-images --repository-name "$RELAY_REPO_NAME" --region "$REGION" \
      --query 'length(imageIds)' --output text 2>/dev/null || echo 0)"
    ECR_IMAGE_COUNT="${ECR_IMAGE_COUNT:-0}"
  else
    echo ""
    echo "⚠️  ECR repository '$RELAY_REPO_NAME' has no images yet."
    echo "   Run build-and-push.sh first, then re-run this script:"
    echo "   bash infra/scripts/build-and-push.sh $STACK_NAME $REGION"
    echo "   bash infra/scripts/deploy-scale-test.sh $STACK_NAME $SCALE_TARGET $REGION"
    echo ""
    echo "   Stack created successfully — ECR repos and all other resources are ready."
    exit 0
  fi
fi

if [ "$ECR_IMAGE_COUNT" = "0" ] || [ "$ECR_IMAGE_COUNT" = "None" ]; then
  echo "ERROR: ECR repository '$RELAY_REPO_NAME' is still empty after build-and-push."
  exit 1
fi

SEED_COUNT=$((SCALE_TARGET * SEED_PERCENT / 100))
if [ "$SEED_COUNT" -lt 1 ]; then SEED_COUNT=1; fi
HOSTILE_SEED=$((SEED_COUNT * 9 / 10))
if [ "$HOSTILE_SEED" -lt 1 ]; then HOSTILE_SEED=1; fi
FREE_SEED=$((SEED_COUNT - HOSTILE_SEED))
if [ "$FREE_SEED" -lt 1 ]; then FREE_SEED=1; fi

FULL_HOSTILE=$((SCALE_TARGET * 9 / 10))
FULL_FREE=$((SCALE_TARGET - FULL_HOSTILE))

if [ "$SMOKE_TOPOLOGY" = "1" ]; then
  # Smoke test topology override:
  HOSTILE_SEED=2
  FREE_SEED=1
  echo "  SMOKE_TOPOLOGY enabled: using 2 hostile + 1 free seed relays"
fi

GATE_SEED_TASKS=$((HOSTILE_SEED + FREE_SEED))

if [ "$SMOKE_TOPOLOGY" = "1" ]; then
  echo "[5/7] Scaling to smoke topology (2 hostile, 1 free) + 1 creator..."
else
  echo "[5/7] Scaling to seed topology + 1 creator..."
  echo "  Seed percent:   $SEED_PERCENT%"
  echo "  Seed relay:     1 static EC2 node (unchanged)"
fi
echo "  Hostile relays: $HOSTILE_SEED"
echo "  Free relays:    $FREE_SEED"
redeploy_service "$CLUSTER_NAME" "$HOSTILE_SERVICE_NAME" "$HOSTILE_SEED" &
hostile_redeploy_pid=$!
redeploy_service "$CLUSTER_NAME" "$FREE_SERVICE_NAME" "$FREE_SEED" &
free_redeploy_pid=$!
if [ -n "$CREATOR_SERVICE_NAME" ] && [ "$CREATOR_SERVICE_NAME" != "None" ]; then
  redeploy_service "$CLUSTER_NAME" "$CREATOR_SERVICE_NAME" 1 &
  creator_redeploy_pid=$!
  echo "  Creator: 1"
fi
wait "$hostile_redeploy_pid"
wait "$free_redeploy_pid"
if [ -n "${creator_redeploy_pid:-}" ]; then
  wait "$creator_redeploy_pid"
fi

echo "[6/7] Stabilization Gate 1 (ECS running tasks >= 90% seed; BootstrapResult for diagnostics)..."
if [ "$SMOKE_TOPOLOGY" = "1" ]; then
  SEED_THRESHOLD=$((GATE_SEED_TASKS))
else
  SEED_THRESHOLD=$((GATE_SEED_TASKS * 90 / 100))
fi
if [ "$SEED_THRESHOLD" -lt 1 ]; then SEED_THRESHOLD=1; fi
echo "  Target: $SEED_THRESHOLD/$GATE_SEED_TASKS tasks running  (timeout: ${POLL_TIMEOUT_SECONDS}s)"

start_ts=$(date +%s)
last_cw_ts=0
cw_val="--"
hostile_running=0
free_running=0
creator_running=0

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

  hostile_running="$(aws ecs describe-services --cluster "$CLUSTER_NAME" --services "$HOSTILE_SERVICE_NAME" --region "$REGION" --query 'services[0].runningCount' --output text 2>/dev/null || echo 0)"
  free_running="$(aws ecs describe-services --cluster "$CLUSTER_NAME" --services "$FREE_SERVICE_NAME" --region "$REGION" --query 'services[0].runningCount' --output text 2>/dev/null || echo 0)"
  creator_running=0
  if [ -n "${CREATOR_SERVICE_NAME:-}" ] && [ "$CREATOR_SERVICE_NAME" != "None" ]; then
    creator_running="$(aws ecs describe-services --cluster "$CLUSTER_NAME" --services "$CREATOR_SERVICE_NAME" --region "$REGION" --query 'services[0].runningCount' --output text 2>/dev/null || echo 0)"
  fi

  printf "  [Gate1 %s] ECS total=%d/%d hostile=%s free=%s creator=%s | CW BootstrapResult(10m sum)=%s (next CW in %ds)\n" \
    "$(show_countdown "$elapsed" "$POLL_TIMEOUT_SECONDS")" "$running_total" "$SEED_THRESHOLD" "$hostile_running" "$free_running" "$creator_running" "$cw_val" "$cw_next"

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

if [ "$SMOKE_TOPOLOGY" = "1" ]; then
  echo "[7/7] Smoke topology active: skipping full-scale expansion."
  echo "  Hostile retained: $HOSTILE_SEED"
  echo "  Free retained:    $FREE_SEED"
  echo ""
  echo "✅ Smoke topology deployed and scaled to 2 hostile + 1 free + 1 creator."
else
  echo "[7/7] Scaling to full target..."
  echo "  Hostile full: $FULL_HOSTILE"
  echo "  Free full:    $FULL_FREE"
  redeploy_service "$CLUSTER_NAME" "$HOSTILE_SERVICE_NAME" "$FULL_HOSTILE" &
  hostile_full_pid=$!
  redeploy_service "$CLUSTER_NAME" "$FREE_SERVICE_NAME" "$FULL_FREE" &
  free_full_pid=$!
  wait "$hostile_full_pid"
  wait "$free_full_pid"

  echo ""
  echo "✅ Scale test stack deployed and scaled to full target."
fi
echo "[8/8] Applying chaos engine state..."
configure_chaos_lambda "$CHAOS_LAMBDA_NAME"
if [ "$ENABLE_CHAOS" = "1" ] && [ "$CHAOS_ENABLE_DELAY_SECONDS" -gt 0 ]; then
  echo "  Waiting ${CHAOS_ENABLE_DELAY_SECONDS}s before enabling chaos rule..."
  delay_elapsed=0
  while [ "$delay_elapsed" -lt "$CHAOS_ENABLE_DELAY_SECONDS" ]; do
    echo "  [Chaos delay $(show_countdown "$delay_elapsed" "$CHAOS_ENABLE_DELAY_SECONDS")]"
    sleep 5
    delay_elapsed=$((delay_elapsed + 5))
  done
fi
set_chaos_rule_state "$CHAOS_RULE_NAME" "$ENABLE_CHAOS"
echo "   Cluster: $CLUSTER_NAME"
echo "   Hostile service: $HOSTILE_SERVICE_NAME"
echo "   Free service:    $FREE_SERVICE_NAME"
