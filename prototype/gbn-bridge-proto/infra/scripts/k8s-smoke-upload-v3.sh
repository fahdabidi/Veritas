#!/usr/bin/env bash
# Pass 3 Smoke 4: validate full upload build, encryption, fanout, receiver
# reconstruction, and creator persistence on the local Kubernetes cluster.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
OBS_NS="${VERITAS_OBS_NAMESPACE:-observability}"
EXPECTED_BRIDGES="${VERITAS_K8S_EXPECTED_BRIDGES:-10}"
SYNTHETIC_SIZE="${VERITAS_K8S_UPLOAD_SYNTHETIC_SIZE:-1048576}"
FAILOVER_SYNTHETIC_SIZE="${VERITAS_K8S_UPLOAD_FAILOVER_SYNTHETIC_SIZE:-65536}"
CHUNK_SIZE="${VERITAS_K8S_UPLOAD_CHUNK_SIZE:-8192}"
TARGET_LANE_COUNT="${VERITAS_K8S_UPLOAD_TARGET_LANE_COUNT:-10}"
UPLOAD_TIMEOUT_SECONDS="${VERITAS_K8S_UPLOAD_TIMEOUT_SECONDS:-120}"
TRACE_TIMEOUT_SECONDS="${VERITAS_K8S_SMOKE_TRACE_TIMEOUT:-120}"
BOOTSTRAP_TIMEOUT_SECONDS="${VERITAS_K8S_SMOKE_BOOTSTRAP_TIMEOUT:-180}"
MIN_ACTIVE_BRIDGES="${VERITAS_K8S_UPLOAD_MIN_ACTIVE_BRIDGES:-$EXPECTED_BRIDGES}"
PLAINTEXT_MARKER="${VERITAS_K8S_UPLOAD_MARKER:-VERITAS-SMOKE-4-PLAINTEXT}"
CHAIN_ID_PREFIX="${VERITAS_K8S_SMOKE_CHAIN_PREFIX:-smoke-4-}"
INCLUDE_FAILOVER=1
INCLUDE_PERSISTENCE_CHECK=1
REQUIRE_OBSERVABILITY=1
UPLOAD_CASE_ATTEMPTS="${VERITAS_K8S_UPLOAD_CASE_ATTEMPTS:-3}"
ARTIFACT_DIR=""

usage() {
  cat <<'EOF'
Usage: k8s-smoke-upload-v3.sh [options]

Options:
  --namespace NAME                    Kubernetes namespace for Conduit pods.
  --observability-namespace NAME      Kubernetes namespace for Prometheus/Loki/Tempo.
  --expected-bridges N                Expected exit-bridge pod count. Default: 10.
  --synthetic-size N                  Normal upload synthetic bytes. Default: 1048576.
  --failover-synthetic-size N         Failover upload synthetic bytes. Default: 65536.
  --chunk-size N                      Chunk size in bytes. Default: 8192.
  --target-lane-count N               Target upload lanes. Default: 10.
  --upload-timeout N                  SendUpload timeout in seconds. Default: 120.
  --trace-timeout N                   Trace/log wait timeout in seconds. Default: 120.
  --bootstrap-timeout N               Smoke 2 fallback timeout in seconds. Default: 180.
  --min-active-bridges N              Compatibility flag; Smoke 4 requires all expected bridges.
  --include-failover                  Run the forced-lane-failure upload. Default.
  --no-include-failover               Skip forced-lane-failure upload.
  --include-persistence-check         Restart creator-new and assert session persists. Default.
  --no-include-persistence-check      Skip creator-new restart persistence check.
  --require-observability             Require observability backend hits. Default.
  --no-require-observability          Validate pod logs only.
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
    --synthetic-size) SYNTHETIC_SIZE="$2"; shift 2 ;;
    --failover-synthetic-size) FAILOVER_SYNTHETIC_SIZE="$2"; shift 2 ;;
    --chunk-size) CHUNK_SIZE="$2"; shift 2 ;;
    --target-lane-count) TARGET_LANE_COUNT="$2"; shift 2 ;;
    --upload-timeout) UPLOAD_TIMEOUT_SECONDS="$2"; shift 2 ;;
    --trace-timeout) TRACE_TIMEOUT_SECONDS="$2"; shift 2 ;;
    --bootstrap-timeout) BOOTSTRAP_TIMEOUT_SECONDS="$2"; shift 2 ;;
    --min-active-bridges) MIN_ACTIVE_BRIDGES="$2"; shift 2 ;;
    --include-failover) INCLUDE_FAILOVER=1; shift ;;
    --no-include-failover) INCLUDE_FAILOVER=0; shift ;;
    --include-persistence-check) INCLUDE_PERSISTENCE_CHECK=1; shift ;;
    --no-include-persistence-check) INCLUDE_PERSISTENCE_CHECK=0; shift ;;
    --require-observability) REQUIRE_OBSERVABILITY=1; shift ;;
    --no-require-observability) REQUIRE_OBSERVABILITY=0; shift ;;
    --chain-id-prefix) CHAIN_ID_PREFIX="$2"; shift 2 ;;
    --artifact-dir) ARTIFACT_DIR="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument '$1'." >&2; usage >&2; exit 2 ;;
  esac
done

if [[ "$TARGET_LANE_COUNT" -ne "$EXPECTED_BRIDGES" ]]; then
  echo "ERROR: Smoke 4 requires --target-lane-count to match --expected-bridges so the normal upload proves every ExitBridge was used." >&2
  echo "       target_lane_count=$TARGET_LANE_COUNT expected_bridges=$EXPECTED_BRIDGES" >&2
  exit 2
fi

uname -a | grep -i microsoft >/dev/null || {
  echo "Pass 3 tooling requires WSL2 Ubuntu" >&2
  exit 1
}

cd "$ROOT_DIR"
source "$SCRIPT_DIR/k8s-smoke-common.sh"

export VERITAS_K8S_ADMIN_REQUEST_TIMEOUT_SECONDS="${VERITAS_K8S_ADMIN_REQUEST_TIMEOUT_SECONDS:-$((UPLOAD_TIMEOUT_SECONDS + 60))}"

smoke_require_deps
smoke_artifact_dir smoke-4-upload >/dev/null
trap 'status=$?; if [[ $status -ne 0 ]]; then smoke_collect_diagnostics; fi; smoke_stop_observability; echo "Artifacts: $ARTIFACT_DIR"; exit $status' EXIT

mkdir -p "$ARTIFACT_DIR"/{bridge-logs-by-chain-id,kubectl-logs,loki,tempo,traces-by-chain-id}

rand_suffix() {
  python3 - <<'PY'
import secrets
print(secrets.token_hex(8))
PY
}

write_json_arg() {
  local raw="$1" path="$2"
  RAW_JSON="$raw" python3 - "$path" <<'PY'
import json
import os
import sys
data = json.loads(os.environ["RAW_JSON"])
with open(sys.argv[1], "w", encoding="utf-8") as handle:
    json.dump(data, handle, indent=2, sort_keys=True)
    handle.write("\n")
PY
}

json_file_field() {
  local path="$1" field="$2"
  python3 - "$path" "$field" <<'PY'
import json
import sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
value = data.get(sys.argv[2], "")
print(value if not isinstance(value, (dict, list)) else json.dumps(value))
PY
}

local_dht_ready() {
  local path="$1"
  python3 - "$path" "$EXPECTED_BRIDGES" "$ARTIFACT_DIR/pods.json" <<'PY'
import json
import sys
import time

path, expected_raw, pods_path = sys.argv[1:4]
expected = int(expected_raw)
data = json.load(open(path, encoding="utf-8"))
pods = json.load(open(pods_path, encoding="utf-8"))
pod_ip_by_name = {
    item.get("metadata", {}).get("name"): item.get("status", {}).get("podIP")
    for item in pods.get("items", [])
    if not item.get("metadata", {}).get("deletionTimestamp")
}
now_ms = int(time.time() * 1000)

def fail(code, **detail):
    raise SystemExit(json.dumps({"failure": code, **detail}, sort_keys=True))

def has_key(value):
    return isinstance(value, list) and len(value) == 32

def has_sig(value):
    return isinstance(value, list) and len(value) == 64

def require_creator_entry(name, expected_node_id=None):
    entry = data.get(name) or {}
    if not entry:
        fail(f"missing_{name}")
    if expected_node_id and entry.get("node_id") != expected_node_id:
        fail(f"{name}_node_id_mismatch", actual=entry.get("node_id"), expected=expected_node_id)
    if not entry.get("ip_addr"):
        fail(f"{name}_missing_ip_addr", node_id=entry.get("node_id"))
    if not has_key(entry.get("pub_key")):
        fail(f"{name}_missing_pub_key", node_id=entry.get("node_id"))
    if not has_sig(entry.get("publisher_sig")):
        fail(f"{name}_missing_publisher_sig", node_id=entry.get("node_id"))
    if int(entry.get("udp_punch_port") or 0) <= 0:
        fail(f"{name}_invalid_udp_punch_port", node_id=entry.get("node_id"))
    if int(entry.get("entry_expiry_ms") or 0) <= now_ms:
        fail(f"{name}_expired", node_id=entry.get("node_id"))
    if entry.get("active") is not True:
        fail(f"{name}_inactive", node_id=entry.get("node_id"))
    return entry

state = data.get("self_onboarding_state")
if state != "onboarded":
    fail("self_onboarding_not_complete", state=state)
publisher = data.get("publisher_entry") or {}
if not publisher:
    fail("missing_publisher_entry")
if not publisher.get("node_id"):
    fail("publisher_entry_missing_node_id")
if not publisher.get("authority_url"):
    fail("publisher_entry_missing_authority_url")
if not publisher.get("receiver_url"):
    fail("publisher_entry_missing_receiver_url")
if not has_key(publisher.get("pub_key")):
    fail("publisher_entry_missing_pub_key")
if not has_key(publisher.get("encryption_pub_key")):
    fail("publisher_entry_missing_encryption_pub_key")
if int(publisher.get("entry_expiry_ms") or 0) <= now_ms:
    fail("publisher_entry_expired", entry_expiry_ms=publisher.get("entry_expiry_ms"))

host_creator = require_creator_entry("host_creator_entry")
creator = require_creator_entry("creator_entry", data.get("actor_id"))
session = data.get("current_bootstrap_session") or {}
if not session.get("session_id"):
    fail("missing_current_bootstrap_session")
if session.get("last_state") != state:
    fail("bootstrap_session_state_mismatch", actual=session.get("last_state"), expected=state)

bridges = data.get("bridge_entries") or []
if len(bridges) != expected:
    fail("bridge_entry_count_mismatch", actual=len(bridges), expected=expected)
eligible = []
seen_bridge_ids = set()
for entry in bridges:
    bridge_id = entry.get("bridge_id")
    if not bridge_id:
        fail("bridge_missing_id", entry=entry)
    if bridge_id in seen_bridge_ids:
        fail("duplicate_bridge_id", bridge_id=bridge_id)
    seen_bridge_ids.add(bridge_id)
    if not has_key(entry.get("identity_pub")):
        fail("bridge_missing_identity_pub", bridge_id=bridge_id)
    if not has_sig(entry.get("publisher_sig")):
        fail("bridge_missing_publisher_sig", bridge_id=bridge_id)
    if int(entry.get("udp_punch_port") or 0) <= 0:
        fail("bridge_invalid_udp_punch_port", bridge_id=bridge_id)
    suspect_until = entry.get("suspect_until_ms")
    if suspect_until and int(suspect_until) > now_ms:
        continue
    if not entry.get("active"):
        continue
    if entry.get("reachability_class") not in {"direct", "brokered"}:
        continue
    if int(entry.get("entry_expiry_ms") or 0) <= now_ms:
        continue
    if int(entry.get("lease_expiry_ms") or 0) <= now_ms:
        continue
    endpoints = entry.get("ingress_endpoints") or []
    if not endpoints:
        continue
    pod_ip = pod_ip_by_name.get(entry.get("bridge_id"))
    endpoint_ip = endpoints[0].get("ip_addr")
    if pod_ip and endpoint_ip != pod_ip:
        continue
    if not entry.get("capabilities"):
        fail("bridge_missing_capabilities", bridge_id=bridge_id)
    eligible.append(entry.get("bridge_id"))
if len(eligible) != expected:
    fail("eligible_bridge_count_mismatch", actual=len(eligible), expected=expected)

tunnel_bridge_ids = {
    tunnel.get("peer_id")
    for tunnel in data.get("active_tunnels") or []
    if tunnel.get("peer_role") == "exit_bridge"
}
missing_tunnels = sorted(seen_bridge_ids - tunnel_bridge_ids)
if missing_tunnels:
    fail("missing_active_tunnels", bridge_ids=missing_tunnels)

print(json.dumps({
    "state": state,
    "actor_id": data.get("actor_id"),
    "publisher_node_id": publisher.get("node_id"),
    "publisher_encryption_key_present": True,
    "host_creator_id": host_creator.get("node_id"),
    "creator_id": creator.get("node_id"),
    "bootstrap_session_id": session.get("session_id"),
    "bridge_count": len(bridges),
    "eligible_bridge_ids": eligible,
    "active_tunnel_bridge_ids": sorted(tunnel_bridge_ids),
}, sort_keys=True))
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

  echo "creator-new is not ready for Smoke 4; running Smoke 2 bootstrap fallback..."
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
    --min-active-bridges "$EXPECTED_BRIDGES" \
    "$obs_arg"

  smoke_discover_nodes
  smoke_admin_curl "$CREATOR_NEW_POD" creator-runner GET /v1/admin/local-dht \
    >"$ARTIFACT_DIR/creator-local-dht-before.json"
  local_dht_ready "$ARTIFACT_DIR/creator-local-dht-before.json" \
    >"$ARTIFACT_DIR/creator-local-dht-ready-summary.json" ||
    smoke_fail "creator-new did not have complete bootstrap local-DHT state: Publisher encryption key, HostCreator entry, Creator entry, bootstrap session, and $EXPECTED_BRIDGES active bridge entries are required."
}

build_payload() {
  local size="$1" chunk_size="$2"
  SYNTHETIC_SIZE_VALUE="$size" CHUNK_SIZE_VALUE="$chunk_size" PLAINTEXT_MARKER="$PLAINTEXT_MARKER" python3 - <<'PY'
import json
import os
print(json.dumps({
    "input_source": "synthetic",
    "synthetic_size_bytes": int(os.environ["SYNTHETIC_SIZE_VALUE"]),
    "synthetic_marker": os.environ["PLAINTEXT_MARKER"],
    "chunk_size_bytes": int(os.environ["CHUNK_SIZE_VALUE"]),
    "sanitization_profile": "v3-default-no-visual-anon",
}, separators=(",", ":")))
PY
}

send_payload() {
  local session_id="$1" force_bridge_id="${2:-}"
  SESSION_ID="$session_id" TARGET_LANE_COUNT="$TARGET_LANE_COUNT" UPLOAD_TIMEOUT_SECONDS="$UPLOAD_TIMEOUT_SECONDS" FORCE_BRIDGE_ID="$force_bridge_id" python3 - <<'PY'
import json
import os
timeout_ms = int(os.environ["UPLOAD_TIMEOUT_SECONDS"]) * 1000
payload = {
    "session_id": os.environ["SESSION_ID"],
    "target_lane_count": int(os.environ["TARGET_LANE_COUNT"]),
    "lane_open_timeout_ms": timeout_ms,
    "chunk_ack_timeout_ms": timeout_ms,
}
if os.environ.get("FORCE_BRIDGE_ID"):
    payload["force_lane_failure"] = [os.environ["FORCE_BRIDGE_ID"]]
print(json.dumps(payload, separators=(",", ":")))
PY
}

assert_build_result() {
  local label="$1" chain_id="$2" expected_chunks="$3" path="$4"
  python3 - "$label" "$chain_id" "$expected_chunks" "$path" <<'PY'
import json
import sys
label, chain_id, expected_raw, path = sys.argv[1:5]
expected = int(expected_raw)
data = json.load(open(path, encoding="utf-8"))
def fail(code, **detail):
    raise SystemExit(json.dumps({"label": label, "failure": code, **detail}, sort_keys=True))
if data.get("chain_id") != chain_id:
    fail("chain_id_mismatch", actual=data.get("chain_id"), expected=chain_id)
manifest = data.get("manifest") or {}
if int(manifest.get("total_chunks") or 0) != expected:
    fail("total_chunks_mismatch", actual=manifest.get("total_chunks"), expected=expected)
if not manifest.get("content_hash"):
    fail("missing_content_hash")
report = data.get("sanitization_report") or {}
if report.get("synthetic_marker_zeroed") is not True:
    fail("synthetic_marker_not_zeroed", report=report)
if data.get("ciphertext_only_at_bridge") is not True:
    fail("ciphertext_boundary_false")
PY
}

assert_send_result() {
  local label="$1" chain_id="$2" expected_chunks="$3" path="$4" force_bridge="${5:-}"
  python3 - "$label" "$chain_id" "$expected_chunks" "$path" "$force_bridge" "$UPLOAD_TIMEOUT_SECONDS" "$TARGET_LANE_COUNT" "$ARTIFACT_DIR/creator-local-dht-before.json" <<'PY'
import json
import sys
label, chain_id, expected_raw, path, force_bridge, timeout_raw, target_raw, dht_path = sys.argv[1:9]
expected = int(expected_raw)
timeout_ms = int(timeout_raw) * 1000
target_lanes = int(target_raw)
data = json.load(open(path, encoding="utf-8"))
dht = json.load(open(dht_path, encoding="utf-8"))
def fail(code, **detail):
    raise SystemExit(json.dumps({"label": label, "failure": code, **detail}, sort_keys=True))
expected_bridge_ids = {
    entry.get("bridge_id")
    for entry in dht.get("bridge_entries") or []
    if entry.get("active") and entry.get("reachability_class") != "relay_only"
}
lanes_used = set(data.get("lanes_used") or [])
if data.get("chain_id") != chain_id:
    fail("chain_id_mismatch", actual=data.get("chain_id"), expected=chain_id)
if str(data.get("session_status", "")).lower() != "completed":
    fail("session_not_completed", status=data.get("session_status"))
if int(data.get("total_chunks") or 0) != expected:
    fail("total_chunks_mismatch", actual=data.get("total_chunks"), expected=expected)
if int(data.get("completed_chunks") or 0) != expected:
    fail("completed_chunks_mismatch", actual=data.get("completed_chunks"), expected=expected)
if data.get("failed_chunks"):
    fail("failed_chunks_present", failed_chunks=data.get("failed_chunks"))
if label == "normal":
    if len(lanes_used) != target_lanes:
        fail("normal_lane_count_mismatch", lanes=sorted(lanes_used), actual=len(lanes_used), expected=target_lanes)
    if lanes_used != expected_bridge_ids:
        fail("normal_lanes_do_not_match_local_dht", lanes=sorted(lanes_used), expected=sorted(expected_bridge_ids))
    if int(data.get("lane_count_at_completion") or 0) != target_lanes:
        fail("normal_completion_lane_count_mismatch", actual=data.get("lane_count_at_completion"), expected=target_lanes)
else:
    if force_bridge and force_bridge in lanes_used:
        fail("forced_bridge_carried_chunks", forced=force_bridge, lanes=sorted(lanes_used))
    minimum = min(expected, max(2, target_lanes - 1))
    if len(lanes_used) < minimum:
        fail("failover_lane_count_too_low", lanes=sorted(lanes_used), actual=len(lanes_used), expected_minimum=minimum)
if data.get("ciphertext_only_at_bridge") is not True:
    fail("ciphertext_boundary_false")
first = data.get("first_chunk_dispatched_at_ms")
active = data.get("all_lanes_active_at_ms")
if not isinstance(first, int) or not isinstance(active, int) or first >= active:
    fail("progressive_fanout_missing", first=first, all_lanes_active=active)
if int(data.get("elapsed_ms") or 0) >= timeout_ms:
    fail("elapsed_timeout", elapsed_ms=data.get("elapsed_ms"), timeout_ms=timeout_ms)
if force_bridge:
    if force_bridge not in (data.get("force_lane_failure_used") or []):
        fail("forced_bridge_not_recorded", forced=force_bridge, recorded=data.get("force_lane_failure_used"))
    if int(data.get("failover_events") or 0) < 1:
        fail("missing_failover_event_count")
else:
    if data.get("force_lane_failure_used"):
        fail("unexpected_force_lane_failure", recorded=data.get("force_lane_failure_used"))
PY
}

assert_dispatch_plan() {
  local label="$1" expected_chunks="$2" path="$3" force_bridge="${4:-}"
  python3 - "$label" "$expected_chunks" "$path" "$force_bridge" "$TARGET_LANE_COUNT" "$ARTIFACT_DIR/creator-local-dht-before.json" <<'PY'
import json
import sys
label, expected_raw, path, force_bridge, target_raw, dht_path = sys.argv[1:7]
expected = int(expected_raw)
target_lanes = int(target_raw)
plan = json.load(open(path, encoding="utf-8"))
dht = json.load(open(dht_path, encoding="utf-8"))
assignments = plan.get("chunk_assignments") or []
def fail(code, **detail):
    raise SystemExit(json.dumps({"label": label, "failure": code, **detail}, sort_keys=True))
expected_bridge_ids = {
    entry.get("bridge_id")
    for entry in dht.get("bridge_entries") or []
    if entry.get("active") and entry.get("reachability_class") != "relay_only"
}
if int(plan.get("target_lane_count") or 0) != target_lanes:
    fail("plan_target_lane_count_mismatch", actual=plan.get("target_lane_count"), expected=target_lanes)
if str(plan.get("session_status", "")).lower() != "completed":
    fail("plan_not_completed", status=plan.get("session_status"))
if int(plan.get("completed_chunks") or 0) != expected:
    fail("plan_completed_mismatch", actual=plan.get("completed_chunks"), expected=expected)
if len(assignments) < expected:
    fail("assignment_count_too_low", actual=len(assignments), expected=expected)
acked_bridges = {a.get("assigned_bridge_id") for a in assignments if a.get("ack_at_ms") is not None}
if label == "normal":
    if len(acked_bridges) != target_lanes:
        fail("plan_normal_lane_count_mismatch", actual=len(acked_bridges), expected=target_lanes, bridges=sorted(acked_bridges))
    if acked_bridges != expected_bridge_ids:
        fail("plan_normal_lanes_do_not_match_local_dht", bridges=sorted(acked_bridges), expected=sorted(expected_bridge_ids))
else:
    if force_bridge and force_bridge in acked_bridges:
        fail("plan_forced_bridge_carried_chunks", forced=force_bridge, bridges=sorted(acked_bridges))
    minimum = min(expected, max(2, target_lanes - 1))
    if len(acked_bridges) < minimum:
        fail("plan_failover_lane_count_too_low", actual=len(acked_bridges), expected_minimum=minimum, bridges=sorted(acked_bridges))
if force_bridge:
    if force_bridge not in (plan.get("force_lane_failure_used") or []):
        fail("forced_bridge_not_recorded", forced=force_bridge, recorded=plan.get("force_lane_failure_used"))
    if int(plan.get("failover_events") or 0) < 1:
        fail("missing_failover_event_count")
PY
}

wait_received_summary() {
  local label="$1" session_id="$2" expected_chunks="$3" output="$4"
  local deadline=$((SECONDS + TRACE_TIMEOUT_SECONDS))
  while ((SECONDS <= deadline)); do
    if smoke_received_upload_session "$session_id" "$output"; then
      if python3 - "$label" "$expected_chunks" "$output" <<'PY'
import json
import sys
label, expected_raw, path = sys.argv[1:4]
expected = int(expected_raw)
data = json.load(open(path, encoding="utf-8"))
if data.get("manifest_received") is not True:
    raise SystemExit(1)
if int(data.get("chunks_received") or 0) != expected:
    raise SystemExit(1)
if data.get("content_hash_match") is not True:
    raise SystemExit(1)
if data.get("synthetic_marker_zeroed_at_start") is not True:
    raise SystemExit(1)
if data.get("decrypt_errors"):
    raise SystemExit(1)
PY
      then
        return 0
      fi
    fi
    sleep 2
  done
  smoke_fail "$label receiver summary did not reach manifest+chunk reconstruction success."
}

collect_upload_logs() {
  kubectl -n "$NAMESPACE" logs -l veritas-role=creator --all-containers --tail=12000 \
    >"$ARTIFACT_DIR/kubectl-logs/creator.log" 2>&1 || true
  kubectl -n "$NAMESPACE" logs -l veritas-role=bridge --all-containers --tail=20000 \
    >"$ARTIFACT_DIR/kubectl-logs/bridge.log" 2>&1 || true
  kubectl -n "$NAMESPACE" logs -l veritas-role=authority --all-containers --tail=12000 \
    >"$ARTIFACT_DIR/kubectl-logs/authority.log" 2>&1 || true
  kubectl -n "$NAMESPACE" logs -l veritas-role=receiver --all-containers --tail=8000 \
    >"$ARTIFACT_DIR/kubectl-logs/receiver.log" 2>&1 || true
}

assert_trace_events() {
  local label="$1" chain_id="$2" session_id="$3" require_failover="$4"
  python3 - "$ARTIFACT_DIR" "$label" "$chain_id" "$session_id" "$require_failover" "$PLAINTEXT_MARKER" <<'PY'
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
label, chain_id, session_id, require_failover, marker = sys.argv[2:7]
logs = []
for name in ["creator.log", "bridge.log", "authority.log", "receiver.log"]:
    path = root / "kubectl-logs" / name
    if path.exists():
        logs.append(path.read_text(errors="ignore"))
text = "\n".join(logs)
required = [
    "creator_upload_session_built",
    "creator_upload_lanes_selected",
    "creator_upload_lane_open",
    "creator_upload_chunk_encrypted",
    "creator_upload_chunk_dispatched",
    "creator_upload_lane_reused",
    "bridge_upload_chunk_forwarded",
    "receiver_upload_chunk_ingested",
    "receiver_upload_manifest_received",
    "publisher_upload_chunk_ack_returned",
    "creator_upload_session_complete",
    "bridge_upload_frame_fragment_received",
    "bridge_upload_frame_reassembled",
]
if require_failover == "true":
    required.append("creator_upload_lane_failover")
missing = []
for event in required:
    if not any(event in line and chain_id in line and session_id in line for line in text.splitlines()):
        missing.append(event)
if missing:
    raise SystemExit(f"{label} missing trace/log events for chain_id={chain_id} session_id={session_id}: {missing}")
bridge_log = (root / "kubectl-logs" / "bridge.log").read_text(errors="ignore")
if marker in bridge_log:
    raise SystemExit(f"{label} plaintext marker appeared in bridge logs")
PY
}

wait_authority_frames() {
  local label="$1" chain_id="$2" expected_min="$3" output="$4"
  local deadline=$((SECONDS + TRACE_TIMEOUT_SECONDS))
  while ((SECONDS <= deadline)); do
    smoke_frames_by_chain_id "$chain_id" "$output" "$((expected_min + 5))"
    local count
    count="$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1], encoding="utf-8")).get("frames", [])))' "$output")"
    if [[ "$count" -ge "$expected_min" ]]; then
      return 0
    fi
    sleep 2
  done
  smoke_fail "$label authority frames did not reach $expected_min for chain_id=$chain_id."
}

admin_once_to_file() {
  local output_path="$1"
  shift
  local tmp="${output_path}.tmp" output status
  output="$(smoke_admin_curl_try_once "$@" 2>&1)" && status=0 || status=$?
  printf '%s' "$output" >"$tmp"
  if [[ "$status" -eq 0 ]]; then
    mv "$tmp" "$output_path"
    return 0
  fi
  mv "$tmp" "$output_path.error"
  return "$status"
}

maybe_wait_observability() {
  local label="$1" chain_id="$2"
  [[ "$REQUIRE_OBSERVABILITY" -eq 1 ]] || return 0
  smoke_wait_tempo_hits "$chain_id" "$ARTIFACT_DIR/tempo/${label}.json" 1 ||
    smoke_fail "$label did not produce Tempo hits for chain_id=$chain_id."
  smoke_wait_loki_hits "$chain_id" "$ARTIFACT_DIR/loki/${label}.json" 1 ||
    smoke_fail "$label did not produce Loki hits for chain_id=$chain_id."
}

restart_creator_and_assert_persistence() {
  local session_id="$1"
  echo "Restarting creator-new to validate upload session persistence..."
  kubectl -n "$NAMESPACE" delete pod "$CREATOR_NEW_POD" --wait=false >/dev/null
  kubectl -n "$NAMESPACE" rollout status deployment/creator-new --timeout=180s >/dev/null ||
    smoke_fail "creator-new rollout did not recover after pod restart."
  smoke_discover_nodes
  local deadline=$((SECONDS + 90))
  while ((SECONDS <= deadline)); do
    smoke_admin_curl "$CREATOR_NEW_POD" creator-runner GET /v1/admin/upload-sessions \
      >"$ARTIFACT_DIR/upload-sessions-after-restart.json"
    if python3 - "$ARTIFACT_DIR/upload-sessions-after-restart.json" "$session_id" <<'PY'
import json
import sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
session_id = sys.argv[2]
for session in data.get("sessions") or []:
    if session.get("session_id") == session_id and str(session.get("status", "")).lower() == "completed":
        raise SystemExit(0)
raise SystemExit(1)
PY
    then
      break
    fi
    sleep 3
  done
  python3 - "$ARTIFACT_DIR/upload-sessions-after-restart.json" "$session_id" <<'PY'
import json
import sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
session_id = sys.argv[2]
if not any(s.get("session_id") == session_id and str(s.get("status", "")).lower() == "completed" for s in data.get("sessions") or []):
    raise SystemExit(f"session {session_id} was not completed after creator restart")
PY
  kubectl -n "$NAMESPACE" exec "$CREATOR_NEW_POD" -c creator-runner -- \
    sh -lc 'ls -1 /var/lib/gbn-conduit/upload_sessions' \
    >"$ARTIFACT_DIR/upload-session-dir-after-restart.txt" 2>&1 || true
  grep -F "$session_id" "$ARTIFACT_DIR/upload-session-dir-after-restart.txt" >/dev/null ||
    smoke_fail "creator-new upload session directory did not contain $session_id after restart."
}

run_upload_case_once() {
  local label="$1" chain_id="$2" size="$3" force_bridge="${4:-}"
  local expected_chunks=$(((size + CHUNK_SIZE - 1) / CHUNK_SIZE))
  local build_body send_body session_id
  local lanes=()
  if [[ "$label" == "normal" && "$expected_chunks" -lt "$TARGET_LANE_COUNT" ]]; then
    smoke_fail "Smoke 4 normal upload needs at least $TARGET_LANE_COUNT chunks so every ExitBridge can carry a chunk; got $expected_chunks chunks from size=$size chunk_size=$CHUNK_SIZE."
  fi

  echo "Collecting ${label} pre-send DHT evidence for chain_id=${chain_id}..."
  smoke_collect_dht_evidence "$chain_id" "${label}-pre-send"

  build_body="$(build_payload "$size" "$CHUNK_SIZE")"
  printf '%s\n' "$build_body" >"$ARTIFACT_DIR/build-${label}-payload.json"
  admin_once_to_file "$ARTIFACT_DIR/build-${label}-result.json" \
    "$CREATOR_NEW_POD" creator-runner POST \
    "/v1/admin/build-upload-session?chain_id=${chain_id}" "$build_body" ||
    return 1
  assert_build_result "$label" "$chain_id" "$expected_chunks" "$ARTIFACT_DIR/build-${label}-result.json"
  session_id="$(json_file_field "$ARTIFACT_DIR/build-${label}-result.json" session_id)"

  send_body="$(send_payload "$session_id" "$force_bridge")"
  printf '%s\n' "$send_body" >"$ARTIFACT_DIR/send-${label}-payload.json"
  admin_once_to_file "$ARTIFACT_DIR/send-${label}-result.json" \
    "$CREATOR_NEW_POD" creator-runner POST \
    "/v1/admin/send-upload?chain_id=${chain_id}" "$send_body" ||
    return 1
  assert_send_result "$label" "$chain_id" "$expected_chunks" "$ARTIFACT_DIR/send-${label}-result.json" "$force_bridge"

  smoke_creator_upload_dispatch_plan "$session_id" "$ARTIFACT_DIR/dispatch-plan-${label}.json"
  assert_dispatch_plan "$label" "$expected_chunks" "$ARTIFACT_DIR/dispatch-plan-${label}.json" "$force_bridge"

  wait_authority_frames "$label" "$chain_id" "$((expected_chunks + 1))" "$ARTIFACT_DIR/frames-${label}.json"
  wait_received_summary "$label" "$session_id" "$expected_chunks" "$ARTIFACT_DIR/receiver-session-summary-${label}.json"
  maybe_wait_observability "$label" "$chain_id"
  mapfile -t lanes < <(python3 - "$ARTIFACT_DIR/send-${label}-result.json" <<'PY'
import json
import sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
for lane in data.get("lanes_used") or []:
    print(lane)
PY
)
  smoke_collect_chainid_log_evidence "$chain_id" "$label" "${lanes[@]}"
  printf '%s\n' "$session_id" >"$ARTIFACT_DIR/session-id-${label}.txt"
}

run_upload_case() {
  local label="$1" chain_id="$2" size="$3" force_bridge="${4:-}"
  local attempt attempt_chain
  for ((attempt = 1; attempt <= UPLOAD_CASE_ATTEMPTS; attempt++)); do
    attempt_chain="$chain_id"
    if ((attempt > 1)); then
      attempt_chain="${chain_id}-retry-${attempt}"
    fi
    echo "Running Smoke 4 ${label} upload attempt ${attempt}/${UPLOAD_CASE_ATTEMPTS} chain_id=${attempt_chain}"
    smoke_ensure_cluster_api
    smoke_check_rollouts
    smoke_discover_nodes
    ensure_creator_onboarded
    if run_upload_case_once "$label" "$attempt_chain" "$size" "$force_bridge"; then
      printf '%s\n' "$attempt_chain" >"$ARTIFACT_DIR/chain-id-${label}.txt"
      return 0
    fi
    echo "Smoke 4 ${label} upload attempt ${attempt} failed before assertions; rebuilding a fresh session after cluster stabilization." >&2
    smoke_ensure_cluster_api || true
    sleep 10
  done
  smoke_fail "Smoke 4 ${label} upload did not complete after ${UPLOAD_CASE_ATTEMPTS} fresh-session attempt(s)."
}

smoke_ensure_cluster_api
smoke_check_rollouts
smoke_discover_nodes
smoke_wait_for_bridge_registry
ensure_creator_onboarded

if [[ "$REQUIRE_OBSERVABILITY" -eq 1 ]]; then
  smoke_start_observability
fi

CHAIN_ID_NORMAL_BASE="${CHAIN_ID_PREFIX}normal-$(rand_suffix)"
run_upload_case normal "$CHAIN_ID_NORMAL_BASE" "$SYNTHETIC_SIZE"
CHAIN_ID_NORMAL="$(cat "$ARTIFACT_DIR/chain-id-normal.txt")"
SESSION_ID_NORMAL="$(cat "$ARTIFACT_DIR/session-id-normal.txt")"

if [[ "$INCLUDE_FAILOVER" -eq 1 ]]; then
  FORCE_BRIDGE_ID="$(python3 - "$ARTIFACT_DIR/send-normal-result.json" <<'PY'
import json
import sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
lanes = data.get("lanes_used") or []
if not lanes:
    raise SystemExit("normal upload did not report lanes_used")
print(lanes[0])
PY
)"
  CHAIN_ID_FAILOVER_BASE="${CHAIN_ID_PREFIX}failover-$(rand_suffix)"
  run_upload_case failover "$CHAIN_ID_FAILOVER_BASE" "$FAILOVER_SYNTHETIC_SIZE" "$FORCE_BRIDGE_ID"
  CHAIN_ID_FAILOVER="$(cat "$ARTIFACT_DIR/chain-id-failover.txt")"
else
  printf '{"skipped":true}\n' >"$ARTIFACT_DIR/send-failover-result.json"
  printf '{"skipped":true}\n' >"$ARTIFACT_DIR/dispatch-plan-failover.json"
  printf '{"skipped":true}\n' >"$ARTIFACT_DIR/receiver-session-summary-failover.json"
  mkdir -p "$ARTIFACT_DIR/chainid-evidence/failover"
  printf '{"skipped":true}\n' >"$ARTIFACT_DIR/chainid-evidence/failover/chainid-summary.json"
fi

sleep 5
collect_upload_logs
assert_trace_events normal "$CHAIN_ID_NORMAL" "$SESSION_ID_NORMAL" false
if [[ "$INCLUDE_FAILOVER" -eq 1 ]]; then
  SESSION_ID_FAILOVER="$(cat "$ARTIFACT_DIR/session-id-failover.txt")"
  assert_trace_events failover "$CHAIN_ID_FAILOVER" "$SESSION_ID_FAILOVER" true
fi

if [[ "$INCLUDE_PERSISTENCE_CHECK" -eq 1 ]]; then
  restart_creator_and_assert_persistence "$SESSION_ID_NORMAL"
fi

kubectl -n "$NAMESPACE" get pods -o wide >"$ARTIFACT_DIR/pods.txt"
kubectl -n "$NAMESPACE" get events --sort-by=.lastTimestamp --request-timeout=15s >"$ARTIFACT_DIR/events.txt" || true

python3 - "$ARTIFACT_DIR" "$INCLUDE_FAILOVER" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
include_failover = sys.argv[2] == "1"
rows = []
for label in ["normal"] + (["failover"] if include_failover else []):
    send = json.load(open(root / f"send-{label}-result.json", encoding="utf-8"))
    receiver = json.load(open(root / f"receiver-session-summary-{label}.json", encoding="utf-8"))
    rows.append(
        "| {label} | {session_id} | {lanes} | {completed}/{total} | {failover} | {hash_match} |".format(
            label=label,
            session_id=send.get("session_id"),
            lanes=",".join(send.get("lanes_used") or []),
            completed=send.get("completed_chunks"),
            total=send.get("total_chunks"),
            failover=",".join(send.get("force_lane_failure_used") or []),
            hash_match=receiver.get("content_hash_match"),
        )
    )
summary = [
    "# Smoke 4 Upload Summary",
    "",
    "| Invocation | Session | Lanes Used | Chunks | Forced Failover | Content Hash Match |",
    "|---|---|---:|---:|---|---|",
    *rows,
    "",
]
(root / "upload-summary.md").write_text("\n".join(summary), encoding="utf-8")
PY

python3 - "$ARTIFACT_DIR" "$INCLUDE_FAILOVER" "$EXPECTED_BRIDGES" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
include_failover = sys.argv[2] == "1"
expected_bridges = int(sys.argv[3])

def load(path, default=None):
    try:
        return json.load(open(path, encoding="utf-8"))
    except FileNotFoundError:
        return {} if default is None else default

def scalar(value):
    if isinstance(value, bool):
        return "true" if value else "false"
    if value is None:
        return ""
    if isinstance(value, list):
        return ", ".join(str(item) for item in value)
    return str(value)

def dht_rows(labels):
    rows = []
    for label in labels:
        dht = load(root / "dht-evidence" / f"{label}-pre-send" / "dht-summary.json")
        if not dht:
            continue
        rows.append(
            "| {label} | {publisher} | {creator} | {active} | {tunnels} | {entries} | {metadata} | {state} |".format(
                label=label,
                publisher=dht.get("publisher_dht_entry_count"),
                creator=dht.get("creator_new_dht_entry_count"),
                active=dht.get("creator_new_active_bridge_count"),
                tunnels=dht.get("creator_new_active_tunnel_count"),
                entries=dht.get("publisher_per_bridge_entry_count"),
                metadata=dht.get("bridge_metadata_count"),
                state=dht.get("new_creator_state"),
            )
        )
    return rows

labels = ["normal"] + (["failover"] if include_failover else [])
api_rows = []
chain_rows = []
lane_rows = []
for label in labels:
    build = load(root / f"build-{label}-result.json")
    send = load(root / f"send-{label}-result.json")
    dispatch = load(root / f"dispatch-plan-{label}.json")
    receiver = load(root / f"receiver-session-summary-{label}.json")
    chain = load(root / "chainid-evidence" / label / "chainid-summary.json")
    bridge_lines = chain.get("bridge_log_lines") or {}
    lanes_used = send.get("lanes_used") or []
    lane_rows.append(
        "| {label} | {count}/{expected} | {lanes} | {forced} |".format(
            label=label,
            count=len(lanes_used),
            expected=expected_bridges if label == "normal" else f">={max(2, expected_bridges - 1)}",
            lanes=", ".join(lanes_used),
            forced=", ".join(send.get("force_lane_failure_used") or []),
        )
    )
    api_rows.append(
        "| {label} | {chain_id} | {session} | {built} | {sent} | {dispatch_chunks} | {received} | {hash_match} |".format(
            label=label,
            chain_id=send.get("chain_id") or build.get("chain_id"),
            session=send.get("session_id") or build.get("session_id"),
            built=build.get("total_chunks"),
            sent=send.get("completed_chunks"),
            dispatch_chunks=dispatch.get("completed_chunks"),
            received=receiver.get("total_chunks"),
            hash_match=receiver.get("content_hash_match"),
        )
    )
    chain_rows.append(
        "| {label} | {chain_id} | {creator} | {publisher} | {bridges} | {bridge_count} |".format(
            label=label,
            chain_id=chain.get("chain_id") or send.get("chain_id"),
            creator=chain.get("creator_new_log_lines"),
            publisher=(chain.get("publisher_authority_log_lines") or 0) + (chain.get("publisher_receiver_log_lines") or 0),
            bridges=sum(bridge_lines.values()) if bridge_lines else 0,
            bridge_count=len([count for count in bridge_lines.values() if count > 0]),
        )
    )

normal_dht = load(root / "dht-evidence" / "normal-pre-send" / "dht-summary.json")
bridge_ids = normal_dht.get("publisher_bridge_ids") or []
report = [
    "# Conduit Smoke 4 Detailed Evidence Report",
    "",
    "## DHT Evidence",
    "",
    "| Invocation | Publisher DHT | NewCreator DHT | Active Bridges | Active Tunnels | Publisher Per-Bridge Entries | ExitBridge Metadata | Creator State |",
    "|---|---:|---:|---:|---:|---:|---:|---|",
    *dht_rows(labels),
    "",
    f"- Publisher/NewCreator bridge IDs: `{scalar(bridge_ids)}`",
    "- Raw DHT artifacts: `dht-evidence/<invocation>-pre-send/`.",
    "",
    "## API Completion And Reconstruction Evidence",
    "",
    "| Invocation | ChainID | Session | Built Chunks | Sent Chunks | Dispatch Chunks | Receiver Chunks | Content Hash Match |",
    "|---|---|---|---:|---:|---:|---:|---:|",
    *api_rows,
    "",
    "API artifacts: `build-*-result.json`, `send-*-result.json`, `dispatch-plan-*.json`, `frames-*.json`, and `receiver-session-summary-*.json`.",
    "",
    "## Lane Fanout Evidence",
    "",
    "| Invocation | Lanes Used | Lane IDs | Forced Failure |",
    "|---|---:|---|---|",
    *lane_rows,
    "",
    "The normal invocation must use all 10 ExitBridges; failover must exclude the forced bridge and still complete reconstruction.",
    "",
    "## ChainID Evidence",
    "",
    "| Invocation | ChainID | Creator Lines | Publisher Lines | ExitBridge Lines | ExitBridges With Evidence |",
    "|---|---|---:|---:|---:|---:|",
    *chain_rows,
    "",
    "Raw ChainID artifacts: `chainid-evidence/` plus `upload-logs/`.",
    "",
    "## Result",
    "",
    "Smoke 4 passed only after DHT snapshots, API completions, 10-bridge fanout/reconstruction evidence, and ChainID evidence were collected.",
    "",
]
(root / "report.md").write_text("\n".join(report), encoding="utf-8")
PY

echo "Smoke 4 upload validation passed. Artifacts: $ARTIFACT_DIR"
echo "Detailed evidence report: $ARTIFACT_DIR/report.md"
