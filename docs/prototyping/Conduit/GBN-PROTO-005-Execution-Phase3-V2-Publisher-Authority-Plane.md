# GBN-PROTO-005 - Execution Phase 3 Detailed Plan: Publisher Authority Plane

**Status:** Completed from the committed Phase 2 protocol baseline
**Primary Goal:** implement the Conduit publisher authority service in `gbn-bridge-publisher` without mutating the frozen V1 Lattice publisher or prematurely pulling runtime / deployment concerns into the authority plane
**Source Plan:** [GBN-PROTO-005 Execution Plan](GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign-Execution-Plan.md)
**Phase 0 Baseline Release:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)
**Phase 2 Protocol Baseline:** [GBN-ARCH-002-Bridge-Protocol-V2](../architecture/GBN-ARCH-002-Bridge-Protocol-V2.md)

---

## 1. Current Repo Findings

These findings should drive Phase 3 execution instead of being rediscovered during implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 3 should record the mainline commit used to begin authority-plane work |
| Current HEAD commit | `baa17ea8c35a444ca6665de424d489883559470a` | this is the current committed Conduit baseline that includes the Phase 3 authority plane and the Phase 4 bridge runtime |
| Phase 0 baseline release | `veritas-lattice-0.1.0-baseline` published | the V1 Lattice publisher remains the preservation reference point |
| Existing V2 workspace path | `prototype/gbn-bridge-proto/` exists | Phase 3 must build entirely inside the isolated Conduit workspace |
| Current protocol crate state | canonical M1 Conduit wire model is committed in `gbn-bridge-protocol` and documented in `GBN-ARCH-002` | authority logic now has a fixed protocol surface and should not rename or reshape protocol types casually |
| Current publisher crate state | authority-plane modules and `tests/authority_flow.rs` are committed in `gbn-bridge-publisher` | Phase 3 is now the committed authority baseline for later Conduit phases |
| Current CLI state | `gbn-bridge-cli/src/main.rs` still prints the Phase 1 scaffold message | Phase 3 may only make narrow V2-local CLI adjustments if they help authority tests; it should not turn Phase 3 into a CLI phase |
| Current protected V1 path drift | none | V1 preservation is still a hard sign-off gate |
| Current validation environment risk | OneDrive-backed V2 `target/` writes still fail with Windows `os error 5` during `cargo test --workspace` | Phase 3 validation must record the temp `--target-dir` workaround as the executed test path |

---

## 2. Review Summary

Phase 3 is the first Conduit phase with real authority behavior. It is also the easiest place to accidentally overbuild.

The main execution plan is directionally correct, but a robust Phase 3 implementation needs stronger constraints:

| Gap | Why It Matters | Resolution For Phase 3 |
|---|---|---|
| Service-boundary ambiguity | `server.rs`, `authority.rs`, and `storage.rs` can collapse into one large file or an accidental networking stack | treat `authority.rs` as the orchestration surface, `storage.rs` as a backend abstraction, and `server.rs` as an in-process façade only |
| Persistence overreach | adding durable storage now would slow the phase and obscure the core authority logic | use in-memory authority state in Phase 3 and defer durable persistence |
| Bootstrap policy drift | seed-bridge and initial bridge-set rules can drift from the architecture docs if not pinned | lock direct-only seed-bridge and initial bootstrap eligibility now |
| Catalog trust ambiguity | descriptor signing and catalog signing can diverge if not applied deterministically | sign descriptors and the catalog container through one authority path |
| Batch semantics underdefined | the 10-request / 11th-request rollover is easy to hand-wave and easy to get wrong | model the 0.5 second window and rollover rules explicitly in batching tests |
| Premature runtime coupling | bridges, creators, or deployment code could leak into the authority phase | keep Phase 3 focused on authority decisions, signed outputs, and deterministic tests |

---

## 3. Scope Lock

### In Scope

- implement the V2 publisher authority service in `gbn-bridge-publisher`
- implement in-memory bridge registration, lease, liveness, and revocation state
- implement signed catalog issuance
- implement first-contact bootstrap orchestration using the committed Phase 2 protocol types
- implement short-window batch assignment for new-creator onboarding
- implement direct-vs-non-direct policy hooks sufficient for Phase 3 deliverables
- add authority-flow tests covering success, rejection, expiry, signing, seed selection, and batch rollover

### Out Of Scope

- real network sockets, HTTP services, or production RPC bindings
- durable storage backends
- runtime bridge behavior
- creator runtime behavior
- upload ingest / ACK data-path logic
- AWS deployment assets
- modifying V1 publisher binaries, ports, env vars, or CLI flags
- updating the main repo `README.md`

---

## 4. Preflight Gates

Phase 3 should not begin code edits until all of these are checked:

1. Confirm Phase 2 is committed and the protocol crate exports the canonical M1 Conduit types.
2. Confirm protected V1 paths are clean in the local worktree.
3. Confirm Phase 3 stays inside `gbn-bridge-publisher` plus narrowly related V2-local tests and optional CLI glue.
4. Confirm direct-only eligibility remains the Phase 3 rule for seed-bridge selection and initial first-contact bridge sets.
5. Confirm Phase 3 will use in-memory authority state rather than introducing durable persistence.
6. Decide and record the Phase 3 validation command shape if the default V2 `target/` path remains blocked by OneDrive.
7. Confirm Phase 3 will not modify the main repo `README.md`; any Conduit README rewrite remains deferred until V2 code work is complete.

If any gate fails, Phase 3 should stop. The point here is to implement the authority plane cleanly, not to blend it with later runtime or deployment concerns.

Current blocker:

- none; Phase 3 is complete

---

## 5. Authority Decisions To Lock In Phase 3

### 5.1 Authority Service Boundary

Phase 3 should keep the publisher authority surface structured like this:

| Module | Responsibility |
|---|---|
| `authority.rs` | top-level orchestration API for registration, heartbeats, catalog issuance, bootstrap issuance, and batch assignment |
| `registry.rs` | active bridge records, liveness tracking, revocation state, and lookup indexes |
| `lease.rs` | lease issuance, renewal, expiry checks, and revoke helpers |
| `catalog.rs` | descriptor assembly, filtering, deterministic ordering, and signed catalog issuance |
| `bootstrap.rs` | creator-join acceptance, seed-bridge selection, bootstrap response construction, and bridge-set issuance |
| `batching.rs` | 0.5 second batch-window logic, 10-request packing, and 11th-request rollover |
| `punch.rs` | conversion from authority decisions into signed `BridgePunchStart` / `BridgeBatchAssign` outputs |
| `storage.rs` | in-memory backend abstraction and state containers only |
| `metrics.rs` | counters / snapshots / instrumentation surfaces with no external metrics backend required |
| `server.rs` | in-process façade or adapter layer only; not a production socket server yet |

### 5.2 State Model

Phase 3 should treat authority state as an in-memory model with explicit records for:

- registered bridge identity
- current signed lease
- last heartbeat timestamp
- reachability class
- revocation state
- bootstrap-session tracking
- batch-window tracking

Durable persistence is deferred. The state model should be clean enough that a persistence backend can be added later without rewriting the authority policy.

### 5.3 Registration And Lease Rules

Phase 3 should lock these behaviors:

- reject invalid registrations rather than auto-correcting them
- require at least one ingress endpoint
- require non-zero UDP punch port input or default-assignment policy at issuance time
- keep issued lease metadata authoritative over requested metadata
- heartbeat must extend liveness for an active lease without mutating bridge identity
- revoked or expired bridges must stop appearing in catalogs

### 5.4 Catalog Rules

Catalog generation in Phase 3 should:

- emit only active, unexpired, non-revoked bridges
- preserve the canonical M1 descriptor field set from Phase 2
- sign each bridge descriptor and the catalog response
- expose deterministic ordering so tests do not depend on map iteration order
- support direct-only filtering without implementing the full later reachability scoring phase

### 5.5 Bootstrap Rules

For first-contact bootstrap, Phase 3 should enforce:

- only `direct` bridges are eligible for seed-bridge selection
- the selected seed bridge should differ from the HostCreator relay bridge when possible
- the initial bridge set should contain up to 9 active `direct` bridges
- bootstrap payloads should be Publisher-signed and expiry-bound
- bootstrap entries remain hints under Publisher authority, not independent trust objects

### 5.6 Batch Rules

The batching model should be explicit in Phase 3:

- one batch window = `500ms`
- max `10` join requests per active batch
- the `11th` request rolls into the next batch immediately
- a batch assignment should be representable as one signed Publisher decision
- stalled bootstrap reassignment can remain a policy hook, but the state needed to support it should exist

### 5.7 Policy Hooks

Phase 3 should include only the minimum policy hooks needed to keep later phases clean:

- direct-only bridge filtering for first-contact bootstrap
- a place to plug in richer bridge scoring later
- a place to downgrade or exclude bridges without rewriting the authority API

Do not overbuild the full Phase 8 reachability policy here.

---

## 6. Dependency And Implementation Policy

Phase 3 should keep dependencies minimal and bias toward deterministic tests.

### Required Bias

- depend on `gbn-bridge-protocol` as the sole protocol surface
- prefer `std` collections and in-memory state unless a dependency is clearly justified
- structure time-sensitive behavior so tests can inject timestamps instead of sleeping
- keep authority logic callable in-process from tests

### Avoid In Phase 3

- adding async/network runtime dependencies unless they are strictly needed for compilation boundaries
- adding HTTP or socket frameworks
- adding persistent database dependencies
- embedding deployment or cloud assumptions
- mutating protocol types in `gbn-bridge-protocol` unless a true Phase 2 defect is discovered and documented

---

## 7. Evidence Capture Requirements

Phase 3 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or commit message |
| starting commit SHA | `git rev-parse HEAD` | phase notes or commit message |
| pre-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |
| authority module ownership used | Phase 3 plan + committed files | phase notes |
| direct-only bootstrap rule used | Phase 3 plan + tests | phase notes |
| batch window / rollover rule used | Phase 3 plan + tests | phase notes |
| validation command set used | local command log | phase notes |
| temp `--target-dir` workaround, if needed | local command log | phase notes |
| post-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |

Do not sign off Phase 3 with only "authority crate added." Record the concrete policy assumptions used.

---

## 8. Recommended Execution Order

Implement Phase 3 in this order:

1. Capture the starting branch, commit SHA, and protected-path diff state.
2. Replace the publisher placeholder boundary with module declarations in `lib.rs`.
3. Implement `storage.rs` and `registry.rs` first so later modules share one state model.
4. Implement `lease.rs` and `catalog.rs`.
5. Implement `bootstrap.rs` and `batching.rs`.
6. Implement `punch.rs`, `metrics.rs`, and `server.rs`.
7. Implement `authority.rs` as the orchestration layer over the supporting modules.
8. Add `tests/authority_flow.rs`.
9. Run the V2 validation commands.
10. Run the V1 preservation checks and minimum V1 regressions.

This keeps the mutable state model and signed outputs stable before orchestration code tries to compose them.

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
$target = Join-Path $env:LOCALAPPDATA 'Temp\veritas-bridge-target-phase3'
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

- authority crate builds and tests
- registration validation and lease issuance are covered
- expired or revoked bridges do not appear in catalogs
- bootstrap issuance and batch rollover tests pass
- protected V1 diff remains empty
- minimum V1 regression suite still passes

### 9.1 Executed Phase 3 Validation Results

Phase 3 was validated with the following concrete results:

- `cd prototype/gbn-bridge-proto && cargo fmt --check` passed
- `cd prototype/gbn-bridge-proto && cargo check --workspace` passed
- `cargo test --workspace` in the default V2 workspace target failed with Windows `os error 5` under the OneDrive-backed `target/` directory
- the documented fallback command using `%LOCALAPPDATA%\Temp\veritas-bridge-target-phase3` passed
- the V2 suite now includes:
  - Phase 2 protocol round-trip and signature tests
  - Phase 3 authority-flow tests for registration, rejection, expiry, catalog signing, bootstrap selection, and batch rollover
- protected V1 path diff remained empty after Phase 3 validation
- `cd prototype/gbn-proto && cargo check --workspace` passed
- `cd prototype/gbn-proto && cargo test -p mcn-router-sim` passed

---

## 10. Acceptance Criteria

Phase 3 is complete only when all of the following are true:

- every file listed in the main execution plan exists
- the publisher crate no longer exports only `PublisherPlaceholder`
- bridge registration validation is implemented
- signed lease issuance and heartbeat-driven liveness tracking are implemented
- signed catalog generation is implemented
- seed-bridge selection and publisher-seeded bootstrap payload issuance are implemented
- batch assignment logic covers the 10-request window and 11th-request rollover
- direct-vs-non-direct policy hooks exist without dragging in the later full reachability phase
- authority-flow tests cover success, rejection, expiry, signing, seed selection, and batching
- protected V1 diff is clean after validation
- minimum V1 regression suite still passes

---

## 11. Risks And Blockers

| Risk | What It Looks Like | Mitigation |
|---|---|---|
| Policy drift | seed-bridge or bridge-set rules stop matching the architecture docs | lock direct-only bootstrap policy in tests and notes |
| Overbuilt server layer | Phase 3 becomes a networking phase instead of an authority phase | keep `server.rs` in-process and thin |
| State sprawl | registry, storage, and authority duplicate state | define one in-memory state model first |
| Catalog instability | descriptor ordering varies by hash-map iteration | enforce deterministic ordering before signing |
| Batch edge-case bugs | the 11th join request is dropped, merged incorrectly, or never assigned | add explicit rollover tests |
| OneDrive validation noise | test failures look like code failures when they are really filesystem failures | use and document the temp-target fallback |
| V1 leakage | `mpub-receiver` or V1 CLI gets edited for convenience | keep all authority behavior V2-local and check protected-path diff before sign-off |

Current blocker:

- none; Phase 3 is complete

---

## 12. First Implementation Cut

If Phase 3 is implemented as a single focused change set, use this breakdown:

1. State model and module scaffolding
2. Lease and catalog issuance
3. Bootstrap and batch orchestration
4. Authority façade and tests

That keeps Phase 3 auditable and gives Phase 4 a stable authority plane instead of a moving publisher contract.
