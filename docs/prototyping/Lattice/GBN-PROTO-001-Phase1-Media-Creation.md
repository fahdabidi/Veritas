# GBN-PROTO-001 — Prototyping Plan: Phase 1 — Media Creation Network & Video Reconstruction

**Document ID:** GBN-PROTO-001  
**Phase:** 1 of 3  
**Status:** Draft  
**Last Updated:** 2026-04-07  
**Related Docs:** [GBN-REQ-001](../requirements/GBN-REQ-001-Media-Creation-Network.md), [GBN-ARCH-001](../architecture/GBN-ARCH-001-Media-Creation-Network.md), [GBN-REQ-002](../requirements/GBN-REQ-002-Media-Publishing.md)

---

## 1. Phase Goal

**Prove that a video file can be anonymized, chunked, encrypted, and routed via Telescopic Onion Routing resolving endpoints via a Kademlia DHT. The chunks must securely traverse multiple independent simulated paths across separate EC2 instances, be validated against blackholes, received out-of-order by a Publisher using a cryptographic Root of Trust, and perfectly reconstructed with zero data loss or metadata leakage. Validating route recovery if nodes are taken down mid-transmission must also be proven.**

This phase is no longer a simple TCP-forwarding testbed. It validates the full zero-trust, end-to-end data pipeline from Creator device to Publisher reassembly on AWS infrastructure. Relay hops use nested `Noise_XX` handshakes across EC2 Spot instances in different availability zones, demonstrating cryptographic validation of route integrity against malicious relays and identity spoofers.

---

## 2. Assumptions to Validate

| ID | Assumption | Risk if Wrong |
|---|---|---|
| A1 | FFmpeg can strip ALL identifying metadata from common video containers (MP4, MKV, WebM) without re-encoding the video stream | If metadata survives, Creator identity leaks |
| A2 | Chunk-then-encrypt with independent AES-256-GCM per chunk allows out-of-order decryption and reassembly at the Publisher | If not, we need encrypt-then-chunk which has severe error-resilience problems |
| A3 | X25519 ECDH + HKDF-SHA256 key derivation produces identical session keys on both sides (Creator ephemeral + Publisher static) | If key derivation diverges, Publisher cannot decrypt |
| A4 | BLAKE3 hashing at 1MB chunk granularity is fast enough to not bottleneck the pipeline on mobile-class hardware | If too slow, chunk size or hash algorithm needs adjustment |
| A5 | Multipath routing (sending chunks across N independent simulated paths with random delays) does not cause reassembly failures | If ordering/session-tracking breaks, the manifest protocol needs redesign |
| A6 | A tampered chunk (even 1 flipped bit) is reliably rejected by AES-256-GCM authentication before reassembly | If tampered chunks slip through, integrity model is broken |
| A7 | The entire pipeline (sanitize → chunk → encrypt → route → decrypt → reassemble) can process a 500MB video within the performance targets (~60s preprocessing, <30min total) | If too slow, architecture needs optimization |
| A8 | Nested `Noise_XX` handshakes across 3 relays (Telescopic Circuit) does not exceed computational overhead limits for streaming mobile uploads | If latency is too high, the MCN will stall video ingest |
| A9 | Kademlia DHT passive syncing enables route discovery fast enough to support live, dynamic circuit fallback during node loss | If discovery drags, dropped chunks cannot be quickly requeued via Heartbeats |

---

## 3. Prototype Components

### 3.1 Project Structure

```
gbn-proto/
├── Cargo.toml                    # Workspace root
├── crates/
│   ├── gbn-protocol/             # Shared protocol types and constants
│   ├── mcn-sanitizer/            # Metadata stripping (FFmpeg wrapper)
│   ├── mcn-chunker/              # Video → 1MB chunks + BLAKE3 manifest
│   ├── mcn-crypto/               # X25519 ECDH, AES-256-GCM encrypt/decrypt
│   ├── mcn-router-sim/           # Multipath relay (TCP across EC2 instances)
│   ├── mpub-receiver/            # Chunk reception, buffering, reassembly
│   └── proto-cli/                # CLI tool that orchestrates the full pipeline
├── infra/
│   ├── cloudformation/
│   │   ├── phase1-stack.yaml     # CloudFormation template: VPC, subnets, instances
│   │   ├── parameters.json       # Configurable: region, instance types, key pair
│   │   └── teardown.sh           # Destroy stack to stop billing
│   ├── scripts/
│   │   ├── bootstrap.sh          # User data script: install Rust, FFmpeg, deps
│   │   ├── deploy-creator.sh     # SCP binary + test video to Creator instance
│   │   ├── deploy-relays.sh      # SCP relay binary to relay instances
│   │   ├── deploy-publisher.sh   # SCP receiver binary to Publisher instance
│   │   └── run-tests.sh          # SSH into Creator, execute full test suite
│   └── README-infra.md           # Instructions for launching and tearing down
├── test-vectors/
│   └── README.md                 # Instructions: user provides their own sample video
└── tests/
    ├── integration/
    │   ├── test_full_pipeline.rs        # End-to-end: sanitize → reassemble
    │   ├── test_multipath_reassembly.rs # Chunks via N paths, random delays
    │   ├── test_tamper_detection.rs     # Flip bits, verify rejection
    │   └── test_metadata_stripping.rs   # Verify zero metadata survives
    └── benchmarks/
        ├── bench_chunking.rs
        ├── bench_encryption.rs
        └── bench_blake3.rs
```

### 3.2 AWS Infrastructure Layout

```
┌─────────────────────────────────────────────────────────────────────┐
│  AWS Region: us-east-1 (configurable)                               │
│  VPC: 10.0.0.0/16                                                   │
│                                                                     │
│  Subnet A (us-east-1a)          Subnet B (us-east-1b)               │
│  ┌──────────────────┐           ┌──────────────────┐                │
│  │  Creator Instance │           │  Relay Instance 2 │               │
│  │  t3.small (Spot)  │──────────▶│  t3.micro (Spot)  │               │
│  │  10.0.1.10        │           │  10.0.2.20        │               │
│  └──────────────────┘           └────────┬─────────┘                │
│           │                              │                          │
│           ▼                              ▼                          │
│  ┌──────────────────┐           ┌──────────────────┐                │
│  │  Relay Instance 1 │           │  Relay Instance 3 │               │
│  │  t3.micro (Spot)  │           │  t3.micro (Spot)  │               │
│  │  10.0.1.20        │           │  10.0.2.30        │               │
│  └──────────────────┘           └────────┬─────────┘                │
│                                          │                          │
│  Subnet C (us-east-1c)                   ▼                          │
│  ┌──────────────────┐           ┌──────────────────┐                │
│  │  Relay Instance 4 │──────────▶│  Publisher Instance│               │
│  │  t3.micro (Spot)  │           │  t3.small (Spot)  │               │
│  │  10.0.3.10        │           │  10.0.3.20        │               │
│  └──────────────────┘           └──────────────────┘                │
│                                                                     │
│  Security Group: inter-relay traffic on port 9000-9100 only         │
│  Security Group: SSH (port 22) from deployer IP only                │
│  All instances: Amazon Linux 2023 AMI, Spot pricing                 │
└─────────────────────────────────────────────────────────────────────┘
```

**Estimated Cost:** ~$0.50–$1.00/hour for the full stack (7 Spot instances). Stack is designed to be launched for testing and torn down immediately after.

### 3.3 Test Video

The user provides their own sample video file(s). The deployment script uploads the video to the Creator instance via SCP. Minimum test set:
- One small video (10–50MB) for rapid iteration
- One large video (500MB+) for performance validation

### 3.2 Component Details

#### `mcn-sanitizer`
- **Purpose:** Validate that FFmpeg-based stripping removes ALL metadata
- **Implementation:** Rust wrapper around FFmpeg CLI (`-map_metadata -1 -fflags +bitexact`)
- **Key test:** Run `exiftool` and `mediainfo` on output; assert zero identifying fields survive
- **Edge cases:** iPhone HEVC containers, GoPro telemetry tracks, Android HDR metadata

#### `mcn-chunker`
- **Purpose:** Split sanitized video into fixed 1MB chunks, generate BLAKE3 manifest
- **Implementation:** Streaming file reader; chunk → hash → manifest entry
- **Key output:** `ChunkManifest { session_id, total_chunks, chunks: [(index, blake3_hash)] }`
- **Edge case:** Last chunk is smaller than 1MB — must be padded to standard size

#### `mcn-crypto`
- **Purpose:** Validate the full key exchange and per-chunk encryption pipeline
- **Implementation:**
  1. Generate Publisher Ed25519 keypair (long-term)
  2. Derive Publisher X25519 key from Ed25519 seed
  3. Generate Creator ephemeral X25519 keypair
  4. ECDH → shared secret → HKDF-SHA256("MCN-v1", shared_secret, session_id) → AES key
  5. Per-chunk: AES-256-GCM encrypt with nonce = `nonce_base XOR chunk_index`
- **Key test:** Encrypt with Creator side, decrypt with Publisher side, verify byte-identical plaintext
- **Libraries:** `x25519-dalek`, `aes-gcm`, `hkdf`, `sha2`, `blake3`

#### `mcn-router-sim`
- **Purpose:** Multipath relay across real EC2 instances in different availability zones
- **Implementation:**
  - Each relay is a Rust binary deployed to a separate EC2 Spot instance utilizing `Noise_XX` handshakes.
  - Generates Kademlia `RelayDescriptors` and simulates a rudimentary Node Directory.
  - Creator's "Circuit Manager" performs Telescopic Handshakes (nested envelope encryption) to mathematically validate end-to-end routing sequences.
  - Configurable artificial jitter (50-500ms) added to test Circuit Timeout logic.
  - Simulates Malicious Nodes attempting blackhole/sinkhole routes.
- **Key test:** All chunks traverse real network hops (with real latency + jitter) and arrive at Publisher
- **What this proves vs localhost:** Real TCP connection establishment, real packet reordering, real cross-AZ bandwidth constraints

#### `mpub-receiver`
- **Purpose:** Receive chunks from multiple paths, buffer, verify, decrypt, reassemble
- **Implementation:**
  1. Listen on N TCP ports (one per simulated path)
  2. Buffer incoming encrypted chunks, keyed by `session_id + chunk_index`
  3. When manifest arrives: derive session key via ECDH, decrypt each chunk, verify BLAKE3 hash
  4. Reassemble in manifest order, strip padding from final chunk
  5. Write reconstructed video file
- **Key test:** SHA-256 of original sanitized video === SHA-256 of reassembled video

#### `proto-cli`
- **Purpose:** Single command that runs the entire pipeline end-to-end
- **Usage:** `gbn-proto run --input video.mp4 --publisher-key <pubkey> --paths 3 --hops 3`
- **Output:** Reassembled video + detailed timing report + verification result

---

## 4. Test Plan

### 4.1 Correctness Tests

| Test ID | Test Name | Pass Criteria |
|---|---|---|
| T1.1 | **Metadata Strip Completeness** | `exiftool` and `mediainfo` on sanitized output show zero device-identifying fields (GPS, camera model, software version, creation time) |
| T1.2 | **Chunk-Reassemble Identity (no encryption)** | SHA-256(original_sanitized) == SHA-256(reassembled) with encryption disabled |
| T1.3 | **Chunk-Reassemble Identity (with encryption)** | SHA-256(original_sanitized) == SHA-256(reassembled) with full AES-256-GCM pipeline |
| T1.4 | **Out-of-Order Reassembly** | Deliberately deliver chunks in reverse order; video still reconstructs perfectly |
| T1.5 | **Multipath Reassembly** | Send chunks across 5 independent simulated paths with random 50-500ms jitter per hop; video reconstructs perfectly |
| T1.6 | **Tamper Detection (single bit)** | Flip 1 bit in a random chunk's ciphertext; Publisher MUST reject that chunk with GCM auth failure |
| T1.7 | **Tamper Detection (chunk swap)** | Swap the ciphertext of chunk 5 and chunk 10 (keep headers); Publisher MUST reject both (nonce mismatch) |
| T1.8 | **Wrong Publisher Key** | Attempt decryption with a different Publisher key; ALL chunks MUST fail to decrypt |
| T1.9 | **Session Key Isolation** | Two upload sessions to the same Publisher produce different session keys; chunks from session A cannot be decrypted with session B's key |
| T1.10 | **Large File (500MB)** | Full pipeline on a 500MB video completes successfully; verify file integrity |

### 4.2 Performance Benchmarks

| Benchmark | Target | Environment | Measurement |
|---|---|---|---|
| B1.1 | Metadata stripping (500MB) | Creator instance (t3.small) | < 10 seconds |
| B1.2 | Chunking + BLAKE3 hashing (500MB) | Creator instance | < 5 seconds |
| B1.3 | Encryption (500MB, all chunks) | Creator instance | < 15 seconds |
| B1.4 | Full pipeline (500MB, 3 paths × 3 hops) | Cross-AZ, real network | < 300 seconds |
| B1.5 | Memory usage during 500MB pipeline | Creator instance | < 50MB peak (streaming) |
| B1.6 | BLAKE3 single-chunk hash (1MB) | Creator instance | < 1ms |
| B1.7 | Cross-AZ relay hop latency (per hop) | Relay → Relay | Measure actual; expect 1-5ms |
| B1.8 | Total network transfer time (500MB, 3 paths) | Creator → Publisher via relays | < 180 seconds |

### 4.3 Security Validation Tests

| Test ID | Test Name | What It Proves |
|---|---|---|
| S1.1 | **No plaintext on disk** | During the entire pipeline, scan /tmp and working directories; no unencrypted chunk files exist outside of memory |
| S1.2 | **Ephemeral key destruction** | After upload completes, the Creator's ephemeral private key is zeroed from memory (use `zeroize` crate) |
| S1.3 | **Relay sees only ciphertext** | Inject a logging relay that records all bytes passing through it; verify no plaintext video bytes appear |
| S1.4 | **No relay sees both Creator IP and Publisher IP** | In the 3-hop simulation, verify hop 1 logs show Creator IP but NOT Publisher IP; hop 3 logs show Publisher IP but NOT Creator IP |
| S1.5 | **Metadata stripping edge cases** | Test with: iPhone ProRes, Android HEVC, GoPro w/ GPS telemetry, DJI drone with flight data, OBS screen recording |
| S1.6 | **Telescopic Sinkhole Attack** | Simulate a Guard node dropping a Middle handshake and faking success; Circuit Manager MUST block it |
| S1.7 | **DHT Publisher Spoofing** | Adversary injects IP with invalid signature into DHT; Circuit Manager MUST reject it due to Root of Trust violation |
| S1.8 | **Pre-Flight Blackhole** | Simulate an EC2 node firewalled from the Internet; node MUST abort DHT RelayDescriptor announcement |
| S1.9 | **Heartbeat Fallback & Rebuild** | Simulate node loss during media transmission (kill EC2 instance mid-transfer); Circuit Manager MUST requeue dropped chunks to new circuits with disjoint Guards |

---

## 5. Tech Stack Validation

| Technology | What We're Proving | Fallback if Fails |
|---|---|---|
| **Rust** | Can we build the full crypto pipeline in pure Rust with reasonable ergonomics? | N/A — Rust is non-negotiable for memory safety |
| **x25519-dalek** | ECDH key agreement produces correct shared secrets | Switch to `ring` crate |
| **aes-gcm** crate | Per-chunk AEAD encryption with nonce derivation works correctly | Switch to `ring::aead` |
| **blake3** crate | Hashing performance meets targets on ARM (mobile proxy) | Fall back to SHA-256 (slower but proven) |
| **snow** (Noise Protocol) | Nested `Noise_XX` protocol envelopes securely validate Telescopic links | Require custom libsodium handshake implementation |
| **libp2p-kad** (or equiv) | Kademlia DHT implementation successfully resolves `RelayDescriptors` | Custom bare-bones hash table fallback |
| **FFmpeg CLI** | Metadata stripping is complete across all containers | May need custom Matroska/MP4 parser for edge cases |
| **tokio** | Async I/O for relay network performs adequately across real TCP connections | N/A — tokio is standard |
| **AWS CloudFormation** | Can we define the full test infrastructure as code and launch/teardown reliably? | Terraform or AWS CDK |
| **EC2 Spot Instances** | t3.micro/small Spot instances provide sufficient compute for relay + crypto at minimal cost | On-Demand instances (2-3x cost) |

---

## 6. Success Criteria

Phase 1 is **PASSED** when ALL of the following are true:

- [ ] CloudFormation stack launches successfully and all 7 instances reach `running` state
- [ ] Rust binaries compile and deploy to all instances via bootstrap scripts
- [ ] All 10 correctness tests (T1.1–T1.10) pass on AWS infrastructure
- [ ] All 9 security validation tests (S1.1–S1.9) pass on AWS infrastructure
- [ ] User-provided 500MB video completes the full pipeline in < 300 seconds (cross-AZ)
- [ ] Peak memory usage stays below 50MB on the Creator instance during processing
- [ ] The reassembled video on the Publisher instance is byte-identical to the sanitized original (SHA-256 match)
- [ ] CloudFormation stack tears down cleanly with zero orphaned resources

## 7. AWS Deployment Workflow

```
Step 1: User provides sample video file(s)
Step 2: Run `aws cloudformation create-stack --template-file phase1-stack.yaml`
Step 3: Wait for stack creation (~3-5 minutes for Spot fulfillment)
Step 4: Run `deploy-creator.sh` — uploads binary + test video to Creator instance
Step 5: Run `deploy-relays.sh` — uploads relay binary to all relay instances
Step 6: Run `deploy-publisher.sh` — uploads receiver binary to Publisher instance
Step 7: Run `run-tests.sh` — SSHs into Creator, executes full test suite
         ├── Sanitize video on Creator instance
         ├── Chunk + encrypt on Creator instance
         ├── Send chunks through 3 relay paths (each crossing AZ boundaries)
         ├── Receive + decrypt + reassemble on Publisher instance
         └── Verify SHA-256 match + collect timing metrics
Step 8: Run `teardown.sh` — destroys CloudFormation stack, stops billing
```

## 8. Known Limitations of This Prototype

| Limitation | Why It's Acceptable |
|---|---|
| No cover traffic or timing jitter analysis | Phase 3 validates complex traffic analysis resistance and constant-rate obfuscation |
| No UI — CLI only | UI is a Phase 4+ concern; CLI proves the core logic |
| Same AWS region (different AZs, not different regions) | Cross-region adds cost; cross-AZ proves real network traversal with measurable latency |
| Spot instances may be interrupted | Re-run stack; tests are idempotent; interruption is unlikely for <1hr test runs |
