#!/usr/bin/env bash
# Pass 3 Smoke 2: validate architecture-correct first-time creator bootup.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
OBS_NS="${VERITAS_OBS_NAMESPACE:-observability}"
EXPECTED_BRIDGES="${VERITAS_K8S_EXPECTED_BRIDGES:-10}"
ADMIN_PORT="${VERITAS_K8S_ADMIN_PORT:-9090}"
BOOTSTRAP_TIMEOUT_SECONDS="${VERITAS_K8S_SMOKE_BOOTSTRAP_TIMEOUT:-120}"
TRACE_TIMEOUT_SECONDS="${VERITAS_K8S_SMOKE_TRACE_TIMEOUT:-300}"
MIN_ACTIVE_BRIDGES="${VERITAS_K8S_SMOKE_MIN_ACTIVE_BRIDGES:-5}"
ALLOW_FANOUT_PARTIAL=0
REQUIRE_OBSERVABILITY=1
CHAIN_ID_PREFIX="${VERITAS_K8S_SMOKE_CHAIN_PREFIX:-smoke-2-}"
ARTIFACT_DIR=""

usage() {
  cat <<'EOF'
Usage: k8s-smoke-discovery-v3.sh [options]

Options:
  --namespace NAME                    Kubernetes namespace for Conduit pods.
  --observability-namespace NAME      Kubernetes namespace for Prometheus/Loki/Tempo.
  --expected-bridges N                Expected exit-bridge pod count. Default: 10.
  --bootstrap-timeout N               Seconds to wait for terminal creator bootup. Default: 120.
  --trace-timeout N                   Seconds to wait for Tempo event evidence. Default: 300.
  --min-active-bridges N              Minimum active bridges for allowed fanout_partial. Default: 5.
  --allow-fanout-partial              Accept fanout_partial when active bridge count is high enough.
  --require-observability             Require Tempo evidence. Default.
  --no-require-observability          Skip Tempo event assertions.
  --chain-id-prefix PREFIX            Prefix for the generated smoke ChainID.
  --artifact-dir DIR                  Artifact output directory.
  -h, --help                          Show this help.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --namespace) NAMESPACE="$2"; shift 2 ;;
    --observability-namespace) OBS_NS="$2"; shift 2 ;;
    --expected-bridges) EXPECTED_BRIDGES="$2"; shift 2 ;;
    --bootstrap-timeout) BOOTSTRAP_TIMEOUT_SECONDS="$2"; shift 2 ;;
    --trace-timeout) TRACE_TIMEOUT_SECONDS="$2"; shift 2 ;;
    --min-active-bridges) MIN_ACTIVE_BRIDGES="$2"; shift 2 ;;
    --allow-fanout-partial) ALLOW_FANOUT_PARTIAL=1; shift ;;
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
smoke_artifact_dir smoke-2-discovery >/dev/null
trap 'status=$?; if [[ $status -ne 0 ]]; then smoke_collect_diagnostics; fi; smoke_stop_observability; echo "Artifacts: $ARTIFACT_DIR"; exit $status' EXIT

mkdir -p "$ARTIFACT_DIR/tempo" "$ARTIFACT_DIR/loki" "$ARTIFACT_DIR/pod-logs" "$ARTIFACT_DIR/publisher-dht"
STEP_RESULTS="$ARTIFACT_DIR/step-results.jsonl"
: >"$STEP_RESULTS"

json_string() {
  python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$1"
}

json_field_from_arg() {
  local raw="$1" field="$2"
  python3 -c 'import json,sys; print(json.loads(sys.argv[1]).get(sys.argv[2], ""))' "$raw" "$field"
}

json_field_from_file() {
  local path="$1" field="$2"
  python3 -c 'import json,sys; print(json.load(open(sys.argv[1], encoding="utf-8")).get(sys.argv[2], ""))' "$path" "$field"
}

actor_id_from_metadata() {
  python3 -c 'import json,sys
data=json.loads(sys.argv[1])
print(data.get("conduit_actor") or data.get("node_id") or "")' "$1"
}

bridge_id_from_metadata() {
  python3 -c 'import json,sys
data=json.loads(sys.argv[1])
print(data.get("conduit_actor") or data.get("node_id") or "")' "$1"
}

now_ms() {
  python3 -c 'import time; print(int(time.time() * 1000))'
}

write_json_arg() {
  local raw="$1" path="$2"
  python3 -c 'import json,sys; json.dump(json.loads(sys.argv[1]), open(sys.argv[2], "w", encoding="utf-8"), indent=2, sort_keys=True); print()' "$raw" "$path" >/dev/null
}

record_step() {
  local name="$1" endpoint="$2" expected="$3" observed="$4" artifact="$5"
  python3 - "$STEP_RESULTS" "$name" "$endpoint" "$expected" "$observed" "$artifact" <<'PY'
import json
import sys
import time

path, name, endpoint, expected, observed, artifact = sys.argv[1:7]
with open(path, "a", encoding="utf-8") as handle:
    handle.write(json.dumps({
        "recorded_at_ms": int(time.time() * 1000),
        "step": name,
        "endpoint": endpoint,
        "expected": expected,
        "observed": observed,
        "artifact": artifact,
        "status": "pass",
    }, sort_keys=True) + "\n")
PY
}

build_seed_host_payload() {
  local host_meta="$1" authority_meta="$2" receiver_meta="$3" bridge_entry="$4" expiry_ms="$5"
  HOST_METADATA="$host_meta" \
    AUTHORITY_METADATA="$authority_meta" \
    RECEIVER_METADATA="$receiver_meta" \
    BRIDGE_ENTRY="$bridge_entry" \
    ENTRY_EXPIRY_MS="$expiry_ms" \
    python3 - <<'PY'
import json
import os

host = json.loads(os.environ["HOST_METADATA"])
authority = json.loads(os.environ["AUTHORITY_METADATA"])
receiver = json.loads(os.environ["RECEIVER_METADATA"])
bridge_entry = json.loads(os.environ["BRIDGE_ENTRY"])

pub_hex = authority.get("publisher_public_key") or authority.get("public_key")
if not pub_hex:
    raise SystemExit("authority metadata did not include publisher_public_key")
pub_hex = pub_hex.removeprefix("0x")
enc_hex = (authority.get("publisher_encryption_public_key") or "").removeprefix("0x")

publisher_entry = {
    "node_id": authority.get("node_id") or "publisher-authority",
    "authority_url": "http://publisher-authority:8080",
    "receiver_url": "http://publisher-receiver:8081",
    "pub_key": [int(pub_hex[i:i + 2], 16) for i in range(0, len(pub_hex), 2)],
    "entry_expiry_ms": int(os.environ["ENTRY_EXPIRY_MS"]),
}
if enc_hex:
    if len(enc_hex) % 2:
        raise SystemExit("publisher encryption public key hex has odd length")
    publisher_entry["encryption_pub_key"] = [
        int(enc_hex[i:i + 2], 16) for i in range(0, len(enc_hex), 2)
    ]
payload = {
    "host_creator_id": host.get("conduit_actor") or host.get("node_id"),
    "publisher_entry": publisher_entry,
    "exit_bridge_a_entry": bridge_entry,
    "bootstrap_genesis": True,
    "force": True,
}
print(json.dumps(payload, separators=(",", ":")))
PY
}

build_creator_dht_sign_payload() {
  local metadata="$1" fallback_ip="$2" expiry_ms="$3"
  HOST_METADATA="$metadata" FALLBACK_IP="$fallback_ip" ENTRY_EXPIRY_MS="$expiry_ms" python3 - <<'PY'
import json
import os

metadata = json.loads(os.environ["HOST_METADATA"])
pub_hex = metadata.get("public_key")
if not pub_hex:
    raise SystemExit("creator metadata did not include public_key")
pub_hex = pub_hex.removeprefix("0x")

payload = {
    "creator": {
        "node_id": metadata.get("conduit_actor") or metadata.get("node_id"),
        "ip_addr": metadata.get("ip_addr") or os.environ["FALLBACK_IP"],
        "pub_key": [int(pub_hex[i:i + 2], 16) for i in range(0, len(pub_hex), 2)],
        "udp_punch_port": int(metadata.get("creator_udp_punch_port") or 443),
        "entry_expiry_ms": int(os.environ["ENTRY_EXPIRY_MS"]),
    },
    "active": True,
}
print(json.dumps(payload, separators=(",", ":")))
PY
}

build_seed_new_payload() {
  local new_meta="$1" host_entry="$2" host_admin_url="$3"
  NEW_METADATA="$new_meta" HOST_ENTRY="$host_entry" HOST_ADMIN_URL="$host_admin_url" python3 - <<'PY'
import json
import os

metadata = json.loads(os.environ["NEW_METADATA"])
payload = {
    "new_creator_id": metadata.get("conduit_actor") or metadata.get("node_id"),
    "host_creator_entry": json.loads(os.environ["HOST_ENTRY"]),
    "start_bootstrap": True,
    "force": True,
    "host_admin_url": os.environ["HOST_ADMIN_URL"],
}
print(json.dumps(payload, separators=(",", ":")))
PY
}

local_dht_summary_from_arg() {
  python3 -c 'import json,sys
table=json.loads(sys.argv[1])
bridges=table.get("bridge_entries") or []
active=sum(1 for entry in bridges if entry.get("active"))
session=table.get("current_bootstrap_session") or {}
print("state={} bridges={} active={} chain_id={} bootstrap_session_id={}".format(
    table.get("self_onboarding_state", "unknown"),
    len(bridges),
    active,
    session.get("chain_id") or "",
    session.get("session_id") or "",
))' "$1"
}

reset_creator() {
  local pod="$1" chain_id="$2" output="$3" response
  response="$(smoke_admin_curl "$pod" creator-runner POST "/v1/admin/reset-creator-state?chain_id=${chain_id}" "{}")"
  write_json_arg "$response" "$output"
}

assert_creator_reset() {
  local pod="$1" output="$2" response
  response="$(smoke_admin_curl "$pod" creator-runner GET /v1/admin/local-dht)"
  write_json_arg "$response" "$output"
  python3 - "$output" <<'PY'
import json
import sys

path = sys.argv[1]
table = json.load(open(path, encoding="utf-8"))
if table.get("self_onboarding_state") != "none":
    raise SystemExit(f"creator state was not reset: {table.get('self_onboarding_state')}")
for key in ("publisher_entry", "creator_entry", "host_creator_entry", "current_bootstrap_session", "host_seed_state", "new_creator_seed_state"):
    if table.get(key) is not None:
        raise SystemExit(f"creator reset left {key} populated")
if table.get("bridge_entries") or table.get("active_tunnels"):
    raise SystemExit("creator reset left bridge entries or active tunnels populated")
PY
}

wait_for_terminal_local_dht() {
  local deadline response state summary
  deadline=$((SECONDS + BOOTSTRAP_TIMEOUT_SECONDS))
  : >"$ARTIFACT_DIR/local-dht-progression.jsonl"
  while ((SECONDS <= deadline)); do
    response="$(smoke_admin_curl "$CREATOR_NEW_POD" creator-runner GET /v1/admin/local-dht)"
    summary="$(local_dht_summary_from_arg "$response")"
    state="${summary#state=}"
    state="${state%% *}"
    python3 - "$response" "$summary" >>"$ARTIFACT_DIR/local-dht-progression.jsonl" <<'PY'
import json
import sys
import time

table = json.loads(sys.argv[1])
print(json.dumps({
    "observed_at_ms": int(time.time() * 1000),
    "summary": sys.argv[2],
    "table": table,
}, sort_keys=True))
PY
    echo "  $summary"
    case "$state" in
      onboarded|fanout_partial|fanout_failed|seed_tunnel_failed)
        printf '%s' "$response" >"$ARTIFACT_DIR/local-dht-final.json"
        return 0
        ;;
    esac
    sleep 2
  done
  printf '%s' "$response" >"$ARTIFACT_DIR/local-dht-final.json"
  return 1
}

dump_and_assert_publisher_dht() {
  local dump_path="$ARTIFACT_DIR/publisher-dht/publisher-dht.json"
  local per_entry_dir="$ARTIFACT_DIR/publisher-dht/per-entry"
  mkdir -p "$per_entry_dir"
  smoke_admin_curl "$AUTHORITY_POD" publisher-authority GET "/v1/admin/publisher-dht?chain_id=${CHAIN_ID}" \
    >"$dump_path"

  local bridge_id safe_id
  while IFS= read -r bridge_id; do
    [[ -n "$bridge_id" ]] || continue
    safe_id="$(printf '%s' "$bridge_id" | tr -c 'A-Za-z0-9_.-' '_')"
    smoke_admin_curl "$AUTHORITY_POD" publisher-authority GET "/v1/admin/bridges/${bridge_id}/dht-entry" \
      >"$per_entry_dir/${safe_id}.json"
  done < <(python3 -c 'import json,sys; print("\n".join(json.load(open(sys.argv[1], encoding="utf-8")).get("bridge_ids") or []))' "$dump_path")

  python3 - \
    "$dump_path" \
    "$ARTIFACT_DIR/initialize-publisher-dht-result.json" \
    "$ARTIFACT_DIR/deployed-bridge-ids.json" \
    "$per_entry_dir" \
    "$CHAIN_ID" \
    "$EXPECTED_BRIDGES" \
    "$ARTIFACT_DIR/failure-evidence.json" \
    "$ARTIFACT_DIR/publisher-dht/publisher-dht-summary.json" <<'PY'
import json
import sys
import time
from pathlib import Path

dump_path, init_path, ids_path, per_entry_dir, chain_id, expected_count, failure_path, summary_path = sys.argv[1:9]
expected_count = int(expected_count)
now_ms = int(time.time() * 1000)

def fail(code, **detail):
    detail["code"] = code
    json.dump(detail, open(failure_path, "w", encoding="utf-8"), indent=2, sort_keys=True)
    raise SystemExit(f"{code}: {detail}")

dump = json.load(open(dump_path, encoding="utf-8"))
init = json.load(open(init_path, encoding="utf-8"))
expected_ids = set(json.load(open(ids_path, encoding="utf-8")))
entries = dump.get("bridge_dht_entries") or []
ids = dump.get("bridge_ids") or [entry.get("bridge_id") for entry in entries]

if dump.get("chain_id") != chain_id:
    fail("publisher_dht_chain_mismatch", actual=dump.get("chain_id"), expected=chain_id)
if len(entries) != expected_count:
    fail("publisher_dht_entry_count_mismatch", actual=len(entries), expected=expected_count)
if dump.get("publisher_dht_entry_count") != expected_count:
    fail("publisher_dht_reported_count_mismatch", actual=dump.get("publisher_dht_entry_count"), expected=expected_count)
if set(ids) != expected_ids:
    fail("publisher_dht_id_set_mismatch", actual=sorted(ids), expected=sorted(expected_ids))
if set(init.get("bridge_ids") or []) != expected_ids:
    fail("publisher_dht_init_id_set_mismatch", actual=sorted(init.get("bridge_ids") or []), expected=sorted(expected_ids))

by_id = {}
for entry in entries:
    bridge_id = entry.get("bridge_id")
    by_id[bridge_id] = entry
    if not bridge_id:
        fail("publisher_dht_entry_missing_bridge_id")
    if not entry.get("active"):
        fail("publisher_dht_entry_inactive", bridge_id=bridge_id)
    if not entry.get("publisher_sig"):
        fail("publisher_dht_entry_missing_signature", bridge_id=bridge_id)
    if int(entry.get("lease_expiry_ms") or 0) <= now_ms:
        fail("publisher_dht_entry_lease_expired", bridge_id=bridge_id, lease_expiry_ms=entry.get("lease_expiry_ms"), now_ms=now_ms)
    if int(entry.get("entry_expiry_ms") or 0) <= now_ms:
        fail("publisher_dht_entry_expired", bridge_id=bridge_id, entry_expiry_ms=entry.get("entry_expiry_ms"), now_ms=now_ms)
    if entry.get("reachability_class") not in {"direct", "brokered"}:
        fail("publisher_dht_entry_bad_reachability", bridge_id=bridge_id, reachability_class=entry.get("reachability_class"))
    if not entry.get("ingress_endpoints"):
        fail("publisher_dht_entry_missing_ingress", bridge_id=bridge_id)
    if not entry.get("capabilities"):
        fail("publisher_dht_entry_missing_capabilities", bridge_id=bridge_id)

for bridge_id in sorted(expected_ids):
    safe_id = "".join(ch if ch.isalnum() or ch in "_.-" else "_" for ch in bridge_id)
    path = Path(per_entry_dir) / f"{safe_id}.json"
    if not path.exists():
        fail("publisher_dht_per_entry_missing_file", bridge_id=bridge_id)
    response = json.load(open(path, encoding="utf-8"))
    entry = response.get("bridge")
    if entry != by_id.get(bridge_id):
        fail("publisher_dht_per_entry_mismatch", bridge_id=bridge_id)

json.dump({
    "chain_id": chain_id,
    "publisher_dht_entry_count": len(entries),
    "publisher_bridge_ids": sorted(ids),
    "active_bridge_count": dump.get("active_bridge_count"),
    "per_entry_fetch_count": len(list(Path(per_entry_dir).glob("*.json"))),
}, open(summary_path, "w", encoding="utf-8"), indent=2, sort_keys=True)
PY
}

assert_bootstrap_state() {
  python3 - \
    "$ARTIFACT_DIR/local-dht-final.json" \
    "$ARTIFACT_DIR/bootstrap-session.json" \
    "$ARTIFACT_DIR/deployed-bridge-ids.json" \
    "$ARTIFACT_DIR/publisher-dht/publisher-dht.json" \
    "$CHAIN_ID" \
    "$BRIDGE_A_ID" \
    "$EXPECTED_BRIDGES" \
    "$MIN_ACTIVE_BRIDGES" \
    "$ALLOW_FANOUT_PARTIAL" \
    "$ARTIFACT_DIR/failure-evidence.json" \
    "$ARTIFACT_DIR/bootstrap-assertion-summary.json" <<'PY'
import json
import sys
import time

local_path, session_path, ids_path, publisher_path, chain_id, bridge_a_id, expected_count, min_active, allow_partial, failure_path, summary_path = sys.argv[1:12]
expected_count = int(expected_count)
min_active = int(min_active)
allow_partial = allow_partial == "1"
now_ms = int(time.time() * 1000)

def fail(code, **detail):
    detail["code"] = code
    json.dump(detail, open(failure_path, "w", encoding="utf-8"), indent=2, sort_keys=True)
    raise SystemExit(f"{code}: {detail}")

table = json.load(open(local_path, encoding="utf-8"))
session_response = json.load(open(session_path, encoding="utf-8"))
session = session_response.get("bootstrap_session") or {}
expected_ids = set(json.load(open(ids_path, encoding="utf-8")))
publisher_dump = json.load(open(publisher_path, encoding="utf-8"))
publisher_entries = publisher_dump.get("bridge_dht_entries") or []
publisher_ids = set(publisher_dump.get("bridge_ids") or [entry.get("bridge_id") for entry in publisher_entries])

if publisher_dump.get("chain_id") != chain_id:
    fail("publisher_dht_chain_mismatch", actual=publisher_dump.get("chain_id"), expected=chain_id)
if publisher_ids != expected_ids:
    fail("publisher_dht_expected_id_mismatch", actual=sorted(publisher_ids), expected=sorted(expected_ids))
if len(publisher_entries) != expected_count:
    fail("publisher_dht_entry_count_mismatch", actual=len(publisher_entries), expected=expected_count)

state = table.get("self_onboarding_state")
bridges = table.get("bridge_entries") or []
active = [entry for entry in bridges if entry.get("active")]
if state == "fanout_partial":
    if not allow_partial or len(active) < min_active:
        fail("terminal_state_not_accepted", state=state, active_bridge_count=len(active))
elif state != "onboarded":
    fail("terminal_state_not_onboarded", state=state)

creator = table.get("creator_entry")
if not creator:
    fail("missing_creator_entry")
if creator.get("node_id") != "new-creator":
    fail("creator_entry_wrong_node", node_id=creator.get("node_id"))
if not creator.get("publisher_sig"):
    fail("creator_entry_missing_signature")
if int(creator.get("entry_expiry_ms") or 0) <= now_ms:
    fail("creator_entry_expired", entry_expiry_ms=creator.get("entry_expiry_ms"), now_ms=now_ms)

if len(bridges) != expected_count:
    fail("bridge_count_mismatch", bridge_count=len(bridges), expected=expected_count)
if len(active) < (expected_count if state == "onboarded" else min_active):
    fail("active_bridge_count_mismatch", active_bridge_count=len(active), expected=expected_count)

seen_ids = set()
for entry in bridges:
    bridge_id = entry.get("bridge_id")
    seen_ids.add(bridge_id)
    if bridge_id not in expected_ids:
        fail("unexpected_bridge_id", bridge_id=bridge_id, expected=sorted(expected_ids))
    if not entry.get("publisher_sig"):
        fail("bridge_entry_missing_signature", bridge_id=bridge_id)
    if int(entry.get("lease_expiry_ms") or 0) <= now_ms:
        fail("bridge_lease_expired", bridge_id=bridge_id)
    if int(entry.get("entry_expiry_ms") or 0) <= now_ms:
        fail("bridge_entry_expired", bridge_id=bridge_id)
    if entry.get("reachability_class") not in {"direct", "brokered"}:
        fail("bridge_reachability_invalid", bridge_id=bridge_id, reachability_class=entry.get("reachability_class"))
    if not entry.get("ingress_endpoints"):
        fail("bridge_missing_ingress", bridge_id=bridge_id)
    if not entry.get("capabilities"):
        fail("bridge_missing_capabilities", bridge_id=bridge_id)

if seen_ids != expected_ids:
    fail("bridge_id_set_mismatch", seen=sorted(seen_ids), expected=sorted(expected_ids))
if seen_ids != publisher_ids:
    fail("creator_dht_publisher_dht_id_mismatch", creator=sorted(seen_ids), publisher=sorted(publisher_ids))

current = table.get("current_bootstrap_session") or {}
bootstrap_session_id = current.get("session_id")
if not bootstrap_session_id:
    fail("missing_current_bootstrap_session")
if current.get("chain_id") != chain_id:
    fail("current_session_chain_mismatch", actual=current.get("chain_id"), expected=chain_id)
if current.get("last_state") != state:
    fail("current_session_state_mismatch", actual=current.get("last_state"), expected=state)

if session.get("bootstrap_session_id") != bootstrap_session_id:
    fail("publisher_session_id_mismatch", publisher=session.get("bootstrap_session_id"), local=bootstrap_session_id)
if session.get("chain_id") != chain_id:
    fail("publisher_session_chain_mismatch", actual=session.get("chain_id"), expected=chain_id)

new_creator_id = (session.get("creator_entry") or {}).get("node_id")
host_creator_id = session.get("host_creator_id")
relay_bridge_id = session.get("relay_bridge_id")
seed_bridge_id = session.get("seed_bridge_id")
if new_creator_id != "new-creator":
    fail("new_creator_id_mismatch", actual=new_creator_id)
if host_creator_id != "host-creator":
    fail("host_creator_id_mismatch", actual=host_creator_id)
if relay_bridge_id != bridge_a_id:
    fail("relay_bridge_id_mismatch", actual=relay_bridge_id, expected=bridge_a_id)
if seed_bridge_id == relay_bridge_id:
    fail("seed_bridge_reused_relay", seed_bridge_id=seed_bridge_id, relay_bridge_id=relay_bridge_id)
if len({new_creator_id, host_creator_id, relay_bridge_id, seed_bridge_id}) != 4:
    fail("actor_chain_not_distinct", values=[new_creator_id, host_creator_id, relay_bridge_id, seed_bridge_id])
if seed_bridge_id not in seen_ids:
    fail("seed_bridge_not_in_local_dht", seed_bridge_id=seed_bridge_id)
if not any(entry.get("bridge_id") == seed_bridge_id and entry.get("active") for entry in bridges):
    fail("seed_bridge_not_active", seed_bridge_id=seed_bridge_id)
if len(session.get("bridge_ids") or []) != expected_count:
    fail("publisher_session_bridge_count_mismatch", bridge_ids=session.get("bridge_ids") or [])
session_bridge_ids = set(session.get("bridge_ids") or [])
if session_bridge_ids != publisher_ids:
    fail("publisher_session_publisher_dht_id_mismatch", session=sorted(session_bridge_ids), publisher=sorted(publisher_ids))
session_bridge_set_entries = ((session.get("bridge_set") or {}).get("bridge_dht_entries") or [])
if len(session_bridge_set_entries) != expected_count:
    fail("publisher_session_bridge_set_count_mismatch")
session_bridge_set_ids = {entry.get("bridge_id") for entry in session_bridge_set_entries}
if session_bridge_set_ids != publisher_ids:
    fail("publisher_session_bridge_set_publisher_dht_id_mismatch", session=sorted(session_bridge_set_ids), publisher=sorted(publisher_ids))

summary = {
    "state": state,
    "active_bridge_count": len(active),
    "bridge_count": len(bridges),
    "bootstrap_session_id": bootstrap_session_id,
    "new_creator_id": new_creator_id,
    "host_creator_id": host_creator_id,
    "relay_bridge_id": relay_bridge_id,
    "seed_bridge_id": seed_bridge_id,
    "publisher_dht_entry_count": len(publisher_entries),
    "publisher_dht_bridge_ids": sorted(publisher_ids),
}
json.dump(summary, open(summary_path, "w", encoding="utf-8"), indent=2, sort_keys=True)
PY
}

wait_tempo_bootstrap_events() {
  local deadline search_output trace_output counts_output missing_output
  search_output="$ARTIFACT_DIR/tempo/trace-evidence.tempo-search.json"
  trace_output="$ARTIFACT_DIR/trace-evidence.tempo-traces.json"
  counts_output="$ARTIFACT_DIR/traces-by-event.json"
  missing_output="$ARTIFACT_DIR/tempo/missing-events.txt"
  deadline=$((SECONDS + TRACE_TIMEOUT_SECONDS))
  while ((SECONDS <= deadline)); do
    smoke_tempo_query_chain "$CHAIN_ID" "$search_output"
    if TEMPO_URL="$TEMPO_URL" python3 - \
      "$search_output" "$trace_output" "$counts_output" "$missing_output" \
      "$BOOTSTRAP_SESSION_ID" "${BOOT_EVENTS[@]}" <<'PY'
import json
import os
import sys
import urllib.request

search_path, trace_path, counts_path, missing_path, bootstrap_session_id, *expected = sys.argv[1:]
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

all_strings = []

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
                all_strings.append(maybe)
        for item in value.values():
            walk(item)
    elif isinstance(value, list):
        for item in value:
            walk(item)
    elif isinstance(value, (str, int, float, bool)):
        all_strings.append(str(value))

for detail in details:
    walk(detail)

counts = {event: all_strings.count(event) for event in expected}
missing = [event for event, count in counts.items() if count < 1]
if any("discovery_probe" in value for value in all_strings):
    missing.append("unexpected_discovery_probe")
counts["_trace_ids"] = trace_ids
counts["_bootstrap_session_id_seen"] = bootstrap_session_id in all_strings
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

collect_bootstrap_pod_logs() {
  local check pod rest container
  for check in "${NODE_CHECKS[@]}"; do
    pod="${check%%:*}"
    rest="${check#*:}"
    container="${rest%%:*}"
    kubectl -n "$NAMESPACE" logs --since=20m "$pod" -c "$container" \
      --insecure-skip-tls-verify-backend=true >"$ARTIFACT_DIR/pod-logs/${pod}.log" 2>/dev/null || true
  done
}

assert_pod_log_bootstrap_events() {
  python3 - "$ARTIFACT_DIR/pod-logs" "$CHAIN_ID" "$ARTIFACT_DIR/pod-log-events.json" "$ARTIFACT_DIR/pod-log-missing-events.txt" "${POD_LOG_EVENTS[@]}" <<'PY'
import json
import sys
from pathlib import Path

log_dir = Path(sys.argv[1])
chain_id = sys.argv[2]
counts_path = sys.argv[3]
missing_path = sys.argv[4]
events = sys.argv[5:]

lines = []
for path in sorted(log_dir.glob("*.log")):
    for line in path.read_text(errors="ignore").splitlines():
        if chain_id in line:
            lines.append(line)

counts = {event: sum(1 for line in lines if event in line) for event in events}
counts["_chain_id_line_count"] = len(lines)
json.dump(counts, open(counts_path, "w", encoding="utf-8"), indent=2, sort_keys=True)
missing = [event for event in events if counts.get(event, 0) < 1]
if not lines:
    missing.insert(0, "chain_id_absent_from_pod_logs")
if missing:
    with open(missing_path, "w", encoding="utf-8") as handle:
        handle.write("\n".join(missing) + "\n")
    raise SystemExit(f"pod logs missing bootstrap ChainID evidence: {missing}")
try:
    Path(missing_path).unlink()
except FileNotFoundError:
    pass
PY
}

BOOT_EVENTS=(
  host_creator_seed_stored
  new_creator_seed_stored
  new_creator_join_started
  host_creator_join_relayed_via_bridge
  publisher_join_received
  publisher_response_to_host_via_bridge
  host_response_received_from_bridge
  host_relayed_response_to_new_creator
  new_creator_bootstrap_response_received
  seed_bridge_payload_received
  seed_bridge_punch_progress_publisher
  new_creator_seed_tunnel_ack
  new_creator_punch_progress_publisher
  seed_bridge_bridge_set_returned
  new_creator_local_dht_updated
  new_creator_bridge_entry_active
  new_creator_bootstrap_completed
)

POD_LOG_EVENTS=(
  host_creator_seed_requested
  host_creator_seed_stored
  publisher_dht_initialized
  publisher_dht_dumped
  new_creator_seed_requested
  new_creator_seed_stored
  publisher_bootstrap_payload_created
  publisher_seed_bridge_selected
  "${BOOT_EVENTS[@]}"
)

echo "Checking Pass 3 Conduit rollout in namespace '$NAMESPACE'..."
smoke_check_rollouts
smoke_discover_nodes
smoke_check_admin_metrics
smoke_wait_for_bridge_registry

CHAIN_ID="${CHAIN_ID_PREFIX}$(python3 -c 'import uuid; print(uuid.uuid4().hex)')"
SMOKE_LOKI_QUERY_START_NS="$(date +%s%N)"
printf '%s\n' "$CHAIN_ID" >"$ARTIFACT_DIR/chain-id.txt"

AUTHORITY_METADATA="$(smoke_admin_curl "$AUTHORITY_POD" publisher-authority GET /v1/admin/node-metadata)"
RECEIVER_METADATA="$(smoke_admin_curl "$RECEIVER_POD" publisher-receiver GET /v1/admin/node-metadata)"
HOST_METADATA="$(smoke_admin_curl "$CREATOR_HOST_POD" creator-runner GET /v1/admin/node-metadata)"
NEW_METADATA="$(smoke_admin_curl "$CREATOR_NEW_POD" creator-runner GET /v1/admin/node-metadata)"
write_json_arg "$AUTHORITY_METADATA" "$ARTIFACT_DIR/authority-metadata.json"
write_json_arg "$RECEIVER_METADATA" "$ARTIFACT_DIR/receiver-metadata.json"
write_json_arg "$HOST_METADATA" "$ARTIFACT_DIR/creator-host-metadata.json"
write_json_arg "$NEW_METADATA" "$ARTIFACT_DIR/creator-new-metadata.json"

HOST_ACTOR_ID="$(actor_id_from_metadata "$HOST_METADATA")"
NEW_ACTOR_ID="$(actor_id_from_metadata "$NEW_METADATA")"
if [[ "$HOST_ACTOR_ID" != "host-creator" || "$NEW_ACTOR_ID" != "new-creator" ]]; then
  smoke_fail "expected creator actors host-creator/new-creator, got host=$HOST_ACTOR_ID new=$NEW_ACTOR_ID"
fi
record_step "node metadata" "GET /v1/admin/node-metadata" \
  "authority, receiver, host-creator, and new-creator metadata available" \
  "host=$HOST_ACTOR_ID new=$NEW_ACTOR_ID" \
  "authority-metadata.json, receiver-metadata.json, creator-host-metadata.json, creator-new-metadata.json"

DEPLOYED_BRIDGE_IDS=()
for pod in "${BRIDGE_PODS[@]}"; do
  metadata="$(smoke_admin_curl "$pod" exit-bridge GET /v1/admin/node-metadata)"
  bridge_id="$(bridge_id_from_metadata "$metadata")"
  DEPLOYED_BRIDGE_IDS+=("$bridge_id")
done
printf '%s\n' "${DEPLOYED_BRIDGE_IDS[@]}" |
  python3 -c 'import json,sys; json.dump(sorted([line.strip() for line in sys.stdin if line.strip()]), sys.stdout, indent=2); print()' \
  >"$ARTIFACT_DIR/deployed-bridge-ids.json"
record_step "bridge metadata discovery" "GET /v1/admin/node-metadata" \
  "$EXPECTED_BRIDGES bridge node metadata responses" \
  "${#DEPLOYED_BRIDGE_IDS[@]} bridge ids discovered" \
  "deployed-bridge-ids.json"

BRIDGE_A_POD="${BRIDGE_PODS[0]}"
BRIDGE_A_METADATA="$(smoke_admin_curl "$BRIDGE_A_POD" exit-bridge GET /v1/admin/node-metadata)"
BRIDGE_A_ID="$(bridge_id_from_metadata "$BRIDGE_A_METADATA")"
write_json_arg "$BRIDGE_A_METADATA" "$ARTIFACT_DIR/exit-bridge-a-metadata.json"
echo "Smoke 2 chain_id=$CHAIN_ID HostCreator=$HOST_ACTOR_ID NewCreator=$NEW_ACTOR_ID ExitBridgeA=$BRIDGE_A_ID"

echo "Resetting creator local DHT state..."
reset_creator "$CREATOR_HOST_POD" "$CHAIN_ID" "$ARTIFACT_DIR/reset-host-creator-result.json"
reset_creator "$CREATOR_NEW_POD" "$CHAIN_ID" "$ARTIFACT_DIR/reset-new-creator-result.json"
assert_creator_reset "$CREATOR_HOST_POD" "$ARTIFACT_DIR/reset-host-creator-local-dht.json"
assert_creator_reset "$CREATOR_NEW_POD" "$ARTIFACT_DIR/reset-new-creator-local-dht.json"
record_step "creator local DHT reset" "POST /v1/admin/reset-creator-state" \
  "host and new creator local DHT tables empty" \
  "both creators report self_onboarding_state=none" \
  "reset-host-creator-local-dht.json, reset-new-creator-local-dht.json"

echo "Fetching Publisher-signed ExitBridgeA DHT entry..."
smoke_admin_curl "$AUTHORITY_POD" publisher-authority GET "/v1/admin/bridges/${BRIDGE_A_ID}/dht-entry" \
  >"$ARTIFACT_DIR/bridge-a-dht-entry.json"
BRIDGE_A_ENTRY="$(python3 -c 'import json,sys; print(json.dumps(json.load(open(sys.argv[1], encoding="utf-8"))["bridge"], separators=(",", ":")))' "$ARTIFACT_DIR/bridge-a-dht-entry.json")"
record_step "exit bridge A DHT entry" "GET /v1/admin/bridges/{bridge_id}/dht-entry" \
  "Publisher returns signed DHT entry for selected ExitBridgeA" \
  "bridge_id=$BRIDGE_A_ID" \
  "bridge-a-dht-entry.json"

ENTRY_EXPIRY_MS="$(( $(now_ms) + ${VERITAS_SEED_ENTRY_TTL_MS:-300000} ))"
SEED_HOST_PAYLOAD="$(build_seed_host_payload "$HOST_METADATA" "$AUTHORITY_METADATA" "$RECEIVER_METADATA" "$BRIDGE_A_ENTRY" "$ENTRY_EXPIRY_MS")"
printf '%s' "$SEED_HOST_PAYLOAD" >"$ARTIFACT_DIR/seed-host-creator-payload.json"

echo "Seeding HostCreator..."
SEED_HOST_RESPONSE="$(smoke_admin_curl "$CREATOR_HOST_POD" creator-runner POST "/v1/admin/seed-host-creator?chain_id=${CHAIN_ID}" "$SEED_HOST_PAYLOAD")"
write_json_arg "$SEED_HOST_RESPONSE" "$ARTIFACT_DIR/seed-host-creator-result.json"
python3 - "$ARTIFACT_DIR/seed-host-creator-result.json" "$CHAIN_ID" <<'PY'
import json
import sys

path, chain_id = sys.argv[1:3]
data = json.load(open(path, encoding="utf-8"))
assert data["chain_id"] == chain_id, data
assert data["host_creator_id"] == "host-creator", data
assert data["host_role_state"] == "host_seeded", data
assert data["self_onboarding_state"] == "onboarded", data
PY
record_step "SeedHostCreator" "POST /v1/admin/seed-host-creator" \
  "HostCreator stores Publisher and ExitBridgeA DHT metadata" \
  "host_creator_id=host-creator state=onboarded host_role_state=host_seeded" \
  "seed-host-creator-result.json"

echo "Initializing Publisher bridge DHT..."
INIT_RESPONSE="$(smoke_admin_curl "$AUTHORITY_POD" publisher-authority POST "/v1/admin/publisher-dht/initialize?chain_id=${CHAIN_ID}" "{}")"
write_json_arg "$INIT_RESPONSE" "$ARTIFACT_DIR/initialize-publisher-dht-result.json"
python3 - "$ARTIFACT_DIR/initialize-publisher-dht-result.json" "$CHAIN_ID" "$EXPECTED_BRIDGES" <<'PY'
import json
import sys

path, chain_id, expected = sys.argv[1], sys.argv[2], int(sys.argv[3])
data = json.load(open(path, encoding="utf-8"))
assert data["chain_id"] == chain_id, data
assert data["initialized_bridge_count"] == expected, data
assert data["publisher_dht_entry_count"] == expected, data
PY
record_step "InitializePublisherDht" "POST /v1/admin/publisher-dht/initialize" \
  "$EXPECTED_BRIDGES active ExitBridge DHT entries stored in Publisher DHT" \
  "initialized_bridge_count=$EXPECTED_BRIDGES publisher_dht_entry_count=$EXPECTED_BRIDGES" \
  "initialize-publisher-dht-result.json"

echo "Dumping and validating Publisher bridge DHT..."
dump_and_assert_publisher_dht
record_step "Publisher DHT dump" "GET /v1/admin/publisher-dht and GET /v1/admin/bridges/{bridge_id}/dht-entry" \
  "full Publisher DHT dump and every per-bridge DHT entry match" \
  "$EXPECTED_BRIDGES Publisher DHT entries validated" \
  "publisher-dht/publisher-dht.json, publisher-dht/per-entry, publisher-dht/publisher-dht-summary.json"

HOST_IP="${NODE_IP_BY_POD[$CREATOR_HOST_POD]:-}"
if [[ -z "$HOST_IP" ]]; then
  smoke_fail "could not resolve HostCreator pod IP"
fi
HOST_ENTRY_PAYLOAD="$(build_creator_dht_sign_payload "$HOST_METADATA" "$HOST_IP" "$ENTRY_EXPIRY_MS")"
printf '%s' "$HOST_ENTRY_PAYLOAD" >"$ARTIFACT_DIR/host-creator-dht-sign-payload.json"
HOST_ENTRY_RESPONSE="$(smoke_admin_curl "$AUTHORITY_POD" publisher-authority POST /v1/admin/creator-dht-entry "$HOST_ENTRY_PAYLOAD")"
write_json_arg "$HOST_ENTRY_RESPONSE" "$ARTIFACT_DIR/host-creator-dht-entry.json"
HOST_ENTRY="$(python3 -c 'import json,sys; print(json.dumps(json.load(open(sys.argv[1], encoding="utf-8"))["creator"], separators=(",", ":")))' "$ARTIFACT_DIR/host-creator-dht-entry.json")"
record_step "HostCreator DHT entry" "POST /v1/admin/creator-dht-entry" \
  "Publisher signs HostCreator DHT entry for NewCreator seed input" \
  "host_creator_id=$HOST_ACTOR_ID" \
  "host-creator-dht-entry.json"

HOST_ADMIN_URL="http://${HOST_IP}:${ADMIN_PORT}"
SEED_NEW_PAYLOAD="$(build_seed_new_payload "$NEW_METADATA" "$HOST_ENTRY" "$HOST_ADMIN_URL")"
printf '%s' "$SEED_NEW_PAYLOAD" >"$ARTIFACT_DIR/seed-new-creator-payload.json"

echo "Seeding NewCreator and starting first-contact bootstrap..."
SEED_NEW_RESPONSE="$(smoke_admin_curl "$CREATOR_NEW_POD" creator-runner POST "/v1/admin/seed-new-creator?chain_id=${CHAIN_ID}" "$SEED_NEW_PAYLOAD")"
write_json_arg "$SEED_NEW_RESPONSE" "$ARTIFACT_DIR/seed-new-creator-result.json"
BOOTSTRAP_SESSION_ID="$(json_field_from_arg "$SEED_NEW_RESPONSE" bootstrap_session_id)"
if [[ -z "$BOOTSTRAP_SESSION_ID" ]]; then
  smoke_fail "SeedNewCreator did not return bootstrap_session_id"
fi
record_step "SeedNewCreator" "POST /v1/admin/seed-new-creator" \
  "NewCreator starts first-contact bootstrap through HostCreator and Publisher" \
  "bootstrap_session_id=$BOOTSTRAP_SESSION_ID" \
  "seed-new-creator-result.json"

echo "Polling creator-new local DHT until terminal state..."
wait_for_terminal_local_dht ||
  smoke_fail "creator-new did not reach a terminal bootup state within ${BOOTSTRAP_TIMEOUT_SECONDS}s"
record_step "NewCreator local DHT terminal state" "GET /v1/admin/local-dht" \
  "NewCreator reaches onboarded or accepted fanout terminal state" \
  "$(local_dht_summary_from_arg "$(cat "$ARTIFACT_DIR/local-dht-final.json")")" \
  "local-dht-final.json, local-dht-progression.jsonl"

echo "Fetching Publisher bootstrap session $BOOTSTRAP_SESSION_ID..."
smoke_bootstrap_session_query "$CHAIN_ID" "$BOOTSTRAP_SESSION_ID" "$ARTIFACT_DIR/bootstrap-session.json"
record_step "Publisher bootstrap session" "GET /v1/admin/bootstrap-session" \
  "Publisher session exists and carries the same ChainID/bootstrap_session_id" \
  "bootstrap_session_id=$BOOTSTRAP_SESSION_ID" \
  "bootstrap-session.json"

echo "Asserting local DHT and distinct actor chain..."
assert_bootstrap_state
record_step "Bootstrap DHT agreement" "Publisher DHT + Creator local DHT + Publisher bootstrap session" \
  "Publisher DHT, Creator local DHT, and bootstrap session bridge ID sets match" \
  "DHT agreement validated" \
  "bootstrap-assertion-summary.json"

echo "Collecting pod logs and validating mandatory ChainID bootstrap events..."
collect_bootstrap_pod_logs
assert_pod_log_bootstrap_events
record_step "ChainID pod-log evidence" "kubectl logs --since=20m" \
  "all required bootstrap events appear in pod logs with the Smoke 2 ChainID" \
  "mandatory pod-log ChainID events validated" \
  "pod-log-events.json, pod-logs/*.log"

if [[ "$REQUIRE_OBSERVABILITY" -eq 1 ]]; then
  echo "Starting Tempo port-forward and checking ${#BOOT_EVENTS[@]} bootstrap events..."
  smoke_start_observability
  wait_tempo_bootstrap_events ||
    smoke_fail "Tempo did not report all ${#BOOT_EVENTS[@]} Smoke 2 bootstrap events for chain_id=$CHAIN_ID within ${TRACE_TIMEOUT_SECONDS}s."
else
  printf '{}\n' >"$ARTIFACT_DIR/traces-by-event.json"
fi

python3 - "$ARTIFACT_DIR/bootstrap-assertion-summary.json" "$ARTIFACT_DIR/traces-by-event.json" "$ARTIFACT_DIR/pod-log-events.json" "$STEP_RESULTS" "$ARTIFACT_DIR/summary.md" "$REQUIRE_OBSERVABILITY" <<'PY'
import json
import sys

summary_path, events_path, pod_events_path, step_results_path, output_path, require_obs = sys.argv[1:7]
summary = json.load(open(summary_path, encoding="utf-8"))
events = json.load(open(events_path, encoding="utf-8"))
pod_events = json.load(open(pod_events_path, encoding="utf-8"))
steps = [json.loads(line) for line in open(step_results_path, encoding="utf-8") if line.strip()]
with open(output_path, "w", encoding="utf-8") as handle:
    handle.write("# Conduit Smoke 2 Discovery Summary\n\n")
    for key in ("state", "bootstrap_session_id", "new_creator_id", "host_creator_id", "relay_bridge_id", "seed_bridge_id", "bridge_count", "active_bridge_count"):
        handle.write(f"- {key}: {summary.get(key)}\n")
    handle.write(f"- publisher_dht_entry_count: {summary.get('publisher_dht_entry_count')}\n")
    handle.write(f"- observability_required: {require_obs == '1'}\n")
    handle.write("\n")
    handle.write("| Step | Endpoint | Expected | Observed | Artifact |\n")
    handle.write("|---|---|---|---|---|\n")
    for step in steps:
        handle.write(
            f"| {step['step']} | `{step['endpoint']}` | {step['expected']} | {step['observed']} | `{step['artifact']}` |\n"
        )
    handle.write("\n")
    handle.write("| Pod Log Event | Count |\n|---|---:|\n")
    for key, value in pod_events.items():
        handle.write(f"| {key} | {value} |\n")
    handle.write("\n")
    if events:
        handle.write("| Event | Tempo Count |\n|---|---:|\n")
        for key, value in events.items():
            if key.startswith("_"):
                continue
            handle.write(f"| {key} | {value} |\n")
PY

echo "Conduit Smoke 2 discovery/bootstrap validation passed."
