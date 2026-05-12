# GBN-PROTO-013 - Execution Phase 5 - Mobile To Local k8s Public Internet Validation

**Status:** Pending
**Last Updated:** 2026-05-12
**Parent Plan:** [GBN-PROTO-013](GBN-PROTO-013-Conduit-Mobile-Creator-Public-Internet-Validation-Execution-Plan.md)
**Depends On:** Phases 1-4 complete

## Objective

Run the Android creator app from a physical mobile device over a real mobile-network path
against the local k8s Publisher, HostCreator, and ExitBridges exposed in Phase 4.

This is the first Pass 4 end-to-end mobile validation. The phone must not use private
admin endpoints, kubectl, adb-only evidence retrieval, or a preloaded Publisher/bridge
bootstrap ingest.

At completion:

- the Android app scans a real `BootstrapDHTQRCode`;
- mobile NewCreator starts with only HostCreator DHT seed material;
- Publisher public key/DHT and Seed ExitBridgeB DHT arrive in the encrypted Publisher
  bootstrap payload returned through HostCreator;
- mobile bootstrap reaches `onboarded` or a documented allowed partial terminal state;
- mobile `SendDummy` succeeds through a route selected from local mobile DHT;
- mobile full upload succeeds against local k8s;
- forced failover/churn path produces evidence;
- mobile evidence is uploaded to S3 and retrieved on this workstation;
- local k8s logs/traces correlate by ChainID.

Update the parent plan status tracker when this phase is complete.

---

## Preconditions

Required before running Phase 5:

- Phase 1 strict Bootstrap and SendDummy validations are green.
- Phase 2 mobile FFI builds for `arm64-v8a`.
- Phase 3 Android debug APK installs and passes the manual device smoke.
- Phase 4 public endpoint map is active and generated from a live operator profile, not
  `run-profile.local-k8s-public.example.json`.
- HostCreator QR seed is generated from the active public endpoint map.
- S3 evidence bucket and short-lived upload grant are prepared.
- Phone has cellular service; canonical run disables Wi-Fi.

The operator records the device model, Android version, ABI, carrier/network context, app
build id, Rust build id, endpoint map id, and run id before the live run starts.

---

## Gap Closure Workstreams

The `2026-05-12` Phase 5 attempt proved that the local-k8s strict prerequisites are green,
but the live mobile run is blocked by four concrete gaps. Phase 5 must close all of them
before the physical-phone smoke can be signed off.

### 1. Live `local_k8s_public` Profile

Create a live profile, for example:

```text
prototype/gbn-bridge-proto/infra/pass4/public-ingress/run-profile.local-k8s-public.live.json
```

The live profile must replace every example hostname with operator-owned public DNS names
or public IPs that resolve from the mobile carrier path:

- Publisher authority HTTPS endpoint;
- Publisher receiver HTTPS endpoint;
- HostCreator bootstrap HTTPS endpoint;
- each selected ExitBridge UDP endpoint;
- admin-denial URLs for every private/admin surface that must remain unreachable.

Required profile fields:

- public host, protocol, and port for each endpoint;
- TLS SNI and certificate fingerprint for every HTTPS endpoint;
- endpoint expiry timestamp;
- HostCreator actor id and public key;
- shared run `chain_id`;
- no localhost, RFC1918/private, cluster-local, pod DNS, NodePort-only, or admin endpoint
  shortcuts in mobile-facing DHT descriptors.

The router/NAT/DNS setup must map those public endpoints to the local k8s protocol
surfaces only. The live run must execute Phase 4 prepare and verify without
`--skip-network-checks` or `--skip-k8s-check`.

### 2. Mobile FFI Public Operations

Implement the Phase 5 mobile runtime methods in:

```text
prototype/gbn-bridge-proto/crates/gbn-bridge-mobile-ffi/src/lib.rs
```

Required methods:

- `bootstrapNewCreator`
- `sendDummy`
- `sendUpload`

These methods must stop returning `not_implemented` and must use the same strict rules as
the local-k8s hardened flow:

- `bootstrapNewCreator` starts with only the imported HostCreator DHT seed and the
  mobile NewCreator's DHT entry/public key.
- Publisher public key, Publisher DHT, and Seed ExitBridgeB DHT are accepted only from
  the Publisher bootstrap payload encrypted to the mobile NewCreator.
- HostCreator and relay bridge transit data remains opaque; no plaintext bootstrap
  payload is exposed to transit actors.
- `sendDummy` routes only from the mobile local DHT and returns route source, bridge id,
  result state, payload/hash evidence, and ChainID.
- `sendUpload` dispatches the selected upload session through active mobile local-DHT
  bridge entries and records lane/chunk ACK evidence.
- all network operations are asynchronous or event-emitting from the Kotlin caller's
  perspective and preserve ChainID.

The FFI implementation must keep the existing `creator-runner` HTTP/admin APIs intact so
Pass 3 scripts and reports continue to work.

### 3. Android Phase 5 Controls

Update the Android app in:

```text
prototype/gbn-bridge-proto/mobile/android/app/src/main/java/com/veritas/gbn/mobile/
```

`BootstrapNewCreator`, `SendDummy`, and `SendUpload` must become real buttons in Phase 5
instead of permanently disabled placeholders. They still need strict enablement gates:

- runtime is started;
- selected profile is `local_k8s_public`;
- canonical run has Wi-Fi disabled and a cellular/mobile path available;
- HostCreator seed was imported from a QR/file payload that passed public-key,
  reachability, expiry, payload-hash, and no-Publisher-preload checks;
- `BootstrapNewCreator` is enabled only before onboarding and only after HostCreator seed
  import;
- `SendDummy` is enabled only after the mobile local DHT reaches `onboarded` or an
  explicitly accepted partial terminal state with active bridge routes;
- `SendUpload` is enabled only after onboarding and upload-session build;
- disabled states must show the exact missing prerequisite.

The app must display and export results for bootstrap, SendDummy, upload, failover,
local-DHT snapshots, ChainID events, and S3 upload metadata.

### 4. Phase 5 Collector Script

Add:

```text
prototype/gbn-bridge-proto/infra/scripts/k8s-pass4-mobile-local-collector.sh
```

The collector must:

- accept `--run-id`, one or more `--chain-id`, `--evidence-s3-key`, and required gate
  flags for bootstrap, SendDummy, upload, and failover;
- fetch the mobile evidence ZIP from S3;
- verify downloaded ZIP SHA-256 against the app-side manifest;
- unpack and validate required files, including `local_dht.json`,
  `host_creator_seed.redacted.json`, `trace_events.jsonl`, `remote_trace_queries.json`,
  and `manifest.sha256.json`;
- collect local k8s logs from Publisher authority, Publisher receiver, HostCreator, and
  selected ExitBridges;
- query observability surfaces for every ChainID;
- attach public endpoint map, HostCreator QR manifest, admin-denial transcript, and S3
  retrieval transcript;
- fail if required ChainIDs, local-DHT route evidence, encrypted bootstrap evidence, or
  no-public-admin evidence is missing.

The collector output becomes the canonical Phase 5 report input.

### 5. Standalone S3 Grant Import

The physical phone is standalone in Phase 5. It is not connected through adb and the
pre-signed S3 `PUT` URL is too long and error-prone to type manually. Phase 5 therefore
requires a QR-based evidence grant handoff.

Add an app control:

```text
EvidenceGrantQRReader
```

The canonical grant import path is:

1. The workstation generates a short-lived S3 `PUT` grant JSON.
2. The workstation renders the grant as one or more QR payloads.
3. The phone scans the QR payloads from the workstation screen.
4. The app reconstructs and validates the complete grant.
5. The app stores the grant in app-private state and uses it for evidence upload.

Because AWS pre-signed URLs can be larger than a single reliable QR payload, the QR format
must support chunking:

```json
{
  "type": "gbn.s3_grant.chunk",
  "version": 1,
  "grant_id": "pass4-phase5-20260512T201424Z",
  "index": 1,
  "count": 3,
  "sha256": "hex-sha256-of-complete-grant-json",
  "data": "base64url(json-chunk)"
}
```

Rules:

- `index` is 1-based.
- all chunks must share `grant_id`, `count`, and `sha256`;
- duplicate chunks are accepted only if their payload is byte-identical;
- the reconstructed grant JSON SHA-256 must match `sha256`;
- `expires_at_ms` must be in the future;
- the app must reject grants that contain long-lived AWS credentials or raw
  `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, or session-token fields outside the
  pre-signed URL;
- the app must show scan progress, final bucket/object key, and expiry before upload.

Fallbacks:

- Android document picker import from Downloads/Files;
- Android share intent into the app;
- adb file import for emulator/lab only.

Add workstation tooling to produce both the grant JSON and QR payload files/PNGs. If
`qrencode` is unavailable, the tool must still emit text payloads that can be rendered or
copied by another QR renderer.

---

## Mobile Run Flow

### Bootstrap

1. Install the debug APK produced by Phase 3.
2. Disable Wi-Fi on the phone.
3. Launch the app.
4. Select `local_k8s_public`.
5. Start runtime.
6. Scan the HostCreator QR with `HostCreatorDHTQRReader`.
7. Confirm HostCreator id, public-key fingerprint, public endpoint, expiry, payload hash,
   and ChainID.
8. Import the seed.
9. Tap `BootstrapNewCreator`.
10. Wait for terminal state.

Expected:

- QR import inserts HostCreator seed only.
- Mobile NewCreator sends its own DHT and public key to HostCreator.
- HostCreator relays the request to Publisher through the existing path.
- Publisher encrypts the bootstrap payload to the mobile NewCreator public key.
- HostCreator and relay bridge forward opaque bytes only.
- NewCreator learns Publisher public key/DHT and Seed ExitBridgeB DHT from the encrypted
  payload.
- Seed ExitBridgeB returns the signed bridge catalog after tunnel establishment.
- Remaining ExitBridges establish tunnels before being marked active.

### SendDummy

1. Open Creator Actions.
2. Tap `DumpLocalDht`.
3. Tap `SendDummy`.
4. Accept or enter a dummy payload size.
5. Confirm ChainID.

Expected:

- route source is `local_dht`;
- selected bridge is active and not expired;
- bridge sees ciphertext only;
- Publisher receiver accepts the frame;
- app shows assigned bridge id, result, and ChainID.

### Full Upload

1. Build a synthetic upload session or choose a small local test file.
2. Verify manifest/content hash and chunk count.
3. Tap `SendUpload`.
4. Use default lane count first.
5. Wait for upload completion and receiver content-hash match.

Expected:

- chunks are encrypted before crossing any bridge;
- dispatch plan uses active mobile local-DHT bridge entries;
- receiver reconstructs content with matching hash;
- progressive fanout evidence records lane open and chunk ACK events.

### Failover / Churn

Run one of:

- `SendDummy` with forced bridge-failure option;
- `SendUpload` with forced lane failure;
- operator-side temporary disable of one local ExitBridge public endpoint.

Expected:

- affected bridge is marked suspect or failed;
- route/lane is reselected from local DHT;
- operation completes or records explicit degraded terminal state;
- ChainID evidence includes the failover decision.

---

## Evidence Transfer

The phone exports evidence through the app, uploads it to S3, and this workstation
retrieves it through AWS APIs.

Required workstation setup creates a short-lived S3 `PUT` grant JSON for the app and then
renders it as chunked QR payloads. Do not embed AWS credentials in the APK. Use an
operator-side helper such as boto3
`generate_presigned_url(ClientMethod="put_object", HttpMethod="PUT")`:

```json
{
  "upload_mode": "s3_presigned_put",
  "bucket": "veritas-pass4-mobile-evidence",
  "object_key": "mobile-evidence/<run_id>/<chain_id>/<bundle_id>.zip",
  "presigned_put_url": "https://...",
  "expires_at_ms": 1770000000000
}
```

Then generate the QR handoff:

```bash
infra/scripts/pass4-s3-grant-qr.sh \
  --grant-json /tmp/pass4-s3-grant.json \
  --out-dir target/pass4-s3-grants/$RUN_ID
```

The app imports the grant through `EvidenceGrantQRReader`. The document/share/adb import
paths are fallback paths only; QR import is the canonical standalone-phone path.

The app records:

- bucket and object key;
- upload mode;
- ETag if available;
- local SHA-256;
- upload timestamp;
- ChainIDs included in the bundle.

Workstation retrieval:

```bash
aws s3 cp \
  s3://veritas-pass4-mobile-evidence/mobile-evidence/<run_id>/<chain_id>/<bundle_id>.zip \
  /tmp/veritas-pass4-mobile-evidence/<bundle_id>.zip
```

The report must prove that the S3-downloaded bundle hash matches the app-side manifest.

---

## Validation

Run from WSL2 Ubuntu:

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 4 tooling requires WSL2 Ubuntu" >&2; exit 1; }

cd prototype/gbn-bridge-proto

RUN_ID="pass4-phase5-$(date -u +%Y%m%dT%H%M%SZ)"
PROFILE_CONFIG="infra/pass4/public-ingress/run-profile.local-k8s-public.live.json"

infra/scripts/k8s-pass4-public-ingress-prepare.sh \
  --config "$PROFILE_CONFIG" \
  --profile local_k8s_public \
  --run-id "$RUN_ID"

infra/scripts/k8s-pass4-public-ingress-verify.sh \
  --artifact-dir "target/pass4-public-ingress/$RUN_ID" \
  --require-no-public-admin \
  --require-hostcreator-qr \
  --require-public-dht-endpoints

infra/scripts/k8s-pass4-mobile-local-collector.sh \
  --run-id "$RUN_ID" \
  --chain-id <bootstrap_chain_id> \
  --chain-id <senddummy_chain_id> \
  --chain-id <upload_chain_id> \
  --chain-id <failover_chain_id> \
  --evidence-s3-key mobile-evidence/$RUN_ID/<chain_id>/<bundle_id>.zip \
  --require-bootstrap \
  --require-send-dummy \
  --require-upload \
  --require-failover
```

The collector must gather:

- mobile evidence bundle from S3;
- local k8s Publisher authority logs;
- local k8s Publisher receiver logs;
- local k8s HostCreator logs;
- local k8s ExitBridge logs for selected routes;
- observability traces for each ChainID;
- public endpoint map and HostCreator QR manifest.

After the live run, tear down the temporary public ingress:

```bash
infra/scripts/k8s-pass4-public-ingress-down.sh \
  --artifact-dir "target/pass4-public-ingress/$RUN_ID"
```

---

## Tests

Add tests for:

- live `local_k8s_public` profile validation rejects the example profile, unresolved DNS,
  private hosts, cluster-local names, admin ports, expired descriptors, and missing TLS
  fingerprints;
- app refuses bootstrap when Wi-Fi/cellular requirement is not satisfied for canonical
  validation mode;
- app refuses `BootstrapNewCreator` before HostCreator seed import;
- app enables `BootstrapNewCreator`, `SendDummy`, and `SendUpload` only when their Phase 5
  prerequisites are satisfied;
- app rejects Publisher/bridge DHT preload in run profile config;
- bootstrap accepts Publisher public key/DHT only from encrypted payload;
- `bootstrapNewCreator`, `sendDummy`, and `sendUpload` no longer return
  `not_implemented` for valid Phase 5 requests;
- SendDummy route selection uses mobile local DHT;
- upload session dispatch uses active mobile local DHT entries;
- evidence bundle contains mobile, endpoint, DHT, app build, Rust build, and remote query
  files;
- S3 retrieval hash matches local evidence manifest;
- local k8s collector fails if any required ChainID is missing;
- local k8s collector fails if the mobile evidence bundle omits DHT, trace, manifest,
  endpoint, S3, or admin-denial artifacts.
- S3 grant chunk reconstruction accepts out-of-order QR chunks and rejects hash mismatch,
  duplicate mismatch, expired grants, and grants carrying raw long-lived AWS credentials.

Run:

```bash
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace

cd mobile/android
./gradlew test
./gradlew connectedDebugAndroidTest
```

---

## Acceptance Criteria

- A live `local_k8s_public` profile exists and Phase 4 prepare/verify pass without skip
  flags.
- `infra/scripts/k8s-pass4-mobile-local-collector.sh` exists, is tested, and fails closed
  on missing mobile/k8s/observability evidence.
- `infra/scripts/pass4-s3-grant-qr.sh` exists and emits grant JSON, chunk payloads, and QR
  PNGs when a QR renderer is available.
- Android `EvidenceGrantQRReader` can import a chunked S3 grant without adb or typing.
- Mobile FFI implements `bootstrapNewCreator`, `sendDummy`, and `sendUpload`.
- Android `BootstrapNewCreator`, `SendDummy`, and `SendUpload` buttons are enabled only
  by their strict Phase 5 prerequisites and no longer permanently disabled.
- Physical phone validation runs with Wi-Fi disabled for the canonical run.
- Android app scans a real HostCreator QR generated from local k8s public endpoint data.
- Mobile bootstrap uses no separate Publisher ingest and no private admin endpoint.
- Mobile local DHT contains Publisher and Seed ExitBridgeB entries learned from the
  encrypted Publisher bootstrap payload.
- Mobile local DHT contains bridge catalog entries learned through Seed ExitBridgeB.
- `SendDummy` succeeds with `route_source=local_dht`.
- Full upload completes with content hash match.
- Forced failure reroutes or records an explicit degraded state with ChainID evidence.
- Mobile evidence is uploaded to S3 and retrieved on this workstation.
- Local k8s logs/traces correlate to mobile ChainIDs.
- Public ingress teardown succeeds after validation.
- V1 preservation checks return no files.
- Parent plan status tracker is updated.

---

## Completion Evidence

When this phase is implemented, archive:

- Android app build id and Rust build id;
- physical device/network context;
- live `local_k8s_public` run profile used for the run;
- QR scan/import screenshots or instrumentation captures;
- mobile evidence ZIP from S3;
- S3 retrieval transcript and hash verification;
- S3 grant QR payloads/PNGs and app-side grant import evidence;
- local k8s trace/log bundle;
- public endpoint map;
- HostCreator QR seed and redacted manifest;
- bootstrap report;
- SendDummy report;
- upload report;
- failover/churn report;
- collector transcript and generated report;
- teardown transcript;
- V1 preservation command output.
