# Global Broadcast Network (GBN) - Prototype Workspace

**A decentralized, censorship-resistant video creation, publishing, and distribution platform - designed so truth can travel faster than it can be suppressed.**

> *"The internet treats censorship as damage and routes around it."*
> - John Gilmore

---

## Project Status

This repository is an **active prototype** (`gbn-proto`) for validating core architecture and security assumptions.

- Core Rust workspace and crate boundaries are in place
- Integration test scaffolding exists for metadata stripping, multipath reassembly, tamper detection, and end-to-end pipeline tests
- CLI orchestration commands are partially implemented (see `crates/proto-cli/src/main.rs`)
- Not production-ready; APIs and protocols are expected to evolve during prototyping

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

### 5) Scan the repo for leaked secrets

Use the repo-local scanner before commits or sharing the workspace:

```bash
python tools/scan_secrets.py .
```

Fail CI or pre-push checks when findings exist:

```bash
python tools/scan_secrets.py . --fail-on-findings
```

The scanner looks for likely AWS credentials, private key blocks, GitHub tokens, JWTs,
and suspicious secret-style assignments. To suppress an intentional test fixture or
documented example on a specific line, add `secretscan:allow` to that line.

---

## Vision & Mission

In many countries, a journalist who records police violence, corruption, or protests faces an impossible choice: **publish and be identified, or stay silent and stay safe**.

Existing options leave major gaps:
- **Mainstream platforms** can remove content centrally and log subpoenaable metadata.
- **Tor + generic file sharing** protects uploader routing but does not provide an integrated publisher trust + distribution pipeline.
- **VPNs** shift trust to the VPN operator.

The **Global Broadcast Network** aims to provide a complete, end-to-end pipeline - from capture to playback - such that no single point of failure can trivially identify creators or suppress distribution.

### Design Principles

| Principle | What It Means In Practice |
|---|---|
| **Privacy by Default** | End-to-end encryption and local metadata sanitization before transmission |
| **Resilience over Efficiency** | Erasure-coded distribution across geographically diverse nodes |
| **Legal Responsibility at the Edges** | Editorial/legal responsibility is with Publishers and Content Providers |
| **Adaptive to Adversaries** | Pluggable transport strategy evolves against censorship techniques |
| **Sovereign Updates** | Supply-chain hardening via reproducible builds and multi-party governance (see [GBN-SEC-007](../../docs/security/GBN-SEC-007-Software-Supply-Chain.md)) |

---

## How It Works

**The Root of Trust:** The user journey strictly begins prior to recording the video. The Creator must first establish cryptographic trust by scanning the Publisher's Public Key via a QR code (or by downloading a pre-seeded Sovereign Publisher App). Additionally to seed the network the Publisher must provide (or the Creator must aquire) a few exit relays, located outside the geofence, that can connect to the Publisher to bypass Publisher geofencing. This ensures the MCN encrypts data specifically for that Publisher and structurally prevents adversary traffic interception.

### Journey of a Video

```
  CREATOR                      RELAY NETWORK                       PUBLISHER
  (hostile jurisdiction)       (3-hop onion routing)               (trusted entity)

 +---------------------+                                       +---------------------+
 | 1. Record video     |       +========================+       | 5. Receive chunks   |
 | 2. Strip metadata   |       |  Path 1                |       |    (out-of-order)   |
 |    (GPS, device ID, |------>|  Guard > Middle > Exit |------>| 6. Decrypt each     |
 |     timestamps)     |       +========================+       | 7. Verify BLAKE3    |
 | 3. Chunk (1MB each) |       +========================+       | 8. Reassemble video |
 | 4. Encrypt chunks   |------>|  Path 2 (diff circuit) |------>| 9. Editorial review |
 |    (AES-256-GCM)    |       +=======================+        |10. Sign (Ed25519)   |
 |                     |       +========================+       |                     |
 |                     |------>|  Path 3 (diff circuit) |------>|                     |
 +---------------------+       +========================+       +----------+----------+
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
                          VIEWER        |
                        +----------------+
                        |
                        v
              +-------------------+
              | Discover content  |
              | via peer gossip   |
              |        |          |
              | Fetch 14 of 20    |
              | shards via BON    |
              |        |          |
              | Reconstruct and   |
              | play video        |
              +-------------------+
```


## Publisher Flow Packet Path implemented in Current Prototype

**Path/Return_Path**: Creator → Guard → Middle → Exit → Publisher

The path is created by the creator from its DHT which has been populated by the gossip network

**Onion build (Creator, innermost first):**

```
layer_pub  = seal(publisher_pub,  { next_hop: None,chunk_payload, chunk_id, chunk_hash, return_path, send_timestamp, total_chunks, chunk_index })
layer_exit = seal(exit_pub,       { next_hop: publisher_addr}) + layer_pub 
layer_mid  = seal(middle_pub,     { next_hop: exit_addr}) + layer_pub 
layer_grd  = seal(guard_pub,      { next_hop: middle_addr}) + layer_pub 
```

Creator sends `layer_grd` over TCP to Guard.

**Each relay (Guard / Middle / Exit):**
1. Read length-prefixed bytes from TCP
2. `open(own_priv)` → `{ next_hop}`
3. Connect to `next_hop`, write `layer_pub` as length-prefixed bytes
4. (No response needed for data forwarding)

**Publisher:**
1. `open(layer_pub)` → `{ next_hop: None,chunk_payload, chunk_id, chunk_hash, return_path, send_timestamp, total_chunks, chunk_index }`
2. Verify hash, store chunk
3. Build reverse-direction ACK (ChunkID, Receive Timestamp, Hash, ChunkIndex) onion using `return_path` → send back to Creator

**ACK return path**: Publisher → Exit → Middle → Guard → Creator
Creator must listen on an ACK port; return_path contains Creator's ack address.

---

### Gossip Network Design

Every node in the GBN relay network participates in a **PlumTree epidemic broadcast** protocol (implemented over libp2p request/response) plus a **direct validation / anti-entropy control path**. Discovery, direct liveness, and routing trust are tracked separately.

**Current message roles:**

| Message Type | Transport | Purpose |
|---|---|---|
| NodeAnnounce | PlumTree broadcast | Periodic epidemic self-announcement for eventual convergence |
| DirectNodeAnnounce | Direct request/response | Immediate self-announcement to a newly connected peer |
| DirectNodePropagate | Direct request/response | Sampled anti-entropy batch of freshest known nodes |
| DirectNodeProbe | Direct request/response | Probe used to validate a propagated-only node directly |
| DirectNodeProbeResponse | Direct request/response | Direct self-response that upgrades a node from propagated-only to directly seen |

**How PlumTree works:**

PlumTree separates peers into two sets per node:

- **Eager peers** receive full message payloads immediately.
- **Lazy peers** receive only IHave message-ID announcements and pull with IWant if they missed the payload.

This keeps redundant traffic low under normal conditions while still repairing missed deliveries. Peers are promoted and demoted between eager and lazy sets with Graft and Prune.

**Deduplication:** Each message carries a 32-byte MessageId (content hash). Every node tracks a sliding window of seen IDs; duplicate deliveries are dropped immediately.

**Rate limiting:** Each node enforces a token-bucket bandwidth budget on outbound gossip to prevent a single announcement storm from saturating the network under high churn.

**Propagation diagram:**

~~~text
  A relay node announces itself through PlumTree:
  NodeAnnounce { addr, pub_key, role }

                        +--------------+
                        |  Originator  |
                        +------+-------+
            +------------------+------------------+
     eager push           eager push           IHave only
     (full payload)       (full payload)     (message-ID)
            |                  |                   :
            v                  v                   :
     +------------+    +--------------+    +--------------+
     |  Seed Node |    |   Guard A    |    |   Guard B    |
     +-----+------+    +------+-------+    +------+-------+
           |                  |                   | IWant (not yet seen)
    eager  |  lazy        eager|                  v
           v  : : :>           v           +-----------------+
     +---------+  IHave  +----------+      | full payload    |
     | Creator |         |  Middle  |      | pulled on-demand|
     +---------+         +----+-----+      +-----------------+
                              | eager
                              v
                        +----------+
                        |   Exit   |
                        +----------+

  --- full payload pushed immediately to eager peers
  : : IHave (message-ID only); receiver sends IWant to pull if not yet seen

  PlumTree spreads discovery widely, but direct validation still decides whether
  a discovered node is trusted for routing or for propagating more DHT entries.
~~~

**How the DHT is populated now:**

1. A live node periodically broadcasts NodeAnnounce through PlumTree.
2. When two peers connect directly, they exchange DirectNodeAnnounce.
3. Every 10 seconds, a node sends a sampled DirectNodePropagate batch containing the freshest live entries from its local DHT to a sampled subset of neighbors.
4. A node learned only through propagation is queued for DirectNodeProbe.
5. Only a successful direct probe response populates last_direct_seen_ms.
6. `DirectNodePropagate` updates are only accepted from nodes whose `validation_state` is `complete`.

**How the Creator uses the gossip DHT:**

~~~text
GOSSIP DESCRIPTOR (advertised in DHT / signed metadata)
+----------------------+-----------------------------------------------+
| Field                | Type / Meaning                               |
+----------------------+-----------------------------------------------+
| identity_key         | [u8; 32]                                     |
|                      | Node identity public key                     |
+----------------------+-----------------------------------------------+
| address              | SocketAddr                                   |
|                      | Globally reachable IP:port for onion ingress |
+----------------------+-----------------------------------------------+
| subnet_tag           | String                                       |
|                      | Role tag (HostileSubnet / FreeSubnet / etc.) |
+----------------------+-----------------------------------------------+
| announce_ts_ms       | u64                                          |
|                      | Self-advertised timestamp (ms)               |
+----------------------+-----------------------------------------------+
| signature            | [u8; 64]                                     |
|                      | Signature over identity/address/subnet/time  |
+----------------------+-----------------------------------------------+
~~~

~~~text
LOCAL SEED STORE ENTRY (runtime DHT table used by Creator/relays)
+-------------------------+----------------------------------------------+
| Field                   | Type / Meaning                              |
+-------------------------+----------------------------------------------+
| addr                    | SocketAddr                                   |
|                         | Node onion ingress endpoint                  |
+-------------------------+----------------------------------------------+
| identity_pub            | [u8; 32]                                     |
|                         | Node Noise pubkey used for onion encryption  |
+-------------------------+----------------------------------------------+
| subnet_tag              | String                                       |
|                         | Role tag from gossip                         |
+-------------------------+----------------------------------------------+
| announce_ts_ms          | u64                                          |
|                         | Newest self-announced timestamp from node    |
+-------------------------+----------------------------------------------+
| last_direct_seen_ms     | Option<u64>                                  |
|                         | Last time this node was heard directly       |
+-------------------------+----------------------------------------------+
| last_propagated_seen_ms | Option<u64>                                  |
|                         | Last time this node was heard through        |
|                         | propagation                                  |
+-------------------------+----------------------------------------------+
| last_observed_ms        | u64                                          |
|                         | Most recent local observation of any kind    |
+-------------------------+----------------------------------------------+
| validation_state        | enum                                         |
|                         | propagated_only / unvalidated / direct /     |
|                         | complete / isolated                          |
+-------------------------+----------------------------------------------+
| validation_score        | u32                                          |
|                         | Routing confidence score                     |
+-------------------------+----------------------------------------------+
| last_seen_ms            | u64                                          |
|                         | Legacy compatibility field                   |
+-------------------------+----------------------------------------------+
~~~

~~~text
VALIDATION STATE MACHINE

propagated_only
  Learned indirectly through DHT propagation.
  Not trusted for routing.

unvalidated
  First direct sighting seeds validation_score to 10.
  Still not trusted for normal path construction.

direct
  At least one chunk path using this node produced a valid publisher ACK.
  The node is usable, but still in the preliminary scoring period.

complete
  validation_score > 20.
  Fully trusted for routing and for accepted DHT propagation.

isolated
  validation_score == 0.
  Entry stays in the DHT until stale cleanup, but is excluded from routing.
~~~

~~~text
VALIDATION SCORE RULES
+-------------------------------+--------------------------------------------+
| Event                         | Effect                                     |
+-------------------------------+--------------------------------------------+
| First direct sighting         | score := max(score, 10)                    |
| First direct sighting         | state := unvalidated                       |
| Successful ACKed chunk        | score += 1                                 |
| First ACK while unvalidated   | state := direct                            |
| Score exceeds 20              | state := complete                          |
| Failed routed chunk           | score -= 1                                 |
| Score reaches 0               | state := isolated                          |
+-------------------------------+--------------------------------------------+
~~~

~~~text
DumpDht Control Output (creator control plane response)
+---------------------+-------------------------------------------------------+
| Field               | Type / Meaning                                        |
+---------------------+-------------------------------------------------------+
| store               | Vec<RelayNode>                                        |
|                     | Full local seed-store table                           |
+---------------------+-------------------------------------------------------+
| kademlia_buckets    | Vec<String>                                           |
|                     | Hash bucket peer preimages from libp2p Kademlia view  |
+---------------------+-------------------------------------------------------+
~~~

When a Creator wants to send, it queries its local in-memory DHT and filters candidates by validation state:

- **Guard** - must be a validated HostileRelay or SeedRelay
- **Middle** - normally same validated pool, Guard excluded
- **Exit** - must be a validated FreeRelay
- **Publisher** - the known Publisher address

**Lazy validation of new nodes:**

- propagated_only nodes are queued for direct probe and are not used for routing.
- unvalidated nodes are not trusted for general routing and their inbound DHT propagation is ignored.
- During the next multi-chunk send, the Creator may place one unvalidated node only in the **middle** position of a canary path while keeping the same validated Guard and Exit as a sibling baseline path.
- If both chunks receive valid publisher ACKs, the candidate middle can gain score and promote into direct.
- If the baseline succeeds and the canary fails, only the canary middle is penalized.
- Once the node's score exceeds 20, it becomes complete and its propagated DHT updates are accepted.

The Creator still builds the onion circuit from local DHT state, but it now filters out isolated nodes and prefers nodes with stronger direct-validation evidence.

The Creator still builds the onion circuit from local DHT state, but it now filters out isolated nodes and prefers nodes with stronger direct-validation evidence.

---

### What each participant can observe

```text
Creator      -> Sees: local video + target Publisher key
               Sees full relay topology and Pub Keys

Guard relay  -> Sees: previous hop + next hop
               Cannot see: payload plaintext or final destination context

Middle relay -> Sees: adjacent hops only
               Cannot see: creator identity, publisher identity, or content plaintext

Exit relay   -> Sees: prior hop and destination endpoint
               Cannot see: origin creator identity or content plaintext

Publisher    -> Sees: decrypted submitted content
               Can see: full relay topology and Pub Keys back to creator for Ack message

Storage node -> Sees: encrypted shards by content-addressed ID
               Cannot see: plaintext media

Viewer       -> Sees: playable stream/content
               Cannot see: creator identity or full relay path
```

### Prototype components in this workspace

| Component | Purpose (prototype scope) | Primary use in this prototype |
|---|---|---|
| `gbn-protocol` | Shared wire types and serialization contracts for chunks, manifests, onion routing, DHT, and crypto payloads | Common dependency used across every service role |
| `mcn-sanitizer` | Media sanitization pipeline and FFmpeg-based metadata stripping | Creator-side preprocessing before chunking/upload |
| `mcn-chunker` | Chunking, hashing, and manifest-oriented segmentation helpers | Creator-side chunk generation and integrity bookkeeping |
| `mcn-crypto` | Publisher key generation, upload-session encryption, and Noise-based onion seal/open helpers | Creator and publisher cryptographic flow |
| `mcn-router-sim` | Gossip/DHT, relay control plane, telescopic onion routing, ACK relay path, and distributed trace metadata | Relay, creator, seed relay, and transport orchestration |
| `mpub-receiver` | Publisher-side onion terminal receive path, chunk acceptance, transport ACK generation, and session completion tracking | Publisher role runtime |
| `proto-cli` | The `gbn-proto` binary entrypoint that wires all crates together into runnable commands and service modes | Single executable used by the prototype containers and local CLI |

### Phase Prototype Image Mapping

Both prototype Dockerfiles currently compile the same binary:

```bash
cargo build --release --bin gbn-proto --features distributed-trace
```

That means both images currently link the full workspace transitively through `proto-cli`.

| Component | `gbn-relay` image | `gbn-publisher` image | Notes |
|---|---|---|---|
| `gbn-protocol` | Yes | Yes | Shared dependency of the single `gbn-proto` binary |
| `mcn-sanitizer` | Yes | Yes | Linked through `proto-cli`, even if not exercised by every runtime role |
| `mcn-chunker` | Yes | Yes | Linked through `proto-cli` |
| `mcn-crypto` | Yes | Yes | Linked through `proto-cli` |
| `mcn-router-sim` | Yes | Yes | Linked through `proto-cli` |
| `mpub-receiver` | Yes | Yes | Linked through `proto-cli` |
| `proto-cli` | Yes | Yes | Defines the `gbn-proto` binary built into both images |

### Current Phase Stack Runtime Usage

| Runtime role in `phase1-scale-stack.yaml` | Image currently used | Notes |
|---|---|---|
| `SeedRelayInstance` | `gbn-relay` | Static EC2 bootstrap relay; kept as the single seed relay for network bring-up |
| `HostileRelayService` | `gbn-relay` | ECS/Fargate relay tasks in hostile subnet |
| `FreeRelayService` | `gbn-relay` | ECS/Fargate relay tasks in free subnet |
| `CreatorService` | `gbn-relay` | ECS/Fargate creator role also runs the same `gbn-proto` binary image |
| `PublisherInstance` | `gbn-relay` | Current stack still launches publisher mode from the relay image |
| `gbn-publisher` ECR image | Built and published, but not wired into the current phase stack | `Dockerfile.publisher` exists, but `phase1-scale-stack.yaml` does not currently launch the publisher instance from `ContainerImagePublisher` |

---

## Repository Layout

```text
gbn-proto/
|-- Cargo.toml
|-- README.md
|-- crates/
|   |-- gbn-protocol/
|   |-- mcn-sanitizer/
|   |-- mcn-chunker/
|   |-- mcn-crypto/
|   |-- mcn-router-sim/
|   |-- mpub-receiver/
|   `-- proto-cli/
|-- infra/
|   |-- README-infra.md
|   |-- cloudformation/
|   `-- scripts/
|-- test-vectors/
|   `-- README.md
`-- tests/
    `-- integration/
        |-- test_metadata_stripping.rs
        |-- test_multipath_reassembly.rs
        |-- test_tamper_detection.rs
        `-- test_full_pipeline.rs
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

### Phase 1 - Media Creation Network & zero-trust routing
Plan: [`../../docs/prototyping/GBN-PROTO-001-Phase1-Media-Creation.md`](../../docs/prototyping/GBN-PROTO-001-Phase1-Media-Creation.md)

### Phase 2 - Publishing & distributed storage
Plan: [`../../docs/prototyping/GBN-PROTO-002-Phase2-Publishing-Storage.md`](../../docs/prototyping/GBN-PROTO-002-Phase2-Publishing-Storage.md)

### Phase 3 - Overlay broadcast network & playback
Plan: [`../../docs/prototyping/GBN-PROTO-003-Phase3-Broadcast-Playback.md`](../../docs/prototyping/GBN-PROTO-003-Phase3-Broadcast-Playback.md)

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

## AWS Prototype Scripts

The main AWS bring-up flow for the current phase prototype is:

1. Build the Rust binary and publish container images to ECR.
2. Deploy the CloudFormation stack and start the seed topology.
3. Expand to smoke or scale topology.
4. Use the relay control panel to inspect DHT state and run end-to-end path tests.

All of these scripts use the **AWS CLI** for their AWS operations. Before using them from WSL Ubuntu, make sure:

1. `aws` is installed and available in the WSL Ubuntu shell `PATH`
2. you have completed AWS authentication successfully in that same environment
3. the CLI can make authenticated calls for the target account and region

Minimum prerequisite check from WSL Ubuntu:

```bash
aws configure list
aws sts get-caller-identity
```

If those commands do not work, do not proceed with the deploy scripts yet. Fix AWS CLI installation and complete your AWS sign-in / credential setup first.

### Important scripts

| Script | What it does | Main infrastructure touched |
|---|---|---|
| [`build-and-push.sh`](/C:/Users/fahd_/OneDrive/Documents/Global%20Broadcast%20Network/prototype/gbn-proto/infra/scripts/build-and-push.sh) | Compiles `gbn-proto`, builds `gbn-relay` and `gbn-publisher` Docker images, tags them with `latest` and git SHA, and pushes both to ECR | Existing ECR repositories exposed by the CloudFormation stack |
| [`deploy-smoke-n5.sh`](/C:/Users/fahd_/OneDrive/Documents/Global%20Broadcast%20Network/prototype/gbn-proto/infra/scripts/deploy-smoke-n5.sh) | Thin wrapper around `deploy-scale-test.sh` that forces a 5-node smoke topology | Same phase stack, but runtime topology pinned to 2 hostile relays + 1 free relay + 1 creator + 1 static seed |
| [`deploy-scale-test.sh`](/C:/Users/fahd_/OneDrive/Documents/Global%20Broadcast%20Network/prototype/gbn-proto/infra/scripts/deploy-scale-test.sh) | Deploys the Phase 1 scale stack, generates publisher/seed keys, optionally auto-builds images if ECR is empty, restarts static nodes, scales ECS services, and optionally enables chaos churn | CloudFormation stack, ECS cluster/services, ECR repos, static EC2 seed/publisher nodes, chaos Lambda and EventBridge rule |
| [`relay-control-interactive.sh`](/C:/Users/fahd_/OneDrive/Documents/Global%20Broadcast%20Network/prototype/gbn-proto/infra/scripts/relay-control-interactive.sh) | Discovers live ECS and EC2 nodes and lets you run control-plane commands against them | ECS Exec against creator/relay tasks and SSM against seed/publisher EC2 nodes |

### What the deploy scripts create

The deploy flow targets the Phase 1 CloudFormation template and brings up the current prototype stack:

- ECR repositories for `gbn-relay` and `gbn-publisher`
- ECS cluster for the dynamic relay and creator tasks
- ECS services for:
  - hostile relays
  - free relays
  - creator
- Static EC2 instances for:
  - seed relay
  - publisher
- CloudWatch metrics and the scale/chaos control plane used by the test harness
- Chaos controller Lambda and scheduled EventBridge rule when chaos support is enabled in the stack

### 1. Build and push images

Run from WSL Ubuntu or another shell that has `cargo`, `docker`, and `aws` configured:

```bash
cd prototype/gbn-proto/infra/scripts
bash build-and-push.sh gbn-proto-phase1-scale-n100 us-east-1
```

What this does:

- resolves the stack outputs to find the relay and publisher ECR repositories
- compiles the release `gbn-proto` binary with `distributed-trace`
- builds both Docker images
- pushes both `latest` and git-SHA tags to ECR

If the stack does not yet expose ECR outputs, the script derives the ECR repository URIs from the AWS account ID and region.

### 2. Deploy a smoke topology

Use the smoke wrapper when you want a fast sanity check with a small network:

```bash
cd prototype/gbn-proto/infra/scripts
bash deploy-smoke-n5.sh gbn-proto-phase1-scale-n100 us-east-1
```

This enforces:

- `SMOKE_TOPOLOGY=1`
- 2 hostile ECS relays
- 1 free ECS relay
- 1 ECS creator
- 1 static EC2 seed relay
- 1 static EC2 publisher

The wrapper delegates to `deploy-scale-test.sh` but prevents accidental full-scale expansion.

### 3. Deploy the scale topology

For scale runs, use the scale deploy script directly:

```bash
cd prototype/gbn-proto/infra/scripts
bash deploy-scale-test.sh gbn-proto-phase1-scale-n100 100 us-east-1
```

Key behaviors:

- generates publisher keys if they do not already exist under `prototype/gbn-proto/`
- deploys the CloudFormation stack
- ensures relay images exist in ECR, and can auto-run `build-and-push.sh` if ECR is empty
- restarts the static EC2 seed and publisher nodes so they pull current images and come up with the expected networking mode
- scales ECS services into a seeded topology first
- waits for the seed relay container and initial ECS running-count gate
- expands from seed topology to full target unless `SMOKE_TOPOLOGY=1`

Default scale assumptions:

- `ScaleTarget=100` unless overridden
- `SEED_PERCENT=30`
- `RESTART_STATIC_NODES=1`
- `AUTO_BUILD_PUSH_IF_ECR_EMPTY=1`

### 4. Enable chaos during scale runs

`deploy-scale-test.sh` supports scheduled churn for hostile and free relays.

Example:

```bash
cd prototype/gbn-proto/infra/scripts
ENABLE_CHAOS=1 \
CHAOS_ENABLE_DELAY_SECONDS=180 \
CHAOS_HOSTILE_CHURN_RATE=0.4 \
CHAOS_FREE_CHURN_RATE=0.2 \
bash deploy-scale-test.sh gbn-proto-phase1-scale-n100 100 us-east-1
```

Important chaos knobs:

- `ENABLE_CHAOS=1`
  Enables the EventBridge rule after deploy
- `CHAOS_ENABLE_DELAY_SECONDS`
  Wait time before the rule is enabled
- `CHAOS_HOSTILE_CHURN_RATE`
  Fraction of hostile relay tasks churned by the chaos controller
- `CHAOS_FREE_CHURN_RATE`
  Fraction of free relay tasks churned by the chaos controller

### 5. Operate the deployed network with relay-control-interactive

After deployment, use the control panel to inspect the live topology and run tests:

```bash
cd prototype/gbn-proto/infra/scripts
bash relay-control-interactive.sh \
  gbn-proto-phase1-scale-n100-cluster \
  us-east-1 \
  gbn-proto-phase1-scale-n100
```

What it does:

- discovers live ECS tasks for:
  - `CreatorService`
  - `HostileRelayService`
  - `FreeRelayService`
- discovers the static EC2:
  - `SeedRelayInstance`
  - `PublisherInstance`
- connects through:
  - ECS Exec for ECS tasks
  - SSM for EC2 nodes

Important interactive commands:

| Command | Purpose |
|---|---|
| `DumpDht` | View each node's local DHT / seed-store contents |
| `DumpMetadata` | Dump packet metadata / trace ring buffer entries |
| `BroadcastSeed` | Force a seed-style gossip propagation event |
| `UnicastDHT` | Trigger direct DHT exchange toward a chosen target |
| `SendDummy` | Build a creator -> guard -> middle -> exit -> publisher path and send a dummy payload |
| `Refresh nodes` | Re-scan the current live ECS/EC2 inventory |
| `checkimages` | Verify image/runtime consistency on deployed nodes |

### Typical AWS prototype workflow

```bash
cd prototype/gbn-proto/infra/scripts

# build and publish images
bash build-and-push.sh gbn-proto-phase1-scale-n100 us-east-1

# deploy smoke or scale
bash deploy-smoke-n5.sh gbn-proto-phase1-scale-n100 us-east-1
# or
bash deploy-scale-test.sh gbn-proto-phase1-scale-n100 100 us-east-1

# inspect and test the live network
bash relay-control-interactive.sh \
  gbn-proto-phase1-scale-n100-cluster \
  us-east-1 \
  gbn-proto-phase1-scale-n100
```

For teardown after a run, use the matching infra teardown scripts under `prototype/gbn-proto/infra/scripts/`.

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
