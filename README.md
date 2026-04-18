# ðŸŒ Global Broadcast Network (GBN) â€” Prototype Workspace

**A decentralized, censorship-resistant video creation, publishing, and distribution platform â€” designed so truth can travel faster than it can be suppressed.**

> *"The internet treats censorship as damage and routes around it."*
> â€” John Gilmore

---

## âš ï¸ Project Status

This repository is an **active prototype** (`gbn-proto`) for validating core architecture and security assumptions.

- âœ… Core Rust workspace and crate boundaries are in place
- âœ… Integration test scaffolding exists for metadata stripping, multipath reassembly, tamper detection, and end-to-end pipeline tests
- ðŸš§ CLI orchestration commands are partially implemented (see `crates/proto-cli/src/main.rs`)
- ðŸš§ Not production-ready; APIs and protocols are expected to evolve during prototyping

If you are looking for full system design docs (requirements, architecture, security), see [`../../docs/`](../../docs/).

---

## Quick Start

### Prerequisites

- Rust 1.77+
- FFmpeg 6.0+
- (Optional for infra simulation) AWS CLI + Docker

### 1) Build the workspace

```bash
cargo build --workspace
```

### 2) Run tests

```bash
cargo test --workspace
```

### 3) Add local test videos (for media pipeline tests)

Place `.mp4` files in [`test-vectors/`](./test-vectors/) (this directory is gitignored).

See [`test-vectors/README.md`](./test-vectors/README.md) for expected files and guidance.

### 4) (Optional) AWS phase infrastructure

For EC2-based prototype runs and teardown, see [`infra/README-infra.md`](./infra/README-infra.md).

---

## Vision & Mission

In many countries, a journalist who records police violence, corruption, or protests faces an impossible choice: **publish and be identified, or stay silent and stay safe**.

Existing options leave major gaps:
- **Mainstream platforms** can remove content centrally and log subpoenaable metadata.
- **Tor + generic file sharing** protects uploader routing but does not provide an integrated publisher trust + distribution pipeline.
- **VPNs** shift trust to the VPN operator.

The **Global Broadcast Network** aims to provide a complete, end-to-end pipeline â€” from capture to playback â€” such that no single point of failure can trivially identify creators or suppress distribution.

### Design Principles

| Principle | What It Means In Practice |
|---|---|
| ðŸ”’ **Privacy by Default** | End-to-end encryption and local metadata sanitization before transmission |
| ðŸŒ **Resilience over Efficiency** | Erasure-coded distribution across geographically diverse nodes |
| âš–ï¸ **Legal Responsibility at the Edges** | Editorial/legal responsibility is with Publishers and Content Providers |
| ðŸ§¬ **Adaptive to Adversaries** | Pluggable transport strategy evolves against censorship techniques |
| ðŸ›¡ï¸ **Sovereign Updates** | Supply-chain hardening via reproducible builds and multi-party governance (see [GBN-SEC-007](../../docs/security/GBN-SEC-007-Software-Supply-Chain.md)) |

---

## How It Works

**The Root of Trust:** The user journey strictly begins prior to recording the video. The Creator must first establish cryptographic trust by scanning the Publisher's Public Key via a QR code (or by downloading a pre-seeded Sovereign Publisher App). This ensures the MCN encrypts data specifically for that Publisher and structurally prevents adversary traffic interception.

### Journey of a Video

```
  CREATOR                      RELAY NETWORK                       PUBLISHER
  (hostile jurisdiction)       (3-hop onion routing)               (trusted entity)

 +---------------------+                                       +---------------------+
 | 1. Record video     |       +=======================+       | 5. Receive chunks   |
 | 2. Strip metadata   |       |  Path 1               |       |    (out-of-order)   |
 |    (GPS, device ID, |------>|  Guard > Middle > Exit |------>| 6. Decrypt each     |
 |     timestamps)     |       +=======================+       | 7. Verify BLAKE3    |
 | 3. Chunk (1MB each) |       +=======================+       | 8. Reassemble video |
 | 4. Encrypt chunks   |------>|  Path 2 (diff circuit) |------>| 9. Editorial review |
 |    (AES-256-GCM)    |       +=======================+       |10. Sign (Ed25519)   |
 |                     |       +=======================+       |                     |
 |                     |------>|  Path 3 (diff circuit) |------>|                     |
 +---------------------+       +=======================+       +----------+----------+
                                                                          |
                          GLOBAL DISTRIBUTED STORAGE                      |
                        +---------------------------------------------<---+
                        |
                        v
 +------------------------------------------------------------------------------+
 |  Reed-Solomon erasure coding: split into 20 shards (14 data + 6 parity).    |
 |  Distribute across volunteer nodes worldwide. ANY 14 of 20 shards can       |
 |  reconstruct the original. Content survives seizure of 6 nodes. Each shard  |
 | can have many replicas                                                      |
 +-------------------------------------+----------------------------------------+
                                        |
                          VIEWER         |
                        +----------------+
                        |
                        v
              +-------------------+
              | Discover content  |
              | via peer gossip   |
              |        |          |
              | Fetch 14 of 20   |
              | shards via BON   |
              |        |          |
              | Reconstruct and  |
              | play video       |
              +-------------------+
```


## Packet Path

**Path/Return_Path**: Creator → Guard → Middle → Exit → Publisher

The path is created by the creator from its DHT which has been populated by the gossip network

**Onion build (Creator, innermost first):**

```
layer_pub  = seal(publisher_pub,  { next_hop: None,chunk_payload, chunk_id, chunk_hash, return_path, send_timestamp, total_chunks, chunk_index })
layer_exit = seal(exit_pub,       { next_hop: publisher_addr, inner: layer_pub  })
layer_mid  = seal(middle_pub,     { next_hop: exit_addr,      inner: layer_exit })
layer_grd  = seal(guard_pub,      { next_hop: middle_addr,    inner: layer_mid  })
```

Creator sends `layer_grd` over TCP to Guard.

**Each relay (Guard / Middle / Exit):**
1. Read length-prefixed bytes from TCP
2. `open(own_priv, bytes)` → `{ next_hop, inner }`
3. Connect to `next_hop`, write `inner` as length-prefixed bytes
4. (No response needed for data forwarding)

**Publisher:**
1. `open(own_priv, bytes)` → `{ next_hop: None,chunk_payload, chunk_id, chunk_hash, return_path, send_timestamp, total_chunks, chunk_index }`
2. Verify hash, store chunk
3. Build reverse-direction ACK (ChunkID, Receive Timestamp, Hash, ChunkIndex) onion using `return_path` → send back to Creator

**ACK return path**: Publisher → Exit → Middle → Guard → Creator
Creator must listen on an ACK port; return_path contains Creator's ack address.

---


### What each participant can observe

```text
Creator      â†’ Sees: local video + target Publisher key
               Sees full relay topology and Pub Keys

Guard relay  â†’ Sees: previous hop + next hop
               Cannot see: payload plaintext or final destination context

Middle relay â†’ Sees: adjacent hops only
               Cannot see: creator identity, publisher identity, or content plaintext

Exit relay   â†’ Sees: prior hop and destination endpoint
               Cannot see: origin creator identity or content plaintext

Publisher    â†’ Sees: decrypted submitted content
               Can see: full relay topology and Pub Keys back to creator for Ack message

Storage node â†’ Sees: encrypted shards by content-addressed ID
               Cannot see: plaintext media

Viewer       â†’ Sees: playable stream/content
               Cannot see: creator identity or full relay path
```

### Prototype components in this workspace

| Component | Purpose (prototype scope) |
|---|---|
| `gbn-protocol` | Shared types/contracts (chunks, manifests, crypto/error primitives) |
| `mcn-sanitizer` | Metadata sanitization pipeline |
| `mcn-chunker` | Chunking and hash-oriented segmentation |
| `mcn-crypto` | Key exchange + encryption flow |
| `mcn-router-sim` | Telescopic Onion Router simulation over Kademlia DHT |
| `mpub-receiver` | Publisher-side receive/reassemble prototype path |
| `proto-cli` | CLI orchestrator for prototype workflows |

---

## Repository Layout

```text
gbn-proto/
â”œâ”€â”€ Cargo.toml
â”œâ”€â”€ README.md
â”œâ”€â”€ crates/
â”‚   â”œâ”€â”€ gbn-protocol/
â”‚   â”œâ”€â”€ mcn-sanitizer/
â”‚   â”œâ”€â”€ mcn-chunker/
â”‚   â”œâ”€â”€ mcn-crypto/
â”‚   â”œâ”€â”€ mcn-router-sim/
â”‚   â”œâ”€â”€ mpub-receiver/
â”‚   â””â”€â”€ proto-cli/
â”œâ”€â”€ infra/
â”‚   â”œâ”€â”€ README-infra.md
â”‚   â”œâ”€â”€ cloudformation/
â”‚   â””â”€â”€ scripts/
â”œâ”€â”€ test-vectors/
â”‚   â””â”€â”€ README.md
â””â”€â”€ tests/
    â””â”€â”€ integration/
        â”œâ”€â”€ test_metadata_stripping.rs
        â”œâ”€â”€ test_multipath_reassembly.rs
        â”œâ”€â”€ test_tamper_detection.rs
        â””â”€â”€ test_full_pipeline.rs
```

---

## Technical Stack (Prototype)

| Layer | Technology | Why |
|---|---|---|
| Core implementation | Rust | Memory safety + performance for protocol/security-critical paths |
| Crypto primitives | `x25519-dalek`, `aes-gcm`, `ed25519-dalek`, `blake3`, `hkdf` | Modern, auditable Rust crypto ecosystem |
| Async runtime | Tokio | Mature async I/O runtime |
| Erasure coding target (planned) | `reed-solomon-erasure` | k-of-n reconstruction model |
| Metadata stripping | FFmpeg (CLI integration) | Broad container support |
| Mobile target (planned) | Kotlin + Rust FFI | Native Android UX with shared Rust core |

> Note: Some architectural docs discuss future VCP service implementations in Go. Those are design-stage targets, not part of this prototype workspace.

---

## Prototyping Phases

### Phase 1 â€” Media Creation Network & zero-trust routing
ðŸ“„ Plan: [`../../docs/prototyping/GBN-PROTO-001-Phase1-Media-Creation.md`](../../docs/prototyping/GBN-PROTO-001-Phase1-Media-Creation.md)

### Phase 2 â€” Publishing & distributed storage
ðŸ“„ Plan: [`../../docs/prototyping/GBN-PROTO-002-Phase2-Publishing-Storage.md`](../../docs/prototyping/GBN-PROTO-002-Phase2-Publishing-Storage.md)

### Phase 3 â€” Overlay broadcast network & playback
ðŸ“„ Plan: [`../../docs/prototyping/GBN-PROTO-003-Phase3-Broadcast-Playback.md`](../../docs/prototyping/GBN-PROTO-003-Phase3-Broadcast-Playback.md)

---

## Security Model (Summary)

GBN uses a **Zero-Knowledge Transit** design goal: intermediate nodes should know only what is necessary for forwarding.

Detailed security docs:
- [GBN-SEC-001 â€” Media Creation Network](../../docs/security/GBN-SEC-001-Media-Creation-Network.md)
- [GBN-SEC-002 â€” Media Publishing](../../docs/security/GBN-SEC-002-Media-Publishing.md)
- [GBN-SEC-003 â€” Global Distributed Storage](../../docs/security/GBN-SEC-003-Global-Distributed-Storage.md)
- [GBN-SEC-004 â€” Video Content Providers](../../docs/security/GBN-SEC-004-Video-Content-Providers.md)
- [GBN-SEC-005 â€” Video Playback App](../../docs/security/GBN-SEC-005-Video-Playback-App.md)
- [GBN-SEC-006 â€” Broadcast Network](../../docs/security/GBN-SEC-006-Broadcast-Network.md)
- [GBN-SEC-007 â€” Software Supply Chain](../../docs/security/GBN-SEC-007-Software-Supply-Chain.md)

### Dynamic Circuit Rebuilding & Anonymity

Because the GBN relies on consumer devices scaling dynamically to provide routing services, node churn is inevitable. The architecture implements **Active Heartbeat Disconnects** over the inner `Noise_XX` layer, enabling near-instantaneous detection of relay failure. Upon failure, dropping circuits immediately release un-ACKed chunks into a reassignment queue, dialing fresh circuits. To resist **Temporal Circuit Correlation** (adversaries mapping sequential circuit rebuilds to origin metadata), replacement circuits explicitly select completely separate Guard hubs — rendering temporal drops disjoint and preserving anonymity.

### Important limitations

As documented in the security files, the system **does not fully mitigate**:
- endpoint compromise (malware/physical seizure)
- global passive adversary traffic correlation (partially mitigated)
- complete internet shutdown/physical disconnection events

---

## Documentation Index

All system-level docs live under [`../../docs/`](../../docs/):

- Requirements: `../../docs/requirements/GBN-REQ-*.md`
- Architecture: `../../docs/architecture/GBN-ARCH-*.md`
- Security: `../../docs/security/GBN-SEC-*.md`
- Prototyping: `../../docs/prototyping/GBN-PROTO-*.md`
- Research: `../../docs/research/GBN-RESEARCH-*.md`

---

## Contributing (Prototype)

Contributions are welcome for prototype hardening, test coverage, and correctness improvements.

Suggested contribution flow:
1. Open an issue describing the problem or enhancement
2. Propose scope aligned to the active prototype phase
3. Submit a PR with tests (`cargo test --workspace`)
4. Include doc updates when behavior/protocol assumptions change

---

## License

This prototype workspace is currently licensed under **AGPL-3.0-or-later** (see workspace `Cargo.toml`).
