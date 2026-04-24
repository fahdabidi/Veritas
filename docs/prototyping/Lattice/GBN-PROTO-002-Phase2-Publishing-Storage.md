# GBN-PROTO-002 — Prototyping Plan: Phase 2 — Publishing & Globally Distributed Storage

**Document ID:** GBN-PROTO-002  
**Phase:** 2 of 3  
**Status:** Draft  
**Last Updated:** 2026-04-07  
**Depends On:** Phase 1 (GBN-PROTO-001) — uses the crypto and chunking crates from Phase 1  
**Related Docs:** [GBN-REQ-002](../requirements/GBN-REQ-002-Media-Publishing.md), [GBN-REQ-003](../requirements/GBN-REQ-003-Global-Distributed-Storage.md), [GBN-ARCH-002](../architecture/GBN-ARCH-002-Media-Publishing.md), [GBN-ARCH-003](../architecture/GBN-ARCH-003-Global-Distributed-Storage.md)

---

## 1. Phase Goal

**Prove that a Publisher can take a reassembled video (from Phase 1), re-chunk it for storage, apply Reed-Solomon erasure coding, distribute shards across simulated storage nodes, sign and publish a content manifest, and that any client can later reconstruct the full video from only k-of-n shards — even when (n-k) nodes are dead, corrupted, or hostile.**

This phase validates the durability, integrity, and provenance layers of the GBN. It proves that content survives node failures, that corrupted shards are detected and rejected, and that Publisher signatures create an unforgeable chain of provenance.

---

## 2. Assumptions to Validate

| ID | Assumption | Risk if Wrong |
|---|---|---|
| A1 | Reed-Solomon (k=14, n=20) can encode and decode real video data without introducing artifacts or data corruption | If RS introduces errors, the entire storage model collapses |
| A2 | Any combination of exactly k=14 shards out of 20 can reconstruct the original data, not just the "first 14" | If RS only works with specific shard combinations, redundancy model is weaker than claimed |
| A3 | Ed25519 signatures on content manifests can be verified independently by any node with only the Publisher's public key | If verification requires additional state, the trust model breaks |
| A4 | A Kademlia DHT (even a minimal simulation) can reliably store and retrieve manifest CIDs | If DHT lookup fails under churn, content discovery breaks |
| A5 | BLAKE3-based content addressing (shard CID = BLAKE3(shard_data)) reliably detects corruption, substitution, and forgery of individual shards | If a corrupted shard passes hash verification, integrity guarantee is void |
| A6 | The Publisher can re-chunk from MCN chunk boundaries (1MB) to GDS chunk boundaries (4MB) without data loss at the boundary seams | If boundary alignment causes data loss, the two chunking systems are incompatible |
| A7 | Encrypted-at-rest shards (AES-256-GCM with Publisher content key) can survive RS encoding/decoding without corrupting the ciphertext | If RS operates on ciphertext incorrectly, the encrypt-then-erasure-code pipeline must be reordered |

---

## 3. Prototype Components

### 3.1 Project Structure (extends Phase 1 workspace)

```
gbn-proto/
├── crates/
│   ├── ... (Phase 1 crates)
│   ├── mpub-publisher/           # Editorial signing, re-chunking, manifest creation
│   ├── gds-erasure/              # Reed-Solomon encode/decode
│   ├── gds-storage-sim/          # Simulated storage node (filesystem-backed)
│   ├── gds-dht-sim/              # Minimal Kademlia DHT for manifest discovery
│   ├── gds-retriever/            # Client that fetches k shards and reconstructs
│   └── proto-cli/                # Extended: now supports publish + retrieve commands
├── tests/
│   ├── integration/
│   │   ├── test_erasure_coding.rs        # RS encode/decode with various k/n configs
│   │   ├── test_shard_corruption.rs      # Corrupt shards, verify detection + recovery
│   │   ├── test_node_failure.rs          # Kill (n-k) nodes, verify reconstruction
│   │   ├── test_manifest_signing.rs      # Sign, tamper, verify rejection
│   │   ├── test_full_publish_retrieve.rs # End-to-end: video → publish → retrieve → play
│   │   └── test_rechunking.rs            # MCN 1MB → GDS 4MB boundary alignment
│   └── benchmarks/
│       ├── bench_reed_solomon.rs
│       └── bench_shard_distribution.rs
```

### 3.2 Component Details

#### `mpub-publisher`
- **Purpose:** Take a reassembled video file, apply editorial metadata, re-chunk for GDS, encrypt, and sign
- **Implementation:**
  1. Read reassembled video from Phase 1 output
  2. Re-chunk into 4MB GDS chunks (streaming reader)
  3. Generate a random AES-256 content key
  4. Encrypt each GDS chunk with the content key
  5. Pass encrypted chunks to erasure coder
  6. Build `ContentManifest` struct with all shard CIDs, RS parameters, encrypted content key
  7. Sign manifest with Publisher's Ed25519 key
- **Key test:** Manifest signature verifies with Publisher's public key; tampering any field invalidates signature

#### `gds-erasure`
- **Purpose:** Validate Reed-Solomon encoding and decoding with real video data
- **Implementation:** Use the `reed-solomon-erasure` Rust crate
  1. Take a 4MB encrypted chunk as input
  2. Split into k=14 data shards
  3. Generate n-k=6 parity shards
  4. For each shard: compute BLAKE3 CID
  5. Decoding: accept any 14 of 20 shards → reconstruct original 4MB chunk
- **Key test:** Systematically test ALL C(20,14) = 38,760 combinations of 14 shards (or a statistically significant random sample of 1,000+) to verify every combination reconstructs correctly

#### `gds-storage-sim`
- **Purpose:** Simulate 20+ storage nodes as local filesystem directories
- **Implementation:**
  - Each "node" is a directory: `storage_node_01/`, `storage_node_02/`, etc.
  - Shards are stored as files named by their BLAKE3 CID
  - A simple TCP server per node responds to `GET <cid>` requests
  - Nodes can be "killed" by stopping their server process
  - Nodes can be "corrupted" by flipping random bits in shard files
- **Simulated features:**
  - Node heartbeat (periodic CID listing announcement)
  - Node death (process killed, simulating seizure/failure)
  - Selective corruption (simulating hostile node)

#### `gds-dht-sim`
- **Purpose:** Validate basic Kademlia DHT for manifest storage and discovery
- **Implementation:** Minimal Kademlia with:
  - 50 simulated DHT nodes (in-process, using `tokio::spawn`)
  - `PUT(key=BLAKE3(manifest), value=manifest_bytes)` for publishing
  - `GET(key)` for discovery
  - Node churn simulation (randomly kill/restart 20% of nodes during test)
- **Key test:** After publishing a manifest, a new node joining the DHT can discover and retrieve it within 5 seconds, even after 20% node churn

#### `gds-retriever`
- **Purpose:** Client-side logic that discovers content, fetches k shards, and reconstructs
- **Implementation:**
  1. DHT lookup by content CID → retrieve signed manifest
  2. Verify Publisher signature on manifest
  3. Extract shard CIDs from manifest
  4. Fetch shards from storage nodes (parallel, up to 5 concurrent)
  5. Verify BLAKE3 hash of each shard
  6. Reed-Solomon decode from k=14 shards → reconstruct encrypted chunk
  7. Decrypt chunk with content key (provided out-of-band for this prototype)
  8. Concatenate all decrypted chunks → reconstruct full video
- **Key test:** SHA-256 of reconstructed video === SHA-256 of original

---

## 4. Test Plan

### 4.1 Correctness Tests

| Test ID | Test Name | Pass Criteria |
|---|---|---|
| T2.1 | **RS Encode-Decode Identity** | For a 4MB test block: encode to 20 shards, decode from first 14 → byte-identical to original |
| T2.2 | **RS Any-K Reconstruction** | Randomly select 1,000 different 14-shard subsets from 20; ALL must reconstruct identically |
| T2.3 | **RS with Encrypted Data** | Encrypt 4MB block first, then RS encode. Decode from 14 shards, then decrypt → byte-identical to original plaintext |
| T2.4 | **Shard Corruption Detection** | Flip 1 bit in shard #7; retriever MUST reject it (BLAKE3 mismatch) and fetch replacement from another node |
| T2.5 | **Node Failure Tolerance** | Kill 6 of 20 storage nodes; retriever MUST still reconstruct from remaining 14 |
| T2.6 | **Node Failure + Corruption** | Kill 4 nodes AND corrupt 2 more; retriever rejects corrupted shards, uses remaining 14 clean shards |
| T2.7 | **Manifest Signature Verification** | Valid signature → accepted. Tamper title field → rejected. Tamper shard CID list → rejected. |
| T2.8 | **Manifest Forgery Attempt** | Sign manifest with a DIFFERENT Ed25519 key; verify rejection when checking against known Publisher key |
| T2.9 | **Re-Chunking Boundary Alignment** | Video of 10,000,017 bytes (not aligned to 4MB): verify no lost bytes at 1MB→4MB re-chunk boundary |
| T2.10 | **Full Publish-Retrieve Pipeline (100MB)** | Phase 1 output → publish → distribute 20 shards → kill 6 nodes → retrieve → verify SHA-256 match |
| T2.11 | **Full Publish-Retrieve Pipeline (500MB)** | Same as T2.10 with 500MB video |
| T2.12 | **DHT Discovery Under Churn** | Publish manifest, kill 20% of DHT nodes, new node joins and discovers manifest within 5 seconds |

### 4.2 Performance Benchmarks

| Benchmark | Target | Measurement |
|---|---|---|
| B2.1 | RS encode 500MB video (all chunks) | < 30 seconds |
| B2.2 | RS decode 500MB video (from 14 shards per chunk) | < 30 seconds |
| B2.3 | Shard distribution (20 nodes, localhost) | < 10 seconds for 500MB |
| B2.4 | Shard retrieval (14 nodes, parallel) | < 15 seconds for 500MB |
| B2.5 | DHT lookup latency | < 500ms per manifest lookup |
| B2.6 | Ed25519 sign + verify cycle | < 1ms |

### 4.3 Security Validation Tests

| Test ID | Test Name | What It Proves |
|---|---|---|
| S2.1 | **Blind Storage Verification** | Scan every shard file on every storage node; no shard contains recognizable video headers (MP4 atoms, WebM EBML) or readable strings from the video metadata |
| S2.2 | **Content Key Isolation** | The content key is NEVER stored on any storage node; only the Publisher and authorized VCPs possess it |
| S2.3 | **Cross-Content Key Independence** | Two different videos published by the same Publisher use different content keys; compromising one key does not expose the other video |
| S2.4 | **Sybil Resistance Simulation** | Spin up 100 additional DHT "Sybil" nodes that return garbage manifests. Verify the retriever rejects them via Publisher signature check and fetches the real manifest. |

---

## 5. Tech Stack Validation

| Technology | What We're Proving | Fallback if Fails |
|---|---|---|
| **reed-solomon-erasure** crate | RS coding works correctly on encrypted video data; performance acceptable | `zfec` (Python FFI) or custom Galois Field implementation |
| **ed25519-dalek** | Deterministic signing, correct verification, constant-time ops | `ring::signature` |
| **libp2p-kad** (reference) | Kademlia DHT concept works for manifest discovery | Custom minimal DHT |
| **tokio** | Async parallel shard upload/download scales to 20 concurrent connections | Confirmed in Phase 1 |

---

## 6. Success Criteria

Phase 2 is **PASSED** when ALL of the following are true:

- [ ] All 12 correctness tests (T2.1–T2.12) pass
- [ ] All 4 security validation tests (S2.1–S2.4) pass
- [ ] 500MB video survives full publish → kill 6 nodes → retrieve cycle with byte-perfect SHA-256 match
- [ ] RS decode performance < 30 seconds for 500MB
- [ ] DHT lookup succeeds within 5 seconds under 20% node churn
- [ ] Publisher signature forgery is rejected in every test case
- [ ] No storage node contains any readable/identifiable content from the video

## 7. Known Limitations of This Prototype

| Limitation | Why It's Acceptable |
|---|---|
| Storage nodes are local filesystem directories, not real networked servers | Phase 3 validates real network transport; this phase isolates RS and DHT logic |
| DHT is in-process simulation, not real UDP overlay | DHT networking is a BON concern (Phase 3); this phase proves the data structure works |
| No re-replication on node failure | Re-replication is an operational concern; this phase proves the k-of-n math works |
| No incentive mechanism for storage nodes | Incentive design is a Phase 4+ economic question |
| Content key shared out-of-band (no key distribution protocol) | Key distribution is a VCP concern (Phase 3/4) |
