# GBN-PROTO-005 - Execution Phase 1 Detailed Plan: V2 Workspace Boundary

**Status:** Completed and validated from the published Lattice baseline
**Primary Goal:** create `prototype/gbn-bridge-proto/` as an isolated Rust workspace that compiles without modifying V1
**Source Plan:** [GBN-PROTO-005 Execution Plan](GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign-Execution-Plan.md)
**Phase 0 Baseline Release:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)
**Phase 0 Release Tag:** `veritas-lattice-0.1.0-baseline`

---

## 1. Current Repo Findings

These findings record the executed Phase 1 result state and should guide any later revisit of the workspace-boundary decision:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 1 was implemented on the mainline branch without reopening the V1 workspace |
| Current HEAD commit | `70aadd0ac4d75e63093954a7ccfaeda23f954702` | current repo tip after the committed Phase 1 scaffold and README stabilization work |
| Phase 0 baseline docs | present | Phase 1 now has the required freeze manifest and regression suite docs |
| Phase 0 release automation | `.github/workflows/release-phase0.yml` exists and has published `veritas-lattice-0.1.0-baseline` | the preferred Phase 1 prerequisite is now satisfied |
| Existing V2 workspace path | `prototype/gbn-bridge-proto/` exists | the full sibling workspace boundary created by Phase 1 is in place |
| Current protected V1 path drift | none | the protected-path diff remained clean through Phase 1 validation |

---

## 2. Review Summary

Phase 1 already had the right high-level goal, but it needed the same execution discipline as Phase 0. The main gaps were:

| Gap | Why It Matters | Resolution For Phase 1 |
|---|---|---|
| Phase 0 was treated as optional in practice | later V2 work would start without a frozen V1 reference point | make Phase 0 completion the preferred hard gate and allow waiver only as an explicit exception |
| No current-state awareness | the repo already has protected V1 drift that would muddy Phase 1 validation | treat protected-path cleanliness as a preflight requirement |
| No evidence capture list | Phase 1 could finish without a clear record of what was created and validated | define the exact evidence to capture in phase notes |
| Dependency policy was underdefined | blindly copying V1 workspace patterns would create unnecessary coupling | keep V2 manifests minimal and delay V1 path dependencies |
| Workspace boundary was named but not mechanically constrained | Phase 1 could sprawl into early protocol or runtime logic | keep the crates as compile-only stubs with clear deferral rules |

---

## 3. Scope Lock

### In Scope

- create the `prototype/gbn-bridge-proto/` directory tree
- create the root Cargo workspace manifest
- create empty compileable crates for protocol, runtime, publisher, and CLI
- create V2-local README and infra placeholder files
- define reserved naming conventions for V2 env vars, images, stacks, and metrics
- prove the new workspace builds independently from V1

### Out Of Scope

- protocol wire types beyond minimal placeholder exports
- runtime network behavior
- publisher authority logic
- bridge registration flows
- AWS templates or shell scripts with real deployment logic
- any edit under `prototype/gbn-proto/`
- any attempt to merge V1 and V2 into one Cargo workspace
- any attempt to retrofit V1 crates to host V2 code

---

## 4. Preflight Gates

These were the required preflight gates before Phase 1 code edits began:

1. Confirm `docs/prototyping/GBN-PROTO-005-V1-Baseline-Freeze.md` exists.
2. Confirm `docs/prototyping/GBN-PROTO-005-V1-Regression-Suite.md` exists.
3. Confirm the Phase 0 baseline release is [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline) unless an explicit replacement baseline is approved.
4. Confirm protected V1 paths are clean in the local worktree.
5. Confirm `prototype/gbn-bridge-proto/` does not already exist.
6. Capture the V1 file-integrity diff before any edits.
7. Decide that Phase 1 will not consume V1 crates as path dependencies unless a concrete need appears and is documented in the phase notes.

If any gate fails, Phase 1 should stop. A waiver path is possible, but it should be explicit and justified, not implied.

Current blockers:

- none; Phase 1 is already complete

### 4.1 Execution Result

Phase 1 created the isolated `prototype/gbn-bridge-proto/` workspace with compile-only crates for protocol, runtime, publisher, and CLI, plus V2-local README and infra placeholder files.

Validated results:

- `cargo fmt --check` passed in `prototype/gbn-bridge-proto`
- `cargo check --workspace` passed in `prototype/gbn-bridge-proto`
- `cargo test --workspace` passed using a temp `--target-dir` fallback because the OneDrive-backed workspace can throw Windows `os error 5`
- protected V1 path diff stayed empty
- `cargo check --workspace` passed in `prototype/gbn-proto`
- `cargo test -p mcn-router-sim` passed in `prototype/gbn-proto`

---

## 5. Evidence Capture Requirements

Phase 1 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or commit message |
| starting commit SHA | `git rev-parse HEAD` | phase notes or commit message |
| Phase 0 prerequisite status | Phase 0 docs and release state | phase notes |
| pre-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |
| V2 workspace member list | `prototype/gbn-bridge-proto/Cargo.toml` | README and phase notes |
| reserved naming rules | V2 README and infra README | committed docs |
| V2 sanity command results | local command output | phase notes or release notes if later packaged |
| V1 regression results | local command output | phase notes |
| post-edit protected-path diff | `git diff --name-only -- <protected paths>` | phase notes |

Do not sign off Phase 1 with vague statements like "workspace builds fine." Record the exact commands and target commit used.

---

## 6. Workspace Boundary Decisions To Lock In Phase 1

Use these defaults unless there is a project-level reason to override them before implementation starts:

| Surface | Reserved Convention | Example |
|---|---|---|
| Environment variables | `GBN_BRIDGE_` prefix | `GBN_BRIDGE_PUBLISHER_URL` |
| Container images | `gbn-bridge-proto-` prefix | `gbn-bridge-proto-publisher` |
| CloudFormation stacks | `gbn-bridge-phase2-` prefix | `gbn-bridge-phase2-dev` |
| Metrics namespace | `GBN/BridgeProto` | `GBN/BridgeProto` |
| Crate names | `gbn-bridge-*` | `gbn-bridge-runtime` |
| Future binaries | `gbn-bridge-*` or role-specific names | `gbn-bridge-cli`, `exit-bridge` |

These decisions should be documented in:

- `prototype/gbn-bridge-proto/README.md`
- `prototype/gbn-bridge-proto/infra/README-infra.md`

The point of Phase 1 is to lock the workspace boundary and naming surface before later phases start producing real behavior.

---

## 7. Workspace Manifest Policy

Phase 1 should keep the V2 workspace intentionally lean.

### Root `Cargo.toml`

The root manifest should:

- declare a standalone `[workspace]` with `resolver = "2"`
- list only the four V2 members
- include shared package metadata only if it is clearly useful now
- avoid copying the full V1 `[workspace.dependencies]` block into V2

### Crate Dependency Policy

During Phase 1:

- prefer zero dependencies unless a stub genuinely needs one
- allow V2-local path dependencies between the four new crates if required
- do not add V1 path dependencies by default
- do not create shared helper crates yet

### Stub Policy

Each new crate should compile, but remain intentionally shallow:

- `lib.rs` files may expose a placeholder struct, enum, or module
- `main.rs` may print a placeholder message or exit successfully
- any real protocol schema, network loop, storage logic, or CLI surface belongs to later phases

---

## 8. File-By-File Minimum Content

| File | Minimum Content Required In Phase 1 |
|---|---|
| `prototype/gbn-bridge-proto/Cargo.toml` | standalone workspace with `resolver = "2"`, four members, and minimal shared metadata; no dependency sprawl |
| `prototype/gbn-bridge-proto/.gitignore` | ignore `target/`, local env files, and likely generated key material |
| `prototype/gbn-bridge-proto/README.md` | purpose, isolation rule, workspace layout, build commands, reserved naming rules |
| `crates/gbn-bridge-protocol/Cargo.toml` | package metadata only, with dependencies added only if the stub needs them |
| `crates/gbn-bridge-protocol/src/lib.rs` | crate docs plus minimal placeholder export proving the crate compiles |
| `crates/gbn-bridge-runtime/Cargo.toml` | package metadata only |
| `crates/gbn-bridge-runtime/src/lib.rs` | crate docs plus placeholder type or module |
| `crates/gbn-bridge-publisher/Cargo.toml` | package metadata only |
| `crates/gbn-bridge-publisher/src/lib.rs` | crate docs plus placeholder type or module |
| `crates/gbn-bridge-cli/Cargo.toml` | package metadata and binary declaration only if needed to avoid future naming ambiguity |
| `crates/gbn-bridge-cli/src/main.rs` | trivial executable that exits successfully; no network logic |
| `tests/.gitkeep` | reserve V2-local integration test location |
| `infra/README-infra.md` | naming matrix and statement that real deployment assets begin in later phases |
| `infra/scripts/.gitkeep` | reserve script location |
| `infra/cloudformation/.gitkeep` | reserve infra template location |

---

## 9. Crate Boundary Rules

Use the crate names now, but keep the responsibilities narrow:

| Crate | Phase 1 Responsibility | Explicitly Defer |
|---|---|---|
| `gbn-bridge-protocol` | placeholder home for future wire types and signature helpers | no descriptor or message schemas yet |
| `gbn-bridge-runtime` | placeholder home for creator and bridge runtime logic | no transport loops, sessions, or heartbeats yet |
| `gbn-bridge-publisher` | placeholder home for authority-plane services | no registration, catalog, or ingest logic yet |
| `gbn-bridge-cli` | placeholder binary entrypoint for future local tooling | no real subcommands, no runtime orchestration yet |

Keep the rule simple: if a file would need a real protocol, runtime, or publisher behavior explanation, it probably belongs in Phase 2 or later.

---

## 10. Recommended Execution Order

Implement Phase 1 in this order:

1. Clear the Phase 0 and protected-V1-path preflight blockers.
2. Capture the starting branch, commit SHA, and protected-path diff state.
3. Create the root directory structure for `prototype/gbn-bridge-proto/`, `crates/`, `tests/`, and `infra/`.
4. Add the root `Cargo.toml` and `.gitignore`.
5. Add crate manifests for protocol, runtime, publisher, and CLI.
6. Add compile-only source stubs for each crate.
7. Add `README.md` and `infra/README-infra.md` with the reserved naming rules.
8. Run formatting, workspace build, and workspace test commands from the V2 root.
9. Run the required V1 regression and no-touch checks.
10. Record the exact commands run and the Phase 0 prerequisite status.

This order keeps failures cheap. If Cargo workspace setup is wrong, it fails before later docs or future runtime logic are built on top of it.

---

## 11. Validation Commands

Run these from the repo root unless noted otherwise:

```bash
cd prototype/gbn-bridge-proto
cargo fmt --check
cargo check --workspace
cargo test --workspace
```

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

- V2 commands pass
- the V1 diff check reports no forbidden-path changes
- the V1 regression commands still pass
- the worktree contains only the intended Phase 1 additions and allowed doc updates

---

## 12. Acceptance Criteria

Phase 1 is complete only when all of the following are true:

- `prototype/gbn-bridge-proto/` exists as a sibling of `prototype/gbn-proto/`
- the V2 workspace compiles and tests independently
- V1 `Cargo.toml` and V1 workspace membership are unchanged
- no new files were added under `prototype/gbn-proto/`
- V2 naming conventions are documented in both V2 README files
- the new crates contain only scaffolding and placeholder exports
- the Phase 0 prerequisite status is explicitly recorded
- the protected-path diff is clean after Phase 1 validation

---

## 13. Risks And Blockers

| Risk | What It Looks Like | Mitigation |
|---|---|---|
| Early V1 coupling | V2 manifests copy large V1 dependency blocks or path dependencies without need | keep manifests minimal in Phase 1 |
| Naming collision | future scripts or images accidentally reuse V1 names | document reserved prefixes now |
| Scope bleed | schema or runtime code starts landing inside `lib.rs` stubs | stop at placeholder exports and defer logic to Phase 2+ |
| False sign-off | V2 `cargo check` passes but tests or formatting were skipped | require the full three-command V2 sanity suite |
| Missing baseline gate | team starts V2 work without freezing the V1 baseline | require explicit Phase 0 status before sign-off |
| Dirty protected V1 state | local edits under protected V1 paths invalidate the preservation check | clear or isolate protected-path drift before Phase 1 starts |

Current blockers:

- none; Phase 1 is complete and later phases can treat the workspace boundary as fixed

---

## 14. First Implementation Cut

If Phase 1 is implemented as a single focused change set, use this breakdown:

1. Preflight evidence capture
2. Workspace scaffolding
3. Crate manifests and source stubs
4. V2 README and infra naming documentation
5. Validation and sign-off notes

That keeps Phase 1 auditable and ensures Phase 2 starts from a real isolated workspace instead of an informal folder scaffold.
