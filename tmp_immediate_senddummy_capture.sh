#!/usr/bin/env bash
set -euo pipefail
unset HTTP_PROXY HTTPS_PROXY http_proxy https_proxy

CLUSTER="gbn-proto-phase1-scale-n100-cluster"
REGION="us-east-1"

echo "Resolving services/tasks..."
services=$(aws ecs list-services --cluster "$CLUSTER" --region "$REGION" --query 'serviceArns[]' --output text | tr '\t' '\n')
hostile_svc=$(printf "%s\n" "$services" | grep 'HostileRelayService' | head -n1)
free_svc=$(printf "%s\n" "$services" | grep 'FreeRelayService' | head -n1)
creator_svc=$(printf "%s\n" "$services" | grep 'CreatorService' | head -n1)

mapfile -t hostile_tasks < <(aws ecs list-tasks --cluster "$CLUSTER" --service-name "$hostile_svc" --desired-status RUNNING --region "$REGION" --query 'taskArns[]' --output text | tr '\t' '\n' | sed '/^$/d')
mapfile -t free_tasks < <(aws ecs list-tasks --cluster "$CLUSTER" --service-name "$free_svc" --desired-status RUNNING --region "$REGION" --query 'taskArns[]' --output text | tr '\t' '\n' | sed '/^$/d')
mapfile -t creator_tasks < <(aws ecs list-tasks --cluster "$CLUSTER" --service-name "$creator_svc" --desired-status RUNNING --region "$REGION" --query 'taskArns[]' --output text | tr '\t' '\n' | sed '/^$/d')

if [[ ${#hostile_tasks[@]} -lt 2 || ${#free_tasks[@]} -lt 1 || ${#creator_tasks[@]} -lt 1 ]]; then
  echo "ERROR: expected >=2 hostile, >=1 free, >=1 creator tasks"
  exit 1
fi

read_task_info() {
  local task_arn="$1"
  aws ecs describe-tasks \
    --cluster "$CLUSTER" \
    --tasks "$task_arn" \
    --region "$REGION" \
    --query 'tasks[0].[attachments[0].details[?name==`privateIPv4Address`].value|[0],containers[0].name]' \
    --output text
}

creator_task="${creator_tasks[0]}"
creator_info=$(read_task_info "$creator_task")
creator_ip=$(echo "$creator_info" | awk '{print $1}')
creator_container=$(echo "$creator_info" | awk '{print $2}')

guard_task="${hostile_tasks[0]}"
middle_task="${hostile_tasks[1]}"
exit_task="${free_tasks[0]}"

guard_info=$(read_task_info "$guard_task")
middle_info=$(read_task_info "$middle_task")
exit_info=$(read_task_info "$exit_task")

guard_ip=$(echo "$guard_info" | awk '{print $1}')
guard_container=$(echo "$guard_info" | awk '{print $2}')
middle_ip=$(echo "$middle_info" | awk '{print $1}')
middle_container=$(echo "$middle_info" | awk '{print $2}')
exit_ip=$(echo "$exit_info" | awk '{print $1}')
exit_container=$(echo "$exit_info" | awk '{print $2}')

echo "Creator: $creator_ip task=$creator_task container=$creator_container"
echo "Guard:   $guard_ip task=$guard_task container=$guard_container"
echo "Middle:  $middle_ip task=$middle_task container=$middle_container"
echo "Exit:    $exit_ip task=$exit_task container=$exit_container"

run_control_ecs() {
  local task_arn="$1"
  local container_name="$2"
  local payload="$3"
  local payload_b64
  payload_b64=$(printf '%s' "$payload" | base64 -w0)
  aws ecs execute-command \
    --cluster "$CLUSTER" \
    --task "$task_arn" \
    --container "$container_name" \
    --region "$REGION" \
    --interactive \
    --command "python3 -c \"import base64,socket; p=base64.b64decode('$payload_b64'); s=socket.create_connection(('127.0.0.1',5050),3); s.sendall(p); s.shutdown(1); print(s.recv(65535).decode(errors='replace')); s.close()\"" 2>&1
}

summarize_metadata() {
  local label="$1"
  local raw="$2"
  printf '%s\n' "$raw" | python3 - "$label" <<'PY'
import json, re, sys
label = sys.argv[1]
text = sys.stdin.read()
match = re.findall(r'(\{"type":"Metadata".*\})', text)
if not match:
    print(f'[{label}] ERROR: metadata json not found')
    return_code = 1
    sys.exit(return_code)
obj = json.loads(match[-1])
packets = obj.get('packets', [])
interesting = [p for p in packets if p.get('action') in ('ComponentError','RelayFailureCapture') or 'relay.extend' in p.get('info','') or 'Noise_XX handshake' in p.get('info','')]
print(f'[{label}] packets={len(packets)} interesting={len(interesting)}')
for p in interesting[-12:]:
    ts = p.get('timestamp_ms')
    action = p.get('action')
    info = p.get('info','')
    print(f'  - ts={ts} action={action} info={info}')
PY
}

send_payload=$(printf '{"cmd":"SendDummy","size":512,"path":["%s:9001","%s:9001","%s:9001"]}' "$guard_ip" "$middle_ip" "$exit_ip")
echo "\nSending SendDummy payload: $send_payload"
send_out=$(run_control_ecs "$creator_task" "$creator_container" "$send_payload" || true)
printf '%s\n' "$send_out"

dump_payload='{"cmd":"DumpMetadata"}'

echo "\nImmediate DumpMetadata (within seconds)..."
creator_dump=$(run_control_ecs "$creator_task" "$creator_container" "$dump_payload" || true)
summarize_metadata "Creator" "$creator_dump"

guard_dump=$(run_control_ecs "$guard_task" "$guard_container" "$dump_payload" || true)
summarize_metadata "Guard" "$guard_dump"

middle_dump=$(run_control_ecs "$middle_task" "$middle_container" "$dump_payload" || true)
summarize_metadata "Middle" "$middle_dump"

exit_dump=$(run_control_ecs "$exit_task" "$exit_container" "$dump_payload" || true)
summarize_metadata "Exit" "$exit_dump"