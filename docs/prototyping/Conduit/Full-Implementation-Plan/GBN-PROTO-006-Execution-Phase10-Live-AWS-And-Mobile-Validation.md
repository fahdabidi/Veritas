# GBN-PROTO-006 - Execution Phase 10 Detailed Plan: Live AWS And Mobile Validation

**Status:** Ready to start after Phase 9 distributed end-to-end harness and fault injection are implemented and validated  
**Primary Goal:** validate the full Conduit implementation on live AWS and mobile-network conditions, capture production-shaped bootstrap, fanout, forwarding, ACK, failover, and trace evidence, and produce a durable validation report suitable for the final decision gate  
**Source Plan:** [GBN-PROTO-006 Execution Plan](GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md)  
**Protected V1 Baseline:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)  
**Phase 9 Detailed Plan:** [GBN-PROTO-006-Execution-Phase9-Distributed-End-To-End-Harness-And-Fault-Injection](GBN-PROTO-006-Execution-Phase9-Distributed-End-To-End-Harness-And-Fault-Injection.md)  
**Starting Conduit Baseline:** `2b6d5c5d24e269e96e3fdc820f3f90669607414a`

---

## 1. Current Repo Findings

These findings should drive Phase 10 instead of being rediscovered during implementation:

| Item | Current Value | Why It Matters |
|---|---|---|
| Current branch | `main` | Phase 10 should record the mainline commit used to begin live validation |
| Current HEAD commit | `2b6d5c5d24e269e96e3fdc820f3f90669607414a` | current committed Conduit baseline still documents only prototype-era live evidence |
| Current mobile validation script | [`mobile-validation.sh`](../../../prototype/gbn-bridge-proto/infra/scripts/mobile-validation.sh) supports `local` and `aws` modes but is explicitly tied to the earlier prototype stack | a full implementation validation pass needs a new script surface and new assumptions |
| Current metrics script | [`collect-bridge-metrics.sh`](../../../prototype/gbn-bridge-proto/infra/scripts/collect-bridge-metrics.sh) collects Phase 11 prototype stack evidence only | it is not yet the full trace-aware validation collector for the full implementation |
| Current mobile matrix | [`mobile-test-matrix.md`](../../../prototype/gbn-bridge-proto/docs/mobile-test-matrix.md) still says the deployed binaries are prototype entrypoints rather than full network listeners | the current evidence baseline is explicitly incomplete for the full implementation |
| Current validation report file | `docs/prototyping/GBN-PROTO-006-Conduit-Full-Implementation-Test-Report.md` does not exist | there is no final report surface for the full implementation validation phase |
| Current trace artifact collection | there is no `collect-conduit-traces.sh` | live runs still do not have a dedicated end-to-end trace evidence collector |
| Current full-stack scripts | full implementation deploy/smoke/teardown assets do not yet exist in the current repo state | Phase 10 depends on Phase 8 completing first |

---

## 2. Review Summary

Phase 10 is where Conduit stops being "locally convincing" and becomes either live-validated or not. If this phase is weak, the final decision gate will still be forced to rely on local harnesses and architectural intent rather than real AWS/mobile evidence.

The main gaps the detailed Phase 10 plan must close are:

| Gap | Why It Matters | Resolution For Phase 10 |
|---|---|---|
| current mobile validation surface is prototype-only | the repo still has no full implementation live validation workflow | add `mobile-validation-full.sh` aligned to the full stack |
| no full implementation test report exists | evidence can remain fragmented and non-decision-ready | add `GBN-PROTO-006-Conduit-Full-Implementation-Test-Report.md` |
| no dedicated trace collector exists | live `chain_id` evidence remains hard to assemble across services | add `collect-conduit-traces.sh` |
| current matrix still documents prototype limitations | the repo still lacks a full implementation validation baseline | update or supplement the test matrix with full-stack evidence expectations |

Phase 10 should make the live evidence real, but it should not yet make the promotion decision. That remains Phase 11.

---

## 3. Scope Lock

### In Scope

- create the full implementation validation report doc
- add `mobile-validation-full.sh`
- add `collect-conduit-traces.sh`
- update V2 mobile / test-matrix documentation as needed for the full implementation
- capture live AWS bootstrap, fanout, forwarding, ACK, and failover evidence
- capture end-to-end `chain_id` evidence across participating services and scripts

### Out Of Scope

- making the final promotion decision
- modifying `prototype/gbn-proto/**`
- modifying the main repo `README.md`

---

## 4. Preflight Gates

Phase 10 should not begin code edits or live runs until all of these are checked:

1. Confirm the Phase 0 inventory deliverables exist.
2. Confirm Phases 1 through 9 are implemented and validated so the full system and distributed harness already exist.
3. Confirm protected V1 paths are clean in the local worktree.
4. Confirm a real full-stack deployment from Phase 8 exists and is runnable.
5. Confirm a distributed local harness from Phase 9 exists and is green before live runs begin.
6. Confirm `chain_id` is already fully propagated in code and artifacts from Phase 7.
7. Confirm `README.md` remains out of scope.

If any gate fails, Phase 10 should stop.

Current blocker:

- Phases 1 through 9 are not yet implemented in this full-implementation track, so Phase 10 remains planning-ready only

---

## 5. Live Validation Decisions To Lock In Phase 10

### 5.1 One Canonical Report Rule

Phase 10 should produce one canonical report:

- `GBN-PROTO-006-Conduit-Full-Implementation-Test-Report.md`

That report should consolidate:

- environment details
- stack identity
- bootstrap results
- mobile-network observations
- forwarding / ACK results
- failover / churn results
- trace evidence
- unresolved anomalies

### 5.2 Script Split Rule

Phase 10 should separate:

- execution: `mobile-validation-full.sh`
- trace collection: `collect-conduit-traces.sh`

Do not overload one script with every concern if it makes evidence collection opaque.

### 5.3 Measurement Rule

The live validation set should capture at minimum:

- first-contact bootstrap success and timing
- returning creator refresh timing
- UDP punch success and failure cases
- data forwarding and ACK timing
- bridge failover and reuse behavior under churn
- batch-window behavior
- end-to-end `chain_id` evidence

### 5.4 Trace Evidence Rule

Phase 10 must produce live evidence that one `chain_id` can be followed across:

- authority
- seed bridge
- remaining bridges where applicable
- receiver
- ACK path
- validation scripts / artifacts

This must be explicit in the report, not implied.

### 5.5 Environment Identity Rule

Every live run should record enough environment detail for later review:

- stack name
- region
- build/image identifiers
- test window
- mobile carrier/network context where available
- target ports and key config deviations

### 5.6 Failure Recording Rule

Phase 10 should not suppress failed scenarios.

If a live run shows:

- bootstrap failures
- mobile punch failures
- missing trace continuity
- receiver or ACK anomalies

those must be recorded clearly in the report as blockers or unresolved issues.

---

## 6. Module And Asset Ownership To Lock In Phase 10

Phase 10 should keep responsibilities split like this:

| Asset | Responsibility |
|---|---|
| `GBN-PROTO-006-Conduit-Full-Implementation-Test-Report.md` | canonical consolidated evidence report |
| `mobile-validation-full.sh` | run or coordinate live AWS/mobile validation scenarios |
| `collect-conduit-traces.sh` | gather distributed `chain_id` evidence from the full stack |
| existing V2 AWS/mobile scripts | may be updated or wrapped, but should not remain the only report path |
| V2 mobile / test-matrix docs | document the new full implementation evidence model |

---

## 7. Dependency And Implementation Policy

Phase 10 should reuse the already-built full system rather than layering on new product logic.

### Recommended Dependencies

- full deployment stack from Phase 8
- distributed harness assumptions from Phase 9
- trace propagation model from Phase 7
- existing AWS CLI and log/metrics collection surfaces where practical

### Bias

- prefer reproducible scripted runs
- prefer explicit report artifacts over scattered console output
- prefer preserving raw trace evidence alongside summarized findings

### Avoid In Phase 10

- changing service behavior just to make the report easier
- mutating V1 scripts
- drifting into the final decision record before the evidence is complete

---

## 8. Evidence Capture Requirements

Phase 10 should collect and preserve these exact data points:

| Evidence | Source | Must Appear In |
|---|---|---|
| starting branch | `git branch --show-current` | phase notes or report metadata |
| starting commit SHA | `git rev-parse HEAD` | phase notes or report metadata |
| Phase 1-9 prerequisite status | implementation and validation records | report appendix or notes |
| live stack identity | stack outputs and script logs | report |
| bootstrap and refresh timing | script outputs and traces | report |
| forwarding and ACK behavior | receiver and ACK traces | report |
| failover / churn behavior | live test output and traces | report |
| batch-window behavior | timing output and traces | report |
| `chain_id` evidence | trace collector outputs | report |
| anomalies and blockers | run logs and summaries | report |

Do not sign off Phase 10 with only "smoke passed." Capture enough evidence to support a real decision.

---

## 9. Recommended Execution Order

Implement Phase 10 in this order:

1. Capture the starting branch, commit SHA, and protected-path diff state.
2. Add the report document shell.
3. Add `mobile-validation-full.sh`.
4. Add `collect-conduit-traces.sh`.
5. Update the mobile / test-matrix docs to reference the full implementation validation surface.
6. Run the live validation scenarios.
7. Populate the report with results, anomalies, and trace evidence.
8. Run the required V1 preservation checks.

This order ensures the report and collection surface exist before live runs begin.

---

## 10. Validation Commands

Run these from the repo root unless noted otherwise:

Standard V2 checks:

```bash
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
```

Live validation and evidence collection:

```bash
bash prototype/gbn-bridge-proto/infra/scripts/mobile-validation-full.sh
```

```bash
bash prototype/gbn-bridge-proto/infra/scripts/collect-conduit-traces.sh
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
bash infra/scripts/run-tests.sh <v1-stack-name> <region>
```

Recommended Phase 10-specific checks:

```bash
rg -n "chain_id" prototype/gbn-bridge-proto/infra/scripts prototype/gbn-bridge-proto/docs
```

```bash
git status --short
```

Expected outcome:

- live AWS/mobile evidence is captured
- a full validation report exists
- end-to-end `chain_id` evidence is preserved
- protected V1 paths show no drift
- extended V1 AWS regression remains green

---

## 11. Acceptance Criteria

Phase 10 is complete when:

- a full implementation validation report exists
- live AWS/mobile validation scripts exist and have been used
- end-to-end `chain_id` evidence is captured in live artifacts
- bootstrap, forwarding, ACK, failover, and batch behavior have live evidence
- all required V1 and V2 validation commands have been run and recorded

Phase 10 is not complete if:

- evidence still relies only on local harness output
- there is no consolidated report
- `chain_id` evidence is still incomplete in live runs

---

## 12. Risks And Blockers

| Risk | Why It Matters | Mitigation |
|---|---|---|
| live runs are under-documented | the final decision gate would be weak | make the report and trace collector first-class outputs |
| trace evidence is fragmented | distributed observability cannot be defended | require one dedicated trace collector and explicit report section |
| live anomalies are suppressed | the decision gate would be biased | require anomalies and unresolved issues to be recorded explicitly |
| scripts remain prototype-shaped | live validation would still target the wrong topology | make Phase 8 full stack and Phase 9 harness prerequisites explicit |

---

## 13. Sign-Off Recommendation

The correct Phase 10 sign-off is:

- the full Conduit implementation has live AWS/mobile evidence
- one consolidated report exists
- one distributed flow can be correlated end-to-end by `chain_id` in live artifacts

The correct Phase 10 sign-off is not:

- a local-only pass
- a smoke-only note without measurements
- a report that omits anomalies or missing trace evidence

