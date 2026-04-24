# GBN-PROTO-006 - Execution Phase 1 Detailed Plan: Real Publisher Authority API Service

**Status:** Completed and validated locally on 2026-04-23  
**Primary Goal:** replace the current library-only publisher boundary with a real authenticated authority API service in `gbn-bridge-publisher`, while preserving the existing Conduit authority logic and keeping persistence, bridge control sessions, and deployment promotion deferred to later phases  
**Source Plan:** [GBN-PROTO-006 Execution Plan](GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md)  
**Protected V1 Baseline:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)  
**Starting Conduit Baseline:** `2b6d5c5d24e269e96e3fdc820f3f90669607414a`

---

## 1. Current Repo Findings

These findings should drive Phase 1 instead of being rediscovered during implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 1 should record the mainline commit used to begin the real publisher-service cutover |
| Current HEAD commit | `2b6d5c5d24e269e96e3fdc820f3f90669607414a` | current committed Conduit baseline that still uses a simulated publisher boundary |
| Phase 0 GBN-PROTO-006 state | current-state and gap-inventory docs now exist | Phase 1 can begin against an explicit inventory rather than assumptions |
| Current publisher crate dependency set | only `ed25519-dalek`, `gbn-bridge-protocol`, and `thiserror` in [gbn-bridge-publisher/Cargo.toml](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/Cargo.toml:1>) | proves there is no current HTTP server/runtime stack |
| Current publisher service layer | wrapper-only `AuthorityServer` in [gbn-bridge-publisher/src/server.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/server.rs:1>) | Phase 1 must replace this with a real network listener and request handling layer |
| Current publisher entrypoint | placeholder `bridge-publisher` binary in [gbn-bridge-cli/src/bin/bridge-publisher.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/bin/bridge-publisher.rs:1>) backed by placeholder deployment logic in [gbn-bridge-cli/src/lib.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-cli/src/lib.rs:1>) | Phase 1 must make the publisher binary run a real service instead of a placeholder process |
| Current runtime coupling | `InProcessPublisherClient` in [gbn-bridge-runtime/src/publisher_client.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/publisher_client.rs:1>) | Phase 1 does not replace all runtime clients yet, but it must create the real service they will later connect to |
| Current authority logic | rich in-memory orchestration already exists in `authority.rs`, `bootstrap.rs`, `catalog.rs`, `lease.rs`, `registry.rs`, and related modules | Phase 1 should preserve and wrap this logic, not rewrite it |
| Current V2 `chain_id` support | no search hits under `prototype/gbn-bridge-proto/` | Phase 1 must establish `chain_id` at the authority API edge immediately |
| Current worktree risk | documentation reorganization under `docs/prototyping/` is still visible in the repo status | Phase 1 sign-off must rely on protected-path cleanliness and explicit phase evidence, not on a globally clean tree assumption |

---

## 2. Review Summary

Phase 1 is the first real implementation phase in GBN-PROTO-006. If it is underspecified, later phases will either:

- keep using in-process publisher calls and defeat the whole plan, or
- introduce an ad hoc service boundary that later persistence, control-session, and deployment work must undo

The main gaps the detailed Phase 1 plan must close are:

| Gap | Why It Matters | Resolution For Phase 1 |
|---|---|---|
| No real network service boundary | the publisher is still just a library wrapper | define and implement a real authority API listener now |
| No API contract shape | later clients could integrate against inconsistent request/response patterns | lock route families, request envelopes, response envelopes, and error model |
| No request authentication model | a network service without auth would be a fake cutover | define signed request-envelope verification at the API edge |
| No `chain_id` plan at the API edge | trace continuity would remain absent from the first real service cutover | require `chain_id` in every creator/bootstrap-related API envelope now |
| Persistence temptation | it is easy to over-expand Phase 1 into a database phase | keep durable storage explicitly deferred to Phase 2 |
| Control-session temptation | it is easy to conflate publisher API with publisher-to-bridge command delivery | keep bridge control sessions explicitly deferred to Phase 3 |

Phase 1 should produce a real service boundary, but it should not become a full production cut in one jump.

---

## 3. Scope Lock

### In Scope

- implement a real networked authority API in `gbn-bridge-publisher`
- add real publisher configuration parsing for bind address, auth settings, and basic service mode
- add authenticated request-envelope handling for:
  - bridge registration
  - bridge heartbeat / lease renewal
  - creator catalog refresh
  - host-creator bootstrap join requests
  - progress reporting
- add structured success and error responses
- add `chain_id` at the API boundary for request/response correlation
- update the `bridge-publisher` binary to launch the real service
- add local network integration tests for the authority service

### Out Of Scope

- durable storage backends
- bridge control sessions and push command delivery
- creator/runtime-side client replacement
- full bootstrap fanout orchestration over real bridge sessions
- receiver / ACK data path
- AWS deployment promotion
- modifying `prototype/gbn-proto/**`
- modifying the main repo `README.md`

---

## 4. Preflight Gates

Phase 1 should not begin code edits until all of these are checked:

1. Confirm the Phase 0 inventory artifacts exist:
   - `GBN-PROTO-006-Conduit-Simulation-Baseline.md`
   - `GBN-PROTO-006-Conduit-Gap-Inventory.md`
2. Confirm protected V1 paths are clean in the local worktree.
3. Confirm the current Phase 1 code cut will stay inside:
   - `gbn-bridge-publisher`
   - narrow `gbn-bridge-cli` publisher entrypoint changes
   - V2-local tests
4. Confirm Phase 1 is introducing a real authority API only, not persistence or bridge control sessions.
5. Confirm `chain_id` will be added at the authority API edge even if later phases extend it deeper.
6. Confirm `README.md` remains out of scope.
7. Confirm a temporary `--target-dir` fallback is available if the OneDrive-backed V2 `target/` path still fails.

If any gate fails, Phase 1 should stop.

Phase 1 execution result:

- Phase 0 deliverable docs exist
- protected V1 paths validated clean
- V1 minimum regression suite passed
- V2 workspace sanity suite passed using temp target fallback
- `README.md` remains out of scope
- real HTTP/JSON authority routes implemented for:
  - `GET /healthz`
  - `GET /readyz`
  - `POST /v1/bridge/register`
  - `POST /v1/bridge/heartbeat`
  - `POST /v1/bridge/progress`
  - `POST /v1/creator/catalog`
  - `POST /v1/bootstrap/join`
- the `bridge-publisher` CLI binary now launches the real authority API instead of the placeholder deployment loop
- signed request envelopes and `chain_id` continuity are now enforced at the authority API edge

---

## 5. Service Boundary Decisions To Lock In Phase 1

### 5.1 Transport Choice

Phase 1 should implement the authority API as a real HTTP/JSON service.

Reason:

- the master plan already reserves `http.rs`
- bridge control sessions are a later phase and should not be conflated with the first authority cutover
- HTTP/JSON is sufficient to establish a real service boundary, local network tests, auth, structured errors, and `chain_id` propagation

Do not attempt to introduce gRPC in Phase 1 unless a concrete blocker is discovered. That would create unnecessary churn before the service boundary itself is stable.

### 5.2 Route Families

Phase 1 should expose these minimum route families:

| Route | Purpose |
|---|---|
| `GET /healthz` | liveness check |
| `GET /readyz` | readiness check |
| `POST /v1/bridge/register` | bridge registration |
| `POST /v1/bridge/heartbeat` | bridge lease renewal / liveness update |
| `POST /v1/bridge/progress` | bridge-side bootstrap or session progress reporting |
| `POST /v1/creator/catalog` | returning creator catalog refresh |
| `POST /v1/bootstrap/join` | host-creator relayed first-contact join request |

If extra routes are added, they must remain Phase 1-scoped and not drift into Phase 2 or Phase 3 concerns.

### 5.3 Envelope Choice

Phase 1 should not reopen the Conduit wire model in `gbn-bridge-protocol` unless a true defect is found. Instead, the real authority API should introduce V2-local API envelopes in `api.rs`.

Recommended envelope shape:

| Envelope | Required Fields |
|---|---|
| `AuthorityApiRequest<T>` | `chain_id`, `request_id`, `sent_at_ms`, `actor_id`, `body`, `auth` |
| `AuthorityApiResponse<T>` | `chain_id`, `request_id`, `served_at_ms`, `ok`, `body`, `error`, `publisher_sig` when needed |

This lets Phase 1 carry `chain_id` now without forcing a broad protocol rewrite before Phase 7.

### 5.4 Auth Model

Phase 1 must be a real authenticated service boundary.

Recommended Phase 1 auth approach:

- signed request envelopes using Ed25519
- actor identity bound to the request body
- timestamped requests with bounded skew validation
- replay-resistant `request_id`

Rules:

- bridge requests are signed by the bridge identity key
- host-creator requests are signed by the host-creator identity key
- creator refresh requests are signed by the creator identity key
- the publisher verifies envelope signatures before invoking authority logic

Do not treat a static bearer token as sufficient for this phase. That would be a shortcut and would create a throwaway auth model.

### 5.5 Error Model

Phase 1 should return structured errors with stable codes.

Minimum error categories:

- `bad_request`
- `unauthorized`
- `forbidden`
- `not_found`
- `conflict`
- `expired`
- `internal`

Errors should be JSON, not plain text.

### 5.6 ChainID Rule For Phase 1

Phase 1 must establish `chain_id` at the API boundary.

Rules:

- every creator-originated request must include `chain_id`
- every host-creator bootstrap join request must include `chain_id`
- every bridge progress report that belongs to a creator/bootstrap flow must include `chain_id`
- service logs for handled requests must emit the same `chain_id`
- tests must assert request/response continuity on `chain_id`

Bridge registration and heartbeat may use a generated `chain_id` if they are not associated with an upstream creator flow, but the field name must still be `chain_id`.

---

## 6. Module Ownership To Lock In Phase 1

Phase 1 should keep the publisher crate split like this:

| Module | Responsibility |
|---|---|
| `config.rs` | bind address, auth skew, request limits, service mode configuration |
| `api.rs` | request / response DTOs, envelope types, error serialization, `chain_id` fields |
| `auth.rs` | signed-envelope verification, replay checks, actor identity checks |
| `http.rs` | route registration, HTTP server bootstrap, JSON extraction/serialization |
| `service.rs` | thin application service layer that maps API requests to authority calls |
| `server.rs` | service composition and lifecycle surface; no longer just an authority wrapper |
| existing authority modules | remain the orchestration core and must not be rewritten into HTTP handlers |

This split matters. If HTTP handlers call authority code directly everywhere, Phase 2 and Phase 3 will become harder to evolve cleanly.

---

## 7. Dependency And Implementation Policy

Phase 1 requires a real async HTTP runtime, but the dependency expansion should stay controlled.

### Recommended Dependencies

- `tokio`
- `axum`
- `serde`
- `serde_json`
- `tracing`
- `uuid` or equivalent for request IDs only if justified

### Bias

- keep the request/response layer thin
- keep authority logic reusable in-process
- keep JSON DTOs separate from authority internals
- keep authentication deterministic and testable without external services

### Avoid In Phase 1

- database dependencies
- AWS SDK dependencies
- queue dependencies
- bridge control session dependencies
- runtime bridge / creator client refactors
- modifying `gbn-bridge-protocol` unless a genuine protocol defect blocks the API implementation

---

## 8. Evidence Capture Requirements

Phase 1 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or commit message |
| starting commit SHA | `git rev-parse HEAD` | phase notes or commit message |
| Phase 0 prerequisite status | current Phase 0 docs | phase notes |
| pre-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |
| chosen HTTP stack | committed manifests and imports | phase notes |
| chosen auth model | `auth.rs` and tests | phase notes |
| route inventory | `http.rs` / tests | phase notes |
| `chain_id` envelope fields | `api.rs` / tests | phase notes |
| validation command set used | local command log | phase notes |
| temp `--target-dir` workaround, if needed | local command log | phase notes |
| post-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |

Do not sign off Phase 1 with only "publisher now has an API." Record the route inventory, auth model, and `chain_id` behavior explicitly.

---

## 9. Recommended Execution Order

Implement Phase 1 in this order:

1. Capture the starting branch, commit SHA, and protected-path diff state.
2. Add the Phase 1 dependencies to `gbn-bridge-publisher`.
3. Implement `config.rs` and `api.rs` first so service shape is locked early.
4. Implement `auth.rs` so request verification semantics are fixed before handlers exist.
5. Implement `service.rs` as the adapter from API requests into existing authority logic.
6. Implement `http.rs` and real `server.rs` composition.
7. Replace the publisher placeholder binary behavior with a real `bridge-publisher` service entrypoint.
8. Add `tests/api_flow.rs` with local network integration coverage.
9. Run the V2 workspace sanity suite.
10. Run the V1 preservation checks and minimum V1 regressions.

This order keeps the boundary decisions stable before the server starts to sprawl.

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
$target = Join-Path $env:LOCALAPPDATA 'Temp\\veritas-bridge-target-proto006-phase1'
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

Recommended Phase 1-specific checks:

```bash
rg -n "chain_id" prototype/gbn-bridge-proto/crates/gbn-bridge-publisher
```

```bash
git status --short
```

Expected outcome:

- V2 publisher crate compiles with a real HTTP service stack
- the publisher binary starts a real service instead of a placeholder loop
- local network integration tests pass
- `chain_id` is present in Phase 1 authority API envelopes and tests
- protected V1 paths show no drift
- minimum V1 regression suite remains green

---

## 11. Acceptance Criteria

Phase 1 is complete when:

- `gbn-bridge-publisher` exposes a real networked authority API
- `bridge-publisher` launches the real service rather than the placeholder loop
- the API supports:
  - bridge registration
  - heartbeat / renewal
  - creator catalog refresh
  - host-creator bootstrap join
  - progress reporting
- request authentication is enforced at the API edge
- structured JSON errors are returned
- `chain_id` exists and is preserved for creator/bootstrap-related request paths
- all required V1 and V2 validation commands have been run and recorded

Phase 1 is not complete if:

- the production path still depends on `InProcessPublisherClient`
- the binary still runs the placeholder service loop
- auth is replaced with a throwaway bearer token shortcut

---

## 12. Risks And Blockers

| Risk | Why It Matters | Mitigation |
|---|---|---|
| Phase 1 tries to solve persistence too early | it would blur the boundary between service cutover and storage cutover | keep persistence strictly deferred to Phase 2 |
| Phase 1 mutates the core protocol crate casually | it would destabilize later phases before the real API is even proven | prefer API envelopes in `api.rs` and only touch `gbn-bridge-protocol` for genuine defects |
| Auth model is weakened to speed local tests | later phases would inherit an insecure boundary | require signed envelopes even in local tests |
| `chain_id` is added only to logs, not to request envelopes | trace continuity would still be weak across service boundaries | require `chain_id` in request/response DTOs and tests |
| Placeholder deployment code remains active | the real service could exist in code while images still run the placeholder loop | make the publisher binary itself part of the acceptance criteria |

---

## 13. Sign-Off Recommendation

The correct Phase 1 sign-off is:

- the Publisher is now a real authenticated authority API service
- the existing Conduit authority logic sits behind that service cleanly
- `chain_id` exists at the first real Conduit service boundary

The correct Phase 1 sign-off is not:

- a database-backed publisher
- a bridge control dispatcher
- a fully deployed AWS service
- a complete replacement of all in-process Conduit clients
