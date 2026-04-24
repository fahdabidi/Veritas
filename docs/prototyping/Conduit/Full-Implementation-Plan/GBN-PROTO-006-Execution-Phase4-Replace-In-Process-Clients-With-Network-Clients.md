# GBN-PROTO-006 - Execution Phase 4 Detailed Plan: Replace In-Process Clients With Network Clients

**Status:** Ready to start after Phase 3 bridge control sessions and command delivery are implemented and validated  
**Primary Goal:** replace `InProcessPublisherClient` and the remaining in-process creator/host-creator/bridge production coupling with real network clients and transport abstractions, while preserving the Phase 1 authority API and the Phase 3 bridge control-session model and isolating any remaining simulation path to explicit dev-only use  
**Source Plan:** [GBN-PROTO-006 Execution Plan](GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md)  
**Protected V1 Baseline:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)  
**Phase 3 Detailed Plan:** [GBN-PROTO-006-Execution-Phase3-Bridge-Control-Sessions-And-Command-Delivery](GBN-PROTO-006-Execution-Phase3-Bridge-Control-Sessions-And-Command-Delivery.md)  
**Starting Conduit Baseline:** `2b6d5c5d24e269e96e3fdc820f3f90669607414a`

---

## 1. Current Repo Findings

These findings should drive Phase 4 instead of being rediscovered during implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 4 should record the commit used to begin the runtime client cutover |
| Current HEAD commit | `2b6d5c5d24e269e96e3fdc820f3f90669607414a` | current committed Conduit baseline still relies on in-process publisher coupling in runtime code |
| Current runtime publisher coupling | [publisher_client.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/publisher_client.rs:1>) is entirely `InProcessPublisherClient` over `PublisherAuthority` | proves the production runtime path still bypasses real network boundaries |
| Current bootstrap flow | [bootstrap.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bootstrap.rs:1>) passes around `CreatorRuntime`, `HostCreator`, and `ExitBridgeRuntime` objects directly | first-contact creator bootstrap is still modeled as local object interaction, not a real distributed flow |
| Current host-creator relay path | [host_creator.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/host_creator.rs:1>) builds `CreatorJoinRequest` and calls `relay_bridge.publisher_client_mut().begin_bootstrap(...)` directly | proves the host-creator relay path has not yet crossed a real network boundary |
| Current creator refresh path | [creator.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/creator.rs:1>) uses `bridge.publisher_client_mut().issue_catalog(...)` | returning-creator refresh still depends on in-process publisher access |
| Current bridge runtime coupling | [bridge.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/bridge.rs:1>) embeds `InProcessPublisherClient` for registration, heartbeat, catalog, bootstrap, progress, and ingest | bridge production behavior is still not using the real networked publisher surfaces |
| Current progress reporting path | [progress_reporter.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/progress_reporter.rs:1>) reports directly through `InProcessPublisherClient` | progress still bypasses real authority and control/session boundaries |
| Current bridge control-session target | Phase 3 is intended to add real control sessions for publisher-to-bridge command delivery | Phase 4 must reuse that real bridge control path rather than inventing a second bridge-side command channel |
| Current V2 `chain_id` state | still absent in runtime request creation and host-creator relay code | Phase 4 must establish `chain_id` creation/forwarding at the creator and host-creator edges |

---

## 2. Review Summary

Phase 4 is where Conduit stops pretending that object references are network boundaries. If this phase is weak, later bootstrap and data-path phases will still be implemented against simulation-era coupling even if the Publisher itself is real.

The main gaps the detailed Phase 4 plan must close are:

| Gap | Why It Matters | Resolution For Phase 4 |
|---|---|---|
| `InProcessPublisherClient` is still the runtime default | creator, host-creator, and bridge production paths still bypass the real Publisher service | replace production usage with real network clients and isolate in-process mode to dev/test only |
| Bootstrap flow is object-graph based | first-contact behavior cannot be validated across real boundaries | define networked runtime clients for creator/host-creator/bridge interactions |
| Host-creator relay is not real | the architecture explicitly requires a host-assisted path to the Publisher | implement a real host-creator relay client/path instead of direct bridge object mutation |
| No shared network abstraction | ad hoc clients would fragment transport assumptions | add `network_transport.rs` as a shared abstraction for runtime client code |
| `chain_id` is not established at the runtime edge | trace continuity cannot begin reliably later | require creator-side or host-creator-side `chain_id` creation and forwarding in this phase |
| Simulation retirement risk | in-process code can quietly remain the production path if not explicitly fenced off | make dev-only simulation gating a formal Phase 4 requirement |

Phase 4 should replace runtime-side coupling with real network clients, but it should not yet implement the full publisher-directed bootstrap fanout state machine. That remains Phase 5.

---

## 3. Scope Lock

### In Scope

- replace production-path publisher calls in runtime code with real network clients
- add `publisher_api_client.rs` for authority API calls
- add `host_creator_client.rs` for host-creator relay behavior
- add `network_transport.rs` as the shared runtime transport abstraction
- refactor creator, host-creator, bridge, bootstrap, and progress-report code to use network clients instead of direct in-process publisher calls
- make any remaining in-process simulation path explicit dev-only or test-only
- establish `chain_id` creation or forwarding at creator/bootstrap entry
- add runtime integration tests proving the production path no longer depends on `InProcessPublisherClient`

### Out Of Scope

- full publisher-driven bootstrap distribution and fanout orchestration
- receiver / ACK data path
- AWS deployment promotion
- modifying `prototype/gbn-proto/**`
- modifying the main repo `README.md`

---

## 4. Preflight Gates

Phase 4 should not begin code edits until all of these are checked:

1. Confirm the Phase 0 inventory deliverables exist.
2. Confirm Phase 1 is implemented and validated so a real authority API exists to target.
3. Confirm Phase 3 is implemented and validated so the bridge command path already has a real control-session model.
4. Confirm protected V1 paths are clean in the local worktree.
5. Confirm the production runtime path will stop depending on `InProcessPublisherClient`.
6. Confirm any retained in-process mode is explicitly dev-only or test-only.
7. Confirm `chain_id` will be generated or forwarded at the creator / host-creator boundary.
8. Confirm `README.md` remains out of scope.

If any gate fails, Phase 4 should stop.

Current blocker:

- Phases 1 and 3 are not yet implemented, so Phase 4 remains planning-ready only

---

## 5. Client Replacement Decisions To Lock In Phase 4

### 5.1 Publisher API Client Choice

Phase 4 should add a real `PublisherApiClient` that speaks to the Phase 1 authority API over HTTP/JSON.

It should cover at least:

- bridge registration
- bridge heartbeat / renewal
- creator catalog refresh
- host-creator bootstrap join requests
- bootstrap or session progress reporting if those still use the authority API at this stage

This client should become the default production-path client used by runtime code.

### 5.2 Host-Creator Relay Client Choice

Phase 4 should add a dedicated `HostCreatorClient` rather than hard-coding bootstrap relay logic into `host_creator.rs`.

Responsibilities:

- accept a creator-originated first-contact request
- preserve or generate `chain_id`
- send the join request through the connected relay bridge / publisher path using real runtime transport abstractions
- return the publisher bootstrap response back to the creator-facing side

This keeps the host-creator role transport-scoped instead of letting it become a second authority surface.

### 5.3 Network Transport Abstraction

Phase 4 should introduce `network_transport.rs` as a shared runtime abstraction for request/response client behavior.

Minimum responsibilities:

- request serialization
- response deserialization
- timeout and retry hooks
- request metadata handling
- `chain_id` carriage
- testable transport mocking

The transport abstraction should be thin. It is there to keep runtime clients consistent, not to hide all protocol semantics.

### 5.4 In-Process Path Policy

`InProcessPublisherClient` may remain only under one of these conditions:

- explicit `dev-sim` feature gate
- explicit test-only build path
- explicit local harness-only adapter type

It must not remain the default runtime path for:

- creator refresh
- host-creator bootstrap relay
- bridge registration / heartbeat
- progress reporting

Phase 4 should make this distinction mechanically obvious in code.

### 5.5 ChainID Rule For Phase 4

Phase 4 must establish `chain_id` at the runtime entry points.

Rules:

- creator-originated bootstrap and upload flows must originate a root `chain_id` if one does not already exist
- host-creator relay must preserve the incoming `chain_id`
- publisher API requests from runtime code must carry `chain_id` in request envelopes or request metadata
- bridge-originated progress events tied to bootstrap or upload sessions must preserve the same `chain_id`
- local test assertions must verify `chain_id` continuity from creator to host creator to publisher

This is the first phase where Conduit runtime code should behave like V1 in correlation intent, even though the transport architecture is different.

### 5.6 Runtime Boundary Rule

Phase 4 should not try to replace every runtime edge at once.

Priority order:

1. publisher authority API calls
2. host-creator relay path
3. bridge progress reporting
4. explicit dev/test simulation isolation

The full publisher-directed bootstrap session and fanout path remains Phase 5.

---

## 6. Module Ownership To Lock In Phase 4

Phase 4 should keep runtime responsibilities split like this:

| Module | Responsibility |
|---|---|
| `publisher_api_client.rs` | real runtime client for Phase 1 authority API routes |
| `host_creator_client.rs` | host-creator relay client and transport-facing helper logic |
| `network_transport.rs` | shared client transport abstraction and metadata carriage |
| `publisher_client.rs` | compatibility layer only; should become a trait or dev-only adapter, not the default production client |
| `bootstrap.rs` | runtime bootstrap orchestration over clients and runtimes; must stop assuming local object reach-through |
| `host_creator.rs` | host-creator role model and local state, but not full transport implementation |
| `creator.rs` | creator-side orchestration over real clients and trusted publisher responses |
| `progress_reporter.rs` | progress emission through the new client abstraction, not direct in-process authority mutation |

Do not let `creator.rs` or `bridge.rs` absorb all client behavior directly. That would just replace one kind of coupling with another.

---

## 7. Dependency And Implementation Policy

Phase 4 requires real network clients, but the dependency expansion should stay consistent with the Phase 1 authority-service choices.

### Recommended Dependencies

- reuse the HTTP/JSON stack introduced for Phase 1
- client-side serialization libraries already compatible with Phase 1 envelopes
- timeout / retry helpers only if clearly scoped

### Bias

- keep client code explicit and typed
- keep request metadata and `chain_id` handling consistent across all runtime clients
- prefer one shared transport abstraction over multiple bespoke client implementations

### Avoid In Phase 4

- adding a second control transport stack for bridge commands
- leaving `InProcessPublisherClient` as the production fallback
- hiding `chain_id` only in logs instead of carrying it in runtime requests
- conflating runtime client replacement with bootstrap distribution logic

---

## 8. Evidence Capture Requirements

Phase 4 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or commit message |
| starting commit SHA | `git rev-parse HEAD` | phase notes or commit message |
| Phase 1 and Phase 3 prerequisite status | implementation and validation records | phase notes |
| pre-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |
| chosen runtime transport abstraction | `network_transport.rs` and manifests | phase notes |
| publisher API client route coverage | `publisher_api_client.rs` and tests | phase notes |
| host-creator relay path coverage | `host_creator_client.rs` and tests | phase notes |
| `chain_id` generation / forwarding behavior | runtime clients and tests | phase notes |
| production-path retirement of `InProcessPublisherClient` | code search and tests | phase notes |
| validation command set used | local command log | phase notes |
| temp `--target-dir` workaround, if needed | local command log | phase notes |
| post-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |

Do not sign off Phase 4 with only "network clients added." Record where `InProcessPublisherClient` was removed from production paths and how `chain_id` now enters the system.

---

## 9. Recommended Execution Order

Implement Phase 4 in this order:

1. Capture the starting branch, commit SHA, and protected-path diff state.
2. Introduce `network_transport.rs` first so all new runtime clients share one transport shape.
3. Implement `publisher_api_client.rs`.
4. Implement `host_creator_client.rs`.
5. Refactor `publisher_client.rs` into a trait / dev-only adapter boundary.
6. Refactor `host_creator.rs`, `bootstrap.rs`, `creator.rs`, and `progress_reporter.rs` to depend on the new clients.
7. Add `tests/network_client_flow.rs` covering real client use and `chain_id` continuity.
8. Add an explicit check that production entrypoints no longer instantiate `InProcessPublisherClient`.
9. Run the V2 workspace sanity suite.
10. Run the V1 preservation checks and minimum V1 regressions.

This order keeps the shared client boundary stable before the runtime orchestration code starts depending on it.

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
$target = Join-Path $env:LOCALAPPDATA 'Temp\\veritas-bridge-target-proto006-phase4'
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

Recommended Phase 4-specific checks:

```bash
rg -n "InProcessPublisherClient" prototype/gbn-bridge-proto/crates/gbn-bridge-runtime
```

```bash
rg -n "chain_id" prototype/gbn-bridge-proto/crates/gbn-bridge-runtime
```

```bash
git status --short
```

Expected outcome:

- creator, host-creator, and bridge production paths use real network clients for authority interactions
- `InProcessPublisherClient` is isolated to dev/test-only paths if it still exists
- host-creator relay behavior is exercised over a real client path
- `chain_id` is created or forwarded at runtime entry points and preserved through the tested request path
- protected V1 paths show no drift
- minimum V1 regression suite remains green

---

## 11. Acceptance Criteria

Phase 4 is complete when:

- production-path runtime code no longer depends on `InProcessPublisherClient`
- real runtime clients exist for authority API interactions
- a real host-creator relay path exists
- any retained in-process publisher adapter is explicitly dev-only or test-only
- `chain_id` is created or forwarded at creator/bootstrap entry and preserved through runtime request paths
- all required V1 and V2 validation commands have been run and recorded

Phase 4 is not complete if:

- the production path still instantiates `InProcessPublisherClient`
- `chain_id` is still absent from runtime request generation
- host-creator relay remains a direct object-to-object call

---

## 12. Risks And Blockers

| Risk | Why It Matters | Mitigation |
|---|---|---|
| `InProcessPublisherClient` remains as a quiet default fallback | the runtime cutover would be incomplete and easy to misread | make production-path retirement a formal acceptance criterion and test it explicitly |
| host-creator relay path is replaced only partially | first-contact bootstrap would still be simulation-bound | require a real host-creator client path and test it directly |
| `chain_id` generation is inconsistent between creator and host-creator | distributed tracing would fork early and become unreliable | define one runtime entry rule and enforce it in tests |
| runtime modules become tightly coupled to transport details | later bootstrap/fanout work would be harder to evolve | keep `network_transport.rs` and client modules explicit and thin |
| Phase 4 drifts into full bootstrap distribution | it would blur the boundary with Phase 5 | keep publisher-driven fanout state machine explicitly deferred |

---

## 13. Sign-Off Recommendation

The correct Phase 4 sign-off is:

- runtime production paths now use real network clients
- host-creator relay is no longer a local object call
- `chain_id` now enters Conduit at the runtime edge and survives the client boundary

The correct Phase 4 sign-off is not:

- full bootstrap distribution and fanout completion
- receiver / ACK path completion
- AWS deployment readiness
