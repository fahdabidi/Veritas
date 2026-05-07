# GBN-PROTO-007 - Execution Phase 2 Detailed Plan: Admin Command Injection Endpoint

**Status:** Pending — depends on Phase 1 landing first
**Primary Goal:** add `POST /v1/admin/bridges/{bridge_id}/command` to the
publisher-authority admin listener so an operator can manually inject any
`BridgeCommandPayload` variant (`SeedAssign`, `PunchStart`, `BatchAssign`, `Revoke`,
`CatalogRefresh`) into the bridge's existing WebSocket control stream, matching V1's
`BroadcastSeed` and remediation-equivalent capabilities.
**Source Plan:** [GBN-PROTO-007 Execution Plan](GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md)
**Phase 1 Plan:** [GBN-PROTO-007-Execution-Phase1-Read-Only-Admin-Endpoints](GBN-PROTO-007-Execution-Phase1-Read-Only-Admin-Endpoints.md)

---

## 1. Current Repo Findings

| Item | Current Value | Why It Matters |
|---|---|---|
| Bridge command payload enum | [`BridgeCommandPayload` at control.rs:191-197](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/control.rs#L191-L197) | enumerates the 5 command variants; admin endpoint must accept exactly these |
| Existing control session manager | `gbn-bridge-publisher/src/control.rs` — owns the `bridge_id → BridgeControlChannel` in-memory map and the signed-frame send path | Phase 2 needs a small public method on this manager that wraps the existing send |
| Existing admin module from Phase 1 | `gbn-bridge-publisher/src/admin.rs` (created in Phase 1) | Phase 2's POST handler is a single function added here |
| Authority signing key | already plumbed for issuing `BridgeControlCommand` frames | reused for admin-injected commands; no new key material |
| Existing command sequence numbers | bridge control stream uses monotonic seq numbers ([control.rs:253-261 of protocol crate](../../../prototype/gbn-bridge-proto/crates/gbn-bridge-protocol/src/control.rs#L253-L261)) | admin commands must allocate from the same counter to keep the sequence intact |

---

## 2. Review Summary

| Gap | Why It Matters | Resolution For Phase 2 |
|---|---|---|
| No operator path to issue a control command on demand | V1's `BroadcastSeed` and operator-driven remediation flows have no V2 equivalent | add a single POST endpoint that wraps the existing internal command-send path |
| Direct admin access to control-channel sender map could leak unrelated state | tightly scoped helper avoids exposing the map | add `push_admin_command(bridge_id, payload)` method on the control manager; admin handler calls only that method |
| Sequence-number collision risk | if the admin path used its own counter, real and admin commands could collide | reuse the existing per-bridge counter — `push_admin_command` allocates from the same counter as a real send |

Phase 2 is intentionally small. It adds one POST route, one wrapper method, and one
integration test.

---

## 3. Scope Lock

### In Scope

- one new POST route `/v1/admin/bridges/{bridge_id}/command` on the **authority binary
  only** (receiver and bridge admin listeners return HTTP 404 for this path)
- one new public method `BridgeControlManager::push_admin_command(bridge_id, payload) ->
  Result<BridgeAdminCommandReceipt, AdminError>` in `gbn-bridge-publisher/src/control.rs`
- one new struct `BridgeAdminCommandReceipt { command_id: Uuid, seq_no: u64, dispatched_at_ms: u64 }`
  returned to the operator
- request body schema `{ "payload": <BridgeCommandPayload as JSON> }` — reuse the
  existing serde derive on the protocol enum
- integration test exercising each of the 5 payload variants

### Out Of Scope

- broadcast-to-all-bridges convenience (operator calls the endpoint per bridge_id; a
  shell loop in the script is enough)
- new command variants beyond the existing 5
- admin endpoints on the receiver or bridge binaries for command injection (commands
  always originate at authority)
- changing the wire-level `BridgeControlCommand` frame structure

---

## 4. Preflight Gates

1. Phase 1 has landed and is validated.
2. `cargo fmt --all --check` and `cargo test --workspace` pass on V2.
3. V1 protected-path diff is clean.
4. The current bridge control session manager exposes a way to send a command to a single
   bridge; if not, Phase 2 must first refactor that path before adding `push_admin_command`.

---

## 5. File-by-File Specification

### 5.1 Modify: `gbn-bridge-publisher/src/control.rs`

Locate the existing `BridgeControlManager` struct (or whatever the V2 name is — confirm
during implementation). Append a new public method:

```rust
impl BridgeControlManager {
    /// Inject an operator-supplied command into the named bridge's control stream.
    ///
    /// Allocates a sequence number from the same counter as real commands so the bridge's
    /// keepalive ack-replay logic stays consistent.
    ///
    /// Returns immediately after the command is queued onto the channel; does NOT wait for
    /// the bridge's `BridgeCommandAck`. Caller can correlate via the returned `command_id`.
    pub async fn push_admin_command(
        &self,
        bridge_id: &BridgeId,
        payload: BridgeCommandPayload,
    ) -> Result<BridgeAdminCommandReceipt, AdminCommandError> {
        // 1. Look up the channel sender for `bridge_id` in the in-memory map.
        // 2. If absent, return AdminCommandError::BridgeNotConnected.
        // 3. Allocate a fresh seq_no via the existing internal allocator.
        // 4. Build a `BridgeControlCommand` frame: sign with publisher key, set seq_no.
        // 5. Send via the channel. If channel is closed, return AdminCommandError::BridgeDisconnected.
        // 6. Return BridgeAdminCommandReceipt { command_id, seq_no, dispatched_at_ms: now() }.
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgeAdminCommandReceipt {
    pub command_id: Uuid,
    pub seq_no: u64,
    pub dispatched_at_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum AdminCommandError {
    #[error("bridge {0:?} is not currently connected")]
    BridgeNotConnected(BridgeId),
    #[error("bridge {0:?} disconnected mid-send")]
    BridgeDisconnected(BridgeId),
    #[error("internal error: {0}")]
    Internal(String),
}
```

The implementation must reuse:
- the existing channel sender map (read access only)
- the existing seq-number allocator (consume one)
- the existing publisher signing key (already held by the manager)

The implementation **must not**:
- mutate any state other than incrementing the seq counter
- bypass the existing signing path (admin commands are signed identically to real ones)

### 5.2 Modify: `gbn-bridge-publisher/src/admin.rs`

Extend `AdminState` (created in Phase 1) to carry an `Arc<BridgeControlManager>`:

```rust
pub struct AdminState {
    pub storage: Arc<Storage>,
    pub metrics: Arc<tokio::sync::Mutex<crate::metrics::AuthorityMetrics>>,
    pub control: Option<Arc<BridgeControlManager>>, // None on receiver / bridge binaries
}
```

Add a new POST handler:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct InjectCommandRequest {
    pub payload: BridgeCommandPayload,
}

async fn inject_bridge_command(
    state: &AdminState,
    bridge_id: BridgeId,
    request: InjectCommandRequest,
) -> Result<BridgeAdminCommandReceipt, AdminError> {
    let control = state.control.as_ref().ok_or(AdminError::NotSupportedOnThisBinary)?;
    control.push_admin_command(&bridge_id, request.payload).await
        .map_err(AdminError::from)
}
```

Mount the route inside `serve_admin_listener`:

```rust
router = router.route(
    "/v1/admin/bridges/:bridge_id/command",
    post(inject_bridge_command_handler),
);
```

Where `inject_bridge_command_handler` is the axum / hyper adapter that extracts the path
param and JSON body, then calls `inject_bridge_command`.

### 5.3 Modify: `gbn-bridge-publisher/src/api.rs`

Append one variant to `AuthorityRoute` (after the Phase 1 admin variants):

```rust
    AdminBridgeCommand,
```

In the `path()` impl:

```rust
            Self::AdminBridgeCommand => "/v1/admin/bridges/:bridge_id/command",
```

(Note: the `:bridge_id` placeholder reflects the routing pattern; the literal path emitted
to clients still includes the substituted UUID.)

### 5.4 Modify: each authority binary entry point

In `crates/gbn-bridge-cli/src/bin/publisher-authority.rs`, the `AdminState` construction
gains the control manager:

```rust
let admin_state = gbn_bridge_publisher::admin::AdminState {
    storage: storage.clone(),
    metrics: metrics.clone(),
    control: Some(control_manager.clone()),
};
```

In `publisher-receiver.rs` and `exit-bridge.rs`, set `control: None` so attempts to inject
a command return HTTP 501 / 404 there.

### 5.5 New file: `gbn-bridge-publisher/tests/admin_command_inject.rs`

```rust
//! Phase 2 admin command-injection coverage.

#[tokio::test]
async fn inject_seed_assign_to_connected_bridge_returns_receipt() { ... }

#[tokio::test]
async fn inject_to_unknown_bridge_returns_404() { ... }

#[tokio::test]
async fn inject_to_disconnected_bridge_returns_409() { ... }

#[tokio::test]
async fn inject_each_of_five_payload_variants_succeeds() { ... }

#[tokio::test]
async fn admin_command_seq_no_does_not_collide_with_real_command_seq_no() { ... }
```

The seq-no collision test is the most important: build a fixture where a real command and
an admin command race; assert each got a unique seq_no allocated from the shared counter.

---

## 6. Validation

1. `cargo fmt --all --check` and `cargo test --workspace` pass.
2. Phase 2 integration tests pass.
3. Deploy a stack and verify:
   - From inside the authority container: `curl -X POST -H 'Content-Type: application/json'
     http://127.0.0.1:9090/v1/admin/bridges/<bridge-id>/command -d '{"payload":{"CatalogRefresh":{}}}'`
     returns `{"command_id":"...", "seq_no":N, "dispatched_at_ms":...}`.
   - The chosen bridge container's logs show the `CatalogRefresh` arriving at seq N.
   - The bridge's `BridgeCommandAck` for that seq is observable in the authority logs
     (or alternately a future Phase 1 admin endpoint listing recent acks).
4. From a non-authority container, the same POST returns HTTP 501.
5. POSTing an unknown `bridge_id` returns HTTP 404 with `{error: "..."}`.
6. Update the Status Trackers table in
   [GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md](GBN-PROTO-007-Conduit-V2-V1-Parity-Execution-Plan.md)
   before starting the next phase.

---

## 7. Open Questions Carried Into Implementation

1. **Idempotency** — should two identical admin commands deduplicate via `command_id`?
   Recommendation: no — operators should be able to retry on demand; if dedup is wanted
   later, add `If-Match` header support.
2. **Auth scope** — Phase 2 inherits Phase 1's localhost-only model. Confirm no per-route
   auth needed.
3. **Receipt persistence** — should admin command receipts be persisted to a new
   `conduit_admin_commands` table for audit? Recommendation: defer; logs already capture
   them.
