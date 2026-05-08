# GBN-PROTO-012 - Execution Phase 12 - Smoke 4 Full Upload Pipeline Suite Implementation

**Document ID:** GBN-PROTO-012-Smoke-4
**Status:** Pending
**Last Updated:** 2026-05-08
**Phase:** 12 (Smoke 4 — Full Upload Pipeline Suite Implementation)
**Parent Plan:** [GBN-PROTO-012](GBN-PROTO-012-Conduit-Architecture-Correct-Bootstrap-Execution-Plan.md)
**Depends On:** Phases 0–11 complete; Phases 7 (Smoke 1), 8 (Smoke 2), 9 (Smoke 3)
all green

This is **Smoke 4** in the Pass 3 local Kubernetes Conduit suite. It validates the
full §3.4–§3.7 upload pipeline end-to-end: sanitization → chunking → manifest →
per-chunk envelope encryption → multi-lane progressive fanout → receiver
reconstruction. Smoke 4 is the architecturally complete media upload demo on the
local cluster.

Smoke 4 has no Pass 2 predecessor (the upload pipeline did not exist before
Pass 3). There is no Pass 2 file to supersede.

---

## 1. Goal

Prove that:

1. `creator-new` (in `onboarded` state from Smoke 2) can build an upload session
   from a synthetic 1 MiB test file, chunked at 8 KiB (≈ 128 chunks).
2. The full pipeline output is durable on `creator-new`'s container-local PVC
   (Pass 3 D1 persistence): manifest + per-chunk ciphertext blobs.
3. `SendUpload` against the session reaches `session_status=Completed` within
   the timeout against the 10-bridge cluster.
4. Receiver reconstructs the plaintext content from the chunks; the
   `SHA-256(reassembled_plaintext)` equals `manifest.content_hash`.
5. Every bridge that carried a chunk saw only ciphertext (plaintext marker grep
   on bridge logs returns empty for every involved bridge).
6. ≥ 2 distinct lanes appear in `chunk_assignments` (multi-lane proof, §3.6).
7. `first_chunk_dispatched_at_ms < all_lanes_active_at_ms` (progressive fanout,
   §3.7).
8. With `--include-failover`, an additional run with `force_lane_failure` on one
   selected lane still completes; `creator_upload_lane_failover` event fires.
9. All 12 §2.5 upload-pipeline events appear in Tempo for at least one
   successful session, indexed by `session_id`.

---

## 2. Pre-Conditions

- WSL2 Ubuntu host (Master plan §2.8 guard at top of script).
- Smoke 1, Smoke 2, Smoke 3 all just passed.
- `creator-new` reports `self_onboarding_state ∈ { onboarded, fanout_partial }`.
- `creator-new` local DHT bridge entries came from the Publisher-seeded bootstrap set;
  full upload must not refresh lanes from a direct authority catalog shortcut.
- ≥ 5 active bridge entries in `creator-new`'s local DHT (failover proof needs
  redundancy).

---

## 3. Implemented Command

```bash
bash prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-upload-v3.sh \
  --require-observability \
  --synthetic-size 1048576 \
  --chunk-size 8192 \
  --target-lane-count 10 \
  --include-failover \
  --upload-timeout 120
```

Flags:

- `--require-observability`: as Smoke 1.
- `--synthetic-size N`: synthetic test bytes (default 1 MiB).
- `--chunk-size N`: chunk size (default 8 KiB → ~128 chunks for default size).
- `--target-lane-count N`: passed to `send-upload` (default 10).
- `--include-failover`: also run a second invocation with
  `force_lane_failure=[<one selected lane>]`. Default on.
- `--upload-timeout N`: max seconds to wait for `session_status=Completed`.

---

## 4. Probe Sequence

1. Generate `chain_id_normal = smoke-4-normal-<UUID>` and
   `chain_id_failover = smoke-4-failover-<UUID>`.
2. Capture `local-dht` snapshot before any send: `creator-local-dht-before.json`.
3. **Build session**: POST `/v1/admin/build-upload-session?chain_id=<chain_id_normal>`
   on `creator-new` with `input_source=synthetic`,
   `synthetic_size_bytes=<--synthetic-size>`,
   `synthetic_marker=VERITAS-SMOKE-4-PLAINTEXT`,
   `chunk_size_bytes=<--chunk-size>`.
4. Capture response → `build-session-result.json`. Extract `session_id_normal`.
5. **Normal SendUpload**: POST
   `/v1/admin/send-upload?chain_id=<chain_id_normal>` with `session_id=<session_id_normal>`,
   `target_lane_count=<--target-lane-count>`, `force_lane_failure=null`.
6. Wait for response (long-running; capped at `--upload-timeout`). Capture →
   `send-upload-normal-result.json`.
7. Fetch dispatch plan: GET
   `/v1/admin/upload-sessions/<session_id_normal>/dispatch-plan` →
   `dispatch-plan-normal.json`.
8. **Failover invocation** (if `--include-failover`):
   - Build a fresh session: `session_id_failover` (each session is single-shot;
     replaying `send-upload` against an already-completed session is rejected
     with `session_already_dispatched`).
   - From the local-DHT snapshot, pick the bridge with the most recent ACK as
     the deliberate failure target.
   - POST `/v1/admin/send-upload?chain_id=<chain_id_failover>` with
     `force_lane_failure=[<chosen_bridge_id>]`.
   - Capture → `send-upload-failover-result.json`.
   - Fetch dispatch plan → `dispatch-plan-failover.json`.
9. Wait 5 s for trace export and receiver persistence.
10. Run assertions.

---

## 5. Assertions

### 5.1 Build Session Response

- `manifest.total_chunks` matches `ceil(synthetic_size / chunk_size)`.
- `manifest.content_hash` is non-empty 32-byte base64.
- `sanitization_report.synthetic_marker_zeroed == true`.
- `ciphertext_only_at_bridge == true`.

### 5.2 Normal SendUpload Response

- `session_status == "completed"`.
- `completed_chunks == total_chunks`.
- `failed_chunks == []`.
- `lanes_used.length >= 2` (multi-lane proof, §3.6).
- `force_lane_failure_used == []`.
- `ciphertext_only_at_bridge == true`.
- `first_chunk_dispatched_at_ms < all_lanes_active_at_ms` (progressive fanout,
  §3.7).
- `elapsed_ms < (--upload-timeout * 1000)`.

### 5.3 Failover SendUpload Response (if `--include-failover`)

- `session_status == "completed"`.
- `completed_chunks == total_chunks`.
- `failed_chunks == []`.
- `force_lane_failure_used == [<chosen_bridge_id>]`.
- `lanes_used` does not include `<chosen_bridge_id>` (or includes it only with
  `attempts > 1` showing reroute).
- At least one chunk in `dispatch-plan-failover.json.chunk_assignments` has
  `attempts >= 2` (a failover reroute happened).

### 5.4 Receiver Content Reconstruction

For each `session_id`:

- The Publisher (receiver surface) admin API
  `GET /v1/admin/upload-sessions/<session_id>` (added in Phase 11 on receiver)
  returns:
  - `chunks_received == total_chunks`;
  - `manifest_received == true`;
  - `content_hash_match == true` (receiver computes `SHA-256(reassembled
    plaintext)` and compares to manifest.content_hash);
  - `synthetic_marker_first_byte_at == 0` (receiver finds the test marker at the
    start of the reconstructed plaintext, proving end-to-end integrity).

### 5.5 Bridge Ciphertext-Only

For each `lanes_used` bridge in each session:

- Loki query `{namespace="veritas", actor_id="<bridge_id>"} |= "<chain_id>"`
  returns log lines containing `payload_bytes` and `chunk_index`, but `grep`
  for `VERITAS-SMOKE-4-PLAINTEXT` against the matched lines returns empty.
- The bridge `bridge_upload_chunk_forwarded` events all have
  `ciphertext_only=true`.

### 5.6 Trace Coverage (12 §2.5 Upload-Pipeline Events)

For each `chain_id`, Tempo returns spans covering all 12 upload-pipeline events
from Master plan §2.5:

1. `creator_upload_session_built` (Phase 10 — emitted on the
   build-upload-session call)
2. `creator_upload_lanes_selected`
3. `creator_upload_lane_open` (one per lane)
4. `creator_upload_chunk_encrypted` (one per chunk; from Phase 10)
5. `creator_upload_chunk_dispatched` (one per chunk × bridge; ≥ total_chunks
   total)
6. `creator_upload_lane_reused` (count depends on active-lane count — must be ≥
   0 with explicit value asserted)
7. `creator_upload_lane_failover` (only in failover invocation; must be ≥ 1 in
   that run)
8. `bridge_upload_chunk_forwarded` (one per chunk × bridge)
9. `receiver_upload_chunk_ingested` (one per chunk × bridge)
10. `receiver_upload_manifest_received` (once per session)
11. `publisher_upload_chunk_ack_returned` (one per chunk)
12. `creator_upload_session_complete` (once per session, with `session_status =
    completed`)

### 5.7 Persistence Behavior (Pass 3 D1)

- After the normal session completes, restart the `creator-new` pod:
  `kubectl delete pod creator-new-<id>` and wait for the new pod to reach
  Ready.
- `GET /v1/admin/upload-sessions` on the new pod returns the same `session_id`
  with `session_status=Completed`.
- `kubectl exec creator-new -- ls /var/lib/gbn-conduit/upload_sessions/` lists
  the directory.

### 5.8 Cluster Destroy Behavior (Pass 3 D1)

(Optional, gated by `--include-cluster-destroy`; off by default since it's
expensive in the inner loop.)

- `k3d cluster delete && k3d cluster create veritas` then `k8s-up.sh`.
- `GET /v1/admin/upload-sessions` returns empty array.

---

## 6. Artifacts

Written to `/tmp/conduit-smoke-4-${chain_id_normal}/`:

- `pods.json`
- `creator-local-dht-before.json`
- `build-session-result.json`
- `send-upload-normal-result.json`
- `dispatch-plan-normal.json`
- `send-upload-failover-result.json`
- `dispatch-plan-failover.json`
- `receiver-session-summary-normal.json`
- `receiver-session-summary-failover.json`
- `bridge-logs-by-chain-id/` (logs from the lanes_used bridges)
- `bridge-plaintext-grep.txt` (must be empty)
- `traces-by-chain-id/` (Tempo dumps; 12 events per chain)
- `progressive-timeline.csv` (chunk_index, first_dispatch_at_ms, ack_at_ms,
  bridge_id — for visual inspection of progressive ordering)
- `upload-summary.md` (table: invocation, session_id, lanes_used,
  completed_chunks, failover_used, content_hash_match)

---

## 7. Failure Modes And Triage

| Failure | Likely Cause |
|---|---|
| `session_status=Partial` after upload-timeout | Some chunks never ACKed; check bridge `frames_forwarded` per bridge_id |
| `content_hash_match=false` | Receiver decryption broken; check ciphertext length and AAD binding |
| Plaintext marker found in bridge logs | Phase 10 envelope not applied per chunk; check `pipeline/envelope.rs` |
| `lanes_used.length < 2` | Lane planner not selecting multiple bridges; check Phase 11 `plan_lanes` filter |
| `first_chunk_dispatched_at_ms >= all_lanes_active_at_ms` | Dispatcher waiting for all lanes Active before sending; §3.7 progressive rule violated |
| Failover `attempts == 1` for all chunks | `force_lane_failure` not respected by dispatcher |
| 12 events not in Tempo | Phase 11 observability not wired |
| `receiver_upload_chunk_ingested` count < total_chunks | Receiver dropping chunks; check Postgres write path |
| Persistence loss after pod restart | Phase 10 session directory not on PVC; check k8s manifest volume mount |

---

## 8. Out Of Scope

- Real media files at production scale (Master plan §6 — Pass 3 caps at 1 MiB
  synthetic).
- Optional visual anonymization (Master plan §6 carve-out).
- AWS validation (handled in Phase 6 AWS acceptance flow with a parallel
  `aws-smoke-upload-v3.sh` runner).
- Performance / latency / throughput measurements (§10.2 promotion gate).
- Concurrent multi-creator parallel uploads.

---

## 9. Implementation Tasks

1. Add `infra/scripts/k8s-smoke-upload-v3.sh` (new file).
2. Test fixture: synthetic byte generator with the marker prefix
   `VERITAS-SMOKE-4-PLAINTEXT` (Phase 10 supplies the build-session input
   source).
3. Extend `k8s-smoke-common.sh` with `upload_session_query(creator_id,
   session_id)` helpers for both creator and receiver views.
4. WSL2 guard at top of script per Master plan §2.8.
5. Print artifact directory at end.

---

## 10. Acceptance

- `bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-smoke-upload-v3.sh`
  passes.
- Against a Smoke-3-green cluster, the script exits 0 with completed normal +
  completed failover sessions, content_hash match, and bridge plaintext grep
  empty.
- All 12 §2.5 upload-pipeline events present in Tempo.
- `lanes_used.length >= 2` and progressive timeline asserted.
- Persistence check: post-restart `GET /v1/admin/upload-sessions` returns the
  prior session.
- `git diff --stat -- docs/prototyping/Conduit/Full-Implementation-Plan-Pass2/`
  is empty.
- V1 (`prototype/gbn-proto/**`) is unchanged.
- Parent plan status tracker is updated.
