# GBN-PROTO-007 - Conduit V2-to-V1 Parity Execution Plan (Pass 2)

**Document ID:** GBN-PROTO-007
**Status:** Implemented locally - Phases 1 through 5 complete; AWS live acceptance deferred
**Last Updated:** 2026-05-07
**Related Docs:**
[GBN-PROTO-006 Execution Plan](../Full-Implementation-Plan/GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md),
[GBN-ARCH-002-V2 Bridge Protocol](../../architecture/GBN-ARCH-002-Bridge-Protocol-V2.md)

This Pass 2 plan upgrades the Conduit V2 implementation produced by GBN-PROTO-006 from a
deployable but operationally bare distributed system into one that has feature parity with
the V1 Lattice operational toolset. The trigger is that the V2 prototype now deploys
correctly, but the operator interactive control script
[relay-control-interactive-v2.sh](../../../prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh)
has only 3 menu items (status, bootstrap-smoke, teardown), while the equivalent V1 script
[relay-control-interactive.sh](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh)
has 9 (DumpDht, DumpMetadata, BroadcastSeed, UnicastDHT, SendDummy, LiveMetrics, CheckImages,
Refresh, Exit). Investigation found this is not just a script gap — it is a backend gap.

## Status Trackers

- `[ ]` Pending
- `[/]` In Progress
- `[x]` Completed

| Phase | Title | Status |
|---|---|---|
| 1 | Read-only Admin HTTP Endpoints | `[x]` |
| 2 | Admin Command Injection Endpoint | `[x]` |
| 3 | CloudWatch Metrics Emission | `[x]` |
| 4 | Universal Creator Capability Library | `[x]` |
| 5 | Interactive Control Script Port | `[x]` |

---

## 1. Pass 1 Gap Inventory

GBN-PROTO-006 delivered the deployable Conduit V2 system but did not include any operator
control or observability surface beyond `/healthz` and `/readyz`. These gaps were discovered
during a Pass 2 capability review in May 2026 by mapping every V1 lattice control command
to its V2 equivalent.

| Pass 1 Gap | Where Pass 1 Stopped | Why It Blocks Operator Parity | Pass 2 Phase That Closes It |
|---|---|---|---|
| No admin HTTP API on the publisher-authority service | [`AuthorityRoute` enum at api.rs:204-215](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/api.rs#L204-L215) exposes only protocol routes plus health checks | operator cannot list registered bridges, query ingested frames, or read metrics from outside the database | **Phase 1** |
| No admin command-injection path into the bridge control WebSocket | [`control.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/control.rs) sends `BridgeCommandPayload` only on real triggers (registration, batching, etc.) | operator cannot manually drive `SeedAssign`, `CatalogRefresh`, or `Revoke` for testing or remediation | **Phase 2** |
| Zero CloudWatch metrics emission | [`metrics.rs:1-65`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/metrics.rs) keeps the snapshot in memory only; no `cloudwatch:PutMetricData` call exists in the workspace | operator cannot build a live metrics dashboard equivalent to V1's `LiveMetrics`; only logs are visible to outside observers | **Phase 3** |
| No creator client implementation in V2 | V1 ships a full `CreatorService` ECS task; V2 has no creator binary or library, only authority/receiver/bridge. Creator role is described in the protocol but unimplemented | operator cannot synthesize an end-to-end test packet; `SendDummy` has no callable surface | **Phase 4** |
| Operator script is 47 lines | [relay-control-interactive-v2.sh](../../../prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh) covers only status / bootstrap-smoke / teardown | operator has to drop into raw `aws` CLI for every other diagnostic | **Phase 5** |

These gaps are real Pass 1 gaps, not regressions. GBN-PROTO-006 explicitly deferred operator
tooling to a follow-on track, recorded in the
[GBN-PROTO-006 main execution plan §2](../Full-Implementation-Plan/GBN-PROTO-006-Conduit-Full-Implementation-Execution-Plan.md).

---

## 2. V1 Lattice Capability Reference

These are the V1 capabilities Pass 2 brings to V2 parity. Each entry cross-references the
V1 source so reviewers can compare behavior 1:1.

| V1 Capability | V1 Source | V2 Pass 2 Equivalent |
|---|---|---|
| `DumpDht` — list local DHT / gossip seed store | [relay-control-interactive.sh:543-546](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh#L543-L546) | `GET /v1/admin/bridges` (list registered bridges from `conduit_bridges`) — Phase 1 |
| `DumpMetadata` — packet ring buffer dump with `chain_id` filter | [relay-control-interactive.sh:548-568](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh#L548-L568) | `GET /v1/admin/frames?chain_id=...&limit=...` (query `conduit_ingested_frames`) — Phase 1 |
| Authority metrics snapshot | implicit in lattice CW metrics | `GET /v1/admin/metrics` (return `AuthorityMetricsSnapshot` JSON) — Phase 1 |
| `BroadcastSeed` — force gossip seed broadcast | [relay-control-interactive.sh:570-573](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh#L570-L573) | `POST /v1/admin/bridges/{id}/command` (push `SeedAssign` / `CatalogRefresh` / `Revoke`) — Phase 2 |
| `LiveMetrics` — CloudWatch dashboard | [relay-control-interactive.sh:776-872](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh#L776-L872) | `Veritas/Conduit` CloudWatch namespace populated by 60s emitter in each service binary — Phase 3 |
| `SendDummy` — dispatch test packet through full circuit | [relay-control-interactive.sh:874-1272](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh#L874-L1272) | New `gbn-bridge-creator` library + `POST /v1/admin/send-dummy` on every node — Phase 4 |
| `UnicastDHT` — direct NodeAnnounce to peer | [relay-control-interactive.sh:575-612](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh#L575-L612) | Existing public route `GET /v1/creator/catalog` invoked via ECS exec — Phase 5 (no backend change) |
| `CheckImages` — compare task image vs ECR `:latest` | [relay-control-interactive.sh:1274-1380](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh#L1274-L1380) | Same logic, ECS-only branch — Phase 5 |
| Node discovery + table | [relay-control-interactive.sh:117-221](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh#L117-L221) | Adapted to V2: 3 ECS services, no EC2 — Phase 5 |
| `Refresh nodes` | [relay-control-interactive.sh:1405-1406](../../../prototype/gbn-proto/infra/scripts/relay-control-interactive.sh#L1405-L1406) | Same — Phase 5 |

Capabilities that do **not** map to V2 by design (kept here so future readers do not look
for them):
- DHT/gossip seed broadcast semantics — V2 is centralized, no DHT.
- `SeedRelay` / `Publisher` EC2 nodes — V2 is all Fargate.
- `GBN/ScaleTest` CloudWatch namespace — V2 uses `Veritas/Conduit`.

---

## 3. Execution Rules

### 3.1 Phase Sequencing Rule

Each Pass 2 phase must finish with:
- all phase-specific validation tests passing
- the existing GBN-PROTO-006 test suite passing
- the V1 regression suite passing
- a clean diff against V1 protected paths (see §3.2)

No later Pass 2 phase may begin until the current phase has been explicitly approved.

### 3.2 V1 Preservation Rule

Pass 2 must preserve the published V1 Lattice baseline exactly. The V1 no-touch paths from
GBN-PROTO-005 / GBN-PROTO-006 remain in effect:

- `prototype/gbn-proto/Cargo.toml`
- `prototype/gbn-proto/Cargo.lock`
- `prototype/gbn-proto/crates/gbn-protocol/**`
- `prototype/gbn-proto/crates/mcn-crypto/**`
- `prototype/gbn-proto/crates/mcn-router-sim/**`
- `prototype/gbn-proto/crates/mpub-receiver/**`
- `prototype/gbn-proto/crates/proto-cli/**`
- `prototype/gbn-proto/Dockerfile.relay`
- `prototype/gbn-proto/Dockerfile.publisher`
- `prototype/gbn-proto/docker-compose.scale-test.yml`
- `prototype/gbn-proto/infra/cloudformation/**`
- `prototype/gbn-proto/infra/scripts/**`
- `prototype/gbn-proto/tests/integration/**`

Pass 2 reads V1 source for behavioral comparison; it does not modify V1 source.

### 3.3 GBN-PROTO-006 Preservation Rule

Pass 2 must not regress any GBN-PROTO-006 deliverable. Specifically:
- `chain_id` propagation (Phase 7 of Pass 1) must continue to work end-to-end.
- The current public API surface listed in
  [`AuthorityRoute` at api.rs:204-215](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/api.rs#L204-L215)
  must not change behavior; Pass 2 only **adds** routes.
- Bridge control protocol message types defined in
  [`control.rs:191-261`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/control.rs#L191-L261)
  must not change signatures; Pass 2 only adds an admin entry path.

### 3.4 Admin Surface Isolation Rule

All Pass 2 admin endpoints bind to `127.0.0.1:9090` inside the service container. Operator
access is via `aws ecs execute-command --interactive --command "curl ..."`. No public
ingress, no security-group rule, no auth token. This is the smallest blast radius and
matches the user-confirmed decision (§4.1).

### 3.5 Cost Budget Rule

Pass 2 must not raise the steady-state per-stack AWS cost beyond:
- ~$4.50/month additional for CloudWatch custom metrics (Phase 3)
- $0 for admin endpoints (no new ports exposed publicly)
- $0 for the creator library (no new ECS service)

If a phase appears to push the per-stack daily cost above $5 with one stack running, the
phase must stop and revisit design.

### 3.6 V1 Field-Name Preservation Rule

`chain_id` remains the canonical distributed-trace field name across all Pass 2 admin and
metric surfaces. Do not introduce `trace_id`, `request_id`, or `correlation_id` as
substitutes.

### 3.7 PR Granularity Rule

User-confirmed: one PR per phase. Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5 are
landed in order, each behind its own review gate.

---

## 4. Locked Decisions

These were debated during planning and are now locked. Each phase doc references this
section instead of re-deriving the choice.

### 4.1 Admin Endpoint Binding

**Decision:** all admin HTTP routes bind to `127.0.0.1:9090` inside each service
container. No public exposure. No bearer-token auth. Operator access is exclusively via
ECS exec.

**Rationale:** smallest possible blast radius. ECS exec already requires IAM permission,
so the existing AWS auth boundary is reused. No new secret material to rotate.

### 4.2 SendDummy Architecture

**Decision:** create a shared library crate `gbn-bridge-creator/` with no binary and no
service. Add as a dependency to all three service binaries in `gbn-bridge-cli`. Each service
exposes `POST /v1/admin/send-dummy` on the admin port; this triggers a `CreatorClient`
inside that service to perform a real bootstrap-join + frame-upload through the V2 protocol.

**Rationale:** matches the operator's stated requirement that "any node can be taken over
for sending/creating a dummy packet" without paying for a dedicated CreatorService.
Exercises real V2 protocol code paths.

### 4.3 CheckImages Repo Discovery

**Decision:** derive the ECR repo name by parsing the running task's image URI (e.g.
`123.dkr.ecr.us-east-1.amazonaws.com/veritas/publisher-authority:abc → repo
veritas/publisher-authority`). No `--ecr-*-repo` CLI flags.

**Rationale:** matches V1 behavior. Operator does not have to remember repo names.

### 4.4 Script File Strategy

**Decision:** replace [relay-control-interactive-v2.sh](../../../prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh)
in place. Do not ship a sibling file.

**Rationale:** the new script subsumes all functionality of the existing 47-line version.
Coexistence creates two divergent operator experiences.

### 4.5 CloudWatch Metric Namespace

**Decision:** namespace `Veritas/Conduit`. Dimensions `{Service: authority|receiver|bridge,
Stack: <EnvironmentName>}`. Period: 60 seconds.

**Rationale:** distinguishes V2 metrics from any future V1 / Lattice export, while staying
under the same `Veritas/` namespace umbrella for operator discovery.

---

## 5. Phase Summaries

Detailed plan per phase is in each `GBN-PROTO-007-Execution-PhaseN-*.md` file alongside this
document.

### Phase 1 — Read-only Admin HTTP Endpoints
[GBN-PROTO-007-Execution-Phase1-Read-Only-Admin-Endpoints.md](GBN-PROTO-007-Execution-Phase1-Read-Only-Admin-Endpoints.md)

Add three GET endpoints behind `127.0.0.1:9090` in each service binary:
`/v1/admin/bridges`, `/v1/admin/frames`, `/v1/admin/metrics`. Touches
`gbn-bridge-publisher` (new `admin.rs` module) and `gbn-bridge-cli` (mount second
listener in each binary entry point). Foundational — Phases 2, 3, 4 all reuse the admin
port and module.

### Phase 2 — Admin Command Injection Endpoint
[GBN-PROTO-007-Execution-Phase2-Admin-Command-Injection.md](GBN-PROTO-007-Execution-Phase2-Admin-Command-Injection.md)

Add `POST /v1/admin/bridges/{bridge_id}/command` to the publisher-authority binary's admin
listener. Body specifies which `BridgeCommandPayload` variant to inject. Hooks into the
existing in-memory `bridge_id → control-channel-sender` map by adding a
`push_admin_command` method to the existing control module.

### Phase 3 — CloudWatch Metrics Emission
[GBN-PROTO-007-Execution-Phase3-CloudWatch-Metrics-Emission.md](GBN-PROTO-007-Execution-Phase3-CloudWatch-Metrics-Emission.md)

Add `aws-sdk-cloudwatch` dependency to the workspace. Each of the three service binaries
spawns a 60-second emitter task that publishes its metric snapshot to namespace
`Veritas/Conduit`. CloudFormation TaskExecutionRole gains `cloudwatch:PutMetricData` policy.
Receiver and Bridge gain in-memory metric structs analogous to the existing
`AuthorityMetricsSnapshot`.

### Phase 4 — Universal Creator Capability Library
[GBN-PROTO-007-Execution-Phase4-Universal-Creator-Library.md](GBN-PROTO-007-Execution-Phase4-Universal-Creator-Library.md)

New crate `crates/gbn-bridge-creator/` (library only). Implements `CreatorClient` with
methods `bootstrap_join` and `upload_frame`. Dependency added to `gbn-bridge-cli`. Each of
the three service binaries' admin module gains `POST /v1/admin/send-dummy` that uses
`CreatorClient` to perform a real V2 protocol round-trip.

### Phase 5 — Interactive Control Script Port
[GBN-PROTO-007-Execution-Phase5-Interactive-Control-Script-Port.md](GBN-PROTO-007-Execution-Phase5-Interactive-Control-Script-Port.md)

Replace `relay-control-interactive-v2.sh` (47 lines) with ~600 lines structurally adapted
from V1's 1,415-line `relay-control-interactive.sh`. Drops the EC2 / SSM branch (V2 has no
EC2). Calls Phase 1-4 admin endpoints via ECS exec + curl. Calls `aws logs filter-log-events`
with `chain_id` for trace collection.

---

## 6. Out Of Scope For Pass 2

The following are explicitly deferred. Document them here so future readers know they are
known gaps, not oversights.

- **Real Noise handshake hardening for `CreatorClient`** — Phase 4 reuses existing protocol
  primitives; if the V2 Noise implementation has hardening gaps, those are addressed in a
  separate hardening track, not Pass 2.
- **Public admin gateway** — Pass 2 binds admin to localhost only. If the team ever wants
  cross-VPC admin access, that requires its own auth design (bearer tokens, mTLS, IAM-signed
  requests) and is its own track.
- **Multi-region / multi-stack management** — admin endpoints are per-stack. Cross-stack
  operator tooling is out of scope.
- **Web UI** — Pass 2 is shell-based only. Any web UI is a separate track.
- **`prototype/gbn-proto/**` modifications** — V1 stays frozen.
- **Top-level `README.md` modifications** — out of scope.

---

## 7. Validation Strategy

Each phase's detailed plan includes its own validation matrix. The full Pass 2 acceptance
criteria, after Phase 5 lands, are:

1. Deploy a single `gbn-conduit-full-dev` stack via
   [deploy-conduit-full.sh](../../../prototype/gbn-bridge-proto/infra/scripts/deploy-conduit-full.sh).
2. Run `bash prototype/gbn-bridge-proto/infra/scripts/relay-control-interactive-v2.sh`.
3. Walk every menu item; each must succeed or print a meaningful error (no panics, no
   silent failures).
4. `SendDummy` from each of the 5 nodes (Authority, Receiver, all 3 Bridges) must produce
   a valid `chain_id` that appears in `conduit_ingested_frames` and in the log groups of
   the originating-node + assigned-bridge + receiver.
5. `LiveMetrics` displays non-zero data points within 3 minutes of stack deployment.
6. V1 protected-path diff stays clean.
7. GBN-PROTO-006 cargo test suite passes.
8. Per-stack steady-state cost (one stack running 24/7 with template defaults) stays at or
   below $5/day.

### 7.1 Local Kubernetes Validation Results (2026-05-07)

The deferred local validation items from this plan were rerun against the GBN-PROTO-008
k3d stack in WSL Ubuntu.

Passed:

- `cargo fmt --all`
- `cargo check -p gbn-bridge-publisher -p gbn-bridge-cli`
- Fresh local cluster recreation with:
  `VERITAS_K8S_ASSUME_YES=1 VERITAS_K8S_RUN_SMOKE=1 VERITAS_K8S_RUN_CARGO_PERSISTENCE=1 bash infra/scripts/k8s-down.sh && bash infra/scripts/k8s-up.sh`
- The local smoke run registered 3 bridge pods and successfully ran `SendDummy` from the
  authority pod, receiver pod, and each bridge pod. Each response returned a `chain_id`,
  an assigned bridge, and `frames=1`.
- The targeted Postgres persistence recovery test passed against the Kubernetes Postgres
  StatefulSet:
  `postgres_backed_authority_recovers_bridges_bootstrap_catalog_and_upload_sessions`.
- The full V2 workspace test suite passed through a Kubernetes Postgres port-forward via
  `bash infra/scripts/k8s-test-publisher-postgres.sh --workspace`.
- The V1 regression suite passed with `cargo test --workspace` in `prototype/gbn-proto`.

Fixes made during validation:

- The OTLP tracer now installs the tonic batch exporter inside a kept-alive Tokio runtime,
  fixing the prior local pod panic: `there is no reactor running`.
- The OTLP runtime now enables Tokio IO as well as timers, allowing tonic/OTLP gRPC spans
  to reach Tempo instead of failing with transport errors.
- Bridge registration is idempotent when the same pod identity re-registers during a local
  restart, while still rejecting active duplicate bridge IDs with different identities.
- Exit bridges now retry authority/control startup and reconnect dropped control sessions,
  preventing transient rollout connection refusals from terminating bridge pods.
- The local authority Deployment uses `Recreate` so a single in-memory authority is never
  split across old and new pods during same-tag image rollouts.
- `k8s-up.sh` now restarts existing same-tag deployments after image import, while avoiding
  duplicate fresh-cluster rollouts. Restarts are now sequenced authority -> receiver ->
  bridges so local dependency races do not create false failures.
- `k8s-smoke.sh` now handles local k3d kubelet certificate drift and avoids false negatives
  from `grep -q` under `pipefail`.
- The local kube-prometheus-stack values disable operator TLS for this single-node dev
  shape, preventing the missing `kube-prom-admission` secret mount failure.
- WSL Docker now has explicit daemon DNS resolvers configured, avoiding the generated WSL
  DNS tunneling address leaking into Docker bridge containers and causing resolver
  timeouts.
- A corrupted Docker image/build layer was pruned and the base images were re-pulled after
  Docker reported an `unpigz` CRC mismatch.

Completed backend validation after fixing the WSL Docker instability:

- Prometheus `/ready`, Tempo `/ready`, and Loki `/ready` returned healthy responses.
- Prometheus query validation passed:
  - `up{namespace="veritas"}` returned 5 `up=1` series.
  - `conduit_authority_successful_registrations_total` returned value `3`.
  - `conduit_receiver_frames_accepted_total` returned value `5`.
  - `conduit_bridge_frames_forwarded_total` returned 3 bridge series with non-zero traffic
    after smoke traffic.
- Loki validation passed:
  - label discovery included `chain_id`.
  - `{namespace="veritas", chain_id=~".+"}` returned 4 streams and 10 recent entries.
- Tempo validation passed:
  - tag discovery included `chain_id`.
  - distributor metrics showed spans arriving:
    `tempo_distributor_spans_received_total{tenant="single-tenant"} 1605`.
- Docker stayed active after the fix:
  `ActiveState=active`, `SubState=running`, `NRestarts=0`.

Still deferred:

- AWS ECS/CloudWatch acceptance items above remain AWS-only and were not exercised by the
  local k8s validation run.
