# GBN-PROTO-013 Phase 5 Mobile To Local k8s Validation Attempt

> Superseded note: this blocked attempt remains archived as historical evidence. The
> updated Phase 5 plan no longer uses local k8s public ingress for sign-off; canonical
> physical-phone validation now requires an AWS-deployed Publisher, HostCreator, and
> ExitBridge topology.

## Run Metadata

- Date: `2026-05-12`
- Workspace: `prototype/gbn-bridge-proto`
- Host shell: Windows PowerShell invoking WSL2 Ubuntu
- Result: `BLOCKED`

Phase 5 was started, but the end-to-end physical mobile-network validation cannot be
completed from the current repo/environment state.

## What Passed

| Gate | Status | Evidence |
|---|---:|---|
| Strict bootstrap prerequisite | pass | `infra/scripts/k8s-smoke-bootstrap-strict-v4.sh --require-observability` |
| Strict SendDummy prerequisite | pass | `infra/scripts/k8s-smoke-senddummy-strict-v4.sh --require-observability` |
| Local k8s stack readiness | pass | Existing `veritas` namespace Publisher, HostCreator, NewCreator, 10 ExitBridges, Postgres, and observability pods were running |

Strict bootstrap standalone run:

```text
chain_id=smoke-2-0600647cb57f4b88a66ba152bc888261
state=onboarded
bridges=10
active=10
artifact_dir=target/k8s-smoke-artifacts/smoke-2-bootstrap-strict-v4/20260512-130846-1180686
tracked_report=docs/prototyping/Conduit/Full-Implementation-Plan-Pass4/Test-Reports/GBN-PROTO-013-Phase1-Strict-Bootstrap-20260512-130846-1180686.md
```

Strict SendDummy run:

```text
bootstrap_chain_id=smoke-2-d2669cc97f6e45e5aa1f638338493de7
normal_senddummy_chain_id=smoke-3-normal-d9765ec9172a4e969067fef9c08b803f
failover_senddummy_chain_id=smoke-3-failover-7a971431f1744d99a4db231760361505
artifact_dir=target/k8s-smoke-artifacts/smoke-3-senddummy-strict-v4/20260512-131044-1196783
tracked_report=docs/prototyping/Conduit/Full-Implementation-Plan-Pass4/Test-Reports/GBN-PROTO-013-Phase1-Strict-SendDummy-20260512-131044-1196783.md
```

## Blocking Evidence

### Live Public Endpoint Profile

Command attempted:

```bash
infra/scripts/k8s-pass4-public-ingress-prepare.sh \
  --config infra/pass4/public-ingress/run-profile.local-k8s-public.example.json \
  --profile local_k8s_public \
  --run-id pass4-phase5-live-20260512T201424Z
```

Result:

```text
ERROR: one or more public endpoint reachability checks failed
```

Reachability transcript:

```text
target/pass4-public-ingress/pass4-phase5-live-20260512T201424Z/public_reachability_transcript.txt
```

The transcript shows every example host failed DNS resolution:

```text
pub-auth.pass4-conduit.example.com: [Errno -5] No address associated with hostname
pub-recv.pass4-conduit.example.com: [Errno -5] No address associated with hostname
hostcreator.pass4-conduit.example.com: [Errno -5] No address associated with hostname
bridge01.pass4-conduit.example.com: [Errno -5] No address associated with hostname
bridge02.pass4-conduit.example.com: [Errno -5] No address associated with hostname
bridge03.pass4-conduit.example.com: [Errno -5] No address associated with hostname
```

Phase 5 needs an operator-provided live public endpoint profile with real public hosts,
certificate fingerprints, descriptor expiry, and route/NAT rules that expose only the
required protocol surfaces.

### Mobile App Public Actions

The Android app still intentionally disables the live Phase 5 actions:

```text
BootstrapNewCreator: disabled, Phase 5 public HostCreator path required
SendDummy: disabled, Phase 5 onboarded mobile path required
SendUpload: disabled, Phase 5 onboarded mobile path required
```

Source evidence:

```text
prototype/gbn-bridge-proto/mobile/android/app/src/main/java/com/veritas/gbn/mobile/MainActivity.kt
prototype/gbn-bridge-proto/mobile/android/app/src/main/java/com/veritas/gbn/mobile/model/CreatorActionCatalog.kt
```

### Mobile FFI Public Operations

The Rust mobile runtime still returns `not_implemented` for the live network operations
that Phase 5 requires:

```text
seedHostCreator
bootstrapNewCreator
sendDummy
sendUpload
```

Source evidence:

```text
prototype/gbn-bridge-proto/crates/gbn-bridge-mobile-ffi/src/lib.rs
```

### Phase 5 Collector

The Phase 5 execution doc references:

```text
infra/scripts/k8s-pass4-mobile-local-collector.sh
```

That collector script does not exist yet. The repo currently contains Phase 4 public
ingress prepare/verify/down tooling, but not the Phase 5 S3/log/trace collector.

## Conclusion

Phase 5 is not complete and the parent plan tracker must remain pending for Phase 5.

The prerequisite local-k8s hardening gates are green. The next implementation work before
the physical phone run is:

1. Implement mobile FFI public operations for `bootstrapNewCreator`, `sendDummy`, and
   `sendUpload`.
2. Enable the Android Phase 5 buttons behind runtime/onboarding/profile gates.
3. Add `infra/scripts/k8s-pass4-mobile-local-collector.sh`.
4. Provide a live `local_k8s_public` profile with reachable public endpoints.
5. Run the physical Android phone validation with Wi-Fi disabled.
6. Upload mobile evidence to S3 and collect local k8s traces/logs by ChainID.

## Resolution Addendum - 2026-05-12

This blocked local-k8s attempt was superseded by the AWS Phase 5 implementation path.
The later repo-side implementation added:

- `aws_public` Android/mobile FFI profile support;
- AWS topology plan/up/verify/down scripts;
- AWS mobile evidence collector;
- HostCreator non-admin bootstrap hint endpoint;
- live AWS run-profile and HostCreator QR artifact generation.

The remaining Phase 5 work is now the physical-phone AWS validation run, not this local
k8s public-ingress path.
