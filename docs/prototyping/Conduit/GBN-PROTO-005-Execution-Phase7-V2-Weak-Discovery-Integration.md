# GBN-PROTO-005 - Execution Phase 7 Detailed Plan: Weak Discovery Integration

**Status:** Completed from the committed Phase 6 bridge-mode data-path baseline
**Primary Goal:** add a V2-local weak-discovery layer that can surface candidate bridge hints without granting transport trust, mutating the frozen V1 discovery surfaces, or pulling Phase 8 reachability policy forward prematurely
**Source Plan:** [GBN-PROTO-005 Execution Plan](GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign-Execution-Plan.md)
**Phase 0 Baseline Release:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)
**Protocol Baseline:** [GBN-ARCH-002-Bridge-Protocol-V2](../architecture/GBN-ARCH-002-Bridge-Protocol-V2.md)

---

## 1. Current Repo Findings

These findings should drive Phase 7 execution instead of being rediscovered during implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 7 notes should capture the mainline commit used to begin weak-discovery work |
| Current HEAD commit | `8a6efc9c319faac533582542a02b57c966a38d1e` | this is the committed Phase 6 bridge-mode data-path baseline that Phase 7 now builds on |
| Phase 0 baseline release | `veritas-lattice-0.1.0-baseline` published | V1 Lattice remains the frozen reference point for discovery and transport preservation checks |
| Current protocol crate state | Phase 2 wire types are committed and stable | Phase 7 should avoid reopening the Phase 2 wire model unless a strictly local runtime type is insufficient |
| Current runtime state | creator bootstrap, local DHT cache, selector, and bridge-mode upload path are committed | Phase 7 should integrate with those committed surfaces instead of bypassing them |
| Current authority state | publisher catalogs and bootstrap payloads are already signed and authoritative | weak discovery must remain below publisher authority in the merge order |
| Current architecture rule | V2 docs already say weak discovery is non-authoritative and cannot elevate trust | the implementation must preserve that rule explicitly in code and tests |
| Current missing V2 surface | there is no dedicated discovery source, seed catalog, or hint-merge layer in `gbn-bridge-runtime` yet | Phase 7 should add those boundaries cleanly instead of spreading discovery logic across existing creator modules |
| Current protected V1 path drift | none expected at phase start | V1 preservation remains a hard sign-off gate |
| Current validation environment risk | OneDrive-backed V2 `target/` writes still fail with Windows `os error 5` during `cargo test --workspace` | Phase 7 validation should expect the temp `--target-dir` fallback again |

---

## 2. Review Summary

The master plan correctly frames Phase 7 as hint-only discovery, but it is still underspecified in three places that matter to implementation quality:

| Gap | Why It Matters | Resolution For Phase 7 |
|---|---|---|
| Hint shape ambiguity | discovery can sprawl into ad hoc half-descriptors if hint fields are not bounded | lock a V2-local `DiscoveryHint` / `DiscoveryCandidate` surface that is explicitly weaker than a signed descriptor |
| Merge precedence ambiguity | creators can accidentally let weak hints override fresher signed state | make merge precedence deterministic: active publisher-seeded entries, then freshest signed catalog entries, then weak discovery hints |
| Phase 8 bleed risk | reachability classification and transport eligibility can be accidentally reimplemented in discovery code | keep Phase 7 trust decisions binary: signed and non-expired descriptors are transport-eligible; weak hints only trigger refresh / candidate lookup |

Phase 7 should improve discovery ergonomics without changing the trust model. That means the output of discovery is not “a usable bridge.” The output is “a candidate worth trying to refresh against publisher authority.”

---

## 3. Scope Lock

### In Scope

- add V2-local weak-discovery sources under `gbn-bridge-runtime`
- add a creator-local seed catalog or static seed source for discovery hints
- add deterministic hint merge logic across:
  - active publisher-seeded bootstrap entries
  - freshest publisher-signed catalog descriptors
  - weak discovery hints
- add creator refresh behavior that can use weak hints to obtain a later signed catalog
- add tests proving weak discovery cannot override signed publisher authority
- document the V2 weak-discovery design in a V2-local doc

### Out Of Scope

- modifying V1 DHT, gossip, or direct-validation logic
- changing V1 `NodeAnnounce`, `DirectNodeProbe`, or router-sim state machines
- changing Phase 2 wire messages unless a strictly runtime-local representation is impossible
- implementing Phase 8 reachability scoring, class transitions, or seed-bridge eligibility policy
- introducing production network services for discovery
- updating the main repo `README.md`

---

## 4. Preflight Gates

Phase 7 should not begin code edits until all of these are checked:

1. Confirm the committed Phase 6 bridge-mode data-path baseline is present and clean.
2. Confirm protected V1 paths are clean in the local worktree.
3. Confirm signed publisher catalogs and bootstrap entries remain the only transport-authoritative inputs.
4. Confirm Phase 7 stays inside `gbn-bridge-runtime`, V2-local tests, and V2-local docs unless a minimal V2-local support change is unavoidable.
5. Confirm Phase 7 does not reopen reachability classification or scoring policy that belongs to Phase 8.
6. Confirm weak discovery will not mark a bridge as active, trusted, or upload-eligible by itself.
7. Confirm the V2 validation command shape if the default V2 `target/` path remains blocked by OneDrive.
8. Confirm Phase 7 will not modify the main repo `README.md`; any Conduit README rewrite remains deferred until V2 code work is complete.

Current blocker:

- none; Phase 7 implementation is complete locally and the validation gates passed

---

## 5. Discovery Decisions To Lock In Phase 7

### 5.1 Runtime Module Boundary

Phase 7 should keep the weak-discovery surface structured like this:

| Module | Responsibility |
|---|---|
| `discovery.rs` | discovery-source orchestration, hint retrieval, and discovery refresh entrypoints |
| `seed_catalog.rs` | static seeds or preconfigured discovery endpoints used to start hint collection |
| `hint_merge.rs` | deterministic precedence, dedupe, staleness handling, and trust boundary enforcement |
| existing `catalog_cache.rs` | cache only signed catalog state; do not turn it into a generic hint store |
| existing `local_dht.rs` | creator-local store for publisher-seeded or signed bridge knowledge, plus any explicitly weak hint state kept separate from trusted entries |
| existing `selector.rs` | choose transport candidates only from trusted signed data, never directly from weak hints |

Do not let `creator.rs` become the place where discovery source logic, merge logic, and trust logic all collapse together.

### 5.2 Hint Trust Boundary

Phase 7 should lock these behaviors:

- weak discovery may suggest bridge IDs, IPs, ports, or candidate ingress hints
- weak discovery may seed an attempted catalog refresh path
- weak discovery may populate creator-local weak hint state
- weak discovery must not make a bridge transport-eligible by itself
- only publisher-signed, non-expired descriptors may become trusted bridge entries

### 5.3 Merge Precedence

Phase 7 should lock this deterministic precedence order:

1. active publisher-seeded bootstrap entries
2. freshest publisher-signed catalog descriptors
3. weak discovery hints

Additional rules:

- a weak hint cannot replace an active signed entry for the same bridge ID
- a stale weak hint cannot displace a fresher signed descriptor
- a weak hint for an unknown bridge may remain as a refresh candidate only
- if signed data exists and weak discovery is disabled, creator behavior should remain functional

### 5.4 Bootstrap Protection

Phase 7 should lock these behaviors:

- weak discovery cannot replace the publisher-chosen seed bridge for first-contact bootstrap
- weak discovery cannot replace the initial bridge set returned during bootstrap
- weak hints can only matter after bootstrap by helping a creator attempt later refresh or recovery
- host-creator and seed-bridge bootstrap trust remains fully rooted in publisher-signed material

### 5.5 Hint Shape

Phase 7 should prefer a deliberately weak V2-local representation, for example:

- `node_id` or candidate bridge ID
- host / ingress hint
- port or transport hint
- observed timestamp
- source kind
- optional freshness metadata

Avoid storing full transport-authoritative semantics in the hint type. If a complete `BridgeDescriptor` is needed, it should come from publisher-signed catalog material instead.

---

## 6. Dependency And Implementation Policy

Phase 7 should keep dependencies minimal and bias toward deterministic in-process tests.

### Required Bias

- reuse committed creator bootstrap, local DHT, catalog cache, and selector surfaces
- keep discovery source logic callable in-process from tests
- prefer static or deterministic seed data over network-bound discovery services
- represent weak hints with V2-local runtime types rather than reopening the protocol crate unless absolutely necessary

### Avoid In Phase 7

- adding async runtimes or network clients solely to simulate discovery
- mutating V1 DHT, gossip, or direct-validation code
- introducing discovery-derived transport authorization
- implementing reachability-class ranking or seed-bridge policy that belongs to Phase 8

---

## 7. Evidence Capture Requirements

Phase 7 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or commit message |
| starting commit SHA | `git rev-parse HEAD` | phase notes or commit message |
| pre-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |
| merge precedence used | Phase 7 plan + tests | phase notes |
| weak-hint trust boundary used | Phase 7 plan + tests | phase notes |
| bootstrap protection rule used | Phase 7 plan + tests | phase notes |
| validation command set used | local command log | phase notes |
| temp `--target-dir` workaround, if needed | local command log | phase notes |
| post-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |

---

## 8. Recommended Execution Order

Implement Phase 7 in this order:

1. Capture the starting branch, commit SHA, and protected-path diff state.
2. Add `seed_catalog.rs` and `discovery.rs` with deterministic weak-hint sources first.
3. Add `hint_merge.rs` and lock precedence rules before touching creator selection.
4. Integrate weak-hint storage into creator-local state without promoting trust.
5. Update creator refresh paths so weak hints can trigger signed catalog refresh attempts.
6. Add `tests/discovery.rs`.
7. Run the V2 validation commands.
8. Run the V1 preservation checks and minimum V1 regressions.

This keeps the trust model stable before creator behavior starts depending on discovered hints.

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
$target = Join-Path $env:LOCALAPPDATA 'Temp\veritas-bridge-target-phase7'
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

- discovery candidates without valid publisher signatures are never transport-eligible
- weak discovery cannot override active bootstrap entries for a new creator session
- weak discovery can seed a later successful signed catalog refresh
- stale weak hints do not displace fresher signed data
- creator still functions when discovery is disabled but cached signed catalog exists
- protected V1 diff remains empty
- minimum V1 regression suite still passes

Executed result:

- `cargo fmt --all --check --manifest-path prototype/gbn-bridge-proto/Cargo.toml` passed
- `cargo check --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml --target-dir %LOCALAPPDATA%\Temp\veritas-bridge-target-phase7` passed
- `cargo test --workspace --manifest-path prototype/gbn-bridge-proto/Cargo.toml --target-dir %LOCALAPPDATA%\Temp\veritas-bridge-target-phase7` passed
- `git diff --name-only -- <protected V1 paths>` returned no output
- `cargo check --workspace` passed in `prototype/gbn-proto`
- `cargo test -p mcn-router-sim` passed in `prototype/gbn-proto`

---

## 10. Acceptance Criteria

Phase 7 is complete only when all of the following are true:

- every file listed in the main execution plan exists
- weak-discovery sources are implemented in V2-local code
- deterministic hint merge precedence is implemented
- only publisher-signed descriptors are transport-eligible
- bootstrap protection rules prevent weak discovery from replacing publisher-selected seed or initial bridge state
- creator refresh logic can use weak hints to obtain later signed catalog state
- tests cover signature rejection, precedence, stale-hint handling, signed refresh recovery, and discovery-disabled fallback
- protected V1 diff is clean after validation
- minimum V1 regression suite still passes

---

## 11. Risks And Blockers

| Risk | What It Looks Like | Mitigation |
|---|---|---|
| Trust bleed | weak hints accidentally become transport-eligible | keep trust and transport selection rooted in signed descriptors only |
| Merge instability | hint ordering depends on incidental iteration order | centralize precedence and dedupe logic in `hint_merge.rs` |
| Bootstrap weakening | discovery overrides first-contact publisher choices | add explicit bootstrap-protection tests and code guards |
| Phase 8 bleed | discovery starts scoring or classifying reachability | defer reachability policy and keep Phase 7 binary about trust |
| Runtime sprawl | discovery logic gets smeared across creator modules | lock dedicated module ownership early |
| OneDrive validation noise | V2 tests fail for filesystem reasons instead of code reasons | use and document the temp-target fallback |

Current blocker:

- none; Phase 7 is implemented locally and validated

---

## 12. First Implementation Cut

If Phase 7 is implemented as a single focused change set, use this breakdown:

1. Discovery-source and seed-catalog scaffolding
2. Hint merge and precedence enforcement
3. Creator integration and refresh fallback behavior
4. Discovery-focused tests and V1 preservation validation

That keeps the weak-discovery surface auditable and prevents transport trust from getting entangled with hint collection.
