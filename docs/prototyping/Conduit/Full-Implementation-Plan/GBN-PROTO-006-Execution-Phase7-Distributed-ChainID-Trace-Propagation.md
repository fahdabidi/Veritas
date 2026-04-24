# GBN-PROTO-006 - Execution Phase 7 Detailed Plan: Distributed ChainID Trace Propagation

**Status:** Completed - implemented and validated locally on 2026-04-24
**Primary Goal:** complete and enforce one canonical `chain_id` propagation model across Conduit protocol messages, runtime clients, publisher services, persistence records, logs, metrics, validation scripts, and distributed tests, while preserving the V1 field name `chain_id` and the service boundaries introduced in Phases 1 through 6  
**Source Plan:** [GBN-PROTO-006 Execution Plan](GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md)  
**Protected V1 Baseline:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)  
**Phase 6 Detailed Plan:** [GBN-PROTO-006-Execution-Phase6-Real-Publisher-Receiver-And-ACK-Path](GBN-PROTO-006-Execution-Phase6-Real-Publisher-Receiver-And-ACK-Path.md)  
**Starting Conduit Baseline:** `b44a1713a022ed9e5213831798cc3ee98738b245`
**Validation Outcome:** V2 `cargo fmt --all --check` and `cargo test --workspace` passed, WSL local `mobile-validation.sh --mode local` passed with explicit chain-id evidence, protected V1 path diff stayed clean, and V1 `cargo check --workspace` plus `cargo test -p mcn-router-sim` passed

---

## 1. Current Repo Findings

These findings should drive Phase 7 instead of being rediscovered during implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 7 should record the mainline commit used to begin trace propagation hardening |
| Current HEAD commit | `b44a1713a022ed9e5213831798cc3ee98738b245` | current committed Conduit baseline was the starting point for the completed Phase 7 trace-hardening pass |
| Current protocol trace module | [`trace.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/trace.rs) exists and exports `ChainId`, `CHAIN_ID_FIELD_NAME`, and `validate_chain_id` | the protocol crate now has one centralized trace helper surface |
| Current runtime trace module | [`trace.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/trace.rs) exists and defines canonical generation/import helpers | runtime trace generation and forwarding are now centralized |
| Current publisher trace module | [`trace.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/trace.rs) exists and validates inherited/persisted `chain_id` values | publisher-side trace persistence and matching rules are now centralized |
| Current code search result | `rg -n "chain_id" prototype/gbn-bridge-proto/crates prototype/gbn-bridge-proto/tests prototype/gbn-bridge-proto/infra/scripts prototype/gbn-bridge-proto/docs` returns coverage across protocol, runtime, publisher, tests, scripts, and docs | confirms `chain_id` is now a first-class V2 field instead of an architectural placeholder |
| Current bootstrap protocol types | [`bootstrap.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/bootstrap.rs) now carries `chain_id` on `CreatorJoinRequest`, `CreatorBootstrapResponse`, `BootstrapJoinReply`, `BridgeSetRequest`, `BridgeSetResponse`, and `BridgeSeedAssign` | first-contact and refresh bootstrap messages now preserve one root distributed trace |
| Current punch and progress protocol types | [`punch.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/punch.rs) now carries `chain_id` on `BridgePunchStart`, `BridgePunchProbe`, `BridgePunchAck`, `BootstrapProgress`, `BatchAssignment`, and `BridgeBatchAssign` | bridge command delivery, bootstrap progress, and batched fanout now preserve trace continuity |
| Current data-path protocol types | [`session.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/session.rs) now carries `chain_id` on `BridgeOpen`, `BridgeData`, `BridgeAck`, and `BridgeClose` | receiver and ACK paths now preserve one root trace through payload ingress |
| Current validation artifacts | [`test_chain_id.rs`](../../../prototype/gbn-bridge-proto/tests/integration/test_chain_id.rs) exists under the root integration harness | there is now a dedicated integration test that proves end-to-end chain-id continuity |
| Current trace design doc | [`docs/chain-id-design.md`](../../../prototype/gbn-bridge-proto/docs/chain-id-design.md) exists | the V2 workspace now has one local reference for canonical trace behavior |
| Current AWS/mobile validation output | [`mobile-validation.sh`](../../../prototype/gbn-bridge-proto/infra/scripts/mobile-validation.sh) prints deterministic local chain-id evidence and [`collect-bridge-metrics.sh`](../../../prototype/gbn-bridge-proto/infra/scripts/collect-bridge-metrics.sh) accepts `--chain-id` log filtering | local and AWS/mobile validation artifacts can now preserve or filter by one distributed trace id |

---

## 2. Review Summary

Phase 7 is where Conduit must stop treating `chain_id` as an architectural intention and make it a hard protocol and validation requirement. If this phase is weak, later deployment and validation phases will still produce logs, metrics, and test artifacts that cannot be correlated reliably across creator, bridge, and publisher boundaries.

The main gaps the detailed Phase 7 plan must close are:

| Gap | Why It Matters | Resolution For Phase 7 |
|---|---|---|
| No canonical trace model exists | each phase could add inconsistent ad hoc trace fields | add one canonical `chain_id` type and helper model across protocol, runtime, and publisher crates |
| Core protocol messages still lack `chain_id` | bootstrap, control, data, ACK, and close flows cannot be correlated end-to-end | add `chain_id` to the required message families and test their serialization/compatibility |
| No runtime generation / forwarding rule exists | different entry points could fork traces or mint conflicting ids | define one root-generation/import rule in runtime `trace.rs` |
| No publisher persistence and emission rule exists | stored authority records and emitted logs could still lose trace continuity | define publisher-side persistence, record enrichment, and emission behavior in publisher `trace.rs` |
| No integration tests exist for trace continuity | later phases could regress correlation silently | add dedicated cross-service trace tests and script-output checks |
| No validation script correlation exists | local and AWS/mobile evidence would remain hard to join | update validation scripts and harness outputs to emit and preserve `chain_id` |
| Overreach risk | it is easy to drift into deployment or observability platform work | keep this phase focused on canonical trace propagation, not external telemetry backend rollout |

Phase 7 should make `chain_id` one explicit, enforced, test-covered field across all Conduit paths, but it should not yet try to solve deployment image promotion or the full AWS control plane.

---

## 3. Scope Lock

### In Scope

- add `trace.rs` modules to protocol, runtime, and publisher crates
- define one canonical `chain_id` generation / import / validation model
- add `chain_id` to the required protocol messages across bootstrap, control, data, ACK, close, and progress paths
- propagate `chain_id` through runtime clients and publisher service boundaries
- persist `chain_id` in the required durable authority / receiver records
- add structured logging and metrics enrichment where practical
- add `test_chain_id.rs` integration coverage
- add `docs/chain-id-design.md`
- update local and AWS/mobile validation scripts so outputs preserve and emit `chain_id`

### Out Of Scope

- deployment image promotion
- full AWS control-plane rollout
- external observability backend integration
- modifying `prototype/gbn-proto/**`
- modifying the main repo `README.md`

---

## 4. Preflight Gates

Phase 7 should not begin code edits until all of these are checked:

1. Confirm the Phase 0 inventory deliverables exist.
2. Confirm Phases 1 through 6 are implemented and validated so all major Conduit service boundaries already exist.
3. Confirm protected V1 paths are clean in the local worktree.
4. Confirm Phase 7 will preserve the V1 field name `chain_id` exactly.
5. Confirm no competing root trace field will be introduced.
6. Confirm the trace plan covers protocol messages, persistence, logs, metrics, scripts, and tests.
7. Confirm `README.md` remains out of scope.

If any gate fails, Phase 7 should stop.

Current blocker:

- none after local Phase 7 validation; Phases 1 through 6 are implemented and the Phase 7 trace surface is now live across protocol, persistence, services, tests, and scripts

---

## 5. ChainID Decisions To Lock In Phase 7

### 5.1 Canonical Field Rule

Phase 7 must preserve the V1 field name exactly:

- `chain_id`

Do not introduce competing root fields such as:

- `trace_id`
- `request_id` as the primary distributed trace root
- `correlation_id` as the primary distributed trace root

Auxiliary ids may still exist for specific protocols, but `chain_id` remains the root distributed trace field.

### 5.2 Root Generation And Import Rule

Phase 7 should define one canonical root rule:

- if a creator-originated request already has a trusted `chain_id`, preserve it
- otherwise the creator runtime generates a new root `chain_id`
- host creator must preserve the incoming `chain_id`
- bridges and publisher services must forward the existing `chain_id`, not mint a replacement

Only isolated bridge-local or service-local housekeeping flows with no creator/session lineage may mint a local `chain_id`, and those must still use the same field name.

### 5.3 Protocol Propagation Rule

Phase 7 must add `chain_id` to the protocol messages that participate in one distributed flow.

At minimum this includes:

- `CreatorJoinRequest`
- `CreatorBootstrapResponse`
- `BridgeSetRequest`
- `BridgeSetResponse`
- `BridgePunchStart`
- `BridgePunchProbe`
- `BridgePunchAck`
- `BootstrapProgress`
- `BridgeBatchAssign`
- `BridgeOpen`
- `BridgeData`
- `BridgeAck`
- `BridgeClose`

If a supporting envelope already exists and is the correct place to carry the trace root, that is acceptable, but the end result must still make `chain_id` visible and testable on every correlated path.

### 5.4 Persistence Rule

Phase 7 must persist `chain_id` in every durable record that belongs to a distributed creator/bootstrap/upload flow.

This includes at minimum:

- bootstrap sessions
- bridge command records
- progress reports
- upload sessions
- ingested frame or ACK correlation records where needed for reconstruction

Do not leave `chain_id` as a log-only field. It must survive restarts and postmortem analysis.

### 5.5 Logging And Metrics Rule

Phase 7 should add structured trace emission rules for runtime and publisher code.

Rules:

- logs for creator/bootstrap/upload flows should include `chain_id`
- metrics should include `chain_id` only where cardinality remains safe and practical; otherwise emit it in structured logs or artifacts instead
- validation artifacts and summaries must preserve enough `chain_id` output to correlate multi-hop tests

The point is trace continuity, not unbounded metrics cardinality.

### 5.6 Script And Harness Rule

Phase 7 must update the validation surface so `chain_id` does not disappear when the code leaves Rust.

Required touch points:

- integration tests
- local harness outputs
- AWS/mobile validation scripts
- any snapshot or metrics collection script that reports one distributed flow

At minimum, those scripts must print or persist the `chain_id` values associated with the flow under test.

### 5.7 Backward-Compatibility Rule

Phase 7 should introduce `chain_id` in a way that keeps the Conduit codebase internally coherent during the rollout.

Rules:

- protocol changes should be batched so tests compile consistently
- any temporary compatibility adapter must be local and short-lived
- do not create one half-traced path and leave the other half untyped or implicit

### 5.8 Single-Source Helper Rule

Phase 7 should avoid duplicating trace helper logic.

Recommended split:

- protocol `trace.rs` defines the typed `chain_id` value model and common serialization helpers
- runtime `trace.rs` defines generation/import/forwarding rules
- publisher `trace.rs` defines persistence enrichment and structured emission helpers

That keeps one field name and one value model across the whole Conduit implementation.

---

## 6. Module Ownership To Lock In Phase 7

Phase 7 should keep responsibilities split like this:

| Module | Responsibility |
|---|---|
| `gbn-bridge-protocol/src/trace.rs` | canonical `chain_id` type, parsing, validation, and serialization helpers |
| `gbn-bridge-runtime/src/trace.rs` | root generation/import policy and runtime forwarding helpers |
| `gbn-bridge-publisher/src/trace.rs` | persistence enrichment, structured logging, and receiver / authority trace helpers |
| protocol message files | explicit `chain_id` field addition where the protocol surface must carry the trace root |
| runtime clients and orchestration modules | preserve and forward `chain_id`, not mint competing trace state |
| publisher authority / receiver modules | persist and emit `chain_id` as part of durable and structured outputs |
| `prototype/gbn-bridge-proto/tests/integration/test_chain_id.rs` | end-to-end trace continuity assertions |
| `prototype/gbn-bridge-proto/docs/chain-id-design.md` | one local design reference for the canonical V2 trace model |

Do not let each runtime or publisher module roll its own trace string handling. Keep trace helpers centralized.

---

## 7. Dependency And Implementation Policy

Phase 7 should keep trace work lightweight and explicit.

### Recommended Dependencies

- prefer standard Rust typing and serde support already used in the protocol layer
- add lightweight ID / formatting helpers only if clearly scoped
- reuse the current logging and metrics facilities already present in the V2 workspace

### Bias

- keep `chain_id` typed and explicit
- keep field naming identical to V1
- keep trace propagation testable at protocol, service, and script boundaries
- prefer structured outputs over ad hoc string concatenation

### Avoid In Phase 7

- introducing a second root trace field
- adding a heavyweight observability stack just to carry `chain_id`
- hiding trace propagation only in logs without putting it in protocol or storage
- spreading trace helper logic across unrelated modules
- drifting into deployment promotion or infrastructure rollout

---

## 8. Evidence Capture Requirements

Phase 7 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or commit message |
| starting commit SHA | `git rev-parse HEAD` | phase notes or commit message |
| Phase 1-6 prerequisite status | implementation and validation records | phase notes |
| pre-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |
| canonical trace type and helper model | `protocol/src/trace.rs` and tests | phase notes |
| runtime generation/import rules | `runtime/src/trace.rs` and tests | phase notes |
| publisher persistence / emission rules | `publisher/src/trace.rs` and tests | phase notes |
| protocol message coverage | protocol diffs and serialization tests | phase notes |
| persistence coverage | storage and record tests | phase notes |
| script-output coverage | updated script outputs or fixture samples | phase notes |
| `chain_id` integration evidence | `test_chain_id.rs` and any local harness outputs | phase notes |
| validation command set used | local command log | phase notes |
| temp `--target-dir` workaround, if needed | local command log | phase notes |
| post-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |

Do not sign off Phase 7 with only "chain_id added." Record where the field now lives in protocol, persistence, services, and scripts, and show one end-to-end correlation path.

---

## 9. Recommended Execution Order

Implement Phase 7 in this order:

1. Capture the starting branch, commit SHA, and protected-path diff state.
2. Introduce `gbn-bridge-protocol/src/trace.rs` first and lock the canonical `chain_id` type and helper model.
3. Add `chain_id` to the required protocol message families in `bootstrap.rs`, `punch.rs`, `session.rs`, and any shared envelope types.
4. Introduce `runtime/src/trace.rs` and wire generation/import/forwarding rules into creator, host-creator, bridge, bootstrap, and forwarding paths.
5. Introduce `publisher/src/trace.rs` and wire persistence, logging, receiver, ACK, and progress handling to preserve `chain_id`.
6. Add `docs/chain-id-design.md` so the V2-local trace model is explicitly documented before the script layer is updated.
7. Update integration tests and add `tests/integration/test_chain_id.rs`.
8. Update local and AWS/mobile validation scripts to emit or persist `chain_id` in their outputs.
9. Run the V2 workspace sanity suite.
10. Run the V1 preservation checks and minimum V1 regressions.

This order locks the canonical type first, then the wire model, then the services, and only then the script and evidence layer.

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
$target = Join-Path $env:LOCALAPPDATA 'Temp\\veritas-bridge-target-proto006-phase7'
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

Recommended Phase 7-specific checks:

```bash
rg -n "chain_id" prototype/gbn-bridge-proto/crates/gbn-bridge-protocol prototype/gbn-bridge-proto/crates/gbn-bridge-runtime prototype/gbn-bridge-proto/crates/gbn-bridge-publisher
```

```bash
rg -n "chain_id" prototype/gbn-bridge-proto/infra/scripts prototype/gbn-bridge-proto/tests
```

```bash
git status --short
```

Expected outcome:

- one canonical `chain_id` model exists in the V2 codebase
- required protocol messages carry `chain_id`
- runtime and publisher service paths preserve one root `chain_id`
- persistent records and validation artifacts retain `chain_id`
- integration and script outputs can correlate one full distributed flow by `chain_id`
- protected V1 paths show no drift
- minimum V1 regression suite remains green

---

## 11. Acceptance Criteria

Phase 7 is complete when:

- the V2 codebase has one canonical `chain_id` model and helper path
- required protocol message families carry `chain_id`
- runtime and publisher code preserve `chain_id` end-to-end across bootstrap, progress, receiver, ACK, and close/error paths
- durable records persist `chain_id` where required for distributed flow reconstruction
- integration tests prove end-to-end `chain_id` continuity
- local and AWS/mobile validation outputs preserve `chain_id`
- all required V1 and V2 validation commands have been run and recorded

Phase 7 is not complete if:

- `chain_id` still exists only as an architectural concept or log convention
- key protocol message families still lack the field
- a competing root trace field is introduced
- script outputs and validation artifacts still drop the trace id

---

## 12. Risks And Blockers

| Risk | Why It Matters | Mitigation |
|---|---|---|
| trace fields are added inconsistently across message families | distributed correlation would still fail on some paths | define the canonical field set first in `protocol/src/trace.rs` and update message families in one batch |
| runtime or publisher mint competing ids | one distributed flow could fork into multiple roots | enforce one root-generation/import rule and test it explicitly |
| `chain_id` is persisted only partially | restart or postmortem analysis would still lose trace continuity | make persistence coverage an acceptance criterion |
| scripts are left behind after code changes | local and AWS/mobile evidence would remain hard to correlate | require script-output updates in this phase, not later |
| a heavyweight telemetry stack is introduced prematurely | the phase would sprawl and slow implementation | keep this phase focused on canonical field propagation and evidence output |
| Phase 7 drifts into deployment or validation rollout | it would blur the boundary with later phases | keep deployment promotion and live stack concerns deferred |

---

## 13. Sign-Off Recommendation

The correct Phase 7 sign-off is:

- Conduit now has one canonical `chain_id` model
- protocol, runtime, publisher, persistence, and validation surfaces all preserve the same root `chain_id`
- one distributed flow can be correlated end-to-end through code, records, and artifacts

The correct Phase 7 sign-off is not:

- only adding `chain_id` to a subset of messages
- only logging `chain_id` without persisting it
- introducing a second competing trace root
