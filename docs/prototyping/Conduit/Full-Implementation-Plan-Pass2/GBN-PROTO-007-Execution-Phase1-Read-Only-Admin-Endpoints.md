# GBN-PROTO-007 - Execution Phase 1 Detailed Plan: Read-Only Admin HTTP Endpoints

**Status:** Pending
**Primary Goal:** add three read-only admin HTTP endpoints (`/v1/admin/bridges`,
`/v1/admin/frames`, `/v1/admin/metrics`) bound to `127.0.0.1:9090` inside every service
binary, sharing one new admin module so Phases 2–4 can extend it without restructuring.
**Source Plan:** [GBN-PROTO-007 Execution Plan](GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md)
**Protected V1 Baseline:** [Veritas Lattice 0.1.0](https://github.com/fahdabidi/Veritas/releases/tag/veritas-lattice-0.1.0-baseline)
**Pass 1 Reference:** [GBN-PROTO-006 Phase 8 Detailed Plan](../Full-Implementation-Plan/GBN-PROTO-006-Execution-Phase8-Real-Deployment-Images-And-AWS-Control-Plane.md)
**Starting Conduit Baseline:** `ca7cb1e` (commit `ca7cb1e6` head of main as of 2026-05-07)

---

## 1. Current Repo Findings

| Item | Current Value | Why It Matters |
|---|---|---|
| Service binary crate | [`gbn-bridge-cli`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-cli/Cargo.toml) — produces three binaries (`publisher-authority`, `publisher-receiver`, `exit-bridge`) | every Phase 1 mount point lives in this crate's bin sources |
| Service-side library | [`gbn-bridge-publisher`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/Cargo.toml) — owns storage, control, metrics, api | the new `admin.rs` module is added here for reuse across all three binaries |
| Public route enum | [`AuthorityRoute` at api.rs:204-215](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/api.rs#L204-L215) — only `/healthz`, `/readyz`, and protocol routes | Phase 1 must not change this enum's existing variants; only append admin variants |
| Existing metrics struct | [`AuthorityMetricsSnapshot` at metrics.rs:1-13](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/metrics.rs#L1-L13) — 10 `u64` counters, `derive(Debug, Clone, Copy, Default, PartialEq, Eq)` | Phase 1 must add `Serialize` derive without changing fields |
| Existing storage record | [`IngestedFrameRecord` at storage.rs:120-125](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/storage.rs#L120-L125) | already used internally; Phase 1 ensures it has `Serialize` |
| Existing bridge record | `BridgeRecord` in `gbn-bridge-publisher/src/storage/schema.rs` (read implementation lives in storage.rs) | Phase 1 confirms `Serialize` |
| Service runtime image base | `debian:bookworm-slim` per [Dockerfile.publisher-authority:13](../../../prototype/gbn-bridge-proto/Dockerfile.publisher-authority#L13), [Dockerfile.bridge:11](../../../prototype/gbn-bridge-proto/Dockerfile.bridge#L11) | `curl` is **not** in `debian:bookworm-slim` by default; Phase 1 must `apt-get install curl` in the runtime stage |
| ECS exec capability | [`EnableExecuteCommand: true`](../../../prototype/gbn-bridge-proto/infra/cloudformation/conduit-full-stack.yaml#L457) on every service | operator can already shell into each task once Phase 1 lands |
| Network model | each service container has a private IP and binds public-protocol ports on `0.0.0.0`; loopback interface always available | binding admin on `127.0.0.1:9090` is reachable only from `aws ecs execute-command` |

---

## 2. Review Summary

Phase 1 is the foundational backend phase. Phases 2 (command injection), 3 (CloudWatch
emission lookups for verification), and 4 (`POST /v1/admin/send-dummy`) all assume an
admin HTTP listener already exists with a clean module to extend. If Phase 1 is done well,
each later phase is a single-file change in `admin.rs`.

| Gap | Why It Matters | Resolution For Phase 1 |
|---|---|---|
| No admin HTTP surface | operator cannot read bridge registry, frames, or metrics from outside the database | add three GET routes behind a localhost-only listener |
| No reusable admin module | next phases would each invent their own pattern | place all admin handlers in one new `gbn-bridge-publisher::admin` module |
| `AuthorityMetricsSnapshot` is not `Serialize` | endpoint cannot return JSON | add `derive(Serialize)` to the snapshot struct |
| Runtime image lacks `curl` | operator cannot probe localhost:9090 from ECS exec | add `curl` to all three Dockerfiles' final stage |
| Listener wiring is per-binary | all three binaries need the new admin server, not just authority | add a small helper in `gbn-bridge-publisher::admin` that takes a `tokio::TcpListener` and mounts the routes; each binary calls it once |

Phase 1 must not introduce any new authentication mechanism, bearer-token check, or public
ingress. Locking the listener to `127.0.0.1` is the entire authorization story.

---

## 3. Scope Lock

### In Scope

- new module `gbn-bridge-publisher/src/admin.rs`
- three GET handlers: `list_bridges`, `list_frames`, `metrics_snapshot`
- extension of `AuthorityRoute` enum with three admin variants
- `derive(Serialize)` on `AuthorityMetricsSnapshot`, `IngestedFrameRecord`, and `BridgeRecord`
- second `tokio::TcpListener` bound to `127.0.0.1:9090` mounted in each of the three
  service binaries' `main` function
- `curl` added to all three Dockerfile runtime stages
- a small unit test under `gbn-bridge-publisher/tests/admin_routes.rs` exercising each
  handler with an in-memory storage stub

### Out Of Scope

- any `POST` admin route (Phase 2 and 4 add those)
- any auth/token check
- any public exposure of port 9090 (no security-group rule, no port mapping)
- modifying any existing protocol route's behavior
- modifying the receiver or exit-bridge binaries' protocol behavior
- Cargo.toml changes other than possibly enabling existing `serde` feature flags

---

## 4. Preflight Gates

Phase 1 must not begin code edits until all of these are true:

1. `git status` is clean on `main` at HEAD `ca7cb1e` or later.
2. `cargo fmt --all --check` passes on the V2 workspace.
3. `cargo test --workspace` passes on the V2 workspace.
4. V1 protected paths show no local diff.
5. The service Dockerfiles still build and produce `publisher-authority`,
   `publisher-receiver`, `exit-bridge` images.
6. The CloudFormation template
   [conduit-full-stack.yaml](../../../prototype/gbn-bridge-proto/infra/cloudformation/conduit-full-stack.yaml)
   still passes `aws cloudformation validate-template`.

---

## 5. File-by-File Specification

### 5.1 New file: `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/admin.rs`

**Purpose:** module containing the three Phase 1 handlers and a `serve_admin_listener`
helper used by each service binary.

**Functional Spec:**

```rust
//! Admin HTTP surface bound to 127.0.0.1:9090 inside every Conduit V2 service container.
//!
//! Operator access is exclusively via `aws ecs execute-command --interactive --command
//! "curl http://127.0.0.1:9090/v1/admin/<route>"`. No public ingress, no auth.

use std::net::SocketAddr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::metrics::AuthorityMetricsSnapshot;
use crate::storage::{BridgeRecord, IngestedFrameRecord, Storage};

#[derive(Debug, Clone, Deserialize)]
pub struct FramesQuery {
    pub chain_id: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgesResponse {
    pub bridges: Vec<BridgeRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FramesResponse {
    pub frames: Vec<IngestedFrameRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricsResponse {
    pub authority: AuthorityMetricsSnapshot,
}

pub struct AdminState {
    pub storage: Arc<Storage>,
    pub metrics: Arc<tokio::sync::Mutex<crate::metrics::AuthorityMetrics>>,
}

/// Spawn the admin server on the supplied listener.
///
/// Caller is responsible for binding the listener (always to 127.0.0.1:9090 in production
/// service binaries; tests may bind to a free port).
///
/// Returns a `JoinHandle` that can be aborted on shutdown.
pub async fn serve_admin_listener(
    listener: TcpListener,
    state: AdminState,
) -> tokio::task::JoinHandle<()> { /* axum::Router or hyper service mounting GET routes only */ }

async fn list_bridges(state: &AdminState) -> Result<BridgesResponse, AdminError>;
async fn list_frames(state: &AdminState, query: FramesQuery) -> Result<FramesResponse, AdminError>;
async fn metrics_snapshot(state: &AdminState) -> MetricsResponse;
```

**Routing table for Phase 1:**

| Method | Path | Handler |
|---|---|---|
| GET | `/v1/admin/bridges` | `list_bridges` |
| GET | `/v1/admin/frames` | `list_frames` (query string: `chain_id`, `limit`) |
| GET | `/v1/admin/metrics` | `metrics_snapshot` |

The handlers must wrap their results in `axum::Json` (or hyper equivalent) and return HTTP
200 with `Content-Type: application/json` on success, HTTP 4xx with a JSON `{error}` body
on operator error (bad query string), HTTP 5xx with a JSON `{error}` body on server error.

**Storage layer reuse:** `list_bridges` calls a new `Storage::list_bridges()` method
(see §5.3). `list_frames` calls a new `Storage::list_frames(chain_id, limit)`.
`metrics_snapshot` reads from the in-memory `AuthorityMetrics` already maintained at
[metrics.rs:16-18](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/metrics.rs#L16-L18).

### 5.2 Modify: `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/api.rs`

Existing
[`AuthorityRoute` at lines 204-215](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/api.rs#L204-L215)
is currently:

```rust
pub enum AuthorityRoute {
    Healthz,
    Readyz,
    BridgeRegister,
    BridgeHeartbeat,
    BridgeProgress,
    CreatorCatalog,
    BootstrapJoin,
    ReceiverOpen,
    ReceiverFrame,
    ReceiverClose,
}
```

After Phase 1, append three variants in this order at line 215 (just before the closing
brace):

```rust
    AdminBridges,
    AdminFrames,
    AdminMetrics,
```

In the corresponding `path()` impl at
[lines 217-232](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/api.rs#L217-L232),
add at line 230 (just before the closing brace of the match):

```rust
            Self::AdminBridges => "/v1/admin/bridges",
            Self::AdminFrames => "/v1/admin/frames",
            Self::AdminMetrics => "/v1/admin/metrics",
```

No other change to this file.

### 5.3 Modify: `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/storage.rs`

Add two new public methods to the `Storage` impl block. Approximate location: after the
existing `IngestedFrameRecord` and `UploadSessionRecord` definitions (around line 140).

```rust
impl Storage {
    /// List all currently registered bridges. Returns an unordered Vec.
    pub async fn list_bridges(&self) -> Result<Vec<BridgeRecord>, StorageError> {
        // SELECT bridge_id, public_key, last_heartbeat_at, lease_expires_at, status
        // FROM conduit_bridges;
    }

    /// List ingested frames, optionally filtered by chain_id, with optional limit.
    /// If limit is None, applies a default cap of 1000 to bound response size.
    pub async fn list_frames(
        &self,
        chain_id: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<IngestedFrameRecord>, StorageError> {
        // SELECT chain_id, via_bridge_id, frame, received_at_ms
        // FROM conduit_ingested_frames
        // WHERE ($1::text IS NULL OR chain_id = $1)
        // ORDER BY received_at_ms DESC
        // LIMIT $2;
    }
}
```

Existing `IngestedFrameRecord` at line 120 currently lacks `derive(Serialize)`. Add it:

```rust
// Before
#[derive(Debug, Clone)]
pub struct IngestedFrameRecord { ... }

// After
#[derive(Debug, Clone, Serialize)]
pub struct IngestedFrameRecord { ... }
```

`BridgeRecord` (location: confirm in `storage/schema.rs`) gains the same derive.

### 5.4 Modify: `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/metrics.rs`

Existing snapshot at
[lines 1-13](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/metrics.rs#L1-L13)
adds `Serialize`:

```rust
// Before line 1
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AuthorityMetricsSnapshot {

// After
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct AuthorityMetricsSnapshot {
```

Add `use serde::Serialize;` at the top of the file.

### 5.5 Modify: `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/lib.rs`

Add the new module alongside existing module declarations:

```rust
pub mod admin;
```

### 5.6 Modify: `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/Cargo.toml`

Add to `[dependencies]` if not already present (confirm with current file):

- `tokio = { workspace = true, features = ["net", "rt", "macros"] }`
- `axum = { workspace = true }` or `hyper` if axum is not yet a workspace dep
- `serde = { workspace = true, features = ["derive"] }` (already present per current file)

If `axum` is not yet in the workspace, add it to the workspace `Cargo.toml`'s
`[workspace.dependencies]` table at version `0.7` (confirm latest stable at implementation
time).

### 5.7 Modify: each service binary entry point

The three binary entry points live under `crates/gbn-bridge-cli/src/bin/` (file names
inferred from `--bin <name>` in Dockerfiles). Confirm exact file paths during
implementation:
- `crates/gbn-bridge-cli/src/bin/publisher-authority.rs`
- `crates/gbn-bridge-cli/src/bin/publisher-receiver.rs`
- `crates/gbn-bridge-cli/src/bin/exit-bridge.rs`

In each `main` (or its async equivalent), after the existing public-port listener is bound
and the service's `Storage` / metrics state is built, add:

```rust
let admin_addr: SocketAddr = "127.0.0.1:9090".parse().unwrap();
let admin_listener = tokio::net::TcpListener::bind(admin_addr).await
    .expect("admin port 9090 already bound — refusing to start");
let admin_state = gbn_bridge_publisher::admin::AdminState {
    storage: storage.clone(),
    metrics: metrics.clone(),
};
let _admin_handle = gbn_bridge_publisher::admin::serve_admin_listener(
    admin_listener,
    admin_state,
).await;
```

The `_admin_handle` must live for the same duration as the public listener — typically
held by the service main loop and awaited at shutdown.

For binaries that do not own a `Storage` (e.g. `exit-bridge` may not have a Postgres
handle), `AdminState` may degrade gracefully: set `storage: None` and have the bridge-only
handlers (none in Phase 1, but Phases 2 and 4 will add them) handle the `None` case. The
authority binary always has `Storage`. Receiver and bridge handlers in Phase 1 only serve
`/v1/admin/metrics` — the bridge and receiver versions of the snapshot are added in
Phase 3 alongside CloudWatch emission. Until Phase 3 lands, receiver and bridge return
HTTP 501 for `/v1/admin/bridges` and `/v1/admin/frames` (those are authority-only by
definition) and return a stub metrics object with all zeros for `/v1/admin/metrics`.

### 5.8 Modify: `prototype/gbn-bridge-proto/Dockerfile.publisher-authority`

Existing runtime stage at
[lines 13-20](../../../prototype/gbn-bridge-proto/Dockerfile.publisher-authority#L13-L20):

```dockerfile
FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates \
  && rm -rf /var/lib/apt/lists/*
```

Add `curl` to the package list:

```dockerfile
FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl \
  && rm -rf /var/lib/apt/lists/*
```

### 5.9 Modify: `prototype/gbn-bridge-proto/Dockerfile.publisher-receiver`

Same change as §5.8 applied to the receiver Dockerfile's runtime stage.

### 5.10 Modify: `prototype/gbn-bridge-proto/Dockerfile.bridge`

Existing runtime stage at
[lines 11-19](../../../prototype/gbn-bridge-proto/Dockerfile.bridge#L11-L19) currently has
no `apt-get install` step (only `useradd`). Add one before the `useradd`:

```dockerfile
FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl \
  && rm -rf /var/lib/apt/lists/*
RUN useradd --create-home --uid 10001 veritas
```

### 5.11 New file: `prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/tests/admin_routes.rs`

Integration test exercising each handler against an in-memory or test-container Postgres.

```rust
//! Phase 1 admin endpoint coverage.
//!
//! Each test binds the admin listener to an OS-assigned port (127.0.0.1:0), spawns the
//! handler, and uses reqwest to GET each route. Asserts on JSON shape, not contents.

#[tokio::test]
async fn list_bridges_returns_empty_when_storage_empty() { ... }

#[tokio::test]
async fn list_frames_filters_by_chain_id() { ... }

#[tokio::test]
async fn metrics_snapshot_returns_serialized_snapshot() { ... }

#[tokio::test]
async fn list_frames_caps_at_default_limit_when_unspecified() { ... }
```

The test fixture sets up an `AdminState` with a `Storage` backed by either a `pg-embed`
test container or an in-memory mock implementing the same trait. Reuse whatever pattern
GBN-PROTO-006 Phase 2 adopted.

---

## 6. Module And Asset Ownership Locked In Phase 1

| Asset | Responsibility |
|---|---|
| `gbn-bridge-publisher/src/admin.rs` | all admin HTTP handlers, today and in Phases 2 / 4 |
| `gbn-bridge-publisher::admin::AdminState` | shared state container; grows as later phases need new fields (e.g. control-channel sender map for Phase 2) |
| `gbn-bridge-publisher::admin::serve_admin_listener` | single mount point; grows new routes via `Router::route` calls inside the helper |
| Each binary's `main` | calls `serve_admin_listener` exactly once after building public service state |

No other module owns admin endpoint routing. If Phase 2 / 3 / 4 needs a new helper, it
goes in `admin.rs`, not in a new module.

---

## 7. Dependency And Implementation Policy

### Recommended Dependencies

- `axum` 0.7+ for HTTP routing (or whatever is already in workspace). If hyper-only is
  preferred to avoid a new dep, hyper directly is acceptable.
- `tokio` already in workspace.
- `serde` already in workspace.

### Bias

- prefer reusing existing storage methods over adding new ones; if a query already exists
  for a different purpose, expose it rather than duplicate.
- prefer unit tests over integration tests where possible; integration test only for
  end-to-end JSON shape assertion.

### Forbidden

- adding any auth/token logic; if the user later asks for non-localhost admin access, that
  is a separate phase.
- changing the `AuthorityRoute` enum order or behavior of existing variants.
- exposing port 9090 in the CloudFormation template.

---

## 8. Validation

After Phase 1 lands:

1. `cargo fmt --all --check` and `cargo test --workspace` pass.
2. New tests in `tests/admin_routes.rs` pass.
3. Build and push the three updated container images.
4. Deploy `gbn-conduit-full-dev` stack via existing deploy script.
5. Wait for ECS services to reach `runningCount == desiredCount`.
6. Walk every route on every node:
   - `aws ecs execute-command --cluster <c> --task <t> --container publisher-authority
     --interactive --command "curl -s http://127.0.0.1:9090/v1/admin/bridges"` returns JSON
     `{"bridges": []}` on a freshly deployed stack (no bridges have heartbeat-registered yet).
   - After bridges register, repeat — returns up to 3 bridge records.
   - Same for `/v1/admin/frames` (returns empty until SendDummy runs in Phase 4).
   - Same for `/v1/admin/metrics` on each node — returns JSON with all 10 counter fields.
7. Confirm `/v1/admin/metrics` on receiver and bridge containers returns a stub all-zero
   snapshot (Phase 3 fills these in with real receiver/bridge counters).
8. Confirm a `nmap -p 9090 <task-private-ip>` from another VPC host fails (port not
   exposed).
9. V1 protected-path diff stays clean.

---

## 9. Open Questions Carried Into Implementation

1. **`axum` vs hyper-only** — confirm the current workspace router preference. If neither
   is in workspace, hyper-only minimizes new deps but slightly more boilerplate.
2. **Storage handle in non-authority binaries** — confirm whether `publisher-receiver` and
   `exit-bridge` already hold a `Storage` handle. If yes, all three binaries can serve
   `list_bridges` / `list_frames` (each reads the same shared DB). If no, those routes
   return HTTP 501 from non-authority binaries until Phase 3.
3. **Default frame limit** — current spec uses 1000. Confirm this is acceptable for the
   biggest expected `chain_id` cardinality in a single SendDummy run.
4. **Bridge bin location** — confirm `crates/gbn-bridge-cli/src/bin/exit-bridge.rs` is the
   correct path; the Dockerfile uses `--bin exit-bridge` which may be a Cargo-target
   alias declared explicitly in `gbn-bridge-cli/Cargo.toml`.
