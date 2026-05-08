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
  python3 - <<'PY'
import json
import sys

metadata = json.load(sys.stdin)
for key in ("conduit_actor", "node_id"):
    value = metadata.get(key)
    if value:
        print(value)
        break
else:
    raise SystemExit("bridge metadata did not include conduit_actor or node_id")
PY
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
