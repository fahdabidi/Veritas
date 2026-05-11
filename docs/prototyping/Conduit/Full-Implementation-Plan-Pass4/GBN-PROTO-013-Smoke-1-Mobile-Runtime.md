# GBN-PROTO-013 - Smoke 1 - Mobile Runtime

**Status:** Complete
**Last Updated:** 2026-05-11
**Parent Plan:** [GBN-PROTO-013](GBN-PROTO-013-Conduit-Mobile-Creator-Public-Internet-Validation-Execution-Plan.md)
**Depends On:** Phases 1-3 complete

## Objective

Prove the Android app can load the Rust mobile creator runtime, execute local creator
operations, expose the required button panel, persist state, and export evidence without
using public network validation.

This smoke is the mobile equivalent of a bring-up gate. It blocks public-internet
validation if the app cannot load the `.so`, manage runtime lifecycle, or export the DHT
and trace evidence needed for later phases.

---

## Scope

Smoke 1 validates:

- `x86_64` Flutter-managed emulator library load for Phase 3;
- `arm64-v8a` physical-device library load in Phase 5;
- `MobileCreatorRuntime` start/stop;
- `nodeMetadata()`;
- `localDht()`;
- HostCreator QR preview/import with sample payload;
- synthetic `BuildUploadSession`;
- Creator Actions button panel test ids;
- ChainID event stream;
- evidence export;
- S3 upload with short-lived grant or local mocked upload in CI;
- Pass 3 creator-runner compatibility after shared runtime extraction.

It does not require a public local-k8s ingress, live bootstrap, SendDummy, or upload over
the network.

---

## Required Commands

Run from WSL2 Ubuntu:

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 4 tooling requires WSL2 Ubuntu" >&2; exit 1; }

cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
cargo test -p gbn-bridge-mobile-ffi

cd mobile/android
./gradlew test
./gradlew lint
./gradlew assembleDebug
./gradlew connectedDebugAndroidTest
```

Connected emulator/device test must record:

- device model;
- Android SDK;
- ABI;
- app build id;
- Rust build id;
- generated ChainID.

---

## Manual Device Steps

1. Install debug APK.
2. Launch app.
3. Select `offline_test`.
4. Start runtime.
5. Confirm Runtime screen shows app build id, Rust build id, ABI, device id, and state
   path.
6. Scan or import a sample `BootstrapDHTQRCode` payload.
7. Confirm preview shows HostCreator public-key fingerprint and endpoint metadata.
8. Import seed and verify local DHT is not marked onboarded.
9. Tap `RefreshStatus`, `RefreshState`, `DumpLocalDht`, `DumpNodeState`, and
   `RuntimeMetrics`.
10. Build a synthetic upload session.
11. Export evidence.
12. Upload evidence to S3 with a short-lived grant or use share/document fallback for
   manual proof.

---

## Pass Conditions

- App does not crash.
- Runtime starts and stops cleanly.
- No mobile action calls private admin URLs.
- QR import rejects invalid samples and accepts the valid sample.
- Local DHT snapshot is visible in app and evidence.
- Event stream is filterable by ChainID.
- Evidence bundle includes required files and manifest hashes.
- S3 upload or fallback export produces a retrievable ZIP.
- Pass 3 compatibility suite remains green when shared Rust code changed.
- V1 preservation checks return no files.

---

## Report Artifacts

Archive under `Test-Reports/`:

- Gradle output;
- Rust test output;
- connected-device output;
- APK hash;
- screenshots or instrumentation captures for Runtime, Creator Actions, Evidence;
- sample evidence ZIP;
- S3 upload/retrieval transcript if used;
- Pass 3 compatibility transcript;
- V1 preservation output.
