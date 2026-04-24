#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MODE="local"
STACK_NAME="${GBN_BRIDGE_STACK_NAME:-gbn-conduit-full-dev}"
REGION="${GBN_BRIDGE_AWS_REGION:-${AWS_REGION:-us-east-1}}"
WINDOW_MINUTES="${GBN_BRIDGE_TRACE_WINDOW_MINUTES:-15}"
TARGET_DIR="${VERITAS_BRIDGE_TARGET_DIR:-${TMPDIR:-/tmp}/veritas-proto006-phase10-target}"
ARTIFACT_DIR="${VERITAS_CONDUIT_VALIDATION_ARTIFACT_DIR:-$ROOT_DIR/artifacts/conduit-full-validation}"
CHAIN_ID="${GBN_BRIDGE_CHAIN_ID:-}"
MOBILE_CONTEXT="${GBN_BRIDGE_MOBILE_CONTEXT:-unspecified}"
REQUIRE_CHAIN_ID="false"

usage() {
  cat <<USAGE
Usage: $0 [--mode local|aws] [--stack-name NAME] [--region REGION] [--target-dir DIR]
          [--artifact-dir DIR] [--window-minutes MINUTES] [--chain-id ID]
          [--mobile-context TEXT] [--require-chain-id]

Run the GBN-PROTO-006 Phase 10 full implementation validation workflow.

Modes:
  local  Run the distributed local e2e harness and write a validation summary.
  aws    Validate a deployed gbn-conduit-full-* stack, collect ECS/CloudWatch
         evidence, and preserve chain_id trace artifacts.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode) MODE="$2"; shift 2 ;;
    --stack-name) STACK_NAME="$2"; shift 2 ;;
    --region) REGION="$2"; shift 2 ;;
    --target-dir) TARGET_DIR="$2"; shift 2 ;;
    --artifact-dir) ARTIFACT_DIR="$2"; shift 2 ;;
    --window-minutes) WINDOW_MINUTES="$2"; shift 2 ;;
    --chain-id) CHAIN_ID="$2"; shift 2 ;;
    --mobile-context) MOBILE_CONTEXT="$2"; shift 2 ;;
    --require-chain-id) REQUIRE_CHAIN_ID="true"; shift ;;
    --help|-h) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if ! [[ "$WINDOW_MINUTES" =~ ^[0-9]+$ ]] || [[ "$WINDOW_MINUTES" -lt 1 ]]; then
  echo "window minutes must be a positive integer: $WINDOW_MINUTES" >&2
  exit 2
fi

mkdir -p "$ARTIFACT_DIR"

RUN_ID="$(date -u +"%Y%m%dT%H%M%SZ")"
SUMMARY_FILE="$ARTIFACT_DIR/validation-summary-$RUN_ID.md"
LOCAL_LOG="$ARTIFACT_DIR/local-e2e-$RUN_ID.log"
SMOKE_LOG="$ARTIFACT_DIR/aws-smoke-$RUN_ID.log"
TRACE_LOG="$ARTIFACT_DIR/aws-trace-$RUN_ID.log"

write_common_header() {
  {
    echo "# Conduit Full Validation Summary"
    echo
    echo "RunId: $RUN_ID"
    echo "Mode: $MODE"
    echo "Stack: $STACK_NAME"
    echo "Region: $REGION"
    echo "ChainId: ${CHAIN_ID:-<not provided>}"
    echo "MobileContext: $MOBILE_CONTEXT"
    echo "ArtifactDir: $ARTIFACT_DIR"
    echo
  } > "$SUMMARY_FILE"
}

run_local() {
  write_common_header
  echo "==> Phase 10 local full implementation validation"
  echo "Target dir: $TARGET_DIR"
  echo "Artifact dir: $ARTIFACT_DIR"

  VERITAS_BRIDGE_TARGET_DIR="$TARGET_DIR" \
  VERITAS_CONDUIT_E2E_ARTIFACT_DIR="$ARTIFACT_DIR/local-e2e" \
    "$ROOT_DIR/infra/scripts/run-conduit-e2e.sh" | tee "$LOCAL_LOG"

  {
    echo "Status: COMPLETE"
    echo
    echo "LocalEvidence:"
    echo "- distributed e2e harness passed"
    echo "- bootstrap, refresh, data path, failover, restart recovery, and trace scenarios were exercised"
    echo "- log: $LOCAL_LOG"
    echo
    echo "LiveEvidence:"
    echo "- AWS/mobile evidence not collected in local mode"
  } >> "$SUMMARY_FILE"

  cat "$SUMMARY_FILE"
}

run_aws() {
  if [[ "$STACK_NAME" != gbn-conduit-full-* ]]; then
    echo "stack name must start with gbn-conduit-full-: $STACK_NAME" >&2
    exit 2
  fi

  command -v aws >/dev/null 2>&1 || {
    echo "required command not found: aws" >&2
    exit 127
  }

  write_common_header
  echo "==> Phase 10 AWS/mobile full implementation validation"
  echo "Stack: $STACK_NAME"
  echo "Region: $REGION"
  echo "Artifact dir: $ARTIFACT_DIR"

  "$ROOT_DIR/infra/scripts/smoke-conduit-full.sh" \
    --stack-name "$STACK_NAME" \
    --region "$REGION" | tee "$SMOKE_LOG"

  local -a trace_args=(
    --stack-name "$STACK_NAME"
    --region "$REGION"
    --window-minutes "$WINDOW_MINUTES"
    --artifact-dir "$ARTIFACT_DIR"
  )
  if [[ -n "$CHAIN_ID" ]]; then
    trace_args+=(--chain-id "$CHAIN_ID")
  fi
  if [[ "$REQUIRE_CHAIN_ID" == "true" ]]; then
    trace_args+=(--require-chain-id)
  fi

  "$ROOT_DIR/infra/scripts/collect-conduit-traces.sh" "${trace_args[@]}" | tee "$TRACE_LOG"

  {
    echo "Status: COMPLETE"
    echo
    echo "AwsEvidence:"
    echo "- smoke: $SMOKE_LOG"
    echo "- traces: $TRACE_LOG"
    echo
    echo "MeasurementWindowMinutes: $WINDOW_MINUTES"
    echo "MobileContext: $MOBILE_CONTEXT"
    echo
    echo "RequiredManualAnnotations:"
    echo "- actual mobile carrier / network path used"
    echo "- observed bootstrap timing"
    echo "- observed upload / ACK timing"
    echo "- observed failover or churn timing"
    echo "- unresolved anomalies"
  } >> "$SUMMARY_FILE"

  cat "$SUMMARY_FILE"
}

case "$MODE" in
  local) run_local ;;
  aws) run_aws ;;
  *) echo "unsupported mode: $MODE" >&2; usage >&2; exit 2 ;;
esac
