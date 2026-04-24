# GBN-PROTO-005 - Execution Phase 2 Detailed Plan: Lock The V2 Wire Model

**Status:** Completed and validated from the committed Phase 2 protocol baseline
**Primary Goal:** implement the canonical M1 Conduit protocol schema set in `gbn-bridge-protocol` without introducing runtime behavior or mutating the V1 Lattice codebase
**Source Plan:** [GBN-PROTO-005 Execution Plan](GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign-Execution-Plan.md)
**Phase 0 Baseline Release:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)
**Phase 1 Boundary Plan:** [GBN-PROTO-005-Execution-Phase1-V2-Workspace-Boundary](GBN-PROTO-005-Execution-Phase1-V2-Workspace-Boundary.md)

---

## 1. Current Repo Findings

These findings should record the actual current Phase 2 state instead of forcing it to be rediscovered during review:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 2 was implemented and committed on the mainline branch |
| Current HEAD commit | `e58b1bf130807c57289b92f014b2d35ca43ca5ce` | current repo tip after the committed Phase 2 wire-model change set |
| Phase 0 baseline release | `veritas-lattice-0.1.0-baseline` published | V2 protocol work must continue to preserve the frozen V1 baseline |
| Existing V2 workspace path | `prototype/gbn-bridge-proto/` exists | Phase 2 should build inside the sibling workspace created in Phase 1 |
| Current protocol crate state | Phase 2 wire-model modules, tests, and `GBN-ARCH-002` are committed | the protocol placeholder has been replaced with the canonical M1 Conduit wire model |
| Canonical M1 descriptor decision | locked in the main execution plan | Phase 2 should not improvise additional descriptor fields |
| Current protected V1 path drift | none | V1 preservation remained clean through the Phase 2 validation pass |
| Current validation environment risk | Cargo writes inside the OneDrive-backed V2 workspace may fail with `os error 5` | validation commands may need a temp `--target-dir` workaround to distinguish code issues from environment issues |

---

## 2. Review Summary

Phase 2 is the first point where Conduit stops being only a workspace boundary and starts becoming a real protocol. That makes it the first high-churn phase if the model is not pinned down early.

The main execution plan now covers the minimum scope, but a robust implementation still needs stronger guardrails:

| Gap | Why It Matters | Resolution For Phase 2 |
|---|---|---|
| Descriptor sprawl risk | adding optional fields now would cause avoidable schema churn later | lock the M1 descriptor to the minimal canonical field set and explicitly defer extras |
| Message ownership ambiguity | types will drift if everything lands in `messages.rs` | assign each message family to a specific module now |
| Encoding overcommitment risk | choosing final framing too early can distort the schema layer | keep Phase 2 focused on schema shape, serde compatibility, signing, and semantics; defer transport framing details |
| Underdefined trust semantics | signatures, expiry, and replay-prevention rules can diverge between crates later | define those rules once in the protocol layer |
| Validation fragility in OneDrive | Cargo target writes can fail even for valid code | document the temp-target workaround as an accepted validation fallback |

---

## 3. Scope Lock

### In Scope

- implement the canonical M1 `BridgeDescriptor`
- implement publisher-seeded bootstrap entry types
- implement the full Phase 2 message surface for registration, lease, catalog, bootstrap, punch, and bridge-session traffic
- implement shared versioning, signature, expiry, and replay-prevention semantics in the protocol crate
- add protocol round-trip and negative tests
- document the schema set in `docs/architecture/GBN-ARCH-002-Bridge-Protocol-V2.md`

### Out Of Scope

- runtime network loops
- publisher authority business logic
- bridge registration handling or heartbeat timers
- upload scheduling or chunk fanout logic
- AWS or Docker assets
- modifying V1 protocol, DHT, or onion schemas
- committing to a final production wire encoding or framing transport if the architecture docs do not already require one

---

## 4. Preflight Gates

These were the required preflight gates before Phase 2 code edits began:

1. Confirm Phase 1 is committed and the `prototype/gbn-bridge-proto/` workspace exists.
2. Confirm the protected V1 path diff is clean in the local worktree.
3. Confirm the canonical M1 `BridgeDescriptor` field set is accepted as written in the main execution plan.
4. Confirm `BridgeRefreshHint` is treated as part of the Phase 2 message surface.
5. Confirm this phase will not introduce runtime behavior into `gbn-bridge-runtime`, `gbn-bridge-publisher`, or `gbn-bridge-cli`.
6. Decide and record the Phase 2 validation command shape if the default `target/` path remains blocked by OneDrive.
7. Confirm the Phase 2 output remains V2-local and does not touch `prototype/gbn-proto/crates/gbn-protocol/**`.

If any gate fails, Phase 2 should stop. The point here is to freeze the wire model once, not to iterate it blindly under implementation pressure.

Current blocker:

- none; Phase 2 is complete

### 4.1 Execution Result

Phase 2 replaced the protocol placeholder crate boundary with the canonical M1 Conduit wire model.

Implemented and committed:

- `descriptor.rs` with canonical `BridgeDescriptor`, `ReachabilityClass`, and bridge capabilities
- `bootstrap.rs` with publisher-seeded bootstrap entries and creator bootstrap responses
- `catalog.rs` with signed catalog responses and `BridgeRefreshHint`
- `lease.rs` with register, lease, heartbeat, and revoke messages
- `punch.rs` with punch, progress, and batch-assignment messages
- `session.rs` with bridge open/data/ack/close messages
- `messages.rs` with protocol versioning, envelope, and replay metadata
- `signing.rs` and `error.rs`
- `tests/protocol_roundtrip.rs`
- `docs/architecture/GBN-ARCH-002-Bridge-Protocol-V2.md`

Validated results:

- `cargo fmt --check` passed in `prototype/gbn-bridge-proto`
- `cargo check --workspace` passed in `prototype/gbn-bridge-proto`
- default `cargo test --workspace` failed only because the OneDrive-backed workspace denied writes with Windows `os error 5`
- `cargo test --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml --target-dir $env:LOCALAPPDATA\\Temp\\veritas-bridge-target-phase2` passed
- protected V1 path diff stayed empty
- `cargo check --workspace` passed in `prototype/gbn-proto`
- `cargo test -p mcn-router-sim` passed in `prototype/gbn-proto`

---

## 5. Canonical Wire Decisions To Lock In Phase 2

### 5.1 Canonical `BridgeDescriptor` For M1

Phase 2 should implement exactly this required field set:

- `bridge_id`
- `identity_pub`
- `ingress_endpoints[]`
- `udp_punch_port`
- `reachability_class`
- `lease_expiry_ms`
- `capabilities[]`
- `publisher_sig`

Do not add these in Phase 2 unless explicitly approved:

- `network_type`
- `geo_tag`
- `observed_reliability_score`

These are not rejected forever. They are deferred so the first real protocol surface stays small and testable.

### 5.2 Canonical Bootstrap Entry For M1

Publisher-seeded bootstrap entries should carry at least:

- `node_id` / `iid`
- `ip_addr`
- `pub_key`
- `udp_punch_port`
- `entry_expiry_ms`
- `publisher_sig`

Bootstrap entries are still hints plus signed authority, not free-floating DHT trust objects.

### 5.3 Reachability And Eligibility Rules

The protocol layer should encode:

- `direct`
- `brokered`
- `relay_only`

Only `direct` is creator-ingress eligible for the Phase 2 assumptions. `brokered` and `relay_only` must remain representable, but not silently treated as creator-ready.

### 5.4 Versioning Rule

The wire model should define an explicit protocol version surface in Phase 2. That version must be testable and reject unsupported versions instead of silently accepting them.

### 5.5 Signature Rule

Every object treated as authoritative by a creator should be representable as Publisher-signed:

- bridge descriptors
- catalog responses or signed catalog payloads
- bootstrap entries
- bootstrap bridge sets

The protocol crate should own the verification contract even if later phases own the actual trust-root loading and service logic.

### 5.6 Expiry And Replay Rule

Phase 2 should define:

- explicit expiry fields for leases and bootstrap entries
- nonce, request ID, or comparable replay-prevention material where a replay would change authority-plane behavior
- tests that prove stale or tampered authority objects are rejected

If a message can trigger transport state, bootstrap progress, or bridge assignment, it should not be replay-safe by accident.

---

## 6. Module Boundary Decisions To Lock In Phase 2

Do not use `messages.rs` as a dumping ground. Use these ownership rules:

| Module | Owns |
|---|---|
| `descriptor.rs` | `BridgeDescriptor`, ingress endpoint types, `ReachabilityClass`, capability enums or sets |
| `bootstrap.rs` | `BootstrapDhtEntry`, `CreatorJoinRequest`, `CreatorBootstrapResponse`, `BridgeSetRequest`, `BridgeSetResponse` |
| `catalog.rs` | `BridgeCatalogRequest`, `BridgeCatalogResponse`, `BridgeRefreshHint`, signed catalog payload/container types |
| `lease.rs` | `BridgeRegister`, `BridgeLease`, `BridgeHeartbeat`, `BridgeRevoke`, lease-state primitives |
| `punch.rs` | `BridgePunchStart`, `BridgePunchProbe`, `BridgePunchAck`, `BootstrapProgress`, `BridgeBatchAssign` |
| `session.rs` | `BridgeOpen`, `BridgeData`, `BridgeAck`, `BridgeClose` |
| `signing.rs` | shared signing and verification helpers, signed wrapper traits or envelope types |
| `error.rs` | protocol-local error types |
| `messages.rs` | only shared envelope glue if still needed after the message families are split cleanly |

The implementation should bias toward small modules with obvious ownership.

---

## 7. Dependency And Serialization Policy

Phase 2 should keep dependencies minimal and deliberate.

### Required Bias

- add only the dependencies required to model, serialize, and validate the protocol layer
- prefer `serde`-friendly types and deterministic round-trip tests
- keep cryptographic verification helpers focused on signature binding, not on service-side key management

### Avoid In Phase 2

- adding async runtime dependencies only for the schema crate
- adding network stack dependencies to the protocol crate
- embedding runtime policy decisions into the protocol types
- choosing a permanent transport framing story if the schema layer can remain encoding-agnostic

If an encoding decision is needed for tests, make it explicit that the tests validate schema stability, not final wire transport.

---

## 8. Evidence Capture Requirements

Phase 2 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or commit message |
| starting commit SHA | `git rev-parse HEAD` | phase notes or commit message |
| pre-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |
| canonical descriptor field set used | Phase 2 plan + execution plan | phase notes or architecture doc |
| deferred descriptor fields | Phase 2 plan + architecture notes | architecture doc and phase notes |
| exact message inventory implemented | protocol modules and tests | phase notes |
| validation command set used | local command log | phase notes |
| temp `--target-dir` workaround, if needed | local command log | phase notes |
| post-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |

Do not sign off Phase 2 with only "protocol types added." Record which fields were intentionally excluded as well as which ones were implemented.

---

## 9. Recommended Execution Order

Implement Phase 2 in this order:

1. Capture the starting branch, commit SHA, and protected-path diff state.
2. Replace the protocol placeholder boundary with module declarations in `lib.rs`.
3. Implement `descriptor.rs` and `bootstrap.rs` first, because they anchor most later message families.
4. Implement `catalog.rs` and `lease.rs`.
5. Implement `punch.rs` and `session.rs`.
6. Implement `signing.rs` and `error.rs`.
7. Add protocol round-trip and negative tests.
8. Draft `GBN-ARCH-002-Bridge-Protocol-V2.md` to describe the canonical model that was actually implemented.
9. Run V2 validation commands.
10. Run V1 preservation checks and minimum V1 regressions.

This keeps the hard-to-change trust objects and authority messages ahead of the more mechanical session types.

---

## 10. Validation Commands

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
$target = Join-Path $env:LOCALAPPDATA 'Temp\veritas-bridge-target'
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

Before sign-off, also verify:

```bash
git status --short
```

Expected outcome:

- the protocol crate builds and tests
- the canonical descriptor field set is present
- `BridgeRefreshHint` is represented in the protocol surface
- the protected V1 diff is empty
- the minimum V1 regression suite still passes

---

## 11. Acceptance Criteria

Phase 2 is complete only when all of the following are true:

- every file listed in the main execution plan exists
- the protocol crate no longer exports only a placeholder boundary
- the canonical M1 `BridgeDescriptor` field set is implemented exactly
- `network_type`, `geo_tag`, and `observed_reliability_score` remain deferred unless separately approved
- publisher-seeded bootstrap entries are implemented
- registration, lease, catalog, bootstrap, punch, and session message families are implemented
- `BridgeRefreshHint` is part of the creator refresh message surface
- signing, expiry, versioning, and replay-prevention semantics are represented and tested
- protocol round-trip and negative tests pass
- the protected-path diff is clean after validation
- the minimum V1 regression suite still passes

---

## 12. Risks And Blockers

| Risk | What It Looks Like | Mitigation |
|---|---|---|
| Descriptor creep | optional metadata fields get added because they seem useful during coding | freeze the M1 field set and defer extras explicitly |
| Message churn | names or ownership shift across files after runtime work starts | lock module ownership before implementation |
| Overcommitted encoding | schema choices get distorted by an early transport/framing guess | keep Phase 2 schema-focused and defer transport specifics when possible |
| Weak trust semantics | signatures and expiries exist informally but not canonically | make the protocol crate own the authority-object contract |
| Replay blind spots | authority or punch messages can be replayed without detection | model replay-prevention fields and negative tests in Phase 2 |
| Validation noise from OneDrive | `cargo test` fails with `os error 5` despite correct code | use and document the temp-target fallback |
| V1 leakage | implementation borrows or mutates V1 types directly | create V2-native types and keep the no-touch diff clean |

Current blocker:

- none at the design or implementation level; the only recurring environment issue is local Cargo target write-denial in the OneDrive-backed workspace

---

## 13. First Implementation Cut

If Phase 2 is implemented as a single focused change set, use this breakdown:

1. Canonical trust objects
2. Authority and bootstrap message families
3. Punch and session message families
4. Signing / version / expiry / replay semantics
5. Round-trip and negative tests
6. `GBN-ARCH-002-Bridge-Protocol-V2.md`

That keeps Phase 2 auditable and gives Phase 3 a stable protocol surface instead of a moving schema target.
