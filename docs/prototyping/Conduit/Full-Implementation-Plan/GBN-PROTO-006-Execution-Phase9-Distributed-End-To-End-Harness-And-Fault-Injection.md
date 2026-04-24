# GBN-PROTO-006 - Execution Phase 9 Detailed Plan: Distributed End-To-End Harness And Fault Injection

**Status:** Complete and validated locally after Phase 8 real deployment images and AWS control plane implementation
**Primary Goal:** build a real distributed end-to-end Conduit harness that exercises the full control and data paths across actual service boundaries, adds deterministic fault injection, and proves `chain_id` continuity through realistic multi-service scenarios  
**Source Plan:** [GBN-PROTO-006 Execution Plan](GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md)  
**Protected V1 Baseline:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)  
**Phase 8 Detailed Plan:** [GBN-PROTO-006-Execution-Phase8-Real-Deployment-Images-And-AWS-Control-Plane](GBN-PROTO-006-Execution-Phase8-Real-Deployment-Images-And-AWS-Control-Plane.md)  
**Starting Conduit Baseline:** `2b6d5c5d24e269e96e3fdc820f3f90669607414a`

---

## 1. Current Repo Findings

These findings should drive Phase 9 instead of being rediscovered during implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 9 was implemented on the mainline branch and validated there |
| Current HEAD commit | `b0aa4f2` | starting committed baseline for the Phase 9 distributed harness cutover |
| Current root integration harness | [`tests/integration.rs`](../../../prototype/gbn-bridge-proto/tests/integration.rs) still exists for fast local regression | distributed assertions are now separated into a dedicated `tests/e2e/` surface |
| Current integration coverage | legacy integration tests still cover local correctness for bootstrap, registration, confidentiality, reachability, and UDP ACK paths | Phase 9 preserves them as fast checks while moving distributed assertions to the new e2e harness |
| Current local bridge test script | [`run-local-bridge-tests.sh`](../../../prototype/gbn-bridge-proto/infra/scripts/run-local-bridge-tests.sh) remains the fast local smoke wrapper | Phase 9 adds a distinct distributed runner instead of overloading the old script |
| Current local compose stack | [`docker-compose.conduit-e2e.yml`](../../../prototype/gbn-bridge-proto/docker-compose.conduit-e2e.yml) and [`run-conduit-e2e.sh`](../../../prototype/gbn-bridge-proto/infra/scripts/run-conduit-e2e.sh) now provide the local distributed topology entrypoint | the distributed harness can exercise real authority, receiver, bridge, and creator boundaries |
| Current V2 docs surface | `tests/e2e/` now exists with dedicated bootstrap, refresh, data-path, failover, and trace scenarios | the repo now has a stable distributed e2e boundary |
| Current fault-injection surface | deterministic scenarios now cover bridge timeout, seed reassignment, and authority restart recovery | failure handling is asserted instead of being left to incidental behavior |
| Current trace-specific coverage | dedicated distributed trace scenarios now exist in [`tests/e2e/trace.rs`](../../../prototype/gbn-bridge-proto/tests/e2e/trace.rs) | `chain_id` continuity is now asserted across multi-service bootstrap and upload flows |

---

## 2. Review Summary

Phase 9 is where Conduit must stop proving behavior mainly through local runtime/unit-style tests and start proving behavior across real service boundaries under controlled failure. If this phase is weak, the implementation may be feature-complete in code but still not defensible as a distributed system.

The main gaps the detailed Phase 9 plan must close are:

| Gap | Why It Matters | Resolution For Phase 9 |
|---|---|---|
| current harness is still local-first | service-boundary regressions can still slip through | add a real `tests/e2e/` harness tied to the deployed local topology |
| no deterministic fault injection exists | failure handling claims remain under-tested | add explicit fault-injection scenarios for timeouts, reassignment, restart recovery, and bridge failure |
| current local smoke script is not a real scenario runner | test execution is still too coarse and local | add `run-conduit-e2e.sh` as the distributed harness entrypoint |
| no dedicated distributed trace scenario exists | `chain_id` continuity still lacks full-system proof | add a dedicated distributed trace scenario and assertions |
| current compose stack is placeholder-only | the e2e harness has no real local stack to target until the Phase 8 stack exists | make Phase 8 completion a real prerequisite |

Phase 9 should make full-system distributed validation real, but it should not yet claim live AWS/mobile evidence. That remains Phase 10.

---

## 3. Scope Lock

### In Scope

- add a `tests/e2e/` harness structure with shared helpers
- add distributed scenarios for refresh, first bootstrap, data path, failover, and trace continuity
- add deterministic fault injection for bridge failure, timeout, reassignment, and restart recovery
- add `run-conduit-e2e.sh`
- integrate the harness with the real local compose topology from Phase 8
- preserve `chain_id` visibility and assertions through the distributed harness

### Out Of Scope

- live AWS/mobile measurements
- final promotion decision
- modifying `prototype/gbn-proto/**`
- modifying the main repo `README.md`

---

## 4. Preflight Gates

Phase 9 should not begin code edits until all of these are checked:

1. Confirm the Phase 0 inventory deliverables exist.
2. Confirm Phases 1 through 8 are implemented and validated so the full local service topology already exists.
3. Confirm protected V1 paths are clean in the local worktree.
4. Confirm the e2e harness will target real service boundaries rather than re-wrapping the old local harness.
5. Confirm deterministic fault injection is part of the plan, not optional.
6. Confirm `chain_id` continuity assertions are included in at least one dedicated distributed scenario.
7. Confirm `README.md` remains out of scope.

If any gate fails, Phase 9 should stop.

Current phase result:

- all Phase 1 through 8 prerequisites were present and validated before the Phase 9 harness cutover
- the distributed e2e harness was added under `tests/e2e/`
- the dedicated WSL-backed runner succeeded
- full `cargo test --workspace` succeeded after moving the target directory to `D:\veritas-proto006-phase9-target` because `C:` was full on this host

---

## 5. Harness Decisions To Lock In Phase 9

### 5.1 Real E2E Harness Boundary

Phase 9 should add a real distributed harness tree under `tests/e2e/`.

The harness should target:

- authority service
- receiver service
- bridges
- host creator / creator entrypoints where applicable

It should not simply re-export the current `tests/integration/test_*.rs` files under a new directory.

### 5.2 Deterministic Scenario Rule

Phase 9 scenarios should be explicit and scenario-named:

- `bootstrap.rs`
- `refresh.rs`
- `data_path.rs`
- `failover.rs`
- `trace.rs`

Each scenario should define:

- topology assumptions
- fault injection points
- expected terminal states
- required `chain_id` assertions

### 5.3 Fault Injection Rule

At minimum the harness should support deterministic injection of:

- bridge timeout
- bridge process failure
- bridge restart recovery
- seed reassignment trigger
- receiver-side duplicate / retry trigger where relevant

Fault injection should be explicit and scriptable, not dependent on ad hoc manual intervention.

### 5.4 Trace Rule

At least one dedicated scenario must prove:

- one root `chain_id` enters the flow
- the same `chain_id` appears at authority, bridges, receiver, ACK, and validation artifacts

Trace continuity must be asserted, not just printed.

### 5.5 Local Execution Rule

`run-conduit-e2e.sh` should become the one obvious local distributed harness entrypoint.

It should orchestrate:

- environment setup
- compose or local stack launch
- scenario execution
- teardown or cleanup hooks
- artifact / trace capture locations

### 5.6 Existing Harness Reuse Rule

The current integration tests are still useful as fast local correctness checks, but Phase 9 should not confuse them with the distributed harness.

Keep:

- existing fast harnesses for quick regression

Add:

- true e2e scenarios that cross real service boundaries

---

## 6. Module And Asset Ownership To Lock In Phase 9

Phase 9 should keep responsibilities split like this:

| Asset | Responsibility |
|---|---|
| `tests/e2e/common/mod.rs` | shared distributed harness helpers |
| `tests/e2e/bootstrap.rs` | first-contact bootstrap and fanout scenarios |
| `tests/e2e/refresh.rs` | returning creator refresh scenarios |
| `tests/e2e/data_path.rs` | forwarding, receiver, and ACK scenarios |
| `tests/e2e/failover.rs` | deterministic failure, reassignment, and restart recovery scenarios |
| `tests/e2e/trace.rs` | `chain_id` continuity and artifact assertions |
| `infra/scripts/run-conduit-e2e.sh` | local distributed harness runner |
| root V2 test harness files | minimal glue only; should not absorb all distributed scenario logic |

Do not let the root `tests/integration.rs` file become the long-term distributed harness coordinator. Phase 9 needs a dedicated `tests/e2e/` surface.

---

## 7. Dependency And Implementation Policy

Phase 9 should keep the distributed harness explicit and reproducible.

### Recommended Dependencies

- reuse the local deployment topology introduced in Phase 8
- reuse existing Rust test and helper patterns where practical
- add fault-injection helpers only if clearly scoped to the harness

### Bias

- prefer deterministic scenario setup and teardown
- prefer explicit fault injection over timing-based flakiness
- keep `chain_id` assertions first-class in distributed scenarios
- keep the fast local harness and the slower distributed harness clearly separated

### Avoid In Phase 9

- rebranding the current local integration suite as e2e without adding real service boundaries
- hiding fault injection in shell sleeps or manual steps only
- burying `chain_id` verification in incidental log scraping
- drifting into live AWS/mobile validation work

---

## 8. Evidence Capture Requirements

Phase 9 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or commit message |
| starting commit SHA | `git rev-parse HEAD` | phase notes or commit message |
| Phase 1-8 prerequisite status | implementation and validation records | phase notes |
| pre-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |
| distributed harness topology evidence | compose / harness config and scenario docs | phase notes |
| fault injection coverage | `failover.rs` and harness helpers | phase notes |
| trace continuity evidence | `trace.rs` and scenario artifacts | phase notes |
| scenario runner evidence | `run-conduit-e2e.sh` usage and outputs | phase notes |
| validation command set used | local command log | phase notes |
| post-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |

Do not sign off Phase 9 with only "tests added." Record the actual distributed scenarios, the injected faults, and the trace evidence they produce.

---

## 9. Recommended Execution Order

Implement Phase 9 in this order:

1. Capture the starting branch, commit SHA, and protected-path diff state.
2. Add `tests/e2e/common/mod.rs` and define the shared harness helpers.
3. Add the distributed scenario files in order: bootstrap, refresh, data_path, failover, trace.
4. Add deterministic fault-injection helpers and wiring.
5. Add `run-conduit-e2e.sh`.
6. Hook the distributed harness to the local compose topology from Phase 8.
7. Run the full local distributed harness.
8. Run the required V1 and V2 preservation checks.

This order stabilizes the shared harness boundary before scenario files start depending on it.

---

## 10. Validation Commands

Run these from the repo root unless noted otherwise:

Standard V2 checks:

```bash
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
```

If the default Windows temp or target location on `C:` is space-constrained, rerun the workspace suite with an explicit alternate target directory and record it, for example:

```bash
cargo test --workspace --target-dir D:\veritas-proto006-phase9-target
```

Distributed harness checks:

```bash
bash prototype/gbn-bridge-proto/infra/scripts/run-conduit-e2e.sh
```

```bash
docker compose -f prototype/gbn-bridge-proto/docker-compose.conduit-e2e.yml config
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
bash validate-scale-test.sh
```

Recommended Phase 9-specific checks:

```bash
rg -n "tests/e2e|run-conduit-e2e" prototype/gbn-bridge-proto
```

```bash
git status --short
```

Expected outcome:

- a real distributed e2e harness exists
- deterministic fault-injection scenarios exist
- one scenario proves end-to-end `chain_id` continuity
- the local full topology can be exercised through one harness entrypoint
- protected V1 paths show no drift
- minimum V1 regression suite remains green
- the full V2 workspace suite is green, using an alternate target directory if the default Windows temp path is space-constrained

---

## 11. Acceptance Criteria

Phase 9 is complete when:

- `tests/e2e/` exists with distributed scenarios
- deterministic fault injection exists and is exercised
- a dedicated `chain_id` continuity scenario exists
- `run-conduit-e2e.sh` exists and runs the distributed harness
- all required V1 and V2 validation commands have been run and recorded

Phase 9 is not complete if:

- the repo still relies only on the old root integration harness for distributed claims
- no deterministic fault injection exists
- `chain_id` continuity is not asserted in at least one distributed scenario

---

## 12. Risks And Blockers

| Risk | Why It Matters | Mitigation |
|---|---|---|
| old integration tests are mistaken for true e2e coverage | distributed regressions could still slip through | create a dedicated `tests/e2e/` boundary and keep its role explicit |
| fault injection is nondeterministic | failures will be flaky and hard to trust | use explicit triggers and controlled scenario hooks |
| distributed harness depends on placeholder topology | the harness would validate the wrong thing | make Phase 8 completion a real prerequisite |
| trace assertions are too weak | `chain_id` regressions may still pass silently | add dedicated distributed trace scenario and explicit assertions |

---

## 13. Sign-Off Recommendation

The correct Phase 9 sign-off is:

- Conduit now has a real distributed e2e harness
- failure and recovery behavior are tested deterministically
- one distributed trace can be correlated end-to-end by `chain_id`
- the dedicated WSL runner and full V2 workspace suite both pass, with the workspace suite allowed to use an alternate non-`C:` target directory on space-constrained hosts

The correct Phase 9 sign-off is not:

- adding more local-only integration tests
- relying on placeholder smoke containers
- claiming distributed behavior without fault-injection coverage
