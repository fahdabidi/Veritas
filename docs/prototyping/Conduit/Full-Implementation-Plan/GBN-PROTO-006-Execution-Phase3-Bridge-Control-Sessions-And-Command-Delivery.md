# GBN-PROTO-006 - Execution Phase 3 Detailed Plan: Bridge Control Sessions And Command Delivery

**Status:** Ready to start after Phase 2 durable publisher storage and recovery is implemented and validated  
**Primary Goal:** implement real authenticated long-lived bridge-to-publisher control sessions so the Publisher can actively deliver bootstrap seed assignments, fanout assignments, punch directives, revocations, and refresh notifications over a real network boundary, while preserving the Phase 1 authority API and the Phase 2 durable state model  
**Source Plan:** [GBN-PROTO-006 Execution Plan](GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md)  
**Protected V1 Baseline:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)  
**Phase 2 Detailed Plan:** [GBN-PROTO-006-Execution-Phase2-Durable-Publisher-Storage-And-Recovery](GBN-PROTO-006-Execution-Phase2-Durable-Publisher-Storage-And-Recovery.md)  
**Starting Conduit Baseline:** `2b6d5c5d24e269e96e3fdc820f3f90669607414a`

---

## 1. Current Repo Findings

These findings should drive Phase 3 instead of being rediscovered during implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 3 should record the mainline commit used to begin the control-session cutover |
| Current HEAD commit | `2b6d5c5d24e269e96e3fdc820f3f90669607414a` | current committed Conduit baseline still lacks real bridge command delivery |
| Current bridge runtime coupling | `ExitBridgeRuntime` still owns `InProcessPublisherClient` in [bridge.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bridge.rs:1>) | proves bridges still depend on in-process publisher calls rather than a real control session |
| Current progress reporting path | `ProgressReporter` reports directly through `InProcessPublisherClient` in [progress_reporter.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/progress_reporter.rs:1>) | proves bootstrap progress still bypasses any real networked bridge-to-publisher session |
| Current publisher command output | signed `BridgePunchStart` and `BridgeBatchAssign` objects already exist in [punch.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/punch.rs:1>) | Phase 3 should deliver these authoritative commands over a real control channel instead of inventing different command semantics |
| Current bootstrap orchestration | `bootstrap.rs` returns `AuthorityBootstrapPlan` from publisher logic directly in process | proves the authority can already decide assignments, but cannot yet deliver them to real bridge runtimes |
| Current publisher service state | Phase 1 is intended to introduce a real authority API; bridge push delivery is still missing | Phase 3 must extend the Publisher with a bidirectional bridge control plane, not re-open the authority API design |
| Current durable state need | Phase 2 is intended to persist bootstrap sessions, assignments, and progress records | Phase 3 depends on durable pending-command and resume state for reconnect and replay-safe delivery |
| Current V2 `chain_id` state | still absent from bridge/runtime and publisher command paths | Phase 3 must carry `chain_id` through every assignment, command, and progress-report path |
| Current test coverage | runtime tests cover bootstrap, reachability, and data path locally, but there is no `control_session.rs` test yet | Phase 3 needs dedicated control-session and reconnect coverage |

---

## 2. Review Summary

Phase 3 is the first phase that makes the Publisher truly active instead of request/response-only. If this phase is weak, the Publisher may be durable and networked, but still unable to perform the architecture’s core responsibility of distributing instructions to bridges.

The main gaps the detailed Phase 3 plan must close are:

| Gap | Why It Matters | Resolution For Phase 3 |
|---|---|---|
| No long-lived bridge control channel | the Publisher cannot actively distribute seed or fanout instructions | implement outbound bridge-to-publisher authenticated control sessions |
| No command envelope or delivery model | signed command objects exist, but there is no real delivery/resume/ACK mechanism | define an explicit control envelope with sequence, command id, `chain_id`, and ACK semantics |
| No reconnect / resume behavior | bridge disconnects would lose pending commands or duplicate them unsafely | persist pending command state and define last-acked resume semantics |
| No separation between command generation and command transport | authority logic could become tangled with session state | keep `bootstrap.rs` and `batching.rs` responsible for decisions, and put delivery into `control.rs` / `dispatcher.rs` |
| No `chain_id` rule for bridge command delivery | trace continuity would still break at the Publisher-to-bridge boundary | require `chain_id` on every assignment, command envelope, and progress ACK tied to a creator/bootstrap flow |
| Overreach risk | it is easy to turn Phase 3 into creator-client replacement or full receiver work | keep creator network clients deferred to Phase 4 and data-path receiver work deferred to Phase 6 |

Phase 3 should make the Publisher able to push, resume, and correlate authoritative bridge instructions, but it should not yet replace creator-side clients or the receiver path.

---

## 3. Scope Lock

### In Scope

- implement real authenticated bridge-to-publisher control sessions
- add bridge-side control client behavior in `gbn-bridge-runtime`
- add publisher-side control-session acceptance, session registry, and command dispatch
- add explicit command envelopes, ACK envelopes, and resume behavior
- deliver real authoritative publisher outputs over the control channel for:
  - seed bridge assignment
  - batch assignment
  - punch directives
  - revocations
  - descriptor refresh notifications
- carry `chain_id` through command delivery and progress ACK/report flows
- add local network integration tests for connect, reconnect, command ACK, stale-session rejection, and trace continuity

### Out Of Scope

- creator/host-creator runtime client replacement
- full bootstrap response delivery through the host-creator path
- receiver / ACK data plane
- AWS deployment promotion
- modifying `prototype/gbn-proto/**`
- modifying the main repo `README.md`

---

## 4. Preflight Gates

Phase 3 should not begin code edits until all of these are checked:

1. Confirm the Phase 0 inventory deliverables exist.
2. Confirm Phase 1 is implemented and validated so the Publisher already exposes a real authority API boundary.
3. Confirm Phase 2 is implemented and validated so pending commands, assignments, and progress state can be persisted durably.
4. Confirm protected V1 paths are clean in the local worktree.
5. Confirm the control-session transport will be a long-lived outbound bridge session rather than naive polling.
6. Confirm `chain_id` will be carried through control envelopes for creator/bootstrap-related commands and progress reports.
7. Confirm Phase 3 is not replacing creator-side clients yet.
8. Confirm `README.md` remains out of scope.

If any gate fails, Phase 3 should stop.

Current blocker:

- Phases 1 and 2 are not yet implemented, so Phase 3 remains planning-ready only

---

## 5. Control Session Decisions To Lock In Phase 3

### 5.1 Transport Choice

Phase 3 should implement bridge control sessions as long-lived outbound WebSocket sessions over the Publisher’s existing HTTP service surface.

Reason:

- Phase 1 already establishes an HTTP authority service
- WebSocket gives bidirectional streaming without introducing a second RPC stack prematurely
- bridges dialing outbound to the Publisher works better across NAT and restrictive networks than expecting publisher-initiated inbound control channels
- this keeps the bridge command plane and the authority API under one service surface while still separating handlers cleanly

Do not implement short-interval HTTP polling as the production control-channel model. Polling is a shortcut and does not satisfy the intent of an active Publisher-dispatch design.

### 5.2 Session Establishment Model

The bridge should establish the control session with a signed `BridgeControlHello` envelope that includes:

- `bridge_id`
- `lease_id`
- `bridge_pub`
- `sent_at_ms`
- `request_id`
- optional resume cursor
- `chain_id`
- bridge signature over the hello payload

The Publisher should validate:

- bridge identity
- active or resumable lease state
- timestamp skew bounds
- replay resistance on the handshake

If accepted, the Publisher returns a signed `BridgeControlWelcome` containing:

- `bridge_id`
- `session_id`
- `accepted_at_ms`
- negotiated heartbeat / idle timeout values
- last-known publisher cursor or resume point
- `chain_id`

### 5.3 Command Envelope Model

Phase 3 should not send raw `BridgePunchStart` or `BridgeBatchAssign` objects directly over the socket without transport metadata.

Use a V2-local control envelope with at least:

- `session_id`
- `bridge_id`
- `command_id`
- `seq_no`
- `issued_at_ms`
- `chain_id`
- `command_type`
- `payload`

Where `payload` is one of:

- signed `BridgePunchStart`
- signed `BridgeBatchAssign`
- signed revoke or refresh-notify payload
- later bridge command types as the design expands

This preserves the existing signed authority messages while adding delivery, replay, and trace semantics at the session layer.

### 5.4 ACK And Resume Model

The bridge must ACK received commands explicitly.

Minimum ACK fields:

- `session_id`
- `bridge_id`
- `command_id`
- `seq_no`
- `acked_at_ms`
- `chain_id`
- command-result status

Resume rule:

- the bridge reconnects with its last fully acked `seq_no` or `command_id`
- the Publisher reloads unacked commands from durable storage and redelivers them in order
- duplicate delivery must be tolerated and deduplicated by `command_id`

### 5.5 Keepalive And Liveness Model

Phase 3 should define:

- control-session heartbeat interval
- idle timeout
- reconnect backoff
- session-expiry behavior after prolonged disconnect

These are session-level liveness rules and must remain separate from lease renewal semantics.

### 5.6 ChainID Rule For Phase 3

Phase 3 must carry `chain_id` through all control-plane command delivery that belongs to a creator/bootstrap flow.

Required paths:

- publisher seed assignment to `ExitBridgeB`
- publisher batch assignment to remaining bridges
- publisher punch directive delivery
- bridge progress reports for tunnel establishment
- bridge ACKs for received commands

For purely bridge-local session-liveness traffic with no creator flow, Phase 3 may generate a session-local `chain_id`, but the field name must still be `chain_id`.

### 5.7 Ordering Rule

Publisher command delivery must be deterministic and replay-safe.

Rules:

- commands are ordered per bridge session
- command ids are globally unique enough for dedupe
- redelivery must not create ambiguous side effects
- ordering should be stable across reconnect for commands that were durable before disconnect

---

## 6. Module Ownership To Lock In Phase 3

Phase 3 should keep module responsibilities split like this:

| Module | Responsibility |
|---|---|
| `gbn-bridge-publisher/src/control.rs` | server-side control-session handshake, message framing, keepalive, session lifecycle |
| `gbn-bridge-publisher/src/dispatcher.rs` | loading pending commands from storage, sending them to live sessions, redelivery and resume coordination |
| `gbn-bridge-publisher/src/assignment.rs` | typed command/state records, envelope mapping, command result and dedupe helpers |
| `gbn-bridge-runtime/src/control_client.rs` | bridge-side WebSocket client, handshake, receive loop, ACK loop, reconnect behavior |
| `gbn-bridge-publisher/src/bootstrap.rs` | still generates authoritative bootstrap outputs; must not absorb session transport logic |
| `gbn-bridge-publisher/src/batching.rs` | still generates authoritative batch decisions; must not absorb session transport logic |
| `gbn-bridge-runtime/src/bridge.rs` | integrates the control client into the bridge lifecycle, but should not own command-queue persistence logic |

Do not turn `bridge.rs` into the full control-channel implementation. The bridge runtime should consume the control client, not subsume it.

---

## 7. Dependency And Implementation Policy

Phase 3 requires a real streaming transport, but the dependency expansion should stay narrow.

### Recommended Dependencies

- publisher-side WebSocket support on top of the Phase 1 HTTP stack
- bridge-side WebSocket client support
- serialization utilities already compatible with the current V2 stack
- `tokio` runtime support if not already present from Phase 1

### Bias

- prefer one streaming control transport over multiple overlapping mechanisms
- keep command envelopes explicit and typed
- keep control-session storage and delivery state separate from authority decision logic

### Avoid In Phase 3

- bridge polling loops as the production mechanism
- queue infrastructure outside the Publisher database unless a concrete blocker is discovered
- creator-client replacement
- receiver/data-path implementation
- AWS-only assumptions that prevent local network tests

---

## 8. Evidence Capture Requirements

Phase 3 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or commit message |
| starting commit SHA | `git rev-parse HEAD` | phase notes or commit message |
| Phase 1 and Phase 2 prerequisite status | implementation notes and validation records | phase notes |
| pre-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |
| chosen control transport | manifests and control modules | phase notes |
| command envelope shape | `assignment.rs` / tests | phase notes |
| handshake and resume semantics | `control.rs`, `control_client.rs`, and tests | phase notes |
| `chain_id` propagation points | control-envelope types and tests | phase notes |
| validation command set used | local command log | phase notes |
| temp `--target-dir` workaround, if needed | local command log | phase notes |
| post-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |

Do not sign off Phase 3 with only "WebSocket added." Record the handshake model, ACK semantics, resume semantics, and `chain_id` behavior explicitly.

---

## 9. Recommended Execution Order

Implement Phase 3 in this order:

1. Capture the starting branch, commit SHA, and protected-path diff state.
2. Define the control-envelope and assignment state model in `assignment.rs`.
3. Implement publisher-side control-session handshake and lifecycle in `control.rs`.
4. Implement dispatcher and redelivery behavior in `dispatcher.rs`.
5. Implement bridge-side `control_client.rs`.
6. Integrate the control client into `bridge.rs` without replacing creator/runtime clients yet.
7. Add `tests/control_session.rs` covering connect, reconnect, ACK, stale-session rejection, and `chain_id` continuity.
8. Run the V2 workspace sanity suite.
9. Run the V1 preservation checks and minimum V1 regressions.

This order keeps the wire contract and resume semantics stable before bridge integration code starts depending on them.

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
$target = Join-Path $env:LOCALAPPDATA 'Temp\\veritas-bridge-target-proto006-phase3'
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

Recommended Phase 3-specific checks:

```bash
rg -n "chain_id" prototype/gbn-bridge-proto/crates/gbn-bridge-publisher prototype/gbn-bridge-proto/crates/gbn-bridge-runtime
```

```bash
rg -n "InProcessPublisherClient" prototype/gbn-bridge-proto/crates/gbn-bridge-runtime
```

```bash
git status --short
```

Expected outcome:

- bridges establish real authenticated control sessions to the Publisher
- the Publisher can push authoritative commands over those sessions
- reconnect and resume tests pass
- `chain_id` is preserved across command delivery and progress reporting
- protected V1 paths show no drift
- minimum V1 regression suite remains green

Note: `InProcessPublisherClient` may still exist elsewhere until Phase 4, but Phase 3 should prove that bridge command delivery no longer depends on it in the bridge control path.

---

## 11. Acceptance Criteria

Phase 3 is complete when:

- a real authenticated bridge control session exists
- the Publisher can actively deliver:
  - seed assignments
  - batch assignments
  - punch directives
  - revocations or refresh notifications
- bridges ACK commands explicitly
- reconnect and resume behavior is implemented and tested
- `chain_id` is present and preserved on command and progress paths
- all required V1 and V2 validation commands have been run and recorded

Phase 3 is not complete if:

- bridge command delivery still depends on direct in-process publisher calls
- the delivery model is polling-based in the production path
- commands cannot be resumed safely after disconnect
- `chain_id` disappears between publisher dispatch and bridge progress reporting

---

## 12. Risks And Blockers

| Risk | Why It Matters | Mitigation |
|---|---|---|
| Polling is used as a shortcut | the Publisher would still not be truly active in distributing instructions | require long-lived control sessions as the production path |
| Dispatcher logic becomes mixed into authority decisions | bootstrap and batching logic would become hard to evolve | keep decision generation and transport delivery in separate modules |
| Resume semantics are weak or implicit | duplicate or lost commands would be likely during reconnect | make command ids, seq numbers, ACKs, and durable pending state explicit |
| `chain_id` is added only to logs | trace continuity would still break across reconnect and persistence boundaries | require `chain_id` in control envelopes and command-state records |
| Bridge integration sprawls into creator/runtime refactors | Phase 3 would overrun into Phase 4 | keep creator-network replacement explicitly out of scope |

---

## 13. Sign-Off Recommendation

The correct Phase 3 sign-off is:

- the Publisher can actively and durably deliver bridge commands
- bridges can maintain, lose, and resume authenticated control sessions safely
- `chain_id` survives command delivery and progress-report paths

The correct Phase 3 sign-off is not:

- creator runtime network-client replacement
- full bootstrap response path replacement
- receiver / ACK data-path completion
- full AWS deployment readiness
