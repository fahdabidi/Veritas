# GBN-PROTO-012 - Execution Phase 11 - Multi-Lane Progressive Fanout

**Status:** Pending
**Last Updated:** 2026-05-08
**Phase:** 11 (Multi-Lane Progressive Fanout)
**Parent Plan:** [GBN-PROTO-012](GBN-PROTO-012-Conduit-Architecture-Correct-Bootstrap-Execution-Plan.md)
**Depends On:** Phase 10 (`UploadSession` build pipeline) and Phases 0–5

## Objective

Implement `GBN-ARCH-001-V2` §3.6 (multi-lane upload route construction) and §3.7
(progressive fanout). Given an `UploadSession` produced by Phase 10, the creator
must:

1. Select N upload lanes from its local DHT, where N ≤ active bridge count and
   N target = 10 (`GBN-ARCH-001-V2` §3.7's "10 chunks across 10 lanes" spirit
   applied per-cluster: with our 10-bridge cluster, target 10 lanes when 10
   bridges are active).
2. Open `BridgeOpen` on each selected lane.
3. Disperse encrypted chunks (and the manifest) across active lanes.
4. **Progressive**: start sending chunks as each individual lane becomes active —
   do not wait for all lanes before sending the first chunk.
5. **Reuse**: if fewer than 10 lanes become active before
   `lane_open_timeout_ms`, send remaining chunks through the already-active
   lanes (per §3.7 reuse rule).
6. **Failover**: if a lane fails mid-session (no `BridgeAck` for a chunk within
   `chunk_ack_timeout_ms`), mark the bridge `suspect_until_ms = now +
   suspect_ttl_ms`, reroute that chunk to a still-active lane, and continue
   (per §7.1).
7. Track per-chunk per-lane state and emit the §2.5 upload-pipeline events.
8. Mark the `UploadSession` as `Completed`, `Partial`, or `Failed` based on chunk
   success counts.

Bridges still see only ciphertext (Phase 10 §3.5 envelope reused per chunk).

Update the parent plan status tracker when this phase is complete.

---

## Modules Added (`gbn-bridge-creator`)

New modules under
`prototype/gbn-bridge-proto/crates/gbn-bridge-creator/src/`:

- `pipeline/lane_planner.rs`
- `pipeline/dispatcher.rs`
- `pipeline/lane_state.rs`
- `pipeline/upload_runner.rs`

---

## Lane Planner (§3.6)

Selects upload lanes from a local-DHT snapshot. Inputs: `LocalDiscoveryTable`
snapshot from Phase 10's session, `target_lane_count` (default 10), filters
inherited from Phase 5 §Route Selection Rules.

```rust
pub struct LanePlan {
    pub target_lane_count: u32,
    pub selected_bridges: Vec<BridgeId>,    // up to target_lane_count
    pub overflow_pool: Vec<BridgeId>,       // candidates available for failover reuse
}

pub fn plan_lanes(
    local_dht: &LocalDiscoveryTable,
    target_lane_count: u32,
) -> Result<LanePlan, LanePlanError>;
```

Filter cascade (same as Phase 5 single-lane SendDummy):

1. drop entries with `active=false`;
2. drop expired (`lease_expiry_ms` or `entry_expiry_ms` in past);
3. drop entries with bad `publisher_sig`;
4. drop entries with `reachability_class = relay_only` (T1.9);
5. drop entries with `suspect_until_ms` still in the future;
6. rank surviving entries by recent `BridgePunchAck` time descending, then
   `lease_expiry_ms` descending;
7. take the top `target_lane_count`. Anything beyond goes into `overflow_pool` for
   later failover reuse.

If filtered set is empty, `LanePlanError::NoEligibleBridges`. If filtered set has
fewer than `target_lane_count` entries, `LanePlan` is returned with
`selected_bridges.len() < target_lane_count` — Phase 11 then triggers reuse (§3.7).

### Tests (`gbn-bridge-creator/tests/lane_planner.rs`)

- 10 active bridges, target 10 → 10 lanes selected, overflow_pool empty.
- 5 active bridges, target 10 → 5 lanes selected, overflow_pool empty (reuse comes
  from `selected_bridges` itself, see Dispatcher).
- 12 active bridges, target 10 → 10 selected, 2 in overflow_pool.
- relay_only filtered.
- suspect filtered.
- empty surviving set → `NoEligibleBridges`.

---

## Lane State Tracker

Per-lane state during a send. Concurrency: the `upload_runner` task owns the
`LaneStates` map; dispatcher and ack-handler send messages over an `mpsc` channel.

```rust
pub enum LaneStatus {
    Pending,       // BridgeOpen sent, waiting for ACK
    Active,        // BridgeOpen ACKed; ready to carry chunks
    SendingChunk { chunk_index: u32, sent_at_ms: u64 },
    Failed,        // marked suspect; no more chunks routed here
    Drained,       // session complete
}

pub struct LaneState {
    pub bridge_id: BridgeId,
    pub status: LaneStatus,
    pub chunks_sent: Vec<u32>,
    pub chunks_acked: Vec<u32>,
    pub last_ack_at_ms: Option<u64>,
}
```

### Tests (`gbn-bridge-creator/tests/lane_state.rs`)

- Transition Pending → Active on `BridgeOpenAck`.
- Transition SendingChunk → Active on `BridgeAck` for that chunk index.
- Transition any state → Failed on chunk timeout.
- Drained reached when all `selected_bridges` have no more pending chunks.

---

## Progressive Fanout Dispatcher (§3.7)

Given a `LanePlan` and an `UploadSession`, the dispatcher coordinates the
progressive send order.

### Send Order

1. Manifest is sent first, on the first lane to reach `Active`. The dispatcher
   waits for at least one lane Active before sending the manifest.
2. Data chunks 0..N-1 are dispatched in order, each routed to the next available
   lane (round-robin among `Active` lanes).
3. As additional lanes become `Active`, they immediately receive the next pending
   chunk.
4. **Reuse**: if `lane_open_timeout_ms` elapses with `selected_bridges.len()` ≤
   target but `Active` lanes < `selected_bridges.len()`, the dispatcher continues
   sending chunks through whatever is `Active` — no abort.
5. **Failover**: if a chunk's `BridgeAck` does not arrive within
   `chunk_ack_timeout_ms`, the lane transitions to `Failed`, the chunk is
   re-queued, and the dispatcher routes it to another `Active` lane. The bridge's
   `suspect_until_ms` is updated in the local DHT (mutation goes through the
   Phase 1 single-writer channel).
6. Session reaches `Completed` when every chunk has at least one `BridgeAck`. If
   `chunk_ack_timeout_ms` fires for a chunk and no other lane can carry it (all
   Failed), the session reaches `Partial` (≥ 1 chunk delivered) or `Failed`
   (zero chunks delivered).

### Configuration

- `lane_open_timeout_ms` default 30 000 (30 s). Configurable via env
  `GBN_BRIDGE_LANE_OPEN_TIMEOUT_MS`.
- `chunk_ack_timeout_ms` default 15 000 (15 s). Configurable via env
  `GBN_BRIDGE_CHUNK_ACK_TIMEOUT_MS`.
- `suspect_ttl_ms` default 300 000 (5 min) — same value used in Phase 4.

### Reuse Rule (§3.7)

If, at the moment chunk K is ready to send, the number of `Active` lanes is less
than `selected_bridges.len()`, chunk K still goes out — it does not block. This
is the architectural distinction from "wait for all lanes before sending":
chunks flow as soon as a single lane is open. The dispatcher records every
`creator_upload_lane_reused` event when a lane that already carried a chunk is
selected again.

### Failover Rule (§7.1)

When a lane fails mid-session, the chunk currently `SendingChunk` on that lane is
re-queued. Subsequent lane selection skips the failed lane. If the `overflow_pool`
contains fresh candidates and `Active` lane count drops below 1, the dispatcher
attempts a `BridgeOpen` against an `overflow_pool` bridge before declaring
session failure — this matches §7.1's "creator retries another cached valid
bridge".

---

## `UploadDispatchPlan` Structure

Stored alongside the `UploadSession` (Phase 10) and updated as the dispatcher
runs:

```rust
pub struct UploadDispatchPlan {
    pub plan_started_at_ms: u64,
    pub target_lane_count: u32,
    pub lanes: Vec<LaneState>,
    pub overflow_pool: Vec<BridgeId>,
    pub chunk_assignments: Vec<ChunkAssignment>,    // chunk_index → bridge_id, attempts
    pub manifest_lane: Option<BridgeId>,
    pub completed_chunks: u32,
    pub failed_chunks: Vec<u32>,
    pub session_status: UploadSessionStatus,
}

pub struct ChunkAssignment {
    pub chunk_index: u32,
    pub assigned_bridge_id: BridgeId,
    pub attempts: u32,
    pub first_dispatch_at_ms: u64,
    pub ack_at_ms: Option<u64>,
}
```

The `chunk_assignments` field is what Smoke 4 reads to assert "≥ 2 distinct lanes
used" and "progressive timeline" (chunk_index N's `first_dispatch_at_ms` < the
ms when all lanes became Active).

---

## Admin API

Add:

```http
POST /v1/admin/send-upload
GET  /v1/admin/upload-sessions/{session_id}/dispatch-plan
```

### `POST /v1/admin/send-upload`

Request:

```json
{
  "session_id": "base64...",
  "target_lane_count": 10,
  "lane_open_timeout_ms": 30000,
  "chunk_ack_timeout_ms": 15000,
  "force_lane_failure": null
}
```

`force_lane_failure` is an optional debug field for Smoke 4: it's a list of
`bridge_id`s that the dispatcher should mark `Failed` immediately after
`BridgeOpenAck` (before the first chunk goes through). Used to deterministically
exercise the failover path. Default null.

Response (returned after the dispatch terminates — `Completed`, `Partial`, or
`Failed`):

```json
{
  "session_id": "base64...",
  "chain_id": "send-upload-...",
  "session_status": "completed",
  "total_chunks": 128,
  "completed_chunks": 128,
  "failed_chunks": [],
  "lanes_used": ["exit-bridge-0", "exit-bridge-3", "exit-bridge-7"],
  "lane_count_at_first_dispatch": 1,
  "lane_count_at_completion": 9,
  "ciphertext_only_at_bridge": true,
  "elapsed_ms": 4123,
  "manifest_lane": "exit-bridge-0",
  "force_lane_failure_used": [],
  "first_chunk_dispatched_at_ms": 1780000000412,
  "all_lanes_active_at_ms": 1780000001834
}
```

`first_chunk_dispatched_at_ms < all_lanes_active_at_ms` proves progressive
fanout: the first chunk left the creator before the cluster reached the steady
state of all lanes Active.

### `GET /v1/admin/upload-sessions/{session_id}/dispatch-plan`

Returns the full `UploadDispatchPlan` (all chunk assignments, lane states,
timestamps). Smoke 4 polls this endpoint after `send-upload` completes to capture
the progressive timeline for assertions.

Both endpoints are mounted on `creator-runner` only.

---

## Operator Command: `SendUpload`

Add `SendUpload` to the menu actions in `_seed_actions.sh`. Flow:

1. Discover creator pods. Refuse if none in `onboarded` or `fanout_partial`.
2. Prompt: select creator (default `creator-new`).
3. Query `GET /v1/admin/upload-sessions` on the selected creator.
4. Prompt: select session_id (most recent first).
5. Prompt: target_lane_count (default 10).
6. Prompt: enable forced lane failure for failover demo? (default no.)
7. POST `/v1/admin/send-upload` with the chosen parameters.
8. Print `session_status`, `lanes_used`, `completed_chunks`, `failed_chunks`,
   timing fields, `chain_id`.
9. Offer to fetch the dispatch plan and trace bundle.

---

## Observability

Emit logs/spans (the 11 §2.5 upload-pipeline events Phase 11 owns; Phase 10 owns
event #1):

2. `creator_upload_lanes_selected` (one per session; includes `target_lane_count`,
   `selected_bridges.len()`, `overflow_pool.len()`)
3. `creator_upload_lane_open` (one per lane × N when `BridgeOpen` is sent)
4. `creator_upload_chunk_encrypted` (one per chunk; from Phase 10 envelope)
5. `creator_upload_chunk_dispatched` (one per chunk × bridge; for failover this
   fires more than once for the same chunk_index)
6. `creator_upload_lane_reused` (when chunk_assignment hits a bridge already in
   `chunks_sent`)
7. `creator_upload_lane_failover` (when a lane transitions to `Failed`
   mid-session)
8. `bridge_upload_chunk_forwarded` (per chunk × bridge)
9. `receiver_upload_chunk_ingested` (per chunk × bridge)
10. `receiver_upload_manifest_received` (once per session)
11. `publisher_upload_chunk_ack_returned` (per chunk)
12. `creator_upload_session_complete` (once per session, with terminal
    `session_status`)

Every event includes `chain_id`, `session_id`, and `chunk_index` where
applicable.

---

## Tests

Unit tests listed inline above. Integration tests in
`prototype/gbn-bridge-proto/crates/gbn-bridge-creator/tests/multi_lane_fanout.rs`:

- 10 bridges active, 128 chunks, target 10: every selected lane carries
  approximately 13 chunks; ≥ 2 distinct lanes used.
- 5 bridges active, 128 chunks, target 10: 5 lanes used; reuse events fire
  (`creator_upload_lane_reused` count > 0).
- 1 bridge active, 128 chunks, target 10: all chunks go through the single lane;
  `creator_upload_lane_reused` count = 127.
- `force_lane_failure` includes 1 of 10 selected lanes: dispatcher marks it
  Failed after BridgeOpenAck, chunks reroute, session completes; failover event
  count ≥ 1.
- Mid-session lane failure (simulated via test harness dropping `BridgeAck` from
  one bridge after chunk 50): pending chunk re-queues, completes via another
  lane, `creator_upload_lane_failover` event fires.
- All Active lanes fail mid-session, no `overflow_pool`: session_status =
  `Failed`, `completed_chunks` < `total_chunks`.
- Receiver content reconstruction: paired Publisher private key (test fixture)
  decrypts every chunk and the manifest; `SHA-256(reassembled_plaintext)` ==
  `manifest.content_hash`.
- `first_chunk_dispatched_at_ms < all_lanes_active_at_ms` (progressive fanout
  proof).
- AAD binding: tampered chunk_index in transit causes receiver decryption
  failure.

Run inside WSL2 Ubuntu (Master plan §2.8):

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 3 tooling requires WSL2 Ubuntu" >&2; exit 1; }
cd prototype/gbn-bridge-proto
cargo test -p gbn-bridge-creator --test lane_planner
cargo test -p gbn-bridge-creator --test lane_state
cargo test -p gbn-bridge-creator --test multi_lane_fanout
cargo test -p gbn-bridge-publisher --test admin_send_upload
```

---

## Acceptance Criteria

- `creator-new` (state `onboarded`) can run `SendUpload` against a session built
  in Phase 10 and reach `session_status=Completed` for a 1 MiB / 8 KiB session in
  the 10-bridge cluster.
- Receiver reconstructs the plaintext content and the content_hash matches the
  manifest.
- ≥ 2 distinct lane bridge_ids appear in `chunk_assignments`.
- `first_chunk_dispatched_at_ms < all_lanes_active_at_ms` (progressive fanout
  timeline preserved).
- Forced single-lane failover causes the chunk to reroute and the session still
  completes; `creator_upload_lane_failover` event fires at least once.
- 5-active-bridges scenario triggers `creator_upload_lane_reused` events
  (reuse rule §3.7 honored).
- Bridges see only ciphertext (the Phase 10 envelope is reused; bridge
  intercept tests confirm).
- All 11 of Phase 11's §2.5 events appear in Tempo for at least one successful
  session.
- V1 (`prototype/gbn-proto/**`) is unchanged.
- Parent plan status tracker is updated.
