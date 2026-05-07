# GBN-PROTO-007 - Execution Phase 4 Detailed Plan: Universal Creator Capability Library

**Status:** Completed
**Primary Goal:** introduce a shared library crate `gbn-bridge-creator` that implements
the creator side of the V2 bootstrap + frame-upload protocol, link it into all three
Conduit V2 service binaries, and expose `POST /v1/admin/send-dummy` on every node so any
of the 5 deployed tasks (Authority, Receiver, all 3 Bridges) can act as a test creator on
operator demand. Returns a `chain_id` that downstream tracing in Phase 5 can correlate
across logs.
**Source Plan:** [GBN-PROTO-007 Execution Plan](GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md)

---

## 1. Current Repo Findings

| Item | Current Value | Why It Matters |
|---|---|---|
| Existing protocol primitives | [`bootstrap.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/bootstrap.rs), [`session.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/session.rs), [`punch.rs`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/punch.rs) | the wire types are already defined; Phase 4 reuses them — does not redefine |
| Existing runtime crate | [`gbn-bridge-runtime`](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/Cargo.toml) | has reusable trace and signing helpers; Phase 4 depends on it |
| Existing public Authority routes | [`AuthorityRoute` at api.rs:204-215](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-publisher/src/api.rs#L204-L215) — includes `BootstrapJoin`, `CreatorCatalog`, `ReceiverFrame` | the creator client invokes these as a real client would |
| Existing creator code | none — no `gbn-bridge-creator` crate exists | Phase 4 creates it from scratch |
| Existing in-process test client | various unit tests instantiate creator-like flows inline | Phase 4 extracts the common pattern into a reusable `CreatorClient` |
| Existing chain_id helpers | [runtime trace.rs](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-runtime/src/trace.rs) — generates and validates root chain_ids | reused so `CreatorClient` mints chain_ids in the same way as production creators |
| Bridge UDP punch port | [`UdpPunchPort` parameter (default 443)](../../../prototype/gbn-bridge-proto/infra/cloudformation/conduit-full-stack.yaml#L53-L58) | creator must reach assigned bridge over UDP after bootstrap |

---

## 2. Review Summary

The user's stated requirement is critical to this phase's design:

> "Instead of a dedicated 'Creator' service. The desired goal is that any node can be
> taken over for sending/creating a dummy packet. We don't want a 'dedicated' node for
> this task. And then as the packet is sent, we want to of course also monitor the trace
> and completion like we did by inspecting the tracelogs from each node that was used to
> send the packet."

This drives the library-not-service design.

| Gap | Why It Matters | Resolution For Phase 4 |
|---|---|---|
| V2 has no creator code | Phase 5's `SendDummy` cannot synthesize a packet | new `gbn-bridge-creator` library crate |
| V1 deploys a CreatorService | matching V1 literally would add a 24/7 Fargate task | do not deploy a creator service; instead, link the creator library into existing service binaries |
| Each node may need to act as creator | binding creator capability to a single node makes it a SPOF for testing | expose `POST /v1/admin/send-dummy` on **every** binary |
| Creator → Bridge path is UDP | creator library must implement UDP punch + frame upload | reuse `gbn-bridge-runtime` punch helpers if present; otherwise add minimal UDP send logic in the creator crate |
| Operator needs to trace the resulting packet | chain_id must be returned + must appear in originator/bridge/receiver logs | response payload includes `chain_id`, `assigned_bridge_id`, `elapsed_ms`; existing chain_id propagation (Pass 1 Phase 7) ensures it appears in logs |

### Circular bridge assignment

If a Bridge node acts as creator and Authority's catalog assigns the same bridge as the
egress, the path collapses to `X → X → Receiver`. **Decision (locked):** accept the
collapse for simplicity. The packet still exercises the upload + receiver ingest paths
end-to-end. If operator testing finds this hides bugs, a follow-up phase can add an
optional `?exclude_self=1` query parameter.

---

## 3. Scope Lock

### In Scope

- new crate `prototype/gbn-bridge-proto/crates/gbn-bridge-creator/` (library only)
- exports `CreatorClient` with `bootstrap_join`, `upload_frame`, and a convenience
  `send_dummy` method
- reuses `gbn-bridge-protocol` types verbatim (no protocol fork)
- reuses `gbn-bridge-runtime` signing + chain_id helpers
- new POST handler in `gbn-bridge-publisher::admin`: `inject_send_dummy`
- new route variant `AuthorityRoute::AdminSendDummy` with path `/v1/admin/send-dummy`
- mounted on every binary's admin listener (Authority / Receiver / Bridge all serve it)
- integration test exercising send-dummy from each role
- Cargo workspace changes to register the new crate

### Out Of Scope

- new ECS service or new container image
- new ECR repo
- modifying existing protocol types
- exposing `/v1/admin/send-dummy` publicly (still localhost-only via Phase 1's listener)
- a separate creator CLI binary (the user explicitly rejected this; library is shared
  across services instead)
- `?exclude_self=1` exclusion logic (deferred per circular-collapse decision)

---

## 4. Preflight Gates

1. Phase 1 has landed; admin listener and module exist.
2. Phase 2 / Phase 3 may or may not have landed; Phase 4 does not depend on them.
3. The V2 protocol crate compiles and passes its existing tests.
4. The `chain_id` propagation from GBN-PROTO-006 Phase 7 is intact.
5. V1 protected-path diff is clean.

---

## 5. File-by-File Specification

### 5.1 New crate: `prototype/gbn-bridge-proto/crates/gbn-bridge-creator/`

#### `Cargo.toml`

```toml
[package]
name = "gbn-bridge-creator"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
ed25519-dalek = { workspace = true }
gbn-bridge-protocol = { path = "../gbn-bridge-protocol" }
gbn-bridge-runtime = { path = "../gbn-bridge-runtime" }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true, features = ["net", "rt", "macros"] }
tracing = { workspace = true }
url = { workspace = true }

[dev-dependencies]
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
```

#### `src/lib.rs`

```rust
//! Creator-side V2 protocol implementation, library-only.
//!
//! The creator role is "the entity that originates a payload and pushes it through an
//! assigned bridge to the receiver". This crate is linked into every Conduit V2 service
//! binary so any of them can be told to act as a creator on operator demand.

pub mod client;
pub mod error;
pub mod session;

pub use client::CreatorClient;
pub use error::CreatorError;
pub use session::CreatorSession;
```

#### `src/error.rs`

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CreatorError {
    #[error("authority bootstrap failed: {0}")]
    BootstrapFailed(String),
    #[error("no bridge assigned by authority")]
    NoBridgeAssigned,
    #[error("frame upload to bridge failed: {0}")]
    FrameUploadFailed(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("signing error: {0}")]
    Signing(String),
}
```

#### `src/session.rs`

```rust
use gbn_bridge_protocol::{BridgeId, ChainId};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct CreatorSession {
    pub session_id: uuid::Uuid,
    pub bridge_id: BridgeId,
    pub bridge_address: String,    // ip:udp_port
    pub bootstrap_chain_id: ChainId,
    pub started_at: Instant,
}
```

#### `src/client.rs`

```rust
use std::time::{Duration, Instant};

use ed25519_dalek::{Signer, SigningKey};
use gbn_bridge_protocol::{BridgeData, ChainId};
use gbn_bridge_runtime::trace;
use url::Url;

use crate::error::CreatorError;
use crate::session::CreatorSession;

/// Synchronous-construction, async-method creator client.
///
/// One `CreatorClient` may be reused for many `bootstrap_join + upload_frame` round trips.
pub struct CreatorClient {
    signing_key: SigningKey,
    timeout: Duration,
}

impl CreatorClient {
    pub fn new(signing_key: SigningKey) -> Self {
        Self { signing_key, timeout: Duration::from_secs(15) }
    }

    pub fn with_timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }

    /// Perform `/v1/bootstrap/join` against the authority and receive a bridge assignment.
    pub async fn bootstrap_join(
        &self,
        authority_url: &Url,
    ) -> Result<CreatorSession, CreatorError> {
        // 1. Mint a fresh root chain_id via runtime trace helper.
        // 2. Build a CreatorJoinRequest (from gbn-bridge-protocol::bootstrap).
        // 3. Sign with self.signing_key.
        // 4. POST to authority_url + AuthorityRoute::BootstrapJoin.path()
        //    with a stdlib reqwest-equivalent or hyper client.
        // 5. Parse CreatorBootstrapResponse → BridgeSeedAssign.
        // 6. Build CreatorSession {session_id, bridge_id, bridge_address, bootstrap_chain_id}.
        // 7. Return.
    }

    /// Upload one synthetic frame through the assigned bridge.
    ///
    /// Returns the chain_id that downstream services will see for this frame.
    /// For a fresh upload this is typically equal to session.bootstrap_chain_id; if the
    /// caller wants a per-frame sub-chain, add a child id derivation step (out of scope).
    pub async fn upload_frame(
        &self,
        session: &CreatorSession,
        frame_bytes: Vec<u8>,
    ) -> Result<ChainId, CreatorError> {
        // 1. Wrap frame_bytes into a BridgeData (from gbn-bridge-protocol::session)
        //    carrying session.bootstrap_chain_id.
        // 2. Sign and serialize.
        // 3. UDP send to session.bridge_address (port = configured punch port).
        // 4. Wait for BridgeAck (with timeout = self.timeout) or fail.
        // 5. Return session.bootstrap_chain_id (or ack-returned chain_id if it differs).
    }

    /// Convenience: bootstrap_join + upload_frame in one call. Used by the admin handler.
    pub async fn send_dummy(
        &self,
        authority_url: &Url,
        size: usize,
    ) -> Result<SendDummyResult, CreatorError> {
        let started = Instant::now();
        let session = self.bootstrap_join(authority_url).await?;
        let frame = self::synthesize_frame(size);
        let chain_id = self.upload_frame(&session, frame).await?;
        Ok(SendDummyResult {
            chain_id,
            assigned_bridge_id: session.bridge_id,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SendDummyResult {
    pub chain_id: ChainId,
    pub assigned_bridge_id: gbn_bridge_protocol::BridgeId,
    pub elapsed_ms: u64,
}

fn synthesize_frame(size: usize) -> Vec<u8> {
    // Deterministic fill so test packets are recognizable in dumps.
    let mut buf = Vec::with_capacity(size);
    for i in 0..size { buf.push((i % 251) as u8); }
    buf
}
```

#### `tests/integration.rs`

```rust
//! Integration test: stand up a fake authority + bridge + receiver in-process,
//! drive CreatorClient through a full send_dummy, assert chain_id appears at every hop.

#[tokio::test]
async fn send_dummy_round_trips_through_fake_topology() { ... }

#[tokio::test]
async fn bootstrap_join_returns_no_bridge_when_catalog_empty() { ... }

#[tokio::test]
async fn upload_frame_times_out_when_bridge_unreachable() { ... }
```

### 5.2 Modify: workspace `Cargo.toml`

Add the new crate to the `members` list under `[workspace]`:

```toml
[workspace]
members = [
    # ... existing members ...
    "crates/gbn-bridge-creator",
]
```

### 5.3 Modify: `gbn-bridge-publisher/Cargo.toml`

Add to `[dependencies]`:

```toml
gbn-bridge-creator = { path = "../gbn-bridge-creator" }
```

### 5.4 Modify: `gbn-bridge-publisher/src/admin.rs` (extends Phase 1 module)

Add a new POST handler:

```rust
use gbn_bridge_creator::{CreatorClient, SendDummyResult};

#[derive(Debug, Clone, Deserialize)]
pub struct SendDummyRequest {
    pub size: Option<usize>,    // default 512
}

async fn inject_send_dummy(
    state: &AdminState,
    request: SendDummyRequest,
) -> Result<SendDummyResult, AdminError> {
    let size = request.size.unwrap_or(512);
    let signing_key = state.local_signing_key()
        .ok_or(AdminError::NotSupportedOnThisBinary)?;
    let authority_url = state.authority_url();
    let client = CreatorClient::new(signing_key);
    client.send_dummy(&authority_url, size).await
        .map_err(AdminError::from)
}
```

The handler is mounted in `serve_admin_listener`:

```rust
router = router.route(
    "/v1/admin/send-dummy",
    post(inject_send_dummy_handler),
);
```

`AdminState` gains two new methods (or fields):

```rust
pub struct AdminState {
    pub storage: Arc<Storage>,
    pub metrics: Arc<tokio::sync::Mutex<crate::metrics::AuthorityMetrics>>,
    pub control: Option<Arc<BridgeControlManager>>,
    pub local_signing_key: Option<SigningKey>,    // each binary supplies its own; node-as-creator uses it
    pub authority_internal_url: Url,              // each binary knows the authority private DNS URL
}
```

### 5.5 Modify: `gbn-bridge-publisher/src/api.rs`

Append one variant to `AuthorityRoute`:

```rust
    AdminSendDummy,
```

In the `path()` impl:

```rust
            Self::AdminSendDummy => "/v1/admin/send-dummy",
```

### 5.6 Modify: each service binary `main`

Each of the three binaries (`publisher-authority`, `publisher-receiver`, `exit-bridge`)
must construct an `AdminState` with `local_signing_key` populated and `authority_internal_url`
set:

- **publisher-authority** binary already holds a publisher signing key — reuse it.
  `authority_internal_url = "http://127.0.0.1:8080"` (calls itself).
- **publisher-receiver** binary holds receiver-side keys; if a separate signing key is
  needed for creator, generate an ephemeral one at startup or read from a new env var
  `GBN_BRIDGE_CREATOR_SIGNING_KEY_HEX`.
- **exit-bridge** binary already holds a bridge signing seed — reuse it (it can sign as a
  creator; the protocol does not forbid bridges from being creators).
  `authority_internal_url = $GBN_BRIDGE_AUTHORITY_URL` (env var already set per
  [conduit-full-stack.yaml:425](../../../prototype/gbn-bridge-proto/infra/cloudformation/conduit-full-stack.yaml#L425)).

### 5.7 Modify: `prototype/gbn-bridge-proto/infra/cloudformation/conduit-full-stack.yaml`

If the receiver binary needs an ephemeral creator signing key (preferred) or a configured
one, add an env var to the ReceiverTaskDefinition Environment list at line 389:

```yaml
- Name: GBN_BRIDGE_CREATOR_SIGNING_MODE
  Value: ephemeral
```

`ephemeral` mode means the binary generates a fresh key at startup; this is fine because
admin send-dummy is for testing only and identity persistence is not required.

For the bridge binary, a similar env var ensures the existing
`GBN_BRIDGE_BRIDGE_SIGNING_SEED_HEX` secret is reusable as the creator key.

### 5.8 New file: `gbn-bridge-publisher/tests/admin_send_dummy.rs`

```rust
//! Phase 4 admin send-dummy coverage.

#[tokio::test]
async fn send_dummy_from_authority_returns_chain_id() { ... }

#[tokio::test]
async fn send_dummy_from_receiver_returns_chain_id() { ... }

#[tokio::test]
async fn send_dummy_from_bridge_returns_chain_id_even_when_assigned_to_self() { ... }

#[tokio::test]
async fn send_dummy_chain_id_appears_in_admin_frames_response() { ... }
```

The last test is the key correlation test: after `send_dummy` returns chain_id X, calling
Phase 1's `GET /v1/admin/frames?chain_id=X` returns at least one row.

---

## 6. Validation

After Phase 4 lands:

1. `cargo fmt --all --check` and `cargo test --workspace` pass.
2. New crate builds and its integration test passes.
3. New admin handler tests pass.
4. Build and push the three updated container images.
5. Deploy `gbn-conduit-full-dev` stack.
6. From inside each of the 5 running tasks, run:
   ```sh
   curl -X POST -H 'Content-Type: application/json' \
     http://127.0.0.1:9090/v1/admin/send-dummy \
     -d '{"size": 512}'
   ```
   Each call returns `{"chain_id":"...","assigned_bridge_id":"...","elapsed_ms":N}`.
7. For each chain_id returned, confirm `GET /v1/admin/frames?chain_id=<id>` on the
   authority returns the row.
8. For each chain_id, confirm `aws logs filter-log-events --log-group <originator-log>
   --filter-pattern '<chain_id>'` returns log lines from the originating node.
9. Same filter against the assigned bridge's log group returns lines.
10. Same filter against the receiver's log group returns lines.
11. Update the Status Trackers table in
    [GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md](GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md)
    before starting the next phase.

---

## 7. Open Questions Carried Into Implementation

1. **Creator signing key for receiver** — generate ephemeral on startup vs require a
   provisioned key in Secrets Manager. Recommendation: ephemeral. The receiver-as-creator
   is a test-only path and identity persistence is not required.
2. **Frame synthesis content** — current spec uses deterministic fill. Should it be random
   to avoid CRC collisions across runs? Recommendation: deterministic fill (easier debug);
   add a `seed` parameter if randomness is later needed.
3. **`?exclude_self=1` for bridge nodes** — deferred per §2 decision.
4. **Per-frame sub-chain ids** — `upload_frame` currently returns the bootstrap chain_id.
   If V2 protocol later supports per-frame derived ids, the return becomes a vector.
5. **Reqwest vs hyper for HTTP** — the creator client makes one HTTPS-ish call to authority.
   Pick whichever is already in workspace; do not add a new HTTP client dep.

---

## 8. Implementation Notes

- Implemented `gbn-bridge-creator` as a library-only crate with `CreatorClient`,
  `CreatorSession`, `SendDummyResult`, and a small UDP upload envelope used between the
  creator client and the assigned bridge.
- The creator bootstrap path posts the existing signed `/v1/bootstrap/join` request shape
  to the authority and verifies the signed `BootstrapJoinReply`.
- The upload path sends `BridgeOpen`, `BridgeData`, and `BridgeClose` over UDP to the
  assigned bridge. The bridge listener forwards through the existing bridge runtime, so the
  receiver ingest routes still see frames signed and forwarded by the assigned bridge.
- The implementation intentionally avoids adding a dedicated creator service, image, or ECS
  task. Authority, receiver, and bridge binaries all expose `POST /v1/admin/send-dummy`.
- Receiver-as-creator uses an ephemeral startup signing key. Bridge-as-creator reuses the
  bridge signing key and accepts the documented self-assignment collapse.
- The detailed spec's initial `tokio`/`url` sketch was adapted to the workspace's existing
  synchronous stdlib HTTP style to avoid introducing another runtime into the admin path.
