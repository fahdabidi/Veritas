# GBN-PROTO-013 - Execution Phase 3 - Android Kotlin Creator App

**Status:** Complete
**Last Updated:** 2026-05-11
**Parent Plan:** [GBN-PROTO-013](GBN-PROTO-013-Conduit-Mobile-Creator-Public-Internet-Validation-Execution-Plan.md)
**Depends On:** Phase 1 bootstrap hardening and Phase 2 mobile runtime boundary / FFI API shape

## Objective

Create the Android Kotlin app that embeds the Phase 2 Rust creator runtime and gives an
operator enough controls to run Pass 4 mobile validation from a real device. This phase
does not yet require public local-k8s or hybrid AWS-bridge endpoints to be live. It builds
the app shell, lifecycle, state management, synthetic upload controls, event display, and
evidence export needed by later mobile-public validation phases.

At completion:

- The Android project builds from WSL2 Ubuntu with Gradle.
- The app loads the Rust mobile FFI library on the Flutter-managed Android emulator in
  Phase 3. Physical Android phone validation moves to Phase 5 with the public mobile
  network path.
- The app can start and stop `MobileCreatorRuntime`.
- The app can display node metadata, local DHT state, ChainIDs, runtime events, and
  upload-session summaries.
- The app can reset local creator state and export an evidence bundle.
- No production/private admin endpoint is exposed or called.
- Existing `creator-runner` HTTP/admin APIs and Pass 3 smoke tests remain green.

Update the parent plan status tracker when this phase is complete.

---

## Project Layout

Add the Android app under the Conduit prototype workspace:

```text
prototype/gbn-bridge-proto/mobile/android/
  settings.gradle.kts
  build.gradle.kts
  app/
    build.gradle.kts
    src/main/AndroidManifest.xml
    src/main/java/.../MainActivity.kt
    src/main/java/.../runtime/MobileCreatorRuntime.kt
    src/main/java/.../ui/
    src/main/jniLibs/arm64-v8a/
    src/main/jniLibs/x86_64/
    src/test/
    src/androidTest/
```

Use Kotlin and the standard Android Gradle Plugin. Jetpack Compose is allowed for the
debug/operator UI, but the UI must remain simple and validation-oriented.

---

## App Screens

This is a validation app, not a consumer release. Required screens:

| Screen | Purpose |
|---|---|
| Runtime | Start/stop runtime, show app build id, Rust build id, ABI, device id, and state path |
| Network Profile | Select `offline_test`, `local_k8s_public`, or `hybrid_local_publisher_aws_bridges`; import optional run profile/evidence config |
| Creator State | Show node metadata, onboarding state, local DHT counts, active bridge ids, and learned Publisher trust root after bootstrap |
| Creator Actions | Button panel for mobile-safe creator operations derived from `relay-control-interactive-v2.sh` |
| Bootstrap | Scan HostCreator bootstrap DHT QR code, show HostCreator public key/reachability, start mobile bootstrap, and show terminal state |
| Upload | Build synthetic upload session, send upload in later phases, show content hash and chunk count |
| Events | Live event stream filtered by ChainID |
| Evidence | Export evidence bundle and show local path/share intent |
| Reset | Clear creator state after explicit confirmation |

Phase 3 must fully implement Runtime, Network Profile, Creator State, Upload session build
for synthetic/offline mode, HostCreator QR scan/import UI, Events, Evidence, and Reset.
Bootstrap network start and network send buttons may be present but disabled until public
endpoint phases land.

---

## Creator Capability Buttons

The mobile app must expose button-driven equivalents for the creator-relevant actions in
`prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh`. These are
validation controls, so each button must show its ChainID, final state, and evidence
location.

Button rules:

- Buttons call the embedded Rust runtime through FFI or public protocol endpoints only.
- Buttons must not call private `creator-runner` admin HTTP URLs from the phone.
- Disabled buttons must show the missing prerequisite: runtime stopped, HostCreator DHT
  seed missing, Publisher bootstrap payload not received, not onboarded, no upload
  session, or future phase required.
- Every button-driven operation creates or accepts a ChainID and emits an event.
- Button events include `button_id`, `relay_action`, `operation`, and `chain_id`.
- Every result that mutates state or sends data must be included in the evidence bundle.

Required mobile button map:

| Relay control action | Android button | Mobile implementation | Phase behavior |
|---|---|---|---|
| `Status` | `RefreshStatus` | Read runtime status, app build, Rust build, selected profile, and latest error | Phase 3 |
| `Refresh` | `RefreshState` | Reload runtime state, local DHT summary, upload sessions, and event counters | Phase 3 |
| `ShowCatalog`, `DumpBridges`, `DumpPublisherDht` | `RefreshBridgeCatalog` | Read signed public Publisher catalog learned through bootstrap; never admin dump | Disabled until bootstrap/catalog data exists |
| `DumpLocalDht` | `DumpLocalDht` | Call `localDht()` and render/export the local mobile DHT snapshot | Phase 3 |
| `DumpNodeDht` | `DumpNodeState` | Render mobile node metadata plus local DHT; remote node DHT remains operator-only evidence | Phase 3 |
| `AdminMetrics` | `RuntimeMetrics` | Show mobile runtime counters, session counts, and event counts only | Phase 3 |
| `BootstrapDHTQRCode` | `HostCreatorDHTQRReader` | Scan/import QR payload, validate HostCreator public key and mobile reachability, and store HostCreator seed in local DHT | Phase 3 scan/import, Phase 5 live HostCreator |
| `SeedHostCreator` | `SeedHostCreator` | Optional debug HostCreator mode through FFI using public/signed seed inputs | Disabled by default in Phase 3 |
| `SeedNewCreator` | `BootstrapNewCreator` | Start NewCreator bootstrap through FFI using imported HostCreator DHT only; Publisher data arrives in encrypted bootstrap payload | Disabled until Phase 5 |
| `BuildUploadSession` | `BuildUploadSession` | Build synthetic/inline upload session in app-private state | Phase 3 synthetic, later real file inputs |
| `SendDummy` | `SendDummy` | Send dummy frame through onboarded mobile creator runtime | Disabled until Phase 5 |
| `SendUpload` | `SendUpload` | Send selected upload session with lane count and forced-failure options | Disabled until Phase 5 |
| `DumpFrames` | `SessionFrameSummary` | Show local session/dispatch/frame summary and add remote query hints to evidence | Phase 3 local summary, later remote correlation |
| `CollectTraces` | `ExportEvidence` and `UploadEvidenceToS3` | Export DHT/trace bundle and upload ZIP to S3 with short-lived grant | Phase 3 |
| `ResetCreatorState` | `ResetCreatorState` | Confirm, call `resetState(chain_id)`, and clear app-private creator state only | Phase 3 |

Operator-only relay actions must not become mobile buttons:

| Relay control action | Mobile treatment |
|---|---|
| `InitializePublisherDht` | Publisher-side prerequisite shown as readiness/status, executed by operator tooling |
| `StackOutputs`, `TailLogs`, `ExecShell`, `LiveMetrics`, `TriggerCommand`, `CheckImages`, `BootstrapSmoke`, `Teardown` | Infrastructure/admin controls remain in WSL2/k8s/AWS tooling |
| `DiscoveryProbe` | Deprecated Pass 3 probe; not exposed in the mobile app |

The UI may group the buttons by Runtime, Bootstrap, DHT/Catalog, Upload, Evidence, and
Maintenance, but the implementation must preserve the operation names above in test IDs
and event metadata so evidence can be correlated with the relay script.

---

## Android Permissions And Lifecycle

Required permissions:

```xml
<uses-permission android:name="android.permission.INTERNET" />
<uses-permission android:name="android.permission.ACCESS_NETWORK_STATE" />
<uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
<uses-permission android:name="android.permission.CAMERA" />
```

Foreground service:

- Add a foreground service for long-running bootstrap/upload operations.
- The service is required even if Phase 3 only uses offline synthetic session build, so
  later phases do not redesign lifecycle.
- The notification must show the active ChainID and operation name during validation.

State:

- Store runtime state under `context.filesDir/creator-runtime/`.
- Store exportable evidence under `context.cacheDir/evidence-exports/` or a user-selected
  document destination.
- Never store private keys in external/shared storage.

Lifecycle:

- Runtime starts explicitly from the Runtime screen.
- Runtime stops on operator action or app shutdown.
- Rotation/backgrounding must not lose active event history.
- Upload/bootstrap operations must not run on the main thread.
- Camera access is used only for `HostCreatorDHTQRReader`; file import of the same seed
  payload remains available when camera permission is denied.

---

## Evidence Bundle

The app must export a ZIP or directory bundle with:

```text
evidence.json
events.jsonl
trace_events.jsonl
local_dht.json
node_metadata.json
host_creator_seed.redacted.json
upload_sessions.json
endpoint_config.redacted.json
device_context.json
network_context.json
app_build.json
rust_build.json
chain_ids.txt
remote_trace_queries.json
manifest.sha256.json
```

Required `device_context.json` fields:

- manufacturer
- model
- Android SDK version
- ABI
- app version
- install source if available

Required `network_context.json` fields:

- active transport type (`cellular`, `wifi`, `vpn`, `unknown`)
- roaming flag if available
- carrier name if Android exposes it
- public validation note entered by operator
- timestamp

Phase 3 can export `network_context` from the currently available network. Later phases
must require cellular/public-internet context for sign-off.

### Remote Retrieval Workflow

Because the validation phone is remote relative to the WSL/k8s/AWS operator environment,
the app must provide a retrieval workflow that does not require direct filesystem access:

1. Operator taps `ExportEvidence`.
2. App calls Rust `exportEvidence()`.
3. App packages the returned bundle directory as a ZIP.
4. App shows bundle id, ChainIDs, file count, and SHA-256 manifest hash.
5. App uploads the ZIP to the configured AWS S3 evidence bucket using a short-lived,
   scoped upload grant.
6. Operator can also send the ZIP through Android's share sheet or save it through the
   document picker as a fallback.
7. The bundle includes `remote_trace_queries.json`, which tells the WSL/AWS operator which
   local k8s and CloudWatch commands to run for the exported ChainIDs.

For lab runs with the device attached over USB, adb pull is allowed as an additional
retrieval option. S3 upload is the primary remote retrieval path because the canonical
mobile validation can be performed away from the development machine.

The app must not start a public HTTP server on the phone for evidence retrieval in Phase 3.
If a later phase adds a remote-pull helper, it must be opt-in, authenticated, local/debug
only, and time-bounded.

### Transfer To This Workstation

Pass 4 supports three evidence transfer paths from the phone to the development
workstation:

| Transfer Path | When Used | Mechanism | Requirement |
|---|---|---|---|
| S3 evidence upload | Canonical remote path | App uploads evidence ZIP to an AWS S3 bucket with a short-lived scoped upload grant; workstation retrieves with AWS APIs | Must not embed long-lived AWS credentials in the app |
| Share/document export | Fallback remote-friendly path | Android share sheet or document picker sends/saves the evidence ZIP | Must work without adb and without the phone exposing a server |
| USB lab retrieval | Device is physically attached to this computer | `adb pull` or MTP copy of the exported ZIP | Convenience only; not the sole canonical path |

The S3 upload grant is produced outside the app by the operator or a later-phase helper.
Supported grant types:

- pre-signed S3 `PUT` URL for one object key;
- short-lived STS credentials scoped to one bucket prefix;
- later evidence-token service that returns one of the above.

Required object-key shape:

```text
s3://<pass4-evidence-bucket>/mobile-evidence/<run_id>/<chain_id>/<bundle_id>.zip
```

Expected app-side upload config shape:

```json
{
  "upload_mode": "s3_presigned_put",
  "bucket": "veritas-pass4-mobile-evidence",
  "object_key": "mobile-evidence/20260511-test/mobile-chain-id/mobile-bundle.zip",
  "presigned_put_url": "https://s3...",
  "expires_at_ms": 0,
  "expected_sha256": "optional-before-upload"
}
```

Expected workstation retrieval:

```bash
aws s3 cp \
  s3://veritas-pass4-mobile-evidence/mobile-evidence/<run_id>/<chain_id>/<bundle_id>.zip \
  /tmp/veritas-pass4-mobile-evidence/<bundle_id>.zip
```

The bucket must block public access, use server-side encryption, and have lifecycle
expiration. The app records the S3 bucket, object key, ETag if available, upload timestamp,
and local SHA-256 in `evidence.json`.

---

## Run Profile Config Import

The app may import run profile configuration as JSON for lab behavior, evidence routing,
S3 upload grants, and phase labels. This config must not be used as a separate Publisher
bootstrap ingest for first-time mobile onboarding.

For first-time bootstrap, the mobile NewCreator starts with only the HostCreator DHT seed
from `BootstrapDHTQRCode`. Publisher public key, Publisher DHT, and Seed ExitBridgeB DHT
arrive later in the Publisher bootstrap payload encrypted to the NewCreator public key.

Shape:

```json
{
  "profile": "local_k8s_public",
  "run_id": "pass4-mobile-local-k8s-...",
  "evidence_bucket": "veritas-pass4-mobile-evidence",
  "evidence_prefix": "mobile-evidence/<run_id>/",
  "notes": "Publisher and bridge DHT are learned through encrypted bootstrap payload"
}
```

Hybrid AWS-bridge shape:

```json
{
  "profile": "hybrid_local_publisher_aws_bridges",
  "run_id": "pass4-mobile-hybrid-aws-...",
  "aws_exitbridge_region": "ca-central-1",
  "evidence_bucket": "veritas-pass4-mobile-evidence",
  "evidence_prefix": "mobile-evidence/<run_id>/",
  "notes": "Local k8s Publisher remains the bootstrap authority; AWS bridge DHT arrives through Publisher catalog"
}
```

Rules:

- Phase 3 validates JSON shape but does not require Publisher trust root presence.
- The app must reject run profile config that attempts to preload Publisher DHT,
  Publisher public key, bridge catalog, or Seed ExitBridge DHT for first-time bootstrap.
- The app must show a warning for `offline_test` configs.

---

## Bootstrap DHT QR Import

The mobile NewCreator cannot receive `SeedNewCreator` through in-cluster admin HTTP.
The app therefore needs a camera-driven seed import before public bootstrap.

HostCreator side:

- A seeded k8s HostCreator exposes `BootstrapDHTQRCode` through private operator tooling.
- The QR image encodes a signed HostCreator bootstrap DHT seed payload.
- The QR payload includes the HostCreator public key and mobile-reachable endpoint
  information, not private local DHT state.

Android side:

- `HostCreatorDHTQRReader` opens the camera and scans the QR code.
- File import accepts the same payload for emulator/lab fallback.
- The app calls `previewBootstrapDhtQr(payload)` to show HostCreator id, public-key
  fingerprint, reachability class, endpoint host/port, issue time, expiry, and ChainID.
- The app calls `importHostCreatorDhtSeed(...)` only after explicit operator confirmation.
- Successful import writes the HostCreator seed into app-private local DHT and enables
  `BootstrapNewCreator` without requiring Publisher DHT or Publisher trust material to
  be preloaded.

Minimum QR payload shape:

```json
{
  "schema_version": 1,
  "chain_id": "pass4-bootstrap-seed-...",
  "run_id": "pass4-mobile-local-k8s-...",
  "host_creator_id": "host-creator",
  "host_creator_public_key_hex": "REPLACE_ME",
  "host_creator_entry": {},
  "host_creator_reachability": {
    "class": "direct",
    "capabilities": ["bootstrap_seed"]
  },
  "host_creator_bootstrap_endpoints": [
    {
      "protocol": "https",
      "host": "host-creator.example.test",
      "port": 443,
      "tls_sni": "host-creator.example.test"
    }
  ],
  "issued_at_ms": 0,
  "expires_at_ms": 0,
  "payload_hash": "sha256:...",
  "signature": "..."
}
```

Validation rules:

- `host_creator_public_key_hex` must match the public key in `host_creator_entry`.
- At least one HostCreator bootstrap endpoint must be mobile-reachable and must not be a
  cluster-local name, pod IP, localhost, or admin listener.
- QR import must reject payloads that attempt to preload Publisher public key,
  Publisher DHT, Seed ExitBridgeB DHT, or a bridge catalog.
- Expired QR payloads are rejected.
- Imported seed material is included in `host_creator_seed.redacted.json` and
  `local_dht.json` evidence with public keys and endpoints preserved, but no private keys
  or admin URLs.
- `BootstrapNewCreator` sends the mobile NewCreator DHT entry and public key to the
  HostCreator. Publisher public key/DHT and Seed ExitBridgeB DHT are accepted only from
  the encrypted Publisher bootstrap payload returned through the HostCreator path.

---

## Runtime Integration

The app wraps the Phase 2 Kotlin-facing API:

```kotlin
class CreatorViewModel(
    private val runtimeFactory: MobileCreatorRuntimeFactory
) : ViewModel()
```

Required behaviors:

- Start runtime from selected network profile and app-private state directory.
- Subscribe to runtime events and append them to `events.jsonl`.
- Render ChainID-scoped trace events by calling `traceEvents(filter)`.
- Derive a new ChainID for each operator action unless the operator enters one manually.
- Echo the ChainID in UI and evidence exports.
- Display structured errors without dropping the runtime process.
- Keep the event stream bounded in UI memory while preserving full JSONL on disk.

The app integration must not require `creator-runner` to change its HTTP/admin API.
If Phase 3 reveals missing shared runtime primitives, add them underneath both adapters
without removing or renaming the existing Pass 3 HTTP surfaces.

---

## Debug/Operator Actions

Phase 3 actions:

- `StartRuntime`
- `StopRuntime`
- `ImportEndpointConfig`
- `HostCreatorDHTQRReader`
- `PreviewBootstrapDHTQR`
- `ImportHostCreatorDHTSeed`
- `RefreshStatus`
- `RefreshState`
- `RefreshBridgeCatalog`
- `ShowNodeMetadata`
- `DumpLocalDht`
- `DumpNodeState`
- `RuntimeMetrics`
- `ResetCreatorState`
- `BuildUploadSession`
- `SessionFrameSummary`
- `ExportEvidence`
- `UploadEvidenceToS3`

Later phases enable:

- `SeedHostCreator`
- `BootstrapNewCreator`
- `SendDummy`
- `SendUpload`
- `ForceFailoverUpload`
- `CollectRemoteTraceInstructions`

Disabled actions must explain which future phase enables them. They must not silently
perform admin shortcuts.

---

## Tests

Add tests for:

- App builds with Rust `.so` present for `arm64-v8a` and `x86_64`.
- Runtime wrapper loads native library.
- Runtime screen starts and stops `MobileCreatorRuntime`.
- Endpoint config parser accepts valid `local_k8s_public` and
  `hybrid_local_publisher_aws_bridges` run-profile examples without requiring Publisher
  trust root.
- HostCreator DHT QR reader can parse a valid QR payload and file-import fallback.
- HostCreator DHT QR preview shows HostCreator id, public-key fingerprint, reachability,
  endpoint, expiry, and ChainID before import.
- HostCreator DHT QR import rejects missing HostCreator public key, expired payload,
  cluster-local endpoint, localhost/admin endpoint, malformed signature/hash, and any
  Publisher/bridge DHT preload fields.
- HostCreator DHT QR import stores the HostCreator public key and mobile reachability
  metadata in local DHT without marking the NewCreator onboarded.
- Creator action panel renders all required button test IDs from the mobile button map.
- Buttons enable and disable according to runtime state, HostCreator DHT seed state,
  Publisher bootstrap payload receipt, onboarding state, upload-session presence, and
  future-phase gates.
- `RefreshBridgeCatalog` reads imported or public signed catalog data without calling
  private admin endpoints.
- `DumpLocalDht`, `DumpNodeState`, `RuntimeMetrics`, and `SessionFrameSummary` render
  local runtime data and write evidence events.
- Event stream renders at least one runtime event.
- ChainID filter renders only events for the selected ChainID.
- Evidence export writes all required files.
- Evidence ZIP can be shared or saved without adb.
- Evidence ZIP can be uploaded to S3 with a pre-signed URL or short-lived scoped
  credentials.
- Evidence upload result records S3 bucket, object key, ETag if available, timestamp, and
  local SHA-256.
- Evidence bundle includes `local_dht.json`, `host_creator_seed.redacted.json`,
  `trace_events.jsonl`, `remote_trace_queries.json`, and `manifest.sha256.json`.
- Reset requires confirmation and clears only app-private creator state.
- Synthetic upload session build returns non-empty `session_id`, `content_hash`, and
  chunk count.
- App does not request unsupported dangerous permissions.
- App does not call localhost admin URLs except explicit test fixtures under
  `offline_test`.
- App does not call private k8s/AWS admin URLs from mobile actions.

Run inside WSL2 Ubuntu:

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 4 tooling requires WSL2 Ubuntu" >&2; exit 1; }

# V1 untouched
git diff --stat -- prototype/gbn-proto/
git diff --stat -- docs/prototyping/Lattice/

# Rust workspace remains green
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace

# Android app
cd mobile/android
./gradlew test
./gradlew lint
./gradlew assembleDebug
./gradlew connectedDebugAndroidTest

# Pass 3 compatibility after app/runtime integration changes
cd ../..
infra/scripts/k8s-up.sh
infra/scripts/k8s-observability-up.sh
infra/scripts/k8s-pass3-acceptance.sh --require-observability
```

---

## Manual Device Smoke

Run on the Flutter-managed Android emulator during Phase 3 after installing the debug
APK. Repeat on a physical Android phone in Phase 5 when public mobile-network validation
begins:

1. Launch app.
2. Import an optional `offline_test` run profile config.
3. Start runtime.
4. Confirm node metadata shows role `creator`.
5. Scan or file-import a sample `BootstrapDHTQRCode` payload with
   `HostCreatorDHTQRReader`.
6. Confirm preview shows HostCreator id, public-key fingerprint, reachability, endpoint,
   expiry, and ChainID.
7. Import the HostCreator DHT seed and confirm local DHT shows a NewCreator seed state,
   not onboarded state.
8. Tap `RefreshStatus`, `RefreshState`, `DumpLocalDht`, `DumpNodeState`, and
   `RuntimeMetrics`.
9. Tap `RefreshBridgeCatalog` against imported offline catalog data.
10. Build a 1 MiB synthetic upload session with `BuildUploadSession`.
11. Tap `SessionFrameSummary`.
12. Confirm Events screen shows ChainID-tagged runtime events.
13. Open the ChainID filter and confirm only the active ChainID's events are displayed.
14. Export evidence bundle.
15. Upload the bundle ZIP to S3 using a short-lived upload grant.
16. Confirm the ZIP contains `local_dht.json`, `host_creator_seed.redacted.json`,
    `trace_events.jsonl`, `remote_trace_queries.json`, and `manifest.sha256.json`.
17. Reset creator state.
18. Restart app.
19. Confirm reset state persisted and old local DHT entries are gone.

Expected:

- No crash.
- No main-thread network error.
- Evidence bundle contains required files.
- `events.jsonl` includes `creator_runtime_started`, `creator_upload_session_built`,
  and `creator_evidence_exported`.
- `events.jsonl` includes `creator_bootstrap_dht_qr_previewed` and
  `creator_host_dht_seed_imported`.
- `events.jsonl` includes events for the button operations run during the smoke.
- `remote_trace_queries.json` includes query hints for local k8s Publisher surfaces and,
  when the hybrid profile is selected, AWS ExitBridge CloudWatch surfaces.

---

## Acceptance Criteria

- Android project exists under `prototype/gbn-bridge-proto/mobile/android`.
- App builds a debug APK from WSL2 Ubuntu.
- App loads the Rust mobile FFI library on the Android emulator in Phase 3; physical
  phone load validation is deferred to Phase 5.
- Runtime, Network Profile, Creator State, Bootstrap, Upload, Events, Evidence, and Reset
  screens exist and are usable for validation.
- Bootstrap screen can scan `BootstrapDHTQRCode`, preview HostCreator public key and
  mobile reachability, and import the HostCreator DHT seed into local state.
- Imported HostCreator seed enables `BootstrapNewCreator` only after runtime state,
  HostCreator public key, reachability, and expiry checks pass. Publisher trust is learned
  from the encrypted Publisher bootstrap payload.
- Creator Actions screen exposes mobile-safe equivalents for relevant
  `relay-control-interactive-v2.sh` creator operations.
- Infrastructure/admin-only relay actions remain outside the mobile app and are shown only
  as prerequisites or operator status.
- App-private state path is used for identity, local DHT, upload sessions, and events.
- Evidence export contains all files listed above and no private key material.
- Evidence export is retrievable from a remote phone through Android share/document export,
  not only through adb.
- Evidence export is uploaded to S3 with no long-lived AWS credentials embedded in the app.
- Workstation retrieval through `aws s3 cp` or equivalent AWS SDK/API can fetch the same
  bundle by object key for inspection.
- UI can display local DHT snapshots and ChainID-scoped runtime trace events for debugging.
- UI can show signed Publisher/bridge catalog data without using private admin endpoints.
- Synthetic upload session build works through the embedded Rust runtime.
- Disabled future network actions do not call admin shortcuts.
- Existing `creator-runner` HTTP/admin APIs remain compatible with Pass 3 operator and
  smoke scripts.
- Pass 3 Smoke 1 through Smoke 4 still pass against local k8s.
- Android unit tests and connected instrumentation tests pass.
- Rust workspace remains green.
- V1 preservation checks return no files.
- Parent plan status tracker is updated.

---

## Completion Evidence

Phase 3 implementation report:
[GBN-PROTO-013-Phase3-Android-Kotlin-Creator-App-20260511-223257.md](Test-Reports/GBN-PROTO-013-Phase3-Android-Kotlin-Creator-App-20260511-223257.md)

When this phase is implemented, archive:

- `./gradlew test` output.
- `./gradlew lint` output.
- `./gradlew assembleDebug` output.
- `./gradlew connectedDebugAndroidTest` output.
- APK path and SHA-256 hash.
- Emulator model/SDK/ABI for Phase 3, and physical device model/SDK/ABI in Phase 5.
- Screenshot or instrumentation capture showing the Creator Actions button panel.
- Screenshot or instrumentation capture showing `HostCreatorDHTQRReader` preview with
  HostCreator public-key fingerprint and endpoint redacted as needed.
- Sample `BootstrapDHTQRCode` payload and matching `host_creator_seed.redacted.json`.
- Sample evidence bundle from the manual device smoke.
- Evidence ZIP retrieval method used during the manual smoke.
- If S3 upload was used, bucket name, object key, ETag if available, local SHA-256, and
  `aws s3 cp` retrieval output.
- Pass 3 acceptance output proving no `creator-runner` regression.
- V1 preservation command output.
