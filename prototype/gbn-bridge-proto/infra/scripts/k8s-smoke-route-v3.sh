#!/usr/bin/env bash
# Pass 3 Smoke 3: validate local-DHT SendDummy routing and encryption boundary.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
OBS_NS="${VERITAS_OBS_NAMESPACE:-observability}"
EXPECTED_BRIDGES="${VERITAS_K8S_EXPECTED_BRIDGES:-10}"
ADMIN_PORT="${VERITAS_K8S_ADMIN_PORT:-9090}"
MESSAGE_SIZE="${VERITAS_K8S_SMOKE_MESSAGE_SIZE:-256}"
TRACE_TIMEOUT_SECONDS="${VERITAS_K8S_SMOKE_TRACE_TIMEOUT:-300}"
BOOTSTRAP_TIMEOUT_SECONDS="${VERITAS_K8S_SMOKE_BOOTSTRAP_TIMEOUT:-120}"
MIN_ACTIVE_BRIDGES="${VERITAS_K8S_SMOKE_MIN_ACTIVE_BRIDGES:-5}"
CHAIN_ID_PREFIX="${VERITAS_K8S_SMOKE_CHAIN_PREFIX:-smoke-3-}"
PLAINTEXT_MARKER="${VERITAS_K8S_SMOKE_PLAINTEXT_MARKER:-VERITAS-SMOKE-3-PLAINTEXT}"
REQUIRE_OBSERVABILITY=1
INCLUDE_FAILOVER=1
BRIDGE_DECRYPT_ATTEMPT=1
ARTIFACT_DIR=""

usage() {
  cat <<'EOF'
Usage: k8s-smoke-route-v3.sh [options]

Options:
  --namespace NAME                    Kubernetes namespace for Conduit pods.
  --observability-namespace NAME      Kubernetes namespace for Prometheus/Loki/Tempo.
  --expected-bridges N                Expected exit-bridge pod count. Default: 10.
  --message-size N                    Dummy plaintext bytes. Default: 256.
  --plaintext-marker TEXT             Marker placed in dummy plaintext before encryption.
  --trace-timeout N                   Seconds to wait for Tempo event evidence. Default: 300.
  --bootstrap-timeout N               Seconds for automatic Smoke 2 bootstrap if needed. Default: 120.
  --min-active-bridges N              Minimum active bridges for bootstrap fallback. Default: 5.
  --include-failover                  Run forced bridge-failure invocation. Default.
  --no-include-failover               Only run the normal invocation.
  --bridge-decrypt-attempt            Grep assigned bridge logs for plaintext marker. Default.
  --no-bridge-decrypt-attempt         Skip bridge plaintext grep.
  --require-observability             Require Tempo evidence. Default.
  --no-require-observability          Skip Tempo event assertions.
  --chain-id-prefix PREFIX            Prefix for generated chain ids.
  --artifact-dir DIR                  Artifact output directory.
  -h, --help                          Show this help.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --namespace) NAMESPACE="$2"; shift 2 ;;
    --observability-namespace) OBS_NS="$2"; shift 2 ;;
    --expected-bridges) EXPECTED_BRIDGES="$2"; shift 2 ;;
    --message-size) MESSAGE_SIZE="$2"; shift 2 ;;
    --plaintext-marker) PLAINTEXT_MARKER="$2"; shift 2 ;;
    --trace-timeout) TRACE_TIMEOUT_SECONDS="$2"; shift 2 ;;
    --bootstrap-timeout) BOOTSTRAP_TIMEOUT_SECONDS="$2"; shift 2 ;;
    --min-active-bridges) MIN_ACTIVE_BRIDGES="$2"; shift 2 ;;
    --include-failover) INCLUDE_FAILOVER=1; shift ;;
    --no-include-failover) INCLUDE_FAILOVER=0; shift ;;
    --bridge-decrypt-attempt) BRIDGE_DECRYPT_ATTEMPT=1; shift ;;
    --no-bridge-decrypt-attempt) BRIDGE_DECRYPT_ATTEMPT=0; shift ;;
    --require-observability) REQUIRE_OBSERVABILITY=1; shift ;;
    --no-require-observability) REQUIRE_OBSERVABILITY=0; shift ;;
    --chain-id-prefix) CHAIN_ID_PREFIX="$2"; shift 2 ;;
    --artifact-dir) ARTIFACT_DIR="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument '$1'." >&2; usage >&2; exit 2 ;;
  esac
done

uname -a | grep -i microsoft >/dev/null || {
  echo "Pass 3 tooling requires WSL2 Ubuntu" >&2
  exit 1
}

cd "$ROOT_DIR"
source "$SCRIPT_DIR/k8s-smoke-common.sh"

smoke_require_deps
smoke_artifact_dir smoke-3-route >/dev/null
trap 'status=$?; if [[ $status -ne 0 ]]; then smoke_collect_diagnostics; fi; smoke_stop_observability; echo "Artifacts: $ARTIFACT_DIR"; exit $status' EXIT

mkdir -p "$ARTIFACT_DIR/bridge-logs-by-chain-id" \
  "$ARTIFACT_DIR/traces-by-chain-id" \
  "$ARTIFACT_DIR/loki" \
  "$ARTIFACT_DIR/tempo"

json_string() {
  python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$1"
}

json_field_from_arg() {
  local raw="$1" field="$2"
  python3 -c 'import json,sys; print(json.loads(sys.argv[1]).get(sys.argv[2], ""))' "$raw" "$field"
}

local_dht_ready() {
  local path="$1"
  python3 - "$path" <<'PY'
import json
import sys
import time

path = sys.argv[1]
data = json.load(open(path, encoding="utf-8"))
now_ms = int(time.time() * 1000)
state = data.get("self_onboarding_state")
if state not in {"onboarded", "fanout_partial"}:
    raise SystemExit(1)
eligible = []
future_suspect = []
for entry in data.get("bridge_entries") or []:
    suspect_until = entry.get("suspect_until_ms")
    if suspect_until and int(suspect_until) > now_ms:
        future_suspect.append(entry.get("bridge_id"))
        continue
    if not entry.get("active"):
        continue
    if entry.get("reachability_class") == "relay_only":
        continue
    if int(entry.get("entry_expiry_ms") or 0) <= now_ms:
        continue
    if int(entry.get("lease_expiry_ms") or 0) <= now_ms:
        continue
    if not entry.get("ingress_endpoints"):
        continue
    eligible.append(entry.get("bridge_id"))
if len(eligible) < 2:
    raise SystemExit(1)
if future_suspect:
    raise SystemExit(1)
print(json.dumps({"state": state, "eligible_bridge_ids": eligible}, sort_keys=True))
PY
}

ensure_creator_onboarded() {
  smoke_admin_curl "$CREATOR_NEW_POD" creator-runner GET /v1/admin/local-dht \
    >"$ARTIFACT_DIR/creator-local-dht-before-bootstrap-check.json"
  if local_dht_ready "$ARTIFACT_DIR/creator-local-dht-before-bootstrap-check.json" \
    >"$ARTIFACT_DIR/creator-local-dht-ready-summary.json"; then
    cp "$ARTIFACT_DIR/creator-local-dht-before-bootstrap-check.json" \
      "$ARTIFACT_DIR/creator-local-dht-before.json"
    return 0
  fi

  echo "creator-new is not ready for Smoke 3; running Smoke 2 bootstrap fallback..."
  local obs_arg="--no-require-observability"
  if [[ "$REQUIRE_OBSERVABILITY" -eq 1 ]]; then
    obs_arg="--require-observability"
  fi
  bash "$SCRIPT_DIR/k8s-smoke-discovery-v3.sh" \
    --namespace "$NAMESPACE" \
    --observability-namespace "$OBS_NS" \
    --expected-bridges "$EXPECTED_BRIDGES" \
    --bootstrap-timeout "$BOOTSTRAP_TIMEOUT_SECONDS" \
    --trace-timeout "$TRACE_TIMEOUT_SECONDS" \
    --min-active-bridges "$MIN_ACTIVE_BRIDGES" \
    --allow-fanout-partial \
    "$obs_arg"

  smoke_discover_nodes
  smoke_admin_curl "$CREATOR_NEW_POD" creator-runner GET /v1/admin/local-dht \
    >"$ARTIFACT_DIR/creator-local-dht-before.json"
  local_dht_ready "$ARTIFACT_DIR/creator-local-dht-before.json" \
    >"$ARTIFACT_DIR/creator-local-dht-ready-summary.json" ||
    smoke_fail "creator-new did not have at least two eligible local-DHT bridges after Smoke 2 bootstrap."
}

build_send_dummy_body() {
  local force="$1"
  MESSAGE_SIZE="$MESSAGE_SIZE" FORCE="$force" PLAINTEXT_MARKER="$PLAINTEXT_MARKER" python3 - <<'PY'
import json
import os

print(json.dumps({
    "size": int(os.environ["MESSAGE_SIZE"]),
    "force_bridge_failure": os.environ["FORCE"] == "true",
    "plaintext_marker": os.environ["PLAINTEXT_MARKER"],
}, separators=(",", ":")))
PY
}

assert_send_dummy_response() {
  local label="$1" chain_id="$2" force="$3" result_path="$4" dht_path="$5"
  python3 - "$label" "$chain_id" "$force" "$result_path" "$dht_path" "$MESSAGE_SIZE" <<'PY'
import json
import sys
import time

label, chain_id, force_raw, result_path, dht_path, size_raw = sys.argv[1:7]
force = force_raw == "true"
size = int(size_raw)
result = json.load(open(result_path, encoding="utf-8"))
dht = json.load(open(dht_path, encoding="utf-8"))
now_ms = int(time.time() * 1000)

def fail(code, **detail):
    raise SystemExit(json.dumps({"label": label, "failure": code, **detail}, sort_keys=True))

if result.get("chain_id") != chain_id:
    fail("chain_id_mismatch", actual=result.get("chain_id"), expected=chain_id)
if result.get("route_source") != "local_dht":
    fail("route_source_mismatch", route_source=result.get("route_source"))
assigned = result.get("assigned_bridge_id")
if not assigned:
    fail("missing_assigned_bridge_id")
entries = {entry.get("bridge_id"): entry for entry in dht.get("bridge_entries") or []}
entry = entries.get(assigned)
if not entry:
    fail("assigned_bridge_not_in_local_dht", assigned_bridge_id=assigned)
if not entry.get("active"):
    fail("assigned_bridge_inactive", assigned_bridge_id=assigned)
if entry.get("reachability_class") == "relay_only":
    fail("assigned_bridge_relay_only", assigned_bridge_id=assigned)
if int(entry.get("entry_expiry_ms") or 0) <= now_ms:
    fail("assigned_bridge_entry_expired", assigned_bridge_id=assigned)
if int(entry.get("lease_expiry_ms") or 0) <= now_ms:
    fail("assigned_bridge_lease_expired", assigned_bridge_id=assigned)
if entry.get("suspect_until_ms") and int(entry["suspect_until_ms"]) > now_ms:
    fail("assigned_bridge_suspect", assigned_bridge_id=assigned)
if assigned not in (result.get("candidate_bridge_ids") or []):
    fail("assigned_bridge_not_in_candidates", assigned_bridge_id=assigned)
if result.get("selected_bridge_ids") != [assigned]:
    fail("selected_bridge_ids_mismatch", selected=result.get("selected_bridge_ids"), assigned=assigned)
if result.get("ciphertext_only_at_bridge") is not True:
    fail("ciphertext_only_false")
if result.get("frames") != 1:
    fail("frames_mismatch", frames=result.get("frames"))
if int(result.get("elapsed_ms") or 0) >= 10000:
    fail("elapsed_too_high", elapsed_ms=result.get("elapsed_ms"))
if result.get("force_bridge_failure_used") is not force:
    fail("force_flag_mismatch", actual=result.get("force_bridge_failure_used"), expected=force)
if size < len("VERITAS-SMOKE-3-PLAINTEXT"):
    fail("message_size_too_small_for_marker", size=size)
PY
}

wait_for_frames() {
  local chain_id="$1" output="$2" deadline count
  deadline=$((SECONDS + 60))
  while ((SECONDS <= deadline)); do
    smoke_frames_by_chain_id "$chain_id" "$output" 10
    count="$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1], encoding="utf-8")).get("frames", [])))' "$output")"
    if [[ "$count" -ge 1 ]]; then
      return 0
    fi
    sleep 2
  done
  return 1
}

assert_frames() {
  local label="$1" chain_id="$2" result_path="$3" frames_path="$4"
  python3 - "$label" "$chain_id" "$result_path" "$frames_path" "$MESSAGE_SIZE" <<'PY'
import json
import sys

label, chain_id, result_path, frames_path, size_raw = sys.argv[1:6]
size = int(size_raw)
result = json.load(open(result_path, encoding="utf-8"))
frames = json.load(open(frames_path, encoding="utf-8")).get("frames") or []

def fail(code, **detail):
    raise SystemExit(json.dumps({"label": label, "failure": code, **detail}, sort_keys=True))

if not frames:
    fail("missing_frames")
frame = frames[0]
assigned = result.get("assigned_bridge_id")
if frame.get("chain_id") != chain_id:
    fail("frame_chain_id_mismatch", actual=frame.get("chain_id"), expected=chain_id)
if frame.get("via_bridge_id") != assigned:
    fail("via_bridge_mismatch", actual=frame.get("via_bridge_id"), expected=assigned)
ciphertext = ((frame.get("frame") or {}).get("ciphertext") or [])
if len(ciphertext) < size + 16:
    fail("ciphertext_too_short", ciphertext_len=len(ciphertext), message_size=size)
PY
}

collect_bridge_log() {
  local chain_id="$1" bridge_id="$2" output="$3"
  kubectl -n "$NAMESPACE" logs --since=30m "$bridge_id" -c exit-bridge \
    --insecure-skip-tls-verify-backend=true >"$output" 2>/dev/null || true
  if ! grep -q "$chain_id" "$output"; then
    smoke_fail "assigned bridge $bridge_id logs did not contain chain_id=$chain_id."
  fi
  if ! grep -q "payload_bytes=" "$output"; then
    smoke_fail "assigned bridge $bridge_id logs did not include payload_bytes for chain_id=$chain_id."
  fi
}

wait_tempo_send_dummy_events() {
  local chain_id="$1" output_dir="$2" deadline search_output trace_output counts_output missing_output
  search_output="$output_dir/tempo-search.json"
  trace_output="$output_dir/tempo-traces.json"
  counts_output="$output_dir/traces-by-event.json"
  missing_output="$output_dir/missing-events.txt"
  deadline=$((SECONDS + TRACE_TIMEOUT_SECONDS))
  while ((SECONDS <= deadline)); do
    smoke_tempo_query_chain "$chain_id" "$search_output"
    if TEMPO_URL="$TEMPO_URL" python3 - \
      "$search_output" "$trace_output" "$counts_output" "$missing_output" "${SEND_DUMMY_EVENTS[@]}" <<'PY'
import json
import os
import sys
import urllib.request

search_path, trace_path, counts_path, missing_path, *expected = sys.argv[1:]
tempo_url = os.environ["TEMPO_URL"].rstrip("/")
search = json.load(open(search_path, encoding="utf-8"))
trace_ids = []
for trace in search.get("traces", search.get("data", {}).get("traces", [])):
    trace_id = trace.get("traceID") or trace.get("traceId")
    if trace_id and trace_id not in trace_ids:
        trace_ids.append(trace_id)

details = []
for trace_id in trace_ids:
    try:
        with urllib.request.urlopen(f"{tempo_url}/api/traces/{trace_id}", timeout=10) as response:
            details.append(json.load(response))
    except Exception as exc:
        details.append({"traceID": trace_id, "fetch_error": str(exc)})

json.dump(details, open(trace_path, "w", encoding="utf-8"), indent=2, sort_keys=True)

strings = []

def scalar(value):
    if isinstance(value, dict):
        for key in ("stringValue", "intValue", "doubleValue", "boolValue"):
            if key in value:
                return str(value[key])
    if isinstance(value, (str, int, float, bool)):
        return str(value)
    return None

def walk(value):
    if isinstance(value, dict):
        if "key" in value and "value" in value:
            maybe = scalar(value.get("value"))
            if maybe is not None:
                strings.append(maybe)
        for item in value.values():
            walk(item)
    elif isinstance(value, list):
        for item in value:
            walk(item)
    elif isinstance(value, (str, int, float, bool)):
        strings.append(str(value))

for detail in details:
    walk(detail)

counts = {event: strings.count(event) for event in expected}
for forbidden in ("discovery_probe", "admin_discovery_probe", "catalog_request", "bootstrap_request"):
    counts[f"_forbidden_{forbidden}"] = strings.count(forbidden)
missing = [event for event in expected if counts[event] < 1]
for forbidden in ("discovery_probe", "admin_discovery_probe", "catalog_request", "bootstrap_request"):
    if counts[f"_forbidden_{forbidden}"] > 0:
        missing.append(f"forbidden:{forbidden}")
counts["_trace_ids"] = trace_ids
json.dump(counts, open(counts_path, "w", encoding="utf-8"), indent=2, sort_keys=True)
if missing:
    with open(missing_path, "w", encoding="utf-8") as handle:
        handle.write("\n".join(missing) + "\n")
    raise SystemExit(1)
try:
    os.remove(missing_path)
except FileNotFoundError:
    pass
PY
    then
      return 0
    fi
    sleep 5
  done
  return 1
}

assert_failover() {
  local normal_path="$1" failover_path="$2" dht_after_path="$3"
  python3 - "$normal_path" "$failover_path" "$dht_after_path" <<'PY'
import json
import sys
import time

normal_path, failover_path, dht_after_path = sys.argv[1:4]
normal = json.load(open(normal_path, encoding="utf-8"))
failover = json.load(open(failover_path, encoding="utf-8"))
dht_after = json.load(open(dht_after_path, encoding="utf-8"))
now_ms = int(time.time() * 1000)

def fail(code, **detail):
    raise SystemExit(json.dumps({"failure": code, **detail}, sort_keys=True))

b1 = normal.get("assigned_bridge_id")
b2 = failover.get("assigned_bridge_id")
if not b1 or not b2 or b1 == b2:
    fail("failover_bridge_not_distinct", normal=b1, failover=b2)
if failover.get("force_bridge_failure_used") is not True:
    fail("failover_force_flag_false")
candidates = set(failover.get("candidate_bridge_ids") or [])
if b1 not in candidates or b2 not in candidates:
    fail("failover_candidates_missing_bridge", candidates=sorted(candidates), normal=b1, failover=b2)
entries = {entry.get("bridge_id"): entry for entry in dht_after.get("bridge_entries") or []}
suspect_until = (entries.get(b1) or {}).get("suspect_until_ms")
if not suspect_until or int(suspect_until) <= now_ms:
    fail("normal_bridge_not_marked_suspect", bridge_id=b1, suspect_until_ms=suspect_until)
PY
}

SEND_DUMMY_EVENTS=(
  creator_send_dummy_requested
  creator_local_dht_loaded
  creator_route_selected
  creator_bridge_open_sent
  creator_dummy_frame_sent
  bridge_dummy_frame_forwarded
  receiver_dummy_frame_ingested
  publisher_dummy_ack_returned
)

echo "Checking Pass 3 Conduit rollout in namespace '$NAMESPACE'..."
smoke_check_rollouts
smoke_discover_nodes
smoke_check_admin_metrics
smoke_wait_for_bridge_registry

ensure_creator_onboarded

CHAIN_ID_NORMAL="${CHAIN_ID_PREFIX}normal-$(python3 -c 'import uuid; print(uuid.uuid4().hex)')"
CHAIN_ID_FAILOVER="${CHAIN_ID_PREFIX}failover-$(python3 -c 'import uuid; print(uuid.uuid4().hex)')"
SMOKE_LOKI_QUERY_START_NS="$(date +%s%N)"
printf '%s\n' "$CHAIN_ID_NORMAL" >"$ARTIFACT_DIR/chain-id-normal.txt"
printf '%s\n' "$CHAIN_ID_FAILOVER" >"$ARTIFACT_DIR/chain-id-failover.txt"

echo "Running normal SendDummy with chain_id=$CHAIN_ID_NORMAL..."
NORMAL_BODY="$(build_send_dummy_body false)"
printf '%s' "$NORMAL_BODY" >"$ARTIFACT_DIR/send-dummy-normal-payload.json"
NORMAL_RESPONSE="$(smoke_admin_curl "$CREATOR_NEW_POD" creator-runner POST "/v1/admin/send-dummy?chain_id=${CHAIN_ID_NORMAL}" "$NORMAL_BODY")"
printf '%s' "$NORMAL_RESPONSE" | python3 -m json.tool >"$ARTIFACT_DIR/send-dummy-normal-result.json"
assert_send_dummy_response normal "$CHAIN_ID_NORMAL" false "$ARTIFACT_DIR/send-dummy-normal-result.json" "$ARTIFACT_DIR/creator-local-dht-before.json"
smoke_admin_curl "$CREATOR_NEW_POD" creator-runner GET /v1/admin/local-dht \
  >"$ARTIFACT_DIR/creator-local-dht-after-normal.json"
wait_for_frames "$CHAIN_ID_NORMAL" "$ARTIFACT_DIR/frames-normal.json" ||
  smoke_fail "receiver did not persist a frame for normal chain_id=$CHAIN_ID_NORMAL."
assert_frames normal "$CHAIN_ID_NORMAL" "$ARTIFACT_DIR/send-dummy-normal-result.json" "$ARTIFACT_DIR/frames-normal.json"

NORMAL_BRIDGE_ID="$(json_field_from_arg "$NORMAL_RESPONSE" assigned_bridge_id)"
collect_bridge_log "$CHAIN_ID_NORMAL" "$NORMAL_BRIDGE_ID" "$ARTIFACT_DIR/bridge-logs-by-chain-id/${CHAIN_ID_NORMAL}.log"

if [[ "$INCLUDE_FAILOVER" -eq 1 ]]; then
  echo "Running failover SendDummy with chain_id=$CHAIN_ID_FAILOVER..."
  FAILOVER_BODY="$(build_send_dummy_body true)"
  printf '%s' "$FAILOVER_BODY" >"$ARTIFACT_DIR/send-dummy-failover-payload.json"
  FAILOVER_RESPONSE="$(smoke_admin_curl "$CREATOR_NEW_POD" creator-runner POST "/v1/admin/send-dummy?chain_id=${CHAIN_ID_FAILOVER}" "$FAILOVER_BODY")"
  printf '%s' "$FAILOVER_RESPONSE" | python3 -m json.tool >"$ARTIFACT_DIR/send-dummy-failover-result.json"
  assert_send_dummy_response failover "$CHAIN_ID_FAILOVER" true "$ARTIFACT_DIR/send-dummy-failover-result.json" "$ARTIFACT_DIR/creator-local-dht-before.json"
  smoke_admin_curl "$CREATOR_NEW_POD" creator-runner GET /v1/admin/local-dht \
    >"$ARTIFACT_DIR/creator-local-dht-after-failover.json"
  wait_for_frames "$CHAIN_ID_FAILOVER" "$ARTIFACT_DIR/frames-failover.json" ||
    smoke_fail "receiver did not persist a frame for failover chain_id=$CHAIN_ID_FAILOVER."
  assert_frames failover "$CHAIN_ID_FAILOVER" "$ARTIFACT_DIR/send-dummy-failover-result.json" "$ARTIFACT_DIR/frames-failover.json"
  FAILOVER_BRIDGE_ID="$(json_field_from_arg "$FAILOVER_RESPONSE" assigned_bridge_id)"
  collect_bridge_log "$CHAIN_ID_FAILOVER" "$FAILOVER_BRIDGE_ID" "$ARTIFACT_DIR/bridge-logs-by-chain-id/${CHAIN_ID_FAILOVER}.log"
  assert_failover "$ARTIFACT_DIR/send-dummy-normal-result.json" \
    "$ARTIFACT_DIR/send-dummy-failover-result.json" \
    "$ARTIFACT_DIR/creator-local-dht-after-failover.json"
else
  cp "$ARTIFACT_DIR/creator-local-dht-after-normal.json" "$ARTIFACT_DIR/creator-local-dht-after-failover.json"
  printf '{}\n' >"$ARTIFACT_DIR/send-dummy-failover-result.json"
  printf '{"frames":[]}\n' >"$ARTIFACT_DIR/frames-failover.json"
fi

if [[ "$BRIDGE_DECRYPT_ATTEMPT" -eq 1 ]]; then
  if grep -R -- "$PLAINTEXT_MARKER" "$ARTIFACT_DIR/bridge-logs-by-chain-id" \
    >"$ARTIFACT_DIR/bridge-plaintext-grep.txt"; then
    smoke_fail "plaintext marker appeared in assigned bridge logs."
  fi
  : >"$ARTIFACT_DIR/bridge-plaintext-grep.txt"
fi

if [[ "$REQUIRE_OBSERVABILITY" -eq 1 ]]; then
  echo "Starting Tempo port-forward and checking SendDummy events..."
  smoke_start_observability
  mkdir -p "$ARTIFACT_DIR/traces-by-chain-id/normal"
  wait_tempo_send_dummy_events "$CHAIN_ID_NORMAL" "$ARTIFACT_DIR/traces-by-chain-id/normal" ||
    smoke_fail "Tempo did not report all 8 SendDummy events for normal chain_id=$CHAIN_ID_NORMAL."
  if [[ "$INCLUDE_FAILOVER" -eq 1 ]]; then
    mkdir -p "$ARTIFACT_DIR/traces-by-chain-id/failover"
    wait_tempo_send_dummy_events "$CHAIN_ID_FAILOVER" "$ARTIFACT_DIR/traces-by-chain-id/failover" ||
      smoke_fail "Tempo did not report all 8 SendDummy events for failover chain_id=$CHAIN_ID_FAILOVER."
  fi
fi

python3 - \
  "$ARTIFACT_DIR/send-dummy-normal-result.json" \
  "$ARTIFACT_DIR/send-dummy-failover-result.json" \
  "$ARTIFACT_DIR/frames-normal.json" \
  "$ARTIFACT_DIR/frames-failover.json" \
  "$ARTIFACT_DIR/route-summary.md" \
  "$INCLUDE_FAILOVER" <<'PY'
import json
import sys

normal_path, failover_path, frames_normal_path, frames_failover_path, output_path, include_failover = sys.argv[1:7]
rows = []
for label, result_path, frames_path in [
    ("normal", normal_path, frames_normal_path),
    ("failover", failover_path, frames_failover_path),
]:
    if label == "failover" and include_failover != "1":
        continue
    result = json.load(open(result_path, encoding="utf-8"))
    frames = json.load(open(frames_path, encoding="utf-8")).get("frames") or []
    rows.append({
        "invocation": label,
        "chain_id": result.get("chain_id"),
        "assigned_bridge_id": result.get("assigned_bridge_id"),
        "route_source": result.get("route_source"),
        "ciphertext_only_at_bridge": result.get("ciphertext_only_at_bridge"),
        "force_bridge_failure_used": result.get("force_bridge_failure_used"),
        "frame_count": len(frames),
    })
with open(output_path, "w", encoding="utf-8") as handle:
    handle.write("# Conduit Smoke 3 Route Summary\n\n")
    handle.write("| Invocation | ChainID | Assigned Bridge | Route Source | Ciphertext Only | Force Failure | Frames |\n")
    handle.write("|---|---|---|---|---:|---:|---:|\n")
    for row in rows:
        handle.write(
            f"| {row['invocation']} | {row['chain_id']} | {row['assigned_bridge_id']} | "
            f"{row['route_source']} | {row['ciphertext_only_at_bridge']} | "
            f"{row['force_bridge_failure_used']} | {row['frame_count']} |\n"
        )
PY

echo "Conduit Smoke 3 route/encryption validation passed."
