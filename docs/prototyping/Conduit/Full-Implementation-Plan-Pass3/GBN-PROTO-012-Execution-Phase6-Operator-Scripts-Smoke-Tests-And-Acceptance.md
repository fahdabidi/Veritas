# GBN-PROTO-012 - Execution Phase 6 - Operator Scripts And Acceptance Gate

**Status:** Completed
**Last Updated:** 2026-05-08
**Parent Plan:** [GBN-PROTO-012](GBN-PROTO-012-Conduit-Architecture-Correct-Bootstrap-Execution-Plan.md)
**Depends On:** Phases 0–5 complete

## Objective

Land the operator-facing tooling that drives the Pass 3 architecture-correct flow,
the AWS ECS Exec verification for the new creator pods, and the Pass-3 acceptance
gate (local-k8s + AWS walkthroughs). After this phase no script path treats the
old direct-authority `discovery-probe` shortcut as successful first-time creator
bootup.

This phase does **not** implement the smoke test scripts themselves. The smoke
implementation is split into Phases 7, 8, and 9, one per smoke. Each has its own
plan doc:

- Phase 7: [GBN-PROTO-012-Smoke-1-Tracing.md](GBN-PROTO-012-Smoke-1-Tracing.md)
- Phase 8: [GBN-PROTO-012-Smoke-2-Discovery.md](GBN-PROTO-012-Smoke-2-Discovery.md)
- Phase 9: [GBN-PROTO-012-Smoke-3-Route.md](GBN-PROTO-012-Smoke-3-Route.md)

The Pass 2 smoke plans (`GBN-PROTO-009`, `010`, `011`) remain frozen in
`Full-Implementation-Plan-Pass2/` as historical record. Per Master plan §2.9, the
Pass 3 smoke docs above supersede them.

Update the parent plan status tracker when this phase is complete.

Completed 2026-05-08. The shared operator action library is implemented and wired
into both AWS and local k8s control scripts. Every traceable action generates an
explicit `chain_id`, passes it through the admin request, prints the echoed id, and
can collect chain-scoped trace artifacts. The Pass 3 acceptance runner and smoke
script placeholders are present; live k8s/AWS execution remains part of the later
smoke implementation gates.

---

## Operator Script Updates

Files to update:

- `prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh` (AWS)
- `prototype/gbn-bridge-proto/infra/scripts/k8s-control-interactive.sh` (local k8s)
- new: `prototype/gbn-bridge-proto/infra/scripts/_seed_actions.sh` (T1.15 shared
  library; sourced by both top-level scripts)

### Shared Menu Library (T1.15)

`_seed_actions.sh` owns all menu actions. Each top-level script supplies only:

- `discover_nodes()` — AWS uses `aws ecs list-tasks` + `describe-tasks`; k8s uses
  `kubectl get pods -n veritas -o json`. Both return a JSON array of
  `{ id, role, conduit_actor, ip, admin_url }` records.
- `admin_call(node_id, method, path, body)` — AWS uses
  `aws ecs execute-command --command "curl -s -X $method http://127.0.0.1:9090$path -d $body"`;
  k8s uses `kubectl exec $node_id -- curl -s -X $method http://127.0.0.1:9090$path -d $body`.
- entry point: `main` calls `source _seed_actions.sh; render_menu`.

`_seed_actions.sh` exports:

- `action_seed_host_creator`
- `action_initialize_publisher_dht`
- `action_seed_new_creator`
- `action_dump_local_dht`
- `action_send_dummy`
- `action_build_upload_session` (Phase 10)
- `action_send_upload` (Phase 11)
- `action_reset_creator_state` (T2.3)
- `action_collect_traces`

Menu rendering, prompts, validation, error printing, trace collection are all in the
shared library. The top-level scripts are thin transport adapters.

### Required Menu Actions (Master plan §3.1)

```text
SeedHostCreator
InitializePublisherDht
SeedNewCreator
DumpLocalDht
SendDummy              (single-lane envelope demo, Phase 5)
BuildUploadSession     (sanitize + chunk + manifest + per-chunk encrypt, Phase 10)
SendUpload             (multi-lane progressive fanout, Phase 11)
ResetCreatorState
CollectTraces
```

The legacy `TriggerCommand`, `BootstrapSmoke`, and `DiscoveryProbe` actions remain in
the menu for one Pass-3 release for backward operator familiarity, but each prints a
deprecation warning pointing to the new commands. They are removed in a subsequent
pass.

### Action Flow Summary

| Action | Steps |
|---|---|
| **SeedHostCreator** | discover creator pods → select → discover Publisher (one role, two surfaces) → discover direct ExitBridges → select ExitBridgeA → fetch metadata → resolve `bootstrap_genesis` flag → POST `/v1/admin/seed-host-creator` → print state and `chain_id` |
| **InitializePublisherDht** | discover Publisher authority surface → POST `/v1/admin/publisher-dht/initialize` → require 10 initialized Publisher-side ExitBridge DHT entries before NewCreator bootstrap |
| **SeedNewCreator** | discover creator pods → select NewCreator → select HostCreator → verify `host_role_state=host_seeded` → verify Publisher DHT has 10 initialized bridge entries → POST `/v1/admin/seed-new-creator` → poll `/local-dht` until terminal state or 120 s → print state, bridge counts, `chain_id` |
| **DumpLocalDht** | discover any node → check `role` field → on creator pod print full table; on Publisher/exit_bridge print role-tagged not-applicable message |
| **SendDummy** | discover creator pods → check `self_onboarding_state` ∈ {`onboarded`, `fanout_partial`} → prompt normal vs `force_bridge_failure` → POST `/v1/admin/send-dummy` → print `route_source`, `selected_bridge_ids`, `assigned_bridge_id`, `ciphertext_only_at_bridge`, `chain_id` |
| **BuildUploadSession** | discover creator pods → check onboarded → prompt input source (synthetic / inline / path) → prompt chunk size → POST `/v1/admin/build-upload-session` → print `session_id`, manifest fields, sanitization report, `chain_id` |
| **SendUpload** | discover creator pods → check onboarded → list sessions via `/v1/admin/upload-sessions` → prompt session selection → prompt target lane count → optionally prompt `force_lane_failure` for failover demo → POST `/v1/admin/send-upload` → print `session_status`, `lanes_used`, `completed_chunks`, progressive-timeline timestamps, `chain_id` |
| **ResetCreatorState** | discover creator pods → confirm prompt → POST `/v1/admin/reset-creator-state` → print prior state and prior `chain_id` |
| **CollectTraces** | prompt for `chain_id` → query Loki + Tempo (k8s) or CloudWatch + X-Ray (AWS) → write traces to `/tmp/conduit-traces-${chain_id}/` |

---

## Traceability And ChainID Contract

Every shared menu action generates an explicit operator `chain_id` before it calls
an admin endpoint. The path includes `?chain_id=<generated-id>` for:

- `SeedHostCreator`
- `InitializePublisherDht`
- `SeedNewCreator`
- `ResetCreatorState`
- `SendDummy`
- `DiscoveryProbe` while the legacy action remains available
- `BuildUploadSession` and `SendUpload` when their owning phases land

If an action sends a JSON body that also contains `chain_id`, the value must match
the query parameter. A mismatch is a test failure and the endpoint must return
`400 bad_query`; scripts must not retry with a new id because that would split the
diagnostic trail. Responses are printed with the echoed `chain_id`, and
`CollectTraces` uses that same value to collect Loki/Tempo or CloudWatch/X-Ray
artifacts under `/tmp/conduit-traces-${chain_id}/`.

The Pass 3 acceptance runner must preserve the generated chain ids in artifact
directories so failures can be traced from operator action to Rust span without
guessing which endpoint minted the id.

---

## Caller Migration For SendDummy (T1.13)

Phase 5 changes `POST /v1/admin/send-dummy` to fail with `creator_not_onboarded` on
pods whose `self_onboarding_state` is not `onboarded` or `fanout_partial`. Pre-Pass-3
callers that target Publisher or ExitBridge pods will start failing.

Audit and migrate every existing call site before this phase ships:

| Call Site | Pre-Pass-3 Target | Post-Pass-3 Target |
|---|---|---|
| `relay-control-interactive-v2.sh` `SendDummy` action | any node | `creator-host` or `creator-new` |
| `k8s-control-interactive.sh` `SendDummy` action | any node | `creator-host` or `creator-new` |
| Pass 2 `k8s-smoke-route.sh --send-dummy` | authority/bridge | superseded by Pass 3 Smoke 3 (`k8s-smoke-route-v3.sh`); the Pass 2 script is left untouched but the new Smoke 3 doc takes precedence |
| Pass 2 `admin_send_dummy.rs` test | uses `AdminCreatorConfig` to inject creator state; passes today | extend test fixture to set `self_onboarding_state=onboarded` first; existing tests must update the fixture; no behavior shim |
| Any CI workflow calling `send-dummy` against publisher | direct authority | redirect to `creator-host` or `creator-new` |

No backward-compatibility shims. Old call sites are removed (or migrated). The
`creator_not_onboarded` error is the explicit signal that something needs migration.

Document the migration in the new Smoke 3 doc and in the master `gap inventory` row
("`send-dummy` breaking change has no migration").

---

## AWS ECS Exec Verification For New Creator Pods (T2.2)

Phase 0 added `creator-host` and `creator-new` ECS services. Pass 2 wired existing
pods through ECS Exec; the new pods inherit the same listener layout but are brand-new
tasks with their own IAM task roles.

Required verification (in this phase, before AWS acceptance is declared green):

1. CloudFormation deploy creates `creator-host` and `creator-new` services.
2. Each task definition's IAM task role grants
   `ssmmessages:CreateControlChannel`, `CreateDataChannel`, `OpenControlChannel`,
   `OpenDataChannel`.
3. `aws ecs describe-services --services creator-host creator-new --cluster conduit`
   reports `RUNNING_COUNT=1` for each.
4. `aws ecs execute-command --cluster conduit --task <creator-host-task-arn> --command "/bin/sh" --interactive`
   returns an interactive shell.
5. Inside the shell, `curl -s http://127.0.0.1:9090/v1/admin/node-metadata` returns
   `role: "creator"`.
6. `aws ecs execute-command ... --command "curl -s http://127.0.0.1:9090/v1/admin/local-dht"`
   returns the empty `LocalDiscoveryTable`.

Add an automated AWS smoke runner
`prototype/gbn-bridge-proto/infra/scripts/aws-smoke-creator-exec.sh` that performs
those 6 checks and emits a JSON report. This is invoked once during AWS validation;
not part of every local iteration.

---

## Pass 3 Smoke Test Suite Reference

The smoke implementation is owned by Phases 7, 8, 9, and 12 — each a discrete
trackable phase with its own plan doc:

- **Phase 7 (Smoke 1 — Tracing):** [GBN-PROTO-012-Smoke-1-Tracing.md](GBN-PROTO-012-Smoke-1-Tracing.md)
  — first gate; proves Loki/Tempo/Prometheus instrumentation works for all 14 actor
  pods.
- **Phase 8 (Smoke 2 — Discovery / Bootup):** [GBN-PROTO-012-Smoke-2-Discovery.md](GBN-PROTO-012-Smoke-2-Discovery.md)
  — drives real `SeedHostCreator → InitializePublisherDht → SeedNewCreator → onboarded`; asserts full local
  DHT, distinct actor chain, all 16 §2.5 events.
- **Phase 9 (Smoke 3 — Route And Encryption Boundary):** [GBN-PROTO-012-Smoke-3-Route.md](GBN-PROTO-012-Smoke-3-Route.md)
  — drives two SendDummy invocations (normal + forced failover) and asserts
  `route_source=local_dht`, ciphertext-only at bridge, receiver persistence, and
  failover. Single-lane envelope demo (Phase 5).
- **Phase 12 (Smoke 4 — Full Upload Pipeline):** [GBN-PROTO-012-Smoke-4-Full-Upload.md](GBN-PROTO-012-Smoke-4-Full-Upload.md)
  — drives `BuildUploadSession` then `SendUpload` for the full §3.4–§3.7 pipeline
  (sanitize → chunk → manifest → per-chunk encrypt → multi-lane progressive
  fanout → receiver content reconstruction). Multi-lane (Phase 10 + 11).

Phase 6's responsibility for the smoke suite is limited to the operator-side
tooling that the smoke scripts call (`_seed_actions.sh`, the new menu actions —
including `BuildUploadSession` and `SendUpload` from Phases 10/11 — the
ResetCreatorState endpoint, AWS ECS Exec verification) and to the suite's runner
order in the acceptance gate below.

---

## Local Kubernetes Acceptance

Run inside WSL2 Ubuntu (per Master plan §2.8):

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 3 tooling requires WSL2 Ubuntu" >&2; exit 1; }

# Pre-flight: WSL2 host sized for 20-pod cluster (per Phase 0)
free -h
nproc

# Bring-up
bash prototype/gbn-bridge-proto/infra/scripts/k8s-up.sh

# Smoke suite (must run in this order; each gate depends on the previous green)
bash prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-tracing-v3.sh --require-observability   # Phase 7
bash prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-discovery-v3.sh --require-observability # Phase 8
bash prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-route-v3.sh --require-observability     # Phase 9
bash prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-upload-v3.sh --require-observability    # Phase 12
```

Expected:

- All four smoke runs exit 0.
- Tracing smoke shows all 16 bootup events plus 8 SendDummy events plus 12
  upload-pipeline events (36 total in §2.5).
- Discovery smoke proves real `creator-new → creator-host → ExitBridgeA → Publisher
  (authority surface) → ExitBridgeB → creator-new` chain via `chain_id`
  correlation.
- Route smoke shows `route_source=local_dht`, `ciphertext_only_at_bridge=true`, and
  failover selecting a second bridge (single-lane envelope demo, Phase 5).
- Upload smoke shows `session_status=Completed`, content_hash match, ≥ 2 distinct
  lanes used, progressive timeline (`first_chunk_dispatched_at_ms <
  all_lanes_active_at_ms`), bridge ciphertext-only, and lane failover under
  `force_lane_failure` (Phase 10 + 11).
- Artifact bundles include chain_id-grouped trace evidence for every smoke.

---

## AWS Acceptance

Run when AWS validation is requested. Uses
`relay-control-interactive-v2.sh` against the Conduit ECS stack:

```bash
# Inside WSL2 Ubuntu with AWS_PROFILE configured
bash prototype/gbn-bridge-proto/infra/scripts/aws-smoke-creator-exec.sh   # T2.2 verification
bash prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh
# Operator menu: SeedHostCreator → InitializePublisherDht → SeedNewCreator → DumpLocalDht → SendDummy →
#                BuildUploadSession → SendUpload
```

Expected:

- T2.2 verification: all 6 ECS Exec checks pass for `creator-host` and `creator-new`.
- The interactive menu reaches `self_onboarding_state=onboarded` on `creator-new`.
- `SendDummy` returns `route_source=local_dht` and `ciphertext_only_at_bridge=true`.
- `SendDummy` against a non-onboarded node returns `creator_not_onboarded`.
- `BuildUploadSession` produces a synthetic 1 MiB session with the expected chunk
  count and content_hash.
- `SendUpload` reaches `session_status=Completed` against the AWS 10-bridge stack;
  Publisher (receiver surface) reconstructs the content and content_hash matches.
- CloudWatch logs and X-Ray traces contain the bootup `chain_id` and the upload
  `session_id`.

AWS validation is not required for every local iteration, but Pass 3 is not complete
until at least one AWS operator walkthrough succeeds.

---

## Test Commands (Minimum, In Order)

```bash
# WSL2 baseline
uname -a | grep -i microsoft >/dev/null || { echo "Pass 3 tooling requires WSL2 Ubuntu" >&2; exit 1; }

# V1 untouched
git diff --stat -- prototype/gbn-proto/   # must be empty
git diff --stat -- docs/prototyping/Conduit/Full-Implementation-Plan-Pass2/  # must be empty (T1.14)

# Shell syntax
bash -n prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh
bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-control-interactive.sh
bash -n prototype/gbn-bridge-proto/infra/scripts/_seed_actions.sh
bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-tracing-v3.sh
bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-discovery-v3.sh
bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-route-v3.sh
bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-upload-v3.sh
bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-pass3-acceptance.sh
bash -n prototype/gbn-bridge-proto/infra/scripts/aws-smoke-creator-exec.sh

# Cargo
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace

# V1 regression (only if a workspace dependency or shared tooling change occurred)
cd ../gbn-proto && cargo test --workspace
```

---

## Acceptance Criteria

The criteria below are the final Pass 3 acceptance gate that Phase 6 wires into
operator tooling. Phase 6 completion itself is limited to landing the shared
scripts, command surfaces, ChainID propagation, syntax validation, and runner
structure. The smoke scripts exit-0 criteria are satisfied by Phases 7, 8, 9, and
12 when those implementations replace the current placeholders.

- Operator can execute the documented seed flow (`SeedHostCreator` then
  `SeedNewCreator`) and reach `onboarded` on `creator-new` from a clean cluster.
- `ResetCreatorState` clears state and a re-run of `SeedNewCreator` produces a
  fresh `chain_id`.
- Operator can execute `BuildUploadSession` and `SendUpload` against an onboarded
  creator and reach `session_status=Completed` with content_hash match.
- All four Pass 3 smoke scripts (`k8s-smoke-tracing-v3.sh`,
  `k8s-smoke-discovery-v3.sh`, `k8s-smoke-route-v3.sh`,
  `k8s-smoke-upload-v3.sh`) exit 0 against the local k3d cluster.
- Smoke 2 fails (does not "pass by accident") if the cluster path collapses to a
  direct authority catalog/bootstrap shortcut.
- Smoke 3 fails if the selected creator is not onboarded, if `route_source` is not
  `local_dht`, if the bridge can decrypt the dummy frame, or if failover does not
  select a second bridge.
- Smoke 4 fails if `session_status != Completed`, content_hash mismatches, any
  bridge log contains the plaintext marker, fewer than 2 distinct lanes are
  used, the progressive-timeline assertion does not hold, or the failover run
  does not produce `creator_upload_lane_failover` events.
- Failure artifacts identify the exact failed bootup or upload phase by
  `chain_id` / `session_id` and span name, sufficient for diagnosis without
  re-running the cluster.
- Caller migration (T1.13) is complete: every pre-Pass-3 `send-dummy` invocation
  targets a creator pod, not a Publisher or ExitBridge pod.
- AWS T2.2 verification passes for `creator-host` and `creator-new` services.
- AWS operator walkthrough completes the full upload pipeline at least once.
- Pass 2 smoke plan files (`GBN-PROTO-009`, `010`, `011`) are unchanged from their
  committed state — Pass 3 supersedes them via the new Pass-3-folder smoke docs.
- V1 (`prototype/gbn-proto/**`) is unchanged.
- Parent plan status tracker is updated to mark all Pass 3 phases (0–12)
  complete.
