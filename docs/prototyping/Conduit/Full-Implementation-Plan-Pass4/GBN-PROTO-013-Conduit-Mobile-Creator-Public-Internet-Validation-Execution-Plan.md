# GBN-PROTO-013 - Conduit Mobile Creator Public Internet Validation Execution Plan (Pass 4)

**Document ID:** GBN-PROTO-013
**Status:** Pending
**Last Updated:** 2026-05-11
**Related Docs:**
[GBN-ARCH-001-V2 Media Creation Network](../../../architecture/GBN-ARCH-001-Media-Creation-Network-V2.md),
[GBN-PROTO-012 Pass 3 Architecture-Correct Bootstrap](../Full-Implementation-Plan-Pass3/GBN-PROTO-012-Conduit-Architecture-Correct-Bootstrap-Execution-Plan.md),
[GBN-PROTO-012 Smoke 4 Full Upload](../Full-Implementation-Plan-Pass3/GBN-PROTO-012-Smoke-4-Full-Upload.md),
[README-infra.md](../../../../prototype/gbn-bridge-proto/infra/README-infra.md)

Pass 4 closes the local/mobile validation gap left after Pass 3: Conduit can now prove
the architecture-correct creator flow inside local k8s and ECS-style creator tasks, but
it has not yet proven that a real mobile device can run the creator runtime and move data
over the public internet to Publisher and ExitBridge infrastructure.

The goal is a real Android creator app written in Kotlin with the Rust creator runtime
embedded through a mobile FFI layer. The app must run the same creator logic used by
`creator-runner`: local DHT state, bootstrap, route selection, upload session build,
per-chunk encryption, multi-lane fanout, failover, and ChainID evidence. The app must not
call localhost-only admin endpoints or shell scripts to synthesize success.

Current state:

- Pass 3 local k8s validation is complete for Smoke 1 through Smoke 4.
- The Rust creator implementation lives in `prototype/gbn-bridge-proto/crates/gbn-bridge-creator`.
- The deployed creator process is `creator-runner`, a container/task binary with an admin
  HTTP listener. That binary is not directly embeddable in Android because it owns process
  lifecycle, file paths, admin transport, and server binding.
- Local k8s services are currently ClusterIP-only. A real phone on cellular cannot reach
  them without an explicit public ingress path.
- AWS ECS validation is available, but the existing stack shape treats all services as one
  regional deployment. Pass 4's AWS geo test is intentionally hybrid: the Publisher
  remains the existing local k8s Publisher exposed through the controlled public-internet
  path, while only ExitBridges run in the non-U.S. AWS region.

Pass 4 replaces the remaining synthetic/mobile gap with this flow:

1. Harden the existing Pass 3 bootstrap path so the Publisher bootstrap payload is
   encrypted to the NewCreator, returned opaquely through HostCreator, and validated by
   strict Bootstrap plus SendDummy local-k8s gates.
2. Extract a mobile-safe Rust runtime boundary from `gbn-bridge-creator` and expose it to
   Kotlin through a narrow FFI API while preserving the existing `creator-runner`
   HTTP/admin contract.
3. Build an Android Kotlin creator app that loads the Rust library, owns mobile lifecycle,
   presents debug/operator controls, and persists creator state in app-private storage.
4. Expose the existing local k8s Publisher, HostCreator bootstrap, and ExitBridge protocol
   surfaces to the public internet for controlled validation from a physical mobile
   device.
5. Run the Android app from a real mobile network path against local k8s and archive
   bootstrap, SendDummy, upload, failover, and ChainID evidence.
6. Prepare the hybrid AWS geo topology: keep the Publisher in local k8s, keep its public
   protocol ingress from Phases 4/5, and allow AWS ExitBridges to register with that
   Publisher over the public internet while admin surfaces remain private.
7. Deploy ExitBridges in a non-U.S. AWS region for geolocation validation. The default
   planned region is `ca-central-1` (Canada Central) because it demonstrates non-U.S.
   placement with lower expected latency and cost than Australia for a U.S.-based tester.
8. Run the same Android app against the local k8s Publisher plus non-U.S. AWS ExitBridges
   and archive mobile, local-k8s, and CloudWatch evidence.
9. Update reports, operator documentation, and the README validation status.

## Mobile App Walkthrough

This walkthrough describes how the validation operator uses the Android app once the
local k8s public protocol endpoints and the HostCreator bootstrap QR have been prepared.
It is the intended mental model for the implementation and later smoke docs.

### Trigger Bootstrap

Preparation outside the app:

1. Start the local k8s Publisher, Receiver, HostCreator, and ExitBridge topology.
2. Seed the k8s HostCreator through existing private WSL/k8s operator tooling.
3. Initialize the Publisher DHT with mobile-reachable ExitBridge entries.
4. Generate `BootstrapDHTQRCode` from the seeded HostCreator. The QR contains the
   HostCreator public key, HostCreator DHT entry, mobile-reachable HostCreator bootstrap
   endpoint, reachability metadata, expiry, ChainID/run id, and payload hash/signature
   metadata.
5. Ensure the local k8s Publisher and ExitBridges have public protocol reachability, but
   do not import Publisher DHT or Publisher trust material into the mobile app for
   first-time bootstrap. The mobile NewCreator learns that from the Publisher bootstrap
   payload returned through the HostCreator path.

Mobile app flow:

1. Launch the app on a physical Android device.
2. Select the `local_k8s_public` network profile.
3. Start the embedded Rust runtime from the Runtime screen.
4. Open Bootstrap and scan the HostCreator QR with `HostCreatorDHTQRReader`.
5. Review the preview: HostCreator id, HostCreator public-key fingerprint, endpoint,
   reachability class, expiry, and ChainID.
6. Confirm import. The app calls `importHostCreatorDhtSeed(...)` and inserts the
   HostCreator seed into the mobile NewCreator's app-private local DHT.
7. Tap `BootstrapNewCreator`.

Expected result:

- The app refuses to bootstrap if the QR is expired, the HostCreator public key does not
  match the HostCreator DHT entry, or the HostCreator endpoint is not mobile-reachable.
- The app emits ChainID-tagged events for QR preview, seed import, bootstrap start,
  bootstrap progress, local DHT updates, and terminal bootstrap state.
- The mobile NewCreator contacts the imported HostCreator endpoint over the public/mobile
  path.
- The mobile NewCreator sends its own DHT entry and public key to the HostCreator. The
  HostCreator relays that NewCreator DHT to the Publisher through its existing bridge
  path.
- The Publisher creates the bootstrap payload containing the Publisher public key,
  Publisher DHT entry, the NewCreator entry, and the Seed ExitBridgeB DHT entry. The
  bootstrap payload is encrypted to the NewCreator public key received through the
  HostCreator path.
- The encrypted bootstrap payload returns through the existing path back to the
  HostCreator and then to the mobile NewCreator.
- The mobile NewCreator decrypts the bootstrap payload and persists the Publisher entry
  and Seed ExitBridgeB entry in its local DHT.
- The Publisher has already seeded ExitBridgeB with the remaining bridge DHT set.
- The mobile NewCreator and ExitBridgeB establish the seed tunnel and ACK progress.
- ExitBridgeB returns the signed bridge catalog to the mobile NewCreator.
- The Publisher fans out commands to the remaining ExitBridges so those bridges receive
  the NewCreator DHT and start establishing tunnels with the mobile NewCreator.
- Bootstrap succeeds only when the mobile local DHT reaches `onboarded` or an explicitly
  allowed terminal partial state.
- The app shows the final state, active bridge count, selected ChainID, and evidence
  export status.

### Send Dummy Packet

Preconditions:

1. The mobile runtime is running.
2. Bootstrap has reached `onboarded` or an allowed terminal partial state.
3. The mobile local DHT contains a valid Publisher entry and at least one active,
   non-expired, mobile-reachable ExitBridge entry.

Mobile app flow:

1. Open Creator Actions or Upload.
2. Tap `DumpLocalDht` to verify the Publisher entry and active bridge set.
3. Tap `SendDummy`.
4. Enter or accept the default dummy frame size.
5. Confirm the generated ChainID.

Expected result:

- The app refuses to send if the mobile creator is not onboarded or the local DHT lacks a
  valid Publisher entry and active bridge.
- The Rust runtime selects the route from the mobile local DHT, not from a direct admin
  shortcut.
- The dummy payload is encrypted/enveloped for the Publisher and sent through the selected
  ExitBridge over the public/mobile path.
- The selected ExitBridge forwards ciphertext only; it must not have plaintext evidence.
- The Publisher receiver accepts the frame, decrypts or validates as expected by the
  Pass 3 route semantics, and emits an ACK/result.
- The app shows the assigned bridge id, route source `local_dht`, result state, and
  ChainID.
- Mobile events, local DHT snapshot, selected route, Publisher/ExitBridge trace query
  hints, and the SendDummy result are included in the evidence bundle.
- The operator uploads the evidence bundle to S3 and retrieves it from this workstation
  for report inspection.

## Status Trackers

- `[ ]` Pending
- `[/]` In Progress
- `[x]` Completed

| Phase | Title | Status |
|---|---|---|
| 1 | Bootstrap Hardening And Validation | `[x]` |
| 2 | Mobile Runtime Boundary And FFI | `[x]` |
| 3 | Android Kotlin Creator App | `[ ]` |
| 4 | Local k8s Public Internet Exposure | `[ ]` |
| 5 | Mobile To Local k8s Public Internet Validation | `[ ]` |
| 6 | Hybrid Local-Publisher / AWS-Bridge Topology | `[ ]` |
| 7 | Cross-Region ExitBridge Deployment | `[ ]` |
| 8 | Mobile To AWS Geo Validation | `[ ]` |
| 9 | Reports, Operators, And Acceptance | `[ ]` |
| Smoke 1 | Mobile Runtime | `[ ]` |
| Smoke 2 | Mobile Local-k8s Public Path | `[ ]` |
| Smoke 3 | Mobile AWS Geo Path | `[ ]` |
| Smoke 4 | Mobile Churn / Failover | `[ ]` |

Each phase must update this status tracker when completed.

---

## 1. Gap Inventory

| Gap | Current Behavior | Required Pass 4 Behavior | Phase |
|---|---|---|---|
| Bootstrap path still has prototype shortcuts | Pass 3 proves in-cluster bootstrap but may deliver too much bootstrap data directly and may not enforce encrypted NewCreator-only payload handling | Strict bootstrap encrypts the Publisher payload to NewCreator, returns it opaquely through HostCreator, limits the initial payload to Publisher + Seed ExitBridgeB data, and requires real fanout progress before active state | 1 |
| Creator runtime is process-oriented | `creator-runner` is a container/task binary with admin HTTP ownership | Mobile-safe Rust library exposes creator operations through FFI without requiring admin HTTP or a long-lived process binary | 2 |
| No Android FFI wrapper | Kotlin cannot call `gbn-bridge-creator` directly | `gbn-bridge-mobile-ffi` builds Android `.so` artifacts and generated Kotlin bindings for required ABIs | 2 |
| Runtime lifecycle is not mobile-aware | Container process owns async runtime, logs, and state path | Kotlin app controls start/stop, app-private state directory, foreground-service lifecycle, and event subscription | 2, 3 |
| FFI extraction could regress existing HTTP/admin surfaces | Pass 3 tests and operator scripts rely on `creator-runner` and `gbn-bridge-creator` admin HTTP endpoints | Existing creator-runner HTTP APIs remain supported and Pass 3 smoke tests must stay green after every runtime refactor | 2, 3 |
| No mobile app | Validation uses pods/tasks and scripts | Android Kotlin app can bootstrap, inspect DHT, build/upload media or synthetic payloads, export evidence, and reset state | 3 |
| No mobile HostCreator seed handoff | `SeedNewCreator` injects HostCreator metadata through in-cluster admin HTTP | HostCreator can export a signed bootstrap DHT seed as a QR image, and the Android app can scan/import it into mobile NewCreator local DHT before bootstrap | 2, 3, 5 |
| No physical mobile-network validation | Pass 3 runs inside k8s | Validation must run from a phone with Wi-Fi disabled, over cellular/public internet, and archive carrier/network context | 5, 8 |
| Local k8s is private | Services are ClusterIP-only | Publisher, HostCreator bootstrap, and ExitBridge protocol endpoints have controlled public ingress for local validation; admin remains private | 4 |
| Public endpoint contract is undefined | DHT entries can contain pod-internal addresses | Publisher-signed creator and bridge entries contain mobile-reachable public endpoints and preserve signature validation | 4, 6, 7 |
| AWS topology is single-region | Existing stack deploys Publisher and ExitBridges together | AWS geo validation keeps Publisher local in k8s and runs only public ExitBridges in a non-U.S. region | 6, 7 |
| Cross-environment bridge registration not proven | ExitBridges register from the same local/AWS environment as the Publisher | AWS ExitBridges in `ca-central-1` register with the local k8s Publisher over public internet and appear in the Publisher-signed bridge catalog | 6, 7 |
| ExitBridge geolocation not proven | Smoke runs do not prove non-U.S. bridge placement | At least one mobile hybrid AWS run uses public ExitBridges in `ca-central-1`; optional parity run scales to 10 bridges | 7, 8 |
| Evidence report shape is missing | Existing validation report still has mobile gap | Pass 4 reports archive mobile logs, app build identity, ChainIDs, public endpoint map, k8s/AWS traces, and V1 preservation evidence | 9 |

---

## 2. Execution Rules

### 2.1 Mobile Runtime Truth Rule

The Android app must invoke the Rust creator runtime through the Pass 4 FFI layer. It must
not use shell scripts, `kubectl`, ECS Exec, localhost admin endpoints, or operator-only
seed shortcuts to fake creator success.

Debug UI may expose the same operations as operator scripts, but those buttons must call
the embedded creator runtime or public protocol endpoints. Admin-only endpoints remain
outside the mobile app.

### 2.2 FFI Boundary Rule

The FFI surface must be narrow, typed, and mobile-safe:

- no raw pointers exposed to Kotlin callers;
- no borrowed Rust references crossing the FFI boundary;
- all long-running work returns a handle or emits events;
- all errors are structured and serializable;
- every traceable operation accepts or returns a `chain_id`;
- all persisted state lives under the app-provided state directory;
- local DHT snapshots, runtime events, and trace/evidence bundles are retrievable through
  explicit export APIs.

UniFFI is the default binding approach. A hand-written JNI wrapper is allowed only if a
phase document records the blocker and keeps the same public Kotlin API shape.

### 2.3 Mobile Evidence Retrieval Rule

The mobile app is a remote device during Pass 4 validation, so evidence collection cannot
depend on local filesystem access, `kubectl exec`, or adb-only workflows. The FFI and app
must provide explicit evidence retrieval:

- `localDht()` returns the current creator DHT snapshot for UI/debug inspection.
- `exportEvidence()` writes a complete evidence bundle under app-private storage and
  returns a manifest with file paths, hashes, ChainIDs, and operation IDs.
- The Android app uploads that bundle to an AWS S3 evidence bucket using a short-lived,
  scoped upload grant.
- The workstation retrieves the bundle from S3 with AWS APIs for inspection and report
  generation.
- The Android app can also share/export that bundle through the platform share sheet or
  document picker as a fallback.
- For connected-device lab runs, adb pull is allowed as an additional convenience path,
  but it is not the only retrieval mechanism.
- Every evidence bundle includes mobile runtime events, local DHT snapshots, endpoint
  config, app/device/network context, upload/session summaries, and remote trace
  collection instructions for the matching ChainIDs.

The app does not expose a public mobile admin server. Evidence moves off the phone by
operator action:

1. **S3 evidence upload:** app writes a ZIP and uploads it to a dedicated AWS S3 evidence
   bucket with a short-lived scoped grant.
2. **AWS API retrieval:** this computer downloads the artifact from S3 with AWS CLI/API
   using the bundle id, ChainID, or manifest key.
3. **Share/document export fallback:** app writes a ZIP and the operator sends or saves it using
   Android's share sheet or document picker.
4. **USB lab retrieval:** with the device attached, the operator may pull the exported ZIP
   using adb or MTP.

The mobile app must not embed long-lived AWS credentials. S3 upload uses one of:

- a pre-signed `PUT` URL generated by the workstation/operator;
- a short-lived STS credential scoped to one bucket prefix;
- a small evidence-token service documented in a later phase that returns one of the
  above.

The S3 bucket must use server-side encryption, block public access, lifecycle expiration,
and a per-run prefix such as:

```text
s3://<pass4-evidence-bucket>/mobile-evidence/<run_id>/<chain_id>/<bundle_id>.zip
```

Any future remote-pull mechanism must be opt-in, debug-only, authenticated, time-bounded,
and documented in its owning phase before use.

### 2.3.1 Bootstrap DHT QR Handoff Rule

The local k8s HostCreator remains inside the cluster, while the mobile NewCreator runs on
a physical phone. Pass 4 therefore needs an explicit seed handoff that replaces the
in-cluster `SeedNewCreator` admin injection.

The planned handoff is:

1. Operator prepares/seeds the k8s HostCreator through existing private WSL/k8s tooling.
2. HostCreator exposes `BootstrapDHTQRCode` through private operator tooling. The output is
   a QR PNG plus the exact signed bootstrap seed payload used to render it.
3. The phone scans the QR with `HostCreatorDHTQRReader`.
4. The app validates the payload, verifies HostCreator public key, mobile reachability,
   and expiry, and inserts the HostCreator seed into the mobile NewCreator's app-private
   local DHT.
5. The operator taps `BootstrapNewCreator`; the mobile runtime starts first contact
   through the imported HostCreator DHT seed. The mobile NewCreator sends its own DHT and
   public key to the HostCreator, the HostCreator relays that to the Publisher through
   the existing path, and the mobile NewCreator then learns Publisher and bridge DHT data
   from the Publisher bootstrap payload returned through the HostCreator path.

The QR payload is not a private admin dump of the HostCreator. It contains only the
mobile-safe bootstrap seed subset required by a NewCreator:

- schema/version;
- ChainID/run id;
- HostCreator DHT entry;
- HostCreator public key or public-key id, matching the HostCreator DHT entry;
- mobile-reachable HostCreator bootstrap endpoint metadata, including protocol, host,
  port, TLS/SNI or certificate binding when applicable, and relay/direct reachability
  class;
- expiry and issued-at timestamps;
- payload hash and signature metadata.

The QR payload must not contain private keys, admin listener URLs, Kubernetes service
names that are not mobile-reachable, long-lived AWS credentials, Publisher DHT, Publisher
trust-root material, ExitBridge DHT, or arbitrary HostCreator local-DHT state. Publisher
public key/DHT and Seed ExitBridgeB DHT arrive through the Publisher bootstrap payload,
encrypted to the NewCreator public key that the HostCreator relays in the entry request.

### 2.4 Public Protocol Rule

Pass 4 exposes only protocol surfaces required by mobile creator traffic. Admin listeners
stay private:

- local k8s admin remains reachable only from WSL/k8s operator paths;
- AWS ExitBridge admin remains reachable only through ECS Exec or private operator paths;
- public mobile endpoints must enforce normal protocol authentication/signature checks;
- public ingress must be tear-downable and documented.

### 2.4.1 Mobile Bootstrap Payload Rule

Pass 4 must preserve the README first-time bootstrap flow. The mobile NewCreator does not
receive a separate Publisher bootstrap ingest file or endpoint config containing
Publisher DHT/trust state. The only seed the mobile app imports before bootstrap is the
HostCreator DHT seed from `BootstrapDHTQRCode`.

After the QR import:

1. `BootstrapNewCreator` sends the mobile NewCreator DHT entry and public key to the
   HostCreator.
2. The HostCreator uses its existing bridge path to relay that entry request to the
   Publisher.
3. The Publisher creates a signed bootstrap payload containing the NewCreator entry,
   Publisher public key, Publisher DHT entry, and Seed ExitBridgeB DHT entry.
4. The Publisher encrypts that bootstrap payload to the NewCreator public key received
   through the HostCreator path.
5. The encrypted bootstrap payload returns through the existing path to the HostCreator
   and then to the mobile NewCreator.
6. The mobile NewCreator decrypts and validates the payload, stores the Publisher and
   Seed ExitBridgeB DHT entries, and starts the Seed ExitBridgeB tunnel.
7. ExitBridgeB returns the signed bridge catalog.
8. The Publisher fans out the NewCreator DHT to the remaining ExitBridges so they can
   establish tunnels with the mobile NewCreator.

### 2.5 Creator Runner Compatibility Rule

Pass 4 adds an FFI boundary; it does not replace the existing `creator-runner` HTTP/admin
contract. All Pass 3 creator-runner endpoints and script-driven flows must continue to
work:

- `/v1/admin/node-metadata`
- `/v1/admin/local-dht`
- `/v1/admin/reset-creator-state`
- `/v1/admin/seed-host-creator`
- `/v1/admin/seed-new-creator`
- new QR/export helpers for Pass 4, if added, without changing existing endpoint behavior
- `/v1/admin/send-dummy`
- `/v1/admin/build-upload-session`
- `/v1/admin/upload-sessions*`
- `/v1/admin/send-upload`

Shared Rust code may be refactored so both `creator-runner` and mobile FFI call the same
core implementation, but the existing HTTP API shape, response fields, error codes,
ChainID behavior, persistence semantics, and Pass 3 smoke expectations are compatibility
requirements.

### 2.6 Local Public-Internet Rule

Local k8s validation must be from a physical mobile device over public internet. The
canonical run disables phone Wi-Fi and records carrier/network context.

A development tunnel may be used only as a labeled fallback when it preserves the same
protocol behavior being validated. It cannot replace the canonical public-internet run for
Pass 4 sign-off if it hides UDP, public endpoint descriptors, or bridge reachability.

### 2.7 ChainID Evidence Rule

Every mobile validation operation must preserve a single ChainID across:

- Android app event log;
- Rust mobile runtime log/event stream;
- exported mobile evidence bundle;
- Publisher authority logs;
- Publisher receiver logs;
- every ExitBridge selected for the path;
- validation artifacts.

The mobile app must display and export the ChainID for each bootstrap, SendDummy, upload,
and failover invocation. Any report that claims success without ChainID correlation is
incomplete.

### 2.8 V1 Preservation Rule

Pass 4 does not modify `prototype/gbn-proto/**` or Lattice planning docs. V1 remains a
reference only.

### 2.9 Phase Completion Rule

Each phase must finish with:

- targeted Rust tests for new runtime/FFI behavior;
- Android unit or instrumentation tests when app code changes;
- shell syntax checks for any scripts added in that phase;
- Pass 3 regression coverage when shared creator runtime code changes;
- local k8s or AWS validation when runtime behavior is affected;
- V1 preservation check;
- status tracker update in this document.

### 2.10 WSL2 Ubuntu Baseline Rule

All repo validation, Cargo, Docker, k3d, kubectl, Gradle CLI, AWS CLI, and operator-script
commands in Pass 4 phase docs are intended to be run from WSL2 Ubuntu 22.04 or newer.
Android Studio may be used interactively, but every acceptance command must have a
reproducible WSL2 Ubuntu equivalent.

Each Pass 4 script begins with:

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 4 tooling requires WSL2 Ubuntu" >&2; exit 1; }
```

The minimum WSL2 host allocation remains the Pass 3 baseline unless a later phase raises
it: `memory=10GB`, `processors=6`, `swap=4GB` in `~/.wslconfig` on the Windows host.

---

## 3. Locked Decisions

### 3.1 Mobile App Platform

The first mobile creator app is Android, written in Kotlin. iOS and shared UI frameworks
are out of scope for Pass 4.

### 3.2 Runtime Embedding Shape

The Android app embeds a Rust library, not the `creator-runner` process binary. The mobile
library may reuse `gbn-bridge-creator`, `gbn-bridge-protocol`, and runtime helper modules,
but must not bind an admin HTTP server inside the app.

### 3.3 FFI Implementation Preference

Use UniFFI for generated Kotlin bindings if it supports the required async/event shape
cleanly. If not, use a hand-written JNI wrapper with the same Kotlin-facing API:
`MobileCreatorRuntime`, `CreatorConfig`, `CreatorEvent`, `BootstrapResult`,
`UploadSession`, and `UploadResult`.

### 3.4 Android ABI Set

Required:

- `arm64-v8a` for physical Android devices.
- `x86_64` for emulator/instrumentation tests.

Optional after baseline:

- `armeabi-v7a` only if a test device requires it.

### 3.5 App Roles

The app is a creator app. It must support NewCreator validation. HostCreator-on-device is
allowed as an advanced/debug mode, but the first Pass 4 validation may use a controlled
HostCreator pod/task from the local k8s or AWS test topology while the mobile app proves
the NewCreator path and upload path over the public internet.

### 3.6 Bootstrap Seed Transfer

The primary HostCreator-to-mobile transfer mechanism is QR-based:

- `BootstrapDHTQRCode` is generated from the seeded HostCreator's DHT seed state and
  mobile-reachable HostCreator entry.
- `HostCreatorDHTQRReader` scans or imports that QR payload on Android.
- The mobile runtime validates and stores the imported HostCreator public key,
  HostCreator DHT entry, and mobile-reachable HostCreator bootstrap endpoint under the
  app-private local DHT before `BootstrapNewCreator` is enabled.
- The mobile app must not import Publisher DHT or Publisher public key as a separate
  bootstrap prerequisite. Those values arrive only in the Publisher bootstrap payload
  encrypted to the NewCreator public key.

File import of the same payload is allowed as a lab fallback, but the physical-device
validation must include a real camera scan of the QR code.

### 3.7 Local k8s Public Endpoint Policy

The local public-internet validation must expose only the minimum protocol endpoints
needed by the app. The planned public exposure is documented in Phase 4 and must be
removable after each validation run.

### 3.8 Hybrid AWS Geo Topology

Pass 4's AWS geo test is not a full AWS Publisher deployment. The Publisher remains the
local k8s Publisher from the Phase 4/5 public-internet exposure. AWS contributes only the
non-U.S. ExitBridge fleet. Those bridges register with the local Publisher through its
public protocol endpoint, and the Android app uses the Publisher-signed catalog returned
by that local Publisher.

This keeps the test focused on the unresolved mobile/public-internet and geolocation
questions without replacing the already validated local Publisher with a second AWS
Publisher control plane.

### 3.9 AWS Geo Region Choice

Use `ca-central-1` for the first non-U.S. ExitBridge geolocation run. It demonstrates
non-U.S. bridge placement while keeping expected latency and cost lower than a first run
in Australia from a U.S.-based tester. Australia remains a later optional stress scenario.

### 3.10 Bridge Count Policy

Local k8s validation keeps the Pass 3 10-bridge topology. AWS geo validation may start
with 3 public ExitBridges in `ca-central-1` for a cost-minimum proof, then optionally run
a short 10-bridge parity validation before sign-off if the cost envelope allows it.

### 3.11 Mobile Creator Button Parity

The Android app must expose mobile-safe button equivalents for the relevant creator
operations currently driven by `relay-control-interactive-v2.sh`: status refresh,
catalog/DHT inspection, HostCreator bootstrap QR scan/import, HostCreator seeding in
debug mode, NewCreator bootstrap,
synthetic upload session build, dummy send, upload send, frame/session summary, trace
evidence export, S3 evidence upload, and creator state reset.

Infrastructure and admin-only actions remain in WSL2/k8s/AWS operator tooling. The app
must not call private admin HTTP endpoints from the phone. It may show those actions as
readiness prerequisites or remote evidence query hints.

---

## 4. Phase Summaries

### Phase 1 - Bootstrap Hardening And Validation

[GBN-PROTO-013-Execution-Phase1-Bootstrap-Hardening-And-Validation.md](GBN-PROTO-013-Execution-Phase1-Bootstrap-Hardening-And-Validation.md)

Harden the existing local-k8s bootstrap path before mobile integration: encrypted
Publisher bootstrap payloads, opaque HostCreator relay, Seed ExitBridgeB catalog handoff,
real fanout progress, strict Bootstrap validation, strict SendDummy validation, and Pass 3
compatibility evidence.

### Phase 2 - Mobile Runtime Boundary And FFI

[GBN-PROTO-013-Execution-Phase2-Mobile-Runtime-Boundary-And-FFI.md](GBN-PROTO-013-Execution-Phase2-Mobile-Runtime-Boundary-And-FFI.md)

Extract a mobile-safe Rust boundary around the creator runtime, build Android ABI
artifacts, generate Kotlin bindings, define HostCreator bootstrap seed import, define
event/log export, and prove the library can bootstrap its state and execute no-network
smoke operations from Kotlin tests.

### Phase 3 - Android Kotlin Creator App

[GBN-PROTO-013-Execution-Phase3-Android-Kotlin-Creator-App.md](GBN-PROTO-013-Execution-Phase3-Android-Kotlin-Creator-App.md)

Create the Android app shell, load the Rust runtime, implement debug/operator screens,
foreground upload service, creator capability button panel, state reset/export,
synthetic upload input, HostCreator QR reader/import, ChainID display, S3 evidence
upload, and evidence export.

### Phase 4 - Local k8s Public Internet Exposure

[GBN-PROTO-013-Execution-Phase4-Local-K8s-Public-Internet-Exposure.md](GBN-PROTO-013-Execution-Phase4-Local-K8s-Public-Internet-Exposure.md)

Expose local k8s Publisher, HostCreator bootstrap, and ExitBridge protocol endpoints to
the public internet for a real phone on cellular, without exposing admin endpoints.
Define endpoint descriptors that can be signed into Publisher DHT entries.

### Phase 5 - Mobile To Local k8s Public Internet Validation

[GBN-PROTO-013-Execution-Phase5-Mobile-To-Local-K8s-Validation.md](GBN-PROTO-013-Execution-Phase5-Mobile-To-Local-K8s-Validation.md)

Run the Android app against the local k8s topology from a mobile carrier path. Validate
bootstrap, local DHT, SendDummy, full upload, failover, and ChainID evidence.

### Phase 6 - Hybrid Local-Publisher / AWS-Bridge Topology

[GBN-PROTO-013-Execution-Phase6-Hybrid-Local-Publisher-AWS-Bridge-Topology.md](GBN-PROTO-013-Execution-Phase6-Hybrid-Local-Publisher-AWS-Bridge-Topology.md)

Prepare the hybrid public-internet topology: local k8s Publisher remains the authority
and receiver, while AWS ExitBridges in the selected non-U.S. region can register with it
over public internet. Define DNS, TLS, security group, CloudWatch, k8s log, and teardown
behavior.

### Phase 7 - Cross-Region ExitBridge Deployment

[GBN-PROTO-013-Execution-Phase7-Cross-Region-ExitBridge-Deployment.md](GBN-PROTO-013-Execution-Phase7-Cross-Region-ExitBridge-Deployment.md)

Deploy public ExitBridges in `ca-central-1` and ensure the Publisher-signed bridge catalog
from the local k8s Publisher contains non-U.S. endpoint metadata reachable by the Android
app over the public internet.

### Phase 8 - Mobile To AWS Geo Validation

[GBN-PROTO-013-Execution-Phase8-Mobile-To-AWS-Geo-Validation.md](GBN-PROTO-013-Execution-Phase8-Mobile-To-AWS-Geo-Validation.md)

Run the same Android app against the local k8s Publisher plus Canada ExitBridges. Archive
mobile logs, local k8s logs/traces, CloudWatch logs, ChainIDs, upload/ACK timings, and
failover/churn observations.

### Phase 9 - Reports, Operators, And Acceptance

[GBN-PROTO-013-Execution-Phase9-Reports-Operators-And-Acceptance.md](GBN-PROTO-013-Execution-Phase9-Reports-Operators-And-Acceptance.md)

Update README validation status, the mobile validation matrix, and Pass 4
test reports. Preserve all artifacts and document remaining production blockers.

### Smoke 1 - Mobile Runtime

[GBN-PROTO-013-Smoke-1-Mobile-Runtime.md](GBN-PROTO-013-Smoke-1-Mobile-Runtime.md)

Prove the Android app can load the Rust library, run local runtime operations, expose the
button panel, persist local DHT state, and export/upload evidence before live public
network validation.

### Smoke 2 - Mobile Local-k8s Public Path

[GBN-PROTO-013-Smoke-2-Mobile-Local-K8s-Public-Path.md](GBN-PROTO-013-Smoke-2-Mobile-Local-K8s-Public-Path.md)

Run the physical Android phone over cellular against local k8s public protocol endpoints
and validate QR bootstrap, SendDummy, upload, failover, S3 evidence retrieval, and local
k8s ChainID correlation.

### Smoke 3 - Mobile AWS Geo Path

[GBN-PROTO-013-Smoke-3-Mobile-AWS-Geo-Path.md](GBN-PROTO-013-Smoke-3-Mobile-AWS-Geo-Path.md)

Run the same Android app against the local k8s Publisher plus AWS `ca-central-1`
ExitBridges and validate AWS bridge route/lane use with CloudWatch evidence.

### Smoke 4 - Mobile Churn / Failover

[GBN-PROTO-013-Smoke-4-Mobile-Churn-Failover.md](GBN-PROTO-013-Smoke-4-Mobile-Churn-Failover.md)

Exercise forced bridge/lane failure, suspect marking, reroute or degraded terminal state,
foreground-service continuity, and ChainID evidence across local and hybrid paths.

---

## 5. Full Pass 4 Acceptance Criteria

Pass 4 is complete when:

1. Phase 1 strict Bootstrap validation passes against local k8s with encrypted
   NewCreator-only Publisher bootstrap payload evidence, Seed ExitBridgeB catalog handoff,
   and real remaining-bridge fanout progress.
2. Phase 1 strict SendDummy validation passes against the same hardened local-k8s
   bootstrap path with `route_source=local_dht` and ciphertext-only bridge evidence.
3. `gbn-bridge-mobile-ffi` exposes a stable Kotlin API for creator bootstrap, catalog/DHT
   inspection, upload session build, SendDummy, SendUpload, reset, and event export.
4. Android debug/instrumentation tests prove the Kotlin app can load the Rust library on
   `arm64-v8a` and `x86_64`, and verify the Creator Actions button panel exposes the
   relevant mobile-safe `relay-control-interactive-v2.sh` creator operations.
5. Existing `creator-runner` HTTP/admin APIs remain backward compatible with Pass 3.
6. The full Pass 3 local k8s acceptance suite remains green after the bootstrap hardening
   and mobile FFI/runtime refactor.
7. The Android app can persist and reload creator local DHT state in app-private storage.
8. The app can export a validation evidence bundle from the remote mobile device containing
   ChainIDs, runtime events, local DHT snapshot, app build identity, device/network
   context, run profile config, operation results, file hashes, and remote trace collection
   instructions.
9. Local k8s public-internet validation runs from a physical phone with Wi-Fi disabled.
10. Mobile local-k8s bootstrap imports the HostCreator bootstrap DHT seed by scanning a
   `BootstrapDHTQRCode` from the k8s HostCreator, including HostCreator public key and
   mobile-reachable endpoint information.
11. Mobile local-k8s bootstrap proves there is no separate mobile Publisher ingest:
   Publisher public key/DHT and Seed ExitBridgeB DHT are learned from the encrypted
   Publisher bootstrap payload returned through HostCreator.
12. Mobile local-k8s bootstrap reaches onboarded state without admin shortcuts.
13. Mobile local-k8s full upload completes with content hash match and bridge
   ciphertext-only evidence.
14. Mobile local-k8s failover/churn validation completes and records timing.
15. Hybrid AWS geo validation runs from the same Android app used for local-k8s mobile
   validation.
16. The local k8s Publisher-signed bridge catalog includes AWS non-U.S. bridges in
    `ca-central-1`.
17. Mobile hybrid AWS full upload completes with content hash match and ChainID evidence
    in mobile logs, local k8s Publisher/Receiver logs, and AWS ExitBridge CloudWatch logs.
18. Hybrid trace collection succeeds for the mobile validation ChainID: local k8s
    Publisher/Receiver evidence is collected from k8s logs/observability, and AWS
    ExitBridge evidence is collected from CloudWatch for the `ca-central-1` bridge fleet.
19. README remaining validation gap is updated only after reports are archived.
20. V1 (`prototype/gbn-proto/**`) and Lattice docs are unchanged.

---

## 6. Validation Commands

Run from WSL2 Ubuntu.

```bash
# WSL2 baseline
uname -a | grep -i microsoft >/dev/null || { echo "Pass 4 tooling requires WSL2 Ubuntu" >&2; exit 1; }

# V1 untouched
git diff --stat -- prototype/gbn-proto/
git diff --stat -- docs/prototyping/Lattice/

# Rust workspace
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace

# Existing Pass 3 local baseline remains green
infra/scripts/k8s-up.sh
infra/scripts/k8s-observability-up.sh
infra/scripts/k8s-pass3-acceptance.sh --require-observability

# Android workspace commands, after Phase 3 creates the app
cd ../../..
cd prototype/gbn-bridge-proto/mobile/android
./gradlew test
./gradlew lint
./gradlew assembleDebug
./gradlew connectedDebugAndroidTest
```

Phase-specific docs add the local-public and hybrid AWS-bridge validation scripts.

---

## 7. Out Of Scope

- iOS app.
- Production app-store packaging.
- Consumer-grade UX beyond the debug/operator workflow required for validation.
- Multi-account AWS production topology.
- Australia-region stress validation; Canada is the first geo proof.
- Public admin API exposure.
- V1 Lattice source changes.
