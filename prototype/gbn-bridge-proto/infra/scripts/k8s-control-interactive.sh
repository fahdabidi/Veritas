#!/usr/bin/env bash
# k8s-control-interactive.sh - Conduit V2 local Kubernetes operator panel.
#
# All admin calls reach 127.0.0.1:9090 inside the selected pod through
# kubectl exec. This preserves the localhost-only admin listener while still
# giving an operator one menu for local development.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
OBS_NS="${VERITAS_OBS_NAMESPACE:-observability}"
GRAFANA_URL="${VERITAS_GRAFANA_URL:-http://localhost:30030}"
ADMIN_PORT="${VERITAS_K8S_ADMIN_PORT:-9090}"

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

for dep in kubectl python3; do
  command -v "$dep" >/dev/null 2>&1 || {
    echo "ERROR: '$dep' not found in PATH." >&2
    exit 1
  }
done

NODE_LABELS=()
NODE_DESCS=()
NODE_IPS=()
NODE_ROLES=()

_shell_quote() {
  local value="$1"
  printf "'%s'" "$(printf '%s' "$value" | sed "s/'/'\\\\''/g")"
}

_pretty_json() {
  local raw
  raw="$(cat)"
  if [[ -z "$raw" ]]; then
    return 0
  fi
  printf '%s' "$raw" | python3 -m json.tool 2>/dev/null || printf '%s\n' "$raw"
}

_now_epoch_ms() {
  python3 -c 'import time; print(int(time.time() * 1000))'
}

_json_field() {
  local field="$1"
  python3 -c "import json,sys; print(json.load(sys.stdin).get('$field',''))" 2>/dev/null || true
}

_urlencode() {
  python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.stdin.read().strip(), safe=""))'
}

_grafana_explore_url() {
  local datasource="$1" query_key="$2" query_value="$3"
  python3 - "$GRAFANA_URL" "$datasource" "$query_key" "$query_value" <<'PY'
import json
import sys
import urllib.parse

base, datasource, query_key, query_value = sys.argv[1:5]
uid = datasource.lower()
query = {
    "refId": "A",
    "datasource": {"type": uid, "uid": uid},
}
query[query_key] = query_value
state = {
    "datasource": uid,
    "queries": [query],
    "range": {"from": "now-30m", "to": "now"},
}
encoded = urllib.parse.quote(json.dumps(state, separators=(",", ":")), safe="")
print(base.rstrip("/") + "/explore?left=" + encoded)
PY
}

_kubectl_exec_retry() {
  local pod="$1" container="$2" inner="$3"
  local attempt max_attempts rc raw
  max_attempts=3

  for ((attempt = 1; attempt <= max_attempts; attempt++)); do
    set +e
    raw="$(kubectl -n "$NAMESPACE" exec "$pod" -c "$container" -- sh -lc "$inner" 2>&1)"
    rc=$?
    set -e

    if [[ $rc -eq 0 ]]; then
      printf '%s' "$raw"
      return 0
    fi

    if ((attempt < max_attempts)); then
      echo "  [WARN] kubectl exec failed (attempt ${attempt}/${max_attempts}); retrying..." >&2
      sleep $((attempt * 2))
      continue
    fi

    printf '%s\n' "$raw" >&2
    return "$rc"
  done

  return 1
}

_curl_admin() {
  local idx="$1" method="$2" path="$3" body="${4:-}"
  local desc pod container target inner
  desc="${NODE_DESCS[$idx]}"
  pod="${desc%%:*}"
  container="${desc##*:}"
  target="http://127.0.0.1:${ADMIN_PORT}${path}"

  if [[ -n "$body" ]]; then
    inner="curl -sS -X $method -H 'Content-Type: application/json' $(_shell_quote "$target") -d $(_shell_quote "$body")"
  else
    inner="curl -sS -X $method $(_shell_quote "$target")"
  fi

  _kubectl_exec_retry "$pod" "$container" "$inner"
}

discover_all_nodes() {
  echo "Discovering Conduit pods in namespace '$NAMESPACE'..." >&2

  local rows
  rows="$(kubectl -n "$NAMESPACE" get pods -l veritas-role -o json | python3 -c '
import json
import sys

data = json.load(sys.stdin)
for item in data.get("items", []):
    status = item.get("status", {})
    if status.get("phase") != "Running":
        continue
    labels = item.get("metadata", {}).get("labels", {})
    role = labels.get("veritas-role")
    if role not in {"authority", "receiver", "bridge", "creator"}:
        continue
    containers = item.get("spec", {}).get("containers", [])
    if not containers:
        continue
    name = item["metadata"]["name"]
    container = containers[0]["name"]
    ip = status.get("podIP", "")
    print(f"{name}\t{container}\t{ip}\t{role.upper()}")
')"

  while IFS=$'\t' read -r pod container ip role; do
    [[ -z "$pod" ]] && continue
    NODE_LABELS+=("$pod  [$role / $container / ${ip:-no-ip}]")
    NODE_DESCS+=("$pod:$container")
    NODE_IPS+=("$ip")
    NODE_ROLES+=("$role")
  done <<<"$rows"

  echo "  Found ${#NODE_LABELS[@]} live pod(s)." >&2
  if [[ "${#NODE_LABELS[@]}" -eq 0 ]]; then
    echo "ERROR: no running Conduit pods discovered in namespace '$NAMESPACE'." >&2
    exit 1
  fi
}

print_node_table() {
  echo ""
  printf "  %-4s  %-10s  %s\n" "IDX" "ROLE" "POD"
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
    echo "  ERROR: no pods available${role_filter:+ for role $role_filter}." >&2
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

do_status() {
  echo "Kubernetes context:"
  kubectl config current-context 2>/dev/null || true
  echo ""
  kubectl -n "$NAMESPACE" get pods,svc,statefulset,deployment -o wide
  echo ""
  echo "Observability:"
  kubectl -n "$OBS_NS" get pods,svc 2>/dev/null || echo "  namespace '$OBS_NS' is not present"
}

do_describe_pod() {
  local idx pod
  idx="$(_pick_node "Pick pod to describe:")"
  pod="${NODE_DESCS[$idx]%%:*}"
  kubectl -n "$NAMESPACE" describe pod "$pod"
}

do_tail_logs() {
  local idx pod container tail
  idx="$(_pick_node "Pick pod to tail logs from:")"
  pod="${NODE_DESCS[$idx]%%:*}"
  container="${NODE_DESCS[$idx]##*:}"
  read -r -p "Tail last N lines before following [100]: " tail
  tail="${tail:-100}"
  echo "Tailing $pod / $container. Ctrl-C returns to the shell." >&2
  kubectl -n "$NAMESPACE" logs -f --tail="$tail" "$pod" -c "$container"
}

do_exec_shell() {
  local idx pod container
  idx="$(_pick_node "Pick pod to shell into:")"
  pod="${NODE_DESCS[$idx]%%:*}"
  container="${NODE_DESCS[$idx]##*:}"
  kubectl -n "$NAMESPACE" exec -it "$pod" -c "$container" -- sh
  restore_tty
}

do_show_catalog() {
  local idx
  idx="$(_pick_node "Pick AUTHORITY pod:" "AUTHORITY")"
  echo "Active authority bridge registry, used as the current V2 bridge catalog:"
  _curl_admin "$idx" GET /v1/admin/bridges | _pretty_json
}

do_dump_bridges() {
  local idx
  idx="$(_pick_node "Pick AUTHORITY pod:" "AUTHORITY")"
  _curl_admin "$idx" GET /v1/admin/bridges | _pretty_json
}

do_dump_frames() {
  local cid lim query idx cid_encoded
  echo "" >&2
  read -r -p "  Filter by chain_id (blank = all): " cid
  read -r -p "  Limit (blank = default 1000): " lim
  query=""
  if [[ -n "$cid" || -n "$lim" ]]; then
    query="?"
    if [[ -n "$cid" ]]; then
      cid_encoded="$(printf '%s' "$cid" | _urlencode)"
      query+="chain_id=${cid_encoded}&"
    fi
    [[ -n "$lim" ]] && query+="limit=${lim}&"
    query="${query%&}"
  fi
  idx="$(_pick_node "Pick AUTHORITY pod:" "AUTHORITY")"
  _curl_admin "$idx" GET "/v1/admin/frames${query}" | _pretty_json
}

do_admin_metrics() {
  local idx
  idx="$(_pick_node "Pick pod:")"
  _curl_admin "$idx" GET /v1/admin/metrics | _pretty_json
}

do_live_metrics() {
  echo ""
  echo "Local LiveMetrics is served by Grafana and Prometheus."
  echo ""
  echo "  Grafana:    $GRAFANA_URL  (admin/admin)"
  echo "  Dashboard:  $GRAFANA_URL/d/conduit-overview"
  echo ""
  echo "  If the NodePort is not reachable, port-forward Grafana:"
  echo "    kubectl -n $OBS_NS port-forward svc/kube-prom-grafana 3000:80"
  echo "    then open http://localhost:3000/d/conduit-overview"
  echo ""
  echo "  Direct Prometheus UI:"
  echo "    kubectl -n $OBS_NS port-forward svc/kube-prom-prometheus 9090:9090"
  echo ""
  read -r -p "Open dashboard URL in default browser? [Y/n]: " yn
  if [[ "${yn,,}" != "n" ]]; then
    if command -v wslview >/dev/null 2>&1; then
      wslview "$GRAFANA_URL/d/conduit-overview" >/dev/null 2>&1 || true
    elif command -v xdg-open >/dev/null 2>&1; then
      xdg-open "$GRAFANA_URL/d/conduit-overview" >/dev/null 2>&1 &
    else
      echo "  No browser opener found; copy the URL above."
    fi
  fi
}

do_send_dummy() {
  local idx size result chain_id assigned tempo_url loki_expr loki_url
  idx="$(_pick_node "Pick pod to act as creator:")"
  read -r -p "Frame size in bytes [512]: " size
  size="${size:-512}"
  if ! [[ "$size" =~ ^[0-9]+$ ]] || ((size < 1)); then
    echo "ERROR: size must be a positive integer." >&2
    return 1
  fi

  echo "Triggering send_dummy on ${NODE_LABELS[$idx]}..." >&2
  result="$(_curl_admin "$idx" POST /v1/admin/send-dummy "{\"size\":${size}}")"
  printf '%s\n' "$result" | _pretty_json

  chain_id="$(printf '%s' "$result" | _json_field chain_id)"
  assigned="$(printf '%s' "$result" | _json_field assigned_bridge_id)"
  if [[ -z "$chain_id" ]]; then
    echo "WARN: no chain_id in response." >&2
    return 1
  fi

  loki_expr="{namespace=\"$NAMESPACE\"} |= \"$chain_id\""
  tempo_url="$(_grafana_explore_url Tempo query "$chain_id")"
  loki_url="$(_grafana_explore_url Loki expr "$loki_expr")"

  echo ""
  echo "  Root chain_id:       $chain_id"
  echo "  Assigned bridge_id:  ${assigned:-unknown}"
  echo ""
  echo "  Tempo trace search:"
  echo "    $tempo_url"
  echo ""
  echo "  Loki log search:"
  echo "    $loki_url"
  echo ""
  read -r -p "Collect chain_id hits from recent pod logs now? [Y/n]: " yn
  [[ "${yn,,}" == "n" ]] && return 0
  _collect_chain_traces "$chain_id"
}

_collect_chain_traces() {
  local chain_id="$1"
  local i pod container
  for ((i = 0; i < ${#NODE_LABELS[@]}; i++)); do
    pod="${NODE_DESCS[$i]%%:*}"
    container="${NODE_DESCS[$i]##*:}"
    echo ""
    echo "=== ${NODE_ROLES[$i]} / $pod / $container ==="
    kubectl -n "$NAMESPACE" logs --since=10m "$pod" -c "$container" 2>/dev/null |
      grep "$chain_id" | head -50 || true
  done
}

do_trigger_command() {
  local idx target choice payload now catalog_id lease_id reason body
  idx="$(_pick_node "Pick AUTHORITY pod:" "AUTHORITY")"
  echo ""
  echo "  [1] CatalogRefresh test payload"
  echo "  [2] Revoke test payload"
  echo "  [3] Raw BridgeCommandPayload JSON"
  read -r -p "  Command [1-3]: " choice
  read -r -p "  Target bridge_id: " target
  if [[ -z "$target" ]]; then
    echo "ERROR: target bridge_id is required." >&2
    return 1
  fi

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
      if [[ -z "$body" ]]; then
        echo "ERROR: payload JSON is required." >&2
        return 1
      fi
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
  local i pod image
  echo ""
  printf "  %-30s  %-10s  %s\n" "POD" "ROLE" "IMAGE"
  printf "  %-30s  %-10s  %s\n" "------------------------------" "----------" "------------------------------------------"
  for ((i = 0; i < ${#NODE_LABELS[@]}; i++)); do
    pod="${NODE_DESCS[$i]%%:*}"
    image="$(kubectl -n "$NAMESPACE" get pod "$pod" -o jsonpath='{.spec.containers[0].image}' 2>/dev/null || echo unknown)"
    printf "  %-30s  %-10s  %s\n" "$pod" "${NODE_ROLES[$i]}" "${image:-unknown}"
  done
  echo ""
  echo "  Local images should normally be versioned tags imported into k3d by k8s-up.sh."
  echo "  To rebuild and reload: bash $SCRIPT_DIR/k8s-up.sh"
}

do_smoke_validation() {
  "$SCRIPT_DIR/k8s-smoke.sh" --namespace "$NAMESPACE" --send-dummy
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
  read -r -p "Type the namespace name to confirm local cluster teardown: " confirm
  if [[ "$confirm" == "$NAMESPACE" ]]; then
    "$SCRIPT_DIR/k8s-down.sh"
  else
    echo "confirmation mismatch; not tearing down"
  fi
}

source "$SCRIPT_DIR/_seed_actions.sh"

main() {
  echo "Veritas Conduit V2 Local Operator Control Panel (Kubernetes)"
  echo "  Context:       $(kubectl config current-context 2>/dev/null || echo unknown)"
  echo "  Namespace:     $NAMESPACE"
  echo "  Observability: $OBS_NS"
  echo "  Grafana:       $GRAFANA_URL"
  echo ""
  discover_all_nodes
  print_node_table

  while true; do
    echo "Action:"
    select CMD in \
      "Status" \
      "DescribePod" \
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
      "SmokeValidation" \
      "Refresh" \
      "Teardown" \
      "Exit"; do
      case "$CMD" in
        Status) do_status ;;
        DescribePod) do_describe_pod ;;
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
        SmokeValidation) do_smoke_validation ;;
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
