# GBN-PROTO-008 - Execution Phase 4 Detailed Plan: Local Kubernetes Operator Script

**Status:** Implemented - variant of GBN-PROTO-007 Phase 5
**Primary Goal:** ship a sibling operator script
`prototype/gbn-bridge-proto/infra/scripts/k8s-control-interactive.sh` that mirrors the V1
operator panel using `kubectl exec` instead of `aws ecs execute-command`. Same menu items
(Status / DumpBridges / DumpFrames / SendDummy / TriggerCommand / etc.). Replaces
CloudWatch metric reads with Grafana / Prometheus URL display, and replaces
`aws logs filter-log-events` with Loki queries via Grafana's Explore deep-link.
**Source Plan:** [GBN-PROTO-008 Execution Plan](GBN-PROTO-008-Local-Kubernetes-Test-Infrastructure-Execution-Plan.md)
**AWS Sibling Plan:** [GBN-PROTO-007 Phase 5](GBN-PROTO-007-Execution-Phase5-Interactive-Control-Script-Port.md)

---

## 1. Current Repo Findings

| Item | Current Value | Why It Matters |
|---|---|---|
| Phase 1 cluster | `veritas` namespace with 5 pods | script discovers pods via `kubectl -n veritas get pods` |
| Phase 2 observability stack | `observability` namespace | script knows the Grafana URL `http://localhost:30030` |
| Phase 3 metrics | `/metrics` on each pod | scraped automatically; script doesn't need to call them directly |
| AWS sibling script | not yet implemented | this phase is independent — could land first |
| Tempo OTLP endpoint | `http://tempo.observability.svc.cluster.local:4317` | not directly queried by script; operator uses Grafana Explore |
| Loki HTTP API | `http://loki.observability.svc.cluster.local:3100` | optional direct query for chain_id trace collection |

---

## 2. Review Summary

| Gap | Why It Matters | Resolution For Phase 4 |
|---|---|---|
| No operator panel for k8s | operator has to type `kubectl` commands one by one | port the GBN-PROTO-007 Phase 5 design with kubectl substitutions |
| Trace collection differs from AWS | no `aws logs filter-log-events`; uses Loki | call Loki HTTP API directly for chain_id queries, or open Grafana Explore URL |
| LiveMetrics differs from AWS | no `aws cloudwatch get-metric-statistics`; uses Grafana | print `kubectl port-forward` instructions and the Grafana URL |
| ECS exec retry logic doesn't apply | `kubectl exec` has its own failure modes | replace `_ecs_execute_command_retry` with `_kubectl_exec_retry` |

---

## 3. Scope Lock

### In Scope

- new file `prototype/gbn-bridge-proto/infra/scripts/k8s-control-interactive.sh` (~500
  lines) modeled after the GBN-PROTO-007 Phase 5 design
- node discovery via `kubectl get pods -l veritas-role`
- 14 menu actions analogous to the AWS sibling
- `_kubectl_exec_retry` helper paralleling lattice's `_ecs_execute_command_retry`
- `_curl_admin` helper that wraps `kubectl exec`
- LiveMetrics action prints Grafana URL + `kubectl port-forward` example
- SendDummy action calls Phase 4-of-007 admin endpoint, then prints the chain_id and a
  pre-built Grafana Explore URL with the chain_id template variable populated, so the
  operator clicks once to see the trace
- TailLogs action runs `kubectl logs -f` with optional pod selector
- Refresh + Teardown (calling `k8s-down.sh`) + Exit
- README section in [infra/README-infra.md](../../../prototype/gbn-bridge-proto/infra/README-infra.md)

### Out Of Scope

- HTML report generation for SendDummy
- LiveMetrics in-terminal rendering (Grafana is much better; operator is told to open it)
- Alerting / pager integration
- Multi-cluster operator support
- Modifying the AWS sibling script

---

## 4. Preflight Gates

1. GBN-PROTO-008 Phase 1 + Phase 2 are landed; cluster + observability are up.
2. GBN-PROTO-007 Phase 1, 2, 4 are landed (admin endpoints exist on all pods).
3. GBN-PROTO-008 Phase 3 is landed (Prometheus metrics + Tempo spans flowing).
4. `kubectl` is on the operator's PATH.

---

## 5. File-by-File Specification

### 5.1 New file: `prototype/gbn-bridge-proto/infra/scripts/k8s-control-interactive.sh`

```bash
#!/usr/bin/env bash
# k8s-control-interactive.sh — Local Conduit V2 operator control panel (Kubernetes).
#
# Adapted from prototype/gbn-proto/infra/scripts/relay-control-interactive.sh.
# All admin actions reach 127.0.0.1:9090 inside each pod via kubectl exec + curl.

set -euo pipefail

NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
OBS_NS="${VERITAS_OBS_NAMESPACE:-observability}"
GRAFANA_URL="${VERITAS_GRAFANA_URL:-http://localhost:30030}"
ADMIN_PORT="9090"

# TTY save/restore — same approach as V1 lines 27-45.
TTY_STATE=""
if [ -t 0 ]; then
  TTY_STATE="$(stty -g 2>/dev/null || true)"
  stty sane 2>/dev/null || true
fi
restore_tty() {
  if [ -t 0 ] && [[ -n "${TTY_STATE:-}" ]]; then
    stty "$TTY_STATE" 2>/dev/null || true
  fi
}
trap restore_tty EXIT INT TERM

for dep in kubectl python3; do
  command -v "$dep" >/dev/null 2>&1 || { echo "ERROR: '$dep' not found." >&2; exit 1; }
done

# --- Node registry ---
NODE_LABELS=()  NODE_DESCS=()  NODE_IPS=()  NODE_ROLES=()

_kubectl_exec_retry() {
  local pod="$1" container="$2" cmd="$3"
  local attempt max_attempts=3 rc raw
  for (( attempt=1; attempt<=max_attempts; attempt++ )); do
    set +e
    raw="$(kubectl -n "$NAMESPACE" exec -i "$pod" -c "$container" -- sh -c "$cmd" 2>&1)"
    rc=$?
    set -e
    if [[ $rc -eq 0 ]]; then printf '%s' "$raw"; return 0; fi
    if (( attempt < max_attempts )); then sleep $((attempt * 2)); continue; fi
    printf '%s' "$raw" >&2; return $rc
  done
}

_curl_admin() {
  local idx="$1" method="$2" path="$3" body="${4:-}"
  local desc="${NODE_DESCS[$idx]}"
  local pod="${desc%%:*}" rest="${desc#*:}"
  local container="${rest%%:*}"
  local curl_cmd
  if [[ -n "$body" ]]; then
    curl_cmd="curl -sS -X $method -H 'Content-Type: application/json' http://127.0.0.1:${ADMIN_PORT}${path} -d '${body}'"
  else
    curl_cmd="curl -sS -X $method http://127.0.0.1:${ADMIN_PORT}${path}"
  fi
  _kubectl_exec_retry "$pod" "$container" "$curl_cmd"
}

_pretty_json() { python3 -m json.tool 2>/dev/null || cat; }

# --- Discovery ---
discover_all_nodes() {
  echo "Discovering pods in namespace '$NAMESPACE'..." >&2

  local rows
  rows="$(kubectl -n "$NAMESPACE" get pods \
    -l 'veritas-role in (authority,receiver,bridge)' \
    -o json | python3 -c '
import json, sys
data = json.load(sys.stdin)
for item in data.get("items", []):
    if item.get("status", {}).get("phase") != "Running":
        continue
    name = item["metadata"]["name"]
    labels = item["metadata"].get("labels", {})
    role = labels.get("veritas-role", "unknown").upper()
    container = item["spec"]["containers"][0]["name"]
    ip = item.get("status", {}).get("podIP", "")
    print(f"{name}\t{container}\t{ip}\t{role}")
')"

  while IFS=$'\t' read -r pod container ip role; do
    [[ -z "$pod" ]] && continue
    NODE_LABELS+=("$pod  [$role / $container / $ip]")
    NODE_DESCS+=("$pod:$container")
    NODE_IPS+=("$ip")
    NODE_ROLES+=("$role")
  done <<< "$rows"

  echo "  Found ${#NODE_LABELS[@]} live pod(s)." >&2
  if [[ "${#NODE_LABELS[@]}" -eq 0 ]]; then
    echo "ERROR: no Conduit pods discovered in namespace '$NAMESPACE'." >&2
    exit 1
  fi
}

print_node_table() {
  echo ""
  printf "  %-4s  %-10s  %s\n" "IDX" "ROLE" "POD"
  printf "  %-4s  %-10s  %s\n" "----" "----------" "------------------------------------------"
  local i
  for (( i=0; i<${#NODE_LABELS[@]}; i++ )); do
    printf "  [%2d]  %-10s  %s\n" "$((i+1))" "${NODE_ROLES[$i]}" "${NODE_LABELS[$i]}"
  done
  echo ""
}

# Pickers — same shape as V1 lines 227-303, reading from NODE_* arrays.
_pick_node() {
  local prompt="$1" role_filter="${2:-}"
  local -a p_idxs=() p_labels=()
  local i
  for (( i=0; i<${#NODE_LABELS[@]}; i++ )); do
    [[ -z "$role_filter" || "${NODE_ROLES[$i]}" == "$role_filter" ]] || continue
    p_idxs+=("$i"); p_labels+=("${NODE_LABELS[$i]}  (${NODE_ROLES[$i]})")
  done
  [[ "${#p_idxs[@]}" -eq 0 ]] && { echo "  no nodes for role $role_filter" >&2; return 1; }
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
    echo "  Invalid." >&2
  done
}

# --- Action handlers ---
do_status() {
  kubectl -n "$NAMESPACE" get pods,svc,statefulset,deployment
  echo ""
  echo "Observability:"
  kubectl -n "$OBS_NS" get pods 2>/dev/null || echo "  (observability namespace not present)"
}

do_describe_pod() {
  local idx; idx="$(_pick_node "Pick a pod to describe:")"
  local pod="${NODE_DESCS[$idx]%%:*}"
  kubectl -n "$NAMESPACE" describe pod "$pod"
}

do_tail_logs() {
  local idx; idx="$(_pick_node "Pick a pod to tail logs from:")"
  local pod="${NODE_DESCS[$idx]%%:*}"
  local container="${NODE_DESCS[$idx]##*:}"
  echo "Tailing $pod / $container — Ctrl-C to stop." >&2
  kubectl -n "$NAMESPACE" logs -f "$pod" -c "$container"
}

do_exec_shell() {
  local idx; idx="$(_pick_node "Pick a pod to shell into:")"
  local pod="${NODE_DESCS[$idx]%%:*}"
  local container="${NODE_DESCS[$idx]##*:}"
  kubectl -n "$NAMESPACE" exec -it "$pod" -c "$container" -- sh
  restore_tty
}

do_show_catalog() {
  local idx; idx="$(_pick_node "Pick AUTHORITY pod:" "AUTHORITY")"
  local desc="${NODE_DESCS[$idx]}"
  local pod="${desc%%:*}" container="${desc##*:}"
  _kubectl_exec_retry "$pod" "$container" \
    "curl -sS http://127.0.0.1:8080/v1/creator/catalog" | _pretty_json
}

do_dump_bridges() {
  local idx; idx="$(_pick_node "Pick AUTHORITY pod:" "AUTHORITY")"
  _curl_admin "$idx" GET /v1/admin/bridges | _pretty_json
}

do_dump_frames() {
  echo "" >&2
  read -r -p "  Filter by chain_id (blank = all): " cid
  read -r -p "  Limit (blank = default): " lim
  local query="?"
  [[ -n "$cid" ]] && query+="chain_id=${cid}&"
  [[ -n "$lim" ]] && query+="limit=${lim}&"
  query="${query%?}"; query="${query%&}"
  local idx; idx="$(_pick_node "Pick AUTHORITY pod:" "AUTHORITY")"
  _curl_admin "$idx" GET "/v1/admin/frames${query}" | _pretty_json
}

do_admin_metrics() {
  local idx; idx="$(_pick_node "Pick a pod:")"
  _curl_admin "$idx" GET /v1/admin/metrics | _pretty_json
}

do_live_metrics() {
  echo ""
  echo "Local LiveMetrics is served by Grafana (Phase 2 stack)."
  echo ""
  echo "  Grafana:    $GRAFANA_URL  (admin/admin)"
  echo "  Conduit overview dashboard UID: conduit-overview"
  echo ""
  echo "  If port 30030 is not reachable, port-forward manually:"
  echo "    kubectl -n $OBS_NS port-forward svc/kube-prom-grafana 3000:80"
  echo ""
  echo "  Direct Prometheus UI (read-only):"
  echo "    kubectl -n $OBS_NS port-forward svc/kube-prom-kube-prome-prometheus 9090:9090"
  echo ""
  read -r -p "Open dashboard URL in default browser? [Y/n]: " yn
  if [[ "${yn,,}" != "n" ]]; then
    if command -v xdg-open >/dev/null; then
      xdg-open "$GRAFANA_URL/d/conduit-overview" >/dev/null 2>&1 &
    elif command -v wslview >/dev/null; then
      wslview "$GRAFANA_URL/d/conduit-overview"
    else
      echo "  No browser opener available; copy the URL manually."
    fi
  fi
}

do_send_dummy() {
  local idx; idx="$(_pick_node "Pick pod to act as creator:")"
  read -r -p "Frame size in bytes [512]: " size
  size="${size:-512}"
  echo "Triggering /v1/admin/send-dummy on ${NODE_LABELS[$idx]} ..." >&2
  local result
  result="$(_curl_admin "$idx" POST /v1/admin/send-dummy "{\"size\":${size}}")"
  printf '%s\n' "$result" | _pretty_json
  local chain_id
  chain_id="$(printf '%s' "$result" | python3 -c "import sys,json; print(json.loads(sys.stdin.read()).get('chain_id',''))" 2>/dev/null || true)"
  [[ -z "$chain_id" ]] && return 1
  echo ""
  echo "  Root chain_id: $chain_id"
  echo ""
  echo "  Tempo trace search:"
  echo "    $GRAFANA_URL/explore?left=%7B%22datasource%22:%22Tempo%22,%22queries%22:%5B%7B%22query%22:%22${chain_id}%22%7D%5D%7D"
  echo ""
  echo "  Loki log search:"
  echo "    $GRAFANA_URL/explore?left=%7B%22datasource%22:%22Loki%22,%22queries%22:%5B%7B%22expr%22:%22%7Bnamespace%3D%5C%22${NAMESPACE}%5C%22%7D%20%7C%3D%20%5C%22${chain_id}%5C%22%22%7D%5D%7D"
  echo ""
  read -r -p "Collect chain_id traces from each pod's logs now? [y/N]: " yn
  [[ "${yn,,}" == "y" ]] && _collect_chain_traces "$chain_id"
}

_collect_chain_traces() {
  local chain_id="$1"
  local i
  for (( i=0; i<${#NODE_LABELS[@]}; i++ )); do
    local pod="${NODE_DESCS[$i]%%:*}"
    local container="${NODE_DESCS[$i]##*:}"
    echo ""
    echo "=== ${NODE_ROLES[$i]} / $pod ==="
    kubectl -n "$NAMESPACE" logs --since=10m "$pod" -c "$container" 2>/dev/null | grep "$chain_id" | head -50 || true
  done
}

do_trigger_command() {
  local idx; idx="$(_pick_node "Pick AUTHORITY pod:" "AUTHORITY")"
  echo ""
  echo "  [1] CatalogRefresh"
  echo "  [2] SeedAssign"
  echo "  [3] Revoke"
  read -r -p "  Command [1-3]: " cmd_choice
  read -r -p "  Target bridge_id: " target
  local payload
  case "$cmd_choice" in
    1) payload='{"payload":{"CatalogRefresh":{}}}' ;;
    2) payload='{"payload":{"SeedAssign":{}}}' ;;
    3) payload='{"payload":{"Revoke":{}}}' ;;
    *) echo "Invalid"; return 1 ;;
  esac
  _curl_admin "$idx" POST "/v1/admin/bridges/${target}/command" "$payload" | _pretty_json
}

do_check_images() {
  echo ""
  printf "  %-30s  %-25s  %s\n" "POD" "ROLE" "IMAGE"
  printf "  %-30s  %-25s  %s\n" "------------------------------" "-------------------------" "----------------------------"
  local i
  for (( i=0; i<${#NODE_LABELS[@]}; i++ )); do
    local pod="${NODE_DESCS[$i]%%:*}"
    local image
    image="$(kubectl -n "$NAMESPACE" get pod "$pod" -o jsonpath='{.spec.containers[0].image}' 2>/dev/null || echo unknown)"
    printf "  %-30s  %-25s  %s\n" "$pod" "${NODE_ROLES[$i]}" "$image"
  done
  echo ""
  echo "  (Local cluster: images are typically tagged ':dev' from k3d image import.)"
  echo "  To refresh: bash $SCRIPT_DIR/k8s-up.sh"
}

do_refresh() {
  NODE_LABELS=(); NODE_DESCS=(); NODE_IPS=(); NODE_ROLES=()
  discover_all_nodes
  print_node_table
}

do_teardown() {
  read -r -p "Type the namespace name to confirm cluster teardown: " confirm
  if [[ "$confirm" == "$NAMESPACE" ]]; then
    "$(dirname "${BASH_SOURCE[0]}")/k8s-down.sh"
  else
    echo "confirmation mismatch; not tearing down"
  fi
}

# --- Main loop ---
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

main() {
  echo "Veritas Conduit V2 Local Operator Control Panel (Kubernetes)"
  echo "  Namespace:  $NAMESPACE"
  echo "  Observability: $OBS_NS"
  echo "  Grafana:    $GRAFANA_URL"
  echo ""
  discover_all_nodes
  print_node_table

  while true; do
    echo "Action:"
    select CMD in \
      "Status" "DescribePod" "TailLogs" "ExecShell" "ShowCatalog" \
      "DumpBridges" "DumpFrames" "AdminMetrics" "LiveMetrics" \
      "SendDummy" "TriggerCommand" "CheckImages" "Refresh" "Teardown" "Exit"; do
      case "$CMD" in
        Status)          do_status ;;
        DescribePod)     do_describe_pod ;;
        TailLogs)        do_tail_logs ;;
        ExecShell)       do_exec_shell ;;
        ShowCatalog)     do_show_catalog ;;
        DumpBridges)     do_dump_bridges ;;
        DumpFrames)      do_dump_frames ;;
        AdminMetrics)    do_admin_metrics ;;
        LiveMetrics)     do_live_metrics ;;
        SendDummy)       do_send_dummy ;;
        TriggerCommand)  do_trigger_command ;;
        CheckImages)     do_check_images ;;
        Refresh)         do_refresh ;;
        Teardown)        do_teardown; exit 0 ;;
        Exit)            exit 0 ;;
      esac
      break
    done
    echo ""
  done
}

main "$@"
```

### 5.2 Modify: `prototype/gbn-bridge-proto/infra/README-infra.md`

Append a section "Local Kubernetes Operator Panel" mirroring the AWS section's tone:

```markdown
### Local Kubernetes Operator Panel

After running `bash infra/scripts/k8s-up.sh` and
`bash infra/scripts/k8s-observability-up.sh`, drive the running cluster from a single
menu-driven script:

```bash
bash prototype/gbn-bridge-proto/infra/scripts/k8s-control-interactive.sh
```

Override defaults via `VERITAS_K8S_NAMESPACE`, `VERITAS_OBS_NAMESPACE`,
`VERITAS_GRAFANA_URL`. The script discovers all running Conduit pods (Authority,
Receiver, all Bridges) and presents a numbered menu. Every admin call goes via
`kubectl exec -- curl http://127.0.0.1:9090/...`.

Menu items:
- Status / DescribePod / TailLogs / ExecShell / ShowCatalog — read-only diagnostics.
- DumpBridges / DumpFrames / AdminMetrics — Phase 1 admin endpoints.
- LiveMetrics — opens Grafana with the Conduit overview dashboard.
- SendDummy — pick any pod to act as creator; prints chain_id and pre-built Grafana
  Tempo + Loki search URLs so the operator can click through to the trace.
- TriggerCommand — push a `BridgeCommandPayload` into a bridge's WebSocket stream.
- CheckImages — list each pod's running image tag.
- Teardown — runs `k8s-down.sh`.
```

---

## 6. Module And Asset Ownership Locked In Phase 4

| Asset | Responsibility |
|---|---|
| `infra/scripts/k8s-control-interactive.sh` | the local operator panel |
| `infra/scripts/k8s-up.sh` | cluster + topology bring-up (Phase 1) |
| `infra/scripts/k8s-observability-up.sh` | observability bring-up (Phase 2) |
| `infra/scripts/k8s-down.sh` | cluster tear-down (Phase 1) |
| `infra/scripts/k8s-observability-down.sh` | observability tear-down (Phase 2) |
| AWS sibling: `relay-control-interactive-v2.sh` | unchanged; serves the AWS deployment target |

The two control scripts coexist. The local one is for inner-loop dev; the AWS one is for
EKS / ECS deployments.

---

## 7. Implementation Notes

- Added `prototype/gbn-bridge-proto/infra/scripts/k8s-control-interactive.sh`.
- The script discovers running Conduit pods by `veritas-role`, keeps the admin listener
  private by using `kubectl exec -- curl http://127.0.0.1:9090/...`, and preserves TTY
  state around interactive shells.
- Menu actions include status, pod description, log tailing, shell exec, catalog/bridge
  dumps, frame dumps, admin metrics, Grafana/Prometheus links, SendDummy, bridge command
  injection, image inspection, smoke validation, refresh, teardown, and exit.
- SendDummy prints the returned `chain_id`, assigned bridge id, Grafana Tempo and Loki
  Explore deep links, and can grep recent pod logs for the same chain id.
- Updated `infra/README-infra.md` and the GBN-PROTO-008 tracker to mark Phase 4 complete.

## 8. Validation

Completed static/local validation in the current Windows-hosted shell:

1. `bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-control-interactive.sh` passed.
2. `git diff --check` passed with only Windows LF/CRLF warnings.
3. V1 protected-path diff was clean.
4. `shellcheck` was not available in this shell, so shellcheck validation remains deferred.

Deferred live k8s validation because this PowerShell environment does not have `kubectl`
on PATH. Run the live checks below from the WSL2 shell after `k8s-up.sh`,
`k8s-observability-up.sh`, and Phase 3 image redeployment complete.

1. Cluster + observability + Phase 3 metrics emission are live (run `k8s-up.sh` then
   `k8s-observability-up.sh`).
2. `shellcheck infra/scripts/k8s-control-interactive.sh` passes with no errors when
   `shellcheck` is available on the WSL2 host.
3. Run the script. Walk every menu item:
   - **Status** prints 5 Conduit pods + observability pods.
   - **TailLogs** streams logs from chosen pod.
   - **ExecShell** drops the operator into `/bin/sh` inside the pod; exiting returns to
     the menu without TTY corruption.
   - **DumpBridges / DumpFrames / AdminMetrics** return JSON.
   - **LiveMetrics** prints the Grafana URL and offers to open it (Ubuntu/WSL via
     `wslview`).
   - **SendDummy** picks a pod, returns chain_id, prints Tempo + Loki deep-link URLs,
     and (on `y`) shows `kubectl logs | grep <chain_id>` from each pod with hits in the
     creator + bridge + receiver pods.
   - **TriggerCommand** with `CatalogRefresh` against a bridge id from `DumpBridges`
     succeeds; the bridge pod's logs show the command arriving.
   - **CheckImages** lists `:dev` for all pods.
   - **Refresh** re-runs discovery cleanly.
   - **Teardown** prompts for namespace name and tears down the cluster.
4. Shellcheck: zero errors when run on the WSL2 host.
5. After teardown, re-running the script prints "no Conduit pods discovered" and exits
   non-zero without panicking.

---

## 9. Open Questions Carried Into Implementation

1. **Browser opener on WSL2** — `wslview` (from `wslu`) is the WSL-native opener; check
   if installed and document the apt install in README if not.
2. **Grafana NodePort vs port-forward** — Phase 2 sets NodePort 30030. If port conflicts
   with the operator's other tooling, switch to port-forward as the documented default.
3. **Loki direct HTTP query vs Grafana deep-link** — current spec uses Grafana deep-links
   for trace collection. Direct HTTP query is doable too; defer until Grafana proves
   noisy.
4. **Multiple clusters** — script assumes a single context. If the operator switches
   `kubectl` context away mid-session, behavior is undefined. Recommendation: print the
   current context at startup and warn if it doesn't look like k3d-veritas.
5. **Send-dummy pod-to-pod assignment quirks** — same circular-bridge concern as the AWS
   sibling (GBN-PROTO-007 Phase 4 §2). Same resolution: accept the collapse.
