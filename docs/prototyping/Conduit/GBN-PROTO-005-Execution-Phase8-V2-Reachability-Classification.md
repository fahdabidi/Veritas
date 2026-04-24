# GBN-PROTO-005 - Execution Phase 8 Detailed Plan: Reachability Classification

**Status:** Completed from the committed Phase 7 weak-discovery baseline
**Primary Goal:** implement V2 reachability classification and policy for `direct`, `brokered`, and `relay_only` bridges without mutating the frozen V1 role/tag semantics or allowing non-direct bridges to leak into first-contact bootstrap and immediate creator-ingress paths
**Source Plan:** [GBN-PROTO-005 Execution Plan](GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign-Execution-Plan.md)
**Phase 0 Baseline Release:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)
**Protocol Baseline:** [GBN-ARCH-002-Bridge-Protocol-V2](../architecture/GBN-ARCH-002-Bridge-Protocol-V2.md)

---

## 1. Current Repo Findings

These findings should drive Phase 8 execution instead of being rediscovered during implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 8 notes should capture the mainline commit used to begin reachability work |
| Current HEAD commit | `92eba2f291d46b798760320595b98c49228f734d` | this is the committed Phase 7 weak-discovery baseline that Phase 8 now builds on |
| Phase 0 baseline release | `veritas-lattice-0.1.0-baseline` published | V1 Lattice remains the preservation reference point for transport semantics and no-touch checks |
| Current protocol crate state | `BridgeDescriptor` already carries `reachability_class` and signed `udp_punch_port` | Phase 8 should consume the committed wire fields rather than reopen Phase 2 schema design |
| Current publisher/runtime state | authority, bridge runtime, creator bootstrap, bridge-mode upload, and weak discovery are committed | Phase 8 can focus on policy and classification instead of transport scaffolding |
| Current discovery state | weak discovery is now committed as hint-only and cannot elevate trust | Phase 8 must preserve that trust boundary while adding classification behavior |
| Current selector state | creator refresh selection is effectively direct-only today | Phase 8 should make that policy explicit and safe under downgrade / port changes |
| Current local state model | `LocalDhtNode` already stores signed `reachability_class` and `udp_punch_port` | Phase 8 should update cache semantics without turning stale entries into valid transport candidates |
| Current architecture rule | V2 docs require only `direct` bridges for first-contact bootstrap and initial bridge set | publisher and creator policy must enforce that rule in code and tests |
| Current protected V1 path drift | none | V1 preservation remains a hard sign-off gate |
| Current validation environment risk | OneDrive-backed V2 `target/` writes still fail with Windows `os error 5` during `cargo test --workspace` | Phase 8 validation should expect the temp `--target-dir` fallback again |

---

## 2. Review Summary

The master plan already says Phase 8 is about `direct`, `brokered`, and `relay_only`, but a robust implementation needs tighter policy boundaries:

| Gap | Why It Matters | Resolution For Phase 8 |
|---|---|---|
| Classification source ambiguity | publisher and creator can drift if each infers class differently | make publisher authority the canonical class issuer and creator the canonical class enforcer |
| Bootstrap eligibility ambiguity | brokered or relay-only bridges can accidentally appear in first-contact paths | lock direct-only seed-bridge and initial bridge-set eligibility |
| Port transition ambiguity | signed `udp_punch_port` changes can leave creators with stale local state | update local signed state atomically and invalidate stale active assumptions safely |
| Discovery interaction ambiguity | weak hints can accidentally bypass class filtering | keep Phase 7 hint logic below signed reachability policy |
| Phase 9 bleed risk | policy work can sprawl into broader integration harness work | keep Phase 8 focused on classification, cache transitions, and selector behavior only |

Phase 8 should classify every bridge, but only `direct` bridges should remain creator-ingress eligible in the current Conduit milestone. `brokered` and `relay_only` need to exist as signed states now even if richer broker flows arrive later.

---

## 3. Scope Lock

### In Scope

- implement publisher-side bridge classification and scoring in V2-local code
- implement explicit bridge eligibility rules for:
  - seed-bridge selection
  - initial bootstrap bridge-set selection
  - ordinary signed catalog refresh
- implement creator-side filtering by signed `reachability_class`
- implement creator-side handling for signed `udp_punch_port` transitions
- implement safe downgrade behavior when a bridge moves from `direct` to `brokered` or `relay_only`
- add V2-local tests for class filtering, downgrade, cache updates, and signed port transitions

### Out Of Scope

- modifying V1 subnet tags such as `FreeSubnet` or `HostileSubnet`
- changing V1 DHT validation loops or V1 role semantics
- adding broker rendezvous transports or relay-only upload paths
- reopening Phase 7 trust / discovery precedence rules
- updating the main repo `README.md`

---

## 4. Preflight Gates

Phase 8 should not begin code edits until all of these are checked:

1. Confirm the committed Phase 7 weak-discovery baseline is present and clean.
2. Confirm protected V1 paths are clean in the local worktree.
3. Confirm `BridgeDescriptor` already carries signed `reachability_class` and `udp_punch_port`.
4. Confirm Phase 8 stays inside `gbn-bridge-publisher`, `gbn-bridge-runtime`, V2-local tests, and any minimal V2-local protocol/cache support changes.
5. Confirm the current trust rule remains intact: only signed, non-expired descriptors are transport-authoritative.
6. Confirm brokered and relay-only states are policy states in this phase, not fully supported creator-ingress transports.
7. Confirm the V2 validation command shape if the default V2 `target/` path remains blocked by OneDrive.
8. Confirm Phase 8 will not modify the main repo `README.md`; any Conduit README rewrite remains deferred until V2 code work is complete.

Current blocker:

- none; Phase 8 is implemented locally, validated, and ready to commit

---

## 5. Reachability Decisions To Lock In Phase 8

### 5.1 Module Boundary

Phase 8 should keep the reachability surface structured like this:

| Module | Responsibility |
|---|---|
| `publisher/src/policy.rs` | authoritative eligibility rules for seed, bootstrap set, and refresh selection |
| `publisher/src/bridge_scoring.rs` | deterministic ranking and tie-break behavior within a reachability class |
| `runtime/src/reachability.rs` | creator-side class filtering, signed port handling, downgrade transitions, and cache safety helpers |
| existing `selector.rs` | transport candidate ordering after class filtering has been applied |
| existing `catalog_cache.rs` / `local_dht.rs` | store signed class / port state, but do not invent policy independently |

Do not smear classification rules across unrelated creator/bootstrap/upload files.

### 5.2 Canonical Class Semantics

Phase 8 should lock these meanings:

- `direct`
  - creator-ingress capable now
  - eligible for seed-bridge selection
  - eligible for initial bridge-set assignment
  - eligible for ordinary direct refresh selection
- `brokered`
  - signed as alive but not creator-ingress capable for current M1 bootstrap
  - not eligible for first-contact seed or initial bridge set
  - not eligible for current direct refresh selection
- `relay_only`
  - signed as publisher-connected but not creator-ingress capable
  - not eligible for creator bootstrap or refresh transport

### 5.3 Eligibility Rules

Phase 8 should lock these behaviors:

- seed-bridge selection: `direct` only
- initial bootstrap bridge set: `direct` only
- immediate creator punch fanout: `direct` only
- current creator-side refresh selection: `direct` only
- brokered and relay-only descriptors may remain in signed catalogs, but creator transport selection must filter them out

### 5.4 Signed UDP Punch Port Rules

Phase 8 should lock these behaviors:

- creators must respect the signed `udp_punch_port` from the freshest valid descriptor or bootstrap entry
- when a bridge’s signed port changes, local signed state should update atomically
- stale active-tunnel assumptions should not silently survive a signed port transition
- tests should prove creators do not keep using an old port after signed state changes

### 5.5 Downgrade And Cache Rules

Phase 8 should lock these behaviors:

- if a bridge downgrades from `direct` to `brokered` or `relay_only`, it must be removed from new creator selections
- if a downgraded bridge is present in creator-local signed state, it may remain as signed metadata but not as a transport candidate
- active candidate lists should be recomputed from the freshest signed state, not stale local assumptions
- discovery hints cannot re-promote a downgraded bridge

---

## 6. Dependency And Implementation Policy

Phase 8 should keep dependencies minimal and bias toward deterministic in-process tests.

### Required Bias

- reuse committed publisher authority, creator runtime, weak discovery, and selector surfaces
- represent class transitions and port transitions through signed catalog/bootstrap data
- structure ranking logic so tests inject descriptors and timestamps directly
- prefer small, deterministic scoring / policy helpers over broad policy frameworks

### Avoid In Phase 8

- changing V1 subnet or role semantics
- adding broker rendezvous networking or relay transport support
- reopening weak discovery trust rules
- mixing Phase 9 harness concerns into Phase 8 policy code

---

## 7. Evidence Capture Requirements

Phase 8 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or commit message |
| starting commit SHA | `git rev-parse HEAD` | phase notes or commit message |
| pre-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |
| seed/bootstrap eligibility rules used | Phase 8 plan + tests | phase notes |
| class downgrade behavior used | Phase 8 plan + tests | phase notes |
| signed port transition behavior used | Phase 8 plan + tests | phase notes |
| validation command set used | local command log | phase notes |
| temp `--target-dir` workaround, if needed | local command log | phase notes |
| post-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |

---

## 8. Recommended Execution Order

Implement Phase 8 in this order:

1. Capture the starting branch, commit SHA, and protected-path diff state.
2. Add publisher-side `policy.rs` and `bridge_scoring.rs` first so class/eligibility rules are fixed early.
3. Add runtime-side `reachability.rs` next so creator cache/selection behavior follows those rules.
4. Integrate the new policy with existing selector, catalog, bootstrap, and local-state updates.
5. Add `tests/reachability.rs`.
6. Run the V2 validation commands.
7. Run the V1 preservation checks and minimum V1 regressions.

This keeps publisher-issued policy stable before creator-side cache and selector behavior start depending on it.

---

## 9. Validation Commands

Run these from the repo root unless noted otherwise:

Standard path:

```bash
cd prototype/gbn-bridge-proto
cargo fmt --check
cargo check --workspace
cargo test --workspace
```

If the OneDrive-backed workspace still throws Windows `os error 5` on target writes, use the documented temp-target fallback and record it in the phase notes:

```powershell
$target = Join-Path $env:LOCALAPPDATA 'Temp\veritas-bridge-target-phase8'
New-Item -ItemType Directory -Path $target -Force | Out-Null
$env:CARGO_INCREMENTAL='0'
cargo test --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml --target-dir $target
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

- only `direct` bridges are returned to creators for first-contact bootstrap
- only `direct` bridges are included in the initial new-creator bridge set
- `brokered` and `relay_only` bridges are excluded from first-contact seed selection and immediate creator fanout
- class downgrade removes a bridge from new creator selections and active refresh candidates
- signed `udp_punch_port` transitions update local signed state safely
- protected V1 diff remains empty
- minimum V1 regression suite still passes

---

## 10. Acceptance Criteria

Phase 8 is complete only when all of the following are true:

- every file listed in the main execution plan exists
- publisher-side classification and scoring are implemented in V2-local code
- seed-bridge, initial bridge-set, and current refresh eligibility are class-aware
- creator-side filtering by signed `reachability_class` is implemented
- signed `udp_punch_port` transitions are handled safely in local signed state
- downgrade behavior removes non-direct bridges from creator transport selection
- tests cover class filtering, downgrade, and signed port transitions
- protected V1 diff is clean after validation
- minimum V1 regression suite still passes

---

## 11. Risks And Blockers

| Risk | What It Looks Like | Mitigation |
|---|---|---|
| Policy drift | publisher and creator disagree about what class is eligible | centralize publisher-issued class semantics and keep creator-side enforcement deterministic |
| Bootstrap weakening | brokered or relay-only bridges leak into first-contact flows | lock direct-only eligibility in publisher policy and test it explicitly |
| Port-staleness bugs | creator keeps using an old UDP port after signed state changes | treat signed port updates as authoritative cache transitions |
| Discovery re-promotion | weak hints make downgraded bridges appear usable again | preserve Phase 7 trust ordering and filter by signed class after merge |
| Phase 9 bleed | reachability work expands into broader integration harness concerns | keep Phase 8 focused on classification and cache transitions only |
| OneDrive validation noise | V2 tests fail for filesystem reasons instead of code reasons | use and document the temp-target fallback |

Current blocker:

- none; Phase 8 is implemented locally, validated, and ready to commit

---

## 12. First Implementation Cut

If Phase 8 is implemented as a single focused change set, use this breakdown:

1. Publisher policy and scoring
2. Runtime reachability filtering and signed-port cache handling
3. Bootstrap / refresh integration
4. Reachability-focused tests and V1 preservation validation

That keeps the class semantics auditable and ensures Phase 9 inherits a stable transport-eligibility policy instead of a moving selector contract.
