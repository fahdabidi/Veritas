# GBN-PROTO-005 - Execution Phase 9 Detailed Plan: V2 Test Harness

**Status:** Implemented locally and validated from the committed Phase 8 reachability-classification baseline; final sign-off still requires the V1 extended local regression in a Docker-capable environment
**Primary Goal:** build a reproducible Conduit-local integration harness for registration, refresh, bootstrap, punch ACKs, batching, failover, reuse, confidentiality, and reachability filtering without mutating the frozen V1 Lattice test, docker, or validation surfaces
**Source Plan:** [GBN-PROTO-005 Execution Plan](GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign-Execution-Plan.md)
**Phase 0 Baseline Release:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)
**Protocol Baseline:** [GBN-ARCH-002-Bridge-Protocol-V2](../architecture/GBN-ARCH-002-Bridge-Protocol-V2.md)

---

## 1. Current Repo Findings

These findings should drive Phase 9 execution instead of being rediscovered during implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 9 notes should capture the committed baseline used to begin harness work |
| Current HEAD commit | `fed74abdc684baac7fa42ab199cac033cdc132a3` | this is the committed Phase 8 reachability-classification baseline that Phase 9 now builds on |
| Phase 0 baseline release | `veritas-lattice-0.1.0-baseline` published | V1 Lattice remains the preservation reference point for no-touch test and tooling boundaries |
| Current V2 workspace manifest | `prototype/gbn-bridge-proto/Cargo.toml` is now both a minimal root package and the V2 workspace manifest | root integration tests now execute under Cargo instead of remaining inert files |
| Current V2 test coverage | per-crate tests already exist in `gbn-bridge-protocol`, `gbn-bridge-publisher`, and `gbn-bridge-runtime` | Phase 9 should add a cross-crate harness, not reimplement existing focused crate tests |
| Current workspace-root `tests/` directory | now contains `tests/common/`, `tests/integration.rs`, and scenario files under `tests/integration/` | the cross-crate harness is now real and runs under `cargo test --workspace` |
| Current smoke assets | `docker-compose.bridge-smoke.yml`, `infra/scripts/run-local-bridge-tests.sh`, and `test-vectors/README.md` now exist | local smoke support is in place, even though compose-specific validation was skipped here because Docker is unavailable |
| Current V2 configs | `configs/creator.example.toml` and `configs/host_creator.example.toml` already exist | harness docs and smoke assets should reuse existing config shape instead of inventing a new one |
| Current transport/runtime state | Phases 3 through 8 are committed and the Phase 9 harness is implemented locally | the harness now exercises real publisher, bridge, creator, discovery, and reachability policy rather than placeholders |
| Current validation environment risk | OneDrive-backed V2 `target/` writes still fail with Windows `os error 5` during `cargo test --workspace` | Phase 9 validation should expect the temp `--target-dir` fallback again |
| Current protected V1 path drift | none | V1 preservation remains a hard sign-off gate |

---

## 2. Review Summary

The master plan is directionally correct, but a robust Phase 9 needs tighter harness assumptions:

| Gap | Why It Matters | Resolution For Phase 9 |
|---|---|---|
| Cargo execution ambiguity | root-level `tests/` at a workspace-only root will not run | make the workspace root a minimal package for the harness, or explicitly switch to a dedicated harness crate |
| Test-boundary ambiguity | per-crate tests and cross-crate tests can become duplicates | keep crate tests focused and put end-to-end behavior in the Phase 9 harness only |
| Smoke-topology ambiguity | Phase 9 asks for docker-compose before the AWS deployment phase introduces deployment images | make the compose file a local smoke scaffold only, not a production deployment artifact |
| Confidentiality-assertion ambiguity | “payload confidentiality” can drift into cryptography redesign or into meaningless smoke checks | assert only the current Phase 6 boundary: bridges handle opaque framed payload, publisher receives the same framed payload, and clear payload is not required at bridges |
| CI/local split ambiguity | harness logic can become shell-only and hard to run under Cargo | keep the primary signal in Cargo-driven tests and use shell/docker smoke as secondary support |

The harness should prove that committed Conduit behavior works across crates together. It should not reopen protocol design, cloud deployment, or mobile-network validation.

---

## 3. Scope Lock

### In Scope

- create a V2-local integration harness for cross-crate end-to-end tests
- add integration cases for:
  - bridge registration
  - returning-creator catalog refresh
  - first-time creator bootstrap
  - UDP punch ACK flow
  - creator failover
  - batched onboarding
  - bridge reuse after insufficient fanout / timeout
  - payload confidentiality boundary
  - reachability filtering
- add a V2-local smoke runner script
- add a V2-local docker-compose smoke topology descriptor
- document V2 test-vector and harness assumptions

### Out Of Scope

- AWS deployment assets or deployment images beyond the minimum local smoke scaffold
- mobile-network validation
- V1 integration tests, V1 docker-compose files, or V1 validation scripts
- root `README.md`
- protocol-surface redesign or new runtime capabilities beyond what is needed to exercise committed Phase 8 behavior

---

## 4. Preflight Gates

Phase 9 should not begin code edits until all of these are checked:

1. Confirm the committed Phase 8 baseline is present and clean.
2. Confirm protected V1 paths are clean in the local worktree.
3. Confirm the V2 root manifest is still workspace-only and record the required harness-execution adjustment.
4. Confirm existing per-crate tests remain the primary fine-grained checks and will not be duplicated blindly in the new harness.
5. Confirm the smoke topology is V2-local only and will not reuse V1 docker-compose or V1 validation entrypoints.
6. Confirm Phase 9 will not modify the main repo `README.md`; any Conduit README rewrite remains deferred.
7. Confirm the temp-target validation fallback remains available for OneDrive-backed runs.

Current blocker:

- none at implementation scope; final phase sign-off is blocked in this environment because `docker-compose` is unavailable for the V1 extended local regression suite

---

## 5. Decisions To Lock In Phase 9

### 5.1 Harness Execution Model

Phase 9 must resolve the current Cargo mismatch explicitly.

Recommended decision:

- keep the root-level harness file layout from the master plan
- minimally extend `prototype/gbn-bridge-proto/Cargo.toml` so the workspace root is also a package
- add a minimal root `src/lib.rs` only if Cargo requires it for executable root integration tests

This keeps the master-plan file paths valid while making `cargo test --workspace` actually execute the new harness.

If that approach proves awkward, the fallback is:

- create a dedicated harness crate under `crates/`
- move the root test layout into that crate

Do not leave the execution model ambiguous.

### 5.2 Test Boundary

Lock the boundary like this:

| Layer | Responsibility |
|---|---|
| existing per-crate tests | unit and focused integration behavior inside one crate |
| Phase 9 root harness | cross-crate end-to-end scenarios and topology orchestration |
| local smoke script / docker-compose | human-invoked smoke checks, not the main correctness proof |

Do not bloat crate tests with every full-stack scenario once the root harness exists.

### 5.3 Determinism Rules

Phase 9 tests should be deterministic by default:

- use in-process publisher authority and runtime instances where possible
- control timestamps directly in tests
- avoid real network sockets when behavior can be proven through in-memory flows
- reserve docker-compose smoke for sanity checks rather than primary correctness

### 5.4 Scenario Matrix

Phase 9 should explicitly cover these behaviors:

- bridge registration path still works in the full-stack harness
- returning creator refresh path still works after Phase 8 reachability rules
- first-time bootstrap still works with host creator, relay bridge, seed bridge, and follow-on bridge set
- UDP punch ACK propagation still works and correlates to the right sessions
- batch onboarding still respects the current batch window / rollover behavior
- creator failover still reassigns pending work to another active direct bridge
- insufficient fanout / timeout still falls back to active bridge reuse
- payload confidentiality boundary still keeps bridge-visible data opaque
- reachability filtering still blocks non-direct creator-ingress use

### 5.5 Confidentiality Assertion Boundary

Phase 9 should not attempt to prove more than the code currently implements.

Lock the confidentiality assertions to:

- bridge receives framed opaque payload
- bridge forwards framed opaque payload without interpreting clear content
- publisher receives the same framed payload / ingest record expected by the data-path contract

Do not turn Phase 9 into a new cryptographic redesign or claim E2E properties not implemented yet.

### 5.6 Smoke Topology Boundary

The Phase 9 compose file is a local smoke tool only.

Lock these assumptions:

- it lives entirely under `prototype/gbn-bridge-proto/`
- it does not replace Cargo integration tests
- it does not depend on V1 compose assets
- it does not become the Phase 10 AWS deployment artifact

---

## 6. Dependency And Implementation Policy

### Required Bias

- reuse the committed Phase 3 through Phase 8 crates and configs
- use shared test helpers under `tests/common/` instead of duplicating topology bootstrapping in every file
- keep harness helpers small and topology-focused
- prefer Cargo-executable test assets over shell-only orchestration

### Avoid In Phase 9

- mixing AWS deployment concerns into the harness
- rewriting existing crate tests just to move them
- adding real infra dependencies where in-process topology is enough
- modifying V1 validation scripts, V1 docker-compose, or V1 test paths

---

## 7. Evidence Capture Requirements

Phase 9 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or commit message |
| starting commit SHA | `git rev-parse HEAD` | phase notes or commit message |
| pre-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |
| harness execution model chosen | Phase 9 plan + code | phase notes |
| temp-target fallback used, if needed | local command log | phase notes |
| smoke runner entrypoint used | local command log | phase notes |
| post-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |

---

## 8. Recommended Execution Order

Implement Phase 9 in this order:

1. Capture branch, commit SHA, and protected-path diff state.
2. Resolve the Cargo harness execution model first.
3. Add shared root harness helpers in `tests/common/`.
4. Add the root integration tests one scenario at a time.
5. Add the local smoke runner script.
6. Add the docker-compose smoke descriptor.
7. Add the test-vectors README last so it documents the harness that actually exists.
8. Run V2 validation.
9. Run V1 preservation and regression checks.

This keeps the harness executable early and avoids writing a large test tree that Cargo never runs.

---

## 9. Validation Commands

Run these from the repo root unless noted otherwise:

Standard V2 path:

```bash
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
```

If the OneDrive-backed workspace still throws Windows `os error 5` on target writes, use the documented temp-target fallback and record it in the phase notes:

```powershell
$target = Join-Path $env:LOCALAPPDATA 'Temp\veritas-bridge-target-phase9'
New-Item -ItemType Directory -Path $target -Force | Out-Null
$env:CARGO_INCREMENTAL='0'
cargo test --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml --target-dir $target
```

If the Phase 9 smoke script exists by the end of implementation, also run:

```bash
bash prototype/gbn-bridge-proto/infra/scripts/run-local-bridge-tests.sh
```

Also run:

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

Expected outcome:

- the full V2 workspace sanity suite passes
- all new V2 harness tests pass
- returning-creator refresh and first-time bootstrap both pass under automation
- batch onboarding and active-bridge reuse paths pass under automation
- confidentiality and reachability assertions pass under automation
- protected V1 diff remains empty
- minimum V1 regression suite passes

Executed local validation result for the current implementation:

- `cargo fmt --all --check --manifest-path prototype/gbn-bridge-proto/Cargo.toml` passed
- `cargo check --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml` passed
- `cargo test --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml --target-dir %LOCALAPPDATA%\Temp\veritas-bridge-target-phase9` passed
- `bash prototype/gbn-bridge-proto/infra/scripts/run-local-bridge-tests.sh` passed for formatting, workspace check, and workspace tests; compose validation was skipped because `docker compose` was unavailable
- protected V1 diff remained empty
- `cd prototype/gbn-proto && cargo check --workspace` passed
- `cd prototype/gbn-proto && cargo test -p mcn-router-sim` passed
- `cd prototype/gbn-proto && bash validate-scale-test.sh` could not complete in this environment because `docker-compose` is not installed

---

## 10. Acceptance Criteria

Phase 9 is complete only when all of the following are true:

- the V2 harness execution model is explicit and executable
- all files listed in the master plan exist, plus any minimal root-package files required to make root integration tests runnable
- shared test helpers exist under `tests/common/`
- root integration tests cover registration, refresh, bootstrap, punch ACKs, batching, failover, reuse, confidentiality, and reachability
- the smoke runner script exists and works locally
- the V2-local docker-compose smoke descriptor exists
- `test-vectors/README.md` documents the harness/test-vector assumptions
- protected V1 diff is clean after validation
- minimum V1 regression suite still passes

---

## 11. Risks And Blockers

| Risk | What It Looks Like | Mitigation |
|---|---|---|
| inert harness files | root integration tests are added but Cargo never executes them | resolve the root package vs harness crate decision first |
| duplicate coverage | new harness simply repeats existing crate tests | keep crate tests focused and root harness scenario-driven |
| smoke-only false confidence | docker-compose passes while Cargo tests remain weak | keep Cargo tests as the primary correctness gate |
| Phase 10 bleed | harness grows deployment-oriented images or infra assumptions | keep compose and scripts local-only and dev-oriented |
| OneDrive validation noise | V2 tests fail for filesystem reasons instead of code reasons | use and document the temp-target fallback |

Current blocker:

- Phase 9 code and V2 validation are complete locally, but full phase sign-off still depends on rerunning `bash validate-scale-test.sh` in an environment with `docker-compose` available

---

## 12. First Implementation Cut

If Phase 9 is implemented as a single focused change set, use this breakdown:

1. Make the root harness executable under Cargo
2. Add `tests/common/` topology helpers
3. Add root integration scenarios
4. Add local smoke script and docker-compose scaffold
5. Add `test-vectors/README.md`
6. Run V2 and V1 validation

That keeps the harness runnable from the start and prevents Phase 9 from turning into an inert pile of test files.
