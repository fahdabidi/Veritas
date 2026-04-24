#!/usr/bin/env bash
set -euo pipefail

STACK_NAME="${GBN_BRIDGE_STACK_NAME:-gbn-bridge-phase2-dev}"
REGION="${GBN_BRIDGE_AWS_REGION:-${AWS_REGION:-us-east-1}}"
WINDOW_MINUTES="${GBN_BRIDGE_METRICS_WINDOW_MINUTES:-15}"
CHAIN_ID="${GBN_BRIDGE_CHAIN_ID:-}"

usage() {
  cat <<USAGE
Usage: $0 [--stack-name NAME] [--region REGION] [--window-minutes MINUTES] [--chain-id ID]

Collect a compact Phase 11 metrics snapshot for the V2 Conduit prototype stack.
This is a read-only helper for AWS/mobile validation runs.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --stack-name) STACK_NAME="$2"; shift 2 ;;
    --region) REGION="$2"; shift 2 ;;
    --window-minutes) WINDOW_MINUTES="$2"; shift 2 ;;
    --chain-id) CHAIN_ID="$2"; shift 2 ;;
    --help|-h) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

command -v aws >/dev/null 2>&1 || {
  echo "required command not found: aws" >&2
  exit 127
}

stack_output() {
  local key="$1"
  aws cloudformation describe-stacks \
    --region "$REGION" \
    --stack-name "$STACK_NAME" \
    --query "Stacks[0].Outputs[?OutputKey=='${key}'].OutputValue | [0]" \
    --output text
}

now_epoch="$(date +%s)"
start_epoch="$((now_epoch - (WINDOW_MINUTES * 60)))"
start_ms="$((start_epoch * 1000))"

CLUSTER="$(stack_output ClusterName)"
PUBLISHER_SERVICE="$(stack_output PublisherServiceName)"
BRIDGE_SERVICE="$(stack_output BridgeServiceName)"
PUBLISHER_LOG_GROUP="$(stack_output PublisherLogGroup)"
BRIDGE_LOG_GROUP="$(stack_output BridgeLogGroup)"
UDP_PORT="$(stack_output UdpPunchPort)"
BATCH_WINDOW="$(stack_output BatchWindowMs)"

echo "Phase 11 metrics snapshot"
echo "Stack: $STACK_NAME"
echo "Region: $REGION"
echo "WindowMinutes: $WINDOW_MINUTES"
if [[ -n "$CHAIN_ID" ]]; then
  echo "ChainIdFilter: $CHAIN_ID"
fi
echo "UdpPunchPort: $UDP_PORT"
echo "BatchWindowMs: $BATCH_WINDOW"

echo
echo "ECS services"
aws ecs describe-services \
  --region "$REGION" \
  --cluster "$CLUSTER" \
  --services "$PUBLISHER_SERVICE" "$BRIDGE_SERVICE" \
  --query "services[].{Service:serviceName,Desired:desiredCount,Running:runningCount,Pending:pendingCount,Status:status}" \
  --output table

print_log_summary() {
  local log_group="$1"
  local label="$2"
  local -a filter_args=()
  if [[ -n "$CHAIN_ID" ]]; then
    filter_args+=(--filter-pattern "\"$CHAIN_ID\"")
  fi

  echo
  echo "$label recent log stream summary"
  aws logs describe-log-streams \
    --region "$REGION" \
    --log-group-name "$log_group" \
    --order-by LastEventTime \
    --descending \
    --max-items 5 \
    --query "logStreams[].{Stream:logStreamName,LastEvent:lastEventTimestamp}" \
    --output table || true

  echo "$label recent log events"
  aws logs filter-log-events \
    --region "$REGION" \
    --log-group-name "$log_group" \
    --start-time "$start_ms" \
    --limit 20 \
    "${filter_args[@]}" \
    --query "events[].{Timestamp:timestamp,Message:message}" \
    --output table || true
}

print_log_summary "$PUBLISHER_LOG_GROUP" "Publisher"
print_log_summary "$BRIDGE_LOG_GROUP" "ExitBridge"
