# GBN-PROTO-003 — Prototyping Plan: Phase 3 — Broadcast Overlay Network & Video Playback

**Document ID:** GBN-PROTO-003  
**Phase:** 3 of 3  
**Status:** Draft  
**Last Updated:** 2026-04-07  
**Depends On:** Phase 1 (GBN-PROTO-001), Phase 2 (GBN-PROTO-002)  
**Related Docs:** [GBN-REQ-006](../requirements/GBN-REQ-006-Broadcast-Network.md), [GBN-ARCH-006](../architecture/GBN-ARCH-006-Broadcast-Network.md), [GBN-REQ-005](../requirements/GBN-REQ-005-Video-Playback-App.md), [GBN-ARCH-005](../architecture/GBN-ARCH-005-Video-Playback-App.md)

---

## 1. Phase Goal

**Prove that two devices on separate real networks can establish a censorship-resistant encrypted connection through a multi-hop onion relay chain using pluggable transports, and that a viewer can discover, stream, and play a video by fetching erasure-coded shards from distributed storage peers — all without exposing any participant's true IP address to any other participant.**

This is the integration phase. It combines the data pipeline from Phases 1 & 2 with the real network transport layer. For the first time, we test on actual networked machines (VPS instances or Docker containers with isolated network namespaces), not localhost.

---

## 2. Assumptions to Validate

| ID | Assumption | Risk if Wrong |
|---|---|---|
| A1 | The Noise_XX handshake (`snow` crate) can establish forward-secret sessions between two Rust processes on separate machines within 500ms | If too slow, circuit build time exceeds 2-second target |
| A2 | WebTunnel transport (TLS WebSocket on port 443) is indistinguishable from normal HTTPS traffic when analyzed by DPI tools (Wireshark, nDPI, Cisco Joy) | If DPI tools fingerprint it, the primary transport is broken |
| A3 | Three-hop onion routing works correctly: Guard peels layer 1, Middle peels layer 2, Exit peels layer 3, and the final plaintext reaches the destination | If onion layering is implemented incorrectly, the entire routing model fails |
| A4 | A Noise session can sustain high-throughput data transfer (≥10 Mbps) for streaming video shards through the circuit | If throughput is too low, video playback will buffer excessively |
| A5 | HyParView gossip protocol maintains a connected peer graph under 30% node churn within 60 seconds | If gossip fails under churn, content discovery breaks |
| A6 | NAT traversal (STUN + UDP hole punching) succeeds for at least 80% of residential NAT configurations | If NAT traversal fails for most users, TURN fallback becomes the primary path (expensive) |
| A7 | A viewer can begin video playback within 5 seconds of requesting content, fetching shards from peers via BON circuits | If startup latency exceeds 10 seconds, UX is unacceptable |
| A8 | IP renegotiation messages propagate through gossip within 30 seconds after a node's IP changes | If propagation is too slow, recently-moved nodes become unreachable |

---

## 3. Prototype Components

### 3.1 Project Structure (extends Phase 1 & 2 workspace)

```
gbn-proto/
├── crates/
│   ├── ... (Phase 1 & 2 crates)
│   ├── bon-transport-webtunnel/  # WebTunnel pluggable transport (TLS WebSocket)
│   ├── bon-transport-obfs4/      # obfs4 pluggable transport (randomized bytes)
│   ├── bon-noise/                # Noise_XX session manager
│   ├── bon-onion/                # Onion routing: layered encrypt/decrypt
│   ├── bon-circuit/              # Circuit manager: build, extend, maintain circuits
│   ├── bon-relay/                # Relay daemon: forward packets for other nodes
│   ├── bon-gossip/               # HyParView peer discovery
│   ├── bon-nat/                  # NAT traversal (ICE/STUN/TURN client)
│   ├── vpa-streamer/             # Video playback: fetch shards, RS decode, stream
│   └── proto-cli/                # Extended: full end-to-end demo
├── deploy/
│   ├── docker-compose.yml        # Multi-container testbed (3 relays, 5 storage, 2 clients)
│   ├── Dockerfile.relay          # BON relay node image
│   ├── Dockerfile.storage        # GDS storage node image
│   ├── Dockerfile.client         # Creator/Viewer client image
│   └── network-topology.sh       # Creates isolated Docker networks with simulated latency
├── tests/
│   ├── integration/
│   │   ├── test_noise_handshake.rs         # Noise_XX between two networked processes
│   │   ├── test_onion_routing.rs           # 3-hop onion: build circuit, send data, verify
│   │   ├── test_webtunnel_dpi.rs           # Capture traffic, analyze with nDPI
│   │   ├── test_circuit_under_churn.rs     # Kill middle relay during transfer
│   │   ├── test_gossip_convergence.rs      # HyParView with 50 nodes, 30% churn
│   │   ├── test_nat_traversal.rs           # STUN + hole punch between Docker containers
│   │   ├── test_ip_renegotiation.rs        # Change IP, verify gossip propagation
│   │   ├── test_full_e2e.rs                # Creator → MCN → Publisher → GDS → BON → Viewer
│   │   └── test_video_streaming.rs         # Shard fetch → RS decode → HLS playback
│   ├── security/
│   │   ├── test_relay_sees_no_plaintext.rs  # PCAP analysis on relay traffic
│   │   ├── test_guard_no_destination.rs     # Guard node logs show no Publisher address
│   │   ├── test_exit_no_source.rs           # Exit node logs show no Creator address
│   │   └── test_viewer_ip_hidden.rs         # Storage node logs show BON address, not viewer IP
│   └── benchmarks/
│       ├── bench_noise_throughput.rs
│       ├── bench_circuit_build.rs
│       └── bench_shard_streaming.rs
```

### 3.2 Component Details

#### `bon-transport-webtunnel`
- **Purpose:** Prove that GBN traffic over TLS WebSocket is indistinguishable from normal HTTPS
- **Implementation:**
  - Server: `tokio-tungstenite` WebSocket server behind `rustls` on port 443
  - Serves a legitimate static HTML page to non-BON clients (active probe defense)
  - BON clients initiate a WebSocket upgrade with a specific URI path containing a Noise handshake payload
  - All subsequent traffic is framed as WebSocket binary messages
- **Key test:** Capture 60 seconds of BON WebTunnel traffic and 60 seconds of real HTTPS video streaming (YouTube). Run both through `nDPI` and `Wireshark` protocol dissectors. WebTunnel traffic MUST be classified as "TLS/HTTPS" — not flagged as unknown or suspicious.

#### `bon-noise`
- **Purpose:** Validate Noise_XX mutual authentication and session encryption
- **Implementation:** Wrap the `snow` crate
  1. Initiator sends ephemeral public key
  2. Responder replies with ephemeral + static keys
  3. Initiator sends static key
  4. Session key derived; ChaCha20-Poly1305 for payload encryption
- **Key test:** Two processes on different Docker containers complete handshake < 200ms on loopback, < 500ms cross-region

#### `bon-onion`
- **Purpose:** Build and peel multi-layer onion encryption
- **Implementation:**
  1. Sender knows public keys of Guard (G), Middle (M), Exit (E)
  2. Onion build: `encrypt(encrypt(encrypt(DATA, key_E), key_M), key_G)`
  3. G peels outer → sees `{route_to_M, middle_payload}`
  4. M peels middle → sees `{route_to_E, inner_payload}`
  5. E peels inner → sees `{route_to_Dest, DATA}`
- **Key test:** 3-hop circuit, 1000 packets sent; ALL packets arrive correctly at destination; NO relay can reconstruct the original DATA

#### `bon-circuit`
- **Purpose:** Manage circuit lifecycle: build, extend, teardown, retry
- **Implementation:**
  - Maintains a pool of 3 pre-built circuits
  - On circuit failure: tear down, select new relays, rebuild within 2 seconds
  - Implements jurisdiction-aware guard selection (guard must be in different AS from client)
- **Key test:** Kill middle relay during active transfer of 100 chunks; circuit manager detects failure, builds new circuit, resumes transfer; zero chunks lost

#### `bon-gossip`
- **Purpose:** Validate HyParView maintains connected graph under churn
- **Implementation:**
  - Active view: 5 peers (directly connected)
  - Passive view: 30 peers (known, not connected)
  - Join/Leave/Shuffle messages per HyParView spec
  - Content manifest propagation via epidemic gossip
- **Key test:** 50-node network; kill 15 nodes (30%) simultaneously; remaining 35 nodes re-converge to a connected graph within 60 seconds; new content manifests propagate to all surviving nodes within 30 seconds

#### `vpa-streamer`
- **Purpose:** Prove that a viewer can stream video by fetching shards through BON circuits
- **Implementation:**
  1. Receive content manifest via gossip
  2. Verify Publisher signature
  3. For each chunk needed for playback: fetch k=14 shards via BON from GDS peers
  4. RS decode → decrypt with content key → buffer decoded video segment
  5. Feed decoded segments to an embedded HTTP server serving HLS playlist
  6. ExoPlayer (or `ffplay` for prototype) connects to localhost HLS stream
- **Key test:** Viewer begins playback within 5 seconds; no visible buffering during sustained playback of 100MB video

---

## 4. Test Plan

### 4.1 Correctness Tests

| Test ID | Test Name | Pass Criteria |
|---|---|---|
| T3.1 | **Noise_XX Handshake (same host)** | Two processes complete mutual authentication in < 100ms |
| T3.2 | **Noise_XX Handshake (cross-container)** | Two Docker containers complete handshake in < 500ms |
| T3.3 | **Onion Routing (3 hops, 1 packet)** | Single packet traverses G→M→E→Dest correctly |
| T3.4 | **Onion Routing (3 hops, 1000 packets)** | 1000 packets, all arrive; zero corruption |
| T3.5 | **Circuit Build Time** | From zero to fully established 3-hop circuit in < 2 seconds |
| T3.6 | **Circuit Rebuild on Failure** | Kill middle node; new circuit built and transfer resumed in < 5 seconds |
| T3.7 | **WebTunnel DPI Evasion** | `nDPI` classifies WebTunnel traffic as TLS/HTTPS, not "unknown" |
| T3.8 | **WebTunnel Active Probe Defense** | HTTP GET to WebTunnel port returns plausible HTML page; no BON leak |
| T3.9 | **HyParView Convergence** | 50 nodes; kill 30%; connected graph restores in < 60 seconds |
| T3.10 | **Gossip Manifest Propagation** | New manifest reaches all 50 nodes within 30 seconds |
| T3.11 | **IP Renegotiation Propagation** | Change a node's IP; all active peers update within 30 seconds |
| T3.12 | **NAT Traversal (symmetric NAT sim)** | Two nodes behind simulated symmetric NATs connect via TURN fallback |
| T3.13 | **Video Streaming Startup** | Viewer begins playback within 5 seconds of requesting content |
| T3.14 | **Video Streaming Sustained** | 100MB video plays without visible buffering at 720p |
| T3.15 | **Full E2E Pipeline** | Creator → sanitize → chunk → encrypt → 3-hop MCN relay → Publisher → re-chunk → RS encode → distribute 20 shards via BON → Viewer fetches 14 shards via separate BON circuits → RS decode → decrypt → play video. SHA-256 of played content matches original. |

### 4.2 Performance Benchmarks

| Benchmark | Target | Measurement |
|---|---|---|
| B3.1 | Noise session throughput (single circuit) | ≥ 10 Mbps sustained |
| B3.2 | Circuit build time | < 2 seconds |
| B3.3 | Shard fetch (14 shards, 4MB each, via BON) | < 30 seconds for 56MB |
| B3.4 | Gossip convergence after 30% churn | < 60 seconds |
| B3.5 | End-to-end latency (Creator upload → Viewer playback available) | < 10 minutes for 100MB video |
| B3.6 | WebTunnel overhead vs raw TCP | < 15% throughput penalty |

### 4.3 Security Validation Tests

| Test ID | Test Name | What It Proves |
|---|---|---|
| S3.1 | **Relay PCAP Analysis** | Capture all traffic at Middle relay with `tcpdump`. Analyze with `tshark`. Zero plaintext video bytes, zero identifiable headers, zero session IDs in plaintext. |
| S3.2 | **Guard Node Destination Blindness** | Guard relay logs show Creator container IP and Middle container IP. Guard NEVER sees Publisher IP or Exit IP. |
| S3.3 | **Exit Node Source Blindness** | Exit relay logs show Middle container IP and Publisher IP. Exit NEVER sees Creator IP or Guard IP. |
| S3.4 | **Viewer IP Hidden from Storage** | Storage node logs show BON relay IP, NOT the Viewer container's real IP. |
| S3.5 | **Forward Secrecy Verification** | Extract Guard relay's long-term Ed25519 key. Attempt to decrypt a previously captured PCAP of a Noise session. Decryption MUST fail (session keys were ephemeral). |
| S3.6 | **WebTunnel vs Real HTTPS Indistinguishability** | Feed WebTunnel PCAP and real HTTPS PCAP to a trained ML classifier. Classification accuracy MUST be < 60% (near-random). |
| S3.7 | **Active Probe Resistance** | Send standard HTTP GET, HEAD, OPTIONS, and malformed TLS ClientHello to WebTunnel port. All MUST receive plausible non-BON responses. |

---

## 5. Deployment Testbed

### 5.1 Docker Topology

```
┌────────────────────────────────────────────────────────────┐
│                    Docker Test Network                       │
│                                                              │
│  [Creator Client]──┐                                         │
│    172.20.1.10      │                                        │
│                     ├──[Guard Relay]──[Middle Relay]──[Exit]  │
│                     │   172.20.2.10    172.20.3.10   .4.10   │
│  [Viewer Client]───┘                                         │
│    172.20.1.20                                               │
│                                                              │
│  [Storage Node 1-5]     [Publisher]     [DHT Bootstrap]      │
│   172.20.5.10-14        172.20.6.10     172.20.7.10          │
│                                                              │
│  Network namespaces: each subnet is isolated                 │
│  tc netem: 50ms latency + 5ms jitter per link                │
└────────────────────────────────────────────────────────────┘
```

### 5.2 Simulated Network Conditions

| Scenario | Configuration |
|---|---|
| **Normal** | 50ms latency, 0% packet loss |
| **Degraded** | 200ms latency, 2% packet loss |
| **Hostile** | 500ms latency, 10% packet loss, 1% corruption |
| **Churn** | Random node restart every 30 seconds |
| **Firewall** | iptables rules blocking all non-443 traffic from Creator subnet |

---

## 6. Tech Stack Validation

| Technology | What We're Proving | Fallback if Fails |
|---|---|---|
| **snow** (Noise_XX) | Correct handshake, forward secrecy, throughput | `noise-protocol` crate or custom Noise implementation |
| **tokio-tungstenite** | WebSocket framing works for arbitrary binary; TLS via rustls | `tungstenite` (sync) + native-tls |
| **webrtc-rs** | STUN/ICE/TURN NAT traversal in pure Rust | `libnice` via FFI |
| **HyParView** | Custom implementation converges correctly under churn | Switch to SWIM or T-Man gossip protocol |
| **ExoPlayer / ffplay** | Can consume HLS stream from localhost for playback | `mpv` with custom HLS source |
| **Docker + tc netem** | Realistic network simulation | Real VPS testbed (more expensive, higher fidelity) |

---

## 7. Success Criteria

Phase 3 is **PASSED** when ALL of the following are true:

- [ ] All 15 correctness tests (T3.1–T3.15) pass
- [ ] All 7 security validation tests (S3.1–S3.7) pass
- [ ] Full E2E test (T3.15) passes: Creator uploads video, Viewer plays it back perfectly via BON
- [ ] WebTunnel traffic is undetectable by `nDPI` (classified as standard HTTPS)
- [ ] No relay in the onion chain can determine both Creator and Publisher identity
- [ ] Viewer's real IP is never exposed to any storage node or relay
- [ ] Circuit rebuild completes in < 5 seconds after relay failure
- [ ] Video playback begins within 5 seconds and sustains without buffering at 720p

## 8. Known Limitations of This Prototype

| Limitation | Why It's Acceptable |
|---|---|
| Docker network ≠ real internet with diverse AS paths | Commercial-scale testing requires real VPS deployment (post-prototype budget decision) |
| No mobile device testing (Android) | Desktop Rust binary validates the core protocol; Android JNI wrapper is an engineering task, not an architectural risk |
| No obfs4 transport (only WebTunnel) | WebTunnel is the primary transport; obfs4 can be added in parallel |
| HLS packaging is rudimentary (no adaptive bitrate) | ABR is a UX polish concern, not an architectural risk |
| No cover traffic | Cover traffic is an enhancement; the prototype proves the core anonymous routing |
| Single STUN/TURN server | Production will need geographically distributed servers; prototype validates the protocol |

---

## 9. Post-Prototype Decision Points

After all three phases pass, the following architectural decisions should be revisited:

| Decision | Input From Phase |
|---|---|
| Is the Chunk-then-Encrypt approach correct, or does RS coding require raw ciphertext? | Phase 1 T1.3, Phase 2 A7 |
| Is k=14, n=20 the right RS ratio, or does performance require adjustment? | Phase 2 B2.1, B2.2 |
| Is Noise_XX throughput sufficient for 1080p streaming, or do we need a lighter cipher? | Phase 3 B3.1 |
| Can HyParView scale to 10M+ nodes, or do we need a hierarchical gossip layer? | Phase 3 T3.9 |
| Is WebTunnel alone sufficient for DPI evasion, or is obfs4 required from Day 1? | Phase 3 S3.6 |
| What is the real-world NAT traversal success rate, and how much TURN infrastructure is needed? | Phase 3 T3.12 |
