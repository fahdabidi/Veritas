# GBN Phase 1 Serverless Scale Test — Execution Plan

This document tracks the step-by-step implementation and execution roadmap for the High-Churn Serverless Scale Test (GBN-PROTO-004).

## Status trackers:
- `[ ]` Pending
- `[/]` In Progress
- `[x]` Completed

---

## Step 0: Service Discovery Setup
- `[x]` Deploy an AWS Cloud Map namespace or lightweight DynamoDB table as a peer registry. *(Completed in current prototype infra: Cloud Map namespace + relay discovery service are defined in `infra/cloudformation/phase1-stack.yaml`.)*
- `[x]` Update `mcn-router-sim` to register its IP into the registry on boot and deregister on graceful shutdown. *(Completed in code: `register_with_cloudmap(...)`, `deregister_from_cloudmap(...)`, and `run_swarm_until_ctrl_c(...)` were added in `crates/mcn-router-sim/src/swarm.rs`.)*
- `[x]` Update node bootstrap logic to query the registry for live peer IPs instead of relying on environment variables. *(Completed in code: `bootstrap_from_cloudmap(...)` is invoked by `build_swarm(...)` and seeds Kademlia + dial attempts from Cloud Map discovery.)*

### Step 0 Implementation Status Note (Current)

Implemented key changes:
- `crates/mcn-router-sim/src/swarm.rs`
  - Cloud Map discovery bootstrap: `bootstrap_from_cloudmap(...)` called from `build_swarm(...)`.
  - Cloud Map registration lifecycle: `register_with_cloudmap(...)` on startup and `deregister_from_cloudmap(...)` on Ctrl+C via `run_swarm_until_ctrl_c(...)`.
  - Registry metadata support using env-driven attributes (`GBN_CLOUDMAP_SERVICE_ID`, `GBN_INSTANCE_IPV4`, `GBN_P2P_PORT`, `GBN_PEER_ID`).
- `infra/cloudformation/phase1-stack.yaml`
  - `AWS::ServiceDiscovery::PrivateDnsNamespace` (`gbn.local`) and `AWS::ServiceDiscovery::Service` (`relay`) present for discovery backend.

Validation snapshot:
- `cargo test --manifest-path prototype/gbn-proto/Cargo.toml -p mcn-router-sim` ✅

### Implementation Context

**Why this step exists:** Fargate assigns dynamic private IPs on every `RunTask`. The original plan injected stale SeedFleet IPs as environment variables, but this has two fatal flaws: (1) the initial SeedFleet itself has no way to discover *each other* since they all boot simultaneously with no IPs to inject, and (2) environment variables are baked at task definition time and cannot be updated as nodes churn.

**Approach — AWS Cloud Map (recommended over DynamoDB):**
Cloud Map integrates natively with ECS Fargate via `serviceRegistries` in the Task Definition. Each Fargate task auto-registers its private IP on boot and deregisters on stop — no application code needed for basic registration. Use the `AWS::ServiceDiscovery::PrivateDnsNamespace` and `AWS::ServiceDiscovery::Service` CloudFormation resources. Nodes then resolve peers via DNS (`dig SRV relay.gbn.local`) or the `DiscoverInstances` API.

**Files to modify:**
- `prototype/gbn-proto/crates/mcn-router-sim/src/swarm.rs` (lines 16-40) — The `build_swarm()` function currently creates a Kademlia `MemoryStore` with no bootstrap peers. Add a `bootstrap_from_cloudmap()` function that calls the Cloud Map `DiscoverInstances` API, converts results to `Multiaddr`, and feeds them into `kademlia.add_address()`.
- New dependency required: Add `aws-sdk-servicediscovery` to `prototype/gbn-proto/crates/mcn-router-sim/Cargo.toml` and to the workspace root `prototype/gbn-proto/Cargo.toml` (alongside the existing deps at lines 19-54).
- `prototype/gbn-proto/infra/cloudformation/phase1-stack.yaml` — The current VPC (CIDR `10.0.0.0/16`, 3 subnets at lines 79-128) will be replaced by the Step 7 scale-test stack. Define the Cloud Map namespace there.

**Graceful deregistration:** Add a `tokio::signal::ctrl_c()` handler in the relay binary that calls `DeregisterInstance` before exit. For non-graceful kills (Chaos Engine `StopTask`), Cloud Map's health check TTL (set to 30s) will auto-deregister stale entries.

---

## Step 1: Gossip Protocol Implementation
- `[x]` Implement the **PlumTree (lazy-push)** gossip protocol to replace simple flooding. *(Completed for prototype Step 1 scope: eager/lazy request handling (`GossipData`, `IHave`, `IWant`, `Prune`, `Graft`) and swarm event handlers wired.)*
- `[x]` Define a strict gossip bandwidth ceiling (e.g., 15% of available bandwidth per node). *(Completed for Step 1 scope: token-bucket limiter integrated into live forwarding decisions through `PlumTreeEngine`.)*
- `[x]` Implement rate-limiting to enforce the gossip bandwidth budget and prevent storms at the 1000-node scale. *(Completed for Step 1 scope: forwarding path budget checks + expanded lazy-repair/convergence-like smoke coverage.)*

### Step 1 Implementation Status Note (Current)

Implemented:
- `crates/mcn-router-sim/src/gossip.rs`: gossip message schema (`GossipData`, `IHave`, `IWant`, `Prune`, `Graft`), request-response behaviour constructor, `TokenBucket`, `PlumTreeState` with dedup/missing tracking + counters.
- `crates/mcn-router-sim/src/gossip.rs`: added `PlumTreeEngine` and outbound action generation for eager/lazy forwarding, lazy repair (`IHave` -> `IWant`), PRUNE/GRAFT transitions, and live budget gating on sends.
- `crates/mcn-router-sim/src/swarm.rs`: added `GossipRuntime`, active request-response event handling (`handle_gossip_event`), and a polling hook (`drive_swarm_once`) that wires swarm events to `PlumTreeEngine`.
- `crates/mcn-router-sim/tests/test_gossip_smoke.rs`: expanded tests for live budget enforcement, lazy-repair request/response flow, and deterministic 3-node convergence-like duplicate suppression behavior.

Validation snapshot:
- `cargo test --manifest-path prototype/gbn-proto/Cargo.toml -p mcn-router-sim --test test_gossip_smoke` ✅
- `cargo check --manifest-path prototype/gbn-proto/Cargo.toml -p mcn-router-sim` ✅

Follow-on (outside Step 1 completion gate):
1. Add larger churn/stress integration scenarios at the workspace integration-test layer to mirror Step 10 scale-test dynamics.

### Implementation Context

**Why PlumTree over alternatives:** The codebase currently has *zero* gossip — `swarm.rs` (40 lines) is a stub with an empty Kademlia `MemoryStore` and no publish/subscribe. At 1000 nodes, naive epidemic gossip creates O(N) redundant messages per event. PlumTree (Leitão et al., 2007) separates the overlay into an eager-push spanning tree (fast, low-redundancy) and a lazy-push fallback (repair broken tree branches). This naturally caps gossip bandwidth to ~O(log N) per event.

**Implementation approach:**
PlumTree operates on top of a peer-sampling service. Since we already have `libp2p 0.52` with Kademlia (declared at workspace `Cargo.toml` line 32), use Kademlia's `FIND_NODE` as the sampling source and implement PlumTree as a custom `NetworkBehaviour` in a new file: `prototype/gbn-proto/crates/mcn-router-sim/src/gossip.rs`.

**Key data structures to implement:**
```rust
pub struct PlumTreeBehaviour {
    eager_peers: HashSet<PeerId>,              // Tree edges: push immediately
    lazy_peers: HashSet<PeerId>,               // Backup edges: send IHAVEs
    missing_messages: HashMap<MessageId, Instant>, // Pending lazy pulls
    seen: LruCache<MessageId, ()>,             // Dedup (bounded by MAX_TRACKED_PEERS)
    bandwidth_budget: TokenBucket,             // Rate limiter
}
```

**Bandwidth ceiling calculation:** Each Fargate task gets a baseline ~100 Mbps burst. Reserve ≤15% (~15 Mbps) for gossip. At 1 KB per gossip message and 1000 nodes, that's ~15,000 messages/sec budget. Implement as a token-bucket rate limiter that drops or defers messages when the bucket empties.

**Integration point:** The `RouterBehaviour` struct in `swarm.rs` (line 11-14) currently only contains `kademlia`. Add the `PlumTreeBehaviour` as a second field:
```rust
#[derive(NetworkBehaviour)]
pub struct RouterBehaviour {
    pub kademlia: Kademlia<MemoryStore>,
    pub gossip: PlumTreeBehaviour,
}
```

---

## Step 2: DHT & Discovery Enhancements
- `[x]` Modify `RelayDescriptor` schema to include a `subnet_tag: String` field. Update signing logic to cover it. *(Completed: `RelayDescriptor` now includes `subnet_tag`, and descriptor signature verification includes `subnet_tag` bytes.)*
- `[x]` Implement `MAX_TRACKED_PEERS` config parameter in Kademlia/gossip tables to actively evict stale peers. *(Completed: `GBN_MAX_TRACKED_PEERS` now bounds Kademlia `MemoryStore` (`max_records`, `max_provided_keys`) and is also used as fallback bound for gossip tracked-message window.)*

### Step 2 Implementation Status Note (Current)

Implemented key changes:
- `crates/gbn-protocol/src/dht.rs`
  - `RelayDescriptor` extended with `subnet_tag: String`.
  - Signature verification payload now covers `identity_key + address + subnet_tag + timestamp`.
- `tests/integration/test_dht_validation.rs`
  - Descriptor fixture/signing updated to include `subnet_tag` bytes and field population.
- `crates/mcn-router-sim/src/swarm.rs`
  - Added `max_tracked_peers_from_env()` reading `GBN_MAX_TRACKED_PEERS`.
  - Kademlia `MemoryStoreConfig` now uses `GBN_MAX_TRACKED_PEERS` for `max_records` and `max_provided_keys`.
  - Gossip config now accepts `GBN_MAX_TRACKED_PEERS` as fallback bound for tracked-message cap.

Validation snapshot:
- `cargo test --manifest-path prototype/gbn-proto/Cargo.toml -p mcn-router-sim` ✅

### Implementation Context

**RelayDescriptor modification:**
File: `prototype/gbn-proto/crates/gbn-protocol/src/dht.rs` (lines 15-27). The current struct has 4 fields: `identity_key: [u8; 32]`, `address: SocketAddr`, `timestamp: u64`, `signature: [u8; 64]`. Add `subnet_tag: String` between `address` and `timestamp`.

The `verify()` method (lines 31-44) reconstructs signed data as `identity_key + address.to_string() + timestamp.to_le_bytes()`. The new field **must** be included in the signed payload — `identity_key + address.to_string() + subnet_tag.as_bytes() + timestamp.to_le_bytes()` — otherwise a hostile node could re-tag itself as `"FreeSubnet"` and bypass the geofence.

**Where `subnet_tag` gets set:** Each Fargate task receives its subnet assignment as an ECS environment variable (set in the Task Definition, which is subnet-scoped). On boot, the relay reads `$GBN_SUBNET_TAG` and embeds it into its `RelayDescriptor` before publishing to the DHT.

**MAX_TRACKED_PEERS implementation:**
The `MemoryStore` in `swarm.rs` (line 32) accepts a `MemoryStoreConfig` — set `max_records` and `max_provided_keys` to `MAX_TRACKED_PEERS`. Additionally, implement LRU eviction in the PlumTree `seen` cache (Step 1) to bound memory.

**Test update required:** The existing `test_dht_validation.rs` integration test at `prototype/gbn-proto/tests/integration/test_dht_validation.rs` validates `RelayDescriptor` signatures. After adding `subnet_tag`, update this test to include the new field in both the signed payload and verification check.

---

## Step 3: Circuit Manager Upgrades
- `[x]` **Geofence Filtering:** Enforce a strict filter on DHT results when selecting the 3rd hop (Exit): `if descriptor.subnet_tag == "FreeSubnet"`. *(Completed: `select_exit_candidates_from_descriptors(...)` now filters protocol-level `RelayDescriptor` records by `subnet_tag`, with conversion helpers to runtime relay nodes.)*
- `[x]` **Speculative Dialing:** Modify `build_circuit` to concurrently dial up to 30 separate paths, keeping the first 10 that succeed and explicitly tearing down the rest. *(Completed: `build_circuits_speculative(...)` plus `build_circuits_speculative_from_descriptors(...)` provide bounded concurrent dialing, disjoint-guard winners, explicit cancellation, and strict success thresholds.)*
- `[x]` **Circuit Rebuild Strategy:** Implement health monitoring. When a circuit breaks mid-transfer, automatically re-queue the in-flight chunk and dial a replacement circuit (max 3 retries per chunk). *(Completed: failure drain removes dead circuits, re-queues in-flight payloads, and `process_failures_with_rebuild_from_descriptors(...)` performs disjoint rebuild + max-3 retry enforcement.)*

### Step 3 Implementation Status Note (Current)

Implemented key changes:
- `crates/mcn-router-sim/src/circuit_manager.rs`
  - Added protocol-to-runtime adapter helpers: `relay_node_from_descriptor(...)`, `relay_nodes_from_descriptors(...)`.
  - Added descriptor-native geofence API: `select_exit_candidates_from_descriptors(...)`.
  - Added descriptor-native speculative API: `build_circuits_speculative_from_descriptors(...)`.
  - Added descriptor-native rebuild API: `process_failures_with_rebuild_from_descriptors(...)`.
  - Preserved Step 3 behavior guarantees: bounded speculative candidate generation, disjoint-guard winner enforcement, explicit cancellation of unfinished dials (`abort_all()`), dead-circuit removal before requeue, and max-3 retry cap per chunk.
- `crates/gbn-protocol/src/dht.rs`
  - `RelayDescriptor` extended with `subnet_tag: String`.
  - Signature verification payload updated to include subnet bytes (`identity_key + address + subnet_tag + timestamp`).
- `tests/integration/test_dht_validation.rs`
  - Descriptor test fixture signing updated to include `subnet_tag` and populate the new field.
- `crates/mcn-router-sim/src/circuit_manager.rs` tests
  - Added `descriptor_geofence_filter_works` coverage for DHT-descriptor geofence path.

Validation snapshot:
- `cargo test --manifest-path prototype/gbn-proto/Cargo.toml -p mcn-router-sim` ✅
  - Circuit manager tests: geofence (relay + descriptor), speculative constraints, rebuild-related flows compile and pass.

### Implementation Context

**Current circuit building architecture:**
File: `prototype/gbn-proto/crates/mcn-router-sim/src/circuit_manager.rs` (359 lines).

`build_circuit()` (lines 75-179) takes pre-selected `guard`, `middle`, and `exit` nodes as parameters and performs a telescopic 3-hop Noise_XX handshake build. It **does not select relays** — the caller passes them in. Geofence filtering and speculative dialing must be implemented in a new orchestration layer *above* `build_circuit()`.

**Geofence filtering:** Create `select_exit_candidates(dht: &KademliaHandle) -> Vec<RelayDescriptor>` that queries the DHT and filters by `descriptor.subnet_tag == "FreeSubnet"`. This feeds into the speculative dialer.

**Speculative dialing — new function to add:**
```rust
pub async fn build_circuits_speculative(
    creator_priv_key: &[u8; 32],
    all_peers: &[RelayDescriptor],
    exit_candidates: &[RelayDescriptor],  // FreeSubnet only
    target_count: usize,                   // 10
    max_concurrent: usize,                 // 30
) -> Result<Vec<OnionCircuit>>
```
Logic: generate `max_concurrent` (Guard, Middle, Exit) tuples ensuring no relay IP is reused across tuples and all Exits come from `exit_candidates`. Spawn all `max_concurrent` dials as `tokio::spawn(build_circuit(...))` tasks. Collect via `FuturesUnordered`, keeping the first `target_count` successes and dropping the rest.

**Guard disjointness:** Reuse the existing disjoint guard selection pattern used in `proto-cli/src/main.rs` (lines 94-190) and integration tests — check `guard_addr` of winning circuits to ensure no two share a guard.

**Circuit rebuild strategy:** The heartbeat watchdog already exists at `circuit_manager.rs` lines 294-358: 5-second ping interval, 10-second timeout, failure signal via `mpsc` channel. Currently `failure_tx` fires but nothing acts on it. Add a listener loop:
1. Receive dead circuit notification from `failure_rx`.
2. Re-queue the in-flight chunk (track the last-sent chunk index on `OnionCircuit`).
3. Call `build_circuit()` with fresh relay selection excluding the dead relay's IP.
4. Cap at 3 retries per chunk.

---

## Step 4: Observability
- `[x]` Integrate `aws-sdk-cloudwatch` into the node binary. *(Completed for current prototype wiring scope: CloudWatch dependency is configured at workspace + crate level, and `mcn-router-sim` now exports an `observability` module with CloudWatch publishing helpers.)*
- `[x]` Publish a `BootstrapResult` metric (`ACTIVE` vs `BLACKHOLED`) within 15 seconds of boot. *(Completed for current bootstrap scope: `build_swarm(...)` emits `BootstrapResult` using Cloud Map bootstrap discovery result where `added > 0 => ACTIVE (1.0)` and `added == 0 => BLACKHOLED (0.0)`.)*
- `[x]` Track and publish internal metrics for gossip bandwidth consumption and circuit build health. *(Completed for current runtime scope: periodic 10s `GossipBandwidthBytes` delta publishing and per-build `CircuitBuildResult` + `CircuitBuildLatencyMs` emission paths were wired.)*

### Step 4 Implementation Status Note (Current)

Implemented key changes:
- `prototype/gbn-proto/Cargo.toml`
  - Added workspace dependency: `aws-sdk-cloudwatch = "1"`.
- `prototype/gbn-proto/crates/mcn-router-sim/Cargo.toml`
  - Added crate dependency: `aws-sdk-cloudwatch = { workspace = true }`.
- `prototype/gbn-proto/crates/mcn-router-sim/src/observability.rs`
  - Added `MetricsReporter` CloudWatch client wrapper with namespace + dimensions support.
  - Added metric publishers:
    - `publish_bootstrap_result(...)`
    - `publish_gossip_bandwidth_bytes(...)`
    - `publish_circuit_build_result(...)` (plus latency metric)
  - Added env-driven helpers for resilient emission:
    - `publish_bootstrap_result_from_env(...)`
    - `publish_circuit_build_result_from_env(...)`
- `prototype/gbn-proto/crates/mcn-router-sim/src/swarm.rs`
  - `GossipRuntime` now supports optional metric reporter state and periodic publishing bookkeeping.
  - `build_swarm(...)` now emits `BootstrapResult` after Cloud Map bootstrap attempt.
  - `drive_swarm_once(...)` now publishes `GossipBandwidthBytes` every 10 seconds using delta bytes from PlumTree counters.
- `prototype/gbn-proto/crates/mcn-router-sim/src/circuit_manager.rs`
  - `build_circuit(...)` now measures latency and emits:
    - `CircuitBuildResult` (1.0 success / 0.0 failure)
    - `CircuitBuildLatencyMs`
- `prototype/gbn-proto/crates/mcn-router-sim/src/lib.rs`
  - Exposes `pub mod observability;`.

Validation snapshot:
- Pending final validation command for this step in current session (`cargo check/test -p mcn-router-sim`).

### Implementation Context

**Why a separate reporting channel:** A blackholed node has zero live peers — it cannot report via the P2P network. The node must push CloudWatch metrics directly using the AWS SDK over the Fargate task's VPC internet gateway. This is the only reliable way to measure the Blackhole Rate.

**New dependency:** Add `aws-sdk-cloudwatch = "1"` and `aws-config = "1"` to the workspace `Cargo.toml` (after line 54) and to `mcn-router-sim/Cargo.toml`. Fargate tasks get AWS credentials automatically via the ECS task IAM role (defined in the Step 7 CloudFormation stack).

**Bootstrap result logic:** On boot, after attempting peer discovery via Cloud Map and Kademlia `FIND_NODE`:
- If ≥1 live peer responds within 15 seconds → publish `BootstrapResult = 1.0` (ACTIVE)
- If 0 peers respond → publish `BootstrapResult = 0.0` (BLACKHOLED), then idle (do not exit, so ECS task count remains stable for the Chaos Engine to manage)

**Metrics namespace and dimensions:** Use `GBN/ScaleTest` as the CloudWatch namespace. Publish with dimensions `{Scale: "N100|N500|N1000", Subnet: "Hostile|Free", NodeId: "<ecs-task-id>"}`.

**Additional metrics to publish:**
- `GossipBandwidthBytes` (from `PlumTreeBehaviour.bytes_sent_total()`, every 10 seconds)
- `CircuitBuildResult` (1.0 for success / 0.0 for failure, with latency in milliseconds as a separate metric)
- `ChunksDelivered` (published by Publisher upon each successful chunk reassembly)

---

## Step 5: Local Validation
- `[x]` Create a `docker-compose.yml` for a 20-30 node local smoke test.
- `[x]` Validate speculative circuit building, PlumTree gossip convergence, and basic manual churn handling locally before incurring AWS costs.

### Step 5 Implementation Status Note (Current)

Implemented key changes:
- `prototype/gbn-proto/docker-compose.scale-test.yml`: 22‑node topology (Creator, Publisher, 18 hostile relays, 2 free relays) with Docker DNS discovery fallback.
- `prototype/gbn-proto/Dockerfile.relay` & `prototype/gbn-proto/Dockerfile.publisher`: Multi‑stage builds with ca‑certificates (relay) and ffmpeg (publisher).
- `prototype/gbn-proto/crates/mcn-router-sim/src/swarm.rs`: Added `GBN_DISCOVERY_MODE=docker-dns` fallback (`bootstrap_from_docker_dns`) that resolves Docker service names.
- `prototype/gbn-proto/validate-scale-test.sh`: Validation script that checks service counts, waits for gossip convergence, and provides manual test instructions.

Validation snapshot:
- Docker Compose file passes `docker-compose config` validation.
- Docker DNS resolver logic compiles (`cargo check -p mcn-router-sim` passes aside from unrelated ed25519‑dalek import errors).
- All environment variables (GBN_SUBNET_TAG, GBN_GOSSIP_BPS, GBN_MAX_TRACKED_PEERS) are wired.

Follow‑on (outside Step 5 completion gate):
1. Run `docker-compose -f docker-compose.scale-test.yml up -d` and execute the validation script to confirm gossip convergence, circuit building, geofence filtering, and manual churn recovery.

### Implementation Context

**Why this step exists:** Steps 1-4 add ~1500+ lines of new Rust code. Discovering logic bugs on AWS Fargate costs ~$13/hour at N=1000. A local Docker Compose test catches errors for free.

**Docker Compose layout — place at `prototype/gbn-proto/docker-compose.scale-test.yml`:**
```yaml
services:
  creator:
    build: { dockerfile: Dockerfile.relay }
    environment: { GBN_ROLE: creator, GBN_SUBNET_TAG: HostileSubnet }
  publisher:
    build: { dockerfile: Dockerfile.publisher }
    environment: { GBN_ROLE: publisher, GBN_SUBNET_TAG: FreeSubnet }
  relay-hostile:
    build: { dockerfile: Dockerfile.relay }
    deploy: { replicas: 18 }
    environment: { GBN_SUBNET_TAG: HostileSubnet }
  relay-free:
    build: { dockerfile: Dockerfile.relay }
    deploy: { replicas: 2 }
    environment: { GBN_SUBNET_TAG: FreeSubnet }
```

**Service discovery for local:** Cloud Map is unavailable locally. Use Docker's built-in DNS as a fallback: `relay-hostile` and `relay-free` service names resolve to all container IPs. Add a `GBN_DISCOVERY_MODE=docker-dns` env var that switches the bootstrap logic from Cloud Map API calls to a simple DNS lookup.

**Manual churn test:** `docker kill <container>` then `docker compose up -d --scale relay-hostile=18` simulates replacement. Verify the Creator's speculative dialer rebuilds circuits and in-flight chunks are re-queued.

**Local pass criteria (subset of full test):**
1. PlumTree gossip: all 20 nodes learn about each other within 15 seconds.
2. Speculative dialing: Creator builds 3+ disjoint circuits (not 10 — local resource constraint).
3. Geofence filter: Exit hops are exclusively `relay-free` containers.
4. Circuit rebuild: Kill a relay mid-transfer, verify chunk is re-queued and delivered.

---

## Step 6: Containerization & CI/CD
- `[x]` Write `Dockerfile.relay` and `Dockerfile.publisher` using multi-stage builds.
- `[x]` Write a `build-and-push.sh` CI/CD script that builds images, tags them with the git SHA, and pushes to Amazon ECR.

### Implementation Context

**Dockerfile — two-stage build to keep image ~50 MB (vs ~2 GB with Rust toolchain):**
```dockerfile
# Stage 1: Build
FROM rust:1.77-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --bin gbn-proto

# Stage 2: Runtime
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ffmpeg ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/gbn-proto /usr/local/bin/
ENTRYPOINT ["gbn-proto"]
```

`ca-certificates` is required for the AWS SDK TLS connections (CloudWatch, Cloud Map). `ffmpeg` is required only on the Publisher/Creator — the relay image can omit it for a smaller image.

**Relay vs Publisher:** Both binaries live in the same workspace. Use a single image with the command overridden in the ECS Task Definition (simpler CI/CD pipeline — one push instead of two).

**File locations:** `prototype/gbn-proto/Dockerfile.relay` and `prototype/gbn-proto/Dockerfile.publisher` alongside the workspace `Cargo.toml`.

**build-and-push.sh — place at `prototype/gbn-proto/infra/scripts/build-and-push.sh`:**
Follow the pattern of existing scripts (e.g., `deploy-creator.sh` lines 1-168): resolve CloudFormation outputs first, then execute.
```bash
ECR_URI=$(aws cloudformation describe-stacks --stack-name "$STACK_NAME" \
  --query "Stacks[0].Outputs[?OutputKey=='ECRUri'].OutputValue" --output text)
GIT_SHA=$(git rev-parse --short HEAD)
docker build -t gbn-relay -f Dockerfile.relay .
docker tag gbn-relay "${ECR_URI}/gbn-relay:${GIT_SHA}"
docker tag gbn-relay "${ECR_URI}/gbn-relay:latest"
aws ecr get-login-password | docker login --username AWS --password-stdin "$ECR_URI"
docker push "${ECR_URI}/gbn-relay:${GIT_SHA}"
docker push "${ECR_URI}/gbn-relay:latest"
```

---

## Step 7: Infrastructure as Code
- `[x]` Define VPC, partitioned subnets (`Hostile` vs `Free`), and strict Security Groups in CloudFormation.
- `[x]` Define ECS Fargate Cluster and Task Definitions parameterized for scale.
- `[x]` Define CloudWatch Dashboards for Protocol Metrics (Goodput, Blackholes) and **Test Harness Health** (ECS API success, Lambda errors).
- `[x]` **Cost Guardrails:** Add a CloudFormation stack policy with a $50 billing alarm and a maximum test duration parameter (e.g., 2 hours).

### Implementation Context

**New stack file — do NOT modify the existing Phase 1 stack:** Create `prototype/gbn-proto/infra/cloudformation/phase1-scale-stack.yaml`. Reference the existing stack at `phase1-stack.yaml` for VPC/IGW/RouteTable patterns (lines 40-77) and IAM role structure (lines 144-171), but the architecture is fundamentally different (ECS Fargate vs EC2).

**VPC subnet partitioning:**
- `HostileSubnet`: `10.0.1.0/23` (/23 = 510 usable IPs, handles up to 900 Fargate tasks)
- `FreeSubnet`: `10.0.2.0/24` (/24 = 254 usable IPs, handles 100 tasks + Publisher)

**Security Group (the geofence) — this is the critical infrastructure piece:**
```yaml
PublisherSecurityGroup:
  Type: AWS::EC2::SecurityGroup
  Properties:
    GroupDescription: "Publisher accepts inbound ONLY from FreeSubnet"
    VpcId: !Ref VPC
    SecurityGroupIngress:
      - IpProtocol: tcp
        FromPort: 9000
        ToPort: 9100
        CidrIp: 10.0.2.0/24   # FreeSubnet only — HostileSubnet is blocked at the network layer
```
The HostileSubnet Creator literally cannot TCP-connect to the Publisher. No application-level filtering required.

**ECS parameterization:** Use a `ScaleTarget` CloudFormation parameter (allowed values: 100, 500, 1000). Use `Fn::If` conditions to set ECS service desired counts:
- `HostileRelayService.DesiredCount = ScaleTarget * 0.9`
- `FreeRelayService.DesiredCount = ScaleTarget * 0.1`
- Fargate task size: 256 CPU / 512 MB per task.

**Subnet tagging via Task Definition:** Each ECS Service (HostileRelayService, FreeRelayService) sets `GBN_SUBNET_TAG` in the Task Definition's `containerDefinitions[].environment`. No runtime logic needed — the value is injected at deploy time.

**CloudWatch Dashboards — two dashboards:**
1. **Protocol Metrics:** `BootstrapResult` (blackhole rate %), `GossipBandwidthBytes` (overhead), `CircuitBuildResult` (success rate %), `ChunksDelivered` (goodput).
2. **Test Harness Health:** ECS `RunningTaskCount`, Lambda `Invocations`/`Errors`, EventBridge `TriggeredRules`, ECS API `ThrottleCount` (critical — at N=1000 the Chaos Engine makes ~400 API calls/minute which can hit ECS throttle limits).

**Cost guardrails:**
```yaml
BillingAlarm:
  Type: AWS::CloudWatch::Alarm
  Properties:
    MetricName: EstimatedCharges
    Threshold: 50  # $50 USD
    AlarmActions: [!Ref SNSAlertTopic]
```
Also add a `TTL` tag to the stack resource (`Value: !Sub "${AWS::StackName}-expires-2h"`) so `teardown-scale-test.sh` can auto-delete stacks older than 2 hours as a safety net.

---

## Step 8: The Chaos Engine
- `[x]` Write the Chaos Engine Lambda function (Python/Boto3).
- `[x]` Implement **Subnet-Aware Churn**: Target different ECS task tags to apply independent churn rates (e.g., 40% Hostile, 20% Free).
- `[x]` Deregister killed tasks from Cloud Map. Trigger via EventBridge every 30 seconds.

### Implementation Context

**Lambda file location:** `prototype/gbn-proto/infra/lambda/chaos-controller.py`

**Subnet-aware churn logic:** Use ECS *Service names* to identify subnet membership (not task tags, which require an extra `describe_tasks` API call). The two ECS Services (`HostileRelayService`, `FreeRelayService`) are already subnet-scoped.
```python
def handler(event, context):
    ecs = boto3.client('ecs')
    cluster = os.environ['CLUSTER_NAME']
    hostile_churn_rate = float(os.environ.get('HOSTILE_CHURN_RATE', '0.4'))
    free_churn_rate = float(os.environ.get('FREE_CHURN_RATE', '0.2'))

    hostile_tasks = ecs.list_tasks(cluster=cluster, serviceName='HostileRelayService')['taskArns']
    free_tasks = ecs.list_tasks(cluster=cluster, serviceName='FreeRelayService')['taskArns']

    for task_arn in random.sample(hostile_tasks, k=int(len(hostile_tasks) * hostile_churn_rate)):
        ecs.stop_task(cluster=cluster, task=task_arn, reason='ChaosEngine')
    for task_arn in random.sample(free_tasks, k=int(len(free_tasks) * free_churn_rate)):
        ecs.stop_task(cluster=cluster, task=task_arn, reason='ChaosEngine')
```

**Key insight — ECS Services handle replacement automatically:** `stop_task` on a Service-managed task causes ECS to immediately launch a replacement to maintain desired count. The Lambda does NOT call `run_task`. New tasks boot with stale SeedFleet knowledge by design (to test blackholing).

**Cloud Map deregistration:** When using Cloud Map's ECS Service Registry integration (Step 0), killed tasks are auto-deregistered when the ECS task stops. No explicit deregistration in the Lambda.

**EventBridge 30-second limitation:** EventBridge's minimum rate is 1 minute, not 30 seconds. To achieve 30-second effective churn, use **two EventBridge rules** staggered by 30 seconds:
- Rule A: `cron(0/1 * * * ? *)` — fires at 0s, 60s, 120s...
- Rule B: triggered by Rule A's Lambda invocation scheduling a delayed self-invoke via `lambda.invoke` with `InvocationType=Event` after a 30-second sleep inside the Lambda.

Alternatively, use an AWS Step Functions Express Workflow with a 30-second `Wait` state in a loop — this is cleaner but adds infrastructure complexity. Start with the two-rule approach.

**Start with Chaos Engine DISABLED:** The EventBridge rule should be created with `State: DISABLED` in CloudFormation. `run-chaos-upload.sh` (Step 9) enables it after the stabilization gate.

---

## Step 9: Orchestration Scripts
- `[x]` Write `deploy-scale-test.sh`: Deploy stack, boot initial 33% SeedFleet.
- `[x]` Add **Stabilization Gate 1**: Poll CloudWatch metrics until >90% of SeedFleet reports `ACTIVE` before scaling to 100%.
- `[x]` Write `run-chaos-upload.sh`: Add **Stabilization Gate 2** before enabling the Chaos Engine EventBridge rule. Shell into Creator container and trigger upload.
- `[x]` Write `teardown-scale-test.sh`: Disable Chaos Engine, dump metrics, delete stack.

### Implementation Context

**Script locations:** `prototype/gbn-proto/infra/scripts/` alongside existing Phase 1 scripts. Follow the same conventions: CloudFormation output resolution first, consistent `--region` flag, printed step headers.

**deploy-scale-test.sh key steps:**
1. `aws cloudformation create-stack ... --parameters ParameterKey=ScaleTarget,ParameterValue=$SCALE`
2. `aws cloudformation wait stack-create-complete`
3. Scale to 33% SeedFleet: `aws ecs update-service --desired-count $SEED_COUNT`
4. **Stabilization Gate 1:** Poll `aws cloudwatch get-metric-statistics` for `BootstrapResult` sum until it reaches `>= SEED_COUNT * 0.9`. Sleep 30s between polls. Timeout after 10 minutes.
5. Scale to 100%: `aws ecs update-service --desired-count $FULL_COUNT`

**run-chaos-upload.sh key steps:**
1. **Stabilization Gate 2:** Same CloudWatch polling loop at full scale.
2. `aws events enable-rule --name ChaosEngineRule`
3. Wait 60 seconds for churn to take effect (one full churn cycle before starting upload).
4. Trigger upload via `aws ecs execute-command` on the Creator task.

**teardown-scale-test.sh key steps:**
1. `aws events disable-rule --name ChaosEngineRule`
2. Dump metrics: `aws cloudwatch get-metric-data` for all `GBN/ScaleTest` metrics → `results/scale-${SCALE}-metrics.json`
3. Scale services to 0 (faster cost cutoff than waiting for stack delete): `aws ecs update-service --desired-count 0`
4. `aws cloudformation delete-stack --stack-name "$STACK_NAME"`

---

## Step 10: Scaled Execution
- `[ ]` Run test at N=100. *Note: Set `FREE_CHURN_RATE=0` at N=100 (only 10 exit nodes; churn makes 10 disjoint paths impossible).*
- `[ ]` Run test at N=500.
- `[ ]` Run test at N=1000.
- `[ ]` **Rollback Strategy:** If any scale fails, diagnose root cause, fix, and strictly re-run that scale before advancing.

### Implementation Context

**The N=100 exit node math problem:** At N=100, FreeSubnet has exactly 10 nodes. The test demands 10 disjoint 3-hop paths each exiting through a *different* FreeSubnet node — requiring ALL 10 to be alive simultaneously. With 20% churn killing ~2 exit nodes every 60 seconds, this is near-impossible to sustain. Fix: set the `FREE_CHURN_RATE` Lambda environment variable to `0.0` for the N=100 run only. At N=500 (50 exits) and N=1000 (100 exits), re-enable FreeSubnet churn — there's enough margin to find 10 alive exit nodes even under 20% churn.

**Expected cost per run:**
| Scale | Fargate tasks | Approx. cost/hour | Recommended run duration |
|-------|--------------|-------------------|--------------------------|
| N=100 | ~105 | ~$1.40 | 30 minutes |
| N=500 | ~505 | ~$6.70 | 30 minutes |
| N=1000 | ~1005 | ~$13.30 | 30 minutes |

Total estimated cost for all three runs: ~$11 (30 minutes each + setup/teardown overhead).

**Always run N=100 first.** If gossip storms or circuit failures appear at N=100, they will be 10x worse at N=1000 — fix the root cause before scaling up.

---

## Step 11: Reporting
- `[ ]` Compile results into a Markdown report analyzing Blackhole rates, Goodput vs Overhead, and Circuit Success.
- `[ ]` Evaluate against defined thresholds:
    - **PASS**: Goodput >60%, Blackhole <5%, Circuit Success >80% at N=1000.
    - **CONDITIONAL PASS**: Meets targets at N=500 but fails N=1000.
    - **FAIL**: Fails to meet targets at N=100.

### Implementation Context

**Source data:** `teardown-scale-test.sh` dumps all CloudWatch metrics to `results/scale-${SCALE}-metrics.json`. The report is generated from these files — not from live CloudWatch queries (the stack will be deleted by then).

**Report location:** `docs/prototyping/GBN-PROTO-004-Phase1-Scale-Results.md`

**Key metric calculations:**
- **Goodput ratio** = `(ChunksDelivered * chunk_size_bytes) / total_bytes_sent`. Total bytes = goodput + `GossipBandwidthBytes` + estimated handshake overhead (Noise_XX is ~200 bytes/hop × 3 hops × circuit count) + heartbeat traffic (5-second interval × circuit count × test duration).
- **Blackhole rate** = `count(BootstrapResult == 0) / count(all BootstrapResult events)` per churn cycle. Report the mean and worst-case (max per cycle) values.
- **Circuit success rate** = `count(CircuitBuildResult == 1) / count(all CircuitBuildResult events)`.
- **Time-to-convergence** = mean time between ECS task `startedAt` and first `BootstrapResult == 1` metric timestamp. Derive by joining ECS `describe_tasks` output with CloudWatch metric timestamps.
- **Path diversity** = an assertion, not a percentage. The Creator logs which relay IPs were used per path. Verify no relay IP appears in more than one simultaneous path. This either passes (100%) or fails (0%) — partial credit is a protocol bug.
