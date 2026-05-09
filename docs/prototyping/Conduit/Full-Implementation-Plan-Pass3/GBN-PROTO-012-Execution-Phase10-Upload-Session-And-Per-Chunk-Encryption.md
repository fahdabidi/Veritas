# GBN-PROTO-012 - Execution Phase 10 - Upload Session Build And Per-Chunk Encryption Pipeline

**Status:** Completed
**Last Updated:** 2026-05-09
**Phase:** 10 (Upload Session Build And Per-Chunk Encryption Pipeline)
**Parent Plan:** [GBN-PROTO-012](GBN-PROTO-012-Conduit-Architecture-Correct-Bootstrap-Execution-Plan.md)
**Depends On:** Phases 0–5 complete (creator pods, local DHT, single-frame envelope
already proven)

## Objective

Implement `GBN-ARCH-001-V2` §3.4 (pre-processing pipeline) and the per-chunk part of
§3.5 (envelope encryption applied to every chunk). After this phase, an onboarded
creator can prepare a multi-chunk upload session that is fully sanitized, chunked,
manifest-hashed, and Publisher-encrypted on a per-chunk basis. Bridges still see
only ciphertext.

The upload session builder inherits the Phase 5 route/trust premise: bridge candidates
come from the creator-local DHT populated by the Publisher-seeded 10-entry ExitBridge
DHT set during bootstrap. It must not refresh route candidates through a direct
Publisher authority catalog shortcut.

This phase does **not** dispatch the chunks across multiple bridges — Phase 11 owns
multi-lane progressive fanout. Phase 10 produces a stored session ready for Phase 11
to send.

Update the parent plan status tracker when this phase is complete.

---

## Modules Added (`gbn-bridge-creator`)

New crate modules under
`prototype/gbn-bridge-proto/crates/gbn-bridge-creator/src/`:

- `pipeline/sanitizer.rs`
- `pipeline/chunker.rs`
- `pipeline/manifest.rs`
- `pipeline/session.rs`
- `pipeline/envelope.rs` (extended from Phase 5's single-frame envelope to support
  per-chunk derivation)

The modules form a single `pipeline::build_upload_session()` entry point that runs
sanitize → chunk → encrypt → manifest in order and persists the resulting
`UploadSession` to container-local disk under
`${GBN_BRIDGE_STATE_DIR}/upload_sessions/<session_id>/`.

---

## §3.4 Sanitizer

Strips identifiable metadata from input bytes before they leave the creator process.
Pass 3 sanitizer scope (per Master plan §6 carve-out — visual anonymization is
deferred):

- Strip EXIF and other container metadata blocks (JPEG APP1, PNG tEXt/iTXt/zTXt,
  MP4 udta, etc.).
- Remove encoder/device identifier strings ("HEIC", camera model serial numbers,
  software versions when explicitly tagged).
- Normalize timestamps in container metadata to 0 (epoch) so creation time cannot
  fingerprint the device clock.
- Pass through the actual media payload bytes unchanged (no transcoding).

API:

```rust
pub fn sanitize(input: &[u8], format_hint: SanitizerFormatHint) -> SanitizedBytes;
```

`SanitizedBytes` carries the sanitized byte vector plus a `SanitizationReport` with
counts of stripped fields per category, written into the manifest for traceability
(without revealing what was stripped — counts only).

For test purposes, the sanitizer accepts a `SanitizerFormatHint::Synthetic` mode
that treats the input as opaque bytes and only zeroes out a recognizable
test-marker prefix; this lets Smoke 4 use synthetic test files without needing real
JPEG/MP4 fixtures.

### Tests (`gbn-bridge-creator/tests/sanitizer.rs`)

- JPEG with EXIF: stripped fields = expected count; output bytes parse as a valid
  JPEG without an APP1 segment.
- PNG with tEXt block: tEXt removed; IDAT chunks intact.
- MP4 with udta: udta box removed; mdat box bytes unchanged.
- Synthetic mode: input bytes pass through except for the marker prefix.
- Idempotent: sanitize(sanitize(x)) == sanitize(x).

---

## §3.4 Chunker

Splits sanitized bytes into fixed-size chunks. Each chunk gets a SHA-256
plaintext hash. The whole content gets a SHA-256 content_hash that goes into the
manifest.

```rust
pub struct Chunk {
    pub index: u32,
    pub total: u32,
    pub plaintext_hash: [u8; 32],
    pub plaintext: Vec<u8>,
}

pub fn chunk(input: &[u8], chunk_size: usize) -> ChunkedContent {
    // chunk_size default: 64 KiB
    // last chunk may be shorter than chunk_size
}

pub struct ChunkedContent {
    pub chunks: Vec<Chunk>,
    pub content_hash: [u8; 32],   // SHA-256 of input as a whole
    pub total_bytes: u64,
    pub chunk_size: usize,
}
```

Pass 3 default chunk_size: 64 KiB. Smoke 4 uses 8 KiB so a 1 MiB test file produces
~ 128 chunks (large enough to make multi-lane fanout visible without overloading
WSL2). Operator-configurable via `BuildUploadSession` request.

### Tests (`gbn-bridge-creator/tests/chunker.rs`)

- 1 MiB input + 64 KiB chunk_size → 16 chunks, last one full size.
- 1 MiB + 1 byte input + 64 KiB chunk_size → 17 chunks, last one 1 byte.
- Empty input → 0 chunks (rejected with `EmptyInput` error — uploads must contain
  bytes).
- Determinism: chunk(x) == chunk(x).
- `content_hash` matches `SHA-256(input)`.
- Each `plaintext_hash` matches `SHA-256(chunk.plaintext)`.

---

## §3.4 Manifest Builder

```rust
pub struct UploadManifest {
    pub session_id: [u8; 16],
    pub creator_ephemeral_pubkey: [u8; 32],
    pub publisher_key_id: String,
    pub total_chunks: u32,
    pub content_hash: [u8; 32],
    pub sanitization_profile: String,    // e.g. "v3-default-no-visual-anon"
    pub created_at_ms: u64,
    pub chunk_size: u32,
    pub total_bytes: u64,
}
```

The manifest packet structure mirrors §3.5 exactly. The manifest itself is
encrypted as if it were chunk index `MANIFEST` (a special index value of 0xFFFF_FFFF)
so the receiver decrypts it before any data chunk. Phase 11's send order is:
manifest first, then chunks.

### Tests (`gbn-bridge-creator/tests/manifest.rs`)

- Manifest serialization round-trip is byte-stable.
- `creator_ephemeral_pubkey` matches the X25519 keypair used for the session.
- `publisher_key_id` matches the `node_id` of the local-DHT `publisher_entry`.

---

## §3.4 Session Builder

```rust
pub struct UploadSession {
    pub session_id: [u8; 16],
    pub manifest: UploadManifest,
    pub manifest_ciphertext: EncryptedFrame,
    pub chunk_ciphertexts: Vec<EncryptedFrame>,
    pub local_dht_snapshot: LocalDiscoveryTable,
    pub built_at_ms: u64,
    pub plan: UploadDispatchPlan,        // populated by Phase 11; empty after Phase 10
    pub status: UploadSessionStatus,     // Built, Dispatching, Completed, Partial, Failed
}

pub fn build_upload_session(
    plaintext: &[u8],
    chunk_size: usize,
    publisher_entry: &PublisherDhtEntry,
    local_dht: &LocalDiscoveryTable,
) -> Result<UploadSession, SessionBuildError>;
```

Steps:

1. Sanitize input.
2. Chunk sanitized bytes (compute per-chunk plaintext_hash and content_hash).
3. Generate a path-safe 16-byte session_id (rendered as 32 lowercase hex
   characters) from the creator, chain, sanitized content hash, timestamp, and a
   process-local monotonic counter so repeated byte-identical builds remain unique.
4. Generate creator ephemeral X25519 keypair.
5. Derive `upload_content_key` and `nonce_base` via X25519 + HKDF against
   `publisher_entry.pub_key`.
6. Encrypt the manifest as chunk index `MANIFEST`.
7. Encrypt every data chunk in order with chunk-index-derived nonce and AAD.
8. Snapshot the current `LocalDiscoveryTable` (Phase 11 selects lanes from this
   snapshot — any subsequent local-DHT change does not affect this session).
9. Persist the `UploadSession` to disk: one directory per session_id with
   `manifest.json`, `chunks/<index>.bin`, `local_dht.json`, `session.json`.

The session is durable: a container restart preserves it, and Phase 11 can resume
sending. A cluster destroy wipes it (per Pass 3 D1).

### Tests (`gbn-bridge-creator/tests/session_builder.rs`)

- Build session against a synthetic 256 KiB input: produces 4 chunks + manifest
  with correct counts and hashes.
- Round-trip: build session, write to disk, reload, decrypt manifest with the
  paired Publisher private key (test fixture), assert plaintext_hash matches per
  chunk.
- AAD binding: tampering with `chunk_index` in the AEAD AAD causes decryption
  failure.
- Replay: encrypting two different chunks with the same nonce-base + same chunk
  index must use different nonces (verified by AEAD check) — i.e., nonce
  derivation is injective on chunk_index.
- Reject when `local_dht.self_onboarding_state` is not `onboarded` or
  `fanout_partial`: returns `creator_not_onboarded`.

---

## §3.5 Per-Chunk Envelope Encryption

The envelope is the same construction as Phase 5 (X25519 + HKDF +
AES-256-GCM) but applied to every chunk and the manifest, with chunk-index-derived
nonces and AAD that binds the chunk to the session.

```text
shared_secret      = X25519::dh(creator_ephemeral_priv, publisher_entry.pub_key)
upload_content_key = HKDF-SHA256(shared_secret, "veritas/conduit/v2/upload-content-key", 32)
nonce_base         = HKDF-SHA256(shared_secret, "veritas/conduit/v2/nonce", 12)

For each chunk:
  nonce       = nonce_base XOR (chunk_index encoded as 12-byte big-endian)
  AAD         = session_id || chunk_index_le4 || total_chunks_le4 || plaintext_hash
  ciphertext  = AES-256-GCM-encrypt(upload_content_key, nonce, AAD, plaintext)
```

The manifest uses chunk_index = `MANIFEST` (0xFFFF_FFFF) which falls outside the
valid data-chunk range, so its nonce never collides with a data chunk nonce.

`EncryptedFrame` is the wire shape that Phase 11 wraps in `BridgeData`:

```text
[creator_ephemeral_pubkey: 32]
[publisher_key_id: variable string]
[session_id: 16]
[chunk_index: 4]
[total_chunks: 4]
[plaintext_hash: 32]
[ciphertext: variable]
[auth_tag: 16]
```

Phase 5's `EnvelopeKeyDerivation::PublisherX25519HkdfAes256GcmV1` is reused. No
new envelope variant is needed — the construction is identical, only the loop is new.

---

## Admin API

Add:

```http
POST /v1/admin/build-upload-session
GET  /v1/admin/upload-sessions
GET  /v1/admin/upload-sessions/{session_id}
DELETE /v1/admin/upload-sessions/{session_id}
```

### `POST /v1/admin/build-upload-session`

Accepts an optional `?chain_id=...` query parameter and optional JSON body
`chain_id`. If both are present they must match or the endpoint returns
`400 bad_query`. The response must echo the effective `chain_id`, and every
session-build log/span must use that same value together with the generated
`session_id`.

Request:

```json
{
  "chain_id": "build-upload-session-smoke-4-normal",
  "input_source": "synthetic",
  "synthetic_size_bytes": 1048576,
  "synthetic_marker": "VERITAS-SMOKE-4-PLAINTEXT",
  "chunk_size_bytes": 8192,
  "sanitization_profile": "v3-default-no-visual-anon"
}
```

`input_source` values:

- `"synthetic"`: generate `synthetic_size_bytes` of pseudorandom bytes prefixed by
  `synthetic_marker`. Used by Smoke 4. No real media touched.
- `"inline"`: an `inline_bytes_b64` field carries base64-encoded input. Useful for
  small fixtures; capped at 1 MiB per request.
- `"path"`: a `path` field names a file mounted into the creator container. Useful
  for AWS smoke runs where a test fixture file is shipped via EFS.

Response:

```json
{
  "session_id": "base64...",
  "chain_id": "build-upload-session-...",
  "manifest": {
    "total_chunks": 128,
    "content_hash": "base64...",
    "chunk_size": 8192,
    "total_bytes": 1048576,
    "sanitization_profile": "v3-default-no-visual-anon"
  },
  "sanitization_report": {
    "exif_segments_stripped": 0,
    "container_metadata_blocks_stripped": 0,
    "encoder_id_strings_stripped": 0,
    "synthetic_marker_zeroed": true
  },
  "ciphertext_only_at_bridge": true,
  "elapsed_ms": 412
}
```

### `GET /v1/admin/upload-sessions`

Returns a list of session ids and statuses on the selected creator. Useful for the
operator script to choose which session `SendUpload` should drive.

### `GET /v1/admin/upload-sessions/{session_id}`

Returns the full session metadata (manifest, dispatch plan, status), but **not** the
ciphertext bytes — those stay on disk and only Phase 11's `send-upload` path reads
them.

### `DELETE /v1/admin/upload-sessions/{session_id}`

Removes the on-disk session directory. Used by `ResetCreatorState` to clear all
sessions, and by operator-initiated cleanup.

All four endpoints are mounted on `creator-runner` only. Publisher and ExitBridge
pods return 404.

---

## Operator Command: `BuildUploadSession`

Add `BuildUploadSession` to the menu actions in `_seed_actions.sh`. Flow:

1. Discover creator pods. Refuse if none in `onboarded` or `fanout_partial`.
2. Prompt: select creator (default `creator-new`).
3. Prompt: input source (`synthetic` is default for smoke; `inline`/`path` for
   debug).
4. Prompt: chunk size (default 8 KiB).
5. Prompt: synthetic size (default 1 MiB) and marker (default
   `VERITAS-SMOKE-4-PLAINTEXT`).
6. POST `/v1/admin/build-upload-session?chain_id=<chain_id>` to the selected
   creator.
7. Print `session_id`, chunk count, content_hash, sanitization_report, echoed
   `chain_id`.
8. Offer to continue with `SendUpload` (Phase 11 action) or stop here.

Per Master plan §2.8, all operator-script invocations require WSL2 Ubuntu.

---

## Observability

Emit logs/spans (per Master plan §2.5 upload-pipeline events):

- `creator_upload_session_built` (one per session; includes `session_id`,
  `total_chunks`, `content_hash`, `sanitization_report`)

The event uses the operator-supplied `chain_id` from
`POST /v1/admin/build-upload-session?chain_id=...`; smoke tests must fail if the
response or trace event uses a different chain.

Phase 10 emits 1 of the 12 §2.5 upload-pipeline events; Phase 11 emits the other 11.

---

## Tests

Unit tests listed inline above. Integration coverage lives in
`gbn-bridge-creator/tests/session_builder.rs` and
`gbn-bridge-publisher/tests/admin_build_upload_session.rs`:

- Build a synthetic 1 MiB / 8 KiB session → 128 chunks; manifest content_hash
  matches `SHA-256(sanitized_bytes)`.
- Persistence: build → kill the creator process → re-launch → `GET
  /v1/admin/upload-sessions` returns the session.
- Idempotency: rebuilding with byte-identical input produces a different
  `session_id` (sessions are intentionally unique per call) but the same chunk
  count and content_hash.
- Cluster destroy: `k3d cluster delete && create` → no sessions on disk (per Pass 3
  D1).
- Reject: input_source=`synthetic` with `synthetic_size_bytes` > 1 MiB returns
  `synthetic_size_too_large` (Pass 3 cluster envelope; per Master plan §6).
- Reject: building on a non-onboarded creator returns `creator_not_onboarded`.

Run inside WSL2 Ubuntu (Master plan §2.8):

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 3 tooling requires WSL2 Ubuntu" >&2; exit 1; }
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test -p gbn-bridge-creator --test sanitizer
cargo test -p gbn-bridge-creator --test chunker
cargo test -p gbn-bridge-creator --test manifest
cargo test -p gbn-bridge-creator --test session_builder
cargo test -p gbn-bridge-publisher --test admin_build_upload_session
```

---

## Completion Notes

Implemented in `gbn-bridge-creator` and exposed through creator-only admin
endpoints in `gbn-bridge-publisher`.

Validation completed on 2026-05-09:

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo test -p gbn-bridge-creator --test sanitizer`
- `cargo test -p gbn-bridge-creator --test chunker`
- `cargo test -p gbn-bridge-creator --test manifest`
- `cargo test -p gbn-bridge-creator --test session_builder`
- `cargo test -p gbn-bridge-publisher --test admin_build_upload_session`
- Live k3d validation after image rollout
  `local-20260509T033552Z-d63696d5a88d-dirty`:
  `BuildUploadSession` on onboarded `creator-new` produced 128 encrypted chunk
  files for a 1 MiB / 8 KiB synthetic input, stored manifest/session/local-DHT
  files under `/var/lib/gbn-conduit/upload_sessions/<session_id>/`, preserved the
  session across a `creator-new` rollout restart, returned 404 on the
  `publisher-authority` admin listener, and emitted `creator_upload_session_built`
  to Tempo with matching `chain_id`, `session_id`, `total_chunks`, and
  `content_hash`.

Live artifact directory:
`target/k8s-smoke-artifacts/phase10-upload-session/20260508-205942-3013294`.

The destructive cluster-delete wipe check was not run during this phase completion
so the current Pass 3 cluster remains available for Phase 11; the session data is
stored only in the local k3d PVC and the PVC has `Delete` reclaim semantics.

## Acceptance Criteria

- `creator-runner` exposes the four `/v1/admin/upload-sessions*` endpoints listed
  above; Publisher and ExitBridge pods return 404.
- `BuildUploadSession` against `creator-new` (state `onboarded`) produces a
  durable on-disk session directory with manifest + per-chunk ciphertexts.
- The sanitizer strips EXIF / container metadata / encoder ids when applied to
  real-format fixtures; synthetic mode zeroes the test marker prefix.
- The chunker emits the correct number of chunks for the configured chunk size
  and computes a content_hash equal to `SHA-256(sanitized_input)`.
- Per-chunk encryption round-trips: a paired Publisher private key (test fixture)
  decrypts every chunk and the manifest, and AAD tampering causes decryption
  failure.
- Container restart preserves the session directory (Pass 3 D1 persistence).
- Cluster destroy wipes it (Pass 3 D1).
- The §2.5 event `creator_upload_session_built` appears in Tempo with the echoed
  `chain_id`, `session_id`, `total_chunks`, and `content_hash` attributes.
- V1 (`prototype/gbn-proto/**`) is unchanged.
- Parent plan status tracker is updated.
