# Shared Conduit V2 creator-seeding operator actions.
#
# Source this file from an operator panel after defining:
#   NODE_LABELS, NODE_DESCS, NODE_ROLES, _pick_node, _curl_admin, _pretty_json,
#   _now_epoch_ms, and optionally _collect_chain_traces.

_seed_actions_json_get() {
  local field="$1"
  python3 -c "import json,sys; print(json.load(sys.stdin).get('$field',''))" 2>/dev/null || true
}

_seed_actions_pick_surface() {
  local surface="$1" fallback_role="$2" i metadata found=()
  for ((i = 0; i < ${#NODE_LABELS[@]}; i++)); do
    [[ "${NODE_ROLES[$i]}" == "$fallback_role" ]] || continue
    metadata="$(_curl_admin "$i" GET /v1/admin/node-metadata 2>/dev/null || true)"
    if [[ -n "$metadata" ]] &&
      [[ "$(printf '%s' "$metadata" | _seed_actions_json_get publisher_surface)" == "$surface" ]]; then
      found+=("$i")
    fi
  done

  if [[ "${#found[@]}" -eq 1 ]]; then
    echo "${found[0]}"
    return 0
  fi

  _pick_node "Pick Publisher ${surface} node:" "$fallback_role"
}

_seed_actions_extract_bridge_id() {
  python3 -c 'import json,sys
metadata = json.load(sys.stdin)
for key in ("conduit_actor", "node_id"):
    value = metadata.get(key)
    if value:
        print(value)
        break
else:
    raise SystemExit("bridge metadata did not include conduit_actor or node_id")'
}

_seed_actions_build_payload() {
  local host_meta="$1" authority_meta="$2" receiver_meta="$3" bridge_entry="$4" genesis="$5" force="$6" expiry_ms="$7"
  HOST_METADATA="$host_meta" \
    AUTHORITY_METADATA="$authority_meta" \
    RECEIVER_METADATA="$receiver_meta" \
    BRIDGE_ENTRY="$bridge_entry" \
    GENESIS="$genesis" \
    FORCE="$force" \
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
    raise SystemExit("authority node metadata did not include publisher public key")
pub_hex = pub_hex.removeprefix("0x")
if len(pub_hex) % 2:
    raise SystemExit("publisher public key hex has odd length")

publisher_entry = {
    "node_id": authority.get("node_id") or "publisher",
    "authority_url": authority.get("authority_url") or "http://publisher-authority:8080",
    "receiver_url": receiver.get("receiver_url") or "http://publisher-receiver:8081",
    "pub_key": [int(pub_hex[i:i + 2], 16) for i in range(0, len(pub_hex), 2)],
    "entry_expiry_ms": int(os.environ["ENTRY_EXPIRY_MS"]),
}

payload = {
    "host_creator_id": host.get("conduit_actor") or host.get("node_id"),
    "publisher_entry": publisher_entry,
    "exit_bridge_a_entry": bridge_entry,
    "bootstrap_genesis": os.environ["GENESIS"] == "true",
    "force": os.environ["FORCE"] == "true",
}
print(json.dumps(payload, separators=(",", ":")))
PY
}

_seed_actions_build_creator_dht_sign_payload() {
  local metadata="$1" fallback_ip="$2" expiry_ms="$3"
  HOST_METADATA="$metadata" FALLBACK_IP="$fallback_ip" ENTRY_EXPIRY_MS="$expiry_ms" python3 - <<'PY'
import json
import os

metadata = json.loads(os.environ["HOST_METADATA"])
pub_hex = metadata.get("public_key")
if not pub_hex:
    raise SystemExit("creator metadata did not include public_key")
pub_hex = pub_hex.removeprefix("0x")
if len(pub_hex) % 2:
    raise SystemExit("creator public key hex has odd length")

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

_seed_actions_build_seed_new_payload() {
  local new_meta="$1" host_entry="$2" start_bootstrap="$3" force="$4" host_admin_url="$5"
  NEW_METADATA="$new_meta" HOST_ENTRY="$host_entry" START_BOOTSTRAP="$start_bootstrap" FORCE="$force" HOST_ADMIN_URL="$host_admin_url" python3 - <<'PY'
import json
import os

metadata = json.loads(os.environ["NEW_METADATA"])
host_entry = json.loads(os.environ["HOST_ENTRY"])
payload = {
    "new_creator_id": metadata.get("conduit_actor") or metadata.get("node_id"),
    "host_creator_entry": host_entry,
    "start_bootstrap": os.environ["START_BOOTSTRAP"] == "true",
    "force": os.environ["FORCE"] == "true",
}
if os.environ.get("HOST_ADMIN_URL"):
    payload["host_admin_url"] = os.environ["HOST_ADMIN_URL"]
print(json.dumps(payload, separators=(",", ":")))
PY
}

_seed_actions_local_dht_summary() {
  python3 -c 'import json,sys
table = json.load(sys.stdin)
bridges = table.get("bridge_entries") or []
active = sum(1 for entry in bridges if entry.get("active"))
session = table.get("current_bootstrap_session") or {}
print(f"state={table.get(\"self_onboarding_state\",\"unknown\")} bridges={len(bridges)} active={active} chain_id={session.get(\"chain_id\") or \"\"} bootstrap_session_id={session.get(\"session_id\") or \"\"}")'
}

do_initialize_publisher_dht() {
  local authority_idx result count active ids
  authority_idx="$(_seed_actions_pick_surface authority AUTHORITY)"
  echo "Initializing Publisher bridge DHT entries from active ExitBridge registry..." >&2
  result="$(_curl_admin "$authority_idx" POST /v1/admin/publisher-dht/initialize "{}")"
  printf '%s\n' "$result" | _pretty_json

  count="$(printf '%s' "$result" | _seed_actions_json_get initialized_bridge_count)"
  active="$(printf '%s' "$result" | _seed_actions_json_get active_bridge_count)"
  ids="$(printf '%s' "$result" | python3 -c 'import json,sys; print(",".join(json.load(sys.stdin).get("bridge_ids") or []))' 2>/dev/null || true)"
  if [[ -z "$count" || "$count" == "0" ]]; then
    echo "ERROR: Publisher DHT has no initialized bridge entries; verify ExitBridge pods registered first." >&2
    return 1
  fi
  if [[ "$count" != "$active" ]]; then
    echo "WARN: initialized bridge count ($count) differs from active bridge count ($active)." >&2
  fi
  echo "  Publisher DHT initialized bridge_ids: ${ids:-unknown}" >&2
}

do_seed_host_creator() {
  local host_idx authority_idx receiver_idx bridge_idx
  local host_meta authority_meta receiver_meta bridge_meta bridge_id dht_response bridge_entry
  local local_dht self_state host_role genesis force expiry_ms payload result chain_id yn

  host_idx="$(_pick_node "Pick creator to seed as HostCreator:" "CREATOR")"
  authority_idx="$(_seed_actions_pick_surface authority AUTHORITY)"
  receiver_idx="$(_seed_actions_pick_surface receiver RECEIVER)"
  bridge_idx="$(_pick_node "Pick ExitBridgeA seed bridge:" "BRIDGE")"

  host_meta="$(_curl_admin "$host_idx" GET /v1/admin/node-metadata)"
  authority_meta="$(_curl_admin "$authority_idx" GET /v1/admin/node-metadata)"
  receiver_meta="$(_curl_admin "$receiver_idx" GET /v1/admin/node-metadata)"
  bridge_meta="$(_curl_admin "$bridge_idx" GET /v1/admin/node-metadata)"
  bridge_id="$(printf '%s' "$bridge_meta" | _seed_actions_extract_bridge_id)"

  local_dht="$(_curl_admin "$host_idx" GET /v1/admin/local-dht)"
  self_state="$(printf '%s' "$local_dht" | _seed_actions_json_get self_onboarding_state)"
  host_role="$(printf '%s' "$local_dht" | _seed_actions_json_get host_role_state)"

  genesis=false
  if [[ "$self_state" != "onboarded" ]]; then
    echo "Selected creator self_onboarding_state is '${self_state:-unknown}'." >&2
    read -r -p "Use bootstrap_genesis=true for first HostCreator seed? [y/N]: " yn
    if [[ "${yn,,}" == "y" || "${yn,,}" == "yes" ]]; then
      genesis=true
    fi
  fi

  force=false
  if [[ "$host_role" == "host_seeded" ]]; then
    read -r -p "Host seed state already exists. Replace with force=true? [y/N]: " yn
    if [[ "${yn,,}" == "y" || "${yn,,}" == "yes" ]]; then
      force=true
    fi
  fi

  echo "Requesting Publisher-signed DHT entry for bridge '${bridge_id}'..." >&2
  dht_response="$(_curl_admin "$authority_idx" GET "/v1/admin/bridges/${bridge_id}/dht-entry")"
  bridge_entry="$(printf '%s' "$dht_response" | python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin)["bridge"], separators=(",", ":")))' 2>/dev/null || true)"
  if [[ -z "$bridge_entry" ]]; then
    echo "ERROR: authority did not return a signed bridge DHT entry:" >&2
    printf '%s\n' "$dht_response" | _pretty_json >&2
    return 1
  fi

  expiry_ms="$(( $(_now_epoch_ms) + ${VERITAS_SEED_ENTRY_TTL_MS:-300000} ))"
  payload="$(_seed_actions_build_payload "$host_meta" "$authority_meta" "$receiver_meta" "$bridge_entry" "$genesis" "$force" "$expiry_ms")"

  echo "Seeding HostCreator on ${NODE_LABELS[$host_idx]}..." >&2
  result="$(_curl_admin "$host_idx" POST /v1/admin/seed-host-creator "$payload")"
  printf '%s\n' "$result" | _pretty_json

  chain_id="$(printf '%s' "$result" | _seed_actions_json_get chain_id)"
  if [[ -n "$chain_id" ]]; then
    echo ""
    echo "  Root chain_id:       $chain_id"
    echo "  Host creator:        $(printf '%s' "$result" | _seed_actions_json_get host_creator_id)"
    echo "  Seeded bridge:       $(printf '%s' "$result" | _seed_actions_json_get seeded_bridge_id)"
    echo ""
    if declare -F _collect_chain_traces >/dev/null 2>&1; then
      read -r -p "Collect chain_id hits from recent logs now? [Y/n]: " yn
      [[ "${yn,,}" == "n" ]] || _collect_chain_traces "$chain_id"
    fi
  fi
}

do_seed_new_creator() {
  local new_idx host_idx authority_idx
  local new_meta host_meta host_dht host_role host_entry_request host_entry_response host_entry
  local expiry_ms host_ip host_admin_url force start_bootstrap payload result chain_id state summary deadline now yn

  new_idx="$(_pick_node "Pick creator to seed as NewCreator:" "CREATOR")"
  host_idx="$(_pick_node "Pick seeded HostCreator:" "CREATOR")"
  if [[ "$new_idx" == "$host_idx" ]]; then
    echo "ERROR: NewCreator and HostCreator must be distinct nodes." >&2
    return 1
  fi
  authority_idx="$(_seed_actions_pick_surface authority AUTHORITY)"

  new_meta="$(_curl_admin "$new_idx" GET /v1/admin/node-metadata)"
  host_meta="$(_curl_admin "$host_idx" GET /v1/admin/node-metadata)"
  host_dht="$(_curl_admin "$host_idx" GET /v1/admin/local-dht)"
  host_role="$(printf '%s' "$host_dht" | _seed_actions_json_get host_role_state)"
  if [[ "$host_role" != "host_seeded" ]]; then
    echo "ERROR: selected HostCreator is not host_seeded; run SeedHostCreator first." >&2
    printf '%s\n' "$host_dht" | _pretty_json >&2
    return 1
  fi

  echo "HostCreator seed state:" >&2
  HOST_DHT="$host_dht" python3 - <<'PY' >&2
import json
import os
table = json.loads(os.environ["HOST_DHT"])
seed = table.get("host_seed_state") or {}
publisher = seed.get("publisher_entry") or {}
bridge = seed.get("exit_bridge_a_entry") or {}
print(f"  publisher={publisher.get('node_id','unknown')} authority={publisher.get('authority_url','unknown')}")
print(f"  exit_bridge_a={bridge.get('bridge_id','unknown')} reachability={bridge.get('reachability_class','unknown')}")
PY

  host_ip="$(printf '%s' "$host_meta" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("ip_addr") or "")')"
  [[ -z "$host_ip" ]] && host_ip="${NODE_IPS[$host_idx]}"
  if [[ -z "$host_ip" ]]; then
    echo "ERROR: HostCreator IP address is unavailable." >&2
    return 1
  fi
  host_admin_url="http://${host_ip}:${ADMIN_PORT:-9090}"

  expiry_ms="$(( $(_now_epoch_ms) + ${VERITAS_SEED_ENTRY_TTL_MS:-300000} ))"
  host_entry_request="$(_seed_actions_build_creator_dht_sign_payload "$host_meta" "$host_ip" "$expiry_ms")"
  host_entry_response="$(_curl_admin "$authority_idx" POST /v1/admin/creator-dht-entry "$host_entry_request")"
  host_entry="$(printf '%s' "$host_entry_response" | python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin)["creator"], separators=(",", ":")))' 2>/dev/null || true)"
  if [[ -z "$host_entry" ]]; then
    echo "ERROR: authority did not return a signed HostCreator DHT entry:" >&2
    printf '%s\n' "$host_entry_response" | _pretty_json >&2
    return 1
  fi

  start_bootstrap=true
  read -r -p "Start bootstrap immediately? [Y/n]: " yn
  [[ "${yn,,}" == "n" ]] && start_bootstrap=false

  force=false
  read -r -p "Force replace existing NewCreator seed state? [y/N]: " yn
  if [[ "${yn,,}" == "y" || "${yn,,}" == "yes" ]]; then
    force=true
  fi

  payload="$(_seed_actions_build_seed_new_payload "$new_meta" "$host_entry" "$start_bootstrap" "$force" "$host_admin_url")"
  echo "Seeding NewCreator on ${NODE_LABELS[$new_idx]}..." >&2
  result="$(_curl_admin "$new_idx" POST /v1/admin/seed-new-creator "$payload")"
  printf '%s\n' "$result" | _pretty_json

  chain_id="$(printf '%s' "$result" | _seed_actions_json_get chain_id)"
  [[ -z "$chain_id" ]] && return 1
  if [[ "$start_bootstrap" != "true" ]]; then
    return 0
  fi

  deadline="$(( $(_now_epoch_ms) + (${VERITAS_SEED_NEW_TIMEOUT_SECS:-120} * 1000) ))"
  while true; do
    host_dht="$(_curl_admin "$new_idx" GET /v1/admin/local-dht)"
    summary="$(printf '%s' "$host_dht" | _seed_actions_local_dht_summary)"
    state="${summary#state=}"
    state="${state%% *}"
    echo "  $summary" >&2
    case "$state" in
      onboarded|fanout_partial|fanout_failed|seed_tunnel_failed)
        break
        ;;
      bootstrapping|seed_bridge_assigned|seed_tunnel_active|bridge_set_received|fanout_in_progress|new_creator_seeded)
        ;;
      *)
        ;;
    esac
    now="$(_now_epoch_ms)"
    if ((now >= deadline)); then
      echo "WARN: timed out waiting for terminal NewCreator state." >&2
      break
    fi
    sleep 2
  done

  echo ""
  echo "  Root chain_id: $chain_id"
  if declare -F _collect_chain_traces >/dev/null 2>&1; then
    read -r -p "Collect chain_id hits from recent logs now? [Y/n]: " yn
    [[ "${yn,,}" == "n" ]] || _collect_chain_traces "$chain_id"
  fi
}
