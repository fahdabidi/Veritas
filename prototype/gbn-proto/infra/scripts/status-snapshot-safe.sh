#!/usr/bin/env bash
# status-snapshot-safe.sh - quoting-safe status snapshot for stack + ECS + static EC2 nodes
#
# Usage:
#   ./status-snapshot-safe.sh <stack-name> [region]
#   STACK_NAME=<stack-name> AWS_REGION=<region> ./status-snapshot-safe.sh

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

STACK_NAME="${1:-${STACK_NAME:-}}"
REGION="${2:-${AWS_REGION:-us-east-1}}"

if [ -z "$STACK_NAME" ]; then
  echo "Usage: $0 <stack-name> [region]"
  echo "Or set STACK_NAME environment variable."
  exit 1
fi

cf_field() {
  local logical_id="$1"
  local field="$2"
  aws cloudformation describe-stack-resources \
    --stack-name "$STACK_NAME" \
    --logical-resource-id "$logical_id" \
    --region "$REGION" \
    --query "StackResources[0].${field}" \
    --output text 2>/dev/null || true
}

echo "============================================"
echo "  GBN Status Snapshot (safe)"
echo "  Stack:  $STACK_NAME"
echo "  Region: $REGION"
echo "============================================"

SEED_INSTANCE_ID="$(cf_field SeedRelayInstance PhysicalResourceId)"
SEED_INSTANCE_STATUS="$(cf_field SeedRelayInstance ResourceStatus)"
PUBLISHER_INSTANCE_ID="$(cf_field PublisherInstance PhysicalResourceId)"
PUBLISHER_INSTANCE_STATUS="$(cf_field PublisherInstance ResourceStatus)"
CLUSTER_NAME="$(cf_field ECSCluster PhysicalResourceId)"

echo ""
echo "[EC2 static nodes from CloudFormation]"
echo "  SeedRelayInstance:   ${SEED_INSTANCE_ID:-None}  (${SEED_INSTANCE_STATUS:-None})"
echo "  PublisherInstance:   ${PUBLISHER_INSTANCE_ID:-None}  (${PUBLISHER_INSTANCE_STATUS:-None})"

if [ -n "${SEED_INSTANCE_ID:-}" ] && [ "$SEED_INSTANCE_ID" != "None" ]; then
  echo ""
  echo "[SeedRelay EC2 state]"
  aws ec2 describe-instances \
    --instance-ids "$SEED_INSTANCE_ID" \
    --region "$REGION" \
    --query 'Reservations[0].Instances[0].[State.Name,PrivateIpAddress,PublicIpAddress,SubnetId,VpcId]' \
    --output table || true
fi

if [ -n "${PUBLISHER_INSTANCE_ID:-}" ] && [ "$PUBLISHER_INSTANCE_ID" != "None" ]; then
  echo ""
  echo "[Publisher EC2 state]"
  aws ec2 describe-instances \
    --instance-ids "$PUBLISHER_INSTANCE_ID" \
    --region "$REGION" \
    --query 'Reservations[0].Instances[0].[State.Name,PrivateIpAddress,PublicIpAddress,SubnetId,VpcId]' \
    --output table || true
fi

echo ""
echo "[ECS cluster + services]"
if [ -z "${CLUSTER_NAME:-}" ] || [ "$CLUSTER_NAME" = "None" ]; then
  echo "  ECSCluster not found in stack resources."
  exit 0
fi

echo "  Cluster: $CLUSTER_NAME"
SERVICE_ARNS="$(aws ecs list-services --cluster "$CLUSTER_NAME" --region "$REGION" --query 'serviceArns' --output text 2>/dev/null || true)"
if [ -z "${SERVICE_ARNS:-}" ] || [ "$SERVICE_ARNS" = "None" ]; then
  echo "  No ECS services found."
  exit 0
fi

aws ecs describe-services \
  --cluster "$CLUSTER_NAME" \
  --services $SERVICE_ARNS \
  --region "$REGION" \
  --query 'services[*].[serviceName,desiredCount,runningCount,pendingCount,status]' \
  --output table

echo ""
echo "[Done] Snapshot collected successfully."

