# GBN-PROTO-005 - Execution Phase 11 Detailed Plan: V2 Mobile-Network Validation

**Status:** Implemented locally from the committed Phase 10 AWS-prototype baseline; live AWS/mobile measurements remain pending
**Primary Goal:** add V2-only tooling and documentation for measuring Conduit behavior under mobile-like restart, stale-entry, bootstrap, punch, batching, and failover scenarios without touching frozen V1 validation surfaces
**Source Plan:** [GBN-PROTO-005 Execution Plan](GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign-Execution-Plan.md)
**Phase 0 Baseline Release:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)

---

## 1. Current Repo Findings

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 11 builds on the committed Phase 10 AWS-prototype assets |
| Current HEAD commit | `8b087d74ddbbdfb9774d12e8eb32be6aab2796c4` | this is the committed baseline used to begin Phase 11 |
| Phase 10 state | V2-only AWS scripts and CloudFormation now exist | Phase 11 can measure against a concrete deployment surface rather than placeholders |
| Local harness state | creator/bootstrap, data-path, reachability, and integration tests already cover most control-flow scenarios | Phase 11 can map local evidence to mobile-like scenarios immediately |
| Live environment limitation | no live AWS deployment or mobile carrier path is available in this environment | Phase 11 must separate local proxy evidence from pending live validation |
| V1 protected paths | clean | V1 preservation remains intact |

---

## 2. Phase 11 Boundary

### In Scope

- add V2-only mobile validation orchestration
- add V2-only metrics collection helpers for the deployed Phase 10 stack
- document the mobile scenario matrix
- record current local evidence and remaining live-measurement gaps

### Out Of Scope

- modifying V1 deployment or validation scripts
- modifying root `README.md`
- claiming completed mobile-carrier validation without a live run
- changing protocol or runtime behavior for this phase alone

---

## 3. Files Created Or Modified

Created:

- `docs/prototyping/GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign-Test.md`
- `docs/prototyping/GBN-PROTO-005-Execution-Phase11-V2-Mobile-Network-Validation.md`
- `prototype/gbn-bridge-proto/infra/scripts/mobile-validation.sh`
- `prototype/gbn-bridge-proto/infra/scripts/collect-bridge-metrics.sh`
- `prototype/gbn-bridge-proto/docs/mobile-test-matrix.md`

Modified:

- `prototype/gbn-bridge-proto/infra/README-infra.md`
- master execution plan Phase 11 status and detailed-reference section

---

## 4. Validation Commands

Local validation:

```bash
cargo fmt --all --check --manifest-path prototype/gbn-bridge-proto/Cargo.toml
```

```powershell
$target = Join-Path $env:LOCALAPPDATA 'Temp\veritas-bridge-target-phase11'
New-Item -ItemType Directory -Path $target -Force | Out-Null
$env:CARGO_INCREMENTAL='0'
cargo check --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml --target-dir $target
cargo test --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml --target-dir $target
```

Script syntax / help validation:

```bash
bash -n prototype/gbn-bridge-proto/infra/scripts/mobile-validation.sh
bash -n prototype/gbn-bridge-proto/infra/scripts/collect-bridge-metrics.sh
bash prototype/gbn-bridge-proto/infra/scripts/mobile-validation.sh --help
bash prototype/gbn-bridge-proto/infra/scripts/collect-bridge-metrics.sh --help
```

V1 preservation:

```bash
git diff --name-only -- \
  prototype/gbn-proto \
  docs/prototyping/GBN-PROTO-004-Phase2-Serverless-Scale-Onion-Plan.md \
  docs/prototyping/GBN-PROTO-004-Phase2-Serverless-Scale-Test.md \
  docs/architecture/GBN-ARCH-000-System-Architecture.md \
  docs/architecture/GBN-ARCH-001-Media-Creation-Network.md
```

```bash
cd prototype/gbn-proto
cargo check --workspace
cargo test -p mcn-router-sim
```

Live Phase 11 run still required:

```bash
prototype/gbn-bridge-proto/infra/scripts/mobile-validation.sh --mode aws --stack-name gbn-bridge-phase2-<env> --region <region>
```

---

## 5. Executed Local Validation Result

- `cargo fmt --all --check --manifest-path prototype/gbn-bridge-proto/Cargo.toml` passed
- `cargo check --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml --target-dir %LOCALAPPDATA%\Temp\veritas-bridge-target-phase11` passed
- `cargo test --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml --target-dir %LOCALAPPDATA%\Temp\veritas-bridge-target-phase11` passed
- `bash -n` passed for `mobile-validation.sh` and `collect-bridge-metrics.sh`
- help output succeeded for both new Phase 11 scripts
- protected V1 diff remained empty
- `cd prototype/gbn-proto && cargo check --workspace` passed
- `cd prototype/gbn-proto && cargo test -p mcn-router-sim` passed

---

## 6. Acceptance Criteria

Phase 11 is locally complete when:

- mobile validation tooling exists under V2-only paths
- the mobile scenario matrix exists
- the V2 test-results document records current evidence and unresolved gaps
- local harness evidence is mapped to the intended mobile scenarios
- V2 validation passes
- protected V1 diff remains empty
- minimum V1 regression suite passes

Full Phase 11 sign-off additionally requires:

- at least one live AWS/mobile validation run
- recorded bootstrap and failover latency measurements
- recorded batch rollover timing from a live run
- at least one live network-change or IP-churn scenario

---

## 7. Current Blocker

- full Phase 11 sign-off still depends on a live AWS/mobile test environment; the current implementation provides the tooling and current evidence record, not the final real-world measurements
