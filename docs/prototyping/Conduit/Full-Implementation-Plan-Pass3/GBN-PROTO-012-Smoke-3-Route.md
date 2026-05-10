# GBN-PROTO-012 - Execution Phase 9 - Smoke 3 Route And Encryption Boundary Suite Implementation (Pass 3 Successor To GBN-PROTO-010)

**Document ID:** GBN-PROTO-012-Smoke-3
**Status:** Completed
**Last Updated:** 2026-05-09
**Phase:** 9 (Smoke 3 — Route And Encryption Boundary Suite Implementation)
**Supersedes (in scope only):** [GBN-PROTO-010 Local Kubernetes Creator-To-Publisher Route Smoke Test Plan](../Full-Implementation-Plan-Pass2/GBN-PROTO-010-Local-Kubernetes-Creator-Publisher-Route-Smoke-Test-Plan.md)
**Parent Plan:** [GBN-PROTO-012](GBN-PROTO-012-Conduit-Architecture-Correct-Bootstrap-Execution-Plan.md)
**Depends On:** Phases 0–6 complete; Phase 7 (Smoke 1) green; Phase 8 (Smoke 2) green

This is **Smoke 3** in the Pass 3 local Kubernetes Conduit suite. It validates that
an onboarded NewCreator constructs its upload route from its own local DHT (per
`GBN-ARCH-001-V2` §3.6 and §3.7), envelope-encrypts the dummy frame so bridges only
see ciphertext (per §3.5 / §6 / §9.2), and can fail over to a second bridge after a
forced primary failure (per §7.1 / §9.1).

Implementation completed on 2026-05-09. The live local-k8s run passed with artifacts at
`prototype/gbn-bridge-proto/target/k8s-smoke-artifacts/smoke-3-route/20260508-194801-2480163`
and bootstrap fallback artifacts at
`prototype/gbn-bridge-proto/target/k8s-smoke-artifacts/smoke-2-discovery/20260508-194816-2483142`.

The Pass 2 file `GBN-PROTO-010-Local-Kubernetes-Creator-Publisher-Route-Smoke-Test-Plan.md`
is left unchanged. The Pass 2 baseline `send-dummy` shortcut is **not** treated as
Smoke 3 success in Pass 3.

---

## 1. Goal

Prove that:

1. `creator-new` after Phase 4 reaches `onboarded` with complete bootstrap-local
   DHT state.
2. A SendDummy on `creator-new` selects its bridge from local DHT, not a direct
   authority catalog call.
3. The Publisher decrypts the dummy frame, validates the encrypted envelope
   `plaintext_hash`, records payload hash validation evidence, and emits an
   `BridgeAck`.
4. The bridge cannot decrypt the dummy frame ciphertext (§6 trust boundary held).
5. A second SendDummy with `force_bridge_failure=true` selects a different bridge
   (failover proof per §7.1).
6. `relay_only` bridges are not selected (T1.9).
7. All 9 SendDummy events from Phase 5 §Observability are present in Tempo for each
   invocation.

---

## 2. Pre-Conditions

- Smoke 1 and Smoke 2 just passed.
- `creator-new` reports `self_onboarding_state == onboarded`.
- `creator-new` local DHT was populated from the Publisher-seeded bridge DHT set,
  with no direct authority catalog/bootstrap shortcut during `SendDummy`.
- `creator-new` local DHT contains the bootstrap `publisher_entry` with
  `encryption_pub_key`, plus non-expired `host_creator_entry`, `creator_entry`,
  and `current_bootstrap_session`.
- `creator-new` local DHT contains all expected ExitBridge entries, every entry is
  signed, active, non-expired, route-eligible, and backed by an active tunnel.

---

## 3. Implemented Command

```bash
bash prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-route-v3.sh \
  --require-observability \
  --message-size 256 \
  --include-failover
```

Flags:

- `--require-observability`: as Smoke 1.
- `--message-size N`: dummy plaintext bytes (default 256).
- `--include-failover`: also run `force_bridge_failure=true` invocation. Default on.
- `--bridge-decrypt-attempt`: enable the bridge-side ciphertext intercept assertion.
  Default on.
- `--plaintext-marker TEXT`: marker seeded into dummy plaintext before encryption.
  Default `VERITAS-SMOKE-3-PLAINTEXT`.

---

## 4. Probe Sequence

1. Generate `chain_id_normal = smoke-3-normal-<UUID>` and
   `chain_id_failover = smoke-3-failover-<UUID>`.
2. Capture `local-dht` snapshot before any send: `creator-local-dht-before.json`.
   Fail before `SendDummy` unless the preflight proves the Publisher encryption
   key and complete bootstrap DHT state are present.
   Also capture `dht-evidence/pre-send/`, including the Publisher DHT table,
   Publisher per-bridge DHT entries, HostCreator/NewCreator local DHT snapshots,
   and ExitBridge node metadata/local-DHT admin responses.
3. **Normal invocation**:
   - POST `/v1/admin/send-dummy?chain_id=<chain_id_normal>` to `creator-new` with
     `{ "size": <--message-size>, "force_bridge_failure": false,
     "plaintext_marker": "<--plaintext-marker>" }`.
   - Capture response.
4. **Failover invocation**:
   - POST `/v1/admin/send-dummy?chain_id=<chain_id_failover>` to `creator-new` with
     `{ "size": <--message-size>, "force_bridge_failure": true,
     "plaintext_marker": "<--plaintext-marker>" }`.
   - Capture response.
5. Wait 5 s for trace export and receiver persistence.
6. Collect ChainID-scoped pod logs from `creator-new`, the Publisher authority,
   the Publisher receiver, and each assigned ExitBridge.
7. Run assertions and write `report.md`.

---

## 5. Assertions

### 5.0 Bootstrap Local-DHT Preflight

Before any `SendDummy` call:

- `creator-new.self_onboarding_state == onboarded`;
- `publisher_entry.authority_url`, `publisher_entry.receiver_url`,
  `publisher_entry.pub_key`, and `publisher_entry.encryption_pub_key` are present
  and non-expired;
- `host_creator_entry` and `creator_entry` are present, signed, active, and
  non-expired;
- `current_bootstrap_session.session_id` exists and
  `current_bootstrap_session.last_state == onboarded`;
- `bridge_entries.length == --expected-bridges`;
- every bridge entry is signed, active, non-expired, non-`relay_only`, has an
  ingress endpoint and capabilities, and has a matching `active_tunnels` row.

### 5.1 Response Shape (Both Invocations)

- response `chain_id` exactly matches the request query chain id
  (`chain_id_normal` or `chain_id_failover`);
- `route_source == "local_dht"`;
- `assigned_bridge_id` is non-empty;
- `assigned_bridge_id` is present in `creator-local-dht-before.json` `bridge_entries`;
- `assigned_bridge_id`'s pre-call `active==true` AND
  `reachability_class != "relay_only"` AND `suspect_until_ms` is null or in the past;
- `ciphertext_only_at_bridge == true`;
- `frames == 1`;
- `elapsed_ms < 10000`.

### 5.2 Failover Proof

- Normal invocation's `assigned_bridge_id == B1` (record this value).
- Failover invocation's `assigned_bridge_id == B2`, where `B2 != B1`.
- Failover invocation's `force_bridge_failure_used == true`.
- Failover invocation's `candidate_bridge_ids` includes both `B1` and `B2`.
- After failover invocation, `creator-new`'s local DHT shows
  `B1.suspect_until_ms > now_ms`.

### 5.3 Receiver Persistence

For each invocation's `chain_id`:

- `GET /v1/admin/frames?chain_id=<chain_id>` on the Publisher (receiver surface)
  returns ≥ 1 row.
- The row has `via_bridge_id == <assigned_bridge_id>`.
- The row's stored ciphertext length matches `--message-size + AEAD overhead`
  (within ±32 bytes for AES-256-GCM tag/header).
- Receiver `frames_accepted` counter increased by 1.
- Receiver `bytes_ingested` counter increased by the ciphertext size.
- `GET /v1/admin/received-dummy-frames/<chain_id>` on the Publisher authority
  reports:
  - `frame_count == 1`;
  - `validated_frame_count == 1`;
  - `payload_hash_match == true`;
  - `decrypt_errors == []`;
  - `frames[0].encrypted_payload_hash == frames[0].decrypted_payload_hash`;
  - `frames[0].decrypted_payload_bytes == --message-size`.

### 5.4 Bridge Ciphertext-Only Assertion

For each invocation:

- The bridge's `bridge_dummy_frame_forwarded` log line records `payload_bytes` ≥
  message_size + AEAD overhead, but contains no plaintext substring of the dummy
  bytes (the test fixture seeds the plaintext with a recognizable marker like
  `b"VERITAS-SMOKE-3-PLAINTEXT"`; `grep` the bridge log for that marker — it must
  not appear).
- An adjacent unit-test (run by `cargo test -p gbn-bridge-protocol --test
  encryption_envelope`) proves the bridge's keypair cannot decrypt the captured
  ciphertext (this test runs in CI; Smoke 3 only verifies the runtime log
  property).

### 5.5 Trace Coverage (9 Events Per Invocation)

For each `chain_id`, Tempo returns spans for:

1. `creator_send_dummy_requested`
2. `creator_local_dht_loaded`
3. `creator_route_selected`
4. `creator_bridge_open_sent`
5. `creator_dummy_frame_sent`
6. `bridge_dummy_frame_forwarded`
7. `receiver_dummy_frame_ingested`
8. `publisher_dummy_payload_validated`
9. `publisher_dummy_ack_returned`

Total: 18 spans across the two invocations.

Every matched span must carry the same `chain_id` as the corresponding
SendDummy response. A span with the expected event name but a different chain id
is a failure, not supporting evidence.

When the optional observability backend is disabled, the script still requires
pod-log ChainID evidence for each invocation across `creator-new`, Publisher
authority/receiver, and the assigned ExitBridge before it can pass.

### 5.6 No Authority Catalog Call

Search Tempo for spans within the run window where
`actor_id == publisher-authority` AND
`span.name ∈ { catalog_request, bootstrap_request, discovery_probe }`. Assert 0
hits during the SendDummy windows. Local-DHT routing must not fall back to a
direct authority catalog call.

---

## 6. Artifacts

Written to
`prototype/gbn-bridge-proto/target/k8s-smoke-artifacts/smoke-3-route/<run-id>/`:

- `pods.json`
- `creator-local-dht-before.json`
- `creator-local-dht-ready-summary.json`
- `creator-local-dht-after-normal.json`
- `creator-local-dht-after-failover.json`
- `send-dummy-normal-result.json`
- `send-dummy-failover-result.json`
- `frames-by-chain-id.json` (one entry per chain)
- `received-dummy-normal.json`
- `received-dummy-failover.json`
- `dht-evidence/pre-send/`:
  - `publisher-dht.json`
  - `publisher-local-dht.json`
  - `creator-host-local-dht.json`
  - `creator-new-local-dht.json`
  - `publisher-bridge-entry/*.json`
  - `bridge-node-metadata/*.json`
  - `bridge-local-dht/*.json`
  - `dht-summary.json`
- `chainid-evidence/normal/chainid-summary.json`
- `chainid-evidence/failover/chainid-summary.json`
- `bridge-logs-by-chain-id/` (logs from the assigned bridges)
- `traces-by-chain-id/` (Tempo dumps; 9 spans per chain)
- `bridge-plaintext-grep.txt` (output of grep for the plaintext marker — must be
  empty)
- `route-summary.md` (table: invocation, chain_id, assigned_bridge_id, route_source,
  ciphertext_only, payload_hash_match, validated_frame_count,
  force_bridge_failure_used)
- `report.md` (durable evidence report covering DHT state, API completions,
  Publisher decrypt/hash validation, and ChainID evidence)

---

## 7. Failure Modes And Triage

| Failure | Likely Cause |
|---|---|
| `route_source != local_dht` | Phase 5 not implemented; SendDummy still using authority catalog |
| Plaintext marker found in bridge logs | Encryption envelope not applied; T1.10 / Pass 3 D2 broken |
| Failover assigned same bridge as normal | `force_bridge_failure` not respecting suspect TTL; T1.11 broken |
| Receiver `frames_accepted` did not increase | BridgeData forwarding broken or AEAD decryption failing |
| `relay_only` bridge selected | T1.9 filter missing in Phase 5 route selector |
| 9 events missing from Tempo | Phase 5 observability not wired; instrumentation gap |
| Direct authority catalog span found | SendDummy fallback path is reachable; Phase 5 §Required Behavior step 8 violated |

---

## 8. Out Of Scope

- Multi-chunk fanout (Master plan §6 §3.7 deferred).
- Multi-creator parallel SendDummy at scale.
- AWS validation (handled in Phase 6 AWS acceptance).
- Sanitizer / chunker / manifest builder (§3.4 deferred).

---

## 9. Implementation Tasks

1. Add `infra/scripts/k8s-smoke-route-v3.sh` (new file).
2. Test fixture: ensure `creator-runner` has a way to embed a recognizable plaintext
   marker for the smoke-3 dummy frame (e.g., a `--plaintext-marker` flag on the
   `send-dummy` request body). Default value `VERITAS-SMOKE-3-PLAINTEXT`.
3. Extend `k8s-smoke-common.sh` with `frames_by_chain_id(chain_id)` Publisher-receiver
   helper.
4. Extend `k8s-smoke-common.sh` with
   `received_dummy_frame(chain_id)` Publisher hash-validation helper.
5. WSL2 guard at top of script.

---

## 10. Acceptance

- `bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-route-v3.sh` passes.
- Against a Smoke-2-green cluster, the script exits 0 with two distinct
  `assigned_bridge_id`s and ciphertext-only bridge logs.
- The pre-send local-DHT preflight proves the Publisher encryption key, HostCreator
  entry, Creator entry, bootstrap session, and all expected active bridge entries
  are present on `creator-new`.
- `dht-evidence/pre-send/dht-summary.json` proves the Publisher DHT,
  NewCreator local DHT, active tunnels, Publisher per-bridge DHT entries, and
  ExitBridge node metadata agree on the same expected bridge set.
- `chainid-evidence/*/chainid-summary.json` proves each invocation's ChainID
  appears in creator, Publisher, and assigned ExitBridge logs.
- `report.md` is written and demonstrates the three required evidence classes:
  DHT dumps, API completions, and ChainID trace/log evidence.
- All 18 spans (9 per invocation) present in Tempo.
- Publisher dummy payload hash validation succeeds for both invocations.
- Bridge plaintext grep returns empty.
- `git diff --stat -- docs/prototyping/Conduit/Full-Implementation-Plan-Pass2/` is empty.
- V1 (`prototype/gbn-proto/**`) is unchanged.

Completed validation:

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo test -p gbn-bridge-publisher --test admin_send_dummy`
- `cargo test -p gbn-bridge-publisher --test admin_bootstrap_flow`
- `cargo test -p gbn-bridge-protocol --test encryption_envelope`
- `bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-smoke.sh`
- `bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-common.sh`
- `bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-route-v3.sh`
- `bash prototype/gbn-bridge-proto/infra/scripts/k8s-smoke.sh --send-dummy --check-creator-restart-persistence`
- `bash prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-route-v3.sh --require-observability --message-size 256 --include-failover`
