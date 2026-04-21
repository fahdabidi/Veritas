# GBN-PROTO-005 - Execution Phase 0 Detailed Plan: Freeze The V1 Baseline

**Status:** Ready after the preflight blockers are cleared  
**Primary Goal:** freeze the current `prototype/gbn-proto/` implementation as the protected V1 baseline for all later GBN-PROTO-005 work  
**Source Plan:** [GBN-PROTO-005 Execution Plan](GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign-Execution-Plan.md)

---

## 1. Current Repo Findings

These findings should drive the Phase 0 plan instead of being discovered halfway through implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | the baseline manifest must record the exact branch reference |
| Current HEAD commit | `149e34e159890d1bea846f8e6fa660cfe4b2a0be` | the freeze manifest and release package need an exact commit target |
| Existing git tags | none found | Phase 0 will establish the first tag convention for this repo |
| Existing GitHub remote | `origin = fahdabidi/Global-Broadcast-Network` | the release package should target the GitHub repo already in use |
| Current worktree status | not clean | a baseline freeze and GitHub release must not be cut from a dirty worktree |
| Current blocker on protected V1 paths | `prototype/gbn-proto/infra/cloudformation/phase1-scale-stack.yaml` is modified locally | Phase 0 cannot honestly certify a frozen V1 baseline until protected-path drift is resolved or explicitly excluded |
| Existing GitHub automation | no `.github/` release workflow found | release publication should be planned as a manual GitHub release unless automation is added later |

---

## 2. Review Summary

Phase 0 is small in file count, but it is a release-grade control point. The implementation is not complete when the two markdown files exist. It is complete only when the repo can prove:

- which exact V1 commit is being frozen
- which V1 paths are protected from later edits
- which regression suites later phases must keep green
- which local state would invalidate the freeze
- which tag and GitHub release identify the frozen baseline publicly

The main gaps in the current top-level plan are:

| Gap | Why It Matters | Resolution For Phase 0 |
|---|---|---|
| No clean-worktree gate | a dirty V1 path makes the baseline ambiguous | require protected-path cleanliness before sign-off |
| No evidence checklist | the freeze could omit branch, SHA, or validation outputs | define a concrete evidence capture list |
| No release packaging guidance | the user asked for a GitHub release package, but the phase text did not define one | define tag, release title, release body, and release target rules |
| No distinction between local draft and released baseline | docs alone do not establish an externally referenceable freeze point | require a tagged commit and GitHub release publication step |

---

## 3. Scope Lock

### In Scope

- create `docs/prototyping/GBN-PROTO-005-V1-Baseline-Freeze.md`
- create `docs/prototyping/GBN-PROTO-005-V1-Regression-Suite.md`
- record the exact baseline branch and commit SHA
- enumerate protected V1 paths and protected V1 behavior modules
- define the required V1 regression suites and when each suite is mandatory
- prepare the GitHub release package for the frozen baseline

### Out Of Scope

- editing any file under `prototype/gbn-proto/`
- changing V1 architecture documents or GBN-PROTO-004 documents
- refactoring, formatting, or repairing V1 code
- starting the V2 workspace
- introducing CI or release automation in this phase

---

## 4. Preflight Gates

Phase 0 should not begin writing the freeze artifacts until all of these are checked:

1. Confirm the baseline target commit is agreed. Default target is current `HEAD` on `main`.
2. Confirm the protected V1 paths are clean in the local worktree.
3. If protected V1 paths are dirty, stop and either:
   record an explicit exclusion decision, or
   move the baseline target to a known-clean commit, or
   get the local V1 changes committed or removed outside this phase.
4. Confirm there is no existing tag that would collide with the Phase 0 release tag.
5. Confirm the minimum V1 regression suite can be run from the current environment.
6. Confirm Phase 0 remains documentation-only and does not require any edit under `prototype/gbn-proto/`.

The current blocker is step 2. The modified V1 path must be resolved before the freeze can be released.

---

## 5. Evidence Capture Requirements

Phase 0 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| baseline branch | `git branch --show-current` | baseline freeze doc |
| baseline commit SHA | `git rev-parse HEAD` or chosen clean commit | baseline freeze doc and release notes |
| protected-path diff status before doc edits | `git diff --name-only -- <protected paths>` | phase notes or release notes |
| minimum V1 regression commands | execution plan section 2.2 | regression suite doc |
| minimum V1 regression results | local command run records | baseline freeze doc or release notes |
| protected V1 path list | execution plan sections 1.4 and 1.5 | baseline freeze doc |
| release tag name | Phase 0 release decision | baseline freeze doc and release notes |
| release publication date | GitHub release publication | release notes |

Do not paraphrase away the critical identifiers. The branch name, commit SHA, and release tag should be written exactly.

---

## 6. File-By-File Plan

| File | Required Content |
|---|---|
| `docs/prototyping/GBN-PROTO-005-V1-Baseline-Freeze.md` | purpose, baseline branch, baseline commit SHA, baseline date, protected path list, protected behavior modules, approval gate language, release tag metadata |
| `docs/prototyping/GBN-PROTO-005-V1-Regression-Suite.md` | regression suite definitions, exact commands, pass criteria, when extended suites are required, what later phases must rerun |
| `docs/prototyping/GBN-PROTO-005-Phase2-Distributed-Peer-to-Peer-Onion-Redesign-Execution-Plan.md` | optional narrow updates only to reference this detailed plan and the Phase 0 release package requirement |

The freeze doc should read like a manifest. The regression doc should read like an operational gate checklist.

---

## 7. Recommended Document Structure

Use this minimum structure for the baseline freeze manifest:

1. Purpose
2. Baseline identification
3. Protected V1 paths
4. Protected V1 modules and behaviors
5. Required regression suites
6. Approval gate for Phase 1
7. Release metadata

Use this minimum structure for the regression suite doc:

1. Purpose
2. Minimum required local suites
3. Extended local suite
4. Extended AWS suite
5. When each suite is mandatory
6. Failure handling and phase stop rule

---

## 8. Release Packaging Plan

Phase 0 should end with a GitHub release that makes the frozen baseline referenceable from later work.

Preferred automation path:

- `.github/workflows/release-phase0.yml`

This workflow validates the baseline target, checks the Phase 0 docs for the exact commit SHA and release tag, runs the minimum V1 regression suite, verifies the protected-path diff stays clean, generates release notes, and optionally creates the tag and GitHub release.

### Release Tag

Recommended tag:

`veritas-lattice-0.1.0-baseline`

If the team needs to rerun the freeze after a substantive baseline correction, append a revision suffix:

`veritas-lattice-0.1.0-baseline-r2`

### Release Title

Recommended title:

`Veritas Lattice 0.1.0`

### Release Target

The release must target a clean commit that contains:

- the two new Phase 0 documentation artifacts
- no uncommitted changes
- no protected-path changes under `prototype/gbn-proto/`
- passing minimum V1 regression results for that commit

### Release Notes Template

Use this shape for the release notes:

```md
## Summary
Freezes the V1 onion-mode baseline that GBN-PROTO-005 Phase 2 V2 work must preserve.

## Baseline
- Branch: <branch>
- Commit: <sha>
- Tag: <tag>

## Protected V1 Scope
- No-touch paths: see `GBN-PROTO-005-V1-Baseline-Freeze.md`
- Required regression suites: see `GBN-PROTO-005-V1-Regression-Suite.md`

## Validation
- V1 file integrity check: PASS/FAIL
- cargo check --workspace: PASS/FAIL
- cargo test -p mcn-router-sim: PASS/FAIL

## Approval Gate
Phase 1 must not begin until this baseline freeze is approved.
```

### Publication Procedure

Once the Phase 0 commit is merged or otherwise approved:

1. Run `.github/workflows/release-phase0.yml` in dry-run mode against the approved baseline target.
2. Review the validation artifact and generated release notes.
3. Re-run `.github/workflows/release-phase0.yml` with `publish_release=true`.
4. Confirm the workflow created the annotated tag and published the GitHub release.
5. Link the resulting release in the Phase 0 approval record if needed.

Manual fallback if GitHub Actions is unavailable:

1. Create the annotated tag on the approved commit.
2. Push the tag to `origin`.
3. Publish a GitHub release from that tag using the prepared notes.
4. Link the two Phase 0 docs in the release body.

---

## 9. Recommended Execution Order

Implement Phase 0 in this order:

1. Resolve the dirty protected-path blocker or choose a known-clean baseline commit.
2. Capture the baseline branch, SHA, and protected-path diff state.
3. Draft `GBN-PROTO-005-V1-Baseline-Freeze.md`.
4. Draft `GBN-PROTO-005-V1-Regression-Suite.md`.
5. Run the minimum V1 regression suite against the chosen baseline commit.
6. Update the docs with the exact validated commit SHA and commands.
7. Commit the Phase 0 docs.
8. Create and push the release tag.
9. Publish the GitHub release.
10. Stop and wait for explicit approval before Phase 1.

This order prevents publishing a baseline whose commit or test evidence is still moving.

---

## 10. Validation Commands

Run these from the repo root unless noted otherwise:

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

Before publishing the release, also verify:

```bash
git status --short
git tag --list
```

Expected outcome:

- the protected-path diff is empty
- the minimum V1 regression suite passes
- the release tag does not already exist before creation
- the worktree is clean at release time

---

## 11. Acceptance Criteria

Phase 0 is complete only when all of the following are true:

- both Phase 0 docs exist and contain exact baseline identifiers
- the baseline commit SHA is concrete and not a floating branch reference alone
- the protected V1 path list is copied into the freeze manifest
- the required V1 regression suites are documented with exact commands
- the minimum V1 regression suite has been run for the chosen baseline commit
- the protected-path diff is clean for the frozen commit
- the release tag name and GitHub release notes are prepared
- the Phase 0 commit has been tagged and packaged as a GitHub release
- Phase 1 is explicitly blocked pending approval

---

## 12. Risks And Blockers

| Risk | What It Looks Like | Mitigation |
|---|---|---|
| Dirty V1 state | protected files differ locally from the intended freeze baseline | require a clean protected-path diff before release |
| Wrong baseline commit | docs cite `HEAD` while validation was run on a different commit | record the exact tested SHA in both docs and release notes |
| Weak release traceability | docs exist but no tag or GitHub release points to them | require a named tag and release publication |
| Silent scope creep | someone edits V1 files during the freeze phase | enforce documentation-only changes |
| False sense of safety | only `cargo check` runs, but router tests are skipped | require the full minimum V1 regression suite |

Current blocker:

- `prototype/gbn-proto/infra/cloudformation/phase1-scale-stack.yaml` is already modified in the local worktree and sits inside a protected V1 path.

---

## 13. First Implementation Cut

If Phase 0 is implemented as a single focused change set, use this breakdown:

1. Baseline evidence capture
2. Freeze manifest
3. Regression suite manifest
4. Validation run
5. Git tag and GitHub release packaging

That keeps the freeze auditable and gives Phase 1 a stable reference point instead of a moving target.
