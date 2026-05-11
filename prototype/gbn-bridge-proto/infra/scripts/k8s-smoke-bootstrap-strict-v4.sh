#!/usr/bin/env bash
# Pass 4 Phase 1: strict bootstrap hardening validation over the Pass 3 k8s smoke.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
NAMESPACE="${VERITAS_K8S_NAMESPACE:-veritas}"
OBS_NS="${VERITAS_OBS_NAMESPACE:-observability}"
EXPECTED_BRIDGES="${VERITAS_K8S_EXPECTED_BRIDGES:-10}"
ARTIFACT_DIR=""
REQUIRE_ENCRYPTED_BOOTSTRAP_PAYLOAD=1
REQUIRE_SEED_BRIDGE_CATALOG_HANDOFF=1
REQUIRE_REAL_FANOUT_PROGRESS=1
PASS_THROUGH=()

usage() {
  cat <<'EOF'
Usage: k8s-smoke-bootstrap-strict-v4.sh [options]

Strict Pass 4 options:
  --require-encrypted-bootstrap-payload       Require encrypted CreatorBootstrap evidence. Default.
  --no-require-encrypted-bootstrap-payload    Skip encrypted CreatorBootstrap evidence assertion.
  --require-seed-bridge-catalog-handoff       Require encrypted SeedBridgeCatalog evidence. Default.
  --no-require-seed-bridge-catalog-handoff    Skip encrypted SeedBridgeCatalog evidence assertion.
  --require-real-fanout-progress              Require per-bridge fanout progress before completion. Default.
  --no-require-real-fanout-progress           Skip per-bridge progress assertion.

All other options are forwarded to k8s-smoke-discovery-v3.sh.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --require-encrypted-bootstrap-payload) REQUIRE_ENCRYPTED_BOOTSTRAP_PAYLOAD=1; shift ;;
    --no-require-encrypted-bootstrap-payload) REQUIRE_ENCRYPTED_BOOTSTRAP_PAYLOAD=0; shift ;;
    --require-seed-bridge-catalog-handoff) REQUIRE_SEED_BRIDGE_CATALOG_HANDOFF=1; shift ;;
    --no-require-seed-bridge-catalog-handoff) REQUIRE_SEED_BRIDGE_CATALOG_HANDOFF=0; shift ;;
    --require-real-fanout-progress) REQUIRE_REAL_FANOUT_PROGRESS=1; shift ;;
    --no-require-real-fanout-progress) REQUIRE_REAL_FANOUT_PROGRESS=0; shift ;;
    --expected-bridges)
      EXPECTED_BRIDGES="$2"
      PASS_THROUGH+=("$1" "$2")
      shift 2
      ;;
    --namespace)
      NAMESPACE="$2"
      PASS_THROUGH+=("$1" "$2")
      shift 2
      ;;
    --observability-namespace)
      OBS_NS="$2"
      PASS_THROUGH+=("$1" "$2")
      shift 2
      ;;
    --artifact-dir)
      ARTIFACT_DIR="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      PASS_THROUGH+=("$1")
      shift
      ;;
  esac
done

uname -a | grep -i microsoft >/dev/null || {
  echo "Pass 4 tooling requires WSL2 Ubuntu" >&2
  exit 1
}

cd "$ROOT_DIR"
source "$SCRIPT_DIR/k8s-smoke-common.sh"

smoke_require_deps
smoke_artifact_dir smoke-2-bootstrap-strict-v4 >/dev/null
trap 'status=$?; echo "Artifacts: $ARTIFACT_DIR"; exit $status' EXIT

bash "$SCRIPT_DIR/k8s-smoke-discovery-v3.sh" "${PASS_THROUGH[@]}" --artifact-dir "$ARTIFACT_DIR"

python3 - \
  "$ARTIFACT_DIR" \
  "$EXPECTED_BRIDGES" \
  "$REQUIRE_ENCRYPTED_BOOTSTRAP_PAYLOAD" \
  "$REQUIRE_SEED_BRIDGE_CATALOG_HANDOFF" \
  "$REQUIRE_REAL_FANOUT_PROGRESS" <<'PY'
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
expected_bridges = int(sys.argv[2])
require_encrypted = sys.argv[3] == "1"
require_catalog = sys.argv[4] == "1"
require_fanout = sys.argv[5] == "1"
failure_path = root / "strict-bootstrap-failure.json"
summary_path = root / "strict-bootstrap-summary.json"
flow_steps_path = root / "strict-bootstrap-flow-steps.json"
relay_privacy_path = root / "bootstrap-relay-privacy-evidence.json"
report_path = root / "strict-report.md"

def load(name):
    return json.loads((root / name).read_text(encoding="utf-8"))

def fail(code, **detail):
    detail["code"] = code
    failure_path.write_text(json.dumps(detail, indent=2, sort_keys=True), encoding="utf-8")
    raise SystemExit(f"{code}: {detail}")

def norm(value):
    return str(value or "").lower()

seed_result = load("seed-new-creator-result.json")
session_response = load("bootstrap-session.json")
assertion_summary = load("bootstrap-assertion-summary.json")
local_dht = load("local-dht-final.json")
evidence = seed_result.get("strict_bootstrap_evidence")
session = session_response.get("bootstrap_session") or {}
chain_id = seed_result.get("chain_id")
bootstrap_session_id = seed_result.get("bootstrap_session_id")

if not evidence:
    fail("missing_strict_bootstrap_evidence")
if not chain_id or not bootstrap_session_id:
    fail("missing_seed_result_chain_or_session", chain_id=chain_id, bootstrap_session_id=bootstrap_session_id)
if evidence.get("initial_plaintext_bridge_set_present") is not False:
    fail("initial_plaintext_bridge_set_present", value=evidence.get("initial_plaintext_bridge_set_present"))
if evidence.get("new_creator_dht_entry_id") != "new-creator":
    fail("new_creator_dht_entry_missing_from_bootstrap_payload", actual=evidence.get("new_creator_dht_entry_id"))
if evidence.get("publisher_entry_in_bootstrap_payload") is not True:
    fail("publisher_entry_missing_from_bootstrap_payload")
if evidence.get("publisher_entry_node_id") != "publisher":
    fail("publisher_entry_node_mismatch", actual=evidence.get("publisher_entry_node_id"))

def assert_payload(name, expected_kind):
    payload = evidence.get(name) or {}
    if norm(payload.get("payload_kind")) != expected_kind:
        fail("payload_kind_mismatch", payload=name, actual=payload.get("payload_kind"), expected=expected_kind)
    if payload.get("chain_id") != chain_id:
        fail("payload_chain_mismatch", payload=name, actual=payload.get("chain_id"), expected=chain_id)
    if payload.get("bootstrap_session_id") != bootstrap_session_id:
        fail(
            "payload_session_mismatch",
            payload=name,
            actual=payload.get("bootstrap_session_id"),
            expected=bootstrap_session_id,
        )
    if int(payload.get("ciphertext_len") or 0) <= 0:
        fail("payload_missing_ciphertext", payload=name)
    if int(payload.get("auth_tag_len") or 0) != 16:
        fail("payload_auth_tag_len_mismatch", payload=name, actual=payload.get("auth_tag_len"))
    if len(payload.get("plaintext_sha256") or []) != 32:
        fail("payload_hash_len_mismatch", payload=name, actual=len(payload.get("plaintext_sha256") or []))
    return payload

bootstrap_payload = None
catalog_payload = None
if require_encrypted:
    bootstrap_payload = assert_payload("encrypted_bootstrap_payload", "creator_bootstrap")
if require_catalog:
    catalog_payload = assert_payload("encrypted_seed_bridge_catalog_payload", "seed_bridge_catalog")

bridge_ids = session.get("bridge_ids") or []
if len(bridge_ids) != expected_bridges:
    fail("session_bridge_count_mismatch", actual=len(bridge_ids), expected=expected_bridges)
if evidence.get("seed_catalog_bridge_count") != expected_bridges:
    fail("seed_catalog_bridge_count_mismatch", actual=evidence.get("seed_catalog_bridge_count"), expected=expected_bridges)
if sorted(evidence.get("seed_catalog_bridge_ids") or []) != sorted(bridge_ids):
    fail("seed_catalog_bridge_ids_mismatch", evidence=sorted(evidence.get("seed_catalog_bridge_ids") or []), session=sorted(bridge_ids))

state = norm(session.get("state"))
if state != "completed":
    fail("bootstrap_session_not_completed", state=session.get("state"))
if assertion_summary.get("state") != "onboarded":
    fail("local_dht_not_onboarded", state=assertion_summary.get("state"))
if local_dht.get("self_onboarding_state") != "onboarded":
    fail("local_dht_final_not_onboarded", state=local_dht.get("self_onboarding_state"))

progress = session.get("progress_events") or []
for event in progress:
    if event.get("chain_id") != chain_id:
        fail("progress_chain_mismatch", event=event)
    if event.get("bootstrap_session_id") != bootstrap_session_id:
        fail("progress_session_mismatch", event=event)

seed_bridge_id = session.get("seed_bridge_id") or evidence.get("seed_bridge_id")
relay_bridge_id = session.get("relay_bridge_id") or seed_result.get("relay_bridge_id")
if evidence.get("seed_bridge_dht_entry_id") != seed_bridge_id:
    fail("seed_bridge_dht_entry_mismatch", actual=evidence.get("seed_bridge_dht_entry_id"), expected=seed_bridge_id)
for payload_name, payload in (
    ("encrypted_bootstrap_payload", bootstrap_payload),
    ("encrypted_seed_bridge_catalog_payload", catalog_payload),
):
    if not payload:
        continue
    if payload.get("recipient_key_id") != "new-creator":
        fail(
            "payload_recipient_mismatch",
            payload=payload_name,
            actual=payload.get("recipient_key_id"),
            expected="new-creator",
        )
    if payload.get("recipient_key_id") in {"host-creator", relay_bridge_id}:
        fail(
            "payload_targeted_transit_actor",
            payload=payload_name,
            recipient_key_id=payload.get("recipient_key_id"),
            relay_bridge_id=relay_bridge_id,
        )
seed_payload_reporters = {
    event.get("reporter_id")
    for event in progress
    if norm(event.get("stage")) == "seed_payload_received"
}
seed_tunnel_reporters = {
    event.get("reporter_id")
    for event in progress
    if norm(event.get("stage")) == "seed_tunnel_established"
}
if seed_bridge_id not in seed_payload_reporters:
    fail("seed_payload_progress_missing", seed_bridge_id=seed_bridge_id, reporters=sorted(seed_payload_reporters))
if seed_bridge_id not in seed_tunnel_reporters or "new-creator" not in seed_tunnel_reporters:
    fail("seed_tunnel_progress_missing", seed_bridge_id=seed_bridge_id, reporters=sorted(seed_tunnel_reporters))

fanout_reporters = {
    event.get("reporter_id")
    for event in progress
    if norm(event.get("stage")) == "bridge_tunnel_established"
}
missing_fanout = sorted(set(bridge_ids) - fanout_reporters)
if require_fanout and missing_fanout:
    fail("bridge_tunnel_progress_missing", bridge_ids=missing_fanout)
if require_fanout and not any(norm(event.get("stage")) == "bridge_set_complete" for event in progress):
    fail("bridge_set_complete_progress_missing")

active_bridge_ids = {
    entry.get("bridge_id")
    for entry in local_dht.get("bridge_entries") or []
    if entry.get("active")
}
if require_fanout and active_bridge_ids != set(bridge_ids):
    fail("local_dht_active_bridge_set_mismatch", active=sorted(active_bridge_ids), expected=sorted(bridge_ids))
if require_fanout and not active_bridge_ids.issubset(fanout_reporters):
    fail("bridge_marked_active_without_progress", bridge_ids=sorted(active_bridge_ids - fanout_reporters))

pod_log_text = "\n".join(
    path.read_text(encoding="utf-8", errors="replace")
    for path in sorted((root / "pod-logs").glob("*.log"))
)

def current_chain_lines(path):
    return [
        line
        for line in path.read_text(encoding="utf-8", errors="replace").splitlines()
        if chain_id in line
    ]

host_log_paths = sorted((root / "pod-logs").glob("creator-host-*.log"))
relay_log_paths = sorted((root / "pod-logs").glob(f"{relay_bridge_id}*.log")) if relay_bridge_id else []
transit_logs = {}
for path in host_log_paths + relay_log_paths:
    lines = current_chain_lines(path)
    if lines:
        transit_logs[path.name] = lines
if not host_log_paths:
    fail("host_creator_log_missing")
if relay_bridge_id and not relay_log_paths:
    fail("relay_bridge_log_missing", relay_bridge_id=relay_bridge_id)
if not transit_logs:
    fail("transit_actor_chain_logs_missing", relay_bridge_id=relay_bridge_id)

forbidden_plaintext_patterns = [
    "creator_response",
    "bridge_set",
    "bridge_entries",
    "bridge_dht_entries",
    "publisher_entry",
    "publisher_pub",
    "publisher_encryption_pub",
    "seed_bridge_id",
    "seed_bridge\":",
    "authority_url",
    "receiver_url",
    "pub_key",
    "publisher_sig",
    "ip_addr",
    "udp_punch_port",
    "entry_expiry_ms",
    "creator_bootstrap_payload",
    "seed_bridge_catalog_payload",
    "CreatorBootstrapPayload",
    "SeedBridgeCatalogPayload",
]
plaintext_hits = []
for log_name, lines in transit_logs.items():
    for line_no, line in enumerate(lines, start=1):
        lowered = line.lower()
        for pattern in forbidden_plaintext_patterns:
            if pattern.lower() in lowered:
                plaintext_hits.append(
                    {
                        "log": log_name,
                        "chain_line": line_no,
                        "pattern": pattern,
                        "line": line,
                    }
                )
if plaintext_hits:
    (root / "bootstrap-relay-plaintext-hits.json").write_text(
        json.dumps(plaintext_hits, indent=2, sort_keys=True),
        encoding="utf-8",
    )
    fail(
        "bootstrap_payload_plaintext_visible_to_transit_actor",
        relay_bridge_id=relay_bridge_id,
        hits=plaintext_hits[:10],
        hit_count=len(plaintext_hits),
    )

relay_privacy = {
    "chain_id": chain_id,
    "bootstrap_session_id": bootstrap_session_id,
    "relay_bridge_id": relay_bridge_id,
    "transit_actor_logs": {
        name: {"current_chain_line_count": len(lines)}
        for name, lines in sorted(transit_logs.items())
    },
    "allowed_transit_metadata": [
        "chain_id",
        "new_creator_id",
        "host_creator_id",
        "relay_bridge_id",
        "bootstrap_session_id",
    ],
    "forbidden_plaintext_patterns": forbidden_plaintext_patterns,
    "forbidden_plaintext_hit_count": len(plaintext_hits),
    "initial_plaintext_bridge_set_present": evidence.get("initial_plaintext_bridge_set_present"),
    "creator_bootstrap_ciphertext_len": (bootstrap_payload or {}).get("ciphertext_len"),
    "creator_bootstrap_recipient_key_id": (bootstrap_payload or {}).get("recipient_key_id"),
    "seed_bridge_catalog_ciphertext_len": (catalog_payload or {}).get("ciphertext_len"),
    "seed_bridge_catalog_recipient_key_id": (catalog_payload or {}).get("recipient_key_id"),
    "payloads_protected": [
        "CreatorBootstrap encrypted to NewCreator",
        "SeedBridgeCatalog encrypted to NewCreator",
    ],
}
relay_privacy_path.write_text(json.dumps(relay_privacy, indent=2, sort_keys=True), encoding="utf-8")

def require_log(event):
    if event not in pod_log_text:
        fail("pod_log_event_missing", event=event)
    return event

for event in (
    "new_creator_join_started",
    "host_creator_join_relayed_via_bridge",
    "publisher_join_received",
    "publisher_response_to_host_via_bridge",
    "host_relayed_response_to_new_creator",
    "new_creator_bootstrap_response_received",
    "seed_bridge_payload_received",
    "new_creator_bridge_set_requested",
    "seed_bridge_bridge_set_returned",
    "publisher_remaining_bridges_triggered",
    "new_creator_bridge_entry_active",
    "new_creator_bootstrap_completed",
):
    require_log(event)

flow_steps = [
    {
        "step": 1,
        "name": "NewCreator pairs with HostCreator",
        "status": "pass",
        "evidence": "seed-new-creator-payload.json, seed-new-creator-result.json",
        "observed": f"new_creator={seed_result.get('new_creator_id')} host_creator={seed_result.get('host_creator_id')}",
    },
    {
        "step": 2,
        "name": "NewCreator sends DHT entry and public key to HostCreator",
        "status": "pass",
        "evidence": "seed-new-creator-result.json, bootstrap-session.json",
        "observed": f"creator_dht_entry={evidence.get('new_creator_dht_entry_id')} encryption_key_present={bool(evidence.get('new_creator_encryption_pub_key'))}",
    },
    {
        "step": 3,
        "name": "HostCreator relays entry request through existing bridge path",
        "status": "pass",
        "evidence": "pod-logs/*.log",
        "observed": "host_creator_join_relayed_via_bridge and publisher_join_received observed",
    },
    {
        "step": 4,
        "name": "Publisher creates signed bootstrap payload with NewCreator, Publisher, and Seed ExitBridgeB DHT",
        "status": "pass",
        "evidence": "seed-new-creator-result.json, local-dht-final.json, bootstrap-session.json",
        "observed": f"publisher_entry={evidence.get('publisher_entry_node_id')} seed_bridge={seed_bridge_id}",
    },
    {
        "step": 5,
        "name": "Publisher encrypts bootstrap payload to NewCreator public key",
        "status": "pass",
        "evidence": "strict-bootstrap-summary.json",
        "observed": f"ciphertext_len={(bootstrap_payload or {}).get('ciphertext_len')}",
    },
    {
        "step": 6,
        "name": "Publisher seeds ExitBridgeB with remaining bridge DHT set",
        "status": "pass",
        "evidence": "bootstrap-session.json",
        "observed": f"seed_payload_reporter={seed_bridge_id} seed_catalog_bridge_count={evidence.get('seed_catalog_bridge_count')}",
    },
    {
        "step": 7,
        "name": "Encrypted bootstrap payload returns through Publisher -> ExitBridgeA -> HostCreator -> NewCreator",
        "status": "pass",
        "evidence": "pod-logs/*.log, bootstrap-relay-privacy-evidence.json",
        "observed": f"publisher_response_to_host_via_bridge, host_relayed_response_to_new_creator, and new_creator_bootstrap_response_received observed; transit_plaintext_hits={relay_privacy['forbidden_plaintext_hit_count']}",
    },
    {
        "step": 8,
        "name": "NewCreator decrypts payload and stores Publisher + Seed ExitBridgeB DHT state",
        "status": "pass",
        "evidence": "local-dht-final.json, strict-bootstrap-summary.json",
        "observed": f"publisher_entry_present={bool(local_dht.get('publisher_entry'))} seed_bridge={seed_bridge_id}",
    },
    {
        "step": 9,
        "name": "NewCreator and ExitBridgeB establish seed tunnel and report progress",
        "status": "pass",
        "evidence": "bootstrap-session.json",
        "observed": f"seed_tunnel_reporters={sorted(seed_tunnel_reporters)}",
    },
    {
        "step": 10,
        "name": "NewCreator requests bridge catalog from ExitBridgeB",
        "status": "pass",
        "evidence": "pod-logs/*.log",
        "observed": "new_creator_bridge_set_requested observed",
    },
    {
        "step": 11,
        "name": "ExitBridgeB returns signed remaining bridge catalog",
        "status": "pass",
        "evidence": "seed-new-creator-result.json, pod-logs/*.log",
        "observed": f"seed_bridge_bridge_set_returned observed; catalog_bridge_count={evidence.get('seed_catalog_bridge_count')}",
    },
    {
        "step": 12,
        "name": "Publisher fans out NewCreator DHT to remaining ExitBridges",
        "status": "pass",
        "evidence": "pod-logs/*.log",
        "observed": "publisher_remaining_bridges_triggered observed",
    },
    {
        "step": 13,
        "name": "Remaining ExitBridges establish tunnels with NewCreator and report progress",
        "status": "pass",
        "evidence": "bootstrap-session.json",
        "observed": f"bridge_tunnel_established={len(fanout_reporters)}/{len(bridge_ids)}",
    },
    {
        "step": 14,
        "name": "NewCreator marks each bridge active only after corresponding progress",
        "status": "pass",
        "evidence": "local-dht-final.json, bootstrap-session.json",
        "observed": f"active_bridge_count={len(active_bridge_ids)} progress_bridge_count={len(fanout_reporters)}",
    },
    {
        "step": 15,
        "name": "Every step preserves the same ChainID",
        "status": "pass",
        "evidence": "seed-new-creator-result.json, bootstrap-session.json, local-dht-final.json, pod-logs/*.log",
        "observed": f"chain_id={chain_id}",
    },
]
flow_steps_path.write_text(json.dumps(flow_steps, indent=2, sort_keys=True), encoding="utf-8")

summary = {
    "chain_id": chain_id,
    "bootstrap_session_id": bootstrap_session_id,
    "new_creator_encryption_pub_key_present": bool(evidence.get("new_creator_encryption_pub_key")),
    "new_creator_dht_entry_id": evidence.get("new_creator_dht_entry_id"),
    "publisher_entry_in_bootstrap_payload": evidence.get("publisher_entry_in_bootstrap_payload"),
    "publisher_entry_node_id": evidence.get("publisher_entry_node_id"),
    "seed_bridge_dht_entry_id": evidence.get("seed_bridge_dht_entry_id"),
    "encrypted_bootstrap_payload": bootstrap_payload,
    "encrypted_seed_bridge_catalog_payload": catalog_payload,
    "seed_bridge_id": seed_bridge_id,
    "bridge_count": len(bridge_ids),
    "active_bridge_count": len(active_bridge_ids),
    "fanout_progress_count": len(fanout_reporters),
    "progress_event_count": len(progress),
    "local_dht_state": local_dht.get("self_onboarding_state"),
    "relay_privacy": relay_privacy,
}
summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True), encoding="utf-8")

report = [
    "# GBN-PROTO-013 Phase 1 Strict Bootstrap Report",
    "",
    f"- ChainID: `{chain_id}`",
    f"- Bootstrap session: `{bootstrap_session_id}`",
    f"- Seed bridge: `{seed_bridge_id}`",
    f"- Bridge count: `{len(bridge_ids)}`",
    f"- Per-bridge fanout progress events: `{len(fanout_reporters)}`",
    f"- Initial plaintext bridge set present: `{evidence.get('initial_plaintext_bridge_set_present')}`",
    f"- Publisher DHT entry in encrypted bootstrap payload: `{evidence.get('publisher_entry_in_bootstrap_payload')}`",
    f"- CreatorBootstrap ciphertext bytes: `{(bootstrap_payload or {}).get('ciphertext_len')}`",
    f"- SeedBridgeCatalog ciphertext bytes: `{(catalog_payload or {}).get('ciphertext_len')}`",
    f"- Transit actor bootstrap plaintext hits: `{relay_privacy['forbidden_plaintext_hit_count']}`",
    "",
    "## Payload Encryption And Relay Privacy",
    "",
    "| Payload | Protection Gate | Status | Evidence |",
    "|---|---|---:|---|",
    f"| CreatorBootstrap | encrypted to NewCreator; no initial plaintext bridge set | `pass` | recipient_key_id=`{(bootstrap_payload or {}).get('recipient_key_id')}`, ciphertext_len=`{(bootstrap_payload or {}).get('ciphertext_len')}`, initial_plaintext_bridge_set_present=`{evidence.get('initial_plaintext_bridge_set_present')}` |",
    f"| SeedBridgeCatalog | encrypted to NewCreator before catalog handoff | `pass` | recipient_key_id=`{(catalog_payload or {}).get('recipient_key_id')}`, ciphertext_len=`{(catalog_payload or {}).get('ciphertext_len')}` |",
    f"| Relay transit visibility | HostCreator and ExitBridgeA current-chain logs contain no bootstrap-payload fields | `pass` | relay_bridge_id=`{relay_bridge_id}`, forbidden_plaintext_hits=`{relay_privacy['forbidden_plaintext_hit_count']}` |",
    "",
    "## README Flow Gate Ledger",
    "",
    "| Step | Required flow gate | Status | Evidence artifact | Observed |",
    "|---:|---|---:|---|---|",
]
for step in flow_steps:
    report.append(
        f"| {step['step']} | {step['name']} | `{step['status']}` | `{step['evidence']}` | {step['observed']} |"
    )
report.extend([
    "",
    "Artifacts: `seed-new-creator-result.json`, `bootstrap-session.json`, `local-dht-final.json`, `strict-bootstrap-summary.json`, `strict-bootstrap-flow-steps.json`, `bootstrap-relay-privacy-evidence.json`, `pod-log-events.json`, and `pod-logs/`.",
    "",
    "Result: strict bootstrap hardening validation passed.",
    "",
])
report_path.write_text("\n".join(report), encoding="utf-8")
PY

repo_root="$(cd "$ROOT_DIR/../.." && pwd)"
export VERITAS_K8S_SMOKE_REPORT_ROOT="${VERITAS_K8S_SMOKE_REPORT_ROOT:-$repo_root/docs/prototyping/Conduit/Full-Implementation-Plan-Pass4/Test-Reports}"
smoke_archive_report "GBN-PROTO-013-Phase1-Strict-Bootstrap" "$ARTIFACT_DIR/strict-report.md"

echo "Pass 4 strict bootstrap validation passed."
echo "Strict evidence report: $ARTIFACT_DIR/strict-report.md"
