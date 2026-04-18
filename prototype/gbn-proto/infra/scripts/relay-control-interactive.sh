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

# Normalize terminal so backspace works correctly in interactive prompts.
# WSL/Git Bash often has erase=^H but the terminal sends DEL (^?).
if [ -t 0 ]; then stty sane 2>/dev/null || true; fi

CLUSTER="${1:-gbn-proto-phase1-scale-n100-cluster}"
AWS_REGION="${2:-us-east-1}"
STACK_NAME="${3:-gbn-proto-phase1-scale-n100}"

for dep in aws python3; do
  command -v "$dep" >/dev/null 2>&1 || { echo "ERROR: '$dep' not found in PATH." >&2; exit 1; }
done

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
    'import os,base64,socket,sys\\n'
    'p = base64.b64decode(os.environ[\\\"GBN_CTL_B64\\\"]).decode()\\n'
    's = socket.create_connection((\\\"127.0.0.1\\\", 5050), 5)\\n'
    's.sendall(p.encode())\\n'
    's.shutdown(socket.SHUT_WR)\\n'
    'out = s.recv(131072)\\n'
    's.close()\\n'
    'sys.stdout.write(out.decode(\\\"utf-8\\\", errors=\\\"replace\\\"))\\n'
)
print('GBN_CTL_B64=' + b64 + ' python3 -c ' + json.dumps(py))
" "$b64")"

    aws ecs execute-command \
      --cluster   "$CLUSTER" \
      --task      "$arn" \
      --container "$container" \
      --region    "$AWS_REGION" \
      --command "$ecs_cmd" \
      2>&1 | grep -v 'Session Manager plugin\|Starting session\|Exiting session\|installed successfully'

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
    \"s=socket.create_connection(('127.0.0.1',5050),5); \"
    \"s.sendall(p.encode()+b'\\\\n'); \"
    \"s.shutdown(1); print(s.recv(131072).decode(errors='replace')); s.close()\"
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

# ─────────────────────────── Commands ────────────────────────────────────────

do_dump_dht() {
  local scope; scope="$(_pick_scope "DumpDht")"
  _run_scope "$scope" '{"cmd":"DumpDht"}'
}

do_dump_metadata() {
  echo "" >&2
  read -r -p "Max entries to return (leave blank = all): " lim
  local json
  if [[ "$lim" =~ ^[1-9][0-9]*$ ]]; then
    json="{\"cmd\":\"DumpMetadata\",\"limit\":$lim}"
  else
    json='{"cmd":"DumpMetadata","limit":0}'
  fi
  local scope; scope="$(_pick_scope "DumpMetadata")"
  _run_scope "$scope" "$json"
}

do_broadcast_seed() {
  local scope; scope="$(_pick_scope "BroadcastSeed")"
  _run_scope "$scope" '{"cmd":"BroadcastSeed"}'
}

do_send_dummy() {
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
  _send_cmd "$creator_idx" "$payload"

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

  echo ""
  echo "====  DumpMetadata: circuit nodes in order  ===="
  echo ">>> [1/5] Creator";   _send_cmd "$creator_idx" "$dm_json"
  echo ">>> [2/5] Guard";     _send_cmd "$guard_idx"   "$dm_json"
  echo ">>> [3/5] Middle";    _send_cmd "$middle_idx"  "$dm_json"
  echo ">>> [4/5] Exit";      _send_cmd "$exit_idx"    "$dm_json"
  if (( pub_idx >= 0 )); then
    echo ">>> [5/5] Publisher"; _send_cmd "$pub_idx"   "$dm_json"
  else
    echo ">>> [5/5] Publisher - skipped (no EC2 descriptor found)"
  fi
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
    select CMD in "DumpDht" "DumpMetadata" "BroadcastSeed" "SendDummy" "Refresh nodes" "Exit"; do
      case "$CMD" in
        DumpDht)           do_dump_dht ;;
        DumpMetadata)      do_dump_metadata ;;
        BroadcastSeed)     do_broadcast_seed ;;
        SendDummy)         do_send_dummy ;;
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
