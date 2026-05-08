# GBN-PROTO-012 - Execution Phase 5 - Onboarded-Creator SendDummy Local-DHT Single-Lane Envelope Demo

**Status:** Completed
**Last Updated:** 2026-05-08
**Phase:** 5 (Onboarded-Creator SendDummy And Local-DHT Single-Lane Envelope Demo)
**Parent Plan:** [GBN-PROTO-012](GBN-PROTO-012-Conduit-Architecture-Correct-Bootstrap-Execution-Plan.md)
**Depends On:** Phase 0–4 complete

## Objective

Update `SendDummy` so it uses the architecture-correct **single-lane** route
construction path from `GBN-ARCH-001-V2` §3.6 (the lane-selection half) and the
single-frame envelope from §3.5. Phase 5 is the smallest possible test of the
local-DHT routing + envelope encryption boundary on one bridge with one frame:

- selected node must be onboarded as a NewCreator
  (`self_onboarding_state=onboarded` or `fanout_partial`);
- selected node must choose **one** upload lane from its local DHT / discovery
  table;
- dummy payload must be **encrypted for the Publisher** before crossing the bridge
  (per §3.5 / Pass 3 D2);
- dummy payload must travel through that one active ExitBridge entry to the
  Publisher (receiver surface);
- response and traces must prove the route source was local DHT state and that the
  bridge only saw opaque ciphertext.

Phase 5 deliberately stays single-lane / single-frame. The full §3.4 pre-processing
pipeline (sanitize, chunk, manifest), the §3.5 per-chunk fanout, the §3.6 multi-lane
selection, and the §3.7 progressive fanout are owned by Phases 10 and 11. Phase 5
proves the encryption boundary and local-DHT route source on one frame so Phases
10/11 can build on a known-good envelope rather than introducing the envelope and
the multi-lane logic in the same change.

Phase 5 consumes the bridge entries that Phase 4 stored in the creator's local DHT
from the Publisher-seeded 10-entry ExitBridge DHT set. `SendDummy` must not issue a
fresh Publisher authority catalog/bootstrap request, because that would bypass the
bootstrap state being validated by Pass 3.

Per Master plan §3.5, "Publisher" is one role with two surfaces. The dummy frame's
ciphertext is destined for the Publisher (receiver surface); ACK signing happens at
the Publisher (authority surface). The operator selects "Publisher" once.

Update the parent plan status tracker when this phase is complete.

---

## Required Behavior

`POST /v1/admin/send-dummy` on any node performs:

1. Read local onboarding state from `LocalDiscoveryTable.self_onboarding_state`.
2. If state is not `onboarded` or `fanout_partial`, return `creator_not_onboarded`
   (HTTP 409). Phase 0 deployed creator pods are the only nodes where this can
   succeed; non-creator pods still mount the endpoint (per Pass 2) but the Phase 1
   role-aware response makes the gate trivially fail there with the same error.
3. Load local DHT / discovery `bridge_entries`.
4. Filter:
   - drop entries whose `lease_expiry_ms` or `entry_expiry_ms` is in the past;
   - drop entries whose `publisher_sig` does not verify against the Publisher trust
     root;
   - drop entries with `reachability_class = relay_only` (T1.9, per §4.2 — relay-only
     bridges are not eligible for creator ingress);
   - drop entries with `suspect_until_ms` still in the future;
   - drop entries whose `active=false`.
5. Rank surviving entries by:
   - prefer bridges with most recent successful `BridgePunchAck` (rank tie broken by
     `lease_expiry_ms` descending);
6. If `force_bridge_failure: true` is set in the request (T1.11 §Failover Test), mark
   the top-ranked bridge `suspect_until_ms = now + suspect_ttl_ms` and re-rank.
7. Select the top bridge as the primary route.
8. Construct the encryption envelope (see Content Encryption Envelope below).
9. Open the route through the selected bridge using `BridgeOpen`.
10. Send the encrypted dummy frame via `BridgeData`.
11. Wait for `BridgeAck` from Publisher (receiver surface) routed back through the
    bridge.
12. Persist evidence: bridge `frames_forwarded` counter, receiver `frames_accepted`
    counter, persistence row in `conduit_ingested_frames` keyed by `chain_id`.

### Success Response

```json
{
  "chain_id": "send-dummy-...",
  "actor_id": "creator-new",
  "route_source": "local_dht",
  "candidate_bridge_ids": ["exit-bridge-1", "exit-bridge-3", "exit-bridge-7"],
  "selected_bridge_ids": ["exit-bridge-3"],
  "assigned_bridge_id": "exit-bridge-3",
  "encryption_envelope": "publisher_x25519_hkdf_aes256gcm_v1",
  "ciphertext_only_at_bridge": true,
  "frames": 1,
  "elapsed_ms": 42,
  "force_bridge_failure_used": false
}
```

`candidate_bridge_ids` is the post-filter set; `selected_bridge_ids` is the active
chosen subset (Pass 3 single-bridge SendDummy: 1 entry; later media-upload phase will
expand to multipath). `ciphertext_only_at_bridge: true` is asserted by the test in
§Tests below.

### Failure Response (Non-Onboarded)

```json
{
  "error": {
    "code": "creator_not_onboarded",
    "message": "selected node has not completed NewCreator onboarding",
    "current_state": "bootstrapping"
  }
}
```

The `current_state` field helps the operator diagnose without an extra `local-dht`
call. Returned with HTTP 409.

### Failure Response (No Eligible Bridge)

If the post-filter set is empty (everything expired, signed wrong, suspect, or
relay_only), return `no_eligible_bridge`:

```json
{
  "error": {
    "code": "no_eligible_bridge",
    "message": "no active publisher-signed direct/brokered bridge available in local DHT",
    "filter_drops": {
      "expired_lease": 0,
      "expired_entry": 0,
      "bad_signature": 0,
      "relay_only": 1,
      "suspect": 2,
      "inactive": 6
    }
  }
}
```

This makes Smoke 3 failure analysis precise.

---

## Content Encryption Envelope (T1.10, Pass 3 D2)

`GBN-ARCH-001-V2` §3.5 requires the creator to encrypt content for the Publisher
before assigning chunks to bridges. ExitBridges forward opaque packets; they cannot
decrypt media or inspect plaintext. Pass 3 D2 enforces this for the dummy frame so
Smoke 3 can prove the §6 / §9.2 trust boundary holds in the new local-DHT route path.

### Key Derivation

```text
creator_ephemeral_x25519_keypair = X25519::new()
shared_secret = X25519::dh(
  creator_ephemeral_x25519_keypair.private,
  publisher_entry.pub_key                  // X25519 pubkey from local DHT publisher_entry
)
hkdf_input = "veritas/conduit/v2/upload-content-key"
upload_content_key = HKDF-SHA256(shared_secret, hkdf_input, 32 bytes)
nonce_base = HKDF-SHA256(shared_secret, "veritas/conduit/v2/nonce", 12 bytes)
```

The Publisher's X25519 pubkey is read from the local-DHT `publisher_entry.pub_key`
(per Phase 1, this entry's pubkey is validated against the configured Publisher trust
root — no `publisher_sig` is involved).

### Frame Build

For Pass 3, the dummy frame has session_id, chunk_index=0, total_chunks=1,
plaintext_hash = SHA-256(plaintext_dummy_bytes):

```text
session_id           = random 16 bytes
chunk_index          = 0
total_chunks         = 1
plaintext_dummy      = N bytes (configurable; default 256 from existing send-dummy)
plaintext_hash       = SHA-256(plaintext_dummy)
aad                  = session_id || chunk_index_le4 || total_chunks_le4 || plaintext_hash
nonce                = nonce_base XOR (chunk_index encoded as 12-byte big-endian)
ciphertext           = AES-256-GCM-encrypt(upload_content_key, nonce, aad, plaintext_dummy)
```

Wrap as `BridgeData` payload:

```text
[creator_ephemeral_pubkey]   // 32 bytes — Publisher uses this to derive the same key
[publisher_key_id]           // matches publisher_entry.node_id
[session_id]
[chunk_index]
[total_chunks]
[plaintext_hash]
[ciphertext]
[auth_tag]
```

The bridge's `BridgeData` handler treats this entire blob as opaque bytes — it forwards
to the Publisher (receiver surface) without parsing.

### Decryption At Publisher (Receiver Surface)

Publisher derives the same `upload_content_key` using the creator's ephemeral pubkey
and Publisher's long-term private key, decrypts, verifies the `plaintext_hash`, then
emits a `BridgeAck` carrying the chain_id back through the bridge.

For Pass 3 the dummy plaintext is just the test bytes — no manifest, no chunking, no
content sanitization (those are §3.4 work and explicitly out of scope per Master plan
§6). The envelope alone proves the §6 trust boundary.

### `gbn-bridge-protocol` Additions

Add to `gbn-bridge-protocol::envelope`:

- `EnvelopeKeyDerivation::PublisherX25519HkdfAes256GcmV1`
- `EncryptedFrame { creator_ephemeral_pubkey, publisher_key_id, session_id, chunk_index, total_chunks, plaintext_hash, ciphertext, auth_tag }`
- helper `encrypt_for_publisher(...)` and `decrypt_from_creator(...)`

Use existing `ed25519_dalek` and add `x25519-dalek` as a workspace dependency. Use
`aes-gcm` crate for AEAD.

---

## Failover Test Path (T1.11)

§9.1 minimum validation requires "creator fails over to a second bridge after
first-bridge loss". Add a debug flag to the `send-dummy` request:

```json
{ "size": 256, "force_bridge_failure": true }
```

When `force_bridge_failure=true`:

1. Route selection picks the top-ranked bridge (call it `B1`).
2. Before sending, mark `B1.suspect_until_ms = now + suspect_ttl_ms` (Phase 4
   §Mark Bridge Suspect Action).
3. Re-run route selection; pick the new top-ranked bridge (`B2`).
4. Send the encrypted dummy through `B2`.
5. Response includes `force_bridge_failure_used: true` and `assigned_bridge_id` is
   `B2`. The pre-failure candidate is in `candidate_bridge_ids` but absent from
   `selected_bridge_ids`.

This exercises the §7.1 bridge-failure recovery path: mark suspect → retry another
cached valid bridge → continue. Smoke 3 (Phase 6) runs both `force_bridge_failure=false`
and `force_bridge_failure=true` invocations.

---

## Observability

8 events tagged with `chain_id`, `actor_id`, `route_source`, selected bridge ids,
ciphertext bytes count, force-failure flag:

- `creator_send_dummy_requested`
- `creator_local_dht_loaded`
- `creator_route_selected`
- `creator_bridge_open_sent`
- `creator_dummy_frame_sent` (after encryption envelope built)
- `bridge_dummy_frame_forwarded` (bridge sees ciphertext only — log payload size, not
  content)
- `receiver_dummy_frame_ingested` (Publisher receiver surface; logs decryption
  success and plaintext_hash match)
- `publisher_dummy_ack_returned`

These cover the §2.5 traceability events 17–24 (extending the 16-event bootup list to
24 total when SendDummy runs after onboarding).

---

## Script Changes

Update `SendDummy` in:

- `prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh`
- `prototype/gbn-bridge-proto/infra/scripts/k8s-control-interactive.sh`
- shared `_seed_actions.sh`

Operator flow:

1. Discover live nodes filtering by `role=creator`.
2. Prompt: select creator node.
3. Query `GET /v1/admin/local-dht` on the selected creator.
4. If `self_onboarding_state` is not `onboarded` or `fanout_partial`, print a clear
   error (showing `current_state`) and do not call `send-dummy`. Suggest running
   `SeedNewCreator` first.
5. Print summary of post-filter eligible bridges (count, ids).
6. Prompt: send normal SendDummy or failover SendDummy
   (`force_bridge_failure=true`)?
7. POST `/v1/admin/send-dummy` to the creator with the chosen flag.
8. Print `route_source`, `selected_bridge_ids`, `assigned_bridge_id`,
   `force_bridge_failure_used`, `ciphertext_only_at_bridge`, `chain_id`.
9. Offer trace collection by `chain_id`.

WSL2 guard per Master plan §2.8.

---

## Tests

Add tests in
`prototype/gbn-bridge-proto/crates/gbn-bridge-creator/tests/send_dummy_route.rs`:

- non-onboarded node returns `creator_not_onboarded` with `current_state` populated;
- onboarded node with empty post-filter set returns `no_eligible_bridge` with the
  per-reason `filter_drops` map;
- onboarded node selects route from local DHT; response shows `route_source=local_dht`;
- expired-lease entries are filtered out;
- expired-entry entries are filtered out;
- bad-signature entries are filtered out;
- `relay_only` entries are filtered out (T1.9);
- suspect entries are filtered out;
- ranking prefers most recent ACK then later expiry;
- no fallback direct authority bootstrap occurs during `send-dummy` (network mock
  asserts no Publisher authority HTTP call between local-DHT load and `BridgeOpen`);
- bridge forwarding still increments `frames_forwarded`;
- receiver persists the frame and increments `frames_accepted`;
- `force_bridge_failure=true` causes selection of a different bridge in `B2` than
  would have been chosen at `B1`, and `B1` carries a future `suspect_until_ms`.

Add tests in
`prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/tests/encryption_envelope.rs`:

- `encrypt_for_publisher → decrypt_from_creator` round-trip succeeds;
- AAD mismatch causes decryption failure;
- `plaintext_hash` mismatch causes decryption failure;
- replay (same nonce, same key) is detected by AEAD;
- bridge-side intercept test: capture the ciphertext on the bridge forwarding path;
  attempt to decrypt with the bridge's own keypair; assert decryption fails (proves
  `ciphertext_only_at_bridge` is true).

Run inside WSL2 Ubuntu:

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 3 tooling requires WSL2 Ubuntu" >&2; exit 1; }
cd prototype/gbn-bridge-proto
cargo test -p gbn-bridge-protocol --test encryption_envelope
cargo test -p gbn-bridge-publisher --test admin_send_dummy
```

---

## Implementation Notes

Completed 2026-05-08.

- `POST /v1/admin/send-dummy` now requires a creator-local DHT source and returns
  `creator_not_onboarded` for Publisher/ExitBridge admin listeners or non-onboarded
  creators.
- Creator-side `SendDummy` selects the route from local DHT bridge entries populated
  during Phase 4 bootstrap. The legacy authority-bootstrap client path remains only as
  a lower-level compatibility helper and is not used by the admin endpoint.
- Route filtering rejects expired, unsigned, inactive, suspect, and `relay_only`
  entries before ranking candidates by recent tunnel activity and lease expiry.
- `force_bridge_failure=true` marks the first candidate suspect and reselects from the
  remaining local-DHT candidates.
- The protocol now includes a Publisher-targeted X25519 + HKDF + AES-256-GCM
  `EncryptedFrame`; bridge tests assert that a bridge key cannot decrypt the frame.
- Operator `SendDummy` flow moved into `_seed_actions.sh`, filters to creator nodes,
  checks onboarding state before POST, and exposes the failover flag.

---

## Acceptance Criteria

- `SendDummy` fails on `creator-host` if it has only `host_role_state=host_seeded`
  but `self_onboarding_state != onboarded` (with `creator_not_onboarded`).
- `SendDummy` fails on `creator-new` if it has only `self_onboarding_state=new_creator_seeded`
  or `bootstrapping` (still onboarding).
- `SendDummy` succeeds on `creator-new` after Phase 4 reaches `onboarded` and reports
  `route_source=local_dht`, `ciphertext_only_at_bridge=true`.
- The selected bridge is one of the entries in the creator's local DHT and is
  active, signed, non-expired, non-suspect, and `direct` or `brokered` (never
  `relay_only`).
- A bridge-side intercept test confirms the bridge cannot decrypt the dummy frame
  ciphertext (§6 / §9.2 trust boundary).
- `force_bridge_failure=true` causes selection of a second bridge after marking the
  first suspect, exercising §7.1 failover.
- Smoke 3 (Phase 6) can prove local-DHT route construction, dummy delivery, and
  failover.
- V1 (`prototype/gbn-proto/**`) is unchanged.
- Parent plan status tracker is updated.
