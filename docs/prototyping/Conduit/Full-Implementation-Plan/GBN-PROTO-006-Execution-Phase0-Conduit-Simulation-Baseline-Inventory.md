# GBN-PROTO-006 - Execution Phase 0 Detailed Plan: Inventory The Current Conduit Simulation Baseline

**Status:** Completed and validated locally on 2026-04-23  
**Primary Goal:** record the currently committed Conduit implementation as the starting simulation state, identify the exact production gaps, and define the remediation order without freezing or publishing the simulation itself  
**Source Plan:** [GBN-PROTO-006 Execution Plan](GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md)  
**Protected V1 Baseline:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)  
**Full-Implementation Starting Commit:** `2b6d5c5d24e269e96e3fdc820f3f90669607414a`

---

## 1. Current Repo Findings

These findings should drive the Phase 0 inventory instead of being rediscovered during later implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 0 should record the starting branch for the full-implementation track |
| Current HEAD commit | `2b6d5c5d24e269e96e3fdc820f3f90669607414a` | this is the current committed starting point for the Conduit full-implementation plan |
| V1 baseline release | `veritas-lattice-0.1.0-baseline` | V1 remains the protected published baseline for all GBN-PROTO-006 work |
| Current worktree state | documentation reorganization in progress under `docs/prototyping/` | Phase 0 validated protected-path cleanliness rather than assuming a globally clean worktree |
| Current publisher deployment entrypoint | placeholder binary in [gbn-bridge-cli/src/lib.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/lib.rs:1>) | proves Conduit still lacks a real deployed publisher service boundary |
| Current publisher service layer | wrapper-only `AuthorityServer` in [gbn-bridge-publisher/src/server.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/server.rs:1>) | proves the publisher authority logic is not yet exposed as a real network service |
| Current runtime publisher coupling | `InProcessPublisherClient` in [gbn-bridge-runtime/src/publisher_client.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/publisher_client.rs:1>) | proves creator and bridge production paths still depend on in-process coupling |
| Current smoke topology | `busybox` placeholders in [docker-compose.bridge-smoke.yml](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/docker-compose.bridge-smoke.yml:1>) | local smoke assets still represent placeholders rather than runnable Conduit services |
| Current AWS deployment assets | partial ECS/Fargate scaffold in [phase2-bridge-stack.yaml](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/infra/cloudformation/phase2-bridge-stack.yaml:1>) | confirms the AWS topology exists as a skeleton, not as a production-capable publisher/receiver/bridge control plane |
| Current V2 `chain_id` support | no `chain_id` hits under `prototype/gbn-bridge-proto/` | distributed trace continuity from V1 has not yet been carried into Conduit code paths |

---

## 2. Review Summary

Phase 0 is not a release phase. It is the control point that prevents the full-implementation effort from drifting into vague claims about what is already real versus what is still simulated.

The core facts Phase 0 must capture are:

| Gap | Why It Matters | Resolution For Phase 0 |
|---|---|---|
| No explicit inventory of simulated production boundaries | later phases could implement around assumptions instead of facts | create a written current-state inventory and gap inventory |
| No explicit distinction between committed Conduit code and production-capable Conduit services | local harness success could be mistaken for real service completeness | name the exact simulated boundaries and the exact service boundaries still missing |
| No written remediation order | work could start from the wrong layer and create rework | define the replacement order for publisher API, persistence, control sessions, network clients, bootstrap distribution, receiver path, trace propagation, and deployment |
| No V2 `chain_id` baseline assessment | later phases could bolt on tracing inconsistently | document where `chain_id` is currently absent and require it to become a first-class cross-cutting concern |
| Worktree is not globally clean because docs were reorganized | a naive clean-worktree gate would block planning for the wrong reason | require protected-path cleanliness and explicit documentation of the reorganization state rather than a blanket clean-worktree rule |

The goal of this phase is not to freeze or publish the simulation. The goal is to produce a disciplined starting inventory that later full-implementation phases can execute against.

---

## 3. Scope Lock

### In Scope

- create a detailed Phase 0 execution record for the full-implementation track
- define the exact content expected in:
  - `GBN-PROTO-006-Conduit-Simulation-Baseline.md`
  - `GBN-PROTO-006-Conduit-Gap-Inventory.md`
- capture the committed Conduit starting SHA
- record the current simulated production boundaries
- record the current absence of V2 `chain_id` propagation
- define the remediation order for later phases

### Out Of Scope

- publishing or tagging the Conduit simulation
- creating a GitHub release for Conduit simulation state
- implementing the real publisher service in this phase
- changing any file under `prototype/gbn-proto/`
- modifying `README.md`
- starting Phase 1 code changes

---

## 4. Preflight Gates

Phase 0 should not begin writing the inventory artifacts until all of these are checked:

1. Confirm the starting Conduit commit to inventory is agreed. Default target is current `HEAD` on `main`.
2. Confirm protected V1 paths are clean in the local worktree.
3. Confirm the current non-clean worktree state is understood to be documentation-path reorganization, not protected V1 drift.
4. Confirm the Conduit full-implementation track will not create a release, tag, or freeze artifact for the current simulation state.
5. Confirm the Phase 0 scope remains documentation-only.
6. Confirm V1 regression commands still run from the current environment.
7. Confirm `README.md` remains out of scope.

If any gate fails, Phase 0 should stop and record the blocker explicitly.

Phase 0 execution result:

- starting inventory docs created:
  - `GBN-PROTO-006-Conduit-Simulation-Baseline.md`
  - `GBN-PROTO-006-Conduit-Gap-Inventory.md`
- protected V1 paths validated clean after restoring the two `GBN-PROTO-004` docs at their protected locations
- V1 minimum regression suite passed
- V2 workspace sanity suite passed using temp target dir `%LOCALAPPDATA%\Temp\veritas-proto006-phase0-target`
- no Conduit release, tag, or freeze artifact was created

---

## 5. Evidence Capture Requirements

Phase 0 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | current-state inventory |
| starting commit SHA | `git rev-parse HEAD` | current-state inventory and gap inventory |
| protected-path diff status | `git diff --name-only -- <protected paths>` | phase notes or current-state inventory |
| current worktree status | `git status --short` | phase notes or current-state inventory |
| placeholder publisher entrypoint evidence | `crates/gbn-bridge-cli/src/lib.rs` | gap inventory |
| in-process publisher coupling evidence | `crates/gbn-bridge-runtime/src/publisher_client.rs` | gap inventory |
| wrapper-only server evidence | `crates/gbn-bridge-publisher/src/server.rs` | gap inventory |
| placeholder local smoke evidence | `docker-compose.bridge-smoke.yml` | gap inventory |
| partial AWS scaffold evidence | `infra/cloudformation/phase2-bridge-stack.yaml` | gap inventory |
| V2 `chain_id` search result | `rg -n "chain_id|id_chain|TraceId" prototype/gbn-bridge-proto ...` | current-state inventory and gap inventory |
| V1 `chain_id` reference locations | `prototype/gbn-proto/crates/mcn-router-sim/src/control.rs` and related scripts | current-state inventory and later trace design references |

Do not sign off Phase 0 with generic statements like "Conduit is still simulated." Record the exact files and evidence proving that claim.

---

## 6. Required Current-State Inventory Axes

`GBN-PROTO-006-Conduit-Simulation-Baseline.md` should capture the current state in these sections:

1. Purpose
2. Starting branch and commit
3. Current Conduit workspace summary
4. Current implemented crates and their intended role
5. Current service-boundary reality
6. Current deployment-boundary reality
7. Current trace / observability reality
8. Current validation reality
9. Known non-goals for Phase 0

The current-state inventory should describe what exists today without trying to solve the gaps.

---

## 7. Required Gap Inventory Axes

`GBN-PROTO-006-Conduit-Gap-Inventory.md` should organize the gaps by production boundary, not by file count.

Minimum sections:

1. Publisher authority API gap
2. Durable storage and restart recovery gap
3. Bridge control-session gap
4. Creator/host-creator network client gap
5. Real bootstrap distribution and fanout gap
6. Real receiver / ACK path gap
7. Distributed `chain_id` propagation gap
8. Deployment image and AWS topology gap
9. Distributed test-harness gap
10. Live validation gap
11. Recommended remediation order

Each gap section should state:

- what the architecture requires
- what the current implementation actually does
- why the current implementation is insufficient for production
- which later GBN-PROTO-006 phase should resolve it

---

## 8. File-By-File Plan

| File | Required Content |
|---|---|
| `docs/prototyping/Conduit/Full-Implementation-Plan/GBN-PROTO-006-Conduit-Simulation-Baseline.md` | purpose, starting branch/SHA, committed Conduit scope, current boundary reality, validation reality, current `chain_id` state |
| `docs/prototyping/Conduit/Full-Implementation-Plan/GBN-PROTO-006-Conduit-Gap-Inventory.md` | exact simulated/placeholder/in-process gaps, architecture comparison, why each gap matters, recommended remediation order |
| `docs/prototyping/Conduit/Full-Implementation-Plan/GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md` | optional narrow update only to link this detailed Phase 0 execution reference |

The baseline doc should be descriptive. The gap-inventory doc should be evaluative and operational.

---

## 9. Recommended Document Structure

Use this minimum structure for the current-state inventory:

1. Purpose
2. Starting point
3. Current committed Conduit modules
4. Current runtime and publisher boundaries
5. Current deployment boundaries
6. Current trace and observability boundaries
7. Current validation state
8. Summary of what Conduit is and is not yet

Use this minimum structure for the gap-inventory doc:

1. Purpose
2. Architectural requirement summary
3. Gap-by-gap inventory
4. Trace / `chain_id` inventory
5. Recommended implementation order
6. Exit criteria for Phase 0

---

## 10. Recommended Execution Order

Implement Phase 0 in this order:

1. Capture the starting branch, SHA, and protected-path diff state.
2. Capture the current worktree state and explain the documentation reorganization.
3. Record the real current Conduit service-boundary state.
4. Record the real current deployment-boundary state.
5. Record the current V2 `chain_id` absence and the V1 `chain_id` reference points.
6. Draft `GBN-PROTO-006-Conduit-Simulation-Baseline.md`.
7. Draft `GBN-PROTO-006-Conduit-Gap-Inventory.md`.
8. Link this detailed execution reference from the master GBN-PROTO-006 plan.
9. Run the required V1 and V2 validation commands.
10. Stop and wait for explicit approval before Phase 1.

This order keeps the phase factual. It prevents later phases from being planned on top of an unverified picture of the current Conduit implementation.

---

## 11. Validation Commands

Run these from the repo root unless noted otherwise:

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

```bash
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
```

```bash
rg -n "chain_id|id_chain|TraceId" \
  prototype/gbn-proto \
  prototype/gbn-bridge-proto \
  docs/architecture/GBN-ARCH-000-System-Architecture-V2.md \
  docs/architecture/GBN-ARCH-001-Media-Creation-Network-V2.md
```

```bash
git status --short
```

Expected outcome:

- protected V1 paths show no drift
- V1 regression suite passes
- V2 workspace sanity suite passes
- V2 `chain_id` search results are captured explicitly, even if empty
- current worktree state is explained in the phase notes

---

## 12. Acceptance Criteria

Phase 0 is complete when:

- the starting Conduit commit is recorded explicitly
- the current simulation baseline is documented without pretending it is production-complete
- every known simulated or placeholder production boundary is named
- the absence or incompleteness of V2 `chain_id` propagation is documented explicitly
- the remediation order for later phases is recorded
- no release, tag, or publication artifact is created for the Conduit simulation baseline
- all required V1 and V2 validation commands have been run and recorded

---

## 13. Risks And Blockers

| Risk | Why It Matters | Mitigation |
|---|---|---|
| Documentation reorganization confuses path assumptions | a reviewer may mistake moved docs for unrelated churn | record the current worktree state explicitly and keep Phase 0 path references anchored to the new Conduit/Lattice folders |
| Gap inventory becomes aspirational instead of evidentiary | later phases could be planned against guesses | require file-level evidence for each claimed simulation boundary |
| `chain_id` is deferred mentally to a later "observability phase" | trace continuity would become inconsistent across service boundaries | record Phase 0 that `chain_id` is a cross-cutting requirement from the start |
| Placeholder deployment assets are mistaken for runnable services | later AWS work could be underestimated | document the exact placeholder evidence from compose, CLI, and CloudFormation assets |

---

## 14. Sign-Off Recommendation

The correct Phase 0 sign-off is:

- the Conduit simulation baseline is understood
- the exact implementation gaps are written down
- the full-implementation phases can start from evidence

The correct Phase 0 sign-off is not:

- a release
- a tag
- a publication artifact
- a claim that Conduit is already production-capable
