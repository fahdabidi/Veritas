#!/usr/bin/env bash
set -euo pipefail

STACK_NAME="${GBN_BRIDGE_STACK_NAME:-gbn-conduit-full-dev}"
REGION="${GBN_BRIDGE_AWS_REGION:-${AWS_REGION:-us-east-1}}"

usage() {
  cat <<USAGE
Usage: $0 [--stack-name NAME] [--region REGION]

Print a Conduit full-stack smoke snapshot: stack outputs, ECS service desired/running counts,
and the log groups that should preserve chain_id evidence.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --stack-name) STACK_NAME="$2"; shift 2 ;;
    --region) REGION="$2"; shift 2 ;;
    --help|-h) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ "$STACK_NAME" != gbn-conduit-full-* ]]; then
  echo "stack name must start with gbn-conduit-full-: $STACK_NAME" >&2
  exit 2
fi

command -v aws >/dev/null 2>&1 || {
  echo "required command not found: aws" >&2
  exit 127
}
command -v python3 >/dev/null 2>&1 || {
  echo "required command not found: python3" >&2
  exit 127
}

STACK_JSON="$(aws cloudformation describe-stacks --region "$REGION" --stack-name "$STACK_NAME" --output json)"
CLUSTER_NAME="$(printf '%s' "$STACK_JSON" | aws --region "$REGION" cloudformation describe-stacks --stack-name "$STACK_NAME" --query 'Stacks[0].Outputs[?OutputKey==`ClusterName`].OutputValue' --output text)"
AUTHORITY_SERVICE="$(printf '%s' "$STACK_JSON" | aws --region "$REGION" cloudformation describe-stacks --stack-name "$STACK_NAME" --query 'Stacks[0].Outputs[?OutputKey==`AuthorityServiceName`].OutputValue' --output text)"
RECEIVER_SERVICE="$(printf '%s' "$STACK_JSON" | aws --region "$REGION" cloudformation describe-stacks --stack-name "$STACK_NAME" --query 'Stacks[0].Outputs[?OutputKey==`ReceiverServiceName`].OutputValue' --output text)"
BRIDGE_SERVICE="$(printf '%s' "$STACK_JSON" | aws --region "$REGION" cloudformation describe-stacks --stack-name "$STACK_NAME" --query 'Stacks[0].Outputs[?OutputKey==`BridgeServiceName`].OutputValue' --output text)"
CREATOR_HOST_SERVICE="$(printf '%s' "$STACK_JSON" | aws --region "$REGION" cloudformation describe-stacks --stack-name "$STACK_NAME" --query 'Stacks[0].Outputs[?OutputKey==`CreatorHostServiceName`].OutputValue' --output text)"
CREATOR_NEW_SERVICE="$(printf '%s' "$STACK_JSON" | aws --region "$REGION" cloudformation describe-stacks --stack-name "$STACK_NAME" --query 'Stacks[0].Outputs[?OutputKey==`CreatorNewServiceName`].OutputValue' --output text)"
AUTHORITY_URL="$(printf '%s' "$STACK_JSON" | aws --region "$REGION" cloudformation describe-stacks --stack-name "$STACK_NAME" --query 'Stacks[0].Outputs[?OutputKey==`AuthorityInternalUrl`].OutputValue' --output text)"
RECEIVER_URL="$(printf '%s' "$STACK_JSON" | aws --region "$REGION" cloudformation describe-stacks --stack-name "$STACK_NAME" --query 'Stacks[0].Outputs[?OutputKey==`ReceiverInternalUrl`].OutputValue' --output text)"
CONTROL_URL="$(printf '%s' "$STACK_JSON" | aws --region "$REGION" cloudformation describe-stacks --stack-name "$STACK_NAME" --query 'Stacks[0].Outputs[?OutputKey==`ControlUrl`].OutputValue' --output text)"
AUTHORITY_LOG_GROUP="$(printf '%s' "$STACK_JSON" | aws --region "$REGION" cloudformation describe-stacks --stack-name "$STACK_NAME" --query 'Stacks[0].Outputs[?OutputKey==`AuthorityLogGroup`].OutputValue' --output text)"
RECEIVER_LOG_GROUP="$(printf '%s' "$STACK_JSON" | aws --region "$REGION" cloudformation describe-stacks --stack-name "$STACK_NAME" --query 'Stacks[0].Outputs[?OutputKey==`ReceiverLogGroup`].OutputValue' --output text)"
BRIDGE_LOG_GROUP="$(printf '%s' "$STACK_JSON" | aws --region "$REGION" cloudformation describe-stacks --stack-name "$STACK_NAME" --query 'Stacks[0].Outputs[?OutputKey==`BridgeLogGroup`].OutputValue' --output text)"
CREATOR_LOG_GROUP="$(printf '%s' "$STACK_JSON" | aws --region "$REGION" cloudformation describe-stacks --stack-name "$STACK_NAME" --query 'Stacks[0].Outputs[?OutputKey==`CreatorLogGroup`].OutputValue' --output text)"

SERVICE_STATUS_JSON="$(aws ecs describe-services \
  --region "$REGION" \
  --cluster "$CLUSTER_NAME" \
  --services "$AUTHORITY_SERVICE" "$RECEIVER_SERVICE" "$BRIDGE_SERVICE" "$CREATOR_HOST_SERVICE" "$CREATOR_NEW_SERVICE" \
  --output json)"

aws ecs describe-services \
  --region "$REGION" \
  --cluster "$CLUSTER_NAME" \
  --services "$AUTHORITY_SERVICE" "$RECEIVER_SERVICE" "$BRIDGE_SERVICE" "$CREATOR_HOST_SERVICE" "$CREATOR_NEW_SERVICE" \
  --query 'services[].{serviceName:serviceName,desired:desiredCount,running:runningCount,status:status}' \
  --output table

UNHEALTHY_SERVICES="$(printf '%s' "$SERVICE_STATUS_JSON" | python3 -c 'import json,sys
data=json.load(sys.stdin)
for svc in data.get("services", []):
    if svc.get("runningCount", 0) < svc.get("desiredCount", 0):
        print("%s: desired=%s running=%s" % (svc.get("serviceName"), svc.get("desiredCount"), svc.get("runningCount")))')"

if [[ -n "$UNHEALTHY_SERVICES" ]]; then
  echo "one or more Conduit services are below desired running count:" >&2
  printf '%s\n' "$UNHEALTHY_SERVICES" >&2
  exit 1
fi

cat <<OUTPUTS
AuthorityInternalUrl=$AUTHORITY_URL
ReceiverInternalUrl=$RECEIVER_URL
ControlUrl=$CONTROL_URL
AuthorityLogGroup=$AUTHORITY_LOG_GROUP
ReceiverLogGroup=$RECEIVER_LOG_GROUP
BridgeLogGroup=$BRIDGE_LOG_GROUP
CreatorLogGroup=$CREATOR_LOG_GROUP
OUTPUTS
