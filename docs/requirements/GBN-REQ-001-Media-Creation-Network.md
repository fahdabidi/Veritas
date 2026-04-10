# GBN-REQ-001 — Media Creation Network: Requirements

**Document ID:** GBN-REQ-001  
**Version:** 0.1 (Draft)  
**Status:** In Review  
**Last Updated:** 2026-04-07  
**Parent:** [GBN-REQ-000](GBN-REQ-000-Top-Level-Requirements.md)  
**Architecture:** [GBN-ARCH-001](../architecture/GBN-ARCH-001-Media-Creation-Network.md)

---

## 1. Overview

The **Media Creation Network (MCN)** is the entry point of the GBN system for video content creators. Its sole purpose is to allow a Creator to upload a video to a designated Publisher with maximum anonymity — hiding the Creator's identity, network origin, device characteristics, and behavioral patterns from all parties except the intended Publisher (who receives the assembled video, but NOT the Creator's identity).

The MCN operates on the principle of **zero-knowledge transit**: relay nodes in the network know only enough to forward packets; no node along the chain can determine both the source and destination of a transmission.

### 1.1 Scope

| In Scope | Out of Scope |
|---|---|
| Video capture integration and pre-processing | Live streaming (Phase 1) |
| Metadata stripping and sanitization | Video transcoding / quality conversion |
| Video chunking and per-chunk encryption | Publisher-side reassembly (see GBN-REQ-002) |
| Multi-hop routing of encrypted chunks | Permanent content storage (see GBN-REQ-003) |
| Selection and designation of a target Publisher | Publisher discovery (Publishers are identified by pre-shared public key) |
| Delivery confirmation from Publisher | |

---

## 2. Stakeholders & Actors

| Actor | Role in MCN |
|---|---|
| **Creator** | Initiates upload; the identity to be protected |
| **MCN Client App** | Software running on Creator's device; performs all anonymization and upload steps |
| **Relay Node** | Volunteer node that forwards encrypted packets without being able to read them |
| **Publisher** | Final recipient of the assembled video chunks; identified by their public key |
| **Adversary (ISP/Firewall)** | Monitors network traffic; attempts to identify Creator or block transmission |

---

## 3. Functional Requirements

### 3.1 Pre-Processing & Anonymization

| ID | Requirement | Priority |
|---|---|---|
| MCN-FR-001 | The MCN Client SHALL strip all metadata from video files before processing, including EXIF, GPS coordinates, device identifiers, creation timestamps, encoder version strings, and container-level tags | **Must** |
| MCN-FR-002 | The MCN Client SHALL replace the video creation timestamp with a normalized value (e.g., Unix epoch 0) or a user-selected approximate date | **Must** |
| MCN-FR-003 | The MCN Client SHALL re-encode video container headers to remove software fingerprints (e.g., "recorded with iPhone 15") | **Must** |
| MCN-FR-004 | The MCN Client SHOULD offer an optional visual anonymization mode that automatically detects and blurs faces, license plates, and other identifying visual elements | **Should** |
| MCN-FR-005 | The MCN Client SHOULD offer an optional audio anonymization mode that applies voice distortion without degrading intelligibility | **Should** |
| MCN-FR-006 | The MCN Client SHALL allow the Creator to preview the sanitized video before upload | **Should** |
| MCN-FR-007 | All pre-processing SHALL occur locally on the Creator's device; no raw video SHALL ever leave the device | **Must** |

### 3.2 Chunking

| ID | Requirement | Priority |
|---|---|---|
| MCN-FR-010 | The MCN Client SHALL split the sanitized video into fixed-size chunks (configurable, default: 1MB) | **Must** |
| MCN-FR-011 | Each chunk SHALL be independently addressable by a BLAKE3 hash of its plaintext content | **Must** |
| MCN-FR-012 | The chunking process SHALL produce a chunk manifest listing all chunk hashes, their sequence order, and total count | **Must** |
| MCN-FR-013 | Chunks SHALL NOT need to arrive at the Publisher in order; the manifest enables out-of-order reassembly | **Must** |
| MCN-FR-014 | The MCN MAY send different chunks via different independent multi-hop paths simultaneously, increasing throughput and path diversity | **Should** |

### 3.3 Encryption

| ID | Requirement | Priority |
|---|---|---|
| MCN-FR-020 | Each chunk SHALL be encrypted with a unique, session-derived symmetric key using AES-256-GCM | **Must** |
| MCN-FR-021 | The symmetric keys SHALL be derived from an ephemeral ECDH (X25519) key exchange, using the Publisher's long-term public key as one party | **Must** |
| MCN-FR-022 | The ephemeral creator-side key pair SHALL be generated fresh per upload session and discarded afterward | **Must** |
| MCN-FR-023 | The chunk manifest SHALL be encrypted separately and delivered only to the Publisher at the end of upload | **Must** |
| MCN-FR-024 | The encryption scheme SHALL provide authenticated encryption (AEAD) — any tampered chunk SHALL be detectable | **Must** |
| MCN-FR-025 | No relay node SHALL ever have access to a plaintext chunk or to the session encryption key | **Must** |

### 3.4 Multi-Hop Routing

| ID | Requirement | Priority |
|---|---|---|
| MCN-FR-030 | Encrypted chunks SHALL be routed through a minimum of 3 relay hops before reaching the Publisher | **Must** |
| MCN-FR-031 | Each relay node SHALL see only: the address of the previous hop and the address of the next hop; it SHALL NOT see the Creator's origin or the Publisher's destination | **Must** |
| MCN-FR-032 | Routing information for each hop SHALL be encrypted using that relay node's public key (onion layering) | **Must** |
| MCN-FR-033 | Relay nodes SHALL be selected randomly from the available relay pool, weighted by reputation score | **Must** |
| MCN-FR-034 | The MCN Client SHALL implement circuit isolation (multipath routing) — different chunks from the same video MUST be sent through independent relay circuits to prevent single-Exit-Node data capture | **Must** |
| MCN-FR-035 | The MCN SHALL support a "guard node" concept — a stable first hop chosen from a high-reputation subset. If a guard drops abruptly, the Circuit Manager SHALL select a completely distinct guard to prevent temporal rebuild correlation | **Should** |
| MCN-FR-036 | The MCN Client SHALL add randomized timing jitter (50–500ms) between chunk transmissions to resist timing correlation attacks | **Should** |
| MCN-FR-036a | The Circuit Manager SHALL track all unacknowledged chunks and immediately reassign them to surviving parallel circuits or newly dialed circuits if a circuit collapses or a `CHUNK_ACK` times out | **Must** |
| MCN-FR-037 | The MCN Client SHOULD send cover traffic (dummy encrypted packets) during upload pauses to prevent upload-timing fingerprinting | **Could** |
| MCN-FR-038 | The Circuit Manager SHALL preferentially select Exit Nodes (Hop 3) located in different geographic jurisdictions from the Creator to prevent inherited geo-fencing | **Must** |
| MCN-FR-039 | Upon circuit extension failure to the Publisher (e.g. Exit node unable to reach Publisher due to local geo-fence), the Circuit Manager SHALL automatically tear down the circuit and retry with a new Exit Node in a different region | **Must** |

### 3.5 Publisher Selection & Designation

| ID | Requirement | Priority |
|---|---|---|
| MCN-FR-040 | Publishers SHALL be identified solely by their Ed25519 public key; no IP address or domain name is required | **Must** |
| MCN-FR-041 | The MCN Client SHALL resolve a Publisher's current network address via a DHT lookup keyed to their public key | **Must** |
| MCN-FR-042 | The MCN Client SHALL verify the Publisher's identity by validating their DHT announcement signature before initiating upload | **Must** |
| MCN-FR-043 | The MCN Client SHALL support importing a Publisher public key via: QR code scan, deep-link URL, or manual hex input | **Must** |
| MCN-FR-044 | The MCN Client SHOULD display a human-readable "Publisher fingerprint" (first 8 bytes of public key, displayed as words) for user verification | **Should** |

### 3.6 Delivery Confirmation

| ID | Requirement | Priority |
|---|---|---|
| MCN-FR-050 | The Publisher SHALL send a signed acknowledgment message to the Creator's MCN Client upon successful reassembly | **Must** |
| MCN-FR-051 | The acknowledgment SHALL be routed back through the overlay network (not directly to Creator's IP) | **Must** |
| MCN-FR-052 | The MCN Client SHALL display upload progress as a percentage of confirmed-received chunks | **Should** |
| MCN-FR-053 | If upload is interrupted, the MCN Client SHALL support resuming from the last confirmed chunk | **Should** |

---

## 4. Non-Functional Requirements

| ID | Requirement | Priority |
|---|---|---|
| MCN-NFR-001 | A 500MB video SHALL complete upload within 30 minutes over a 1Mbps uplink under typical relay conditions | **Should** |
| MCN-NFR-002 | Pre-processing (metadata strip + chunk + encrypt) SHALL complete within 60 seconds for a 500MB file | **Should** |
| MCN-NFR-003 | The MCN Client SHALL function natively on Android 8.0+ (via Kotlin/Rust FFI) and desktop environments (Linux, macOS, Windows) | **Must** |
| MCN-NFR-004 | The MCN Client SHALL consume no more than 500MB RAM during upload of a 4GB file (streaming chunks, not loading all into memory) | **Must** |
| MCN-NFR-005 | The MCN Client SHALL clearly communicate to the Creator when a Publisher is unreachable, rather than silently failing | **Must** |
| MCN-NFR-006 | The MCN SHALL never write unencrypted video chunks to disk; chunking and encryption SHALL happen in-memory | **Must** |
| MCN-NFR-007 | The MCN Client SHALL support decentralized distribution mechanisms (e.g., direct APK sideloading, F-Droid) to bypass centralized App Store censorship. Platforms that strictly prohibit sideloading or P2P app distribution (e.g., iOS) SHALL be out of scope for full MCN node deployment. | **Must** |

---

## 5. Data Requirements

### 5.1 Data Flows

| Data Item | Source | Destination | Encrypted? | Who can decrypt? |
|---|---|---|---|---|
| Raw video file | Creator's camera | MCN Client (local only) | N/A (local) | Creator only |
| Sanitized video | MCN Client | Never leaves device pre-encryption | — | — |
| Encrypted chunk | MCN Client | Relay hop 1 → 2 → 3 → Publisher | Yes (AES-256-GCM) | Publisher only |
| Chunk manifest | MCN Client | Publisher (via separate routed path) | Yes | Publisher only |
| Session public key | MCN Client | Embedded in encrypted envelope | Yes (encrypted with Publisher key) | Publisher only |
| Upload acknowledgment | Publisher | MCN Client (via overlay) | Yes | Creator session |

### 5.2 Chunk Format

```
+-----------------------------+
| Chunk Header (encrypted)    |
|  - session_id (16 bytes)    |
|  - chunk_index (4 bytes)    |
|  - total_chunks (4 bytes)   |
|  - chunk_hash (32 bytes)    |
+-----------------------------+
| Encrypted Payload           |
|  - AES-256-GCM ciphertext   |
|  - GCM authentication tag   |
+-----------------------------+
```

---

## 6. Interface Requirements

| Interface | Type | Description |
|---|---|---|
| **MCN ↔ BON** | Internal API | MCN passes encrypted packets to BON for routing |
| **MCN ↔ DHT** | DHT query | MCN queries DHT for Publisher's current BON address |
| **MCN Client UI** | Mobile/Desktop UI | Publisher key import, upload progress, anonymization options |
| **Publisher entry point** | BON-routed socket | Publisher exposes a BON-addressed endpoint for receiving chunks |

---

## 7. Threat Model

| Threat | Severity | Mitigation |
|---|---|---|
| **ISP logs Creator's upload to a specific IP** | Critical | All traffic goes through BON; first hop sees only encrypted data to next relay |
| **Exit Node Inherited Geo-fencing** | High | MCN-FR-038/039: Circuit Manager selects geographically diverse exit nodes and retries automatically upon destination unreachable errors |
| **Relay node logs packet source + destination for correlation** | Critical | Onion encryption; each relay sees only its adjacent hops |
| **Adversary operates many relay nodes (Sybil) to control circuits** | High | Circuit selection weights by reputation; guard nodes; circuit isolation |
| **Video retains device fingerprint in codecs** | High | MCN-FR-003: re-encode container headers; remove encoder metadata |
| **Upload timing enables correlation even without IP** | Medium | MCN-FR-036/037: timing jitter and cover traffic |
| **Publisher is compromised and reveals Creator origin** | Medium | Publisher never learns Creator's IP (only the last relay's IP) |
| **Exit Node steals Publisher key to decrypt upload** | High | MCN-FR-034: Multipath routing ensures no single Exit node receives enough chunks to reassemble the video, even with the key |
| **Chunk replay attack** | Low | GCM authentication tag; session_id in chunk header prevents cross-session replay |

---

## 8. Open Questions

| ID | Question | Impact |
|---|---|---|
| OQ-MCN-001 | Should the MCN support segmented uploads where the Creator can be offline between chunks (store-and-forward via relay)? | Medium — needed for very slow connections |
| OQ-MCN-002 | What is the minimum number of available relay nodes before the MCN declares the network unusable and alerts the Creator? | Low — UX design decision |
| OQ-MCN-003 | Should the MCN support multiple Publisher targets per upload (CC: a second outlet)? | Medium — complicates key management |
| OQ-MCN-004 | Should the MCN perform local CSAM hash-matching before upload, to prevent the Creator from unknowingly transmitting illegal content? | High — ethical and legal duty-of-care question |
