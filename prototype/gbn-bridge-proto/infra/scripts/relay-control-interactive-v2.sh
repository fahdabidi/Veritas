#!/usr/bin/env bash
# relay-control-interactive-v2.sh - Conduit V2 operator control panel.
#
# Adapted from prototype/gbn-proto/infra/scripts/relay-control-interactive.sh.
# V2 is all Fargate; every admin action reaches 127.0.0.1:9090 inside a
# selected ECS task via ECS Exec and curl.

set -euo pipefail
export AWS_PAGER=""

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STACK_NAME="${GBN_BRIDGE_STACK_NAME:-gbn-conduit-full-dev}"
AWS_REGION="${GBN_BRIDGE_AWS_REGION:-${AWS_REGION:-us-east-1}}"
ADMIN_PORT="${GBN_BRIDGE_ADMIN_PORT:-9090}"
CW_NAMESPACE="Veritas/Conduit"
CLUSTER_NAME=""
METRICS_STACK_DIMENSION="${GBN_BRIDGE_METRICS_STACK_DIMENSION:-}"

# Preserve and restore TTY state because ECS Exec interactive sessions can leave
# local terminal erase/echo modes altered.
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

for dep in aws python3; do
  command -v "$dep" >/dev/null 2>&1 || {
    echo "ERROR: '$dep' not found in PATH." >&2
    exit 1
  }
done

cf_output() {
  local key="$1"
  local raw
  raw="$(aws cloudformation describe-stacks --stack-name "$STACK_NAME" --region "$AWS_REGION" --output json 2>/dev/null)" || true
  [ -z "$raw" ] && {
    echo ""
    return
  }
  printf '%s' "$raw" | python3 -c "import json,sys; d=json.load(sys.stdin); o=d['Stacks'][0].get('Outputs',[]); print(next((x['OutputValue'] for x in o if x.get('OutputKey')=='$key'), ''))"
}

cf_parameter() {
  local key="$1"
  local raw
  raw="$(aws cloudformation describe-stacks --stack-name "$STACK_NAME" --region "$AWS_REGION" --output json 2>/dev/null)" || true
  [ -z "$raw" ] && {
    echo ""
    return
  }
  printf '%s' "$raw" | python3 -c "import json,sys; d=json.load(sys.stdin); p=d['Stacks'][0].get('Parameters',[]); print(next((x['ParameterValue'] for x in p if x.get('ParameterKey')=='$key'), ''))"
}

metric_stack_dimension() {
  if [[ -z "$METRICS_STACK_DIMENSION" ]]; then
    METRICS_STACK_DIMENSION="$(cf_parameter EnvironmentName)"
    if [[ -z "$METRICS_STACK_DIMENSION" || "$METRICS_STACK_DIMENSION" == "None" ]]; then
      METRICS_STACK_DIMENSION="${STACK_NAME#gbn-conduit-full-}"
    fi
    [[ -z "$METRICS_STACK_DIMENSION" ]] && METRICS_STACK_DIMENSION="$STACK_NAME"
  fi
  printf '%s\n' "$METRICS_STACK_DIMENSION"
}

_shell_quote() {
  local value="$1"
  printf "'%s'" "$(printf '%s' "$value" | sed "s/'/'\\\\''/g")"
}

_ecs_execute_command_retry() {
  local arn="$1" container="$2" cmd="$3"
  local attempt max_attempts rc raw filtered
  max_attempts=3

  for ((attempt = 1; attempt <= max_attempts; attempt++)); do
    set +e
    if command -v timeout >/dev/null 2>&1; then
      raw="$(timeout --foreground 75 aws ecs execute-command \
        --cluster "$CLUSTER_NAME" \
        --task "$arn" \
        --container "$container" \
        --region "$AWS_REGION" \
        --interactive \
        --command "$cmd" \
        2>&1)"
    else
      raw="$(aws ecs execute-command \
        --cluster "$CLUSTER_NAME" \
        --task "$arn" \
        --container "$container" \
        --region "$AWS_REGION" \
        --interactive \
        --command "$cmd" \
        2>&1)"
    fi
    rc=$?
    set -e

    filtered="$(printf '%s\n' "$raw" | grep -v 'Session Manager plugin\|Starting session\|Exiting session\|installed successfully' || true)"
    [[ -n "$filtered" ]] && printf '%s\n' "$filtered"

    if [[ $rc -eq 0 ]]; then
      return 0
    fi

    if grep -q "Could not connect to the endpoint URL" <<<"$raw" && ((attempt < max_attempts)); then
      echo "  [WARN] ECS endpoint unreachable (attempt ${attempt}/${max_attempts}); retrying..." >&2
      sleep $((attempt * 2))
      continue
    fi

    if [[ $rc -eq 124 ]] && ((attempt < max_attempts)); then
      echo "  [WARN] ECS execute-command timed out (attempt ${attempt}/${max_attempts}); retrying..." >&2
      sleep $((attempt * 2))
      continue
    fi

    return "$rc"
  done

  return 1
}

_pretty_json() {
  local raw
  raw="$(cat)"
  if [[ -z "$raw" ]]; then
    return 0
  fi
  printf '%s' "$raw" | python3 -m json.tool 2>/dev/null || printf '%s\n' "$raw"
}

_curl_admin() {
  local idx="$1" method="$2" path="$3" body="${4:-}"
  local desc arn container target inner cmd
  desc="${NODE_DESCS[$idx]}"
  arn="${desc#ECS:}"
  arn="${arn%:*}"
  container="${desc##*:}"
  target="http://127.0.0.1:${ADMIN_PORT}${path}"
  if [[ -n "$body" ]]; then
    inner="curl -sS -X $method -H 'Content-Type: application/json' $(_shell_quote "$target") -d $(_shell_quote "$body")"
  else
    inner="curl -sS -X $method $(_shell_quote "$target")"
  fi
  cmd="sh -lc $(_shell_quote "$inner")"
  _ecs_execute_command_retry "$arn" "$container" "$cmd"
}

_now_epoch_ms() {
  python3 -c 'import time; print(int(time.time() * 1000))'
}

_iso_minutes_ago() {
  local minutes="$1"
  python3 - "$minutes" <<'PY'
from datetime import datetime, timedelta, timezone
import sys
minutes = int(sys.argv[1])
print((datetime.now(timezone.utc) - timedelta(minutes=minutes)).strftime("%Y-%m-%dT%H:%M:%SZ"))
PY
}

_iso_now() {
  python3 - <<'PY'
from datetime import datetime, timezone
print(datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"))
PY
}

# Parallel arrays - always indexed together.
NODE_LABELS=()
NODE_DESCS=()
NODE_IPS=()
NODE_ROLES=()

_discover_v2_service() {
  local role="$1" svc_output_key="$2" cluster_name="$3"
  local svc_name
  svc_name="$(cf_output "$svc_output_key")"
  [[ -z "$svc_name" || "$svc_name" == "None" ]] && return 0

  local -a arns
  mapfile -t arns < <(
    aws ecs list-tasks --cluster "$cluster_name" --service-name "$svc_name" \
      --desired-status RUNNING --region "$AWS_REGION" \
      --query 'taskArns[]' --output text 2>/dev/null |
      tr '\t' '\n' | grep -v '^$' | grep -v '^None$' || true
  )
  [[ "${#arns[@]}" -eq 0 ]] && return 0

  local -a rows
  mapfile -t rows < <(
    aws ecs describe-tasks --cluster "$cluster_name" --tasks "${arns[@]}" \
      --region "$AWS_REGION" \
      --query 'tasks[?lastStatus==`RUNNING`].[taskArn,attachments[].details[?name==`privateIPv4Address`].value|[0][0],containers[0].name]' \
      --output text 2>/dev/null | grep -v '^$' || true
  )

  local row arn ip container short
  for row in "${rows[@]}"; do
    arn="$(awk '{print $1}' <<<"$row")"
    ip="$(awk '{print $2}' <<<"$row")"
    container="$(awk '{print $3}' <<<"$row")"
    [[ -z "$ip" || "$ip" == "None" || -z "$arn" ]] && continue
    [[ -z "$container" || "$container" == "None" ]] && container="task"
    short="${arn##*/}"
    short="${short:0:8}"
    NODE_LABELS+=("$ip  [$role / $container / ECS ${short}...]")
    NODE_DESCS+=("ECS:$arn:$container")
    NODE_IPS+=("$ip")
    NODE_ROLES+=("$role")
  done
}

discover_all_nodes() {
  echo "Discovering live Conduit V2 tasks..." >&2
  CLUSTER_NAME="$(cf_output ClusterName)"
  if [[ -z "$CLUSTER_NAME" || "$CLUSTER_NAME" == "None" ]]; then
    echo "ERROR: CloudFormation output ClusterName not found for stack $STACK_NAME." >&2
    exit 1
  fi
  _discover_v2_service AUTHORITY AuthorityServiceName "$CLUSTER_NAME"
  _discover_v2_service RECEIVER ReceiverServiceName "$CLUSTER_NAME"
  _discover_v2_service BRIDGE BridgeServiceName "$CLUSTER_NAME"
  _discover_v2_service CREATOR CreatorHostServiceName "$CLUSTER_NAME"
  _discover_v2_service CREATOR CreatorNewServiceName "$CLUSTER_NAME"
  echo "  Found ${#NODE_LABELS[@]} live node(s)." >&2
  if [[ "${#NODE_LABELS[@]}" -eq 0 ]]; then
    echo "ERROR: no nodes discovered." >&2
    exit 1
  fi
}

print_node_table() {
  echo ""
  printf "  %-4s  %-10s  %s\n" "IDX" "ROLE" "NODE"
  printf "  %-4s  %-10s  %s\n" "----" "----------" "--------------------------------------------------------------"
  local i
  for ((i = 0; i < ${#NODE_LABELS[@]}; i++)); do
    printf "  [%2d]  %-10s  %s\n" "$((i + 1))" "${NODE_ROLES[$i]}" "${NODE_LABELS[$i]}"
  done
  echo ""
}

_pick_node() {
  local prompt="$1" role_filter="${2:-}"
  local -a p_idxs=() p_labels=()
  local i
  for ((i = 0; i < ${#NODE_LABELS[@]}; i++)); do
    [[ -z "$role_filter" || "${NODE_ROLES[$i]}" == "$role_filter" ]] || continue
    p_idxs+=("$i")
    p_labels+=("${NODE_LABELS[$i]}  (${NODE_ROLES[$i]})")
  done

  if [[ "${#p_idxs[@]}" -eq 0 ]]; then
    echo "  ERROR: no nodes available${role_filter:+ for role $role_filter}." >&2
    return 1
  fi

  echo "$prompt" >&2
  local j choice
  for ((j = 0; j < ${#p_labels[@]}; j++)); do
    printf "  [%d] %s\n" "$((j + 1))" "${p_labels[$j]}" >&2
  done

  while true; do
    read -r -p "  Select [1-${#p_labels[@]}]: " choice
    if [[ "$choice" =~ ^[0-9]+$ ]] && ((choice >= 1 && choice <= ${#p_labels[@]})); then
      echo "${p_idxs[$((choice - 1))]}"
      return 0
    fi
    echo "  Invalid selection." >&2
  done
}

_pick_log_group() {
  local auth_lg recv_lg bridge_lg creator_lg choice
  auth_lg="$(cf_output AuthorityLogGroup)"
  recv_lg="$(cf_output ReceiverLogGroup)"
  bridge_lg="$(cf_output BridgeLogGroup)"
  creator_lg="$(cf_output CreatorLogGroup)"
  echo "Pick log group:" >&2
  printf "  [1] %s\n" "$auth_lg" >&2
  printf "  [2] %s\n" "$recv_lg" >&2
  printf "  [3] %s\n" "$bridge_lg" >&2
  printf "  [4] %s\n" "$creator_lg" >&2
  while true; do
    read -r -p "  Select [1-4]: " choice
    case "$choice" in
      1) echo "$auth_lg"; return 0 ;;
      2) echo "$recv_lg"; return 0 ;;
      3) echo "$bridge_lg"; return 0 ;;
      4) echo "$creator_lg"; return 0 ;;
      *) echo "  Invalid selection." >&2 ;;
    esac
  done
}

_metric_names_for() {
  local service="$1"
  case "$service" in
    authority)
      printf '%s\n' \
        SuccessfulRegistrations RejectedRegistrations Heartbeats Revocations \
        IssuedCatalogs BootstrapRequests RejectedBootstrapRequests \
        BootstrapProgressReports IssuedBatches BatchRollovers
      ;;
    receiver)
      printf '%s\n' FramesAccepted FramesRejected BytesIngested SessionsOpened SessionsClosed
      ;;
    bridge)
      printf '%s\n' CommandsReceived CommandsAcked CommandsRejected FramesForwarded BytesForwarded ControlReconnects
      ;;
  esac
}

do_status() {
  "$SCRIPT_DIR/smoke-conduit-full.sh" --stack-name "$STACK_NAME" --region "$AWS_REGION"
}

do_outputs() {
  aws cloudformation describe-stacks --stack-name "$STACK_NAME" --region "$AWS_REGION" \
    --query 'Stacks[0].Outputs[].{Key:OutputKey,Value:OutputValue}' --output table
}

do_tail_logs() {
  local lg
  lg="$(_pick_log_group)"
  echo "Tailing $lg. Ctrl-C to stop." >&2
  aws logs tail "$lg" --follow --region "$AWS_REGION"
}

do_exec_shell() {
  local idx desc arn container
  idx="$(_pick_node "Pick node to shell into:")"
  desc="${NODE_DESCS[$idx]}"
  arn="${desc#ECS:}"
  arn="${arn%:*}"
  container="${desc##*:}"
  aws ecs execute-command --cluster "$CLUSTER_NAME" --task "$arn" \
    --container "$container" --interactive --command "/bin/sh" --region "$AWS_REGION"
  restore_tty
}

do_show_catalog() {
  local idx
  idx="$(_pick_node "Pick AUTHORITY node:" "AUTHORITY")"
  echo "Active authority bridge registry, used as the current V2 bridge catalog:"
  _curl_admin "$idx" GET /v1/admin/bridges | _pretty_json
}

do_dump_bridges() {
  local idx
  idx="$(_pick_node "Pick AUTHORITY node:" "AUTHORITY")"
  _curl_admin "$idx" GET /v1/admin/bridges | _pretty_json
}

do_dump_frames() {
  local cid lim query idx
  echo "" >&2
  read -r -p "  Filter by chain_id (blank = all): " cid
  read -r -p "  Limit (blank = default 1000): " lim
  query=""
  if [[ -n "$cid" || -n "$lim" ]]; then
    query="?"
    [[ -n "$cid" ]] && query+="chain_id=${cid}&"
    [[ -n "$lim" ]] && query+="limit=${lim}&"
    query="${query%&}"
  fi
  idx="$(_pick_node "Pick AUTHORITY node:" "AUTHORITY")"
  _curl_admin "$idx" GET "/v1/admin/frames${query}" | _pretty_json
}

do_admin_metrics() {
  local idx
  idx="$(_pick_node "Pick node:")"
  _curl_admin "$idx" GET /v1/admin/metrics | _pretty_json
}

do_live_metrics() {
  local interval iv service metric val start_time end_time stack_dim
  interval=30
  read -r -p "Refresh interval in seconds [30]: " iv
  [[ "$iv" =~ ^[1-9][0-9]*$ ]] && interval="$iv"
  stack_dim="$(metric_stack_dimension)"
  echo "  Polling every ${interval}s. Ctrl-C exits the panel." >&2
  while true; do
    clear
    start_time="$(_iso_minutes_ago 5)"
    end_time="$(_iso_now)"
    echo "Veritas Conduit - Live Metrics"
    echo "  Stack:  $STACK_NAME"
    echo "  Metric dimension Stack: $stack_dim"
    echo "  Region: $AWS_REGION"
    echo "  Window: $start_time to $end_time"
    echo ""
    for service in authority receiver bridge; do
      echo "  $service:"
      while IFS= read -r metric; do
        val="$(aws cloudwatch get-metric-statistics \
          --namespace "$CW_NAMESPACE" \
          --metric-name "$metric" \
          --dimensions Name=Service,Value="$service" Name=Stack,Value="$stack_dim" \
          --start-time "$start_time" \
          --end-time "$end_time" \
          --period 60 --statistics Sum --region "$AWS_REGION" \
          --query 'Datapoints | sort_by(@,&Timestamp)[-1].Sum' --output text 2>/dev/null || true)"
        [[ -z "$val" || "$val" == "None" ]] && val="-"
        printf "    %-32s %s\n" "$metric" "$val"
      done < <(_metric_names_for "$service")
      echo ""
    done
    sleep "$interval"
  done
}

do_send_dummy() {
  local idx size result chain_id assigned
  idx="$(_pick_node "Pick node to act as creator:")"
  read -r -p "Frame size in bytes [512]: " size
  size="${size:-512}"
  if ! [[ "$size" =~ ^[0-9]+$ ]]; then
    echo "ERROR: size must be a positive integer." >&2
    return 1
  fi
  echo "Triggering send_dummy on ${NODE_LABELS[$idx]}..." >&2
  result="$(_curl_admin "$idx" POST /v1/admin/send-dummy "{\"size\":${size}}")"
  printf '%s\n' "$result" | _pretty_json
  chain_id="$(printf '%s' "$result" | python3 -c "import json,sys; print(json.loads(sys.stdin.read()).get('chain_id',''))" 2>/dev/null || true)"
  assigned="$(printf '%s' "$result" | python3 -c "import json,sys; print(json.loads(sys.stdin.read()).get('assigned_bridge_id',''))" 2>/dev/null || true)"
  [[ -z "$chain_id" ]] && {
    echo "WARN: no chain_id in response." >&2
    return 1
  }
  echo ""
  echo "  Root chain_id:       $chain_id"
  echo "  Assigned bridge_id:  ${assigned:-unknown}"
  echo ""
  read -r -p "Collect traces from each Conduit log group? [Y/n]: " yn
  [[ "${yn,,}" == "n" ]] && return 0
  _collect_chain_traces "$chain_id"
}

_collect_chain_traces() {
  local chain_id="$1"
  local auth_lg recv_lg bridge_lg creator_lg lg now_ms start_ms
  auth_lg="$(cf_output AuthorityLogGroup)"
  recv_lg="$(cf_output ReceiverLogGroup)"
  bridge_lg="$(cf_output BridgeLogGroup)"
  creator_lg="$(cf_output CreatorLogGroup)"
  now_ms="$(_now_epoch_ms)"
  start_ms="$((now_ms - 300000))"
  for lg in "$auth_lg" "$recv_lg" "$bridge_lg" "$creator_lg"; do
    [[ -z "$lg" || "$lg" == "None" ]] && continue
    echo ""
    echo "=== $lg ==="
    aws logs filter-log-events \
      --log-group-name "$lg" \
      --filter-pattern "\"$chain_id\"" \
      --start-time "$start_ms" \
      --region "$AWS_REGION" \
      --query 'events[].[timestamp,message]' --output text 2>/dev/null |
      head -50 || true
  done
}

do_trigger_command() {
  local idx target choice payload now catalog_id lease_id reason body
  idx="$(_pick_node "Pick AUTHORITY node:" "AUTHORITY")"
  echo ""
  echo "  [1] CatalogRefresh test payload"
  echo "  [2] Revoke test payload"
  echo "  [3] Raw BridgeCommandPayload JSON"
  read -r -p "  Command [1-3]: " choice
  read -r -p "  Target bridge_id: " target
  [[ -z "$target" ]] && {
    echo "ERROR: target bridge_id is required." >&2
    return 1
  }
  now="$(_now_epoch_ms)"
  case "$choice" in
    1)
      catalog_id="admin-refresh-${target}-${now}"
      payload="{\"payload\":{\"command_type\":\"catalog_refresh\",\"body\":{\"catalog_id\":\"${catalog_id}\",\"issued_at_ms\":${now},\"expires_at_ms\":$((now + 300000)),\"bridges\":[],\"publisher_sig\":[]}}}"
      ;;
    2)
      read -r -p "  Lease id to include [admin-revoke-${target}]: " lease_id
      lease_id="${lease_id:-admin-revoke-${target}}"
      read -r -p "  Reason [operator_disabled]: " reason
      reason="${reason:-operator_disabled}"
      payload="{\"payload\":{\"command_type\":\"revoke\",\"body\":{\"lease_id\":\"${lease_id}\",\"bridge_id\":\"${target}\",\"revoked_at_ms\":${now},\"reason\":\"${reason}\",\"publisher_sig\":[]}}}"
      ;;
    3)
      echo "Paste BridgeCommandPayload JSON, for example:" >&2
      echo '  {"command_type":"catalog_refresh","body":{"catalog_id":"manual","issued_at_ms":1,"expires_at_ms":2,"bridges":[],"publisher_sig":[]}}' >&2
      read -r -p "  payload JSON: " body
      [[ -z "$body" ]] && {
        echo "ERROR: payload JSON is required." >&2
        return 1
      }
      payload="{\"payload\":${body}}"
      ;;
    *)
      echo "Invalid command." >&2
      return 1
      ;;
  esac
  _curl_admin "$idx" POST "/v1/admin/bridges/${target}/command" "$payload" | _pretty_json
}

do_check_images() {
  local i desc arn container row image digest repo_tag repo latest_digest status
  echo ""
  printf "  %-10s  %-32s  %-12s  %s\n" "ROLE" "CONTAINER" "STATUS" "IMAGE"
  printf "  %-10s  %-32s  %-12s  %s\n" "----------" "--------------------------------" "------------" "------------------------------------------------------------"
  for ((i = 0; i < ${#NODE_DESCS[@]}; i++)); do
    desc="${NODE_DESCS[$i]}"
    arn="${desc#ECS:}"
    arn="${arn%:*}"
    container="${desc##*:}"
    row="$(aws ecs describe-tasks --cluster "$CLUSTER_NAME" --tasks "$arn" --region "$AWS_REGION" \
      --query "tasks[0].containers[?name==\`${container}\`].[image,imageDigest]|[0]" --output text 2>/dev/null || true)"
    image="$(awk '{print $1}' <<<"$row")"
    digest="$(awk '{print $2}' <<<"$row")"
    status="unknown"
    if [[ "$image" == *".dkr.ecr."*".amazonaws.com/"* ]]; then
      repo_tag="${image#*.amazonaws.com/}"
      repo="${repo_tag%%[:@]*}"
      latest_digest="$(aws ecr describe-images --repository-name "$repo" --image-ids imageTag=latest \
        --region "$AWS_REGION" --query 'imageDetails[0].imageDigest' --output text 2>/dev/null || true)"
      if [[ -n "$latest_digest" && "$latest_digest" != "None" && "$digest" == "$latest_digest" ]]; then
        status="up-to-date"
      elif [[ -n "$latest_digest" && "$latest_digest" != "None" ]]; then
        status="differs"
      fi
    fi
    printf "  %-10s  %-32s  %-12s  %s\n" "${NODE_ROLES[$i]}" "$container" "$status" "${image:-unknown}"
  done
}

do_bootstrap_smoke() {
  "$SCRIPT_DIR/bootstrap-smoke.sh" --stack-name "$STACK_NAME" --region "$AWS_REGION"
}

do_refresh() {
  NODE_LABELS=()
  NODE_DESCS=()
  NODE_IPS=()
  NODE_ROLES=()
  discover_all_nodes
  print_node_table
}

do_teardown() {
  local confirm
  read -r -p "Type the stack name to confirm deletion: " confirm
  if [[ "$confirm" == "$STACK_NAME" ]]; then
    "$SCRIPT_DIR/teardown-conduit-full.sh" --stack-name "$STACK_NAME" --region "$AWS_REGION"
  else
    echo "confirmation mismatch; not deleting"
  fi
}

source "$SCRIPT_DIR/_seed_actions.sh"

main() {
  echo "Veritas Conduit V2 Operator Control Panel"
  echo "  Stack:  $STACK_NAME"
  echo "  Region: $AWS_REGION"
  echo ""
  discover_all_nodes
  print_node_table

  while true; do
    echo "Action:"
    select CMD in \
      "Status" \
      "StackOutputs" \
      "TailLogs" \
      "ExecShell" \
      "ShowCatalog" \
      "DumpBridges" \
      "DumpFrames" \
      "AdminMetrics" \
      "LiveMetrics" \
      "SeedHostCreator" \
      "SendDummy" \
      "TriggerCommand" \
      "CheckImages" \
      "BootstrapSmoke" \
      "Refresh" \
      "Teardown" \
      "Exit"; do
      case "$CMD" in
        Status) do_status ;;
        StackOutputs) do_outputs ;;
        TailLogs) do_tail_logs ;;
        ExecShell) do_exec_shell ;;
        ShowCatalog) do_show_catalog ;;
        DumpBridges) do_dump_bridges ;;
        DumpFrames) do_dump_frames ;;
        AdminMetrics) do_admin_metrics ;;
        LiveMetrics) do_live_metrics ;;
        SeedHostCreator) do_seed_host_creator ;;
        SendDummy) do_send_dummy ;;
        TriggerCommand) do_trigger_command ;;
        CheckImages) do_check_images ;;
        BootstrapSmoke) do_bootstrap_smoke ;;
        Refresh) do_refresh ;;
        Teardown)
          do_teardown
          exit 0
          ;;
        Exit) exit 0 ;;
        *) echo "Invalid action." >&2 ;;
      esac
      break
    done
    echo ""
  done
}

main "$@"
