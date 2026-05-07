# GBN-PROTO-007 - Execution Phase 5 Detailed Plan: Interactive Control Script Port

**Status:** Completed — implementation landed; live AWS/Kubernetes walk-through deferred until local infrastructure is available
**Primary Goal:** replace the 47-line
[relay-control-interactive-v2.sh](../../../prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh)
with a structurally adapted port of V1's 1,415-line
[relay-control-interactive.sh](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh),
covering every operator capability now backed by Phases 1–4. Drops the V1 EC2 / SSM branch
(V2 is all-Fargate). Adds a chain_id–aware tracing flow that walks the
originator-node + assigned-bridge + receiver log groups after every SendDummy.
**Source Plan:** [GBN-PROTO-007 Execution Plan](GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md)

---

## 1. Current Repo Findings

| Item | Current Value | Why It Matters |
|---|---|---|
| V2 script in place | [relay-control-interactive-v2.sh](../../../prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh) — ECS-only interactive operator panel | Phase 5 replaced the legacy 47-line stub in place per [GBN-PROTO-007 §4.4](GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md) |
| V1 reference script | [relay-control-interactive.sh](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh) — 1,415 lines | source of structure and patterns; do **not** modify |
| Existing companion scripts | `status-snapshot.sh`, `bootstrap-smoke.sh`, `teardown-conduit-full.sh`, `smoke-conduit-full.sh` already exist | the port wraps these, does not duplicate them |
| Phase 1 admin endpoints | available on every binary at `127.0.0.1:9090` | Phase 5 reaches them via `aws ecs execute-command --interactive --command "curl ..."` |
| Phase 2 command injection | available on authority binary | Phase 5's `TriggerCommand` uses it |
| Phase 3 CloudWatch metrics | namespace `Veritas/Conduit` populated by 60s emitter | Phase 5's `LiveMetrics` reads via `aws cloudwatch get-metric-statistics` |
| Phase 4 send-dummy | available on every binary | Phase 5's `SendDummy` calls it |
| chain_id propagation | already in V2 logs per GBN-PROTO-006 Phase 7 | `aws logs filter-log-events --filter-pattern '<chain_id>'` works against existing log groups |

---

## 2. Review Summary

| Gap | Why It Matters | Resolution For Phase 5 |
|---|---|---|
| Operator must drop into raw `aws` CLI for everything beyond status / smoke / teardown | high friction, error-prone, doesn't scale | port the V1 control panel structure |
| Lattice script has EC2 + SSM branch | not applicable to all-Fargate V2 | drop those code paths entirely |
| Lattice script reads `GBN/ScaleTest` CW namespace | V2 emits to `Veritas/Conduit` | swap the namespace constant |
| Lattice script generates HTML report from each SendDummy | useful but heavy; defer to follow-up | first cut emits plain-text trace; HTML report optional later |
| Lattice script's TCP-control-port (5050) JSON sends do not apply | V2 uses HTTP admin endpoints instead | replace `_send_cmd` with `_curl_admin` helper |

---

## 3. Scope Lock

### In Scope

- full rewrite of
  [relay-control-interactive-v2.sh](../../../prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh)
- 14 menu actions (see §5.2 for the final menu)
- node-discovery for the 3 ECS services (Authority, Receiver, Bridge), no EC2 path
- ECS-exec-based admin curl helpers for Phases 1, 2, 4 endpoints
- CloudWatch read for Phase 3 LiveMetrics
- `aws logs filter-log-events` for chain_id trace assembly after SendDummy
- TTY save/restore around interactive ECS exec sessions (copied from V1 lines 27-45)
- Refresh action that re-runs discovery
- Confirmation prompt on Teardown

### Out Of Scope

- HTML report generation for SendDummy (deferred; first cut is plain-text)
- Multi-stack management (single stack at a time, like V1)
- Web UI
- Modifying any companion script (`status-snapshot.sh`, `smoke-conduit-full.sh`, etc.)
- Modifying any V2 source crate
- Modifying any V1 file

---

## 4. Preflight Gates

1. Phases 1, 2, 3, 4 are all landed and validated.
2. The three updated container images are pushed to ECR.
3. A `gbn-conduit-full-dev` stack is deployable with the new images.
4. ECS exec is functional on every service (`EnableExecuteCommand: true` already in
   template per current state).
5. `curl` is present in every container (added in Phase 1 §5.8–5.10).
6. Operator AWS credentials have `ecs:ExecuteCommand`, `cloudwatch:GetMetricStatistics`,
   `logs:FilterLogEvents`, `cloudformation:DescribeStacks`, `ecs:DescribeServices`,
   `ecs:DescribeTasks` permissions.

---

## 5. File-by-File Specification

### 5.1 Replace: `prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh`

The current 47-line file is replaced wholesale. Estimated final size: 600–700 lines.

**Top-of-file structure (line ranges approximate):**

```bash
#!/usr/bin/env bash
# relay-control-interactive-v2.sh — Conduit V2 operator control panel.
#
# Adapted from prototype/gbn-proto/infra/scripts/relay-control-interactive.sh.
# All admin actions reach 127.0.0.1:9090 inside each ECS task via ECS exec + curl.
# See docs/prototyping/Conduit/Full-Implementation-Plan-Pass2/ for the design.

set -euo pipefail
export AWS_PAGER=""
```

**Globals and TTY restore (lines ~10-50):**
- copy verbatim from
  [V1 lines 27-45](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh#L27-L45)
- defaults:
  ```bash
  STACK_NAME="${GBN_BRIDGE_STACK_NAME:-gbn-conduit-full-dev}"
  AWS_REGION="${GBN_BRIDGE_AWS_REGION:-${AWS_REGION:-us-east-1}}"
  ADMIN_PORT="9090"
  CW_NAMESPACE="Veritas/Conduit"
  ```

**Helpers (lines ~50-200):**
- `cf_output(key)` — copy from
  [V1 lines 55-61](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh#L55-L61)
  (verbatim; same `aws cloudformation describe-stacks` query pattern).
- `_ecs_execute_command_retry(arn, container, cmd)` — copy from
  [V1 lines 63-115](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh#L63-L115)
  (verbatim; the same retry logic for `Could not connect to the endpoint URL` and
  timeouts is needed against ECS Exec on V2).
- `_curl_admin(node-index, method, path, [body])` — new helper.
  Wraps `_ecs_execute_command_retry` with a `curl` invocation:
  ```bash
  _curl_admin() {
    local idx="$1" method="$2" path="$3" body="${4:-}"
    local desc="${NODE_DESCS[$idx]}"
    local arn="${desc#ECS:}"; arn="${arn%:*}"
    local container="${desc##*:}"
    local curl_cmd
    if [[ -n "$body" ]]; then
      curl_cmd="curl -sS -X $method -H 'Content-Type: application/json' http://127.0.0.1:${ADMIN_PORT}${path} -d '$body'"
    else
      curl_cmd="curl -sS -X $method http://127.0.0.1:${ADMIN_PORT}${path}"
    fi
    _ecs_execute_command_retry "$arn" "$container" "$curl_cmd"
  }
  ```
- `_pretty_json(json)` — pipes through `python3 -m json.tool` for readable output.

**Node registry (lines ~200-260):**
- 4 parallel arrays, identical to V1 lines 117-122.
- Roles: `AUTHORITY`, `RECEIVER`, `BRIDGE`. **No** `CREATOR`, `HOSTILE`, `FREE`,
  `SEED`, `PUBLISHER`.

**Discovery (lines ~260-340):**

```bash
_discover_v2_service() {
  local role="$1" svc_output_key="$2" cluster_name="$3"
  local svc_name
  svc_name="$(cf_output "$svc_output_key")"
  [[ -z "$svc_name" || "$svc_name" == "None" ]] && return 0

  local -a arns
  mapfile -t arns < <(
    aws ecs list-tasks --cluster "$cluster_name" --service-name "$svc_name" \
      --desired-status RUNNING --region "$AWS_REGION" \
      --query 'taskArns[]' --output text 2>/dev/null \
      | tr '\t' '\n' | grep -v '^$' || true
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
    arn="$(awk '{print $1}' <<< "$row")"
    ip="$(awk '{print $2}' <<< "$row")"
    container="$(awk '{print $3}' <<< "$row")"
    [[ -z "$ip" || "$ip" == "None" || -z "$arn" ]] && continue
    [[ -z "$container" || "$container" == "None" ]] && container="task"
    short="${arn##*/}"; short="${short:0:8}"
    NODE_LABELS+=("$ip  [$role / $container / ECS ${short}...]")
    NODE_DESCS+=("ECS:$arn:$container")
    NODE_IPS+=("$ip")
    NODE_ROLES+=("$role")
  done
}

discover_all_nodes() {
  local cluster_name
  cluster_name="$(cf_output ClusterName)"
  _discover_v2_service AUTHORITY AuthorityServiceName "$cluster_name"
  _discover_v2_service RECEIVER  ReceiverServiceName  "$cluster_name"
  _discover_v2_service BRIDGE    BridgeServiceName    "$cluster_name"
  echo "  Found ${#NODE_LABELS[@]} live node(s)." >&2
  [[ "${#NODE_LABELS[@]}" -eq 0 ]] && { echo "ERROR: no nodes discovered." >&2; exit 1; }
}
```

**Pickers (lines ~340-440):**
- `_pick_node` — copy from
  [V1 lines 227-257](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh#L227-L257)
  with role labels updated.
- `print_node_table` — copy from
  [V1 lines 212-221](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh#L212-L221).
- `_pick_log_group` — new helper that lists 3 log groups (Authority/Receiver/Bridge) and
  returns the chosen one. Reads from cf_output keys `AuthorityLogGroup`,
  `ReceiverLogGroup`, `BridgeLogGroup`.

**Action handlers (lines ~440-700):**

```bash
do_status() {
  local script_dir; script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  "$script_dir/smoke-conduit-full.sh" --stack-name "$STACK_NAME" --region "$AWS_REGION"
}

do_outputs() {
  aws cloudformation describe-stacks --stack-name "$STACK_NAME" --region "$AWS_REGION" \
    --query 'Stacks[0].Outputs[].{Key:OutputKey,Value:OutputValue}' --output table
}

do_tail_logs() {
  local lg; lg="$(_pick_log_group)"
  echo "Tailing $lg — Ctrl-C to stop." >&2
  aws logs tail "$lg" --follow --region "$AWS_REGION"
}

do_exec_shell() {
  local idx; idx="$(_pick_node "Pick node to shell into:")"
  local desc="${NODE_DESCS[$idx]}"
  local arn="${desc#ECS:}"; arn="${arn%:*}"
  local container="${desc##*:}"
  local cluster; cluster="$(cf_output ClusterName)"
  aws ecs execute-command --cluster "$cluster" --task "$arn" \
    --container "$container" --interactive --command "/bin/sh" --region "$AWS_REGION"
  restore_tty
}

do_show_catalog() {
  local idx; idx="$(_pick_node "Pick AUTHORITY node:" "AUTHORITY")"
  local desc="${NODE_DESCS[$idx]}"
  local arn="${desc#ECS:}"; arn="${arn%:*}"
  local container="${desc##*:}"
  _ecs_execute_command_retry "$arn" "$container" \
    "curl -sS http://127.0.0.1:8080/v1/creator/catalog | python3 -m json.tool"
  restore_tty
}

do_dump_bridges() {
  local idx; idx="$(_pick_node "Pick AUTHORITY node:" "AUTHORITY")"
  _curl_admin "$idx" GET /v1/admin/bridges | _pretty_json
}

do_dump_frames() {
  echo "" >&2
  read -r -p "  Filter by chain_id (blank = all): " cid
  read -r -p "  Limit (blank = default 1000): " lim
  local query="?"
  [[ -n "$cid" ]] && query+="chain_id=${cid}&"
  [[ -n "$lim" ]] && query+="limit=${lim}&"
  query="${query%?}"; query="${query%&}"
  local idx; idx="$(_pick_node "Pick AUTHORITY node:" "AUTHORITY")"
  _curl_admin "$idx" GET "/v1/admin/frames${query}" | _pretty_json
}

do_admin_metrics() {
  local idx; idx="$(_pick_node "Pick node:")"
  _curl_admin "$idx" GET /v1/admin/metrics | _pretty_json
}

do_live_metrics() {
  local interval=30
  read -r -p "Refresh interval in seconds [30]: " iv
  [[ "$iv" =~ ^[1-9][0-9]*$ ]] && interval="$iv"
  echo "  Polling every ${interval}s — Ctrl-C to exit" >&2
  # Loop: every $interval seconds, query CW for the Veritas/Conduit metrics, render.
  # Mirrors V1's do_live_metrics structure (lines 776-872) but with V2 metric names
  # and Veritas/Conduit namespace.
  while true; do
    clear
    echo "Veritas Conduit — Live Metrics (Stack: $STACK_NAME, Region: $AWS_REGION)"
    echo ""
    for service in authority receiver bridge; do
      echo "  $service:"
      for metric in $(_metric_names_for "$service"); do
        local val
        val="$(aws cloudwatch get-metric-statistics \
          --namespace "$CW_NAMESPACE" \
          --metric-name "$metric" \
          --dimensions Name=Service,Value="$service" Name=Stack,Value="$(metric_stack_dimension)" \
          --start-time "$(date -u -d '5 minutes ago' +%FT%TZ)" \
          --end-time "$(date -u +%FT%TZ)" \
          --period 60 --statistics Sum --region "$AWS_REGION" \
          --query 'Datapoints[-1].Sum' --output text 2>/dev/null)"
        printf "    %-30s %s\n" "$metric" "${val:-—}"
      done
    done
    sleep "$interval"
  done
}

do_send_dummy() {
  local idx; idx="$(_pick_node "Pick node to act as creator:")"
  read -r -p "Frame size in bytes [512]: " size
  size="${size:-512}"
  echo "Triggering send_dummy on ${NODE_LABELS[$idx]}..." >&2
  local result
  result="$(_curl_admin "$idx" POST /v1/admin/send-dummy "{\"size\":${size}}")"
  printf '%s\n' "$result" | _pretty_json
  local chain_id
  chain_id="$(printf '%s' "$result" | python3 -c "import sys,json; print(json.loads(sys.stdin.read()).get('chain_id',''))")"
  [[ -z "$chain_id" ]] && { echo "WARN: no chain_id in response"; return 1; }
  echo ""
  echo "  Root chain_id: $chain_id"
  echo ""
  read -r -p "Collect traces from each involved log group? [Y/n]: " yn
  [[ "${yn,,}" == "n" ]] && return 0
  _collect_chain_traces "$chain_id" "$idx"
}

_collect_chain_traces() {
  local chain_id="$1" originator_idx="$2"
  local originator_role="${NODE_ROLES[$originator_idx]}"
  local auth_lg recv_lg bridge_lg
  auth_lg="$(cf_output AuthorityLogGroup)"
  recv_lg="$(cf_output ReceiverLogGroup)"
  bridge_lg="$(cf_output BridgeLogGroup)"
  for lg in "$auth_lg" "$recv_lg" "$bridge_lg"; do
    echo ""
    echo "=== $lg ==="
    aws logs filter-log-events \
      --log-group-name "$lg" \
      --filter-pattern "\"$chain_id\"" \
      --start-time "$(($(date -u +%s%3N) - 300000))" \
      --region "$AWS_REGION" \
      --query 'events[].[timestamp,message]' --output text 2>/dev/null \
      | head -50
  done
}

do_trigger_command() {
  local idx; idx="$(_pick_node "Pick AUTHORITY node:" "AUTHORITY")"
  echo ""
  echo "  [1] CatalogRefresh"
  echo "  [2] SeedAssign"
  echo "  [3] Revoke"
  read -r -p "  Command [1-3]: " cmd_choice
  read -r -p "  Target bridge_id: " target
  local payload
  case "$cmd_choice" in
    1) payload='{"payload":{"CatalogRefresh":{}}}' ;;
    2) payload='{"payload":{"SeedAssign":{}}}' ;;  # operator may need to fill seed details
    3) payload='{"payload":{"Revoke":{}}}' ;;
    *) echo "Invalid"; return 1 ;;
  esac
  _curl_admin "$idx" POST "/v1/admin/bridges/${target}/command" "$payload" | _pretty_json
}

do_check_images() {
  # Adapted from V1 do_check_images (lines 1274-1380), ECS-only branch.
  # For each NODE_DESCS entry, parse image URI from the running task, query ECR for
  # :latest digest, compare. Print a status table.
  ...
}

do_bootstrap_smoke() {
  local script_dir; script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  "$script_dir/bootstrap-smoke.sh" --stack-name "$STACK_NAME" --region "$AWS_REGION"
}

do_refresh() {
  NODE_LABELS=(); NODE_DESCS=(); NODE_IPS=(); NODE_ROLES=()
  discover_all_nodes
  print_node_table
}

do_teardown() {
  read -r -p "Type the stack name to confirm deletion: " confirm
  if [[ "$confirm" == "$STACK_NAME" ]]; then
    local script_dir; script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    "$script_dir/teardown-conduit-full.sh" --stack-name "$STACK_NAME" --region "$AWS_REGION"
  else
    echo "confirmation mismatch; not deleting"
  fi
}
```

### 5.2 Final menu (lines ~700-800):

```bash
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
      "SendDummy" \
      "TriggerCommand" \
      "CheckImages" \
      "BootstrapSmoke" \
      "Refresh" \
      "Teardown" \
      "Exit"; do
      case "$CMD" in
        Status)          do_status ;;
        StackOutputs)    do_outputs ;;
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
        BootstrapSmoke)  do_bootstrap_smoke ;;
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

### 5.3 Documentation: `prototype/gbn-bridge-proto/infra/README-infra.md`

Add a new section under operational entrypoints:

```markdown
### Interactive control panel

The interactive operator panel is at
[`infra/scripts/relay-control-interactive-v2.sh`](scripts/relay-control-interactive-v2.sh).
Run it with:

```bash
bash prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh
```

Override stack name or region via `GBN_BRIDGE_STACK_NAME` / `GBN_BRIDGE_AWS_REGION`.

The panel discovers all running ECS tasks for the chosen stack and presents a numbered
menu. Every admin action is performed via `aws ecs execute-command --interactive` against
the chosen task's localhost admin port (9090). No public ingress is required.

Menu items:
- Status / StackOutputs / TailLogs / ExecShell / ShowCatalog — read-only diagnostics.
- DumpBridges / DumpFrames / AdminMetrics — Phase 1 admin endpoints.
- LiveMetrics — CloudWatch dashboard, namespace `Veritas/Conduit`.
- SendDummy — pick any node to act as creator, follow chain_id through the system.
- TriggerCommand — push a `BridgeCommandPayload` into a bridge's WS stream.
- CheckImages — compare each task's running image vs ECR `:latest`.
- BootstrapSmoke — runs `bootstrap-smoke.sh`.
- Refresh / Teardown / Exit.
```

---

## 6. Implementation Notes

Phase 5 landed as an in-place replacement of
[relay-control-interactive-v2.sh](../../../prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh)
plus the operator documentation update in
[README-infra.md](../../../prototype/gbn-bridge-proto/infra/README-infra.md).

Implementation details that changed from the draft specification:

1. `ShowCatalog` uses the Phase 1 authority admin bridge registry
   (`GET /v1/admin/bridges`) instead of `GET /v1/creator/catalog`. The current V2
   creator API exposes signed bootstrap/catalog behavior through `POST /v1/creator/bootstrap`,
   not an unauthenticated live `GET /v1/creator/catalog` route.
2. `TriggerCommand` generates serde-compatible, internally tagged
   `BridgeCommandPayload` JSON for `catalog_refresh` and `revoke`, and also allows raw
   operator-provided `BridgeCommandPayload` JSON. `SeedAssign` is left to the raw JSON
   path because the real variant needs signed seed-assignment fields.
3. `SendDummy` parses `chain_id` and `assigned_bridge_id` from the Phase 4 admin
   response, then offers a CloudWatch Logs trace pass across authority, receiver, and
   bridge log groups.
4. `LiveMetrics` queries the CloudWatch `Stack` dimension using the stack's
   `EnvironmentName` parameter, matching the Phase 3 metrics emitter. Operators can
   override this with `GBN_BRIDGE_METRICS_STACK_DIMENSION`.
5. The Status Trackers table in
   [GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md](GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md)
   has been updated to mark Phase 5 complete.

---

## 7. Validation

Completed local validation:

1. `bash -n prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh`
   passes.
2. `shellcheck` is not installed in the current local environment, so shellcheck validation
   remains a follow-up when that tool is available.
3. V1 protected-path diff remains clean; this phase only edits V2 paths and docs.

Deferred live validation once the local Kubernetes or deployed AWS stack is available:

1. Deploy `gbn-conduit-full-dev` stack.
2. Run the script. Walk every menu item. Each must succeed or print a meaningful error.
3. SendDummy from each discovered node:
   - returns a chain_id
   - the trace collection step finds the chain_id in at least the originator-node and
     receiver log groups (assigned-bridge log group too unless authority assigned the
     originator-bridge to itself, in which case the bridge appears once)
4. TriggerCommand `CatalogRefresh` to a chosen bridge: bridge log group shows the
   refresh arriving with the issued seq_no.
5. LiveMetrics renders non-empty rows within 3 minutes of a fresh deploy.
6. CheckImages prints `up-to-date` for tasks whose images match ECR `:latest`.
7. Teardown removes the stack and exits cleanly.
8. Re-run script after teardown - the discovery step prints "no nodes discovered" and
    exits with non-zero, not a panic.
9. Update this phase document with live-validation results once that pass completes.

---

## 8. Open Questions Carried Into Implementation

1. **HTML report for SendDummy** — V1 generates a styled HTML report at
   [lines 1034-1271](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh#L1034-L1271).
   Skip in this phase, ship as Phase 5b later if operators want it.
2. **`shellcheck` strictness level** — run with default flags once shellcheck is available;
   warnings should be reviewed rather than blanket-suppressed.
3. **Metric discovery** — `_metric_names_for` intentionally hardcodes the Phase 3 metric
   names for fast, predictable output instead of calling `cloudwatch list-metrics` on every
   refresh.
4. **`SeedAssign` payload shape** — the script supports it through the raw JSON path.
   A structured prompt can be added later if operators need frequent seed assignment tests.
5. **Concurrent operator sessions** — two operators running the script at once is
   technically fine (admin endpoints are stateless), but may cause confusion. No locking.
   Recommend: ship without locking; add advisory check later if needed.
