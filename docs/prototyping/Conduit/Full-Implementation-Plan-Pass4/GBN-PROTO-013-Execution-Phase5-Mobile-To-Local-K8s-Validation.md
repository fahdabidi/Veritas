# GBN-PROTO-013 - Execution Phase 5 - Mobile To Local k8s Public Internet Validation

**Status:** Pending
**Last Updated:** 2026-05-11
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
- Phase 4 public endpoint map is active.
- HostCreator QR seed is generated from the active public endpoint map.
- S3 evidence bucket and short-lived upload grant are prepared.
- Phone has cellular service; canonical run disables Wi-Fi.

The operator records the device model, Android version, ABI, carrier/network context, app
build id, Rust build id, endpoint map id, and run id before the live run starts.

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

Required workstation setup:

```bash
aws s3 presign \
  s3://veritas-pass4-mobile-evidence/mobile-evidence/<run_id>/<chain_id>/<bundle_id>.zip \
  --expires-in 3600
```

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
infra/scripts/k8s-pass4-public-ingress-verify.sh \
  --profile local_k8s_public \
  --require-no-public-admin \
  --require-hostcreator-qr

infra/scripts/k8s-pass4-mobile-local-collector.sh \
  --run-id <run_id> \
  --chain-id <mobile_chain_id> \
  --evidence-s3-key mobile-evidence/<run_id>/<chain_id>/<bundle_id>.zip \
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

---

## Tests

Add tests for:

- app refuses bootstrap when Wi-Fi/cellular requirement is not satisfied for canonical
  validation mode;
- app refuses `BootstrapNewCreator` before HostCreator seed import;
- app rejects Publisher/bridge DHT preload in run profile config;
- bootstrap accepts Publisher public key/DHT only from encrypted payload;
- SendDummy route selection uses mobile local DHT;
- upload session dispatch uses active mobile local DHT entries;
- evidence bundle contains mobile, endpoint, DHT, app build, Rust build, and remote query
  files;
- S3 retrieval hash matches local evidence manifest;
- local k8s collector fails if any required ChainID is missing.

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
- QR scan/import screenshots or instrumentation captures;
- mobile evidence ZIP from S3;
- S3 retrieval transcript and hash verification;
- local k8s trace/log bundle;
- public endpoint map;
- bootstrap report;
- SendDummy report;
- upload report;
- failover/churn report;
- teardown transcript;
- V1 preservation command output.
