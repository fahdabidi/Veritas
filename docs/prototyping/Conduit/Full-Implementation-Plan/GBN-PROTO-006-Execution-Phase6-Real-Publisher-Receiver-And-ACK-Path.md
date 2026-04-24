# GBN-PROTO-006 - Execution Phase 6 Detailed Plan: Real Publisher Receiver And ACK Path

**Status:** Implemented and validated locally on 2026-04-24
**Primary Goal:** replace the current simulated bridge-to-publisher forwarding path with a real publisher receiver service and ACK path, while preserving the Phase 1 authority API, the Phase 2 durable storage model, the Phase 3 bridge control sessions, the Phase 4 runtime network clients, and the Phase 5 real bootstrap distribution flow  
**Source Plan:** [GBN-PROTO-006 Execution Plan](GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md)  
**Protected V1 Baseline:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)  
**Phase 5 Detailed Plan:** [GBN-PROTO-006-Execution-Phase5-Real-Bootstrap-Distribution-And-Fanout](GBN-PROTO-006-Execution-Phase5-Real-Bootstrap-Distribution-And-Fanout.md)  
**Starting Conduit Baseline:** `f28741d05400e6f18ee82e29b7d612f7bf08ea98`

---

## 1. Current Repo Findings

These findings should drive Phase 6 instead of being rediscovered during implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 6 should record the mainline commit used to begin the receiver-path cutover |
| Current HEAD commit | `f28741d05400e6f18ee82e29b7d612f7bf08ea98` | records the committed Conduit baseline Phase 6 started from; the worktree now carries the Phase 6 receiver-path cutover on top |
| Current runtime forwarder | [`forwarder.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/forwarder.rs) now records locally forwarded `BridgeData` while [`forwarder_client.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/forwarder_client.rs) sends receiver open/data/close requests over the real authority HTTP surface | the production bridge-to-publisher payload path now crosses a real network boundary instead of mutating publisher state in process |
| Current runtime session model | [`session.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/session.rs) now binds one canonical upload `chain_id` to each `UploadSession`, and `BridgeSessionRegistry` preserves that `chain_id` across open/data/close calls | session framing is now coupled to a real receiver service with consistent per-session correlation |
| Current chunk sending path | [`chunk_sender.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/chunk_sender.rs) now opens sessions, forwards frames, and closes sessions through chain-aware bridge runtime methods that drive the receiver client | payload dispatch and ACK handling now ride the real receiver boundary instead of local authority mutation |
| Current publisher ingest path | [`receiver.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/receiver.rs), [`ack_service.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/ack_service.rs), and [`ingest.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/ingest.rs) now sit behind the authority HTTP service routes for receiver open/frame/close | publisher receiver behavior is now exposed over a real service boundary while keeping ingest domain logic isolated |
| Current publisher ACK generation | [`ack.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/ack.rs) and [`ack_service.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/ack_service.rs) now emit real `BridgeAck` responses through `/v1/receiver/frame` | ACK semantics are now tied to the receiver service instead of remaining local helper-only behavior |
| Current publisher receiver modules | `receiver.rs` and `ack_service.rs` now exist under `gbn-bridge-publisher/src/`, and [`service.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/service.rs) routes signed receiver requests through them | the Publisher now exposes the receiver service and ACK emission surface required by the architecture |
| Current runtime network forwarder module | [`forwarder_client.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/forwarder_client.rs) now exists and is auto-attached for network-mode bridges in [`bridge.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bridge.rs) | the bridge runtime now has a real network client for receiver open/data/close flows |
| Current delivery semantics | `ChunkSender::send_dispatches(...)` still expects a correlated `BridgeAck` per frame, but those ACKs are now returned from the receiver service after durable ingest and duplicate handling | runtime correlation remains synchronous for this phase, but it no longer depends on a simulated in-process ACK path |
| Current durable ingest requirement | Phase 2 is intended to make upload session and frame state durable | Phase 6 must store receiver session state, ingested frames, and ACK correlation records durably rather than assuming in-memory-only authority state |
| Current `chain_id` state | upload sessions now originate a root `chain_id`, receiver open/frame/close requests carry it in signed authority envelopes, and upload session / frame records persist it durably | Phase 6 now preserves one `chain_id` from session open through frame ingest, ACK emission, and close/error paths while leaving broader cross-system enforcement for Phase 7 |

---

## 2. Review Summary

Phase 6 is where Conduit must stop treating publisher ingest and ACKs as a local function call and start treating them as a real receiver service with correlated session, frame, retry, and close semantics. If this phase is weak, the bootstrap path may be real, but the core payload path would still be simulated.

The main gaps the detailed Phase 6 plan must close are:

| Gap | Why It Matters | Resolution For Phase 6 |
|---|---|---|
| Bridge forwarding is still an in-process client call | payload transport still does not cross a real publisher boundary | add a real runtime `forwarder_client.rs` and publisher `receiver.rs` service path |
| Ingest and ACK are library helpers only | the Publisher still does not expose a receiver service that bridges can talk to | wrap ingest/ACK logic behind a real network receiver interface |
| Session open/data/close are not tied to a real receiver | runtime-side session lifecycle cannot be validated over real boundaries | define a real receiver protocol for open, data, ACK, retry, and close |
| ACKs are assumed to be synchronous local returns | retry and out-of-order behavior are not being modeled properly | define explicit ACK correlation and retry semantics over the receiver service |
| No production `forwarder_client.rs` exists | bridges still have no networked forwarding implementation | implement a real bridge-side forwarding client and isolate any in-process path to dev/test only |
| `chain_id` drops across data ingress | distributed tracing would still break at the payload boundary | require `chain_id` continuity on open, data, ACK, and close/error events |
| Overreach risk | it is easy to drift into end-state observability or deployment work | keep trace-hardening and AWS/mobile rollout deferred to later phases |

Phase 6 should make the data receiver and ACK path real, but it should not yet try to solve full observability rollout or deployment promotion.

---

## 3. Scope Lock

### In Scope

- add publisher-side `receiver.rs` and `ack_service.rs`
- add runtime-side `forwarder_client.rs`
- replace in-process payload forwarding in production paths
- implement real session open / data / close handling over a receiver service boundary
- implement real publisher ACK emission for ingested frames
- define correlation, ordering, retry, and duplicate-handling rules for forwarded frames
- persist the receiver-side upload session and ingest state through the Phase 2 durable storage boundary
- propagate `chain_id` on session open, frame ingress, ACK emission, and close/error paths
- add end-to-end receiver flow tests over real service boundaries

### Out Of Scope

- final cross-cutting trace enforcement across every remaining path
- deployment promotion and live AWS/mobile validation
- modifying `prototype/gbn-proto/**`
- modifying the main repo `README.md`

---

## 4. Preflight Gates

Phase 6 should not begin code edits until all of these are checked:

1. Confirm the Phase 0 inventory deliverables exist.
2. Confirm Phase 1 is implemented and validated so the Publisher already exposes a real authority-service boundary.
3. Confirm Phase 2 is implemented and validated so upload session and ingest records can be stored durably.
4. Confirm Phase 3 is implemented and validated so bridge control and bridge liveness are already real.
5. Confirm Phase 4 is implemented and validated so runtime code already uses real publisher-facing network clients.
6. Confirm Phase 5 is implemented and validated so first-contact bootstrap and bridge fanout no longer depend on simulation handoff.
7. Confirm protected V1 paths are clean in the local worktree.
8. Confirm the production payload path will no longer depend on `InProcessPublisherClient`.
9. Confirm `chain_id` will be preserved from session open through close / error.
10. Confirm `README.md` remains out of scope.

If any gate fails, Phase 6 should stop.

Implementation outcome:

- Phase 6 is now implemented locally and validated with:
  - V2 `cargo fmt --all --check`
  - V2 `cargo test --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml --target-dir %LOCALAPPDATA%\\Temp\\veritas-proto006-phase6-target`
  - protected V1 path diff clean
  - V1 `cargo check --workspace`
  - V1 `cargo test -p mcn-router-sim`

---

## 5. Receiver And ACK Decisions To Lock In Phase 6

### 5.1 Real Receiver Service Boundary

Phase 6 should introduce a real Publisher receiver service rather than continuing to expose ingest as library functions only.

The receiver service must accept at least:

- `BridgeOpen`
- `BridgeData`
- `BridgeClose`

It must return:

- correlated `BridgeAck` responses
- explicit receiver-side errors when session, ordering, or dedupe constraints fail

Do not keep `ingest.rs` as the production boundary. It should remain domain logic behind the new receiver service.

### 5.2 Bridge-Side Forwarding Client Rule

The bridge runtime must forward payloads to the Publisher through a real `forwarder_client.rs`.

Responsibilities:

- open receiver session
- forward `BridgeData` frames
- receive and validate `BridgeAck`
- close sessions explicitly
- retry on transient failures under bounded rules
- preserve `chain_id` across every request

Do not let `PayloadForwarder` continue to call `InProcessPublisherClient::forward_frame(...)` in the production path.

### 5.3 Session Lifecycle Rule

Phase 6 should lock a real receiver-side session lifecycle:

- `open_requested`
- `opened`
- `receiving`
- `completed`
- `closed`
- `failed`

Minimum requirements:

- receiver rejects `BridgeData` for unknown sessions
- receiver rejects or deduplicates repeated `BridgeOpen` safely
- receiver accepts duplicate frames idempotently using session/frame correlation
- receiver honors `final_frame`
- receiver handles explicit `BridgeClose`

The runtime and Publisher must agree on one canonical session id and one canonical ordering model per upload session.

### 5.4 ACK Correlation Rule

ACK emission must be explicit and deterministic.

Rules:

- every accepted or duplicate frame receives a correlated `BridgeAck`
- ACKs are tied to the same `session_id`
- ACKs identify the acknowledged frame sequence deterministically
- duplicate frames must not create ambiguous ACK state
- receiver-side completion must produce a stable terminal ACK state

Do not treat ACKs as incidental return values from local function calls. They are protocol events on the real receiver path.

### 5.5 Reordering, Retry, And Duplicate Rule

Phase 6 should define receiver behavior for:

- frame reordering
- duplicate delivery
- bridge retry after timeout
- retries after uncertain ACK delivery

At minimum:

- duplicate `frame_id` or duplicate sequence delivery must be safe and idempotent
- the runtime must be able to retry a frame when the receiver outcome is unknown
- the receiver must not double-ingest payload bytes for duplicate logical frames
- test coverage must include partial retransmit and duplicate ACK scenarios

### 5.6 ChainID Rule For Phase 6

Phase 6 must carry one root `chain_id` across:

- session open
- every `BridgeData` frame
- every `BridgeAck`
- close events
- receiver-side errors that terminate the session

It is not enough to attach `chain_id` only in the runtime log. It must be available in request metadata, receiver records, ACK correlation, and end-to-end tests.

### 5.7 Publisher Receiver Ownership Rule

The Publisher remains authoritative for ingest acceptance and ACK emission.

Phase 6 must not let bridges:

- authoritatively mark payload frames complete on their own
- synthesize final ACK state locally without receiver confirmation
- redefine receiver dedupe or ordering semantics independently

Those remain Publisher responsibilities.

---

## 6. Module Ownership To Lock In Phase 6

Phase 6 should keep responsibilities split like this:

| Module | Responsibility |
|---|---|
| `gbn-bridge-publisher/src/receiver.rs` | real receiver service boundary for session open, data ingest, close, and receiver-facing errors |
| `gbn-bridge-publisher/src/ack_service.rs` | ACK construction, terminal state rules, and receiver-to-runtime ACK emission helpers |
| `gbn-bridge-publisher/src/ingest.rs` | domain logic for upload-session mutation, dedupe, and frame ingest behind the receiver service |
| `gbn-bridge-publisher/src/ack.rs` | typed ACK helpers and shared ACK model support, but not the full receiver service boundary |
| `gbn-bridge-runtime/src/forwarder_client.rs` | bridge-side network client for open/data/close and ACK handling |
| `gbn-bridge-runtime/src/forwarder.rs` | bridge-local forwarding orchestration over `forwarder_client.rs`, not direct publisher mutation |
| `gbn-bridge-runtime/src/session.rs` | upload-session framing and lifecycle model used by the runtime sender side |
| `gbn-bridge-runtime/src/chunk_sender.rs` | session open / dispatch / close orchestration over the real forwarder client and ACK path |
| `gbn-bridge-runtime/tests/receiver_flow.rs` | end-to-end receiver flow, duplicate, retry, reorder, and `chain_id` continuity tests |

Do not let `forwarder.rs` or `ingest.rs` become dumping grounds for all network, ACK, and persistence behavior. Keep the service boundary and domain logic separate.

---

## 7. Dependency And Implementation Policy

Phase 6 should reuse the already selected runtime and publisher networking surfaces instead of introducing a second transport family.

### Recommended Dependencies

- reuse the Phase 1 service stack for the publisher-side receiver surface
- reuse the Phase 4 runtime client patterns for the bridge-side forwarding client
- reuse the Phase 2 durable storage adapter for session and frame persistence
- add retry/backoff helpers only if tightly scoped to forwarding and ACK uncertainty

### Bias

- keep receiver protocol messages explicit and typed
- keep ACK semantics deterministic and auditable
- keep `chain_id` available in receiver requests, stored records, and ACK outputs
- keep domain ingest logic separate from network service code

### Avoid In Phase 6

- introducing a second ad hoc bridge-to-publisher transport only for the receiver path
- retaining `InProcessPublisherClient` as the production forwarding path
- hiding duplicate and retry semantics inside local-only runtime state
- treating ACKs as best-effort logs rather than protocol events
- drifting into deployment promotion or global observability rollout

---

## 8. Evidence Capture Requirements

Phase 6 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or commit message |
| starting commit SHA | `git rev-parse HEAD` | phase notes or commit message |
| Phase 1-5 prerequisite status | implementation and validation records | phase notes |
| pre-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |
| receiver service boundary | `receiver.rs` and tests | phase notes |
| ACK emission path | `ack_service.rs`, `ack.rs`, and tests | phase notes |
| runtime forwarding client path | `forwarder_client.rs`, `forwarder.rs`, and tests | phase notes |
| session open/data/close behavior | `session.rs`, `chunk_sender.rs`, and tests | phase notes |
| duplicate / retry handling | ingest records and `receiver_flow.rs` tests | phase notes |
| `chain_id` continuity evidence | receiver records, ACKs, and tests | phase notes |
| validation command set used | local command log | phase notes |
| temp `--target-dir` workaround, if needed | local command log | phase notes |
| post-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |

Do not sign off Phase 6 with only "frames arrive." Record exactly where the real receiver boundary begins, how ACK correlation works, how duplicates are handled, and where `chain_id` survives the full forwarding path.

---

## 9. Recommended Execution Order

Implement Phase 6 in this order:

1. Capture the starting branch, commit SHA, and protected-path diff state.
2. Introduce `receiver.rs` first and define the real publisher receiver service boundary.
3. Implement `ack_service.rs` and wire explicit ACK emission rules.
4. Refactor `ingest.rs` and `ack.rs` so they serve the receiver service instead of being treated as the service boundary.
5. Implement `forwarder_client.rs` as the real bridge-side client for open/data/close and ACK handling.
6. Refactor `forwarder.rs`, `session.rs`, and `chunk_sender.rs` so production forwarding no longer depends on `InProcessPublisherClient`.
7. Add `tests/receiver_flow.rs` covering normal flow, duplicate delivery, retry, reordering, and `chain_id` continuity.
8. Run the V2 workspace sanity suite.
9. Run the V1 preservation checks and minimum V1 regressions.

This order keeps the Publisher receiver contract stable before the bridge runtime forwarding and ACK logic starts depending on it.

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
$target = Join-Path $env:LOCALAPPDATA 'Temp\\veritas-bridge-target-proto006-phase6'
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

Recommended Phase 6-specific checks:

```bash
rg -n "InProcessPublisherClient|forward_frame" prototype/gbn-bridge-proto/crates/gbn-bridge-runtime
```

```bash
rg -n "open_session|ingest_frame|close_session" prototype/gbn-bridge-proto/crates/gbn-bridge-publisher
```

```bash
rg -n "chain_id" prototype/gbn-bridge-proto/crates/gbn-bridge-runtime prototype/gbn-bridge-proto/crates/gbn-bridge-publisher
```

```bash
git status --short
```

Expected outcome:

- bridge-to-publisher forwarding crosses a real receiver service boundary
- ACKs are emitted from the Publisher over a real correlated path
- runtime forwarding no longer depends on `InProcessPublisherClient`
- duplicates, retries, and out-of-order delivery are covered by tests
- `chain_id` is continuous across session open, frame ingest, ACK emission, and close/error
- protected V1 paths show no drift
- minimum V1 regression suite remains green

---

## 11. Acceptance Criteria

Phase 6 is complete when:

- production-path bridge forwarding no longer depends on `InProcessPublisherClient`
- a real publisher receiver service exists for `BridgeOpen`, `BridgeData`, and `BridgeClose`
- ACKs are emitted through a real receiver/ACK path rather than as local helper returns only
- runtime forwarding uses a real `forwarder_client.rs`
- duplicate, retry, and reordering behavior are defined and covered by tests
- `chain_id` is present in session open, frame ingest, ACK, and close/error paths
- all required V1 and V2 validation commands have been run and recorded

Phase 6 is not complete if:

- `forwarder.rs` still sends frames by calling `InProcessPublisherClient` in the production path
- the publisher receiver is still only `ingest.rs` library calls
- ACK semantics remain implicit or uncorrelated
- `chain_id` still drops between bridge forwarding and publisher ACK emission

---

## 12. Risks And Blockers

| Risk | Why It Matters | Mitigation |
|---|---|---|
| local forwarding remains quietly enabled in production | the payload path would still be simulation-bound | make retirement of `InProcessPublisherClient` forwarding an explicit acceptance criterion |
| receiver and ingest logic become tightly coupled | network/service changes would be harder to evolve and test | keep `receiver.rs` as the service boundary and `ingest.rs` as domain logic |
| ACKs are emitted without durable correlation | retries and duplicate handling become ambiguous | persist enough session/frame correlation to rebuild deterministic ACK state |
| runtime assumes immediate synchronous ACKs | real network behavior will produce timing uncertainty | define bounded retry rules and test unknown-outcome retransmission explicitly |
| `chain_id` is only added to logs | forwarding and ACK trace continuity would still be incomplete | require `chain_id` in receiver requests, stored records, ACK payloads, and tests |
| Phase 6 drifts into deployment or global tracing work | it would blur the boundary with later phases | keep deployment promotion and cross-cutting trace hardening deferred |

---

## 13. Sign-Off Recommendation

The correct Phase 6 sign-off is:

- the Publisher now exposes a real receiver service
- bridges forward open/data/close events over a real network client
- ACKs are emitted by the Publisher over a real correlated path
- duplicate and retry semantics are real and test-covered
- `chain_id` now survives the full receiver and ACK path

The correct Phase 6 sign-off is not:

- deployment readiness
- final cross-cutting trace hardening
- a local ingest simulation that still depends on `InProcessPublisherClient`
