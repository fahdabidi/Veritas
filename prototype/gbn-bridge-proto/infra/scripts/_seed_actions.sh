# Shared Conduit V2 creator-seeding operator actions.
#
# Source this file from an operator panel after defining:
#   NODE_LABELS, NODE_DESCS, NODE_ROLES, _pick_node, _curl_admin, _pretty_json,
#   _now_epoch_ms, and optionally _collect_chain_traces.

_seed_actions_json_get() {
  local field="$1"
  python3 -c "import json,sys; print(json.load(sys.stdin).get('$field',''))" 2>/dev/null || true
}

_seed_actions_chain_id() {
  local prefix="$1" actor="${2:-operator}"
  printf '%s-%s-%s\n' "$prefix" "$actor" "$(_now_epoch_ms)"
}

_seed_actions_path_with_chain_id() {
  local path="$1" chain_id="$2"
  printf '%s?chain_id=%s\n' "$path" "$chain_id"
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
enc_hex = (authority.get("publisher_encryption_public_key") or "").removeprefix("0x")

publisher_entry = {
    "node_id": authority.get("node_id") or "publisher",
    "authority_url": authority.get("authority_url") or "http://publisher-authority:8080",
    "receiver_url": receiver.get("receiver_url") or "http://publisher-receiver:8081",
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

_seed_actions_creator_state() {
  local idx="$1"
  _curl_admin "$idx" GET /v1/admin/local-dht | _seed_actions_json_get self_onboarding_state
}

_seed_actions_require_onboarded_creator() {
  local idx="$1" local_dht state
  local_dht="$(_curl_admin "$idx" GET /v1/admin/local-dht)"
  state="$(printf '%s' "$local_dht" | _seed_actions_json_get self_onboarding_state)"
  if [[ "$state" != "onboarded" && "$state" != "fanout_partial" ]]; then
    echo "ERROR: selected creator is not onboarded (self_onboarding_state=${state:-unknown}). Run SeedNewCreator first." >&2
    printf '%s\n' "$local_dht" | _pretty_json >&2
    return 1
  fi
  printf '%s\n' "$local_dht"
}

do_dump_local_dht() {
  local idx result role state
  idx="$(_pick_node "Pick node to dump local DHT state:")"
  result="$(_curl_admin "$idx" GET /v1/admin/local-dht)"
  role="${NODE_ROLES[$idx]}"
  state="$(printf '%s' "$result" | _seed_actions_json_get self_onboarding_state)"
  if [[ "$role" != "CREATOR" ]]; then
    echo "Node role ${role} has no creator local DHT. Role-tagged response:" >&2
  else
    echo "Creator local DHT summary: $(printf '%s' "$result" | _seed_actions_local_dht_summary)" >&2
  fi
  [[ -n "$state" && "$state" == "not_applicable" ]] && echo "State is not_applicable for this role." >&2
  printf '%s\n' "$result" | _pretty_json
}

do_initialize_publisher_dht() {
  local authority_idx result count active ids chain_id path
  authority_idx="$(_seed_actions_pick_surface authority AUTHORITY)"
  chain_id="$(_seed_actions_chain_id initialize-publisher-dht publisher)"
  path="$(_seed_actions_path_with_chain_id /v1/admin/publisher-dht/initialize "$chain_id")"
  echo "Initializing Publisher bridge DHT entries from active ExitBridge registry..." >&2
  result="$(_curl_admin "$authority_idx" POST "$path" "{}")"
  printf '%s\n' "$result" | _pretty_json

  chain_id="$(printf '%s' "$result" | _seed_actions_json_get chain_id)"
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
  echo "  Chain ID: ${chain_id:-unknown}" >&2
  echo "  Publisher DHT initialized bridge_ids: ${ids:-unknown}" >&2
}

do_dump_publisher_dht() {
  local authority_idx result count ids chain_id path
  authority_idx="$(_seed_actions_pick_surface authority AUTHORITY)"
  chain_id="$(_seed_actions_chain_id dump-publisher-dht publisher)"
  path="$(_seed_actions_path_with_chain_id /v1/admin/publisher-dht "$chain_id")"
  result="$(_curl_admin "$authority_idx" GET "$path")"
  printf '%s\n' "$result" | _pretty_json

  chain_id="$(printf '%s' "$result" | _seed_actions_json_get chain_id)"
  count="$(printf '%s' "$result" | _seed_actions_json_get publisher_dht_entry_count)"
  ids="$(printf '%s' "$result" | python3 -c 'import json,sys; print(",".join(json.load(sys.stdin).get("bridge_ids") or []))' 2>/dev/null || true)"
  echo "  Chain ID: ${chain_id:-unknown}" >&2
  echo "  Publisher DHT entry count: ${count:-unknown}" >&2
  echo "  Publisher DHT bridge_ids: ${ids:-unknown}" >&2
}

do_dump_node_dht() {
  local idx role result chain_id path state count ids
  idx="$(_pick_node "Pick node to dump role-specific DHT state:")"
  role="${NODE_ROLES[$idx]}"
  chain_id="$(_seed_actions_chain_id dump-node-dht "${NODE_LABELS[$idx]%% *}")"
  case "$role" in
    AUTHORITY)
      path="$(_seed_actions_path_with_chain_id /v1/admin/publisher-dht "$chain_id")"
      result="$(_curl_admin "$idx" GET "$path")"
      printf '%s\n' "$result" | _pretty_json
      count="$(printf '%s' "$result" | _seed_actions_json_get publisher_dht_entry_count)"
      ids="$(printf '%s' "$result" | python3 -c 'import json,sys; print(",".join(json.load(sys.stdin).get("bridge_ids") or []))' 2>/dev/null || true)"
      echo "  Node role: AUTHORITY" >&2
      echo "  DHT surface: Publisher bridge DHT" >&2
      echo "  Chain ID: ${chain_id:-unknown}" >&2
      echo "  Entry count: ${count:-unknown}" >&2
      echo "  Bridge IDs: ${ids:-unknown}" >&2
      ;;
    CREATOR)
      result="$(_curl_admin "$idx" GET /v1/admin/local-dht)"
      printf '%s\n' "$result" | _pretty_json
      echo "  Node role: CREATOR" >&2
      echo "  DHT surface: creator local DHT" >&2
      echo "  Summary: $(printf '%s' "$result" | _seed_actions_local_dht_summary)" >&2
      ;;
    *)
      result="$(_curl_admin "$idx" GET /v1/admin/local-dht)"
      printf '%s\n' "$result" | _pretty_json
      state="$(printf '%s' "$result" | _seed_actions_json_get state)"
      echo "  Node role: $role" >&2
      echo "  DHT surface: role local DHT" >&2
      echo "  State: ${state:-unknown}" >&2
      echo "  Note: ExitBridge and receiver nodes do not maintain creator local DHT state in the current V2 prototype." >&2
      ;;
  esac
}

do_seed_host_creator() {
  local host_idx authority_idx receiver_idx bridge_idx
  local host_meta authority_meta receiver_meta bridge_meta bridge_id dht_response bridge_entry
  local local_dht self_state host_role genesis force expiry_ms payload result chain_id path yn

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

  chain_id="$(_seed_actions_chain_id seed-host-creator "$(printf '%s' "$host_meta" | _seed_actions_json_get conduit_actor)")"
  path="$(_seed_actions_path_with_chain_id /v1/admin/seed-host-creator "$chain_id")"
  echo "Seeding HostCreator on ${NODE_LABELS[$host_idx]}..." >&2
  result="$(_curl_admin "$host_idx" POST "$path" "$payload")"
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
  local expiry_ms host_ip host_admin_url force start_bootstrap payload result chain_id path state summary deadline now yn

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
  chain_id="$(_seed_actions_chain_id seed-new-creator "$(printf '%s' "$new_meta" | _seed_actions_json_get conduit_actor)")"
  path="$(_seed_actions_path_with_chain_id /v1/admin/seed-new-creator "$chain_id")"
  echo "Seeding NewCreator on ${NODE_LABELS[$new_idx]}..." >&2
  result="$(_curl_admin "$new_idx" POST "$path" "$payload")"
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

do_send_dummy() {
  local idx local_dht state eligible_summary size force yn payload result chain_id path assigned selected route_source ciphertext_only

  idx="$(_pick_node "Pick onboarded NewCreator:" "CREATOR")"
  local_dht="$(_seed_actions_require_onboarded_creator "$idx")"

  eligible_summary="$(printf '%s' "$local_dht" | python3 -c 'import json,sys,time
table=json.load(sys.stdin)
now=int(time.time()*1000)
eligible=[]
for e in table.get("bridge_entries") or []:
    if not e.get("active"):
        continue
    if e.get("reachability_class") == "relay_only":
        continue
    if int(e.get("lease_expiry_ms") or 0) < now or int(e.get("entry_expiry_ms") or 0) < now:
        continue
    suspect=e.get("suspect_until_ms")
    if suspect is not None and int(suspect) > now:
        continue
    eligible.append(e.get("bridge_id","unknown"))
print(f"eligible={len(eligible)} ids={','.join(eligible)}")' 2>/dev/null || true)"
  echo "Local DHT route candidates: ${eligible_summary:-unknown}" >&2

  read -r -p "Frame size in bytes [512]: " size
  size="${size:-512}"
  if ! [[ "$size" =~ ^[0-9]+$ ]] || ((size < 1)); then
    echo "ERROR: size must be a positive integer." >&2
    return 1
  fi

  force=false
  read -r -p "Force first-bridge failure before send? [y/N]: " yn
  if [[ "${yn,,}" == "y" || "${yn,,}" == "yes" ]]; then
    force=true
  fi

  payload="$(SIZE="$size" FORCE="$force" python3 - <<'PY'
import json
import os
print(json.dumps({
    "size": int(os.environ["SIZE"]),
    "force_bridge_failure": os.environ["FORCE"] == "true",
}, separators=(",", ":")))
PY
)"
  chain_id="$(_seed_actions_chain_id send-dummy "${NODE_LABELS[$idx]%% *}")"
  path="$(_seed_actions_path_with_chain_id /v1/admin/send-dummy "$chain_id")"
  echo "Triggering SendDummy on ${NODE_LABELS[$idx]} using local DHT route selection..." >&2
  result="$(_curl_admin "$idx" POST "$path" "$payload")"
  printf '%s\n' "$result" | _pretty_json

  chain_id="$(printf '%s' "$result" | _seed_actions_json_get chain_id)"
  assigned="$(printf '%s' "$result" | _seed_actions_json_get assigned_bridge_id)"
  route_source="$(printf '%s' "$result" | _seed_actions_json_get route_source)"
  ciphertext_only="$(printf '%s' "$result" | _seed_actions_json_get ciphertext_only_at_bridge)"
  selected="$(printf '%s' "$result" | python3 -c 'import json,sys; print(",".join(json.load(sys.stdin).get("selected_bridge_ids") or []))' 2>/dev/null || true)"
  if [[ -z "$chain_id" ]]; then
    echo "WARN: no chain_id in SendDummy response." >&2
    return 1
  fi

  echo ""
  echo "  Root chain_id:              $chain_id"
  echo "  Route source:               ${route_source:-unknown}"
  echo "  Selected bridge ids:        ${selected:-unknown}"
  echo "  Assigned bridge_id:         ${assigned:-unknown}"
  echo "  Ciphertext only at bridge:  ${ciphertext_only:-unknown}"
  echo ""
  if declare -F _collect_chain_traces >/dev/null 2>&1; then
    read -r -p "Collect chain_id hits from recent logs now? [Y/n]: " yn
    [[ "${yn,,}" == "n" ]] || _collect_chain_traces "$chain_id"
  fi
}

do_discovery_probe() {
  local idx result chain_id path yn
  echo "WARN: DiscoveryProbe is deprecated for Pass 3 creator bootup." >&2
  echo "      Use SeedHostCreator -> InitializePublisherDht -> SeedNewCreator instead." >&2
  idx="$(_pick_node "Pick node for legacy DiscoveryProbe:")"
  chain_id="$(_seed_actions_chain_id discovery-probe "${NODE_LABELS[$idx]%% *}")"
  path="$(_seed_actions_path_with_chain_id /v1/admin/discovery-probe "$chain_id")"
  result="$(_curl_admin "$idx" POST "$path" "{}")"
  printf '%s\n' "$result" | _pretty_json
  chain_id="$(printf '%s' "$result" | _seed_actions_json_get chain_id)"
  if [[ -n "$chain_id" ]] && declare -F _collect_chain_traces >/dev/null 2>&1; then
    read -r -p "Collect chain_id hits from recent logs now? [Y/n]: " yn
    [[ "${yn,,}" == "n" ]] || _collect_chain_traces "$chain_id"
  fi
}

do_reset_creator_state() {
  local idx local_dht state chain confirm result reset_chain_id path
  idx="$(_pick_node "Pick creator to reset:" "CREATOR")"
  local_dht="$(_curl_admin "$idx" GET /v1/admin/local-dht)"
  state="$(printf '%s' "$local_dht" | _seed_actions_json_get self_onboarding_state)"
  chain="$(printf '%s' "$local_dht" | python3 -c 'import json,sys
table=json.load(sys.stdin)
session=table.get("current_bootstrap_session") or {}
print(session.get("chain_id") or "")' 2>/dev/null || true)"
  echo "Selected ${NODE_LABELS[$idx]} current state=${state:-unknown} chain_id=${chain:-none}" >&2
  read -r -p "Type RESET to clear this creator state: " confirm
  if [[ "$confirm" != "RESET" ]]; then
    echo "reset cancelled" >&2
    return 1
  fi
  reset_chain_id="$(_seed_actions_chain_id reset-creator-state "${NODE_LABELS[$idx]%% *}")"
  path="$(_seed_actions_path_with_chain_id /v1/admin/reset-creator-state "$reset_chain_id")"
  result="$(_curl_admin "$idx" POST "$path" "{}")"
  printf '%s\n' "$result" | _pretty_json
}

do_collect_traces() {
  local chain_id out_dir
  read -r -p "chain_id to collect: " chain_id
  if [[ -z "$chain_id" ]]; then
    echo "ERROR: chain_id is required." >&2
    return 1
  fi
  out_dir="/tmp/conduit-traces-${chain_id}"
  mkdir -p "$out_dir"
  printf '%s\n' "$chain_id" >"$out_dir/chain-id.txt"
  if declare -F _write_chain_traces >/dev/null 2>&1; then
    _write_chain_traces "$chain_id" "$out_dir"
  elif declare -F _collect_chain_traces >/dev/null 2>&1; then
    _collect_chain_traces "$chain_id" | tee "$out_dir/operator-trace-output.txt"
  else
    echo "No trace collector is available in this transport adapter." >"$out_dir/README.txt"
  fi
  echo "Trace artifacts: $out_dir"
}

do_build_upload_session() {
  local idx local_dht source chunk_size synthetic_size marker inline input_path payload result chain_id admin_path session_id yn
  idx="$(_pick_node "Pick onboarded creator for BuildUploadSession:" "CREATOR")"
  local_dht="$(_seed_actions_require_onboarded_creator "$idx")" || return 1

  read -r -p "Input source [synthetic|inline|path] [synthetic]: " source
  source="${source:-synthetic}"
  read -r -p "Chunk size in bytes [8192]: " chunk_size
  chunk_size="${chunk_size:-8192}"
  if ! [[ "$chunk_size" =~ ^[0-9]+$ ]] || ((chunk_size < 1)); then
    echo "ERROR: chunk size must be a positive integer." >&2
    return 1
  fi

  case "$source" in
    synthetic)
      read -r -p "Synthetic size in bytes [1048576]: " synthetic_size
      synthetic_size="${synthetic_size:-1048576}"
      read -r -p "Synthetic marker [VERITAS-SMOKE-4-PLAINTEXT]: " marker
      marker="${marker:-VERITAS-SMOKE-4-PLAINTEXT}"
      payload="$(SOURCE="$source" CHUNK="$chunk_size" SIZE="$synthetic_size" MARKER="$marker" python3 - <<'PY'
import json
import os
print(json.dumps({
    "input_source": os.environ["SOURCE"],
    "synthetic_size_bytes": int(os.environ["SIZE"]),
    "synthetic_marker": os.environ["MARKER"],
    "chunk_size_bytes": int(os.environ["CHUNK"]),
    "sanitization_profile": "v3-default-no-visual-anon",
}, separators=(",", ":")))
PY
)"
      ;;
    inline)
      read -r -p "Inline bytes as base64: " inline
      if [[ -z "$inline" ]]; then
        echo "ERROR: inline base64 content is required." >&2
        return 1
      fi
      payload="$(SOURCE="$source" CHUNK="$chunk_size" INLINE="$inline" python3 - <<'PY'
import json
import os
print(json.dumps({
    "input_source": os.environ["SOURCE"],
    "inline_bytes_b64": os.environ["INLINE"],
    "chunk_size_bytes": int(os.environ["CHUNK"]),
    "sanitization_profile": "v3-default-no-visual-anon",
}, separators=(",", ":")))
PY
)"
      ;;
    path)
      read -r -p "Path mounted inside creator container: " input_path
      if [[ -z "$input_path" ]]; then
        echo "ERROR: path is required." >&2
        return 1
      fi
      payload="$(SOURCE="$source" CHUNK="$chunk_size" PATH_VALUE="$input_path" python3 - <<'PY'
import json
import os
print(json.dumps({
    "input_source": os.environ["SOURCE"],
    "path": os.environ["PATH_VALUE"],
    "chunk_size_bytes": int(os.environ["CHUNK"]),
    "sanitization_profile": "v3-default-no-visual-anon",
}, separators=(",", ":")))
PY
)"
      ;;
    *)
      echo "ERROR: input source must be synthetic, inline, or path." >&2
      return 1
      ;;
  esac

  chain_id="$(_seed_actions_chain_id build-upload-session "${NODE_LABELS[$idx]%% *}")"
  admin_path="$(_seed_actions_path_with_chain_id /v1/admin/build-upload-session "$chain_id")"
  result="$(_curl_admin "$idx" POST "$admin_path" "$payload")"
  printf '%s\n' "$result" | _pretty_json
  session_id="$(printf '%s' "$result" | _seed_actions_json_get session_id)"
  chain_id="$(printf '%s' "$result" | _seed_actions_json_get chain_id)"
  [[ -n "$session_id" ]] && echo "  session_id: $session_id"
  [[ -n "$chain_id" ]] && echo "  chain_id:   $chain_id"
  if [[ -n "$session_id" ]]; then
    read -r -p "Continue with SendUpload for this session? [y/N]: " yn
    if [[ "${yn,,}" == "y" || "${yn,,}" == "yes" ]]; then
      do_send_upload "$idx" "$session_id"
    fi
  fi
}

_seed_actions_extract_sessions() {
  python3 -c 'import json,sys
data=json.load(sys.stdin)
sessions=data.get("sessions", data if isinstance(data, list) else [])
for idx, session in enumerate(sessions, 1):
    if isinstance(session, dict):
        sid=session.get("session_id") or session.get("id") or ""
        status=session.get("status") or session.get("session_status") or ""
    else:
        sid=str(session)
        status=""
    if sid:
        print(f"{idx}\t{sid}\t{status}")'
}

do_send_upload() {
  local idx="${1:-}" session_id="${2:-}" local_dht sessions rows row choice target_lanes force yn payload result chain_id path
  if [[ -z "$idx" ]]; then
    idx="$(_pick_node "Pick onboarded creator for SendUpload:" "CREATOR")"
  fi
  local_dht="$(_seed_actions_require_onboarded_creator "$idx")" || return 1

  if [[ -z "$session_id" ]]; then
    sessions="$(_curl_admin "$idx" GET /v1/admin/upload-sessions)"
    printf '%s\n' "$sessions" | _pretty_json
    mapfile -t rows < <(printf '%s' "$sessions" | _seed_actions_extract_sessions)
    if [[ "${#rows[@]}" -gt 0 ]]; then
      echo "Available sessions:" >&2
      local i sid status
      for row in "${rows[@]}"; do
        IFS=$'\t' read -r i sid status <<<"$row"
        printf "  [%s] %s %s\n" "$i" "$sid" "${status:+($status)}" >&2
      done
      read -r -p "Select session number or paste session_id: " choice
      if [[ "$choice" =~ ^[0-9]+$ ]] && ((choice >= 1 && choice <= ${#rows[@]})); then
        IFS=$'\t' read -r _ session_id _ <<<"${rows[$((choice - 1))]}"
      else
        session_id="$choice"
      fi
    else
      read -r -p "session_id: " session_id
    fi
  fi
  if [[ -z "$session_id" ]]; then
    echo "ERROR: session_id is required." >&2
    return 1
  fi

  read -r -p "Target lane count [10]: " target_lanes
  target_lanes="${target_lanes:-10}"
  if ! [[ "$target_lanes" =~ ^[0-9]+$ ]] || ((target_lanes < 1)); then
    echo "ERROR: target lane count must be a positive integer." >&2
    return 1
  fi
  force=null
  read -r -p "Force lane failure bridge_ids as comma list (blank = none): " yn
  if [[ -n "$yn" ]]; then
    force="$(printf '%s' "$yn" | python3 -c 'import json,sys
items=[item.strip() for item in sys.stdin.read().split(",") if item.strip()]
print(json.dumps(items))')"
  fi
  payload="$(SESSION="$session_id" LANES="$target_lanes" FORCE="$force" python3 - <<'PY'
import json
import os
force = json.loads(os.environ["FORCE"]) if os.environ["FORCE"] != "null" else None
print(json.dumps({
    "session_id": os.environ["SESSION"],
    "target_lane_count": int(os.environ["LANES"]),
    "lane_open_timeout_ms": 30000,
    "chunk_ack_timeout_ms": 15000,
    "force_lane_failure": force,
}, separators=(",", ":")))
PY
)"
  chain_id="$(_seed_actions_chain_id send-upload "${NODE_LABELS[$idx]%% *}")"
  path="$(_seed_actions_path_with_chain_id /v1/admin/send-upload "$chain_id")"
  result="$(_curl_admin "$idx" POST "$path" "$payload")"
  printf '%s\n' "$result" | _pretty_json
  chain_id="$(printf '%s' "$result" | _seed_actions_json_get chain_id)"
  [[ -n "$chain_id" ]] && echo "  chain_id: $chain_id"
}
