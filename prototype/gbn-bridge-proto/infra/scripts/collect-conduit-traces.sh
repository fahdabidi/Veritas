#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
STACK_NAME="${GBN_BRIDGE_STACK_NAME:-gbn-conduit-full-dev}"
REGION="${GBN_BRIDGE_AWS_REGION:-${AWS_REGION:-us-east-1}}"
WINDOW_MINUTES="${GBN_BRIDGE_TRACE_WINDOW_MINUTES:-15}"
CHAIN_ID="${GBN_BRIDGE_CHAIN_ID:-}"
ARTIFACT_DIR="${VERITAS_CONDUIT_TRACE_ARTIFACT_DIR:-$ROOT_DIR/artifacts/conduit-full-validation}"
REQUIRE_CHAIN_ID="false"

usage() {
  cat <<USAGE
Usage: $0 [--stack-name NAME] [--region REGION] [--window-minutes MINUTES] [--chain-id ID] [--artifact-dir DIR] [--require-chain-id]

Collect CloudFormation, ECS, and CloudWatch Logs evidence for a deployed
GBN-PROTO-006 Conduit full stack. When --chain-id is provided, log collection
is filtered to that root trace identifier.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --stack-name) STACK_NAME="$2"; shift 2 ;;
    --region) REGION="$2"; shift 2 ;;
    --window-minutes) WINDOW_MINUTES="$2"; shift 2 ;;
    --chain-id) CHAIN_ID="$2"; shift 2 ;;
    --artifact-dir) ARTIFACT_DIR="$2"; shift 2 ;;
    --require-chain-id) REQUIRE_CHAIN_ID="true"; shift ;;
    --help|-h) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ "$STACK_NAME" != gbn-conduit-full-* ]]; then
  echo "stack name must start with gbn-conduit-full-: $STACK_NAME" >&2
  exit 2
fi

if ! [[ "$WINDOW_MINUTES" =~ ^[0-9]+$ ]] || [[ "$WINDOW_MINUTES" -lt 1 ]]; then
  echo "window minutes must be a positive integer: $WINDOW_MINUTES" >&2
  exit 2
fi

command -v aws >/dev/null 2>&1 || {
  echo "required command not found: aws" >&2
  exit 127
}

mkdir -p "$ARTIFACT_DIR"

RUN_ID="$(date -u +"%Y%m%dT%H%M%SZ")"
SUMMARY_FILE="$ARTIFACT_DIR/trace-summary-$RUN_ID.md"
STACK_FILE="$ARTIFACT_DIR/stack-$RUN_ID.json"
SERVICES_FILE="$ARTIFACT_DIR/ecs-services-$RUN_ID.json"

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

aws cloudformation describe-stacks \
  --region "$REGION" \
  --stack-name "$STACK_NAME" \
  --output json > "$STACK_FILE"

CLUSTER_NAME="$(stack_output ClusterName)"
AUTHORITY_SERVICE="$(stack_output AuthorityServiceName)"
RECEIVER_SERVICE="$(stack_output ReceiverServiceName)"
BRIDGE_SERVICE="$(stack_output BridgeServiceName)"
AUTHORITY_URL="$(stack_output AuthorityInternalUrl)"
RECEIVER_URL="$(stack_output ReceiverInternalUrl)"
CONTROL_URL="$(stack_output ControlUrl)"
AUTHORITY_LOG_GROUP="$(stack_output AuthorityLogGroup)"
RECEIVER_LOG_GROUP="$(stack_output ReceiverLogGroup)"
BRIDGE_LOG_GROUP="$(stack_output BridgeLogGroup)"

aws ecs describe-services \
  --region "$REGION" \
  --cluster "$CLUSTER_NAME" \
  --services "$AUTHORITY_SERVICE" "$RECEIVER_SERVICE" "$BRIDGE_SERVICE" \
  --output json > "$SERVICES_FILE"

collect_log_group() {
  local label="$1"
  local log_group="$2"
  local output_file="$ARTIFACT_DIR/${label}-events-$RUN_ID.json"
  local count_file="$ARTIFACT_DIR/${label}-count-$RUN_ID.txt"
  local -a filter_args=()

  if [[ -n "$CHAIN_ID" ]]; then
    filter_args+=(--filter-pattern "\"$CHAIN_ID\"")
  fi

  aws logs filter-log-events \
    --region "$REGION" \
    --log-group-name "$log_group" \
    --start-time "$start_ms" \
    --limit 1000 \
    "${filter_args[@]}" \
    --output json > "$output_file"

  aws logs filter-log-events \
    --region "$REGION" \
    --log-group-name "$log_group" \
    --start-time "$start_ms" \
    --limit 1000 \
    "${filter_args[@]}" \
    --query "length(events)" \
    --output text > "$count_file"
}

collect_log_group authority "$AUTHORITY_LOG_GROUP"
collect_log_group receiver "$RECEIVER_LOG_GROUP"
collect_log_group bridge "$BRIDGE_LOG_GROUP"

AUTHORITY_COUNT="$(cat "$ARTIFACT_DIR/authority-count-$RUN_ID.txt")"
RECEIVER_COUNT="$(cat "$ARTIFACT_DIR/receiver-count-$RUN_ID.txt")"
BRIDGE_COUNT="$(cat "$ARTIFACT_DIR/bridge-count-$RUN_ID.txt")"

if [[ "$REQUIRE_CHAIN_ID" == "true" && -n "$CHAIN_ID" ]]; then
  if [[ "$AUTHORITY_COUNT" == "0" || "$RECEIVER_COUNT" == "0" || "$BRIDGE_COUNT" == "0" ]]; then
    cat > "$SUMMARY_FILE" <<SUMMARY
# Conduit Trace Collection Summary

Status: FAILED
Reason: --require-chain-id was set, but at least one service had no matching events.

Stack: $STACK_NAME
Region: $REGION
ChainId: $CHAIN_ID
WindowMinutes: $WINDOW_MINUTES
AuthorityEvents: $AUTHORITY_COUNT
ReceiverEvents: $RECEIVER_COUNT
BridgeEvents: $BRIDGE_COUNT
ArtifactDir: $ARTIFACT_DIR
SUMMARY
    cat "$SUMMARY_FILE"
    exit 1
  fi
fi

cat > "$SUMMARY_FILE" <<SUMMARY
# Conduit Trace Collection Summary

Status: COMPLETE
Stack: $STACK_NAME
Region: $REGION
RunId: $RUN_ID
WindowMinutes: $WINDOW_MINUTES
ChainId: ${CHAIN_ID:-<not provided>}

AuthorityInternalUrl: $AUTHORITY_URL
ReceiverInternalUrl: $RECEIVER_URL
ControlUrl: $CONTROL_URL

AuthorityEvents: $AUTHORITY_COUNT
ReceiverEvents: $RECEIVER_COUNT
BridgeEvents: $BRIDGE_COUNT

Artifacts:
- $STACK_FILE
- $SERVICES_FILE
- $ARTIFACT_DIR/authority-events-$RUN_ID.json
- $ARTIFACT_DIR/receiver-events-$RUN_ID.json
- $ARTIFACT_DIR/bridge-events-$RUN_ID.json
SUMMARY

cat "$SUMMARY_FILE"
