#!/usr/bin/env bash
# diag-bootup.sh — one-shot diagnostic snapshot of N=100 bootup state
REGION=us-east-1
CLUSTER=gbn-proto-phase1-scale-n100-cluster
STACK=gbn-proto-phase1-scale-n100

NOW=$(python3 -c "from datetime import datetime,timezone; print(datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'))")
START60=$(python3 -c "from datetime import datetime,timedelta,timezone; print((datetime.now(timezone.utc)-timedelta(minutes=60)).strftime('%Y-%m-%dT%H:%M:%SZ'))")
START10=$(python3 -c "from datetime import datetime,timedelta,timezone; print((datetime.now(timezone.utc)-timedelta(minutes=10)).strftime('%Y-%m-%dT%H:%M:%SZ'))")

echo "=== 1. ECS Task Counts ==="
aws ecs describe-services \
  --cluster "$CLUSTER" --region "$REGION" \
  --services \
    "${STACK}-HostileRelayService-B4hlvfURYhQ5" \
    "${STACK}-FreeRelayService-1cL6N7MpDz3p" \
  --query 'services[*].[serviceName,runningCount,desiredCount,pendingCount]' \
  --output table 2>&1

echo ""
echo "=== 2. Sample Running Task Health ==="
TASKS=$(aws ecs list-tasks --cluster "$CLUSTER" --region "$REGION" \
  --desired-status RUNNING --query 'taskArns[:3]' --output text 2>/dev/null | tr '\t' ' ')
for t in $TASKS; do
  echo "Task: ${t##*/}"
  aws ecs describe-tasks --cluster "$CLUSTER" --region "$REGION" \
    --tasks "$t" \
    --query 'tasks[0].[lastStatus,healthStatus,containers[0].[name,lastStatus,exitCode,reason]]' \
    --output json 2>&1 | python3 -c "
import json,sys
try:
    d=json.load(sys.stdin)
    print('  status=%s health=%s container=%s' % (d[0],d[1],d[2]))
except: print('  (parse error)')"
done

echo ""
echo "=== 3. BootstrapResult last 60min (Scale=100) ==="
cat > /tmp/boot_query.json << 'JSON'
[{"Id":"boot","Expression":"SUM(SEARCH(\"{GBN/ScaleTest,Scale,Subnet,NodeId} Scale=\\\"100\\\" MetricName=\\\"BootstrapResult\\\"\", \"SampleCount\", 60))","ReturnData":true}]
JSON
aws cloudwatch get-metric-data \
  --region "$REGION" \
  --start-time "$START60" --end-time "$NOW" \
  --scan-by TimestampDescending \
  --metric-data-queries file:///tmp/boot_query.json \
  --output json 2>&1 | python3 -c "
import json,sys
try:
    d=json.load(sys.stdin)
    r=d.get('MetricDataResults',[{}])[0]
    vs=r.get('Values',[])
    ts=r.get('Timestamps',[])
    print(f'  {len(vs)} datapoints found')
    for t,v in zip(ts[:8],vs[:8]):
        print(f'  {t}: {v}')
except Exception as e:
    print('  error:', e)
    sys.stdin.seek(0)
    print(sys.stdin.read()[:200])"

echo ""
echo "=== 4. GossipBandwidthBytes last 60min ==="
cat > /tmp/bw_query.json << 'JSON'
[{"Id":"bw","Expression":"SUM(SEARCH(\"{GBN/ScaleTest,Scale,Subnet} Scale=\\\"100\\\" MetricName=\\\"GossipBandwidthBytes\\\"\", \"Sum\", 60))","ReturnData":true}]
JSON
aws cloudwatch get-metric-data \
  --region "$REGION" \
  --start-time "$START60" --end-time "$NOW" \
  --scan-by TimestampDescending \
  --metric-data-queries file:///tmp/bw_query.json \
  --output json 2>&1 | python3 -c "
import json,sys
try:
    d=json.load(sys.stdin)
    r=d.get('MetricDataResults',[{}])[0]
    vs=r.get('Values',[])
    ts=r.get('Timestamps',[])
    print(f'  {len(vs)} datapoints')
    for t,v in zip(ts[:5],vs[:5]):
        print(f'  {t}: {v/1024:.0f}KB')
except Exception as e: print('  error:',e)"

echo ""
echo "=== 5. CloudWatch Log Groups (all) ==="
aws logs describe-log-groups --region "$REGION" \
  --query 'logGroups[*].[logGroupName,storedBytes]' \
  --output table 2>&1 | grep -i "gbn\|relay\|scale\|ecs" | head -10

echo ""
echo "=== 6. Recent Relay Task Logs ==="
LOG_GROUP="/aws/ecs/gbn-proto-phase1-scale-n100/gbn"
echo "Log group: $LOG_GROUP"
STREAM=$(aws logs describe-log-streams --region "$REGION" \
  --log-group-name "$LOG_GROUP" \
  --order-by LastEventTime --descending \
  --query 'logStreams[0].logStreamName' --output text 2>/dev/null)
echo "Latest stream: $STREAM"
if [ -n "$STREAM" ] && [ "$STREAM" != "None" ]; then
  aws logs get-log-events --region "$REGION" \
    --log-group-name "$LOG_GROUP" \
    --log-stream-name "$STREAM" \
    --limit 50 \
    --query 'events[*].message' --output text 2>&1
else
  echo "  (no streams found)"
  # List all streams briefly
  aws logs describe-log-streams --region "$REGION" \
    --log-group-name "$LOG_GROUP" \
    --order-by LastEventTime --descending \
    --query 'logStreams[:5].[logStreamName,lastEventTimestamp]' \
    --output table 2>&1
fi

echo ""
echo "=== 7. Seed Relay Instance Status ==="
SEED=$(aws cloudformation describe-stack-resources \
  --stack-name "$STACK" --region "$REGION" \
  --logical-resource-id SeedRelayInstance \
  --query 'StackResources[0].PhysicalResourceId' --output text 2>/dev/null)
echo "SeedRelay: $SEED"
aws ec2 describe-instance-status --instance-ids "$SEED" --region "$REGION" \
  --query 'InstanceStatuses[0].[InstanceState.Name,SystemStatus.Status,InstanceStatus.Status]' \
  --output text 2>&1
