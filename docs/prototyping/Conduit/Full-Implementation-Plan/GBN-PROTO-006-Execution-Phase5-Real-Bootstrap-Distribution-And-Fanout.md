# GBN-PROTO-006 - Execution Phase 5 Detailed Plan: Real Bootstrap Distribution And Fanout

**Status:** Ready to start after Phase 4 runtime network-client replacement is implemented and validated  
**Primary Goal:** replace the current in-process bootstrap handoff with a real publisher-owned bootstrap session, seed-assignment delivery path, bridge-set distribution path, and remaining-bridge fanout activation path, while preserving the Phase 1 authority API, the Phase 2 durable state model, the Phase 3 bridge control sessions, and the Phase 4 runtime network clients  
**Source Plan:** [GBN-PROTO-006 Execution Plan](GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md)  
**Protected V1 Baseline:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)  
**Phase 4 Detailed Plan:** [GBN-PROTO-006-Execution-Phase4-Replace-In-Process-Clients-With-Network-Clients](GBN-PROTO-006-Execution-Phase4-Replace-In-Process-Clients-With-Network-Clients.md)  
**Starting Conduit Baseline:** `2b6d5c5d24e269e96e3fdc820f3f90669607414a`

---

## 1. Current Repo Findings

These findings should drive Phase 5 instead of being rediscovered during implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 5 should record the mainline commit used to begin the bootstrap cutover |
| Current HEAD commit | `2b6d5c5d24e269e96e3fdc820f3f90669607414a` | current committed Conduit baseline still models bootstrap distribution locally |
| Current publisher bootstrap output | [`bootstrap.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/bootstrap.rs) returns `AuthorityBootstrapPlan` directly | proves the Publisher currently computes bootstrap decisions, but does not distribute them over real service boundaries |
| Current host-creator first-contact path | [`host_creator.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/host_creator.rs) returns `AuthorityBootstrapPlan` by direct runtime call | first-contact bootstrap is still a local object handoff, not a real host-creator to publisher to bridge flow |
| Current creator bootstrap path | [`bootstrap.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bootstrap.rs) applies the returned plan directly to the creator and seed bridge | creator first-contact behavior still assumes a local authority plan is already present |
| Current seed-bridge state | [`bootstrap_bridge.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bootstrap_bridge.rs) stores `SeedBridgeAssignment` objects in a local map via `assign_from_plan(...)` | the seed bridge still learns bootstrap state from a local plan object, not from a publisher-issued control command |
| Current bridge-set delivery | [`bootstrap_bridge.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bootstrap_bridge.rs) serves a locally cached `BridgeSetResponse` | bridge-set distribution is still a local cache lookup, not a real publisher-to-bridge distribution path |
| Current remaining-bridge fanout | [`punch_fanout.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/punch_fanout.rs) tracks creator punch attempts entirely in local runtime state | remaining-bridge fanout is still creator-local bookkeeping rather than publisher-driven orchestration across real bridge sessions |
| Current publisher bootstrap modules | no `bootstrap/session.rs`, `bootstrap/distribution.rs`, or `bootstrap/fanout.rs` exist under `gbn-bridge-publisher/src/bootstrap/` | the publisher-side bootstrap state machine, distribution logic, and fanout logic are not yet factored into production modules |
| Current bridge command path | Phase 3 is intended to provide real publisher-to-bridge control sessions | Phase 5 must reuse that control plane for seed assignment and fanout instead of inventing a second bridge command channel |
| Current runtime network path | Phase 4 is intended to replace `InProcessPublisherClient` in production paths | Phase 5 should assume creator, host-creator, and bridge requests already cross real authority/service boundaries |
| Current `chain_id` state | `chain_id` is still absent from bootstrap session creation, seed assignment, bridge-set delivery, and fanout activation | Phase 5 must make bootstrap tracing continuous across every hop in the first-contact flow |

---

## 2. Review Summary

Phase 5 is where Conduit must stop treating bootstrap distribution as a plan that appears locally in memory and start treating it as a publisher-owned distributed workflow. If this phase is weak, the Publisher may be networked and durable, but it still will not satisfy the architecture requirement that it actively distribute the seed bridge assignment, bridge-set payload, and remaining-bridge fanout instructions.

The main gaps the detailed Phase 5 plan must close are:

| Gap | Why It Matters | Resolution For Phase 5 |
|---|---|---|
| `AuthorityBootstrapPlan` is still the effective transport | creator, host-creator, and bridge runtimes are still coupled to a local plan object | replace plan handoff with a real publisher bootstrap session and command delivery path |
| Seed bridge learns assignments locally | `ExitBridgeB` is supposed to receive a publisher-issued bootstrap assignment over the bridge control plane | deliver a signed seed-assignment command over the real control session |
| Bridge-set payload is not actually distributed | the architecture requires the publisher to distribute the 9 bridge entries back through the network | persist the bridge-set payload and deliver it through the seed bridge on demand over real boundaries |
| Remaining bridge fanout is still creator-local | the publisher must actively trigger the remaining bridges | add a publisher-owned fanout activator that dispatches the remaining bridge commands over control sessions |
| No explicit timeout and reassignment model | first-contact bootstrap cannot recover from a seed bridge that never comes up | define timeout, retry, and reassignment rules as part of the durable bootstrap session state machine |
| `chain_id` breaks across bootstrap stages | there is no root distributed trace for host-creator join, publisher decision, seed assignment, tunnel-up progress, and remaining fanout | require `chain_id` continuity across every bootstrap record, command, progress event, and test assertion |
| Overreach risk | it is easy to drift into the receiver / ACK data path | keep the publisher receiver and data-path ACKs deferred to Phase 6 |

Phase 5 should make the full bootstrap distribution path real, but it should not yet replace the bridge-to-publisher forwarding / receiver path for payload ingress.

---

## 3. Scope Lock

### In Scope

- implement a publisher-owned bootstrap session state machine
- add publisher-side `bootstrap/session.rs`, `bootstrap/distribution.rs`, and `bootstrap/fanout.rs`
- replace local `AuthorityBootstrapPlan` handoff in production bootstrap paths
- deliver the seed bridge assignment to `ExitBridgeB` over the real bridge control session
- return the bootstrap response through the real host-creator / relay path
- persist and distribute the signed bridge-set payload for the remaining bridge entries
- activate remaining bridge fanout over real publisher-to-bridge control sessions
- define timeout, retry, and reassignment rules for failed or silent seed bridges
- propagate `chain_id` across the entire bootstrap session
- add end-to-end bootstrap distribution tests over real service boundaries

### Out Of Scope

- publisher receiver and ACK data path
- upload chunk forwarding
- AWS deployment promotion
- modifying `prototype/gbn-proto/**`
- modifying the main repo `README.md`

---

## 4. Preflight Gates

Phase 5 should not begin code edits until all of these are checked:

1. Confirm the Phase 0 inventory deliverables exist.
2. Confirm Phase 1 is implemented and validated so the Publisher already exposes a real authority API.
3. Confirm Phase 2 is implemented and validated so bootstrap session, assignment, and progress state can be persisted durably.
4. Confirm Phase 3 is implemented and validated so real bridge control sessions exist for publisher-issued commands.
5. Confirm Phase 4 is implemented and validated so creator, host-creator, and bridge runtime code no longer depends on in-process publisher coupling.
6. Confirm protected V1 paths are clean in the local worktree.
7. Confirm the seed bridge assignment, bridge-set distribution, and remaining-bridge fanout will all use the real publisher-owned boundary rather than local plan sharing.
8. Confirm `chain_id` will be preserved from host-creator join request through bootstrap completion reporting.
9. Confirm `README.md` remains out of scope.

If any gate fails, Phase 5 should stop.

Current blocker:

- Phases 1 through 4 are not yet implemented in this full-implementation track, so Phase 5 remains planning-ready only

---

## 5. Bootstrap Distribution Decisions To Lock In Phase 5

### 5.1 Publisher-Owned Bootstrap Session State Machine

Phase 5 should treat bootstrap as a real publisher-owned durable workflow, not as a value object returned to runtime code.

Minimum durable states:

- `created`
- `seed_assigned`
- `seed_acknowledged`
- `bootstrap_response_returned`
- `seed_tunnel_reported`
- `bridge_set_delivered`
- `fanout_activated`
- `completed`
- `expired`
- `reassigned`
- `failed`

The session record should persist at least:

- `bootstrap_session_id`
- `chain_id`
- creator identity and endpoint metadata
- host-creator id
- relay bridge id
- current seed bridge id
- remaining fanout bridge ids
- response expiry and timeout timestamps
- current attempt counters
- progress timestamps for each major transition

### 5.2 Seed Assignment Delivery Rule

`ExitBridgeB` must receive the seed assignment over the real bridge control session from Phase 3.

That assignment should carry:

- `bootstrap_session_id`
- `chain_id`
- creator bootstrap descriptor
- seed bridge role metadata
- timeout / expiry information
- the signed punch directive or equivalent authoritative seed command payload

Do not let the seed bridge continue learning bootstrap state by directly calling `assign_from_plan(...)` on a local plan object.

### 5.3 Bootstrap Response Return Rule

The bootstrap response back to `NewCreator` must travel through the real host-creator / relay path rather than being returned as part of an in-process object graph.

The returned response must include:

- `bootstrap_session_id`
- `chain_id`
- signed `CreatorBootstrapResponse`
- seed bridge identity and transport information
- publisher public key

The host creator remains a transport sponsor only. It must not become a second authority or a second source of bridge-set truth.

### 5.4 Bridge-Set Delivery Rule

The 9 bridge entries must be treated as a real publisher-distributed payload.

Phase 5 should lock this behavior:

- the Publisher persists the signed bridge-set payload per bootstrap session
- `ExitBridgeB` receives enough state to serve the bridge-set to the creator on request
- the creator fetches the bridge set through the real seed-bridge path
- delivery is tied to the bootstrap session id and `chain_id`

Do not keep bridge-set delivery as only a local map lookup populated from the original authority plan.

### 5.5 Remaining Bridge Fanout Rule

After the seed tunnel is reported as up, the Publisher must actively fan out to the remaining bridges.

Required behavior:

- the Publisher loads the remaining eligible bridge ids from durable bootstrap session state
- the Publisher issues real fanout commands to those bridges over the control-session layer
- each fanout command is correlated to the same `bootstrap_session_id` and `chain_id`
- the creator is instructed to begin tunneling toward those bridges only after the publisher has activated them

The current creator-local `PunchFanout` bookkeeping may remain as local runtime state, but it must become a consumer of publisher-driven bootstrap fanout rather than the source of truth.

### 5.6 Timeout, Retry, And Reassignment Rule

Phase 5 should explicitly lock a reassignment model.

Minimum requirements:

- a seed-assignment ACK timeout
- a seed-tunnel-up timeout
- a bridge-set-delivery timeout
- a reassignment path if the chosen seed bridge never ACKs or never reports progress
- bounded retry counts
- explicit terminal failure state after configured exhaustion

If the seed bridge is reassigned:

- the bootstrap session must record the reassignment
- a new seed command is issued to the replacement bridge
- the original bridge is not allowed to keep acting on a stale command without dedupe / session validation

### 5.7 Batch Window Rule

Phase 5 should preserve the architecture's batch behavior for first-contact joins.

Rules:

- up to 10 incoming join requests may share one 0.5 second assignment window
- the 11th request must roll into the next batch window
- batch assignment must not blur per-session `chain_id` values
- each creator bootstrap session remains individually addressable even when batch-selected together

### 5.8 ChainID Rule For Phase 5

Phase 5 must preserve the same root `chain_id` across:

- host-creator join request creation
- publisher bootstrap session creation
- seed assignment delivery
- bootstrap response return
- seed-tunnel progress reporting
- bridge-set delivery
- remaining bridge fanout commands
- completion or failure reporting

It is not enough for `chain_id` to appear only in logs. It must be present in the persisted bootstrap session, the command envelopes, and the test assertions that verify the full path.

### 5.9 HostCreator Boundary Rule

The host creator remains a bootstrap sponsor only.

Phase 5 must not let the host creator:

- choose the seed bridge
- edit the bridge set
- authoritatively sign bootstrap data
- become a durable store of bootstrap session truth

Those remain Publisher responsibilities.

---

## 6. Module Ownership To Lock In Phase 5

Phase 5 should keep responsibilities split like this:

| Module | Responsibility |
|---|---|
| `gbn-bridge-publisher/src/bootstrap/session.rs` | durable bootstrap session state machine, state transitions, timeout/reassignment bookkeeping |
| `gbn-bridge-publisher/src/bootstrap/distribution.rs` | creation and delivery of seed assignments and bridge-set distribution state |
| `gbn-bridge-publisher/src/bootstrap/fanout.rs` | remaining-bridge activation logic and fanout progress coordination |
| `gbn-bridge-publisher/src/bootstrap.rs` | top-level bootstrap orchestration glue; should stop returning a local `AuthorityBootstrapPlan` as the production path |
| `gbn-bridge-publisher/src/dispatcher.rs` | reuse Phase 3 command delivery for seed and fanout commands |
| `gbn-bridge-runtime/src/bootstrap.rs` | creator-side bootstrap orchestration over real responses and bridge-set retrieval |
| `gbn-bridge-runtime/src/bootstrap_bridge.rs` | seed-bridge-side assignment cache and serving behavior driven by real publisher commands, not plan injection |
| `gbn-bridge-runtime/src/punch_fanout.rs` | creator-local fanout attempt tracking that consumes publisher-issued fanout state instead of originating it |
| `gbn-bridge-runtime/tests/bootstrap_distribution.rs` | end-to-end bootstrap distribution, timeout, reassignment, batch-window, and `chain_id` continuity tests |

Do not let `bootstrap.rs` or `bootstrap_bridge.rs` become dumping grounds for all bootstrap state transitions. Keep publisher authority state in publisher bootstrap modules.

---

## 7. Dependency And Implementation Policy

Phase 5 should reuse the already selected network surfaces instead of introducing new transport stacks.

### Recommended Dependencies

- reuse the Phase 1 authority API stack for host-creator and creator request/response traffic
- reuse the Phase 3 bridge control-session transport for publisher-to-bridge command delivery
- reuse the Phase 2 durable storage and migration stack for bootstrap session persistence
- add timeout / scheduling helpers only if tightly scoped to bootstrap orchestration

### Bias

- keep bootstrap messages explicit and typed
- keep publisher-owned state transitions deterministic and auditable
- keep `chain_id` available in storage, envelopes, and tests
- prefer one canonical publisher-owned bootstrap session model over per-module ad hoc state

### Avoid In Phase 5

- inventing a second publisher-to-bridge delivery mechanism outside the control-session layer
- retaining `AuthorityBootstrapPlan` as the production data transport
- making the seed bridge or host creator the durable source of bootstrap truth
- silently allowing remaining bridge fanout to remain creator-local simulation
- drifting into payload receiver / ACK path work

---

## 8. Evidence Capture Requirements

Phase 5 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or commit message |
| starting commit SHA | `git rev-parse HEAD` | phase notes or commit message |
| Phase 1-4 prerequisite status | implementation and validation records | phase notes |
| pre-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |
| bootstrap session state model | `bootstrap/session.rs` and tests | phase notes |
| seed-assignment delivery path | `bootstrap/distribution.rs`, dispatcher, and tests | phase notes |
| bridge-set delivery path | `bootstrap_bridge.rs`, runtime tests, and publisher distribution code | phase notes |
| remaining bridge fanout path | `bootstrap/fanout.rs`, control-session traces, and tests | phase notes |
| timeout / reassignment behavior | durable session records and tests | phase notes |
| `chain_id` continuity evidence | storage records, command envelopes, runtime logs/tests | phase notes |
| validation command set used | local command log | phase notes |
| temp `--target-dir` workaround, if needed | local command log | phase notes |
| post-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |

Do not sign off Phase 5 with only "bootstrap works." Record exactly where seed assignment, bridge-set delivery, reassignment, and fanout are now crossing real boundaries and where `chain_id` is preserved.

---

## 9. Recommended Execution Order

Implement Phase 5 in this order:

1. Capture the starting branch, commit SHA, and protected-path diff state.
2. Introduce `bootstrap/session.rs` first and define the canonical durable state machine and timeout model.
3. Implement `bootstrap/distribution.rs` and wire seed-assignment creation plus bridge-set persistence.
4. Refactor `bootstrap.rs` so production bootstrap no longer returns a local `AuthorityBootstrapPlan`.
5. Update `dispatcher.rs` and bridge control handling so `ExitBridgeB` receives seed commands over the real control session.
6. Refactor `bootstrap_bridge.rs` so it consumes real publisher-issued seed assignment and bridge-set state.
7. Implement `bootstrap/fanout.rs` so remaining bridges are activated only after seed-tunnel progress is reported.
8. Refactor runtime `bootstrap.rs` and `punch_fanout.rs` so creator-side behavior consumes publisher-driven fanout instead of local simulation.
9. Add `tests/bootstrap_distribution.rs` covering normal flow, timeout, reassignment, batch-window rollover, and `chain_id` continuity.
10. Run the V2 workspace sanity suite.
11. Run the V1 preservation checks and minimum V1 regressions.

This order keeps the publisher-owned session and distribution model stable before bridge/runtime bootstrap code starts depending on it.

---

## 10. Validation Commands

Run these from the repo root unless noted otherwise:

Standard path:

```bash
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
```

If the OneDrive-backed workspace still throws Windows `os error 5` on target writes, use the temp-target fallback and record it in the phase notes:

```powershell
$target = Join-Path $env:LOCALAPPDATA 'Temp\\veritas-bridge-target-proto006-phase5'
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

Recommended Phase 5-specific checks:

```bash
rg -n "AuthorityBootstrapPlan|assign_from_plan|begin_for_bootstrap_entries" prototype/gbn-bridge-proto
```

```bash
rg -n "chain_id" prototype/gbn-bridge-proto/crates/gbn-bridge-publisher prototype/gbn-bridge-proto/crates/gbn-bridge-runtime
```

```bash
git status --short
```

Expected outcome:

- the Publisher owns and persists the bootstrap session state machine
- seed assignment is delivered to `ExitBridgeB` over the real bridge control path
- bridge-set delivery no longer depends on local plan injection
- remaining bridge fanout is publisher-driven
- timeout and reassignment behavior is covered by tests
- `chain_id` is continuous across the whole bootstrap session
- protected V1 paths show no drift
- minimum V1 regression suite remains green

---

## 11. Acceptance Criteria

Phase 5 is complete when:

- production bootstrap no longer depends on passing `AuthorityBootstrapPlan` through local runtime objects
- the Publisher owns a durable bootstrap session state machine
- seed bridge assignment is delivered over the real bridge control path
- the bridge-set payload is distributed through a real publisher-to-bridge-to-creator path
- remaining bridges are activated by the Publisher, not only by creator-local simulation
- timeout, retry, and reassignment behavior exist and are covered by tests
- `chain_id` is present in bootstrap session state, command delivery, progress reporting, and end-to-end test assertions
- all required V1 and V2 validation commands have been run and recorded

Phase 5 is not complete if:

- `AuthorityBootstrapPlan` is still the production bootstrap transport
- `bootstrap_bridge.rs` still learns assignments only from local plan injection
- remaining bridge fanout is still only local creator bookkeeping
- reassignment behavior is missing or untested
- `chain_id` still drops anywhere between host-creator join and remaining-bridge fanout

---

## 12. Risks And Blockers

| Risk | Why It Matters | Mitigation |
|---|---|---|
| local plan injection remains in the production path | the bootstrap flow would still be simulation-bound | make retirement of `AuthorityBootstrapPlan` transport semantics an explicit acceptance criterion |
| seed bridge and bridge-set delivery are refactored separately | bridge-set retrieval could still depend on stale local state | drive both off one publisher-owned bootstrap session record |
| reassignment is bolted on after the main flow | failure handling would become inconsistent and hard to test | define timeout and reassignment states first in `bootstrap/session.rs` |
| remaining-bridge fanout stays partly creator-driven | the Publisher would still not satisfy the architecture's active distribution responsibility | require publisher-issued fanout commands over control sessions and test them explicitly |
| `chain_id` is added only to logs | trace continuity would still be incomplete | require `chain_id` in storage, command envelopes, progress reports, and tests |
| Phase 5 drifts into payload receiver work | it would blur the boundary with Phase 6 | keep receiver and ACK data path explicitly deferred |

---

## 13. Sign-Off Recommendation

The correct Phase 5 sign-off is:

- bootstrap distribution is now a real publisher-owned distributed workflow
- `ExitBridgeB` learns its assignment from the Publisher over the control plane
- the bridge-set payload is really distributed back to the creator
- remaining bridges are activated by the Publisher over real control sessions
- `chain_id` now survives the full first-contact bootstrap path

The correct Phase 5 sign-off is not:

- payload receiver / ACK completion
- AWS deployment readiness
- a local bootstrap simulation that still depends on `AuthorityBootstrapPlan`
