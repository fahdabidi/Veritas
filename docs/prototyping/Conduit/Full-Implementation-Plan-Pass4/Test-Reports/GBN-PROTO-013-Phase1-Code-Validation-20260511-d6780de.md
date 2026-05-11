# GBN-PROTO-013 Phase 1 Code Validation Report

## Run Metadata

- Date: `2026-05-11`
- Workspace: `prototype/gbn-bridge-proto`
- Commit under test: `d6780de feat: harden pass4 bootstrap validation`
- Host shell: Windows PowerShell
- Result: `PASS`

## Scope

This report covers the implementation-level validation for Pass 4 Phase 1:
Bootstrap Hardening And Validation.

The run validated:

1. Publisher bootstrap payload encryption for the NewCreator.
2. HostCreator relay behavior that does not expose the plaintext initial bridge set.
3. Publisher DHT entry evidence inside the encrypted bootstrap payload when Publisher
   deployment URLs are present.
4. SeedBridgeCatalog encrypted handoff metadata.
4. Per-bridge bootstrap fanout progress before session completion.
5. Pass 3 admin/API compatibility for the changed protocol fields.
6. Syntax readiness for the new strict k8s Bootstrap and SendDummy smoke scripts.

The strict k8s smokes themselves were not executed in this shell because they require
WSL2 Ubuntu plus a live local k8s Conduit stack.

## Validation Command Ledger

| Command | Status | Evidence |
|---|---:|---|
| `cargo fmt --all --check` | pass | Formatter check completed with no changes required |
| `cargo check --workspace` | pass | Workspace compiled successfully |
| `bash -n infra/scripts/k8s-smoke-bootstrap-strict-v4.sh` | pass | Strict bootstrap script parsed successfully |
| `bash -n infra/scripts/k8s-smoke-senddummy-strict-v4.sh` | pass | Strict SendDummy script parsed successfully |
| `cargo test -p gbn-bridge-protocol --test encryption_envelope` | pass | `5 passed; 0 failed` |
| `cargo test -p gbn-bridge-publisher --test admin_seed_new` | pass | `7 passed; 0 failed` |
| `cargo test -p gbn-bridge-publisher --test admin_bootstrap_flow` | pass | `4 passed; 0 failed` |
| `cargo test -p gbn-bridge-protocol --all-targets` | pass | protocol crate tests passed |

## Focused Test Evidence

### Protocol Encryption Envelope

Source command:

```bash
cargo test -p gbn-bridge-protocol --test encryption_envelope
```

Observed result:

```text
running 5 tests
test encrypted_frame_plaintext_hash_mismatch_fails_decryption ... ok
test encrypted_frame_aad_mismatch_fails_decryption ... ok
test encrypt_for_publisher_decrypt_from_creator_round_trip_succeeds ... ok
test bridge_key_cannot_decrypt_publisher_encrypted_frame ... ok
test encrypted_bootstrap_payload_round_trip_succeeds_for_new_creator_only ... ok

test result: ok. 5 passed; 0 failed
```

This proves the new encrypted bootstrap envelope can be decrypted by the intended
NewCreator key and rejected by a wrong creator key.

### Admin SeedNew Strict Bootstrap

Source command:

```bash
cargo test -p gbn-bridge-publisher --test admin_seed_new
```

Observed result:

```text
running 7 tests
test authority_admin_signs_creator_dht_entry ... ok
test unseeded_host_creator_rejects_join_request ... ok
test bootstrapping_state_rejects_new_payload_without_force ... ok
test valid_seed_without_bootstrap_stores_new_creator_seeded_state ... ok
test seed_new_validation_rejects_mismatch_expiry_bad_signature_and_conflict ... ok
test host_join_returns_encrypted_payloads_without_plaintext_bridge_set ... ok
test valid_seed_with_bootstrap_relays_join_through_host_and_records_session ... ok

test result: ok. 7 passed; 0 failed
```

Validated gates:

- HostCreator relay returns encrypted bootstrap payload metadata.
- HostCreator relay does not include the plaintext initial bridge set.
- NewCreator can decrypt Publisher bootstrap payload.
- NewCreator can decrypt SeedBridgeCatalog payload.
- Wrong creator key cannot decrypt the encrypted bootstrap payload.
- SeedNew response includes strict bootstrap evidence metadata.
- Session records per-bridge `bridge_tunnel_established` progress.

### Admin Bootstrap Flow

Source command:

```bash
cargo test -p gbn-bridge-publisher --test admin_bootstrap_flow
```

Observed result:

```text
running 4 tests
test bootstrap_rejects_when_relay_bridge_is_the_only_direct_bridge ... ok
test seed_new_surfaces_insufficient_bootstrap_bridge_rejection ... ok
test initialize_publisher_dht_admin_command_materializes_registered_exit_bridges ... ok
test full_bootstrap_payload_populates_local_dht_and_records_progress ... ok

test result: ok. 4 passed; 0 failed
```

Validated gates:

- Publisher rejects insufficient bootstrap topology.
- Publisher DHT initialization still materializes ExitBridge entries.
- Local DHT reaches onboarded state after strict bootstrap.
- Publisher session is completed only after fanout progress exists.

## API And Protocol Changes Covered

| Area | Evidence |
|---|---|
| Encrypted bootstrap payload | `EncryptedBootstrapPayload`, `BootstrapPayloadKind::CreatorBootstrap` |
| Publisher DHT entry in bootstrap payload | `CreatorBootstrapPayload.publisher_entry` and `StrictBootstrapEvidence.publisher_entry_in_bootstrap_payload` |
| Seed catalog handoff | `BootstrapPayloadKind::SeedBridgeCatalog` |
| NewCreator encryption identity | `PendingCreator.encryption_pub_key` |
| Initial plaintext bridge set suppression | `BootstrapJoinReply.bridge_set == None` in strict relay |
| Backward compatibility | Existing admin/API tests compile and pass with optional new fields |
| Evidence metadata | `SeedNewCreatorResponse.strict_bootstrap_evidence` |
| Fanout progress gate | `BridgeTunnelEstablished` required for all session bridge ids before completion |
| README flow ledger | `k8s-smoke-bootstrap-strict-v4.sh` emits `strict-bootstrap-flow-steps.json` and a 15-step report table |

## Broad Suite Notes

The broad command `cargo test --workspace` was attempted. It did not complete green because
environment-dependent integration tests failed outside the Phase 1 code path:

| Failing area | Observed reason |
|---|---|
| Publisher persistence flow | Postgres connection refused on localhost |
| Harness authority restart | Postgres connection refused on localhost |
| Receiver/upload proxy integration | Existing receiver proxy serialization failure in runtime/e2e tests |

The focused Phase 1 tests, protocol tests, script syntax checks, formatter check, and
workspace compile all passed.

## Result

Phase 1 implementation-level validation passed. The remaining required Phase 1 evidence is
the WSL2 local-k8s execution of the strict Bootstrap and SendDummy smoke scripts.
