#!/usr/bin/env bash
# monitor-bootup.sh <stack-name> [region] [scale-target]
# Run in parallel with deploy-scale-test.sh to capture thundering-herd evidence.
# Polls every 15 seconds, prints a live table, saves raw data to /tmp/gbn-bootup-<pid>/

set -uo pipefail
export AWS_PAGER=""

if ! command -v aws >/dev/null 2>&1; then
  if command -v aws.exe >/dev/null 2>&1; then
    aws() { aws.exe "$@"; }
  else
    echo "ERROR: aws CLI not found in PATH"; exit 1
  fi
fi

STACK="${1:?Usage: $0 <stack-name> [region] [scale-target]}"
REGION="${2:-us-east-1}"
SCALE_TARGET="${3:-100}"
INTERVAL=15
OUT_DIR="/tmp/gbn-bootup-$$"
mkdir -p "$OUT_DIR"

now_iso() {
  python3 -c "from datetime import datetime,timezone; print(datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'))"
}
ago_iso() {
  local mins="$1"
  python3 -c "from datetime import datetime,timedelta,timezone; print((datetime.now(timezone.utc)-timedelta(minutes=$mins)).strftime('%Y-%m-%dT%H:%M:%SZ'))"
}

cf_resource() {
  aws cloudformation describe-stack-resources \
    --stack-name "$STACK" --region "$REGION" \
    --logical-resource-id "$1" \
    --query 'StackResources[0].PhysicalResourceId' \
    --output text 2>/dev/null || echo ""
}

echo "=== GBN Bootup Monitor — waiting for stack to exist ==="
echo "  Stack   : $STACK"
echo "  Region  : $REGION"
echo "  OutDir  : $OUT_DIR"
echo "  Polling : every ${INTERVAL}s"
echo ""

SEED_INSTANCE="" PUB_INSTANCE="" CLUSTER="" HOSTILE_SVC="" FREE_SVC=""

resolve_resources() {
  SEED_INSTANCE="$(cf_resource SeedRelayInstance)"
  PUB_INSTANCE="$(cf_resource PublisherInstance)"
  CLUSTER="$(cf_resource ECSCluster)"
  HOSTILE_SVC="$(cf_resource HostileRelayService)"
  FREE_SVC="$(cf_resource FreeRelayService)"
}

# Wait until stack exists and SeedRelayInstance is known (may take several minutes during CFN deploy)
# Also reject "None" which is what AWS CLI outputs when a resource isn't yet assigned
while [ -z "$SEED_INSTANCE" ] || [ "$SEED_INSTANCE" = "None" ]; do
  echo "  [$(date +%H:%M:%S)] Stack/resources not yet available — waiting ${INTERVAL}s..."
  sleep "$INTERVAL"
  resolve_resources
done
echo "  [$(date +%H:%M:%S)] Resources resolved:"
echo "    SeedRelay instance : $SEED_INSTANCE"
echo "    Publisher instance : $PUB_INSTANCE"
echo "    ECS cluster        : $CLUSTER"
echo "    Hostile service    : $HOSTILE_SVC"
echo "    Free service       : $FREE_SVC"
echo ""
printf "%-8s %-10s %-10s %-10s %-10s %-6s  %-16s %-12s  %s\n" \
  "TIME" "SEED_CPU%" "PUB_CPU%" "SEED_NETIN" "SEED_NETOUT" "SFAIL" "BOOTSTRAP_MAX" "GOSSIP_BW" "ECS(H/F run/des)"
echo "$(printf '%0.s-' {1..110})"

ec2_stat() {
  local instance="$1" metric="$2" stat="$3" start="$4" end="$5"
  aws cloudwatch get-metric-statistics \
    --namespace AWS/EC2 --metric-name "$metric" \
    --dimensions Name=InstanceId,Value="$instance" \
    --start-time "$start" --end-time "$end" \
    --period 60 --statistics "$stat" \
    --region "$REGION" --output json 2>/dev/null \
  | python3 -c "
import json,sys
d=json.load(sys.stdin)
pts=sorted(d.get('Datapoints',[]),key=lambda x:x.get('Timestamp',''))
if pts:
    v=pts[-1].get('$stat',0)
    print(f'{v:.0f}')
else:
    print('-1')
"
}

tick=0
while true; do
  NOW="$(now_iso)"
  START2="$(ago_iso 2)"
  START25="$(ago_iso 25)"

  # EC2 metrics (last 2-minute window, 60s period)
  SEED_CPU="$(ec2_stat "$SEED_INSTANCE" CPUUtilization Average "$START2" "$NOW")"
  PUB_CPU="$(ec2_stat "$PUB_INSTANCE"  CPUUtilization Average "$START2" "$NOW")"
  SEED_NETIN="$(ec2_stat "$SEED_INSTANCE" NetworkIn Sum "$START2" "$NOW")"
  SEED_NETOUT="$(ec2_stat "$SEED_INSTANCE" NetworkOut Sum "$START2" "$NOW")"
  SEED_SFAIL="$(ec2_stat "$SEED_INSTANCE" StatusCheckFailed Maximum "$START2" "$NOW")"

  # Format bytes -> KB
  fmt_kb() {
    local v="$1"
    if [ "$v" = "-1" ] || [ -z "$v" ]; then echo "--"; return; fi
    python3 -c "v=$v; print(f'{v/1024:.0f}KB') if v>=0 else print('--')"
  }
  SEED_NETIN_FMT="$(fmt_kb "$SEED_NETIN")"
  SEED_NETOUT_FMT="$(fmt_kb "$SEED_NETOUT")"
  SEED_CPU_FMT="$([ "$SEED_CPU" = "-1" ] && echo "--" || echo "${SEED_CPU}%")"
  PUB_CPU_FMT="$([ "$PUB_CPU"  = "-1" ] && echo "--" || echo "${PUB_CPU}%")"
  SFAIL_FMT="$([ "$SEED_SFAIL" = "-1" ] && echo "--" || echo "$SEED_SFAIL")"

  # GBN application metrics — write query JSON to temp file to avoid escaping hell
  QUERY_FILE="$(mktemp --suffix=.json)"
  cat > "$QUERY_FILE" <<ENDJSON
[
  {
    "Id": "boot",
    "Expression": "SUM(SEARCH('{GBN/ScaleTest,Scale,Subnet,NodeId} Scale=\"${SCALE_TARGET}\" MetricName=\"BootstrapResult\"', 'SampleCount', 60))",
    "ReturnData": true
  },
  {
    "Id": "bw",
    "Expression": "SUM(SEARCH('{GBN/ScaleTest,Scale,Subnet} Scale=\"${SCALE_TARGET}\" MetricName=\"GossipBandwidthBytes\"', 'Sum', 60))",
    "ReturnData": true
  }
]
ENDJSON

  GBN_JSON="$(aws cloudwatch get-metric-data \
    --region "$REGION" \
    --start-time "$START25" --end-time "$NOW" \
    --scan-by TimestampDescending \
    --metric-data-queries "file://$QUERY_FILE" \
    --output json 2>/dev/null || echo '{}')"
  rm -f "$QUERY_FILE"

  BOOT_MAX="$(echo "$GBN_JSON" | python3 -c "
import json,sys
d=json.load(sys.stdin)
for r in d.get('MetricDataResults',[]):
    if r['Id']=='boot':
        vs=r.get('Values',[])
        print(int(max(vs)) if vs else 0)
        sys.exit()
print(0)" 2>/dev/null || echo 0)"

  BW_TOTAL="$(echo "$GBN_JSON" | python3 -c "
import json,sys
d=json.load(sys.stdin)
for r in d.get('MetricDataResults',[]):
    if r['Id']=='bw':
        vs=r.get('Values',[])
        total=sum(vs)
        print(f'{total/1024:.0f}KB' if total>0 else '0KB')
        sys.exit()
print('0KB')" 2>/dev/null || echo "0KB")"

  # ECS running counts
  ECS_ROW="$(aws ecs describe-services \
    --cluster "$CLUSTER" \
    --services "$HOSTILE_SVC" "$FREE_SVC" \
    --region "$REGION" \
    --query 'services[*].[runningCount,desiredCount]' \
    --output text 2>/dev/null | tr '\n' ' ' || echo "-- --")"
  # Format as H:run/des F:run/des
  ECS_FMT="$(echo "$ECS_ROW" | python3 -c "
import sys
parts=sys.stdin.read().split()
if len(parts)>=4:
    print(f'H:{parts[0]}/{parts[1]} F:{parts[2]}/{parts[3]}')
elif len(parts)>=2:
    print(f'H:{parts[0]}/{parts[1]}')
else:
    print('--')" 2>/dev/null || echo "--")"

  # Warn flags
  WARN=""
  [ "$SFAIL_FMT" = "1" ] && WARN="${WARN} SEED_UNRESPONSIVE!"
  if [ "$SEED_CPU" != "-1" ] && [ -n "$SEED_CPU" ]; then
    python3 -c "import sys; sys.exit(0 if float('${SEED_CPU}') >= 80 else 1)" 2>/dev/null && WARN="${WARN} HIGH_CPU!"
  fi

  printf "%-8s %-10s %-10s %-10s %-10s %-6s  %-16s %-12s  %-22s%s\n" \
    "$(date +%H:%M:%S)" "$SEED_CPU_FMT" "$PUB_CPU_FMT" \
    "$SEED_NETIN_FMT" "$SEED_NETOUT_FMT" "$SFAIL_FMT" \
    "$BOOT_MAX/$SCALE_TARGET" "$BW_TOTAL" "$ECS_FMT" "$WARN"

  # Persist raw tick data
  python3 -c "
import json
d={'tick':$tick,'ts':'$NOW','seed_cpu':'$SEED_CPU','pub_cpu':'$PUB_CPU',
   'seed_netin':$SEED_NETIN,'seed_netout':$SEED_NETOUT,'seed_sfail':'$SFAIL_FMT',
   'bootstrap_max':$BOOT_MAX,'gossip_bw_raw':'$BW_TOTAL','ecs':'$ECS_FMT'}
print(json.dumps(d))
" >> "$OUT_DIR/summary.jsonl" 2>/dev/null || true

  tick=$((tick+1))
  sleep "$INTERVAL"
done
