#!/usr/bin/env bash
# relay-control-interactive.sh
#
# Interactive control panel for GBN relay nodes.
#
# Discovers all live ECS tasks (CreatorService / HostileRelayService /
# FreeRelayService) and EC2 instances (SeedRelay / Publisher), then lets
# you run control-port commands against any subset of them.
#
# Commands:
#   DumpDht       — show every node's local DHT / gossip seed store
#   DumpMetadata  — dump the packet ring buffer (with optional entry limit)
#   BroadcastSeed — force a gossip seed broadcast
#   SendDummy     — build a full circuit and push a dummy payload through it
#
# Connection methods:
#   ECS tasks : aws ecs execute-command (ECS Exec / SSM agent)
#   EC2 nodes : aws ssm send-command + docker exec
#
# Usage:
#   bash relay-control-interactive.sh [cluster] [region] [stack-name]

set -euo pipefail
export AWS_PAGER=""

# Preserve and restore TTY state because ECS Exec interactive sessions can
# leave local terminal erase/echo modes altered (shows backspace as '^?').
TTY_STATE=""
if [ -t 0 ]; then
  TTY_STATE="$(stty -g 2>/dev/null || true)"
  stty sane 2>/dev/null || true
fi

restore_tty() {
  if [ -t 0 ]; then
    if [[ -n "${TTY_STATE:-}" ]]; then
      stty "$TTY_STATE" 2>/dev/null || true
    else
      stty sane 2>/dev/null || true
      stty erase '^?' 2>/dev/null || true
    fi
  fi
}

trap restore_tty EXIT INT TERM

CLUSTER="${1:-gbn-proto-phase1-scale-n100-cluster}"
AWS_REGION="${2:-us-east-1}"
STACK_NAME="${3:-gbn-proto-phase1-scale-n100}"

for dep in aws python3; do
  command -v "$dep" >/dev/null 2>&1 || { echo "ERROR: '$dep' not found in PATH." >&2; exit 1; }
done

cf_output() {
  local key="$1"
  local raw
  raw="$(aws cloudformation describe-stacks --stack-name "$STACK_NAME" --region "$AWS_REGION" --output json 2>/dev/null)" || true
  [ -z "$raw" ] && { echo ""; return; }
  echo "$raw" | python3 -c "import json,sys; d=json.load(sys.stdin); o=d['Stacks'][0].get('Outputs',[]); print(next((x['OutputValue'] for x in o if x.get('OutputKey')=='$key'), ''))"
}

_ecs_execute_command_retry() {
  local arn="$1" container="$2" cmd="$3"
  local attempt max_attempts rc raw filtered
  max_attempts=3

  for (( attempt=1; attempt<=max_attempts; attempt++ )); do
    set +e
    raw="$(aws ecs execute-command \
      --cluster   "$CLUSTER" \
      --task      "$arn" \
      --container "$container" \
      --region    "$AWS_REGION" \
      --interactive \
      --command "$cmd" \
      2>&1)"
    rc=$?
    set -e

    filtered="$(printf '%s\n' "$raw" | grep -v 'Session Manager plugin\|Starting session\|Exiting session\|installed successfully' || true)"
    [[ -n "$filtered" ]] && printf '%s\n' "$filtered"

    if [[ $rc -eq 0 ]]; then
      return 0
    fi

    if grep -q "Could not connect to the endpoint URL" <<<"$raw" && (( attempt < max_attempts )); then
      echo "  [WARN] ECS endpoint unreachable (attempt ${attempt}/${max_attempts}); retrying..." >&2
      sleep $((attempt * 2))
      continue
    fi

    return $rc
  done

  return 1
}

# ─────────────────────────── Node registry ───────────────────────────────────
# Parallel arrays — always indexed together.
NODE_LABELS=()   # human-readable display string
NODE_DESCS=()    # "ECS:<task-arn>:<container>"  or  "EC2:<instance-id>:<container>"
NODE_IPS=()      # "<ip>:9001" for relay hops, empty for Creator/Publisher
NODE_ROLES=()    # CREATOR | HOSTILE | FREE | SEED | PUBLISHER

# ─────────────────────────── Discovery ───────────────────────────────────────

_discover_ecs_service() {
  local role="$1" svc="$2"
  if [[ -z "$svc" || "$svc" == "None" ]]; then return 0; fi

  local -a arns
  mapfile -t arns < <(
    aws ecs list-tasks --cluster "$CLUSTER" --service-name "$svc" \
      --desired-status RUNNING --region "$AWS_REGION" \
      --query 'taskArns[]' --output text 2>/dev/null \
      | tr '\t' '\n' | grep -v '^$' || true
  )
  if [[ "${#arns[@]}" -eq 0 ]]; then return 0; fi

  local -a rows
  mapfile -t rows < <(
    aws ecs describe-tasks \
      --cluster "$CLUSTER" --tasks "${arns[@]}" --region "$AWS_REGION" \
      --query 'tasks[?lastStatus==`RUNNING`].[taskArn,attachments[0].details[?name==`privateIPv4Address`].value|[0],containers[0].name]' \
      --output text 2>/dev/null | grep -v '^$' || true
  )

  local row arn ip container short relay_ip
  for row in "${rows[@]}"; do
    arn="$(awk '{print $1}' <<< "$row")"
    ip="$(awk  '{print $2}' <<< "$row")"
    container="$(awk '{print $3}' <<< "$row")"
    if [[ -z "$ip" || "$ip" == "None" || -z "$arn" ]]; then continue; fi
    if [[ -z "$container" || "$container" == "None" ]]; then container="relay"; fi
    short="${arn##*/}"; short="${short:0:8}"
    relay_ip="$ip:9001"
    if [[ "$role" == "CREATOR" ]]; then relay_ip=""; fi
    NODE_LABELS+=("$ip  [$role / $container / ECS ${short}...]")
    NODE_DESCS+=("ECS:$arn:$container")
    NODE_IPS+=("$relay_ip")
    NODE_ROLES+=("$role")
  done
}

_discover_ec2_node() {
  local logical_id="$1" container="$2" role="$3" port="$4"

  local iid
  iid="$(aws cloudformation describe-stack-resources \
    --stack-name "$STACK_NAME" --logical-resource-id "$logical_id" \
    --region "$AWS_REGION" \
    --query 'StackResources[0].PhysicalResourceId' --output text 2>/dev/null || echo "")"
  if [[ -z "$iid" || "$iid" == "None" ]]; then return 0; fi

  local state ip
  state="$(aws ec2 describe-instances --instance-ids "$iid" --region "$AWS_REGION" \
    --query 'Reservations[0].Instances[0].State.Name' --output text 2>/dev/null || echo "")"
  if [[ "$state" != "running" ]]; then return 0; fi

  ip="$(aws ec2 describe-instances --instance-ids "$iid" --region "$AWS_REGION" \
    --query 'Reservations[0].Instances[0].PrivateIpAddress' --output text 2>/dev/null || echo "")"
  if [[ -z "$ip" || "$ip" == "None" ]]; then return 0; fi

  NODE_LABELS+=("$ip  [$role / $container / EC2 $iid]")
  NODE_DESCS+=("EC2:$iid:$container")
  NODE_IPS+=("$ip:$port")
  NODE_ROLES+=("$role")
}

discover_all_nodes() {
  echo "Discovering live nodes..." >&2

  local hostile_svc free_svc creator_svc
  hostile_svc="$(aws ecs list-services --cluster "$CLUSTER" --region "$AWS_REGION" \
    --query 'serviceArns[?contains(@,`HostileRelayService`)]|[0]' --output text 2>/dev/null || echo "")"
  free_svc="$(aws ecs list-services --cluster "$CLUSTER" --region "$AWS_REGION" \
    --query 'serviceArns[?contains(@,`FreeRelayService`)]|[0]' --output text 2>/dev/null || echo "")"
  creator_svc="$(aws ecs list-services --cluster "$CLUSTER" --region "$AWS_REGION" \
    --query 'serviceArns[?contains(@,`CreatorService`)]|[0]' --output text 2>/dev/null || echo "")"

  _discover_ecs_service "CREATOR" "$creator_svc"
  _discover_ecs_service "HOSTILE" "$hostile_svc"
  _discover_ecs_service "FREE"    "$free_svc"
  _discover_ec2_node "SeedRelayInstance" "gbn-seed-relay" "SEED"      9001
  _discover_ec2_node "PublisherInstance"  "gbn-publisher"  "PUBLISHER" 7001

  echo "  Found ${#NODE_LABELS[@]} live node(s)." >&2
  if [[ "${#NODE_LABELS[@]}" -eq 0 ]]; then echo "ERROR: no nodes discovered." >&2; exit 1; fi
}

# ─────────────────────────── Node table ──────────────────────────────────────

print_node_table() {
  echo ""
  printf "  %-4s  %-10s  %s\n" "IDX" "ROLE" "NODE"
  printf "  %-4s  %-10s  %s\n" "----" "----------" "--------------------------------------------------------------"
  local i
  for (( i=0; i<${#NODE_LABELS[@]}; i++ )); do
    printf "  [%2d]  %-10s  %s\n" "$((i+1))" "${NODE_ROLES[$i]}" "${NODE_LABELS[$i]}"
  done
  echo ""
}

# ─────────────────────────── Interactive pickers ─────────────────────────────

# _pick_node <prompt> [role-filter]
# Prints numbered menu to stderr; echoes chosen 0-based index to stdout.
_pick_node() {
  local prompt="$1" role_filter="${2:-}"

  local -a p_idxs=() p_labels=()
  local i
  for (( i=0; i<${#NODE_LABELS[@]}; i++ )); do
    [[ -z "$role_filter" || "${NODE_ROLES[$i]}" == "$role_filter" ]] || continue
    p_idxs+=("$i")
    p_labels+=("${NODE_LABELS[$i]}  (${NODE_ROLES[$i]})")
  done

  if [[ "${#p_idxs[@]}" -eq 0 ]]; then
    echo "  ERROR: no nodes available${role_filter:+ for role $role_filter}." >&2
    return 1
  fi

  echo "$prompt" >&2
  local j
  for (( j=0; j<${#p_labels[@]}; j++ )); do
    printf "  [%d] %s\n" "$((j+1))" "${p_labels[$j]}" >&2
  done

  local choice
  while true; do
    read -r -p "  Select [1-${#p_labels[@]}]: " choice
    if [[ "$choice" =~ ^[0-9]+$ ]] && (( choice >= 1 && choice <= ${#p_labels[@]} )); then
      echo "${p_idxs[$((choice-1))]}"; return 0
    fi
    echo "  Invalid selection." >&2
  done
}

# Temp arrays used by _build_filtered_list / _pick_from_tmp to avoid
# passing arrays across function boundaries.
_TMP_IDXS=()
_TMP_LABELS=()

# _build_filtered_list <exclude-desc> <role1> [role2...]
_build_filtered_list() {
  local exclude_desc="$1"; shift
  local -a allowed_roles=("$@")
  _TMP_IDXS=(); _TMP_LABELS=()
  local i
  for (( i=0; i<${#NODE_LABELS[@]}; i++ )); do
    local r="${NODE_ROLES[$i]}"
    local ok=0
    local role
    for role in "${allowed_roles[@]}"; do
      if [[ "$r" == "$role" ]]; then ok=1; break; fi
    done
    if [[ $ok -eq 0 ]]; then continue; fi
    if [[ -n "$exclude_desc" && "${NODE_DESCS[$i]}" == "$exclude_desc" ]]; then continue; fi
    _TMP_IDXS+=("$i")
    _TMP_LABELS+=("${NODE_LABELS[$i]}  ($r)")
  done
}

# _pick_from_tmp <prompt>  — uses _TMP_IDXS / _TMP_LABELS
_pick_from_tmp() {
  local prompt="$1"
  if [[ "${#_TMP_IDXS[@]}" -eq 0 ]]; then
    echo "  ERROR: no nodes available for this selection." >&2; return 1
  fi
  echo "$prompt" >&2
  local j
  for (( j=0; j<${#_TMP_LABELS[@]}; j++ )); do
    printf "  [%d] %s\n" "$((j+1))" "${_TMP_LABELS[$j]}" >&2
  done
  local choice
  while true; do
    read -r -p "  Select [1-${#_TMP_IDXS[@]}]: " choice
    if [[ "$choice" =~ ^[0-9]+$ ]] && (( choice >= 1 && choice <= ${#_TMP_IDXS[@]} )); then
      echo "${_TMP_IDXS[$((choice-1))]}"; return 0
    fi
    echo "  Invalid selection." >&2
  done
}

# _pick_scope <cmd-name>  — echoes "one:<idx>" | "all" | "role:<TAG>"
_pick_scope() {
  local cmd="$1"
  echo "" >&2
  echo "Run '$cmd' on:" >&2
  echo "  [1] Single node (choose from full list)" >&2
  echo "  [2] All nodes" >&2
  echo "  [3] All HOSTILE relay nodes" >&2
  echo "  [4] All FREE relay nodes" >&2
  echo "  [5] All HOSTILE + FREE relay nodes" >&2
  local choice
  while true; do
    read -r -p "  Select [1-5]: " choice
    case "$choice" in
      1) local idx; idx="$(_pick_node "Pick target node:")"; echo "one:$idx"; return 0 ;;
      2) echo "all"; return 0 ;;
      3) echo "role:HOSTILE"; return 0 ;;
      4) echo "role:FREE"; return 0 ;;
      5) echo "role:RELAY"; return 0 ;;
    esac
    echo "  Invalid." >&2
  done
}

_ecr_latest_digest_for_repo() {
  local repo_uri="$1"
  if [[ -z "$repo_uri" || "$repo_uri" == "None" ]]; then return 1; fi

  local repo_name="${repo_uri##*/}"
  aws ecr describe-images \
    --region "$AWS_REGION" \
    --repository-name "$repo_name" \
    --image-ids "imageTag=latest" \
    --query 'imageDetails[0].imageDigest' \
    --output text 2>/dev/null
}

_ssm_command_output() {
  local iid="$1" cmd="$2"

  local params
  params="$(python3 -c "import json,sys; print(json.dumps({'commands':[sys.argv[1]]}))" "$cmd")"

  local command_id
  command_id="$(aws ssm send-command \
    --instance-ids     "$iid" \
    --document-name    "AWS-RunShellScript" \
    --parameters       "$params" \
    --region           "$AWS_REGION" \
    --query            'Command.CommandId' \
    --output text)"

  aws ssm wait command-executed \
    --command-id "$command_id" --instance-id "$iid" \
    --region "$AWS_REGION" 2>/dev/null || true

  aws ssm get-command-invocation \
    --command-id "$command_id" \
    --instance-id "$iid" \
    --region "$AWS_REGION" \
    --query 'StandardOutputContent' \
    --output text 2>/dev/null || true
}

_extract_repo_digest_for_uri() {
  local repo_uri="$1" repo_digests_csv="$2"
  local digest_entry

  IFS=',' read -r -a digest_entries <<< "$repo_digests_csv"
  for digest_entry in "${digest_entries[@]}"; do
    if [[ "$digest_entry" == "$repo_uri@"* ]]; then
      echo "${digest_entry##*@}"
      return 0
    fi
  done
  return 1
}

# ─────────────────────────── Execution primitive ─────────────────────────────

# _send_cmd <node-index> <json-payload>
_send_cmd() {
  local idx="$1" json="$2"
  local desc="${NODE_DESCS[$idx]}"
  local label="${NODE_LABELS[$idx]}"

  # Base64-encode to sidestep all shell quoting issues inside the remote command.
  local b64
  b64="$(printf '%s' "$json" | base64 -w0 2>/dev/null || printf '%s' "$json" | base64)"

  echo ""
  echo "--- ${label} ---"

  if [[ "$desc" == ECS:* ]]; then
    local rest="${desc#ECS:}"
    local arn="${rest%:*}" container="${rest##*:}"
    # Use a strict JSON-only request payload for ECS tasks (no shell/TUI framing).
    local ecs_cmd
    ecs_cmd="$(python3 -c "
import json, sys
b64 = sys.argv[1]
py = (
    \"import base64,json,socket,sys; \"
    \"raw=base64.b64decode('\" + b64 + \"').decode('utf-8'); \"
    \"obj=json.loads(raw); \"
    \"wire=json.dumps(obj,separators=(',',':')).encode('utf-8'); \"
    \"s=socket.create_connection(('127.0.0.1',5050),60); \"
    \"s.settimeout(60); \"
    \"s.sendall(wire); \"
    \"s.shutdown(socket.SHUT_WR); \"
    \"out=b''.join(iter(lambda: s.recv(65536), b'')); \"
    \"s.close(); \"
    \"sys.stdout.write(out.decode('utf-8',errors='replace'))\"
)
print('python3 -c ' + json.dumps(py))
" "$b64")"

    if ! _ecs_execute_command_retry "$arn" "$container" "$ecs_cmd"; then
      echo "  ECS execute-command failed for this node." >&2
    fi
    restore_tty

  elif [[ "$desc" == EC2:* ]]; then
    local rest="${desc#EC2:}"
    local iid="${rest%%:*}" container="${rest#*:}"

    # Build the SSM parameters JSON via Python to handle all escaping correctly.
    local params
    params="$(python3 -c "
import json, sys
b64, cont = sys.argv[1], sys.argv[2]
py = (
    \"import base64,socket; \"
    \"p=base64.b64decode('\" + b64 + \"').decode(); \"
    \"s=socket.create_connection(('127.0.0.1',5050),60); \"
    \"s.settimeout(60); \"
    \"s.sendall(p.encode()+b'\\\\n'); \"
    \"s.shutdown(1); \"
    \"out=b''.join(iter(lambda: s.recv(65536), b'')); \"
    \"print(out.decode(errors='replace')); s.close()\"
)
cmd = 'docker exec ' + cont + ' python3 -c ' + json.dumps(py)
print(json.dumps({'commands': [cmd]}))
" "$b64" "$container")"

    local cid
    cid="$(aws ssm send-command \
      --instance-ids     "$iid" \
      --document-name    "AWS-RunShellScript" \
      --parameters       "$params" \
      --region           "$AWS_REGION" \
      --query 'Command.CommandId' --output text)"

    aws ssm wait command-executed \
      --command-id "$cid" --instance-id "$iid" \
      --region "$AWS_REGION" 2>/dev/null || true

    local status
    status="$(aws ssm get-command-invocation \
      --command-id "$cid" --instance-id "$iid" \
      --region "$AWS_REGION" --query 'Status' --output text 2>/dev/null || echo "Unknown")"
    echo "  SSM status: $status"
    aws ssm get-command-invocation \
      --command-id "$cid" --instance-id "$iid" \
      --region "$AWS_REGION" \
      --query '[StandardOutputContent,StandardErrorContent]' --output text 2>/dev/null || true
  fi
}

# _run_scope <scope> <json>
_run_scope() {
  local scope="$1" json="$2"
  if [[ "$scope" == one:* ]]; then
    _send_cmd "${scope#one:}" "$json"
  elif [[ "$scope" == "all" ]]; then
    local i
    for (( i=0; i<${#NODE_LABELS[@]}; i++ )); do
      _send_cmd "$i" "$json"
    done
  elif [[ "$scope" == role:* ]]; then
    local target="${scope#role:}"
    local i
    for (( i=0; i<${#NODE_LABELS[@]}; i++ )); do
      local r="${NODE_ROLES[$i]}"
      case "$target" in
        RELAY) if [[ "$r" == "HOSTILE" || "$r" == "FREE" ]]; then _send_cmd "$i" "$json"; fi ;;
        *)     if [[ "$r" == "$target" ]]; then _send_cmd "$i" "$json"; fi ;;
      esac
    done
  fi
}

# ─────────────────────────── CloudWatch helpers ──────────────────────────────

# _cw_set_window <lookback-minutes>
# Sets _CW_NOW and _CW_AGO ISO-8601 timestamps used by _cw_stat.
_cw_set_window() {
  local mins="${1:-5}"
  _CW_NOW="$(python3 -c "from datetime import datetime,timezone; print(datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'))")"
  _CW_AGO="$(python3 -c "from datetime import datetime,timedelta,timezone; print((datetime.now(timezone.utc)-timedelta(minutes=${mins})).strftime('%Y-%m-%dT%H:%M:%SZ'))")"
}

# _cw_stat <metric-name> <Sum|Average|Maximum> <outfile>
# Requires _CW_NOW, _CW_AGO, AWS_REGION to be set by caller.
_cw_stat() {
  aws cloudwatch get-metric-statistics \
    --namespace GBN/ScaleTest --metric-name "$1" \
    --start-time "${_CW_AGO:-}" --end-time "${_CW_NOW:-}" \
    --period 60 --statistics "$2" \
    --region "$AWS_REGION" --output json 2>/dev/null \
    > "$3" || echo '{"Datapoints":[]}' > "$3"
}

# ─────────────────────────── Commands ────────────────────────────────────────

do_dump_dht() {
  local scope; scope="$(_pick_scope "DumpDht")"
  _run_scope "$scope" '{"cmd":"DumpDht"}'
}

do_dump_metadata() {
  echo "" >&2
  read -r -p "Max entries to return (leave blank = all): " lim
  read -r -p "Filter by Chain ID (leave blank = all): " filter_chain_id
  local json
  if [[ "$lim" =~ ^[1-9][0-9]*$ ]]; then
    json="{\"cmd\":\"DumpMetadata\",\"limit\":$lim}"
  else
    json='{"cmd":"DumpMetadata","limit":0}'
  fi
  if [[ -n "${filter_chain_id:-}" ]]; then
    json="$(python3 -c "
import sys, json
d = json.load(sys.stdin)
d['chain_id'] = sys.argv[1]
print(json.dumps(d))
" "$filter_chain_id" <<< "$json")"
  fi
  local scope; scope="$(_pick_scope "DumpMetadata")"
  _run_scope "$scope" "$json"
}

do_broadcast_seed() {
  local scope; scope="$(_pick_scope "BroadcastSeed")"
  _run_scope "$scope" '{"cmd":"BroadcastSeed"}'
}

do_unicast_dht() {
  echo "" >&2
  echo "======  UnicastDHT: send local NodeAnnounce to a single peer  ======" >&2

  # Pick the node that will SEND the unicast (the sender)
  local sender_idx
  sender_idx="$(_pick_node "Pick SENDER node (which relay sends its NodeAnnounce):")"

  # Build list of candidate targets — any node that has a relay IP (excludes Creator/Publisher)
  _TMP_IDXS=(); _TMP_LABELS=()
  local i
  for (( i=0; i<${#NODE_LABELS[@]}; i++ )); do
    local ip="${NODE_IPS[$i]}"
    if [[ -z "$ip" ]]; then continue; fi          # Creator/Publisher have no relay IP
    if [[ "$i" -eq "$sender_idx" ]]; then continue; fi  # exclude sender from targets
    _TMP_IDXS+=("$i")
    _TMP_LABELS+=("$ip  ${NODE_LABELS[$i]}  (${NODE_ROLES[$i]})")
  done

  if [[ "${#_TMP_IDXS[@]}" -eq 0 ]]; then
    echo "  ERROR: no valid target nodes (need at least one other node with a relay IP)." >&2
    return 1
  fi

  local target_idx
  target_idx="$(_pick_from_tmp "Pick TARGET node (who receives the NodeAnnounce):")"
  local target_ip="${NODE_IPS[$target_idx]}"

  echo "" >&2
  echo "  Sender : ${NODE_LABELS[$sender_idx]}" >&2
  echo "  Target : $target_ip  ${NODE_LABELS[$target_idx]}" >&2
  echo "" >&2

  local payload
  payload="$(printf '{"cmd":"UnicastDHT","target_addr":"%s"}' "$target_ip")"

  _send_cmd "$sender_idx" "$payload"
}

_do_send_dummy_scale() {
  echo ""
  echo "======  SendDummy / Scale mode  ======"
  echo "  Creator auto-selects one unique circuit per chunk from its local DHT."
  echo ""

  local creator_idx
  creator_idx="$(_pick_node "Pick Creator node:" "CREATOR")"

  local chunk_count chunk_size
  read -r -p "  Chunks to send [10]: " chunk_count
  chunk_count="${chunk_count:-10}"
  [[ "$chunk_count" =~ ^[1-9][0-9]*$ ]] || { echo "  Defaulting to 10." >&2; chunk_count=10; }

  read -r -p "  Chunk size in bytes [8192]: " chunk_size
  chunk_size="${chunk_size:-8192}"
  [[ "$chunk_size" =~ ^[1-9][0-9]*$ ]] || { echo "  Defaulting to 8192." >&2; chunk_size=8192; }

  local total_bytes
  total_bytes="$(python3 -c "print(${chunk_count} * ${chunk_size})")"

  echo ""
  printf "+-------------+-------------------------------------------------------------+\n"
  printf "| %-11s | %-59s |\n" "Creator"  "${NODE_LABELS[$creator_idx]}"
  printf "| %-11s | %-59s |\n" "Chunks"   "$chunk_count x $chunk_size B = $total_bytes B"
  printf "| %-11s | %-59s |\n" "Circuits" "auto-built from Creator DHT (one unique path per chunk)"
  printf "+-------------+-------------------------------------------------------------+\n"
  echo ""
  read -r -p "Proceed? [Y/n]: " confirm
  [[ "${confirm,,}" == "n" ]] && { echo "Aborted."; return 0; }

  local payload
  payload="$(printf '{"cmd":"SendScale","chunk_count":%s,"chunk_size":%s}' "$chunk_count" "$chunk_size")"

  echo ""
  echo "Sending SendScale to Creator..."
  local scale_output
  scale_output="$(_send_cmd "$creator_idx" "$payload")"
  printf '%s\n' "$scale_output"

  # ── Parse ScaleResult: display circuit table, write relay IPs + chain_id ──
  local parse_tmp
  parse_tmp="$(mktemp)"

  printf '%s\n' "$scale_output" | python3 - "$parse_tmp" "$chunk_size" <<'PYEOF'
import json, sys

parse_file = sys.argv[1]
chunk_sz   = int(sys.argv[2])

result   = None
chain_id = ''

for line in sys.stdin.read().splitlines():
    line = line.strip()
    if not line: continue
    try:
        d = json.loads(line)
        t = d.get('type', '')
        if t == 'ScaleResult':
            result = d
        elif t == 'TraceId' and not chain_id:
            chain_id = d.get('chain_id', '')
        elif t == 'Error':
            print(f'\n  ERROR from Creator: {d.get("reason","")}')
    except:
        pass

relay_ips = set()

if result:
    acked   = result.get('acked', 0)
    total   = result.get('total', 0)
    elapsed = result.get('elapsed_ms', 0)
    chunks  = result.get('chunks', [])
    icon    = 'PASS' if acked == total > 0 else 'PARTIAL' if acked > 0 else 'FAIL'

    print(f'\n  === Scale Result: {icon} ===')
    print(f'  ACKed  : {acked}/{total} chunks')
    print(f'  Bytes  : {acked} x {chunk_sz} B = {acked*chunk_sz} B delivered')
    print(f'  Elapsed: {elapsed} ms')
    print()
    print(f'  {"#":>5}  {"Guard":22}  {"Middle":22}  {"Exit":22}  Status')
    print('  ' + '-' * 83)
    for c in sorted(chunks, key=lambda x: x.get('chunk_index', 0)):
        guard  = c.get('guard_addr',  '?')
        middle = c.get('middle_addr', '?')
        exit_  = c.get('exit_addr',   '?')
        st     = 'ACK ' if c.get('acked') else 'FAIL'
        err    = c.get('error') or ''
        err_s  = f'  [{err[:55]}]' if err else ''
        relay_ips.update([guard, middle, exit_])
        print(f'  {c.get("chunk_index",0):>5}  {guard:<22}  {middle:<22}  {exit_:<22}  {st}{err_s}')
else:
    print('\n  (no ScaleResult in Creator response)')

relay_ips.discard('?')
relay_ips.discard('')

with open(parse_file, 'w') as f:
    json.dump({'chain_id': chain_id, 'relay_ips': sorted(relay_ips)}, f)
PYEOF

  local chain_id relay_ips_json
  chain_id="$(python3 -c "import json,sys; print(json.load(open(sys.argv[1])).get('chain_id',''))" "$parse_tmp")"
  relay_ips_json="$(python3 -c "import json,sys; print(json.dumps(json.load(open(sys.argv[1])).get('relay_ips',[])))" "$parse_tmp")"
  rm -f "$parse_tmp"

  [[ -n "$chain_id" ]] && echo "" && echo "  Root Chain ID: $chain_id"

  # ── Build ordered circuit node list: Creator + relays (by IP) + Publisher ──
  local -a circuit_node_idxs=("$creator_idx")

  if [[ -n "$relay_ips_json" && "$relay_ips_json" != "[]" ]]; then
    local relay_ip ni
    while IFS= read -r relay_ip; do
      for (( ni=0; ni<${#NODE_IPS[@]}; ni++ )); do
        if [[ "${NODE_IPS[$ni]}" == "$relay_ip" ]]; then
          circuit_node_idxs+=("$ni"); break
        fi
      done
    done < <(python3 -c "import sys,json; [print(x) for x in json.loads(sys.argv[1])]" "$relay_ips_json")
  fi

  local pi
  for (( pi=0; pi<${#NODE_LABELS[@]}; pi++ )); do
    if [[ "${NODE_ROLES[$pi]}" == "PUBLISHER" ]]; then circuit_node_idxs+=("$pi"); break; fi
  done

  echo ""
  printf "  Circuit nodes identified (%d total):\n" "${#circuit_node_idxs[@]}"
  local k
  for (( k=0; k<${#circuit_node_idxs[@]}; k++ )); do
    local idx="${circuit_node_idxs[$k]}"
    printf "    [%d] %-12s  %s\n" "$((k+1))" "${NODE_ROLES[$idx]}" "${NODE_LABELS[$idx]}"
  done

  echo ""
  read -r -p "DumpMetadata on all circuit nodes? [y/N]: " dump_yn
  [[ "${dump_yn,,}" != "y" ]] && return 0

  local dm_json='{"cmd":"DumpMetadata","limit":0}'
  if [[ -n "$chain_id" ]]; then
    dm_json="$(python3 -c "
import sys, json
d = json.load(sys.stdin)
d['chain_id'] = sys.argv[1]
print(json.dumps(d))
" "$chain_id" <<< "$dm_json")"
    echo "  Filtering to chain: $chain_id" >&2
  fi

  echo ""
  echo "====  DumpMetadata: Scale circuit nodes  ===="
  local n=1
  for idx in "${circuit_node_idxs[@]}"; do
    echo ">>> [$n/${#circuit_node_idxs[@]}] ${NODE_ROLES[$idx]}  ${NODE_LABELS[$idx]}"
    _send_cmd "$idx" "$dm_json"
    (( n++ ))
  done
}

do_live_metrics() {
  local interval=30
  read -r -p "Refresh interval in seconds [30]: " iv
  [[ "$iv" =~ ^[1-9][0-9]*$ ]] && interval="$iv"
  echo "  Polling every ${interval}s -- Ctrl-C to exit" >&2

  while true; do
    local tmpdir
    tmpdir="$(mktemp -d)"
    _cw_set_window 5

    _cw_stat GossipBandwidthBytes       Sum     "$tmpdir/bw.json"       &
    _cw_stat GossipBudgetBytesPerWindow Maximum "$tmpdir/budget.json"   &
    _cw_stat GossipMessagesDropped      Sum     "$tmpdir/dropped.json"  &
    _cw_stat GossipMessagesSeen         Sum     "$tmpdir/seen.json"     &
    _cw_stat GossipLazyRepairs          Sum     "$tmpdir/repairs.json"  &
    _cw_stat DhtNodeCount               Average "$tmpdir/dht.json"      &
    _cw_stat GossipEagerPeerCount       Average "$tmpdir/eager.json"    &
    _cw_stat GossipLazyPeerCount        Average "$tmpdir/lazy.json"     &
    _cw_stat CircuitBuildResult         Sum     "$tmpdir/circuits.json" &
    _cw_stat ChunkE2ELatencyMs          Average "$tmpdir/e2e.json"      &
    _cw_stat RelayBytesForwarded        Sum     "$tmpdir/relayb.json"   &
    _cw_stat PublisherChunksReceived    Sum     "$tmpdir/pubchk.json"   &
    _cw_stat BootstrapResult            Sum     "$tmpdir/boot.json"     &
    wait

    clear
    python3 - "$_CW_NOW" "$STACK_NAME" "$tmpdir" <<'PYEOF'
import json, sys

def latest(path, stat):
    try:
        pts = json.load(open(path)).get('Datapoints', [])
        if not pts: return None
        pts.sort(key=lambda p: p['Timestamp'], reverse=True)
        return pts[0].get(stat)
    except:
        return None

def fv(v, fmt):
    return (fmt % v) if v is not None else '--'

now_str, stack, tmpdir = sys.argv[1], sys.argv[2], sys.argv[3]

bw       = latest(f'{tmpdir}/bw.json',       'Sum')
budget   = latest(f'{tmpdir}/budget.json',   'Maximum')
dropped  = latest(f'{tmpdir}/dropped.json',  'Sum')
seen     = latest(f'{tmpdir}/seen.json',     'Sum')
repairs  = latest(f'{tmpdir}/repairs.json',  'Sum')
dht      = latest(f'{tmpdir}/dht.json',      'Average')
eager    = latest(f'{tmpdir}/eager.json',    'Average')
lazy_p   = latest(f'{tmpdir}/lazy.json',     'Average')
circuits = latest(f'{tmpdir}/circuits.json', 'Sum')
e2e      = latest(f'{tmpdir}/e2e.json',      'Average')
relayb   = latest(f'{tmpdir}/relayb.json',   'Sum')
pubchk   = latest(f'{tmpdir}/pubchk.json',   'Sum')
boot     = latest(f'{tmpdir}/boot.json',     'Sum')

bw_s   = f'{bw/1048576:.1f} MB'     if bw     is not None else '--'
bud_s  = f'{budget/1048576:.1f} MB' if budget is not None else '--'
util_s = f'{bw/budget*100:.1f}%'    if (bw is not None and budget and budget > 0) else '--'
dr_s   = f'{dropped/seen*100:.1f}%' if (dropped is not None and seen and seen > 0) else '--'
e2e_s  = f'{e2e:.0f} ms'            if e2e    is not None else '--'
relb_s = f'{relayb/1024:.1f} KB'    if relayb is not None else '--'

W = 79
print()
print(f'  GBN Live Metrics \u2014 {now_str}  (Stack: {stack})')
print('  ' + '\u2500' * W)
print('  GOSSIP HEALTH')
print(f'    Bandwidth (5m window)  : {bw_s:>10}  /  budget {bud_s:>10}  utilization {util_s:>8}')
print(f'    Messages seen          : {fv(seen,    "%12.0f")}')
print(f'    Messages dropped       : {fv(dropped, "%12.0f")}   drop rate {dr_s:>8}')
print(f'    Lazy repairs (IWant)   : {fv(repairs, "%12.0f")}')
print()
print('  GOSSIP MESH  (avg across nodes)')
print(f'    DHT node count         : {fv(dht,    "%12.1f")}')
print(f'    Eager peers / node     : {fv(eager,  "%12.1f")}')
print(f'    Lazy peers  / node     : {fv(lazy_p, "%12.1f")}')
print()
print('  ONION ROUTING')
print(f'    Circuit builds (5 min) : {fv(circuits, "%12.0f")}')
print(f'    E2E chunk latency (avg): {e2e_s:>12}')
print(f'    Relay bytes forwarded  : {relb_s:>12}')
print()
print('  DELIVERY')
print(f'    Publisher chunks recv  : {fv(pubchk, "%12.0f")}')
print(f'    Bootstrap results      : {fv(boot,   "%12.0f")}')
print('  ' + '\u2500' * W)
PYEOF

    rm -rf "$tmpdir"
    echo ""
    echo "  Refreshing in ${interval}s -- Ctrl-C to exit"
    sleep "$interval"
  done
}

do_send_dummy() {
  echo ""
  echo "======  SendDummy  ======"
  echo "  [1] Manual  — pick circuit nodes individually"
  echo "  [2] Scale   — Creator auto-builds one unique circuit per chunk from DHT"
  local sd_mode
  while true; do
    read -r -p "  Mode [1/2]: " sd_mode
    [[ "$sd_mode" == "1" || "$sd_mode" == "2" ]] && break
    echo "  Invalid — enter 1 or 2." >&2
  done

  if [[ "$sd_mode" == "2" ]]; then
    _do_send_dummy_scale
    return 0
  fi

  echo ""
  echo "======  SendDummy: select circuit nodes in order  ======"

  # 1. Creator (CreatorService only)
  local creator_idx
  creator_idx="$(_pick_node "[1/5] Creator  (CreatorService only)" "CREATOR")"

  # 2. Guard (HOSTILE or SEED)
  _build_filtered_list "" "HOSTILE" "SEED"
  local guard_idx
  guard_idx="$(_pick_from_tmp "[2/5] Guard  (HostileRelayService or SeedRelay)")"
  local guard_desc="${NODE_DESCS[$guard_idx]}"

  # 3. Middle (HOSTILE or SEED, Guard excluded)
  _build_filtered_list "$guard_desc" "HOSTILE" "SEED"
  local middle_idx
  middle_idx="$(_pick_from_tmp "[3/5] Middle  (HostileRelayService or SeedRelay; Guard excluded)")"

  # 4. Exit (FREE only)
  local exit_idx
  exit_idx="$(_pick_node "[4/5] Exit  (FreeRelayService only)" "FREE")"

  # 5. Publisher (auto-detect or manual)
  local pub_ip="" pub_idx=-1
  local i
  for (( i=0; i<${#NODE_LABELS[@]}; i++ )); do
    if [[ "${NODE_ROLES[$i]}" == "PUBLISHER" ]]; then pub_idx=$i; break; fi
  done
  if (( pub_idx >= 0 )); then
    pub_ip="${NODE_IPS[$pub_idx]}"
    echo "" >&2
    echo "[5/5] Publisher auto-detected: $pub_ip  (${NODE_LABELS[$pub_idx]})" >&2
  else
    echo "" >&2
    read -r -p "[5/5] Publisher address [10.0.3.10:7001]: " manual_pub
    pub_ip="${manual_pub:-10.0.3.10:7001}"
  fi

  # Payload size
  echo "" >&2
  read -r -p "Payload size in bytes [512]: " size_input
  local size="${size_input:-512}"

  local guard_ip="${NODE_IPS[$guard_idx]}"
  local middle_ip="${NODE_IPS[$middle_idx]}"
  local exit_ip="${NODE_IPS[$exit_idx]}"

  # Summary table
  echo ""
  printf "+----------+----------------------------------------------------------------+\n"
  printf "| %-8s | %-62s |\n" "Creator"   "${NODE_LABELS[$creator_idx]}"
  printf "| %-8s | %-62s |\n" "Guard"     "$guard_ip  ${NODE_LABELS[$guard_idx]}"
  printf "| %-8s | %-62s |\n" "Middle"    "$middle_ip  ${NODE_LABELS[$middle_idx]}"
  printf "| %-8s | %-62s |\n" "Exit"      "$exit_ip  ${NODE_LABELS[$exit_idx]}"
  printf "| %-8s | %-62s |\n" "Publisher" "$pub_ip"
  printf "| %-8s | %-62s |\n" "Size"      "$size bytes"
  printf "+----------+----------------------------------------------------------------+\n"
  echo ""
  read -r -p "Proceed? [Y/n]: " confirm
  if [[ "${confirm,,}" == "n" ]]; then echo "Aborted."; return 0; fi

  local payload
  payload="$(printf '{"cmd":"SendDummy","size":%s,"path":["%s","%s","%s"]}' \
    "$size" "$guard_ip" "$middle_ip" "$exit_ip")"

  echo ""
  echo "Sending SendDummy to Creator..."
  local creator_output chain_id=""
  creator_output="$(_send_cmd "$creator_idx" "$payload")"
  printf '%s\n' "$creator_output"

  # Extract root chain_id from the TraceId line emitted before circuit build
  chain_id="$(printf '%s\n' "$creator_output" | python3 -c "
import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try:
        d = json.loads(line)
        if d.get('type') == 'TraceId' and 'chain_id' in d:
            sys.stdout.write(d['chain_id'])
            break
    except:
        pass
" 2>/dev/null || true)"

  if [[ -n "$chain_id" ]]; then
    echo "" >&2
    echo "  Root Chain ID: $chain_id" >&2
  fi

  # Optional post-test DumpMetadata
  echo ""
  read -r -p "DumpMetadata on all circuit nodes now? [y/N]: " dump_yn
  if [[ "${dump_yn,,}" != "y" ]]; then return 0; fi

  read -r -p "Max entries per node (leave blank = all): " lim_input
  local dm_json
  if [[ "$lim_input" =~ ^[1-9][0-9]*$ ]]; then
    dm_json="{\"cmd\":\"DumpMetadata\",\"limit\":$lim_input}"
  else
    dm_json='{"cmd":"DumpMetadata","limit":0}'
  fi

  if [[ -n "$chain_id" ]]; then
    dm_json="$(python3 -c "
import sys, json
d = json.load(sys.stdin)
d['chain_id'] = sys.argv[1]
print(json.dumps(d))
" "$chain_id" <<< "$dm_json")"
    echo "  DumpMetadata filtered to chain: $chain_id" >&2
  fi

  echo ""
  echo "====  DumpMetadata: circuit nodes in order  ===="
  local creator_meta guard_meta middle_meta exit_meta pub_meta

  echo ">>> [1/5] Creator"
  creator_meta="$(_send_cmd "$creator_idx" "$dm_json")"
  printf '%s\n' "$creator_meta"

  echo ">>> [2/5] Guard"
  guard_meta="$(_send_cmd "$guard_idx" "$dm_json")"
  printf '%s\n' "$guard_meta"

  echo ">>> [3/5] Middle"
  middle_meta="$(_send_cmd "$middle_idx" "$dm_json")"
  printf '%s\n' "$middle_meta"

  echo ">>> [4/5] Exit"
  exit_meta="$(_send_cmd "$exit_idx" "$dm_json")"
  printf '%s\n' "$exit_meta"

  if (( pub_idx >= 0 )); then
    echo ">>> [5/5] Publisher"
    pub_meta="$(_send_cmd "$pub_idx" "$dm_json")"
    printf '%s\n' "$pub_meta"
  else
    pub_meta=""
    echo ">>> [5/5] Publisher - skipped (no EC2 descriptor found)"
  fi

  # Optional HTML report
  echo ""
  read -r -p "Generate HTML report? [Y/n]: " report_yn
  [[ "${report_yn,,}" == "n" ]] && return 0

  local report_dir
  report_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/reports"
  mkdir -p "$report_dir"

  local ts_file chain_safe report_file
  ts_file="$(python3 -c 'from datetime import datetime,timezone; print(datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S"))')"
  chain_safe="$(printf '%s' "${chain_id:-unknown}" | python3 -c 'import sys,re; print(re.sub(r"[^a-zA-Z0-9]","-",sys.stdin.read().strip())[:12])')"
  report_file="$report_dir/senddummy-${ts_file}-${chain_safe}.html"

  echo "  Collecting CloudWatch snapshot..." >&2
  local cw_tmp
  cw_tmp="$(mktemp -d)"
  _cw_set_window 5

  _cw_stat GossipBandwidthBytes       Sum     "$cw_tmp/bw.json"       &
  _cw_stat GossipBudgetBytesPerWindow Maximum "$cw_tmp/budget.json"   &
  _cw_stat GossipMessagesDropped      Sum     "$cw_tmp/dropped.json"  &
  _cw_stat GossipMessagesSeen         Sum     "$cw_tmp/seen.json"     &
  _cw_stat GossipLazyRepairs          Sum     "$cw_tmp/repairs.json"  &
  _cw_stat DhtNodeCount               Average "$cw_tmp/dht.json"      &
  _cw_stat GossipEagerPeerCount       Average "$cw_tmp/eager.json"    &
  _cw_stat GossipLazyPeerCount        Average "$cw_tmp/lazy.json"     &
  _cw_stat CircuitBuildResult         Sum     "$cw_tmp/circuits.json" &
  _cw_stat ChunkE2ELatencyMs          Average "$cw_tmp/e2e.json"      &
  _cw_stat RelayBytesForwarded        Sum     "$cw_tmp/relayb.json"   &
  _cw_stat PublisherChunksReceived    Sum     "$cw_tmp/pubchk.json"   &
  _cw_stat BootstrapResult            Sum     "$cw_tmp/boot.json"     &
  wait

  printf '%s' "$creator_meta" > "$cw_tmp/meta_creator.txt"
  printf '%s' "$guard_meta"   > "$cw_tmp/meta_guard.txt"
  printf '%s' "$middle_meta"  > "$cw_tmp/meta_middle.txt"
  printf '%s' "$exit_meta"    > "$cw_tmp/meta_exit.txt"
  printf '%s' "$pub_meta"     > "$cw_tmp/meta_pub.txt"

  python3 - \
    "$report_file" \
    "$STACK_NAME" "$CLUSTER" \
    "${NODE_LABELS[$creator_idx]}" "${NODE_LABELS[$guard_idx]}" \
    "${NODE_LABELS[$middle_idx]}"  "${NODE_LABELS[$exit_idx]}"  "$pub_ip" \
    "$guard_ip" "$middle_ip" "$exit_ip" \
    "${chain_id:-unknown}" "$size" \
    "$_CW_NOW" "$cw_tmp" \
    <<'PYEOF'
import json, sys, html, re

def latest(path, stat):
    try:
        pts = json.load(open(path)).get('Datapoints', [])
        if not pts: return None
        pts.sort(key=lambda p: p['Timestamp'], reverse=True)
        return pts[0].get(stat)
    except:
        return None

def fv(v, fmt):
    return (fmt % v) if v is not None else '--'

def fmt_bytes(v):
    if v is None: return '--'
    if v >= 1048576: return f'{v/1048576:.1f} MB'
    if v >= 1024:    return f'{v/1024:.1f} KB'
    return f'{v:.0f} B'

def parse_meta(path):
    try:
        for line in reversed(open(path).read().splitlines()):
            line = line.strip()
            if line.startswith('{') and 'packets' in line:
                return json.loads(line).get('packets', [])
    except:
        pass
    return []

(report_file, stack, cluster,
 lbl_creator, lbl_guard, lbl_middle, lbl_exit, lbl_pub,
 ip_guard, ip_middle, ip_exit,
 chain_id, size, ts_now, cw_tmp) = sys.argv[1:16]

bw       = latest(f'{cw_tmp}/bw.json',       'Sum')
budget   = latest(f'{cw_tmp}/budget.json',   'Maximum')
dropped  = latest(f'{cw_tmp}/dropped.json',  'Sum')
seen     = latest(f'{cw_tmp}/seen.json',     'Sum')
repairs  = latest(f'{cw_tmp}/repairs.json',  'Sum')
dht      = latest(f'{cw_tmp}/dht.json',      'Average')
eager    = latest(f'{cw_tmp}/eager.json',    'Average')
lazy_p   = latest(f'{cw_tmp}/lazy.json',     'Average')
circuits = latest(f'{cw_tmp}/circuits.json', 'Sum')
e2e      = latest(f'{cw_tmp}/e2e.json',      'Average')
relayb   = latest(f'{cw_tmp}/relayb.json',   'Sum')
pubchk   = latest(f'{cw_tmp}/pubchk.json',   'Sum')
boot     = latest(f'{cw_tmp}/boot.json',     'Sum')

nodes = [
    ('Creator',   parse_meta(f'{cw_tmp}/meta_creator.txt'), lbl_creator, ''),
    ('Guard',     parse_meta(f'{cw_tmp}/meta_guard.txt'),   lbl_guard,   ip_guard),
    ('Middle',    parse_meta(f'{cw_tmp}/meta_middle.txt'),  lbl_middle,  ip_middle),
    ('Exit',      parse_meta(f'{cw_tmp}/meta_exit.txt'),    lbl_exit,    ip_exit),
    ('Publisher', parse_meta(f'{cw_tmp}/meta_pub.txt'),     lbl_pub,     ''),
]

BADGE = {
    'ComponentInput':          '#3a7bd5',
    'ComponentOutput':         '#27a745',
    'ComponentError':          '#dc3545',
    'RelayData(Intermediate)': '#fd7e14',
    'RelayAckPeel':            '#fd7e14',
    'RelayAckBuild':           '#fd7e14',
    'ExitDelivery':            '#fd7e14',
}

def badge(action):
    c = BADGE.get(action, '#6c757d')
    return (f'<span style="background:{c};color:#fff;padding:1px 6px;'
            f'border-radius:3px;font-size:11px;margin-right:6px">'
            f'{html.escape(action)}</span>')

trace_html = ''
for (role, packets, label, ip) in nodes:
    ip_part = f'{html.escape(ip)} &nbsp;' if ip else ''
    rows = ''
    for p in sorted(packets, key=lambda x: x.get('ts', '')):
        rows += (
            f'<tr>'
            f'<td style="color:#888;white-space:nowrap;padding:3px 8px">{html.escape(str(p.get("ts","")))}</td>'
            f'<td style="padding:3px 8px">{badge(p.get("action",""))}</td>'
            f'<td style="color:#aaa;padding:3px 8px">{html.escape(str(p.get("bytes","")))}</td>'
            f'<td style="color:#ccc;word-break:break-all;font-size:12px;padding:3px 8px">{html.escape(str(p.get("msg","")))}</td>'
            f'<td style="color:#555;word-break:break-all;font-size:11px;padding:3px 8px">{html.escape(str(p.get("chain","")))}</td>'
            f'</tr>'
        )
    if not rows:
        rows = '<tr><td colspan="5" style="color:#555;font-style:italic;padding:6px 8px">no packets captured</td></tr>'
    trace_html += (
        f'<details open><summary style="cursor:pointer;font-size:14px;font-weight:bold;'
        f'color:#e0e0e0;padding:8px 0">{html.escape(role)} &mdash; {ip_part}{html.escape(label)}</summary>'
        f'<table style="width:100%;border-collapse:collapse;font-family:monospace;font-size:12px">'
        f'<thead><tr style="color:#666;border-bottom:1px solid #333">'
        f'<th style="text-align:left;padding:4px 8px">Timestamp</th>'
        f'<th style="text-align:left;padding:4px 8px">Action</th>'
        f'<th style="text-align:left;padding:4px 8px">Bytes</th>'
        f'<th style="text-align:left;padding:4px 8px">Message</th>'
        f'<th style="text-align:left;padding:4px 8px">Chain</th>'
        f'</tr></thead><tbody>{rows}</tbody></table></details>'
    )

bw_s   = fmt_bytes(bw)
bud_s  = fmt_bytes(budget)
util_s = f'{bw/budget*100:.1f}%'    if (bw and budget and budget > 0) else '--'
dr_s   = f'{dropped/seen*100:.1f}%' if (dropped is not None and seen and seen > 0) else '--'
e2e_s  = f'{e2e:.0f} ms'            if e2e    is not None else '--'
relb_s = fmt_bytes(relayb)

def cw_row(lbl, val):
    return (f'<tr><td style="color:#888;padding:3px 14px;width:240px">{html.escape(lbl)}</td>'
            f'<td style="text-align:right;padding:3px 14px">{html.escape(str(val))}</td></tr>')

cw_html = (
    '<table style="border-collapse:collapse;font-family:monospace;font-size:13px">'
    '<tbody>'
    + cw_row('Gossip Bandwidth (5m)',   bw_s)
    + cw_row('Gossip Budget/window',    bud_s)
    + cw_row('Bandwidth Utilization',   util_s)
    + cw_row('Messages Seen',           fv(seen,     '%.0f'))
    + cw_row('Messages Dropped',        fv(dropped,  '%.0f'))
    + cw_row('Drop Rate',               dr_s)
    + cw_row('Lazy Repairs (IWant)',    fv(repairs,  '%.0f'))
    + cw_row('DHT Node Count (avg)',    fv(dht,      '%.1f'))
    + cw_row('Eager Peers/node',        fv(eager,    '%.1f'))
    + cw_row('Lazy Peers/node',         fv(lazy_p,   '%.1f'))
    + cw_row('Circuit Builds (5m)',     fv(circuits, '%.0f'))
    + cw_row('E2E Chunk Latency',       e2e_s)
    + cw_row('Relay Bytes Forwarded',   relb_s)
    + cw_row('Publisher Chunks Recv',   fv(pubchk,   '%.0f'))
    + cw_row('Bootstrap Results',       fv(boot,     '%.0f'))
    + '</tbody></table>'
)

doc = f'''<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>GBN SendDummy Report \u2014 {html.escape(ts_now)}</title>
  <style>
    body {{ background:#1a1a1a; color:#d4d4d4; font-family:monospace; margin:0; padding:24px 32px; }}
    h1 {{ color:#e8e8e8; border-bottom:1px solid #333; padding-bottom:10px; margin-bottom:20px; }}
    h2 {{ color:#b0b0b0; margin-top:28px; margin-bottom:8px; font-size:15px; letter-spacing:.5px; }}
    table tr:hover {{ background:#222; }}
    .chain {{ background:#0d2b0d; border:1px solid #2d5a2d; padding:8px 14px;
              border-radius:4px; color:#7ec87e; font-size:13px;
              word-break:break-all; margin:8px 0 16px 0; }}
    details {{ margin:10px 0; border:1px solid #2a2a2a; border-radius:4px; padding:6px 12px; }}
    table td, table th {{ border-bottom:1px solid #242424; }}
  </style>
</head>
<body>
  <h1>GBN SendDummy Report</h1>
  <h2>Run Summary</h2>
  <table style="border-collapse:collapse;font-size:13px">
    <tr><td style="color:#888;padding:3px 14px;width:120px">Stack</td><td>{html.escape(stack)}</td></tr>
    <tr><td style="color:#888;padding:3px 14px">Cluster</td><td>{html.escape(cluster)}</td></tr>
    <tr><td style="color:#888;padding:3px 14px">Timestamp</td><td>{html.escape(ts_now)}</td></tr>
    <tr><td style="color:#888;padding:3px 14px">Payload</td><td>{html.escape(size)} bytes</td></tr>
  </table>
  <h2>Circuit</h2>
  <table style="border-collapse:collapse;font-size:13px">
    <thead><tr style="color:#666">
      <th style="text-align:left;padding:3px 14px;width:80px">Hop</th>
      <th style="text-align:left;padding:3px 14px">Node</th>
    </tr></thead>
    <tbody>
      <tr><td style="color:#888;padding:3px 14px">Creator</td><td>{html.escape(lbl_creator)}</td></tr>
      <tr><td style="color:#888;padding:3px 14px">Guard</td><td>{html.escape(ip_guard)} &nbsp; {html.escape(lbl_guard)}</td></tr>
      <tr><td style="color:#888;padding:3px 14px">Middle</td><td>{html.escape(ip_middle)} &nbsp; {html.escape(lbl_middle)}</td></tr>
      <tr><td style="color:#888;padding:3px 14px">Exit</td><td>{html.escape(ip_exit)} &nbsp; {html.escape(lbl_exit)}</td></tr>
      <tr><td style="color:#888;padding:3px 14px">Publisher</td><td>{html.escape(lbl_pub)}</td></tr>
    </tbody>
  </table>
  <h2>Chain ID</h2>
  <div class="chain">{html.escape(chain_id)}</div>
  <h2>Trace Timeline</h2>
  {trace_html}
  <h2>CloudWatch Snapshot <span style="font-size:11px;color:#555">(last 5 min @ {html.escape(ts_now)})</span></h2>
  {cw_html}
</body>
</html>'''

with open(report_file, 'w', encoding='utf-8') as f:
    f.write(doc)
print(f'  Report saved: {report_file}')
PYEOF

  rm -rf "$cw_tmp"
}

do_check_images() {
  echo ""
  echo "===== CheckImages: compare running images to ECR :latest ====="

  local ecr_relay ecr_publisher relay_latest publisher_latest
  ecr_relay="$(cf_output ECRUriRelay)"
  ecr_publisher="$(cf_output ECRUriPublisher)"

  if [[ -z "$ecr_relay" || "$ecr_relay" == "None" || -z "$ecr_publisher" || "$ecr_publisher" == "None" ]]; then
    echo "ERROR: Could not resolve ECRUriRelay and/or ECRUriPublisher from stack '$STACK_NAME' in region '$AWS_REGION'." >&2
    return 1
  fi

  relay_latest="$(_ecr_latest_digest_for_repo "$ecr_relay" || true)"
  publisher_latest="$(_ecr_latest_digest_for_repo "$ecr_publisher" || true)"

  if [[ -z "$relay_latest" || "$relay_latest" == "None" ]]; then
    echo "WARN: unable to fetch latest relay digest from ECR (${ecr_relay})"
  fi
  if [[ -z "$publisher_latest" || "$publisher_latest" == "None" ]]; then
    echo "WARN: unable to fetch latest publisher digest from ECR (${ecr_publisher})"
  fi

  printf "\n%-55s %-14s %-74s %-60s %-12s\n" "NODE" "ROLE" "LOCAL_OBSERVED" "ECR_LATEST_TAGGED_IMAGE" "STATUS"
  printf "%-55s %-14s %-74s %-60s %-12s\n" "-------------------------------------------------------" "-------------" "----------------------------------------------" "------------------------------" "------------"

  local i
  for (( i=0; i<${#NODE_LABELS[@]}; i++ )); do
    local role="${NODE_ROLES[$i]}"
    local desc="${NODE_DESCS[$i]}"
    local expected_uri="$ecr_relay"
    local expected_digest="$relay_latest"

    if [[ "$role" == "PUBLISHER" ]]; then
      expected_uri="$ecr_publisher"
      expected_digest="$publisher_latest"
    fi

    local expected_latest="${expected_uri}:latest"

    local local_image_id="" local_image_ref="" local_repo_digest="" local_status="unknown"
    local raw container

    if [[ "$desc" == ECS:* ]]; then
      local rest="${desc#ECS:}"
      local arn="${rest%:*}"
      local container_name="${rest##*:}"

      raw="$(aws ecs describe-tasks \
        --cluster   "$CLUSTER" \
        --tasks     "$arn" \
        --region    "$AWS_REGION" \
        --query "tasks[0].containers[?name=='$container_name'].[image,imageDigest]" \
        --output text 2>/dev/null || true)"

      if [[ -n "$raw" ]]; then
        local_image_ref="$(awk '{print $1}' <<< "$raw")"
        local_image_id="$(awk '{print $2}' <<< "$raw")"
        local_repo_digest="$local_image_id"
      fi
    elif [[ "$desc" == EC2:* ]]; then
      local rest="${desc#EC2:}"
      local iid="${rest%%:*}"
      container="${rest#*:}"

      local inspect_cmd
      inspect_cmd="img_id=\$(docker inspect \"$container\" --format '{{.Image}}' 2>/dev/null || true); img_ref=\$(docker inspect \"$container\" --format '{{.Config.Image}}' 2>/dev/null || true); repo_digests=''; if [ -n \"\$img_id\" ]; then repo_digests=\$(docker image inspect \"\$img_id\" --format '{{join .RepoDigests \",\"}}' 2>/dev/null || true); fi; printf '%s\t%s\t%s\n' \"\$img_id\" \"\$img_ref\" \"\$repo_digests\""
      raw="$(_ssm_command_output "$iid" "$inspect_cmd")"
      if [[ -n "$raw" ]]; then
        local repo_digests_csv=""
        raw="$(printf '%s' "$raw" | head -n 1)"
        IFS=$'\t' read -r local_image_id local_image_ref repo_digests_csv <<< "$raw"
        local_repo_digest="$(_extract_repo_digest_for_uri "$expected_uri" "$repo_digests_csv" || true)"
      fi
    fi

    if [[ -z "$local_image_id" || "$local_image_id" == "None" ]]; then
      if [[ -n "$local_image_ref" && "$local_image_ref" != "None" ]]; then
        local_image_id="$local_image_ref"
      else
        local_image_id="unknown"
      fi
    fi

    local local_observed="${local_repo_digest:-$local_image_id}"
    if [[ -z "$local_observed" || "$local_observed" == "None" ]]; then
      local_observed="unknown"
    fi

    if [[ "$expected_digest" == "None" || -z "$expected_digest" ]]; then
      local_status="unknown"
    elif [[ -n "$local_repo_digest" && "$local_repo_digest" == "$expected_digest" ]]; then
      local_status="up-to-date"
    elif [[ "$desc" == ECS:* && -n "$local_image_id" && "$local_image_id" == "$expected_digest" ]]; then
      local_status="up-to-date"
    elif [[ "$desc" == EC2:* && "$local_image_ref" == "$expected_latest" ]]; then
      local_status="tag-only"
    elif [[ "$local_observed" == "unknown" ]]; then
      local_status="unknown"
    else
      local_status="out-of-date"
    fi

    printf "%-55s %-14s %-74s %-60s %-12s\n" \
      "${NODE_LABELS[$i]}" "$role" "$local_observed" "$expected_latest" "$local_status"
  done
}

# ─────────────────────────── Main ────────────────────────────────────────────

main() {
  echo "GBN Relay Control Panel"
  echo "  Cluster : $CLUSTER"
  echo "  Region  : $AWS_REGION"
  echo "  Stack   : $STACK_NAME"
  echo ""

  discover_all_nodes
  print_node_table

  while true; do
    echo "Command:"
    select CMD in "DumpDht" "DumpMetadata" "BroadcastSeed" "UnicastDHT" "SendDummy" "LiveMetrics" "checkimages" "Refresh nodes" "Exit"; do
      case "$CMD" in
        DumpDht)           do_dump_dht ;;
        DumpMetadata)      do_dump_metadata ;;
        BroadcastSeed)     do_broadcast_seed ;;
        UnicastDHT)        do_unicast_dht ;;
        SendDummy)         do_send_dummy ;;
        LiveMetrics)       do_live_metrics ;;
        checkimages)       do_check_images ;;
        "Refresh nodes")   NODE_LABELS=(); NODE_DESCS=(); NODE_IPS=(); NODE_ROLES=()
                           discover_all_nodes; print_node_table ;;
        Exit)              echo "Bye."; exit 0 ;;
      esac
      break
    done
    echo ""
  done
}

main "$@"
