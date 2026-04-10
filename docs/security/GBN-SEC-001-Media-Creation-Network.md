# GBN-SEC-001 — Product Security Document: Media Creation Network

**Document ID:** GBN-SEC-001  
**Component:** Media Creation Network (MCN)  
**Status:** V1.0  

---

## 1. Executive Summary

The Media Creation Network (MCN) is the most critical privacy barrier in the Global Broadcast Network. Its purpose is to ingest video from a Creator, strip it of all identifying markers, and deliver it to a Publisher such that **the network cannot identify the content, and the Publisher cannot identify the Creator**. 

The MCN operates on a "Zero-Knowledge Transit" model. It assumes the device is the only trusted harbor, and that every network hop—including the internet service provider (ISP) and the relay nodes themselves—is hostile and actively attempting to surveil, correlate, or block the transmission.

## 2. Security Model & Trust Boundaries

### 2.1 Trust Assumptions
* **Trusted:** The Creator's local device hardware and OS (pre-encryption boundaries).
* **Untrusted:** The Creator's local ISP, national firewalls, MCN relay nodes (Guard, Middle, Exit), and the Publisher's physical location.

### 2.2 Security Architecture
To defend the Creator, the MCN uses a layered defense system:
1. **Local Pre-Processing:** Video container headers and metadata (EXIF, GPS, device fingerprints) are destroyed locally using FFmpeg-style stripping before any network connection is initiated.
2. **Hybrid End-to-End Encryption:** Chunks are encrypted with AES-256-GCM using session keys derived from an ephemeral X25519 key exchange with the Publisher's public key. The Creator's identity exists only as an ephemeral mathematical construct that is discarded immediately after upload.
3. **Multi-Hop Onion Routing & Circuit Isolation:** Encrypted chunks are wrapped in routing layers. A minimum of three nodes are used. The Guard node sees the Creator's IP but not the payload or destination. The Exit node sees the Publisher but not the Creator's IP. Furthermore, the MCN uses **multipath routing**, sending different chunks of the same video through entirely different relay circuits. Even if an adversary compromises an Exit node AND steals the Publisher's private key, they only possess a useless, fragmented percentage of the video file because the rest of the chunks took different paths.
4. **Chunk-Then-Encrypt Architecture:** The MCN chunks the sanitized plaintext video *before* encrypting each chunk independently. This prevents memory exhaustion on mobile devices and ensures that if a network error corrupts a single chunk in transit, the Publisher can request retransmission of just that 1MB chunk, rather than failing the decryption authentication tag for the entire multi-gigabyte video file.

---

## 3. Attack Resistance (Mitigated Threats)

### 3.1 Resistance to Disablement & Censorship
If a hostile ISP attempts to disable the MCN by blocking its servers, they will fail because the MCN has no central servers. Traffic is routed via the Broadcast Overlay Network (BON) using pluggable transports (e.g., WebTunnel). The ISP sees only what appears to be an innocent HTTPS connection to a random WebRTC or WebSocket server. Blocking it requires blocking broad swaths of legitimate web traffic.

Additionally, if an Exit Node inherits the geo-fencing of a hostile region (meaning the Exit Node itself cannot reach the Publisher), the MCN Circuit Manager automatically detects the destination-unreachable error, tears down the circuit, and dynamically constructs a new circuit through an Exit Node in a free jurisdiction.

### 3.2 Resistance to App Store Takedowns (Distribution Censorship)
Authoritarian regimes frequently coerce centralized gatekeepers (Google/Apple) into removing dissenting applications from their regional stores. 

The GBN defends against this by utilizing Android's open ecosystem. The application (a Kotlin UI wrapping the Rust core) is designed to be distributed entirely outside of corporate app stores. Users can sideload the APK, download it from independent repositories like F-Droid, or distribute it directly device-to-device via local Bluetooth sharing and compressed QR codes.

iOS inherently fails this threat model. An Apple App Store takedown represents a global, unmitigated "Whiteout" of the app for users in that region (though regional mandates like the EU's Digital Markets Act and AltStore provide highly localized exceptions). Because of this centralized gatekeeping, iOS devices cannot serve as a reliable foundation for the censorship-resistant relay network.

### 3.3 Resistance to Anonymity Circumvention
An adversary attempting to deanonymize the Creator by running their own relay node will fail due to the onion routing. If the adversary controls the Middle node, they see only encrypted traffic from the Guard going to the Exit. Furthermore, the MCN injects randomized timing jitter (50–500ms) between chunk transmissions. This defends against an adversary trying to match packet volumes and timing at the Creator's ISP with packets arriving at the Publisher.

### 3.4 Resistance to Temporal Rebuild Correlation
If an adversary intentionally drops traffic at a node they control (or observes a node dropping offline), they might attempt to correlate the dropped circuit with a newly rebuilt circuit if both use the same Guard node simultaneously. The **Circuit Manager** prevents this by guaranteeing that any replacement circuit dialed to recover dropped chunks uses an entirely distinct path, strictly avoiding the re-use of the previous Guard node.

---

## 4. Formal Threat Model (STRIDE)

| Threat Type | Vector | Mitigation Strategy |
|---|---|---|
| **Spoofing** | Adversary impersonates Publisher to steal upload | The MCN application hard-pins or strictly verifies the Publisher's Ed25519 public key. X25519 key exchange guarantees only the holder of the Publisher's private key can derive the AES decryption key. |
| **Tampering** | Relay node modifies video chunks in transit | AES-256-GCM provides Authenticated Encryption with Associated Data (AEAD). Any flipped bit invalidates the cryptographic tag, prompting the Publisher to drop the chunk. |
| **Repudiation** | Network drops chunks silently | Publisher sends signed ACKs via a reverse BON circuit. MCN client tracks confirmed chunks and retries dropped ones. |
| **Information Disclosure** | ISP inspects packet contents (DPI) | Pluggable transports obfuscate the protocol; AES encryption hides the content. Onion headers hide the final destination. |
| **Information Disclosure** | Metadata extraction from video | Mandatory local pre-processing removes EXIF, timestamps, and resets container header footprints. |
| **Denial of Service** | Adversary floods Creator's app port to crash it | The MCN Client *initiates* outbound connections via BON. It does not run an open listening port that can be trivially flooded by an external ISP. |
| **Elevation of Privilege** | Malicious video file exploiting Publisher's decoder | MCN securely chunks the file. The Publisher relies on secure staging and isolated decoders to prevent RCE during reassembly. |

---

## 5. Unmitigated Threats & Fatal Vulnerabilities

Despite the cryptographic architecture, the MCN *cannot* resist the following attacks. If these occur, the Creator's anonymity or ability to upload will be fatally compromised.

### 5.1 Endpoint Compromise (The "Wrench" Attack)
* **Description:** The Creator's physical device is seized while unlocked, or the device is infected with state-sponsored spyware (e.g., Pegasus) prior to the upload. 
* **Why it succeeds:** Cryptography only protects data in transit and at rest. If the operating system is compromised, the adversary captures the screen or access the raw video file *before* the MCN client strips metadata and encrypts it. 
* **Status:** Unmitigated. Requires external operational security (OpSec) by the Creator (e.g., using a hardened OS like GrapheneOS, rapid screen-locking).

### 5.2 In-Frame Visual & Audio Identification
* **Description:** The video content itself contains identifying markers. For example, a Creator films outside a unique apartment window, captures their face in a reflection, or has a highly identifiable voice.
* **Why it succeeds:** The MCN strips digital metadata, but it cannot fully sanitize semantic context. Algorithms can use voice printing, gait analysis, or geographical triangulation to identify the Creator based purely on the pixels and audio waveforms.
* **Status:** Partially mitigated via optional AI blurring, but ultimately unmitigated against poor manual OpSec.

### 5.3 Global Passive Adversary (GPA) Flow Correlation
* **Description:** An intelligence agency with the ability to monitor the entire internet backbone simultaneously watches the traffic leaving the Creator's ISP and the traffic entering the Publisher's node. 
* **Why it succeeds:** Even with onion routing and timing jitter, deeply resourced adversaries can run statistical correlation models on traffic flows. Over a long enough upload (e.g., a massive 4GB file), the statistical probability of joining the entry flow and the exit flow approaches 100%.
* **Status:** Partially Mitigated. The Publisher utilizes **Dispersed Edge Ingestion**, deploying multiple receiver nodes geographically. The MCN distributes chunks randomly across these edge nodes. The GPA sees massive traffic entering the network, but the exit flow is smeared across multiple jurisdictions and timelines, vastly complicating correlation modeling. True 100% resistance requires a high-latency Mixnet, but Edge Ingestion significantly degrades GPA tracking capabilities.

### 5.4 The "Whiteout" (Total Physical Disconnection)
* **Description:** A government physically severs fiber optic lines to a region or turns off all cellular data towers. 
* **Why it succeeds:** The MCN requires an eventual path to the internet. Pluggable transports disguise traffic, but they cannot transmit without an underlying IP connection.
* **Status:** Unmitigated in Phase 1. Future iterations may require delay-tolerant mesh networking (sneakernet) to bounce data device-to-device via Bluetooth until it reaches an active internet gateway.
