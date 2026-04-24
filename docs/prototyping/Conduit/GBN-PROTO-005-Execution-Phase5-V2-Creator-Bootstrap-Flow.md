# GBN-PROTO-005 - Execution Phase 5 Detailed Plan: Creator Bootstrap Flow

**Status:** Implemented and validated locally from the committed Phase 3 and Phase 4 Conduit baseline; commit pending
**Primary Goal:** implement the creator-side Conduit bootstrap path in `gbn-bridge-runtime` without mutating the frozen V1 Lattice creator flow or letting Phase 5 sprawl into the Phase 6 data path
**Source Plan:** [GBN-PROTO-005 Execution Plan](GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign-Execution-Plan.md)
**Phase 0 Baseline Release:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)
**Protocol Baseline:** [GBN-ARCH-002-Bridge-Protocol-V2](../architecture/GBN-ARCH-002-Bridge-Protocol-V2.md)

---

## 1. Current Repo Findings

These findings should drive Phase 5 execution instead of being rediscovered during implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 5 notes should capture the branch used for creator bootstrap work |
| Current HEAD commit | `baa17ea8c35a444ca6665de424d489883559470a` | this is the committed Phase 3 and Phase 4 baseline that Phase 5 now builds on |
| Phase 0 baseline release | `veritas-lattice-0.1.0-baseline` published | V1 creator and onion behavior remain the preservation reference point |
| Current protocol crate state | canonical M1 Conduit wire model is committed | Phase 5 should consume protocol types as fixed inputs, not redesign them casually |
| Current publisher/runtime state | Phase 3 publisher authority and Phase 4 ExitBridge runtime are committed and validated | Phase 5 now builds on a stable authority/runtime baseline instead of a moving local stack |
| Current runtime crate state | `gbn-bridge-runtime` now contains creator bootstrap, cache, selector, HostCreator, and fanout modules in addition to the Phase 4 bridge runtime | Phase 5 is now an implemented creator/bootstrap surface rather than a planned extension |
| Current CLI state | `exit-bridge`, `creator-client`, and `host-creator` entrypoints now exist locally, together with example creator config files | Phase 5 delivered the planned creator-facing stubs without touching the main repo README |
| Current protected V1 path drift | none | V1 preservation is still a hard sign-off gate |
| Current validation environment risk | OneDrive-backed V2 `target/` writes still fail with Windows `os error 5` during `cargo test --workspace` | Phase 5 validation should expect the temp `--target-dir` fallback again |

---

## 2. Review Summary

Phase 5 is where Conduit starts behaving like an actual creator transport instead of only a publisher/bridge prototype.

The master plan is directionally correct, but a robust Phase 5 implementation needs stronger constraints:

| Gap | Why It Matters | Resolution For Phase 5 |
|---|---|---|
| Returning-vs-first-contact ambiguity | cached reconnect and HostCreator-assisted first contact have different trust and failure rules | treat them as separate flows sharing validation and cache structures, not as one generic bootstrap function |
| Trust-root drift | creator code can quietly accept stale or unsigned bridge data if validation is not centralized | make trust-root loading and publisher-signature checks a first-class module boundary |
| Local DHT overreach | the local DHT can accidentally become an authority source instead of a cache | keep it as a signed-hint cache only; transport use still requires signature and expiry validation |
| HostCreator overreach | HostCreator can be accidentally turned into a trust authority | keep HostCreator as a transport sponsor only and require Publisher-signed seed/bridge-set data |
| Fanout/data-path bleed | upload scheduling and chunk reuse belong mostly to Phase 6 | Phase 5 should stop at bootstrap fanout initiation, tunnel ACKs, retry selection, and cache updates |
| Selector instability | creator-side bridge choice can become ad hoc if direct filtering and fallback policy are not pinned | lock a deterministic selector policy now and leave richer scoring for later phases |

---

## 3. Scope Lock

### In Scope

- implement creator trust-root loading for Publisher authority verification
- implement cached catalog load and validation for returning creators
- implement creator-side selection of direct, valid, non-expired bridges
- implement first-time bootstrap through a HostCreator transport path
- implement seed-bridge establishment, ACK handling, and signed bridge-set retrieval
- implement local DHT / discovery-cache updates from publisher-seeded bootstrap entries
- implement immediate creator-to-bridge UDP punch fanout initiation after catalog refresh or bootstrap
- implement creator retry to the next valid bridge when the first candidate fails
- add creator-bootstrap tests covering cache validation, first-contact flow, tunnel ACKs, and retry

### Out Of Scope

- publisher ingest / ACK data path
- multi-bridge chunk scheduling and payload fanout reuse logic beyond bootstrap initiation
- AWS deployment assets
- weak discovery integration
- V1 creator or upload-path modification
- updating the main repo `README.md`

---

## 4. Preflight Gates

Phase 5 should not begin code edits until all of these are checked:

1. Confirm the local Phase 3 and Phase 4 work has been committed, or explicitly record that Phase 5 is intentionally stacking on an uncommitted local baseline.
2. Confirm the protocol crate exports the committed Conduit bootstrap, catalog, and punch message types unchanged.
3. Confirm protected V1 paths are clean in the local worktree.
4. Confirm Phase 5 stays inside `gbn-bridge-runtime`, creator-facing CLI binaries, config examples, and V2-local tests.
5. Confirm HostCreator remains a transport sponsor and not a trust authority.
6. Confirm direct-only filtering remains the creator rule for first-contact and refresh bridge selection.
7. Confirm the Phase 5 validation command shape if the default V2 `target/` path remains blocked by OneDrive.
8. Confirm Phase 5 will not modify the main repo `README.md`; any Conduit README rewrite remains deferred until V2 code work is complete.

Current blocker:

- none; Phase 5 is implemented locally and only needs commit/sign-off to close

---

## 5. Creator Decisions To Lock In Phase 5

### 5.1 Runtime Module Boundary

Phase 5 should keep the creator bootstrap surface structured like this:

| Module | Responsibility |
|---|---|
| `creator.rs` | top-level creator orchestration API for refresh, bootstrap, retry, and cache update |
| `catalog_cache.rs` | load, validate, store, and replace signed catalog data |
| `host_creator.rs` | transport-sponsor boundary for first-contact join forwarding only |
| `local_dht.rs` | creator-local signed bootstrap-entry and descriptor cache, not an authority source |
| `selector.rs` | deterministic filtering and fallback ordering over valid direct bridges |
| `bootstrap.rs` | first-contact flow composition across HostCreator, seed bridge, and bridge-set retrieval |
| `punch_fanout.rs` | immediate post-refresh / post-bootstrap fanout initiation and tunnel-ACK tracking |

Do not let `creator.rs` become a dump file for all of Phase 5 behavior.

### 5.2 Trust Model

Phase 5 should lock these behaviors:

- creator loads Publisher trust root before using any cached or bootstrap-provided bridge data
- cached descriptors must have valid Publisher signatures and unexpired lease state
- bootstrap entries must have valid Publisher signatures and unexpired entry windows
- HostCreator may transport bootstrap messages but cannot rewrite Publisher-selected bridge or bridge-set data
- local DHT entries are transport hints only and must not bypass authority validation

### 5.3 Returning-Creator Refresh Rules

Returning-creator logic in Phase 5 should:

- load cached signed bridge descriptors
- reject expired or unsigned entries
- filter to `reachability_class=direct`
- connect to one valid bridge
- request a fresh catalog through the connected bridge
- update local cache / local DHT from the fresh signed bridge entries
- immediately trigger creator-side punch fanout toward those bridges

### 5.4 First-Time Bootstrap Rules

First-contact bootstrap in Phase 5 should enforce:

- NewCreator reaches the Publisher only through HostCreator plus a working bridge path
- HostCreator forwards the join request but does not choose the bridge set
- creator validates the Publisher-signed seed-bridge response before tunneling
- creator and seed bridge ACK successful bidirectional tunnel establishment
- creator requests the signed bridge set from the seed bridge
- creator stores the returned Publisher-signed entries locally before fanout begins

### 5.5 Selector And Retry Rules

Phase 5 should keep creator selection deterministic:

- prefer valid direct bridges with the freshest lease expiry
- skip bridges with invalid signatures or expired leases
- retry the next valid bridge after a failed connect / failed ACK event
- keep retry rules deterministic in tests; do not introduce probabilistic scoring yet

### 5.6 Fanout Boundaries

Phase 5 should stop at bootstrap fanout initiation:

- start creator-side UDP probe attempts toward newly assigned bridges
- track which tunnels have ACKed successfully
- record that those bridges are active in the local DHT / discovery cache
- leave payload chunk scheduling and bridge reuse for actual upload to Phase 6

---

## 6. Dependency And Implementation Policy

Phase 5 should keep dependencies minimal and bias toward deterministic in-process tests.

### Required Bias

- depend on `gbn-bridge-protocol`, the local ExitBridge runtime surface, and the local publisher authority surface only where necessary
- structure time-sensitive logic so tests inject timestamps instead of sleeping
- keep HostCreator and creator bootstrap flows callable in-process from tests
- prefer `std` collections and in-memory caches unless a dependency is clearly justified

### Avoid In Phase 5

- adding socket, HTTP, or async runtime dependencies unless a compile boundary truly requires them
- embedding upload/data-path logic that belongs to Phase 6
- introducing persistent database dependencies for cache or DHT state
- mutating V1 creator circuit or upload logic

---

## 7. Evidence Capture Requirements

Phase 5 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or commit message |
| starting commit SHA | `git rev-parse HEAD` | phase notes or commit message |
| whether Phase 3+4 were committed first | local git state | phase notes |
| pre-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |
| trust-root and signature validation rules used | Phase 5 plan + tests | phase notes |
| direct-only selector rule used | Phase 5 plan + tests | phase notes |
| HostCreator trust boundary used | Phase 5 plan + tests | phase notes |
| validation command set used | local command log | phase notes |
| temp `--target-dir` workaround, if needed | local command log | phase notes |
| post-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |

---

## 8. Recommended Execution Order

Implement Phase 5 in this order:

1. Capture the starting branch, commit SHA, and protected-path diff state.
2. Add `catalog_cache.rs`, `local_dht.rs`, and `selector.rs` first so validation and selection rules are fixed early.
3. Add `creator.rs` and `host_creator.rs` as orchestration boundaries over those lower-level modules.
4. Implement `bootstrap.rs` around the committed Phase 4 ExitBridge runtime surface.
5. Implement `punch_fanout.rs` for immediate post-refresh / post-bootstrap fanout and ACK tracking.
6. Add creator-facing CLI binaries and config examples.
7. Add `tests/creator_bootstrap.rs`.
8. Run the V2 validation commands.
9. Run the V1 preservation checks and minimum V1 regressions.

This keeps the trust and cache rules stable before first-contact orchestration tries to compose them.

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
$target = Join-Path $env:LOCALAPPDATA 'Temp\veritas-bridge-target-phase5'
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

- creator bootstrap modules build and tests
- expired and invalid signed data are rejected
- returning creator can reconnect from cached descriptors
- first-time creator can bootstrap through HostCreator and seed bridge
- creator stores Publisher-signed bridge entries locally
- creator retries the next valid bridge when the first one fails
- protected V1 diff remains empty
- minimum V1 regression suite still passes

---

## 10. Acceptance Criteria

Phase 5 is complete only when all of the following are true:

- every file listed in the main execution plan exists
- creator trust-root loading is implemented
- cached catalog load, validation, and update flow are implemented
- creator selection filters direct, valid, non-expired bridges deterministically
- HostCreator-assisted first-time join flow is implemented without elevating HostCreator into an authority role
- seed-bridge establishment, ACK handling, and bridge-set retrieval are implemented
- local DHT / discovery state updates from Publisher-signed entries are implemented
- immediate creator-to-bridge punch fanout is initiated after refresh or bootstrap
- creator retry to the next valid bridge is implemented
- creator-bootstrap tests cover expiry filtering, invalid signatures, cached reconnect, first-time bootstrap, tunnel ACKs, and retry
- protected V1 diff is clean after validation
- minimum V1 regression suite still passes

---

## 11. Risks And Blockers

| Risk | What It Looks Like | Mitigation |
|---|---|---|
| Trust drift | creator uses unsigned or expired entries because validation is scattered | centralize validation in trust-root and cache modules |
| HostCreator authority creep | HostCreator rewrites or substitutes the Publisher-selected bridge set | lock this boundary in tests and notes |
| Cache corruption | local DHT or catalog cache stores transport-eligible garbage | require signature and expiry checks on load and update |
| Selector instability | retry behavior depends on incidental ordering | define deterministic direct-only selection rules now |
| Phase 6 bleed | upload scheduling and bridge reuse logic overrun Phase 5 | stop at fanout initiation and ACK tracking |
| OneDrive validation noise | V2 tests fail for filesystem reasons instead of code reasons | use and document the temp-target fallback |
| Procedural drift | Phase 5 starts before Phase 3/4 are actually committed | record the exact baseline or commit first |

Current blocker:

- none; Phase 5 is implemented locally and only needs commit/sign-off to close

---

## 12. First Implementation Cut

If Phase 5 is implemented as a single focused change set, use this breakdown:

1. Trust root, cache, local DHT, and selector
2. Creator and HostCreator orchestration
3. Seed-bridge bootstrap and bridge-set retrieval
4. Fanout initiation, retry, CLI entrypoints, and tests

That keeps Phase 5 auditable and gives Phase 6 a stable creator/bootstrap surface instead of a moving trust and cache contract.
