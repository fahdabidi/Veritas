#!/usr/bin/env bash
# teardown-scale-test.sh — Disable chaos, dump metrics, scale services to zero, and delete scale stack.
#
# Usage: ./teardown-scale-test.sh <stack-name> [region]

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

STACK_NAME="${1:?Usage: $0 <stack-name> [region]}"
REGION="${2:-us-east-1}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROTO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
RESULTS_DIR="$PROTO_ROOT/results"
mkdir -p "$RESULTS_DIR"

cf_resource_id() {
  local logical_id="$1"
  aws cloudformation describe-stack-resources \
    --stack-name "$STACK_NAME" \
    --region "$REGION" \
    --logical-resource-id "$logical_id" \
    --query 'StackResources[0].PhysicalResourceId' \
    --output text 2>/dev/null || true
}

echo "============================================"
echo "  GBN Phase 2 — Teardown Scale Test"
echo "  Stack:  $STACK_NAME"
echo "  Region: $REGION"
echo "============================================"


CLUSTER_NAME="$(cf_resource_id ECSCluster)"
CHAOS_RULE_NAME="$(cf_resource_id ChaosEngineRule)"
HOSTILE_SERVICE_NAME="$(cf_resource_id HostileRelayService)"
FREE_SERVICE_NAME="$(cf_resource_id FreeRelayService)"
CREATOR_SERVICE_NAME="$(cf_resource_id CreatorService)"
PUBLISHER_SERVICE_NAME="$(cf_resource_id PublisherService)"

TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
METRICS_FILE="$RESULTS_DIR/scale-${STACK_NAME}-${TIMESTAMP}-metrics.json"

# Derive numeric scale from stack name suffix (e.g. gbn-proto-phase1-scale-n100 → 100).
# Used to scope CloudWatch SEARCH to the current run and avoid MaxMetricsExceeded from
# the 700+ accumulated NodeId series across all historical runs.
SCALE_HINT="$(echo "$STACK_NAME" | grep -oE 'n[0-9]+$' | tr -d 'n' || true)"
SCALE_FILTER=""
if [ -n "$SCALE_HINT" ]; then
  SCALE_FILTER=" Scale=\\\"${SCALE_HINT}\\\""
fi

echo "[1/5] Disabling chaos rule (if present)..."
if [ -n "$CHAOS_RULE_NAME" ] && [ "$CHAOS_RULE_NAME" != "None" ]; then
  aws events disable-rule --name "$CHAOS_RULE_NAME" --region "$REGION" || true
  echo "  Chaos disabled: $CHAOS_RULE_NAME"
else
  echo "  Chaos rule not found (already removed or stack not active)."
fi

echo "[2/5] Dumping CloudWatch metrics..."
END_TIME="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || python - <<'PY'
from datetime import datetime, timezone
print(datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'))
PY
)"
START_TIME="$(date -u -d '4 hours ago' +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || python - <<'PY'
from datetime import datetime, timedelta, timezone
print((datetime.now(timezone.utc) - timedelta(hours=4)).strftime('%Y-%m-%dT%H:%M:%SZ'))
PY
)"

# Issue separate get-metric-data calls per metric to avoid MaxMetricsExceeded from
# combined SEARCH expressions (four queries × 400+ series per query exceeds the 500 limit).
#
# cw_query        — searches {Scale,Subnet,NodeId} dimension group (per-node metrics)
# cw_query_agg    — searches {Scale,Subnet} dimension group only (aggregate metrics)
#
# GossipBandwidthBytes is published WITHOUT NodeId (aggregate) so its SEARCH spec must
# also omit NodeId.  After 5 × N=100 runs the per-NodeId series count reached 500 and
# the SEARCH returned only stale series, reporting 0 bytes every run.
cw_query() {
  local metric_name="$1" stat="$2"
  aws cloudwatch get-metric-data \
    --region "$REGION" \
    --start-time "$START_TIME" \
    --end-time "$END_TIME" \
    --scan-by TimestampAscending \
    --metric-data-queries "[{\"Id\":\"m\",\"Expression\":\"SUM(SEARCH('{GBN/ScaleTest,Scale,Subnet,NodeId}${SCALE_FILTER} MetricName=\\\"${metric_name}\\\"', '${stat}', 60))\",\"ReturnData\":true,\"Label\":\"${metric_name}\"}]" \
    --output json 2>/dev/null || echo '{"MetricDataResults":[]}'
}

cw_query_agg() {
  local metric_name="$1" stat="$2"
  aws cloudwatch get-metric-data \
    --region "$REGION" \
    --start-time "$START_TIME" \
    --end-time "$END_TIME" \
    --scan-by TimestampAscending \
    --metric-data-queries "[{\"Id\":\"m\",\"Expression\":\"SUM(SEARCH('{GBN/ScaleTest,Scale,Subnet}${SCALE_FILTER} MetricName=\\\"${metric_name}\\\"', '${stat}', 60))\",\"ReturnData\":true,\"Label\":\"${metric_name}\"}]" \
    --output json 2>/dev/null || echo '{"MetricDataResults":[]}'
}

export BOOT_JSON="$(cw_query BootstrapResult Sum)"
export GOSS_JSON="$(cw_query_agg GossipBandwidthBytes Sum)"
export CIRC_JSON="$(cw_query CircuitBuildResult Sum)"
export CHUNK_JSON="$(cw_query ChunksDelivered Sum)"
# Phase 2 metrics
export REASSEMBLED_JSON="$(cw_query_agg ChunksReassembled Sum)"
export RECEIVED_JSON="$(cw_query_agg ChunksReceived Sum)"
export DIVERSITY_JSON="$(cw_query PathDiversityResult Minimum)"
export HASH_MATCH_JSON="$(cw_query_agg HashMatchResult Minimum)"
export CIRC_COUNT_JSON="$(cw_query CircuitBuildResult SampleCount)"


python3 - <<'PYEOF' > "$METRICS_FILE"
import json, os

def load(env_key):
    raw = os.environ.get(env_key, '{}')
    try:
        d = json.loads(raw)
        return d.get("MetricDataResults", [{}])
    except Exception:
        return [{}]

results = []
for label, env_key in [
    ("bootstrap",          "BOOT_JSON"),
    ("gossipbw",           "GOSS_JSON"),
    ("circuit",            "CIRC_JSON"),
    ("circuit_count",      "CIRC_COUNT_JSON"),
    ("chunks",             "CHUNK_JSON"),
    ("chunks_reassembled", "REASSEMBLED_JSON"),
    ("chunks_received",    "RECEIVED_JSON"),
    ("path_diversity",     "DIVERSITY_JSON"),
    ("hash_match",         "HASH_MATCH_JSON"),
]:
    r = dict(load(env_key)[0]) if load(env_key) else {}
    r["Id"]    = label
    r["Label"] = label
    results.append(r)

# Compute derived pass/fail gates per test spec Section 6
def first_sum(env_key):
    raw = os.environ.get(env_key, '{}')
    try:
        d = json.loads(raw)
        vals = d.get("MetricDataResults", [{}])[0].get("Values", [])
        return sum(vals) if vals else 0
    except Exception:
        return 0

circ_sum   = first_sum("CIRC_JSON")
circ_count = first_sum("CIRC_COUNT_JSON")
gate = {
    "circuit_build_success_rate": round(circ_sum / circ_count, 4) if circ_count > 0 else None,
    "circuit_build_pass":         (circ_sum / circ_count > 0.80) if circ_count > 0 else False,
    "chunks_reassembled_sum":     first_sum("REASSEMBLED_JSON"),
    "chunks_received_sum":        first_sum("RECEIVED_JSON"),
    "path_diversity_min":         first_sum("DIVERSITY_JSON"),
    "hash_match_min":             first_sum("HASH_MATCH_JSON"),
    "gossip_nonzero":             first_sum("GOSS_JSON") > 0,
}
gate["phase2_pass"] = bool(
    gate["circuit_build_pass"] and
    gate["chunks_reassembled_sum"] >= 1 and
    gate["path_diversity_min"] == 1.0 and
    gate["hash_match_min"] == 1.0 and
    gate["gossip_nonzero"]
)

print(json.dumps({"MetricDataResults": results, "Phase2Gate": gate}, indent=4))

PYEOF

echo "  Metrics saved: $METRICS_FILE"

echo "[3/5] Scaling ECS services to 0 (fast cost cutoff)..."
if [ -n "$CLUSTER_NAME" ] && [ "$CLUSTER_NAME" != "None" ]; then
  for svc in "$HOSTILE_SERVICE_NAME" "$FREE_SERVICE_NAME" "$CREATOR_SERVICE_NAME" "$PUBLISHER_SERVICE_NAME"; do
    if [ -n "$svc" ] && [ "$svc" != "None" ]; then
      aws ecs update-service --cluster "$CLUSTER_NAME" --service "$svc" --desired-count 0 --region "$REGION" >/dev/null || true
      echo "  Scaled to zero: $svc"
    fi
  done
else
  echo "  Cluster not found; skipping ECS scale-down."
fi

echo "[4/4] Emptying ECR repositories before stack deletion..."
ECR_RELAY_REPO="$(cf_resource_id EcrRepositoryRelay)"
ECR_PUB_REPO="$(cf_resource_id EcrRepositoryPublisher)"
for repo in "$ECR_RELAY_REPO" "$ECR_PUB_REPO"; do
  if [ -n "$repo" ] && [ "$repo" != "None" ]; then
    image_ids="$(aws ecr list-images --repository-name "$repo" --region "$REGION" --query 'imageIds' --output json 2>/dev/null || echo '[]')"
    if [ "$image_ids" != "[]" ] && [ "$image_ids" != "" ]; then
      aws ecr batch-delete-image --repository-name "$repo" --region "$REGION" --image-ids "$image_ids" >/dev/null 2>&1 || true
      echo "  Emptied ECR repo: $repo"
    else
      echo "  ECR repo already empty: $repo"
    fi
  fi
done

echo "[5/5] Deleting CloudFormation stack..."
aws cloudformation delete-stack --stack-name "$STACK_NAME" --region "$REGION"
echo "  Waiting for stack delete completion..."
aws cloudformation wait stack-delete-complete --stack-name "$STACK_NAME" --region "$REGION"

echo ""
echo "✅ Scale test stack teardown complete."
echo "   Stack:   $STACK_NAME"
echo "   Metrics: $METRICS_FILE"
