# GBN-PROTO-013 - Execution Phase 2 - Mobile Runtime Boundary And FFI

**Status:** Pending
**Last Updated:** 2026-05-11
**Parent Plan:** [GBN-PROTO-013](GBN-PROTO-013-Conduit-Mobile-Creator-Public-Internet-Validation-Execution-Plan.md)

## Objective

Create the mobile-safe Rust boundary that lets Kotlin run the Conduit creator runtime
inside an Android app. This phase does not build the full app UI and does not expose
local k8s or AWS to the public internet. It only proves that the Rust creator capability
can be packaged, loaded, called, persisted, and observed from Kotlin.

At completion:

- Android can load a Rust shared library for `arm64-v8a` and `x86_64`.
- Kotlin can create a `MobileCreatorRuntime` with app-provided config and state path.
- Kotlin can subscribe to creator runtime events and export them as JSONL.
- Runtime state persists under an app-provided directory.
- No admin HTTP listener or container process binary is required.
- Existing `creator-runner` HTTP/admin APIs continue to work unchanged.
- V1 remains untouched.

Update the parent plan status tracker when this phase is complete.

---

## Runtime Boundary

`creator-runner` remains the container/task binary used by k8s and ECS. Android must not
embed or spawn that binary. Phase 2 introduces a library boundary that wraps the reusable
creator logic:

```text
prototype/gbn-bridge-proto/crates/gbn-bridge-mobile-ffi/
```

The crate depends on:

- `gbn-bridge-creator`
- `gbn-bridge-protocol`
- existing runtime/observability helpers where they do not assume a server process

The crate must not depend on:

- `gbn-bridge-cli`
- `axum` admin server wiring
- Kubernetes or ECS discovery helpers
- shell scripts

## Creator Runner Compatibility

Phase 2 may extract shared creator runtime code, but it must not remove or reshape the
existing `creator-runner` HTTP/admin contract used by Pass 3.

The mobile FFI crate and `creator-runner` should share core logic where practical:

```text
gbn-bridge-creator core runtime
  -> creator-runner HTTP/admin adapter
  -> gbn-bridge-mobile-ffi Kotlin/Android adapter
```

Compatibility requirements:

- Existing `creator-runner` routes remain available.
- Existing JSON request/response fields remain stable unless a Pass 3 migration doc is
  explicitly written and accepted.
- Existing error codes remain stable.
- ChainID query/body behavior remains stable.
- PVC/EFS/container-local persistence behavior remains stable.
- Pass 3 scripts continue to call HTTP/admin endpoints; they do not switch to FFI.

Any refactor that touches shared creator runtime behavior must run Pass 3 regression
coverage before Phase 2 is marked complete.

### Binding Strategy

Default: UniFFI.

Fallback: hand-written JNI only if UniFFI cannot represent the required callback/event
shape. A JNI fallback must preserve the same Kotlin-facing API names and object model so
Phase 3 does not need to know which binding generator is underneath.

### Crate Output

Required crate outputs:

```toml
[lib]
crate-type = ["cdylib", "rlib"]
```

Android artifacts:

```text
mobile/android/app/src/main/jniLibs/arm64-v8a/libgbn_bridge_mobile_ffi.so
mobile/android/app/src/main/jniLibs/x86_64/libgbn_bridge_mobile_ffi.so
```

Generated or hand-written Kotlin bindings live under the Android app module created in
Phase 3. Until Phase 3 lands, Phase 2 may place binding fixtures under:

```text
prototype/gbn-bridge-proto/mobile/bindings-fixtures/
```

---

## Kotlin-Facing API Shape

The exact generated code may vary, but the public Kotlin wrapper must expose this shape:

```kotlin
class MobileCreatorRuntime(config: CreatorRuntimeConfig) : AutoCloseable {
    fun nodeMetadata(): NodeMetadata
    fun localDht(): LocalDhtSnapshot
    fun traceEvents(filter: TraceEventFilter): List<CreatorTraceEvent>
    fun resetState(chainId: String): ResetResult
    fun seedHostCreator(request: SeedHostCreatorRequest): SeedHostCreatorResult
    fun previewBootstrapDhtQr(payload: String): BootstrapDhtQrPreview
    fun importHostCreatorDhtSeed(request: HostCreatorDhtSeedImportRequest): HostCreatorDhtSeedImportResult
    fun bootstrapNewCreator(request: BootstrapNewCreatorRequest): BootstrapResult
    fun refreshBridgeCatalog(request: RefreshBridgeCatalogRequest): BridgeCatalogSnapshot
    fun buildSyntheticUploadSession(request: BuildSyntheticUploadRequest): UploadSession
    fun sendDummy(request: SendDummyRequest): SendDummyResult
    fun sendUpload(request: SendUploadRequest): UploadResult
    fun exportEvidence(): EvidenceBundle
    fun subscribeEvents(sink: CreatorEventSink): Subscription
}
```

Phase 2 may implement only the non-network subset below, but the API must be shaped for
the full Pass 4 path:

- `nodeMetadata`
- `localDht`
- `traceEvents`
- `resetState`
- `previewBootstrapDhtQr`
- `importHostCreatorDhtSeed`
- `refreshBridgeCatalog`
- `buildSyntheticUploadSession`
- `exportEvidence`
- `subscribeEvents`

Network operations (`seedHostCreator`, `bootstrapNewCreator`, `sendDummy`, and
`sendUpload`) can return `not_implemented` until the later validation phases wire public
endpoints.

`refreshBridgeCatalog` is the mobile-safe equivalent of the operator script's
Publisher/bridge catalog inspection. It must use Publisher-signed catalog state learned
through bootstrap/refresh or optional run-profile hints, not private admin URLs.

`previewBootstrapDhtQr` and `importHostCreatorDhtSeed` are the mobile side of the
`BootstrapDHTQRCode` handoff. They parse a QR payload, verify that the HostCreator public
key matches the HostCreator DHT entry, validate mobile-reachable HostCreator endpoint
metadata, reject expired payloads, and persist only the HostCreator seed subset into
app-private local DHT. They must not require a Publisher public key or Publisher DHT
before first-time bootstrap.

### `CreatorRuntimeConfig`

Required fields:

| Field | Meaning |
|---|---|
| `state_dir` | App-private directory for local DHT, identity, upload sessions, and evidence |
| `publisher_public_key_hex` | Optional before first-time bootstrap; learned from encrypted Publisher bootstrap payload, required for returning creator/catalog verification |
| `creator_id` | Stable app/device creator identity; generated if omitted |
| `network_profile` | `offline_test`, `local_k8s_public`, or `hybrid_local_publisher_aws_bridges` |
| `endpoint_config_json` | Optional run profile/evidence config; first-time bootstrap must not depend on imported Publisher DHT or Publisher trust material |
| `log_level` | Runtime log level |
| `evidence_dir` | Optional export directory; defaults under `state_dir` |

No global filesystem paths are allowed. All file I/O must stay under `state_dir` or
`evidence_dir`.

### `HostCreatorDhtSeed`

The QR handoff and file-import fallback use one shared, typed seed payload. The exact
serialization can be compact JSON, CBOR, or a signed envelope, but the Kotlin-facing
model must expose at least:

| Field | Meaning |
|---|---|
| `schema_version` | Payload version for forward-compatible QR parsing |
| `chain_id` | ChainID assigned when the seed was produced |
| `run_id` | Operator validation run id |
| `host_creator_id` | HostCreator actor id |
| `host_creator_public_key_hex` | HostCreator public key used to authenticate first contact |
| `host_creator_entry` | HostCreator DHT entry required for first contact |
| `host_creator_reachability` | Direct/relay reachability class and capabilities |
| `host_creator_bootstrap_endpoints` | Mobile-reachable protocol endpoints with host, port, TLS/SNI or certificate binding when applicable |
| `issued_at_ms` | Seed creation time |
| `expires_at_ms` | Seed expiry time |
| `payload_hash` | Hash of the canonical payload |
| `signature` | Signature or envelope metadata for the seed payload |

Rules:

- `host_creator_public_key_hex` must match the public key in `host_creator_entry`.
- QR import must not require Publisher public key, Publisher DHT, or Seed ExitBridge DHT.
- At least one endpoint must be reachable from the phone over the public/mobile network.
- Admin listener URLs, cluster-local DNS names, private keys, and arbitrary HostCreator
  local-DHT fields are rejected.
- Import writes the HostCreator seed into the mobile NewCreator local DHT as
  `new_creator_seed_state` or its mobile-safe equivalent; it must not mark the mobile
  NewCreator onboarded before `bootstrapNewCreator` completes.
- `bootstrapNewCreator()` sends the mobile NewCreator DHT entry and public key to the
  HostCreator. Publisher public key/DHT and Seed ExitBridgeB DHT are accepted only from
  the encrypted Publisher bootstrap payload returned through the HostCreator path.

### `BootstrapDHTQRCode` Producer Contract

`BootstrapDHTQRCode` is a HostCreator/operator capability, not a mobile public admin
surface. It may be implemented as a private `creator-runner` admin endpoint, a WSL2
operator script that calls existing private endpoints, or a shared Rust helper used by
both. In all cases it must emit the same canonical `HostCreatorDhtSeed` payload and QR
image.

Required producer inputs:

- seeded HostCreator local DHT seed state;
- HostCreator DHT entry;
- HostCreator public key;
- mobile-reachable HostCreator bootstrap endpoint metadata;
- run id and ChainID;
- expiry.

Required producer outputs:

- QR PNG or SVG image for scanning by `HostCreatorDHTQRReader`;
- canonical payload file used to render the QR;
- payload hash and signature metadata;
- redacted evidence file proving which HostCreator public key and endpoint were encoded.

The producer must reject HostCreator endpoints that resolve only inside k8s, pod IPs,
localhost, or admin listeners. It must not include private keys or full HostCreator local
DHT dumps. It also must not include Publisher public key, Publisher DHT, or ExitBridge DHT
as a first-time bootstrap shortcut; those are delivered later by the Publisher bootstrap
payload encrypted to the NewCreator public key.

---

## Event Model

The Rust runtime emits structured events instead of requiring log scraping inside the app.

Minimum event fields:

```json
{
  "timestamp_ms": 0,
  "chain_id": "mobile-runtime-smoke-...",
  "event": "creator_runtime_started",
  "severity": "info",
  "actor_id": "mobile-creator-...",
  "operation": "runtime_init",
  "details": {}
}
```

Required Phase 2 events:

- `creator_runtime_started`
- `creator_state_loaded`
- `creator_state_persisted`
- `creator_state_reset`
- `creator_catalog_refreshed`
- `creator_host_seeded`
- `creator_bootstrap_dht_qr_previewed`
- `creator_host_dht_seed_imported`
- `creator_bootstrap_started`
- `creator_bootstrap_completed`
- `creator_upload_session_built`
- `creator_evidence_exported`
- `creator_runtime_error`

Events must be available through:

- Kotlin callback subscription for live UI.
- `traceEvents(filter)` for bounded inspection by ChainID, operation, event name, or time
  window.
- JSONL evidence export for reports.

---

## Evidence And Remote Retrieval

The mobile device is remote during Pass 4 validation. Phase 2 therefore treats evidence
export as part of the FFI contract, not a UI convenience.

`exportEvidence()` must:

1. Flush pending runtime events to disk.
2. Snapshot `node_metadata`.
3. Snapshot the full `local_dht`.
4. Snapshot upload session summaries and dispatch plans.
5. Include imported HostCreator seed summary, excluding QR raw payload secrets if any.
6. Include Publisher bootstrap payload summary after first-time bootstrap, redacted but
   showing Publisher public key id, Publisher DHT entry id, Seed ExitBridgeB id, and
   encryption/decryption status.
7. Include the active run profile config, excluding secrets.
8. Include all ChainIDs known to the runtime.
9. Include runtime build metadata and ABI.
10. Include a manifest with SHA-256 hashes for every exported file.
11. Include remote trace collection instructions that name the ChainIDs and expected remote
   surfaces:
   - local k8s Publisher authority;
   - local k8s Publisher receiver;
   - local k8s ExitBridges for local-only validation;
   - AWS CloudWatch log groups for hybrid AWS ExitBridges.

Minimum `EvidenceBundle` shape:

```json
{
  "bundle_id": "mobile-evidence-...",
  "created_at_ms": 0,
  "state_dir": "/app-private/redacted",
  "chain_ids": ["mobile-..."],
  "files": [
    {
      "path": "events.jsonl",
      "sha256": "..."
    }
  ],
  "remote_trace_queries": [
    {
      "chain_id": "mobile-...",
      "surface": "local_k8s_publisher_authority",
      "query_hint": "kubectl logs deploy/publisher-authority ..."
    },
    {
      "chain_id": "mobile-...",
      "surface": "aws_exitbridge_cloudwatch",
      "region": "ca-central-1",
      "query_hint": "aws logs filter-log-events ..."
    }
  ]
}
```

The FFI layer returns the bundle manifest to Kotlin. The Android app owns how the files are
uploaded to S3 or otherwise shared off-device.

Rules:

- Evidence export must not include private signing keys.
- Evidence export must not include long-lived AWS credentials.
- Local DHT export must include enough bridge entry fields to debug signature, expiry,
  endpoint, reachability, and active/suspect state.
- Trace events must be retained in a bounded on-device log with rotation. Rotation must
  preserve the active validation ChainIDs until export completes.
- Export must work without adb. S3 upload is the primary remote path; adb pull is allowed
  only as an additional lab convenience.
- No public mobile admin listener is introduced in Phase 2.

---

## State And Persistence

The mobile runtime reuses the Pass 3 local DHT semantics, but the storage backend is
app-private filesystem state instead of a k8s PVC or ECS EFS volume.

Required files under `state_dir`:

```text
identity.json
local_dht.json
upload_sessions/
evidence/events.jsonl
```

Rules:

- Startup creates missing state files.
- Startup validates existing returning-creator state against the persisted Publisher trust
  root when one is already present.
- Invalid or expired DHT entries are pruned or marked invalid according to existing
  `gbn-bridge-creator` validation semantics.
- Reset deletes creator-local state only under `state_dir`; it cannot remove arbitrary
  paths.
- Evidence export must not include private signing keys.

---

## Runtime And Threading

Android lifecycle is different from a container process. The FFI layer must own a runtime
handle that can start and stop cleanly:

- Create at most one Tokio runtime per `MobileCreatorRuntime` unless a later phase proves
  a shared runtime is safer.
- Do not block the Android main thread.
- Every long-running operation must be cancellable or bounded by a timeout.
- `close()` flushes state, stops event workers, and releases native resources.
- Panics must be caught and surfaced as structured runtime errors.

---

## Build Plan

Add a script in a later implementation pass:

```text
prototype/gbn-bridge-proto/infra/scripts/build-mobile-ffi.sh
```

Expected behavior:

```bash
bash prototype/gbn-bridge-proto/infra/scripts/build-mobile-ffi.sh \
  --abi arm64-v8a \
  --abi x86_64 \
  --profile debug
```

The script must:

1. Guard for WSL2 Ubuntu.
2. Verify Android NDK toolchain availability.
3. Install or verify Rust Android targets.
4. Build `gbn-bridge-mobile-ffi`.
5. Copy `.so` outputs into the Android app module.
6. Generate Kotlin bindings when UniFFI is used.
7. Write a build metadata file with git SHA, target ABI, Rust version, and timestamp.

---

## Tests

Add focused tests for:

- FFI config validation rejects missing `state_dir`.
- FFI config rejects state paths outside app-provided root.
- Runtime startup creates `identity.json`, `local_dht.json`, and `events.jsonl`.
- Runtime restart reloads the same identity and local DHT snapshot.
- Reset removes local DHT and upload sessions while preserving evidence export metadata.
- Bootstrap QR preview rejects malformed payloads, missing HostCreator public key,
  expired seed, Publisher/ExitBridge bootstrap shortcut fields, and non-mobile-reachable
  endpoints.
- Bootstrap QR import persists HostCreator public key, HostCreator DHT entry, and
  reachability metadata into mobile local DHT without marking the creator onboarded.
- `BootstrapDHTQRCode` producer rejects cluster-local HostCreator endpoints, admin
  listener URLs, missing HostCreator public key, expired seed metadata, and any embedded
  Publisher DHT or ExitBridge DHT shortcut.
- Bootstrap payload handling accepts Publisher public key/DHT and Seed ExitBridgeB DHT
  only from a Publisher payload encrypted to the NewCreator public key.
- Synthetic upload session build works through the mobile runtime boundary and produces
  the same manifest/content hash behavior as Pass 3.
- `refreshBridgeCatalog()` returns signed public catalog data without using admin HTTP.
- Event subscription receives runtime start, state loaded, session built, and evidence
  exported events.
- `localDht()` returns the same DHT snapshot written to the evidence bundle.
- `traceEvents(filter=chain_id)` returns only events for the requested ChainID.
- `exportEvidence()` writes a manifest with file hashes and remote trace query hints.
- Panic/error path returns structured `CreatorRuntimeError` rather than aborting the app.
- Android `arm64-v8a` library loads in a connected physical device test.
- Android `x86_64` library loads in emulator/instrumentation tests.

Run inside WSL2 Ubuntu:

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 4 tooling requires WSL2 Ubuntu" >&2; exit 1; }

# V1 untouched
git diff --stat -- prototype/gbn-proto/
git diff --stat -- docs/prototyping/Lattice/

# Rust workspace
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace

# Mobile FFI crate tests, once the crate exists
cargo test -p gbn-bridge-mobile-ffi

# Pass 3 creator-runner compatibility after shared runtime refactors
infra/scripts/k8s-up.sh
infra/scripts/k8s-observability-up.sh
infra/scripts/k8s-pass3-acceptance.sh --require-observability

# Android load tests, once Phase 3 app skeleton exists
cd mobile/android
./gradlew test
./gradlew connectedDebugAndroidTest
```

---

## Acceptance Criteria

- `gbn-bridge-mobile-ffi` exists and builds as a Rust library with `cdylib` output.
- Android ABI artifacts are produced for `arm64-v8a` and `x86_64`.
- Kotlin can instantiate `MobileCreatorRuntime` without starting an admin HTTP listener.
- `nodeMetadata()` returns a stable mobile creator actor id and role `creator`.
- `localDht()` returns the actual persisted mobile local DHT snapshot.
- `traceEvents()` supports ChainID-scoped inspection of distributed-trace-relevant mobile
  runtime events.
- `resetState(chainId)` changes state and emits ChainID-tagged events.
- `previewBootstrapDhtQr()` and `importHostCreatorDhtSeed()` validate and persist a
  HostCreator seed containing HostCreator public key and mobile reachability information.
- `BootstrapDHTQRCode` producer emits a QR payload and evidence manifest containing the
  HostCreator public key and mobile endpoint metadata, without private keys or admin URLs.
- First-time mobile bootstrap does not require imported Publisher public key/DHT; those
  are learned from the encrypted Publisher bootstrap payload.
- `refreshBridgeCatalog()` exposes bridge catalog state to Kotlin through public protocol
  data, not admin endpoints.
- `bootstrapNewCreator()` and optional `seedHostCreator()` have stable Kotlin-visible
  request/result types even if their network implementations arrive in later phases.
- `buildSyntheticUploadSession()` produces a durable session and event evidence.
- `exportEvidence()` produces a retrievable bundle manifest that includes local DHT,
  runtime trace events, ChainIDs, file hashes, and remote trace collection instructions.
- Native panics or Rust errors return structured Kotlin-visible errors.
- Evidence export excludes private key material.
- `creator-runner` still exposes the Pass 3 HTTP/admin endpoints without response-shape
  regressions.
- Pass 3 Smoke 1 through Smoke 4 still pass against local k8s after any shared runtime
  extraction.
- WSL2 Ubuntu validation commands pass.
- V1 preservation checks return no files.
- Parent plan status tracker is updated.

---

## Completion Evidence

When this phase is implemented, archive:

- Rust test output.
- Android unit/instrumentation output.
- ABI artifact list and hashes.
- Generated binding files or JNI wrapper source paths.
- Sample `events.jsonl`.
- Sample `EvidenceBundle` JSON.
- Sample `local_dht.json` exported through FFI.
- Sample ChainID-scoped `traceEvents()` output.
- Pass 3 acceptance output proving `creator-runner` HTTP/admin compatibility.
- V1 preservation command output.
