# GBN-PROTO-005 - Execution Phase 6 Detailed Plan: Bridge-Mode Data Path

**Status:** Completed from the committed Phase 5 creator/bootstrap baseline
**Primary Goal:** implement the Conduit encrypted upload path across creator, ExitBridge, and publisher authority surfaces without mutating the frozen V1 Lattice data path or letting Phase 6 bleed into later discovery, reachability-policy, or deployment phases
**Source Plan:** [GBN-PROTO-005 Execution Plan](GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign-Execution-Plan.md)
**Phase 0 Baseline Release:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)
**Protocol Baseline:** [GBN-ARCH-002-Bridge-Protocol-V2](../architecture/GBN-ARCH-002-Bridge-Protocol-V2.md)

---

## 1. Current Repo Findings

These findings should drive Phase 6 execution instead of being rediscovered during implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 6 notes should capture the mainline commit used to begin bridge-mode data-path work |
| Current HEAD commit | `8611ac586efa52f8b3d373eb727257e4aa32fa9f` | this is the committed Phase 5 creator/bootstrap baseline that Phase 6 now builds on |
| Phase 0 baseline release | `veritas-lattice-0.1.0-baseline` published | the V1 Lattice upload and onion data path remain the preservation reference point |
| Current protocol crate state | `BridgeOpen`, `BridgeData`, `BridgeAck`, and `BridgeClose` are already committed in `gbn-bridge-protocol` | Phase 6 should consume these as fixed wire inputs rather than redesign the Phase 2 message set |
| Current publisher/runtime state | Phase 3 publisher authority, Phase 4 ExitBridge runtime, and Phase 5 creator bootstrap are committed | Phase 6 can now focus on upload/session behavior instead of bootstrap scaffolding |
| Current runtime crate state | `gbn-bridge-runtime` already contains creator selection, local DHT, fanout initiation, bridge lease/heartbeat, and opaque frame forwarder helpers | Phase 6 should extend the committed runtime surface instead of bypassing it with ad hoc upload code |
| Current publisher crate state | the authority plane now owns an in-memory creator upload ingest / ACK pipeline built around `BridgeOpen`, `BridgeData`, `BridgeAck`, and `BridgeClose` | Phase 6 added the Conduit receive / dedupe / ACK path without mutating the frozen V1 publisher surface |
| Current CLI state | creator/host-creator/exit-bridge entrypoint placeholders exist | Phase 6 may keep CLI work minimal; the center of gravity is runtime and test logic |
| Current protected V1 path drift | none | V1 preservation remains a hard sign-off gate |
| Current validation environment risk | OneDrive-backed V2 `target/` writes still fail with Windows `os error 5` during `cargo test --workspace` | Phase 6 validation should expect the temp `--target-dir` fallback again |

---

## 2. Review Summary

Phase 6 is the first Conduit phase that actually moves creator payload over bridge links instead of only arranging who should talk to whom.

The master plan is directionally correct, but a robust Phase 6 implementation needs tighter boundaries:

| Gap | Why It Matters | Resolution For Phase 6 |
|---|---|---|
| Encryption-boundary ambiguity | upload code can accidentally let a bridge inspect or transform publisher-encrypted payload | keep bridge handling opaque; only creator wraps and publisher unwraps / validates |
| Session-state sprawl | session open, chunk fanout, ACK correlation, and retry can collapse into one large runtime file | split session, bridge-pool, scheduler, framing, ACK tracking, and sender responsibilities early |
| Fanout policy underdefined | “up to 10 bridges” and reuse-after-timeout can easily become ad hoc behavior | lock deterministic scheduler rules for initial active set, timeout reuse, and failover promotion |
| ACK ambiguity | bridge or publisher ACKs can be misrouted without explicit session/frame correlation rules | require ACK tracking by session ID + frame ID + sequence and test duplicates/replays |
| Ingest overreach | publisher ingest can turn into a persistence or delivery-confirmation subsystem | keep Phase 6 ingest in-memory and focused on receive, dedupe, and ACK only |
| Phase 7/8 bleed | discovery hints and richer reachability policy can distract from raw data-path mechanics | treat the current Phase 5 direct-bridge set as the only transport candidate source for Phase 6 |

---

## 3. Scope Lock

### In Scope

- implement creator-side session establishment using committed `BridgeOpen`, `BridgeData`, `BridgeAck`, and `BridgeClose`
- implement creator-side payload framing / chunk fanout across up to 10 active bridges
- implement deterministic bridge-pool selection from the already-active Phase 5 bridge set
- implement ACK correlation and retransmission / failover behavior
- implement bridge-side opaque forwarder handling for upload frames
- implement publisher-side in-memory receive, dedupe, validation, and ACK response flow
- implement bridge reuse when fewer than 10 bridges are active before timeout
- add data-path tests for end-to-end upload, ACK routing, duplicate handling, failover, confidentiality, and bridge reuse

### Out Of Scope

- discovery / weak discovery integration
- richer reachability policy beyond the committed Phase 5 direct-only assumptions
- AWS deployment assets
- durable publisher storage
- production transport sockets or HTTP services
- V1 upload-path edits
- updating the main repo `README.md`

---

## 4. Preflight Gates

Phase 6 should not begin code edits until all of these are checked:

1. Confirm the committed Phase 5 creator/bootstrap baseline is present and clean.
2. Confirm protected V1 paths are clean in the local worktree.
3. Confirm the protocol crate exports the committed session and ACK wire types unchanged.
4. Confirm Phase 6 stays inside `gbn-bridge-runtime`, `gbn-bridge-publisher`, V2-local tests, and any minimal V2-local CLI glue.
5. Confirm bridges remain opaque payload relays and do not inspect publisher-encrypted creator payload.
6. Confirm Phase 6 uses only the committed Phase 5 active-bridge / bootstrap state as its transport candidate source.
7. Confirm the Phase 6 validation command shape if the default V2 `target/` path remains blocked by OneDrive.
8. Confirm Phase 6 will not modify the main repo `README.md`; any Conduit README rewrite remains deferred until V2 code work is complete.

Current blocker:

- none; Phase 6 implementation is complete locally and the validation gates passed

---

## 5. Data-Path Decisions To Lock In Phase 6

### 5.1 Runtime Module Boundary

Phase 6 should keep the bridge-mode upload surface structured like this:

| Module | Responsibility |
|---|---|
| `session.rs` | creator-side session lifecycle, session IDs, open/close rules, and session state transitions |
| `bridge_pool.rs` | active-bridge inventory and deterministic selection / promotion rules |
| `fanout_scheduler.rs` | initial frame-to-bridge placement, timeout reuse, and failover reassignment |
| `framing.rs` | creator-side payload frame construction and publisher-side frame validation helpers |
| `ack_tracker.rs` | outstanding frame tracking, duplicate detection, timeout bookkeeping, and ACK correlation |
| `chunk_sender.rs` | creator-side send orchestration across the current bridge pool |
| `ingest.rs` | publisher-side in-memory receive / dedupe / accept path |
| `ack.rs` | publisher ACK construction and routing semantics |

Do not let `session.rs` or `chunk_sender.rs` become dumping grounds for every Phase 6 behavior.

### 5.2 Encryption And Confidentiality Boundary

Phase 6 should lock these behaviors:

- creator wraps publisher-destined ciphertext before sending `BridgeData`
- bridges only forward opaque `BridgeData` frames and never parse publisher payload contents
- publisher receives frames, validates envelope/session metadata, and ACKs accepted frames
- tests should prove that bridge code never needs access to publisher decryption material

### 5.3 Session Semantics

Phase 6 should keep session behavior deterministic:

- creator opens one logical upload session at a time per test path
- every `BridgeData` frame is correlated by `session_id`, `frame_id`, and `sequence`
- publisher ACKs accepted frames with stable correlation identifiers
- creator closes the logical session only after all frames are ACKed or the test path forces a failure condition

### 5.4 Fanout, Failover, And Reuse Rules

Phase 6 should lock these behaviors:

- prefer already-active direct bridges from the committed Phase 5 state
- use up to 10 active bridges when available
- if fewer than 10 are available before the fanout timeout, reuse already-live bridges deterministically
- when a bridge fails mid-session, reassign pending frames to the next eligible active bridge
- keep retry / reassignment deterministic in tests; do not introduce probabilistic balancing yet

### 5.5 Publisher Ingest Rules

Phase 6 publisher-side ingest should:

- accept `BridgeOpen` and initialize in-memory session state
- receive `BridgeData` frames and reject malformed or duplicate frames cleanly
- emit `BridgeAck` tied to the correct creator session and frame identifiers
- keep state in-memory only for this phase
- stop short of durable storage or full media assembly concerns unless strictly necessary for the test surface

---

## 6. Dependency And Implementation Policy

Phase 6 should keep dependencies minimal and bias toward deterministic in-process tests.

### Required Bias

- reuse committed Phase 3 authority, Phase 4 bridge runtime, and Phase 5 creator bootstrap surfaces
- structure time-sensitive logic so tests inject timestamps instead of sleeping
- keep publisher ingest and creator upload orchestration callable in-process from tests
- prefer `std` collections and in-memory state unless a new dependency is clearly justified

### Avoid In Phase 6

- adding network stacks or async runtimes unless a compile boundary truly requires them
- introducing persistent data stores
- mutating V1 chunk, onion, or publisher receive code
- pulling in discovery or advanced scoring logic that belongs to later phases

---

## 7. Evidence Capture Requirements

Phase 6 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or commit message |
| starting commit SHA | `git rev-parse HEAD` | phase notes or commit message |
| pre-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |
| active-bridge source used | Phase 5 runtime baseline + Phase 6 tests | phase notes |
| confidentiality boundary used | Phase 6 plan + tests | phase notes |
| ACK correlation rule used | Phase 6 plan + tests | phase notes |
| timeout reuse / failover rule used | Phase 6 plan + tests | phase notes |
| validation command set used | local command log | phase notes |
| temp `--target-dir` workaround, if needed | local command log | phase notes |
| post-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |

---

## 8. Recommended Execution Order

Implement Phase 6 in this order:

1. Capture the starting branch, commit SHA, and protected-path diff state.
2. Add `session.rs`, `bridge_pool.rs`, and `ack_tracker.rs` first so lifecycle and correlation rules are fixed early.
3. Add `fanout_scheduler.rs`, `framing.rs`, and `chunk_sender.rs` on top of those lower-level rules.
4. Add publisher-side `ingest.rs` and `ack.rs`.
5. Add `tests/data_path.rs`.
6. Run the V2 validation commands.
7. Run the V1 preservation checks and minimum V1 regressions.

This keeps the core session and ACK model stable before fanout and ingest behavior start composing around it.

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
$target = Join-Path $env:LOCALAPPDATA 'Temp\veritas-bridge-target-phase6'
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

- creator opens a bridge-mode upload session and delivers at least one payload end-to-end
- publisher ACKs correlate to the correct session and frame identifiers
- duplicate / replayed `BridgeData` is handled safely
- bridge failover during upload succeeds without touching V1 code
- creator reuses active bridges deterministically when fewer than 10 are available before timeout
- confidentiality tests show the bridge only handles opaque payload bytes
- protected V1 diff remains empty
- minimum V1 regression suite still passes

Executed result:

- `cargo fmt --all --check --manifest-path prototype/gbn-bridge-proto/Cargo.toml` passed
- `cargo check --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml --target-dir %LOCALAPPDATA%\Temp\veritas-bridge-target-phase6` passed
- `cargo test --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml --target-dir %LOCALAPPDATA%\Temp\veritas-bridge-target-phase6` passed
- `git diff --name-only -- <protected V1 paths>` returned no output
- `cargo check --workspace` passed in `prototype/gbn-proto`
- `cargo test -p mcn-router-sim` passed in `prototype/gbn-proto`

---

## 10. Acceptance Criteria

Phase 6 is complete only when all of the following are true:

- every file listed in the main execution plan exists
- creator-side session lifecycle is implemented
- payload framing and fanout scheduling across the active bridge pool are implemented
- ACK tracking and retransmission / failover behavior are implemented
- bridge-side opaque forwarding is implemented for the data-path flow
- publisher-side in-memory receive, dedupe, validate, and ACK flow is implemented
- active-bridge reuse is implemented when fewer than 10 bridges are available before timeout
- data-path tests cover end-to-end upload, ACK routing, duplicate handling, failover, confidentiality, and bridge reuse
- protected V1 diff is clean after validation
- minimum V1 regression suite still passes

---

## 11. Risks And Blockers

| Risk | What It Looks Like | Mitigation |
|---|---|---|
| Confidentiality drift | bridge code parses or depends on publisher payload contents | keep bridge handling opaque and test for payload opacity |
| ACK misrouting | creator cannot match `BridgeAck` to the correct frame or session | centralize ACK tracking and test correlation explicitly |
| Session sprawl | upload logic becomes fragmented across unrelated runtime modules | lock clear module ownership before implementation starts |
| Fanout instability | retry / reuse behavior depends on incidental ordering | define deterministic bridge-pool and scheduler rules now |
| Publisher ingest overreach | Phase 6 turns into durable storage or delivery-finalization work | keep ingest in-memory and focused on receive + ACK only |
| OneDrive validation noise | V2 tests fail for filesystem reasons instead of code reasons | use and document the temp-target fallback |
| Procedural drift | later-phase discovery or reachability logic leaks into Phase 6 | treat the committed Phase 5 active-bridge set as the only transport candidate source |

Current blocker:

- none; Phase 6 is implemented locally and validated

---

## 12. First Implementation Cut

If Phase 6 is implemented as a single focused change set, use this breakdown:

1. Session, bridge-pool, framing, and ACK tracking
2. Fanout scheduler and chunk sender
3. Publisher ingest / ACK path
4. End-to-end data-path tests

That keeps Phase 6 auditable and gives later discovery/policy phases a stable creator-upload contract instead of a moving upload and ACK surface.
