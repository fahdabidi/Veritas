# GBN-PROTO-013 Phase 3 Android Kotlin Creator App Report

## Run Metadata

- Date: `2026-05-11`
- Workspace: `prototype/gbn-bridge-proto`
- Base commit before this phase: `b2b88a2 feat: add pass4 mobile runtime ffi`
- Host shell: Windows PowerShell invoking WSL2 Ubuntu
- Android validation target: Flutter-managed AVD `PantryVision_API_36`
- Result: `PASS`

## Scope

This report covers Pass 4 Phase 3: Android Kotlin Creator App.

The run validated:

1. Native Android/Kotlin app project exists under `mobile/android`.
2. Gradle wrapper builds from WSL2 Ubuntu.
3. App consumes the Phase 2 Rust runtime through JNI.
4. Required validation screens and creator action button IDs exist.
5. Runtime, run-profile, HostCreator seed, upload-session, evidence, S3 grant, and reset
   flows are implemented in the app shell.
6. Future public-network actions remain visible but disabled with Phase 5 prerequisites.
7. Android unit tests, lint, debug APK assembly, install, launch, and connected
   instrumentation pass on the emulator.
8. Rust workspace checks and V1 preservation checks remain clean.

Physical Android phone validation is intentionally deferred to Phase 5 per the updated
Phase 3 direction: use the Flutter-managed emulator first, then validate the public
mobile-network path on the phone.

## Implementation Evidence

| Area | Evidence |
|---|---|
| Android project | `prototype/gbn-bridge-proto/mobile/android/` |
| Gradle wrapper | `mobile/android/gradlew`, `gradle/wrapper/gradle-wrapper.jar` |
| Main app UI | `app/src/main/java/com/veritas/gbn/mobile/MainActivity.kt` |
| Runtime JNI wrapper | `app/src/main/java/com/veritas/gbn/mobile/runtime/MobileCreatorRuntime.kt` |
| Foreground service | `app/src/main/java/com/veritas/gbn/mobile/service/CreatorForegroundService.kt` |
| Evidence packaging | `app/src/main/java/com/veritas/gbn/mobile/evidence/EvidenceBundleWriter.kt` |
| S3 upload path | `app/src/main/java/com/veritas/gbn/mobile/evidence/S3EvidenceUploader.kt` |
| Run profile guard | `app/src/main/java/com/veritas/gbn/mobile/model/RunProfileConfig.kt` |
| Host seed guard | `app/src/main/java/com/veritas/gbn/mobile/model/HostSeedGuard.kt` |
| Button map | `app/src/main/java/com/veritas/gbn/mobile/model/CreatorActionCatalog.kt` |

## Validation Command Ledger

| Command | Status | Evidence |
|---|---:|---|
| `./gradlew test` | pass | Debug and release unit tests passed |
| `./gradlew lint` | pass | Lint completed; report generated under `app/build/reports/` |
| `./gradlew assembleDebug` | pass | Debug APK built |
| `flutter emulators` | pass | `PantryVision_API_36` AVD available |
| `flutter emulators --launch PantryVision_API_36` | blocked | Flutter launcher exited with emulator startup code `-6` in WSL |
| Direct SDK emulator launch | pass | `/usr/lib/android-sdk/emulator/emulator -avd PantryVision_API_36 -no-window -no-audio -no-snapshot -gpu swiftshader_indirect` booted |
| `./gradlew connectedDebugAndroidTest` | pass | `3 tests; 0 failures` on `PantryVision_API_36(AVD) - 16` |
| Manual emulator runtime start/stop | pass | App opened in visible WSLg emulator, runtime started/stopped, Rust library loaded, no `FATAL EXCEPTION` |
| Manual emulator action-output visibility follow-up | pass | `Show Node Metadata` initially updated only the off-screen output area; app now auto-scrolls to `Output` for action results/errors and instrumentation asserts this |
| Manual emulator S3 negative path follow-up | pass | Blank pre-signed PUT URL now reports `S3 pre-signed PUT URL is required`; Creator Actions and Evidence buttons use the same upload handler |
| Manual emulator S3 grant import follow-up | pass | Long pre-signed grant JSON can be pushed by adb to app external files and imported from the Evidence screen; instrumentation covers the import button |
| Manual emulator S3 upload follow-up | pass | Evidence ZIP uploaded from emulator to `s3://veritas-pass4-mobile-evidence/mobile-evidence/pass4-phase3-emulator/mobile-chain/mobile-bundle.zip`; downloaded object SHA-256 matched app output `8e5934e5e131ab7923bd3be0aee6d58d4d8abbbc6f39da1e0ac723ddce2cb4ef` |
| `./gradlew installDebug` | pass | Installed `com.veritas.gbn.mobile.debug` on emulator |
| `adb shell monkey -p com.veritas.gbn.mobile.debug 1` | pass | App launched; PID observed |
| `cargo fmt --all --check` | pass | Rust formatting clean |
| `cargo check --workspace` | pass | Rust workspace compiled |
| `cargo test -p gbn-bridge-mobile-ffi` | pass | `7 passed; 0 failed` |
| `git diff --stat -- prototype/gbn-proto/` | pass | No V1 file changes |
| `git diff --stat -- docs/prototyping/Lattice/` | pass | No V1 doc changes |

## Android Test Evidence

Unit test summary:

| Test Class | Tests | Failures |
|---|---:|---:|
| `CreatorActionCatalogTest` | 2 | 0 |
| `EvidenceBundleWriterTest` | 1 | 0 |
| `EvidenceUploadConfigTest` | 2 | 0 |
| `HostSeedGuardTest` | 2 | 0 |
| `RunProfileConfigTest` | 2 | 0 |

Instrumentation summary:

```text
Starting 3 tests on PantryVision_API_36(AVD) - 16
Finished 3 tests on PantryVision_API_36(AVD) - 16
```

Instrumentation gates:

- `MobileCreatorRuntime` loads the Rust `gbn_bridge_mobile_ffi` library through JNI.
- Runtime is created with app-private state under `context.filesDir`.
- `nodeMetadata()` returns the expected `creator` runtime metadata.
- MainActivity class is available in the debug APK.
- The app button panel exposes and exercises `StartRuntime`, `PreviewBootstrapDHTQR`,
  `ImportHostCreatorDHTSeed`, `BuildUploadSession`, and `ExportEvidence` tags through
  automated emulator instrumentation.
- Manual validation found that action buttons such as `Show Node Metadata` did update the
  bottom `Output` area, but did not scroll the operator there. The app now scrolls action
  output/errors into view, and the instrumentation workflow asserts the scroll behavior.
- Manual validation also found the blank S3 grant path exposed the low-level URL parser
  message `no protocol:`. The upload handler now rejects blank URLs with the operator
  message `S3 pre-signed PUT URL is required`, and both S3 upload buttons use that same
  handler.
- Manual validation showed long pre-signed URLs are not practical to paste into the WSLg
  emulator. The app now shows an adb-friendly grant file path and can import
  `pass4-s3-grant.json` from app external files into the Evidence grant field.

## Emulator Evidence

| Field | Value |
|---|---|
| AVD | `PantryVision_API_36` |
| Model | `sdk_gphone64_x86_64` |
| Android SDK | `36` |
| ABI | `x86_64` |
| App package | `com.veritas.gbn.mobile.debug` |
| App version | `0.1.0-pass4-phase3-debug` |

Launch evidence:

```text
package:com.veritas.gbn.mobile.debug
com.veritas.gbn.mobile.debug/com.veritas.gbn.mobile.MainActivity
Events injected: 1
```

Screenshot capture:

| Artifact | SHA-256 |
|---|---|
| `/tmp/pass4-phase3-main.png` | `f9f13d957ce6527a35881ab37fbd45329573999e61c2a4b237515dac858583e7` |

No `FATAL EXCEPTION` entries were observed for `com.veritas.gbn.mobile` in the checked
logcat window after launch.

## APK Evidence

| Artifact | Size | SHA-256 |
|---|---:|---|
| `mobile/android/app/build/outputs/apk/debug/app-debug.apk` | 46M | `4068c608644d5518ce0f1e2b7079ec732de20aaf15d74bf8b7806f92fc706cfe` |

The APK includes the generated Phase 2 Android `.so` files through the Gradle
`buildMobileFfiDebug` task. Generated build outputs and `.so` artifacts are ignored by
git and reproducible.

## Feature Gates Covered

| Requirement | Status | Evidence |
|---|---:|---|
| Runtime screen | pass | Start/stop runtime controls, build/ABI/state path display |
| Network Profile screen | pass | Profile selector and run-profile JSON import guard |
| Creator State screen | pass | Node metadata, local DHT, node-state, runtime metrics buttons |
| Creator Actions screen | pass | Required button IDs mapped to relay-control actions |
| Bootstrap screen | pass | HostCreator QR reader action, paste/file-import fallback, preview/import calls |
| Upload screen | pass | Synthetic upload session build and frame summary |
| Events screen | pass | ChainID filter and `traceEvents` call |
| Evidence screen | pass | Evidence ZIP packaging and S3 pre-signed PUT upload path |
| Reset screen | pass | Confirmation dialog before `resetState` |
| Disabled future actions | pass | Bootstrap, SendDummy, and SendUpload are disabled until Phase 5 |
| No mobile admin shortcut | pass | App calls embedded runtime only; no private `creator-runner` admin URLs |

## S3 Evidence Path

The app implements the canonical S3 upload path using a pre-signed PUT grant and rejects
long-lived AWS credentials in app config. Manual emulator validation on `2026-05-12`
executed a real PUT to:

```text
s3://veritas-pass4-mobile-evidence/mobile-evidence/pass4-phase3-emulator/mobile-chain/mobile-bundle.zip
```

The app exported:

```text
/data/user/0/com.veritas.gbn.mobile.debug/cache/evidence-exports/mobile-evidence-1778613524822.zip
sha256=8e5934e5e131ab7923bd3be0aee6d58d4d8abbbc6f39da1e0ac723ddce2cb4ef
```

The object was downloaded from S3 and the local SHA-256 matched the app output:

```text
8e5934e5e131ab7923bd3be0aee6d58d4d8abbbc6f39da1e0ac723ddce2cb4ef  /tmp/mobile-bundle.zip
```

## Compatibility Notes

Phase 3 does not modify `creator-runner`, Publisher, ExitBridge, or Pass 3 k8s scripts.
The app consumes `gbn-bridge-mobile-ffi` and keeps later public-network operations
disabled until Phase 5, so no Pass 3 HTTP/admin contract was reshaped in this phase.
