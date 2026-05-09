#!/usr/bin/env bash
# Verify ECS Exec and localhost admin access for Pass 3 creator tasks.

set -euo pipefail
export AWS_PAGER=""

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STACK_NAME="${GBN_BRIDGE_STACK_NAME:-gbn-conduit-full-dev}"
AWS_REGION="${GBN_BRIDGE_AWS_REGION:-${AWS_REGION:-us-east-1}}"
ADMIN_PORT="${GBN_BRIDGE_ADMIN_PORT:-9090}"
REPORT_FILE="${VERITAS_AWS_CREATOR_EXEC_REPORT:-/tmp/conduit-aws-creator-exec-report-$(date +%Y%m%d-%H%M%S).json}"
CHECKS_JSONL="$(mktemp)"
OVERALL=0

usage() {
  cat <<'EOF'
Usage: aws-smoke-creator-exec.sh [--stack-name NAME] [--region REGION] [--report-file PATH]

Checks that creator-host and creator-new services exist, are running, have ECS
Exec IAM permissions, can execute a shell command, and can reach localhost admin
metadata/local-DHT endpoints.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --stack-name)
      STACK_NAME="$2"
      shift 2
      ;;
    --region)
      AWS_REGION="$2"
      shift 2
      ;;
    --report-file)
      REPORT_FILE="$2"
      shift 2
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

for dep in aws python3; do
  command -v "$dep" >/dev/null 2>&1 || {
    echo "ERROR: '$dep' is required." >&2
    exit 1
  }
done

record_check() {
  local name="$1" status="$2" detail="$3"
  NAME="$name" STATUS="$status" DETAIL="$detail" python3 - <<'PY' >>"$CHECKS_JSONL"
import json
import os
print(json.dumps({
    "name": os.environ["NAME"],
    "status": os.environ["STATUS"] == "true",
    "detail": os.environ["DETAIL"],
}, separators=(",", ":")))
PY
  [[ "$status" == "true" ]] || OVERALL=1
}

finish_report() {
  mkdir -p "$(dirname "$REPORT_FILE")"
  STACK_NAME="$STACK_NAME" AWS_REGION="$AWS_REGION" CHECKS_JSONL="$CHECKS_JSONL" OVERALL="$OVERALL" python3 - <<'PY' >"$REPORT_FILE"
import json
import os
from datetime import datetime, timezone

checks = []
path = os.environ["CHECKS_JSONL"]
if os.path.exists(path):
    with open(path, "r", encoding="utf-8") as handle:
        checks = [json.loads(line) for line in handle if line.strip()]
print(json.dumps({
    "stack_name": os.environ["STACK_NAME"],
    "region": os.environ["AWS_REGION"],
    "generated_at": datetime.now(timezone.utc).isoformat(),
    "ok": os.environ["OVERALL"] == "0" and all(item.get("status") for item in checks),
    "checks": checks,
}, indent=2))
PY
  echo "AWS creator ECS Exec report: $REPORT_FILE"
}

cf_output() {
  local key="$1"
  aws cloudformation describe-stacks \
    --stack-name "$STACK_NAME" \
    --region "$AWS_REGION" \
    --query "Stacks[0].Outputs[?OutputKey==\`${key}\`].OutputValue | [0]" \
    --output text 2>/dev/null | sed '/^None$/d' || true
}

ecs_exec() {
  local task_arn="$1" container="$2" command="$3" raw rc
  set +e
  if command -v timeout >/dev/null 2>&1; then
    raw="$(timeout --foreground 75 aws ecs execute-command \
      --cluster "$CLUSTER_NAME" \
      --task "$task_arn" \
      --container "$container" \
      --region "$AWS_REGION" \
      --interactive \
      --command "$command" 2>&1)"
  else
    raw="$(aws ecs execute-command \
      --cluster "$CLUSTER_NAME" \
      --task "$task_arn" \
      --container "$container" \
      --region "$AWS_REGION" \
      --interactive \
      --command "$command" 2>&1)"
  fi
  rc=$?
  set -e
  printf '%s\n' "$raw" | grep -v 'Session Manager plugin\|Starting session\|Exiting session\|installed successfully' || true
  return "$rc"
}

json_field() {
  local field="$1"
  python3 -c "import json,sys; print(json.load(sys.stdin).get('$field',''))" 2>/dev/null || true
}

check_service() {
  local label="$1" service_name="$2"
  local service_json running desired exec_enabled task_def task_role task_arn task_json container shell_output metadata role local_dht state sim denied

  if [[ -z "$service_name" ]]; then
    record_check "${label}.service_output" false "CloudFormation output missing"
    return
  fi
  record_check "${label}.service_output" true "$service_name"

  service_json="$(aws ecs describe-services \
    --cluster "$CLUSTER_NAME" \
    --services "$service_name" \
    --region "$AWS_REGION" \
    --output json 2>&1)" || {
      record_check "${label}.describe_service" false "$service_json"
      return
    }
  running="$(printf '%s' "$service_json" | python3 -c 'import json,sys; s=json.load(sys.stdin)["services"][0]; print(s.get("runningCount",0))')"
  desired="$(printf '%s' "$service_json" | python3 -c 'import json,sys; s=json.load(sys.stdin)["services"][0]; print(s.get("desiredCount",0))')"
  exec_enabled="$(printf '%s' "$service_json" | python3 -c 'import json,sys; s=json.load(sys.stdin)["services"][0]; print(str(s.get("enableExecuteCommand", False)).lower())')"
  task_def="$(printf '%s' "$service_json" | python3 -c 'import json,sys; s=json.load(sys.stdin)["services"][0]; print(s.get("taskDefinition",""))')"
  [[ "$running" -ge 1 ]] && record_check "${label}.running_count" true "running=$running desired=$desired" ||
    record_check "${label}.running_count" false "running=$running desired=$desired"
  [[ "$exec_enabled" == "true" ]] && record_check "${label}.enable_execute_command" true "enabled" ||
    record_check "${label}.enable_execute_command" false "service has EnableExecuteCommand=false"

  task_role="$(aws ecs describe-task-definition \
    --task-definition "$task_def" \
    --region "$AWS_REGION" \
    --query 'taskDefinition.taskRoleArn' \
    --output text 2>/dev/null | sed '/^None$/d' || true)"
  if [[ -z "$task_role" ]]; then
    record_check "${label}.task_role" false "task definition has no taskRoleArn"
  else
    record_check "${label}.task_role" true "$task_role"
    sim="$(aws iam simulate-principal-policy \
      --policy-source-arn "$task_role" \
      --action-names \
        ssmmessages:CreateControlChannel \
        ssmmessages:CreateDataChannel \
        ssmmessages:OpenControlChannel \
        ssmmessages:OpenDataChannel \
      --region "$AWS_REGION" \
      --output json 2>&1)" || {
        record_check "${label}.ssmmessages_policy" false "$sim"
        sim=""
      }
    if [[ -n "$sim" ]]; then
      denied="$(printf '%s' "$sim" | python3 -c 'import json,sys
data=json.load(sys.stdin)
bad=[item["EvalActionName"] + "=" + item["EvalDecision"] for item in data.get("EvaluationResults", []) if item.get("EvalDecision") != "allowed"]
print(",".join(bad))')"
      [[ -z "$denied" ]] && record_check "${label}.ssmmessages_policy" true "all ssmmessages actions allowed" ||
        record_check "${label}.ssmmessages_policy" false "$denied"
    fi
  fi

  task_arn="$(aws ecs list-tasks \
    --cluster "$CLUSTER_NAME" \
    --service-name "$service_name" \
    --desired-status RUNNING \
    --region "$AWS_REGION" \
    --query 'taskArns[0]' \
    --output text 2>/dev/null | sed '/^None$/d' || true)"
  if [[ -z "$task_arn" ]]; then
    record_check "${label}.running_task" false "no running task ARN"
    return
  fi
  record_check "${label}.running_task" true "$task_arn"

  task_json="$(aws ecs describe-tasks \
    --cluster "$CLUSTER_NAME" \
    --tasks "$task_arn" \
    --region "$AWS_REGION" \
    --output json 2>&1)" || {
      record_check "${label}.describe_task" false "$task_json"
      return
    }
  container="$(printf '%s' "$task_json" | python3 -c 'import json,sys
task=json.load(sys.stdin)["tasks"][0]
containers=task.get("containers") or []
print(containers[0].get("name","") if containers else "")')"
  if [[ -z "$container" ]]; then
    record_check "${label}.container" false "no container name found"
    return
  fi
  record_check "${label}.container" true "$container"

  shell_output="$(ecs_exec "$task_arn" "$container" "sh -lc 'echo ecs-exec-ok'" 2>&1)" &&
    [[ "$shell_output" == *"ecs-exec-ok"* ]] &&
    record_check "${label}.exec_shell" true "ecs-exec-ok" ||
    record_check "${label}.exec_shell" false "$shell_output"

  metadata="$(ecs_exec "$task_arn" "$container" "sh -lc 'curl -s http://127.0.0.1:${ADMIN_PORT}/v1/admin/node-metadata'" 2>&1)" &&
    role="$(printf '%s' "$metadata" | json_field role)" &&
    [[ "$role" == "creator" ]] &&
    record_check "${label}.node_metadata" true "$metadata" ||
    record_check "${label}.node_metadata" false "$metadata"

  local_dht="$(ecs_exec "$task_arn" "$container" "sh -lc 'curl -s http://127.0.0.1:${ADMIN_PORT}/v1/admin/local-dht'" 2>&1)" &&
    state="$(printf '%s' "$local_dht" | json_field self_onboarding_state)" &&
    [[ -n "$state" ]] &&
    record_check "${label}.local_dht" true "$local_dht" ||
    record_check "${label}.local_dht" false "$local_dht"
}

CLUSTER_NAME="$(cf_output ClusterName)"
CREATOR_HOST_SERVICE="$(cf_output CreatorHostServiceName)"
CREATOR_NEW_SERVICE="$(cf_output CreatorNewServiceName)"

if [[ -z "$CLUSTER_NAME" ]]; then
  record_check "cluster_output" false "CloudFormation output ClusterName missing for $STACK_NAME"
  finish_report
  exit 1
fi
record_check "cluster_output" true "$CLUSTER_NAME"

check_service creator-host "$CREATOR_HOST_SERVICE"
check_service creator-new "$CREATOR_NEW_SERVICE"

finish_report
exit "$OVERALL"
