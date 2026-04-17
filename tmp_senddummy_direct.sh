#!/usr/bin/env bash
set -euo pipefail
export AWS_PAGER=""

CLUSTER="gbn-proto-phase1-scale-n100-cluster"
REGION="us-east-1"
CREATOR_TASK="arn:aws:ecs:us-east-1:138472308340:task/gbn-proto-phase1-scale-n100-cluster/df5634b805814e73868f5497a4bb009b"
CREATOR_CONTAINER="creator"

# all-ECS hop path: guard -> middle -> exit
PAYLOAD='{"cmd":"SendDummy","size":512,"path":["10.0.0.6:9001","10.0.0.75:9001","10.0.3.153:9001"]}'
PAYLOAD_B64="$(printf '%s' "$PAYLOAD" | base64 -w0)"

echo "UTC_START: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "Payload: $PAYLOAD"

aws ecs execute-command \
  --cluster "$CLUSTER" \
  --task "$CREATOR_TASK" \
  --container "$CREATOR_CONTAINER" \
  --region "$REGION" \
  --interactive \
  --command "python3 -c \"import base64,socket; p=base64.b64decode('$PAYLOAD_B64'); s=socket.create_connection(('127.0.0.1',5050),3); s.settimeout(15); s.sendall(p); s.shutdown(1); print(s.recv(65535).decode(errors='replace')); s.close()\""

echo "UTC_END: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
