# GBN-PROTO-006 - Execution Phase 2 Detailed Plan: Durable Publisher Storage And Recovery

**Status:** Ready to start after Phase 1 real authority API service is implemented and validated  
**Primary Goal:** replace `gbn-bridge-publisher`'s in-memory production authority state with durable Postgres-backed storage, restart recovery, and production signing-key loading while preserving the Phase 1 service boundary and keeping bridge control sessions and deployment promotion deferred to later phases  
**Source Plan:** [GBN-PROTO-006 Execution Plan](GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md)  
**Protected V1 Baseline:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)  
**Phase 1 Detailed Plan:** [GBN-PROTO-006-Execution-Phase1-Real-Publisher-Authority-API-Service](GBN-PROTO-006-Execution-Phase1-Real-Publisher-Authority-API-Service.md)  
**Starting Conduit Baseline:** `2b6d5c5d24e269e96e3fdc820f3f90669607414a`

---

## 1. Current Repo Findings

These findings should drive Phase 2 instead of being rediscovered during implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 2 should record the commit used to begin the persistence cutover |
| Current HEAD commit | `2b6d5c5d24e269e96e3fdc820f3f90669607414a` | current committed Conduit baseline still uses simulation-era state handling |
| Current authority state model | `PublisherAuthority` owns `InMemoryAuthorityStorage` in [authority.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/authority.rs:1>) | proves production authority state is still process-local and non-durable |
| Current storage implementation | record structs plus sequence counters in [storage.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/storage.rs:1>) | gives Phase 2 a concrete migration target for durable tables and repositories |
| Current stored entities | bridges, bootstrap sessions, upload sessions, current batch window, transient sequence generators | these are the minimum entities that must be persisted or explicitly superseded |
| Current signing model | in-process `SigningKey` owned by `PublisherAuthority` | production mode needs a real signing-key loading abstraction, not a hardcoded in-memory key only |
| Current publisher service boundary | still being introduced in Phase 1 | Phase 2 must preserve the Phase 1 API contract while changing the backing state model |
| Current bridge/runtime coupling | bridge control sessions and runtime network clients are still future work | Phase 2 must not accidentally pull Phase 3 or Phase 4 concerns into persistence |
| Current V2 `chain_id` state | absent from Conduit persistence because Conduit persistence does not exist yet | Phase 2 must establish durable `chain_id` storage on all creator/bootstrap/session records |
| Current environment risk | local V2 workspace writes may still hit Windows OneDrive `os error 5` and local DB tests may depend on Docker/WSL availability | Phase 2 validation must plan for both temp target dir fallback and explicit database-test prerequisites |

---

## 2. Review Summary

Phase 2 is where the Publisher stops being restart-fragile. If this phase is weak, every later phase will be built on a service that still loses authority state on process exit.

The main gaps the detailed Phase 2 plan must close are:

| Gap | Why It Matters | Resolution For Phase 2 |
|---|---|---|
| In-memory-only authority state | bridge registry, leases, bootstrap sessions, and upload sessions disappear on restart | migrate the production path to durable Postgres-backed repositories |
| No schema boundary | ad hoc persistence would hard-code authority internals directly into SQL | define explicit schema ownership and repository boundaries |
| No recovery logic | a restarted Publisher would forget in-flight batches and bootstrap sessions | add replay-safe restart recovery and startup reconciliation |
| No persisted correlation identifiers | `chain_id` would still disappear across restarts or postmortems | persist `chain_id` on bootstrap, progress, and upload-session records |
| In-process signing-key ownership | production authority would still depend on local process memory for the private key | add a production signing-key loader abstraction, with KMS-oriented design even if local mode remains available |
| Overreach risk | it is easy to conflate durable storage with distributed control-session delivery | keep Phase 2 strictly focused on persistence, recovery, and signing-key loading |

Phase 2 should make the authority service durable, but it should not yet try to become the bridge command dispatcher or the full AWS deployment cut.

---

## 3. Scope Lock

### In Scope

- replace the production-path `InMemoryAuthorityStorage` dependency with a durable Postgres-backed storage layer
- define durable schema and migration ownership for:
  - bridge registry
  - leases
  - signed catalog issuance records
  - bootstrap sessions
  - bridge assignments
  - progress events
  - upload sessions
- add restart recovery and startup reconciliation logic
- add signing-key loading abstraction for production mode
- persist `chain_id` on creator/bootstrap/session-related records
- add persistence and recovery integration tests

### Out Of Scope

- bridge control sessions and push command delivery
- creator/runtime-side client replacement
- full bootstrap fanout over bridge control channels
- receiver / ACK service implementation
- AWS deployment promotion
- modifying `prototype/gbn-proto/**`
- modifying the main repo `README.md`

---

## 4. Preflight Gates

Phase 2 should not begin code edits until all of these are checked:

1. Confirm the Phase 0 inventory deliverables exist.
2. Confirm Phase 1 is implemented and validated, and the authority API contract is stable enough to preserve through the persistence cutover.
3. Confirm protected V1 paths are clean in the local worktree.
4. Confirm Phase 2 will keep Postgres as the default production store and will not reopen that database decision casually.
5. Confirm `chain_id` persistence is treated as mandatory, not optional.
6. Confirm a local Postgres-backed test strategy exists:
   - Docker-based local database, or
   - WSL/local Postgres instance, or
   - CI-only Postgres validation plus clearly documented local limitations
7. Confirm signing-key loading will move behind an abstraction in this phase, even if local dev still supports file-based keys.
8. Confirm `README.md` remains out of scope.

If any gate fails, Phase 2 should stop.

Current blocker:

- Phase 1 is not yet implemented, so Phase 2 remains planning-ready only

---

## 5. Storage And Recovery Decisions To Lock In Phase 2

### 5.1 Production Store Choice

Phase 2 should treat Postgres as the canonical production authority store.

Reason:

- the publisher owns multi-entity transactions:
  - bridge registration + lease issuance
  - bootstrap session creation + assignment creation
  - upload session open + frame ingest + close
- these flows benefit from transactional consistency and SQL queryability
- restart recovery and audit reconstruction are simpler with relational state than with scattered key-value records

Aurora/Postgres remains the intended AWS deployment target later, but Phase 2 should build against standard Postgres interfaces, not Aurora-specific APIs.

### 5.2 Repository Boundary

Phase 2 should not embed SQL directly into `authority.rs`.

Recommended split:

| Module | Responsibility |
|---|---|
| `storage.rs` | storage traits, repository interfaces, common persistence DTOs |
| `storage/schema.rs` | schema description, migration ownership, table naming conventions |
| `storage/postgres.rs` | concrete Postgres implementation of storage traits |
| `storage/recovery.rs` | restart reconciliation, stale-session cleanup, and rehydration logic |
| `signing/kms.rs` | production signing-key loading abstraction and provider surface |

The authority layer should depend on repository traits, not on Postgres details directly.

### 5.3 Entity Boundary

These state domains must become durable in Phase 2:

| Domain | Minimum Durable Fields |
|---|---|
| `bridges` | `bridge_id`, identity, ingress endpoints, assigned UDP port, reachability class, capabilities, revocation state |
| `leases` | `lease_id`, `bridge_id`, issue time, expiry, authoritative signed lease payload or reconstructable equivalent |
| `catalog_issuance` | catalog id/version, issued-at, expiry, participating bridge set, signing metadata |
| `bootstrap_sessions` | session id, creator entry, host creator id, relay bridge id, seed bridge id, assigned bridge set, created-at, expiry, `chain_id` |
| `bootstrap_assignments` | assignment id, bootstrap session id, target bridge id, assignment type, delivery state, last update time, `chain_id` |
| `progress_events` | session/assignment link, actor id, event type, event time, structured payload, `chain_id` |
| `upload_sessions` | session id, creator id, creator session key reference, expected chunks, open/close state, `chain_id` |
| `ingested_frames` | session id, sequence, frame id, via bridge id, receive time, dedupe metadata, `chain_id` where available |
| `batch_windows` | batch id, window open/close, queued assignment ids, rollover state |

### 5.4 Sequence And Identifier Rules

The current in-memory sequence generators in [storage.rs](</c:/Users/fahd_/OneDrive/Documents/Global Broadcast Network/prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/storage.rs:1>) are not sufficient for production mode.

Phase 2 should decide one of these patterns and use it consistently:

- database-generated monotonically increasing ids for internal rows, plus stable public ids for external objects
- application-generated UUID/ULID-style ids for public objects

Recommended:

- use stable application-generated public ids for:
  - lease ids
  - bootstrap session ids
  - batch ids
  - upload session ids
- use database primary keys only for internal indexing

Do not rely on process-local incrementing counters in the production path after Phase 2.

### 5.5 Recovery Rules

On restart, the Publisher must be able to:

- load active bridges and lease state
- identify expired leases and mark them inactive
- restore or expire in-flight bootstrap sessions
- restore current upload session state
- identify incomplete batch windows
- preserve auditability of progress events

Phase 2 should implement explicit startup reconciliation, not implicit "load everything and hope" behavior.

### 5.6 Signing-Key Rules

Phase 2 must move signing-key handling behind a production-ready abstraction.

Required modes:

- local dev/test mode:
  - file or generated signing key
- production mode:
  - KMS-oriented abstraction

This does not require live AWS KMS integration tests in Phase 2, but it does require:

- a clean trait boundary
- deterministic tests
- configuration surface that later deployment phases can wire up

### 5.7 ChainID Rules For Phase 2

Phase 2 must persist `chain_id` on every durable record that participates in distributed correlation.

Minimum requirement:

- `bootstrap_sessions.chain_id`
- `bootstrap_assignments.chain_id`
- `progress_events.chain_id`
- `upload_sessions.chain_id`
- `ingested_frames.chain_id` when the frame/session path exposes it

If a record participates in recovery, diagnostics, or cross-node correlation, `chain_id` must survive restart with it.

---

## 6. Schema Ownership And Migration Policy

Phase 2 should define migration ownership explicitly.

Recommended approach:

- maintain a V2-local schema module in `storage/schema.rs`
- define migration files or migration descriptors under a V2-local path if needed later
- keep table naming scoped to the Conduit publisher domain

Recommended naming bias:

- `conduit_bridges`
- `conduit_bridge_leases`
- `conduit_catalog_issuance`
- `conduit_bootstrap_sessions`
- `conduit_bootstrap_assignments`
- `conduit_progress_events`
- `conduit_upload_sessions`
- `conduit_ingested_frames`
- `conduit_batch_windows`

Do not use generic table names that would later collide with V1 or unrelated repo services.

---

## 7. Module Ownership To Lock In Phase 2

Phase 2 should keep the publisher crate split like this:

| Module | Responsibility |
|---|---|
| `storage.rs` | storage traits and storage-facing types |
| `storage/postgres.rs` | Postgres repository implementation |
| `storage/schema.rs` | schema definitions and migration ownership |
| `storage/recovery.rs` | restart rehydration and reconciliation logic |
| `signing/kms.rs` | production signing provider abstraction |
| `authority.rs` | orchestrates through storage traits; must not become a SQL-heavy module |
| `catalog.rs` | catalog issuance logic that persists issuance records via repositories |
| `bootstrap.rs` | bootstrap session logic that persists session and assignment state via repositories |

If SQL starts spreading through business-logic modules, Phase 2 has gone off track.

---

## 8. Dependency And Implementation Policy

Phase 2 requires real storage dependencies, but the dependency expansion should stay controlled.

### Recommended Dependencies

- a Postgres driver and/or query layer
- migration support
- serialization types already used by the publisher and protocol crates
- `tokio` if the chosen DB library requires async runtime integration

### Bias

- prefer explicit repository methods over generic "store blob" persistence
- prefer transactional write boundaries where authority decisions span multiple entities
- prefer typed persistence DTOs over ad hoc JSON columns for core fields

### Avoid In Phase 2

- introducing queueing or pub/sub dependencies
- introducing bridge control-session runtime dependencies
- introducing AWS deployment SDK coupling
- making KMS mandatory for local tests
- postponing `chain_id` persistence to a later "observability" phase

---

## 9. Evidence Capture Requirements

Phase 2 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or commit message |
| starting commit SHA | `git rev-parse HEAD` | phase notes or commit message |
| Phase 1 prerequisite status | Phase 1 implementation and validation notes | phase notes |
| pre-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |
| chosen Postgres library / migration approach | manifests and storage module docs | phase notes |
| schema inventory | `storage/schema.rs` and tests | phase notes |
| recovery behavior | `storage/recovery.rs` and tests | phase notes |
| signing provider shape | `signing/kms.rs` and tests | phase notes |
| `chain_id` persistence fields | schema and tests | phase notes |
| validation command set used | local command log | phase notes |
| database test prerequisites used | Docker/WSL/local Postgres notes | phase notes |
| temp `--target-dir` workaround, if needed | local command log | phase notes |
| post-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |

Do not sign off Phase 2 with only "Postgres added." Record the schema inventory, recovery behavior, and `chain_id` persistence decisions explicitly.

---

## 10. Recommended Execution Order

Implement Phase 2 in this order:

1. Capture the starting branch, commit SHA, and protected-path diff state.
2. Define storage traits and durable entity boundaries in `storage.rs`.
3. Define the schema and migration ownership in `storage/schema.rs`.
4. Implement the Postgres repository layer in `storage/postgres.rs`.
5. Refactor authority logic to depend on storage traits rather than `InMemoryAuthorityStorage` directly.
6. Implement recovery and startup reconciliation logic in `storage/recovery.rs`.
7. Add signing-key provider abstraction in `signing/kms.rs`.
8. Add persistence and restart recovery integration tests.
9. Run the V2 workspace sanity suite and storage-specific tests.
10. Run the V1 preservation checks and minimum V1 regressions.

This order keeps the storage contract stable before the authority layer is rewired to depend on it.

---

## 11. Validation Commands

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
$target = Join-Path $env:LOCALAPPDATA 'Temp\\veritas-bridge-target-proto006-phase2'
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

Recommended Phase 2-specific checks:

```bash
rg -n "chain_id" prototype/gbn-bridge-proto/crates/gbn-bridge-publisher
```

```bash
rg -n "InMemoryAuthorityStorage" prototype/gbn-bridge-proto/crates/gbn-bridge-publisher
```

```bash
git status --short
```

Expected outcome:

- the production publisher path uses durable storage abstractions rather than in-memory-only state
- persistence and recovery tests pass
- `chain_id` is present in durable authority records where required
- protected V1 paths show no drift
- minimum V1 regression suite remains green

If local Postgres-backed tests require Docker or WSL services, record that prerequisite explicitly. Do not silently downgrade Phase 2 validation to unit tests only.

---

## 12. Acceptance Criteria

Phase 2 is complete when:

- the production authority path no longer depends on `InMemoryAuthorityStorage` as its primary store
- Postgres-backed repositories exist for the required authority domains
- restart recovery and startup reconciliation logic exist and are tested
- signing-key loading is behind a production-ready abstraction
- `chain_id` is persisted on bootstrap, progress, and session records
- all required V1 and V2 validation commands have been run and recorded

Phase 2 is not complete if:

- durable storage is added only as an optional side path while the production path still defaults to in-memory state
- recovery behavior is undocumented or untested
- `chain_id` persistence is deferred

---

## 13. Risks And Blockers

| Risk | Why It Matters | Mitigation |
|---|---|---|
| SQL bleeds directly into business logic | later phases would struggle to evolve authority policy cleanly | keep repository traits and DTOs explicit, and keep `authority.rs` storage-agnostic |
| Recovery is treated as "reload rows" only | stale leases and abandoned sessions would survive restart incorrectly | implement explicit reconciliation rules and test them |
| Local DB tests are skipped because Docker is inconvenient | durability claims would become weak and untrustworthy | require an explicit database-backed test path and document any environment prerequisites |
| KMS abstraction is postponed | production signing would remain glued to local in-memory keys | add the abstraction now even if live KMS wiring is later |
| `chain_id` persistence is forgotten on secondary entities | trace continuity would still break at restart or forensic time | make `chain_id` a schema-level requirement for correlated entities |

---

## 14. Sign-Off Recommendation

The correct Phase 2 sign-off is:

- the Publisher now has durable authority state
- restart recovery is real and tested
- signing-key loading has a production-ready abstraction
- `chain_id` survives persistence boundaries

The correct Phase 2 sign-off is not:

- bridge command delivery
- creator network-client replacement
- full AWS deployment readiness
- a local-only in-memory authority service with optional Postgres experiments
