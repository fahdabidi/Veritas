# GBN-PROTO-013 Phase 2 Mobile Runtime FFI Report

## Run Metadata

- Date: `2026-05-11`
- Workspace: `prototype/gbn-bridge-proto`
- Base commit before this phase: `969932c docs: remove superseded pending pass4 reports`
- Host shell: Windows PowerShell invoking WSL2 Ubuntu
- Result: `PASS`

## Scope

This report covers Pass 4 Phase 2: Mobile Runtime Boundary And FFI.

The run validated:

1. A new `gbn-bridge-mobile-ffi` Rust crate exists in the workspace.
2. The crate builds as `rlib` and `cdylib`.
3. Kotlin-facing calls use a narrow JSON/JNI boundary with private numeric runtime handles.
4. The raw C ABI remains available for non-Android harnesses.
5. Mobile local state is created under an app-provided state directory.
6. HostCreator QR seed preview/import validates mobile-safe seed constraints.
7. ChainID-filtered mobile events and evidence export work without private key export.
8. Android `.so` artifacts build for `arm64-v8a` and `x86_64`.
9. V1 files remain untouched.

## Implementation Evidence

| Area | Evidence |
|---|---|
| Workspace member | `prototype/gbn-bridge-proto/Cargo.toml` includes `crates/gbn-bridge-mobile-ffi` |
| Rust runtime crate | `prototype/gbn-bridge-proto/crates/gbn-bridge-mobile-ffi/` |
| Kotlin fixture | `prototype/gbn-bridge-proto/mobile/bindings-fixtures/MobileCreatorRuntime.kt` |
| Android ABI build script | `prototype/gbn-bridge-proto/infra/scripts/build-mobile-ffi.sh` |
| Generated artifact location | `prototype/gbn-bridge-proto/mobile/android/app/src/main/jniLibs/` |
| Generated artifact policy | `.so` files and build metadata are ignored by git and reproducible through the build script |

## Validation Command Ledger

| Command | Status | Evidence |
|---|---:|---|
| `cargo fmt --all` | pass | Formatter applied successfully |
| `bash -n infra/scripts/build-mobile-ffi.sh` | pass | Script parsed successfully |
| `cargo test -p gbn-bridge-mobile-ffi` | pass | `7 passed; 0 failed` |
| `cargo fmt --all --check` | pass | Formatter check completed with no changes required |
| `cargo check --workspace` | pass | Workspace compiled successfully |
| `cargo build -p gbn-bridge-mobile-ffi` | pass | Host `cdylib`/`rlib` build completed |
| `infra/scripts/build-mobile-ffi.sh --abi arm64-v8a --abi x86_64 --profile debug` | pass | Android ABI artifacts written under `mobile/android/app/src/main/jniLibs` |
| `nm -D .../arm64-v8a/libgbn_bridge_mobile_ffi.so` | pass | C ABI and JNI symbols exported |
| `nm -D .../x86_64/libgbn_bridge_mobile_ffi.so` | pass | C ABI and JNI symbols exported |
| `git diff --stat -- prototype/gbn-proto/` | pass | No V1 file changes |
| `git diff --stat -- docs/prototyping/Lattice/` | pass | No V1 doc changes |

## Focused Test Evidence

Source command:

```bash
cargo test -p gbn-bridge-mobile-ffi
```

Observed result:

```text
running 7 tests
test config_validation_rejects_missing_or_escaping_state_dir ... ok
test ffi_json_boundary_hides_native_pointer_and_catches_errors ... ok
test qr_import_rejects_shortcuts_expired_and_private_admin_endpoints ... ok
test startup_creates_identity_local_dht_and_events_then_restarts_same_identity ... ok
test qr_preview_and_import_persist_host_seed_without_onboarding ... ok
test reset_clears_local_state_and_preserves_event_export_path ... ok
test synthetic_upload_trace_filter_and_evidence_export_work ... ok

test result: ok. 7 passed; 0 failed
```

Validated gates:

- `state_dir` is required and path escape is rejected.
- Runtime startup creates `identity.json`, `local_dht.json`, and `evidence/events.jsonl`.
- Runtime restart reloads the same identity.
- HostCreator QR seed import persists HostCreator DHT state without marking the mobile
  creator onboarded.
- QR preview/import rejects expired seeds, Publisher/ExitBridge shortcut fields, and
  private/admin endpoints.
- Synthetic upload session build crosses the mobile runtime boundary and emits ChainID
  events.
- `traceEvents()` filters by ChainID.
- `exportEvidence()` writes a manifest, local DHT, events, upload summaries, and remote
  trace query hints without exporting private identity material.
- C ABI calls return structured JSON errors and numeric handles instead of raw pointers.

## Android ABI Artifact Evidence

Build command:

```bash
infra/scripts/build-mobile-ffi.sh --abi arm64-v8a --abi x86_64 --profile debug
```

Environment evidence:

```text
rustc 1.94.1 (e408947bf 2026-03-25)
cargo 1.94.1 (29ea6fb6a 2026-03-24)
Android NDK: /usr/lib/android-sdk/ndk/28.2.13676358
Android API level: 26
```

Generated artifacts:

| ABI | Size | SHA-256 |
|---|---:|---|
| `arm64-v8a/libgbn_bridge_mobile_ffi.so` | 22M | `709204225fa48be77494595b55092fe080b6e642db1efe2311d70279a5b1fe33` |
| `x86_64/libgbn_bridge_mobile_ffi.so` | 23M | `932536df29fe863c8f3f57572cae7a0375ba9f34d6cc82b1063701ac1e3a0a8b` |
| `mobile-ffi-build-metadata.json` | 210B | `221d165a7b90cbee1fcdfa509777361ee0b8e1adfc246ef83793a9e45ef724dd` |

Build metadata:

```json
{
  "created_at_utc": "2026-05-11T21:05:26Z",
  "git_sha": "969932cccde26c3c8a31ba318c5d8302a277e4f0",
  "rustc": "rustc 1.94.1 (e408947bf 2026-03-25)",
  "profile": "debug",
  "abis": ["arm64-v8a","x86_64"]
}
```

Exported symbol evidence:

```text
gbn_mobile_runtime_create
Java_com_veritas_gbn_mobile_runtime_MobileCreatorRuntime_00024Native_gbnMobileRuntimeCreate
Java_com_veritas_gbn_mobile_runtime_MobileCreatorRuntime_00024Native_gbnMobileRuntimeCall
Java_com_veritas_gbn_mobile_runtime_MobileCreatorRuntime_00024Native_gbnMobileRuntimeClose
```

## Evidence Bundle Sample

Sample bundle path from the focused test run:

```text
/tmp/gbn-mobile-ffi-evidence-1347487-1778533340642/state/evidence/mobile-evidence-1778533340699/
```

Bundle file hashes:

| File | SHA-256 |
|---|---|
| `manifest.json` | `a9d984ea8bcc22003f46ecde21ef6876e61ffc0359cc064696958256f7b55c3d` |
| `local_dht.json` | `574d9e3f998612bc397874977d86cb5d3087815892fdd9e28c08da6502428224` |
| `events.jsonl` | `fb9c8a62339482e0cf2bf1a173123b10cca014bce8c704d8efac1dc38c09222d` |
| `remote_trace_queries.json` | `e4490fbde7d3a5dc95b2b9bc782d76fd4343090706273098f2fa47f3bb3908b0` |
| `upload_sessions.json` | `a8323d17cfcff9c35e5635ead83b2d0c8647afff44e943b5aef98db188f83478` |
| `node_metadata.json` | `fd4ff6acdf7c76ed1ac79f761416436e9416657b61f7295b75659a516a6fd99f` |

Sample events:

```json
{"chain_id":"mobile-runtime-startup","event":"creator_runtime_started","operation":"runtime_init"}
{"chain_id":"mobile-runtime-startup","event":"creator_state_loaded","operation":"state_load"}
{"chain_id":"mobile-upload-chain","event":"creator_state_persisted","operation":"offline_test_publisher_seed"}
{"chain_id":"mobile-upload-chain","event":"creator_upload_session_built","operation":"build_synthetic_upload_session"}
```

The sample `local_dht.json` includes the mobile creator role, onboarding state, and
offline-test Publisher entry. It does not include `identity.json` or signing key material.

The sample `remote_trace_queries.json` includes query hints for:

- local k8s Publisher authority;
- local k8s Publisher receiver;
- local k8s ExitBridges;
- AWS ExitBridge CloudWatch in `ca-central-1`.

## Compatibility Notes

No existing `creator-runner` HTTP/admin files were modified in Phase 2. The mobile crate
depends on `gbn-bridge-creator` and `gbn-bridge-protocol` but does not reshape the Pass 3
admin route contract. Because this phase did not refactor shared request/response types
or local-k8s scripts, the strict Phase 1 k8s Bootstrap and SendDummy reports remain the
runtime compatibility gate for the current branch.

Kotlin compilation and connected-device load tests are deferred to Phase 3 because the
Android app module does not exist yet. Phase 2 verifies the native ABI, Android `.so`
outputs, and exported JNI symbols that Phase 3 will load.
