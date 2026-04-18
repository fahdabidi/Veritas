# GBN Phase 2 Serverless Scale Test — Execution Plan

This document defines the Phase 2 implementation and execution roadmap for GBN-PROTO-004.

**Phase 2 Goal:** Wire the full telescopic onion circuit across real ECS Fargate nodes — from Creator
dialing live ECS relays, through FreeSubnet exit nodes, to a Publisher running `mpub-receiver` — and
verify that actual video payload is transmitted, chunked, encrypted, routed, and reassembled end-to-end
at the remote Publisher. Run at N=100 scale with the same Chaos Engine churn validated in Phase 1.

**Depends On:** Phase 1 N=100 gossip layer validation (completed 2026-04-14, `GBN-PROTO-004-Phase1-Serverless-Scale-Plan.md`).

**Infrastructure reused from Phase 1:** ECS Fargate cluster, CloudFormation stack (`phase1-scale-stack.yaml`),
Cloud Map namespace (`gbn.local`), Chaos Engine Lambda, `entrypoint.sh` IP injection, all orchestration scripts.

## Status trackers:
- `[ ]` Pending
- `[/]` In Progress
- `[x]` Completed

---

## Environment Reference

All Phase 2 work runs across two environments on the same Windows 11 machine. Each step below
calls out explicitly which environment to use. **Never mix environments mid-step** — in particular,
never run Docker commands from PowerShell and AWS CLI commands from WSL in the same script invocation.

### WSL2 Ubuntu (primary environment for all scripts, Docker, and Rust builds)

- **Open:** Windows Terminal → "Ubuntu" tab, or `wsl` in any Windows terminal
- **Project root inside WSL2:**
  ```
  /mnt/c/Users/fahd_/OneDrive/Documents/Global\ Broadcast\ Network/prototype/gbn-proto/
  ```
  Shorthand used throughout this document: `$PROTO_ROOT`
- **AWS CLI:** Use `aws` (the Linux-native AWS CLI v2 installed inside WSL2 Ubuntu).
  All bash scripts (`build-and-push.sh`, `deploy-scale-test.sh`, etc.) call `aws` directly.
  If only `aws.exe` is available the scripts auto-alias it, but prefer native `aws` in WSL2
  to avoid `wslpath` path-conversion issues with `--template-file` flags.
- **Docker:** Docker Desktop with WSL2 backend. The `docker` daemon socket is exposed into
  WSL2 automatically. Verify with `docker version` inside WSL2 before running any build.
- **Rust/Cargo:** Rust toolchain installed inside WSL2 (`rustup`). Use `cargo` (not `cargo.exe`).
  Verify with `cargo --version` inside WSL2. The `keygen` step and local `cargo test` runs
  both require the WSL2 Rust toolchain.
- **AWS authentication:** Uses the same AWS credentials/config as Windows (via
  `~/.aws/credentials` or SSO profile). If using SSO: run `aws sso login --profile <profile>`
  once per session inside WSL2 before running any script. No API keys are used — all AWS
  operations go through the AWS CLI with the configured IAM role/profile.
- **Session Manager plugin:** Required for `aws ecs execute-command` in `run-chaos-upload.sh`.
  Install inside WSL2:
  ```bash
  curl "https://s3.amazonaws.com/session-manager-downloads/plugin/latest/ubuntu_64bit/session-manager-plugin.deb" \
    -o /tmp/session-manager-plugin.deb
  sudo dpkg -i /tmp/session-manager-plugin.deb
  ```

### Windows PowerShell (secondary — only for CloudWatch console and manual AWS Console access)

- Use the AWS Console in a browser for CloudWatch dashboard inspection and ECS task log viewing.
- PowerShell `aws` (Windows-native AWS CLI) can run standalone ad-hoc queries but **must not**
  be used to run the bash orchestration scripts. Those scripts require a bash shell inside WSL2.
- If you must run a one-off AWS CLI command from PowerShell, use Windows path syntax:
  ```powershell
  aws cloudformation describe-stacks --stack-name gbn-proto-phase1-scale-n100 --region us-east-1
  ```

---

## What Phase 1 Proved (Baseline)

| Capability | Status | Evidence |
|---|---|---|
| 100 ECS Fargate nodes deploy and register in Cloud Map | ✅ | ECS `runningCount=95+` across 6 runs |
| PlumTree gossip converges across the mesh | ✅ | `GossipBandwidthBytes` 988→192 bytes/min |
| Creator auto-publishes gossip messages to live peers | ✅ | `ChunksDelivered` 2.0/min × 15 pts |
| Chaos Engine churn doesn't break gossip | ✅ | Non-zero gossip BW sustained through chaos run |
| Cloud Map peer discovery works under `HealthStatusFilter::All` | ✅ | Bootstrap events continuous |

## What Phase 1 Did NOT Prove

| Capability | Reason Missing | Phase 2 Work Item |
|---|---|---|
| Creator builds onion circuit to real ECS relays | `build_circuit()` never called from `serve` path | Step P2-1 |
| Creator sends actual `EncryptedChunkPacket`s through circuit | No upload triggered end-to-end | Step P2-2 |
| Exit relay delivers decrypted payload to Publisher | Handler logs/drops at exit node | Step P2-3 |
| Publisher receives and reassembles chunks | `serve` publisher is `ctrl_c().await` placeholder | Step P2-4 |
| Video payload reconstructed byte-perfectly at Publisher | Requires all of the above | Step P2-5 |
| Any Phase 1 test spec metric measured | Goodput, Blackhole, Circuit %, Path Diversity require circuit | Step P2-6 |

---

## Architecture Change: Phase 1 → Phase 2

**Phase 1 data flow (gossip only):**
```
Creator (ECS) → [PlumTree gossip mesh] → all relay nodes
                         ↑ only capability proven
```

**Phase 2 target data flow (full circuit + gossip):**
```
Creator (ECS, HostileSubnet)
  │  ① build_circuit() via Cloud Map discovered FreeSubnet exits
  │  ② RelayExtend × 3 hops → telescopic Noise_XX setup
  ▼
Guard Relay (ECS, HostileSubnet)
  │  ③ RelayData (triple-encrypted EncryptedChunkPacket)
  ▼
Middle Relay (ECS, HostileSubnet)
  │  ④ Decrypt outer layer, forward inner blob
  ▼
Exit Relay (ECS, FreeSubnet)           ← subnet_tag="FreeSubnet" in Cloud Map
  │  ⑤ Decrypt final layer → plaintext EncryptedChunkPacket bytes
  │  ⑥ TCP-forward to Publisher mpub-receiver (GBN_PUBLISHER_ADDR env var)
  ▼
Publisher (ECS, FreeSubnet)
  ⑦ mpub-receiver: buffer by SessionId, BLAKE3 verify, decrypt, reassemble
  ⑧ CloudWatch: ChunksReassembled metric on file-complete
```

**Geofence enforcement (unchanged from Phase 1 CloudFormation):**
- Security Group blocks direct `HostileSubnet → Publisher` TCP
- All relay-to-relay traffic uses the P2P port (4001) — within-VPC only
- Only FreeSubnet exit relay can reach Publisher on port `GBN_MPUB_PORT` (default: 7001)

---

## Step P2-1: Creator `serve` Path — Wire `build_circuit()`

**File:** `prototype/gbn-proto/crates/proto-cli/src/main.rs`
**Edit in:** VS Code on Windows (file is in `C:\Users\fahd_\OneDrive\Documents\Global Broadcast Network\prototype\gbn-proto\crates\proto-cli\src\main.rs`)
**Compile/test in:** WSL2 Ubuntu — `cd $PROTO_ROOT && cargo check -p proto-cli`
**Status:** `[x]`

### Current state
`Serve { role: "creator" }` calls `run_swarm_until_ctrl_c()` — gossip only. `build_circuit()` in
`circuit_manager.rs` is only called from `Commands::Upload` (local in-process mode, never from ECS).

### Changes required

**1. Add circuit builder call in `Serve { role: "creator" }` branch:**

After swarm boot + Cloud Map registration (i.e., after gossip peers are stable), the creator should:
1. Query Cloud Map `DiscoverInstances` for `subnet_tag=FreeSubnet` entries — these are the candidate exit nodes.
2. Call `build_circuits_speculative()` targeting discovered FreeSubnet exits.
3. On success: log circuit IDs, publish a `ChunksDelivered` metric, and initiate the upload sequence (Step P2-2).
4. On failure (no FreeSubnet exits alive, all paths timed out): log error, increment `CircuitBuildResult=0` metric, retry after 30s.

```rust
// In Serve { role: "creator" } after swarm is stable:
"creator" => {
    let mut swarm = swarm::build_swarm(local_key).await?;
    let mut runtime = swarm::GossipRuntime::from_env().await;

    // Wait for gossip mesh to stabilize before building circuits
    tokio::time::sleep(Duration::from_secs(
        env::var("GBN_CIRCUIT_DELAY_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60),
    )).await;

    // Discover FreeSubnet exit candidates via Cloud Map
    let exit_candidates = circuit_manager::discover_free_subnet_exits().await?;
    tracing::info!("Found {} FreeSubnet exit candidates", exit_candidates.len());

    // Build speculative circuits
    let circuits = circuit_manager::build_circuits_speculative(
        &exit_candidates,
        env::var("GBN_CIRCUIT_PATHS").ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3),
        env::var("GBN_CIRCUIT_HOPS").ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3),
    ).await;

    runtime.obs.publish_circuit_build_result(!circuits.is_empty()).await?;

    if !circuits.is_empty() {
        // Trigger upload sequence (Step P2-2)
        creator_upload_sequence(&circuits, &mut runtime).await?;
    }

    // Continue running gossip swarm
    swarm::run_swarm_until_ctrl_c(&mut swarm, &mut runtime).await?;
}
```

**2. Add `discover_free_subnet_exits()` to `circuit_manager.rs`:**

```rust
pub async fn discover_free_subnet_exits() -> Result<Vec<ExitCandidate>> {
    // Calls Cloud Map DiscoverInstances with attribute filter: subnet_tag = "FreeSubnet"
    // Returns Vec<ExitCandidate { addr: SocketAddr, identity_key: [u8; 32] }>
}
```

The Cloud Map attributes already include `GBN_PEER_ID` (from `register_with_cloudmap()` in
`swarm.rs`). Add a `subnet_tag` attribute using `GBN_SUBNET_TAG` env var (already set in CloudFormation
task definitions for FreeSubnet tasks).

**3. New CloudFormation env var — `GBN_CIRCUIT_DELAY_SECS`:**

Add to `CreatorTaskDefinition` in `phase1-scale-stack.yaml`:
```yaml
- Name: GBN_CIRCUIT_DELAY_SECS
  Value: '60'
- Name: GBN_CIRCUIT_PATHS
  Value: '3'
- Name: GBN_CIRCUIT_HOPS
  Value: '3'
```

### Validation gate
- `CircuitBuildResult` CloudWatch metric appears with value=1 in teardown JSON
- Creator logs show "circuit built" entries with 3-hop path IDs

---

## Step P2-2: Creator Upload Sequence — Send Real Chunks Through Circuit

**Files:** `prototype/gbn-proto/crates/proto-cli/src/main.rs`, `prototype/gbn-proto/crates/mcn-router-sim/src/circuit_manager.rs`
**Edit in:** VS Code on Windows
**Compile/test in:** WSL2 Ubuntu — `cd $PROTO_ROOT && cargo check -p proto-cli && cargo check -p mcn-router-sim`
**Status:** `[x]`

### Current state
`Commands::Upload` builds in-process TCP proxy relays on localhost. The circuit tunnel is real
(Noise_XX, telescopic extend), but exit nodes are local threads, not ECS tasks.

### Changes required

The Phase 2 `creator_upload_sequence()` function reuses the existing `Upload` pipeline (Steps 3–9 in
`main.rs`) but replaces the local `create_multipath_router()` call with circuits already built
against live ECS relay addresses.

**Synthetic video payload for ECS testing:**

In ECS there is no video file to read from disk. Generate a deterministic synthetic payload:

```rust
fn generate_synthetic_payload(size_bytes: usize) -> Vec<u8> {
    // Deterministic pseudo-random fill using ChaCha20 with a fixed seed
    // so the Publisher can verify the hash without receiving the original
    use chacha20::ChaCha20Rng;
    use rand::SeedableRng;
    let mut rng = ChaCha20Rng::seed_from_u64(0xGBN_PHASE2_SEED);
    let mut buf = vec![0u8; size_bytes];
    rng.fill_bytes(&mut buf);
    buf
}
```

Default payload: `GBN_UPLOAD_SIZE_BYTES` env var (default `10485760` = 10 MB).

**Publisher public key distribution:**

The Publisher generates its keypair at container start (or reads from a mounted secret) and
registers its X25519 public key as a Cloud Map attribute: `pub_key_hex`. The Creator reads this
via `DiscoverInstances` filtered by `role=publisher` before encrypting chunks.

```yaml
# New CloudFormation env var for PublisherTaskDefinition:
- Name: GBN_UPLOAD_SIZE_BYTES
  Value: '10485760'
- Name: GBN_PUB_ROLE_TAG
  Value: 'publisher'
```

**Wiring existing Upload steps to live circuits:**

```rust
// creator_upload_sequence() reuses pipeline steps:
// 3. chunk_file (or chunk_bytes for synthetic payload)
// 4. create_upload_session with publisher pub_key from Cloud Map
// 5. Route via live circuits (not localhost proxy)
// 6. session.encrypt_chunk() per chunk
// 7. circuit.send_encrypted_chunk_packet(&packet).await? per path
```

### Validation gate
- Creator logs show `N chunks dispatched through circuit` for each run
- `ChunksDelivered` metric counts actual `EncryptedChunkPacket` sends (not synthetic gossip strings)

---

## Step P2-3: Exit Relay — Hand Off Decrypted Payload to `mpub-receiver`

**File:** `prototype/gbn-proto/crates/mcn-router-sim/src/relay_engine.rs` lines 228–236
**Edit in:** VS Code on Windows
**Compile/test in:** WSL2 Ubuntu — `cd $PROTO_ROOT && cargo check -p mcn-router-sim`
**Status:** `[x]`

### Current state (lines 228–236)
```rust
None => {
    // Exit node: no downstream, this is the final payload.
    tracing::debug!("Exit node received {} bytes of payload", ciphertext.len());
    // In the prototype, we emit it to stdout / log for test capture.
    // The real implementation would forward to `mpub-receiver`.
}
```

### Required change

Replace the log-and-drop stub with a TCP forward to the Publisher's `mpub-receiver` listener.
The Publisher's address is read from `GBN_PUBLISHER_ADDR` env var (e.g., `10.0.3.45:7001`).

```rust
None => {
    // Exit node: forward plaintext to Publisher's mpub-receiver
    let publisher_addr: SocketAddr = std::env::var("GBN_PUBLISHER_ADDR")
        .context("GBN_PUBLISHER_ADDR not set on exit relay")?
        .parse()
        .context("Invalid GBN_PUBLISHER_ADDR")?;

    let mut pub_stream = timeout(
        Duration::from_secs(10),
        TcpStream::connect(publisher_addr),
    )
    .await
    .context("Timeout connecting to Publisher mpub-receiver")?
    .context("Failed to connect to Publisher mpub-receiver")?;

    // Length-prefix the chunk bytes so mpub-receiver can frame them
    let len = ciphertext.len() as u32;
    pub_stream.write_all(&len.to_le_bytes()).await?;
    pub_stream.write_all(&ciphertext).await?;
    pub_stream.flush().await?;

    tracing::info!(
        "Exit relay forwarded {} bytes to Publisher {}",
        ciphertext.len(), publisher_addr
    );
}
```

**Publisher address discovery for exit relays:**

The Publisher registers its private IP in Cloud Map with `role=publisher` attribute. Exit relay
tasks (FreeSubnet) query Cloud Map at startup and cache `GBN_PUBLISHER_ADDR`. Alternatively, set
it as an explicit CloudFormation parameter/env var injected at task definition time (simpler for
Phase 2):

```yaml
# FreeRelayTaskDefinition additional env var:
- Name: GBN_PUBLISHER_ADDR
  Value: ''   # populated at deploy time from Publisher task IP, or discovered via Cloud Map
```

The Cloud Map-based discovery is more robust (Publisher IP changes on each task restart). Use
`discover_publisher_addr()` helper called once at relay startup and cached for the session.

### Validation gate
- Publisher CloudWatch metric `ChunksReceived` increments
- Exit relay logs show `forwarded N bytes to Publisher`

---

## Step P2-4: Publisher `serve` Path — Run `mpub-receiver` TCP Listener

**File:** `prototype/gbn-proto/crates/proto-cli/src/main.rs` lines 230–235
**Edit in:** VS Code on Windows
**Compile/test in:** WSL2 Ubuntu — `cd $PROTO_ROOT && cargo check -p proto-cli`
**Status:** `[x]`

### Current state (lines 230–235)
```rust
"publisher" => {
    tracing::info!("Publisher service: waiting indefinitely (placeholder)");
    tokio::signal::ctrl_c().await?;
}
```

### Required change

Replace the placeholder with `mpub_receiver::Receiver` bound to `GBN_MPUB_PORT` (default 7001),
reading the Publisher private key from `GBN_PUBLISHER_KEY_HEX` env var, and publishing
`ChunksReassembled` to CloudWatch on session completion.

```rust
"publisher" => {
    let port: u16 = env::var("GBN_MPUB_PORT")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(7001);
    let listen_addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;

    tracing::info!("Publisher: starting mpub-receiver on {}", listen_addr);

    let receiver = mpub_receiver::Receiver::new(vec![listen_addr]);
    let mut handle = receiver.start().await?;

    // Load private key from env (hex-encoded seed)
    let key_hex = env::var("GBN_PUBLISHER_KEY_HEX")
        .context("GBN_PUBLISHER_KEY_HEX not set")?;
    let seed = hex::decode(&key_hex).context("Invalid GBN_PUBLISHER_KEY_HEX")?;
    let mut seed_arr = [0u8; 32];
    seed_arr.copy_from_slice(&seed[..32]);
    let pub_secret = mcn_crypto::PublisherSecret::from_seed(seed_arr);

    // Register public key in Cloud Map for Creator to discover
    let pub_key = mcn_crypto::public_key_from_secret(&pub_secret);
    swarm::register_publisher_pubkey_in_cloudmap(&pub_key).await?;

    tracing::info!("Publisher registered pub_key in Cloud Map");

    // Accept upload sessions — this loop runs until Ctrl+C
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            result = handle.await_any_session(Duration::from_secs(300)) => {
                match result {
                    Ok(session) => {
                        // Reassemble and verify
                        let out_path = format!("/tmp/gbn-recv-{}.bin", hex::encode(session.session_id));
                        session.decrypt_and_reassemble(&out_path, &pub_secret, ...)?;
                        let matched = session.verify(expected_hash, &out_path)?;
                        tracing::info!("Session {} reassembled: {} ", hex::encode(session.session_id), if matched { "PASS" } else { "FAIL" });

                        // Publish metric
                        obs.publish_chunks_reassembled(session.total_chunks as u64, matched).await?;
                    }
                    Err(e) => tracing::warn!("Session error: {}", e),
                }
            }
        }
    }
    handle.shutdown();
}
```

**New CloudFormation env vars for `PublisherTaskDefinition`:**
```yaml
- Name: GBN_MPUB_PORT
  Value: '7001'
- Name: GBN_PUBLISHER_KEY_HEX
  ValueFrom: !Sub "arn:aws:ssm:${AWS::Region}:${AWS::AccountId}:parameter/gbn/proto/publisher-key-hex"
```

**Publisher port must be open in Security Groups:**
- Publisher SG: inbound TCP 7001 from `FreeSubnetCidr` only (exit relays are in FreeSubnet)
- Publisher SG: block TCP 7001 from `HostileSubnetCidr` (geofence enforcement)

### Validation gate
- Publisher container logs show `mpub-receiver on 0.0.0.0:7001`
- `ChunksReassembled` CloudWatch metric appears in teardown JSON

---

## Step P2-5: New CloudWatch Metrics — `ChunksReassembled` and `CircuitBuildResult`

**Files:** `prototype/gbn-proto/crates/mcn-router-sim/src/observability.rs`, `prototype/gbn-proto/infra/cloudformation/phase1-scale-stack.yaml`, `prototype/gbn-proto/infra/scripts/teardown-scale-test.sh`
**Edit in:** VS Code on Windows (Rust files + YAML); WSL2 Ubuntu (verify bash syntax in teardown script)
**Compile/test in:** WSL2 Ubuntu — `cd $PROTO_ROOT && cargo check -p mcn-router-sim`
**Status:** `[x]`

### New metrics needed

| Metric Name | Published By | Dimensions | Meaning |
|---|---|---|---|
| `CircuitBuildResult` | Creator | `{Scale, NodeId}` | 1=success, 0=failure per circuit build attempt |
| `ChunksReassembled` | Publisher | `{Scale, Subnet}` (aggregate) | Count of fully reassembled sessions |
| `ChunksReceived` | Publisher | `{Scale, Subnet}` (aggregate) | Running count of individual chunk arrivals |

### Observability additions

**In `observability.rs`:**
```rust
pub async fn publish_circuit_build_result(&self, success: bool) -> Result<()> {
    // publishes CircuitBuildResult with value 1.0 or 0.0
    // uses self.dimensions (per-node — cardinality OK, not aggregated)
}

pub async fn publish_chunks_reassembled(&self, chunk_count: u64, hash_match: bool) -> Result<()> {
    // publishes ChunksReassembled with aggregate_dimensions() (Scale+Subnet only)
}

pub async fn publish_chunks_received(&self, count: u64) -> Result<()> {
    // publishes ChunksReceived with aggregate_dimensions()
}
```

### Teardown script additions (`teardown-scale-test.sh`)

**Run from:** WSL2 Ubuntu terminal.

Add queries for new metrics alongside the existing gossipbw/chunks/bootstrap/circuit queries:

```bash
CIRCUIT_JSON="$(cw_query CircuitBuildResult SampleCount)"
REASSEMBLED_JSON="$(cw_query_agg ChunksReassembled Sum)"
CHUNKS_RECV_JSON="$(cw_query_agg ChunksReceived Sum)"
```

### CloudFormation dashboard additions

Add `ChunksReassembled` and `CircuitBuildResult` widgets to the CloudWatch dashboard
(`gbn-proto-phase1-scale-n100-protocol-metrics`).

### Validation gate
- Teardown JSON has non-empty `Values` arrays for `CircuitBuildResult` and `ChunksReassembled`

---

## Step P2-6: Path Diversity Logging — Verify Disjoint Multi-Hop Paths

**File:** `prototype/gbn-proto/crates/mcn-router-sim/src/circuit_manager.rs`
**Edit in:** VS Code on Windows
**Compile/test in:** WSL2 Ubuntu — `cd $PROTO_ROOT && cargo test -p mcn-router-sim`
**Status:** `[x]`

### Requirement

The test spec requires 100% path diversity: no relay IP appears in more than one simultaneous
path. `circuit_manager.rs` already tracks circuit state. Add structured path logging:

```rust
// After all circuits are built, log the full hop topology:
for (circuit_id, circuit) in &circuits {
    tracing::info!(
        "Circuit {} hops: guard={} middle={} exit={}",
        circuit_id,
        circuit.guard_addr,
        circuit.middle_addr,
        circuit.exit_addr
    );
}

// Assert disjoint paths:
let all_relays: Vec<_> = circuits.values()
    .flat_map(|c| [c.guard_addr, c.middle_addr, c.exit_addr])
    .collect();
let unique_relays: HashSet<_> = all_relays.iter().collect();
let diversity_ok = unique_relays.len() == all_relays.len();
tracing::info!("Path diversity: {} (unique={}/{} total)", 
    if diversity_ok { "PASS" } else { "FAIL" },
    unique_relays.len(), all_relays.len()
);
obs.publish_path_diversity(diversity_ok).await?;
```

**New metric:** `PathDiversityResult` (1=all paths disjoint, 0=relay overlap detected), published
by Creator with `{Scale, NodeId}` dimensions.

### Validation gate
- Creator logs show `Path diversity: PASS` with unique relay count = paths × hops
- `PathDiversityResult` CloudWatch metric value = 1.0

---

## Step P2-7: Infrastructure Changes — CloudFormation Updates

**File:** `prototype/gbn-proto/infra/cloudformation/phase1-scale-stack.yaml`
**Edit in:** VS Code on Windows
**Deploy from:** WSL2 Ubuntu — the deploy script calls `aws cloudformation deploy` with a `--template-file` flag that resolves the YAML path from the WSL2 filesystem. See deployment notes below.
**Status:** `[x]`

### Changes required

**P2-7a — Publisher port (7001) in Security Groups:**
```yaml
# PublisherSecurityGroup: add inbound rule
- IpProtocol: tcp
  FromPort: 7001
  ToPort: 7001
  CidrIp: 10.0.2.0/24   # FreeSubnet CIDR only — geofence enforced here
```

**P2-7b — New CloudFormation parameters:**
```yaml
Parameters:
  PublisherKeyHexParam:
    Type: AWS::SSM::Parameter::Value<String>
    Default: /gbn/proto/publisher-key-hex
    Description: "Hex-encoded 32-byte publisher private key seed"
```

**P2-7c — Publisher task definition env vars** (additions to existing):
```yaml
- Name: GBN_MPUB_PORT
  Value: '7001'
- Name: GBN_PUBLISHER_KEY_HEX
  ValueFrom: !Sub "arn:aws:ssm:${AWS::Region}:${AWS::AccountId}:parameter/gbn/proto/publisher-key-hex"
- Name: GBN_PUB_ROLE_TAG
  Value: 'publisher'
```

**P2-7d — Creator and relay task definitions** (additions):
```yaml
- Name: GBN_CIRCUIT_DELAY_SECS
  Value: '60'
- Name: GBN_CIRCUIT_PATHS
  Value: '3'
- Name: GBN_CIRCUIT_HOPS
  Value: '3'
```

**P2-7e — FreeRelay task definition** (addition for exit relay delivery):
```yaml
- Name: GBN_PUBLISHER_ADDR
  Value: ''   # populated at deploy time from Publisher task IP, or discovered via Cloud Map
```

The cleanest approach: exit relay calls `discover_publisher_addr()` at startup (Cloud Map query
for `role=publisher`), avoiding a static parameter that breaks when Publisher task restarts.

**P2-7f — IAM: Publisher task role needs SSM read permission:**
```yaml
- Effect: Allow
  Action:
    - ssm:GetParameter
  Resource: !Sub "arn:aws:ssm:${AWS::Region}:${AWS::AccountId}:parameter/gbn/proto/*"
```

### Deploying CloudFormation updates

> **Environment: WSL2 Ubuntu**

The CloudFormation deploy script handles path conversion automatically between WSL2 Linux paths
and the Windows-native `aws.exe` if needed. Always run it from WSL2, not from PowerShell:

```bash
# WSL2 Ubuntu terminal
cd /mnt/c/Users/fahd_/OneDrive/Documents/Global\ Broadcast\ Network/prototype/gbn-proto/

# Validate the template before deploying (catches YAML syntax errors locally)
aws cloudformation validate-template \
  --template-body file://infra/cloudformation/phase1-scale-stack.yaml \
  --region us-east-1

# Deploy the updated stack (creates or updates the stack with new P2-7 changes)
bash infra/scripts/deploy-scale-test.sh gbn-proto-phase1-scale-n100 100 us-east-1
```

If the stack already exists from a prior run and you only need to push template changes without
scaling (e.g., to update task definitions), use `aws cloudformation deploy` directly:

```bash
# WSL2 Ubuntu — update stack definition without changing desired task counts
aws cloudformation deploy \
  --stack-name gbn-proto-phase1-scale-n100 \
  --template-file infra/cloudformation/phase1-scale-stack.yaml \
  --capabilities CAPABILITY_IAM \
  --no-fail-on-empty-changeset \
  --parameter-overrides ScaleTarget=100 \
  --region us-east-1
```

---

## Step P2-8: End-to-End Test Execution

**Status:** `[x]`

### Pre-run: Generate Publisher keypair and store in SSM

> **Environment: WSL2 Ubuntu** — both `cargo run` and `aws ssm put-parameter` must run inside
> WSL2. The `aws` command here is the **Linux-native AWS CLI**, not `aws.exe`. Using `aws.exe`
> for SSM SecureString writes from WSL2 can produce path/encoding errors.

```bash
# Open WSL2 Ubuntu terminal
cd /mnt/c/Users/fahd_/OneDrive/Documents/Global\ Broadcast\ Network/prototype/gbn-proto/

# Build and run the keygen subcommand using the WSL2 Rust toolchain
# This produces publisher.key (32-byte binary) and publisher.pub (public key)
cargo run --bin gbn-proto -- keygen

# Convert the binary key to hex (xxd is available in WSL2 Ubuntu by default)
KEY_HEX=$(xxd -p publisher.key | tr -d '\n')

# Store in SSM Parameter Store as SecureString (no API keys — uses configured AWS profile/SSO)
# If using SSO: run 'aws sso login --profile <your-profile>' first
aws ssm put-parameter \
  --name /gbn/proto/publisher-key-hex \
  --value "$KEY_HEX" \
  --type SecureString \
  --region us-east-1 \
  --overwrite

# Verify the parameter was stored (value will be redacted in output)
aws ssm get-parameter \
  --name /gbn/proto/publisher-key-hex \
  --with-decryption \
  --region us-east-1 \
  --query 'Parameter.Version'
```

This step is run **once per test campaign**. The same keypair is reused across multiple N=100
runs unless the Publisher task definition changes or the stack is deleted.

### Run sequence (same scripts as Phase 1)

Checkpoint-aware execution for N=100 with 30% seeded mesh:

> [ ] M0: no-chaos, one-hop onion forwarding (1,048,576 bytes)
> [ ] M0.5: no-chaos, two-hop onion forwarding (1,048,576 bytes)
> [ ] C1: no-chaos, single-chunk upload (1,048,576 bytes)
> [ ] C2: no-chaos, multi-chunk upload (10,485,760 bytes)
> [ ] C3: chaos enabled, single-chunk upload (1,048,576 bytes)
> [ ] C4: chaos enabled, multi-chunk upload (10,485,760 bytes)

```bash
# WSL2 Ubuntu terminal
cd /mnt/c/Users/fahd_/OneDrive/Documents/Global\ Broadcast\ Network/prototype/gbn-proto/

# Step 1: Build Docker images (relay + publisher) and push to ECR
bash infra/scripts/build-and-push.sh gbn-proto-phase1-scale-n100 us-east-1

# Step 2: Deploy CloudFormation stack with configurable seed ratio (30%)
# Note: deploy script now honors SEED_PERCENT.
SEED_PERCENT=30 bash infra/scripts/deploy-scale-test.sh gbn-proto-phase1-scale-n100 100 us-east-1

# Milestone 0 (stable, one-hop onion forwarding)
# Precondition: set GBN_CIRCUIT_HOPS=1, GBN_CIRCUIT_PATHS=1,
# GBN_UPLOAD_SIZE_BYTES=1048576 in phase1-scale-stack.yaml.
ENABLE_CHAOS=0 bash infra/scripts/run-chaos-upload.sh gbn-proto-phase1-scale-n100 us-east-1 "sleep 240"
bash infra/scripts/teardown-scale-test.sh gbn-proto-phase1-scale-n100 us-east-1

# Milestone 0.5 (stable, two-hop onion forwarding)
# Precondition: set GBN_CIRCUIT_HOPS=2, GBN_CIRCUIT_PATHS=1,
# GBN_UPLOAD_SIZE_BYTES=1048576 in phase1-scale-stack.yaml.
SEED_PERCENT=30 bash infra/scripts/deploy-scale-test.sh gbn-proto-phase1-scale-n100 100 us-east-1
ENABLE_CHAOS=0 bash infra/scripts/run-chaos-upload.sh gbn-proto-phase1-scale-n100 us-east-1 "sleep 240"
bash infra/scripts/teardown-scale-test.sh gbn-proto-phase1-scale-n100 us-east-1

# Checkpoint C1 (stable, single chunk)
# Precondition: set GBN_CIRCUIT_HOPS=3, GBN_CIRCUIT_PATHS=10,
# GBN_UPLOAD_SIZE_BYTES=1048576 in phase1-scale-stack.yaml, then redeploy.
SEED_PERCENT=30 bash infra/scripts/deploy-scale-test.sh gbn-proto-phase1-scale-n100 100 us-east-1
ENABLE_CHAOS=0 bash infra/scripts/run-chaos-upload.sh gbn-proto-phase1-scale-n100 us-east-1 "sleep 240"
bash infra/scripts/teardown-scale-test.sh gbn-proto-phase1-scale-n100 us-east-1

# Checkpoint C2 (stable, full payload)
# Precondition: update Creator env to GBN_UPLOAD_SIZE_BYTES=10485760 in phase1-scale-stack.yaml and redeploy.
SEED_PERCENT=30 bash infra/scripts/deploy-scale-test.sh gbn-proto-phase1-scale-n100 100 us-east-1
ENABLE_CHAOS=0 bash infra/scripts/run-chaos-upload.sh gbn-proto-phase1-scale-n100 us-east-1 "sleep 240"
bash infra/scripts/teardown-scale-test.sh gbn-proto-phase1-scale-n100 us-east-1

# Checkpoint C3 (chaos, single chunk)
# Precondition: set GBN_UPLOAD_SIZE_BYTES=1048576 and rerun deploy before this run.
SEED_PERCENT=30 bash infra/scripts/deploy-scale-test.sh gbn-proto-phase1-scale-n100 100 us-east-1
ENABLE_CHAOS=1 bash infra/scripts/run-chaos-upload.sh gbn-proto-phase1-scale-n100 us-east-1 "sleep 240"
bash infra/scripts/teardown-scale-test.sh gbn-proto-phase1-scale-n100 us-east-1

# Checkpoint C4 (chaos, full payload)
# Precondition: set GBN_UPLOAD_SIZE_BYTES=10485760 and rerun deploy before this run.
SEED_PERCENT=30 bash infra/scripts/deploy-scale-test.sh gbn-proto-phase1-scale-n100 100 us-east-1
ENABLE_CHAOS=1 bash infra/scripts/run-chaos-upload.sh gbn-proto-phase1-scale-n100 us-east-1 "sleep 240"
bash infra/scripts/teardown-scale-test.sh gbn-proto-phase1-scale-n100 us-east-1
```
### Monitoring during the run

> **Environment: Browser (AWS Console) or WSL2 Ubuntu for CLI queries**

While the test is running (after Step 3 starts, during the 600s chaos window):

**Option A — AWS Console (browser, no environment required):**
- CloudWatch → Dashboards → `gbn-proto-phase1-scale-n100-protocol-metrics`
- ECS → Clusters → `gbn-proto-phase1-scale-n100-*` → Services → view running/pending counts

**Option B — WSL2 Ubuntu ad-hoc CloudWatch query:**
```bash
# WSL2 Ubuntu — spot-check ChunksReassembled in real time
aws cloudwatch get-metric-statistics \
  --namespace GBN/ScaleTest \
  --metric-name ChunksReassembled \
  --dimensions Name=Scale,Value=100 \
  --start-time $(date -u -d '30 minutes ago' +%Y-%m-%dT%H:%M:%SZ) \
  --end-time $(date -u +%Y-%m-%dT%H:%M:%SZ) \
  --period 60 \
  --statistics Sum \
  --region us-east-1
```

**ECS container logs** (exit relay and publisher logs, to debug handoff issues):
```bash
# WSL2 Ubuntu — tail recent CloudWatch Logs for the publisher container
aws logs get-log-events \
  --log-group-name /aws/ecs/gbn-proto-phase1-scale-n100/gbn \
  --log-stream-name publisher/<container-id> \
  --region us-east-1 \
  --limit 50
```

### Expected results file
`results/scale-gbn-proto-phase1-scale-n100-<TIMESTAMP>-metrics.json`

---

## Step P2-9: Phase 2 Test Results Report

**Status:** `[x]`

### Success criteria (from Phase 1 test spec, now measurable)

| Metric | Target | How Measured |
|---|---|---|
| **Circuit Build Success Rate** | >80% | `CircuitBuildResult` SampleCount vs. Sum (1s = success) |
| **Path Diversity** | 100% disjoint | `PathDiversityResult` = 1.0 |
| **Chunks Reassembled at Publisher** | ≥1 complete session | `ChunksReassembled` non-zero |
| **Goodput vs. Overhead Ratio** | >60% goodput | `(ChunksReassembled * chunk_size) / (total bytes = gossipbw + circuit overhead)` |
| **Blackhole Rate** | <5% | `count(BootstrapResult=0) / count(all BootstrapResult)` per churn cycle |
| **Time-to-Convergence** | <15s | ECS `startedAt` → first `BootstrapResult=1` timestamp delta |

### Pass/Fail gate

- **PASS:** Circuit Build Success >80%, `ChunksReassembled ≥ 1`, `PathDiversityResult = 1`, Goodput >60%, Blackhole <5%
- **PARTIAL PASS:** Chunks reassembled but circuit success <80% or goodput <60% — indicates circuit instability under chaos
- **FAIL:** `ChunksReassembled = 0` — publisher never received a complete session

### Report location
`docs/prototyping/GBN-PROTO-004-Phase2-Scale-Results.md`

---

## Dependency Map

```
P2-7 (CFN infra)
  ↓
P2-1 (Creator build_circuit) ──→ P2-2 (Creator upload sequence)
P2-4 (Publisher mpub-receiver)       ↓
P2-3 (Exit relay handoff) ──────→ P2-5 (New CW metrics)
                                      ↓
P2-6 (Path diversity logging)     P2-8 (Execution)
                                      ↓
                                  P2-9 (Report)
```

P2-1, P2-3, and P2-4 are the three critical wiring tasks. They can be implemented in parallel.
P2-5 and P2-7 support all three and should be done first.

---

## Critical Files

| File | Change | Edit in | Run/Test in |
|---|---|---|---|
| `prototype/gbn-proto/crates/proto-cli/src/main.rs` | P2-1, P2-4: Creator circuit wiring, Publisher mpub-receiver | VS Code (Windows) | WSL2: `cargo check -p proto-cli` |
| `prototype/gbn-proto/crates/mcn-router-sim/src/relay_engine.rs` | P2-3: Exit relay TCP forward to Publisher | VS Code (Windows) | WSL2: `cargo check -p mcn-router-sim` |
| `prototype/gbn-proto/crates/mcn-router-sim/src/circuit_manager.rs` | P2-1: `discover_free_subnet_exits()`, P2-6: path diversity assert | VS Code (Windows) | WSL2: `cargo test -p mcn-router-sim` |
| `prototype/gbn-proto/crates/mcn-router-sim/src/observability.rs` | P2-5: `CircuitBuildResult`, `ChunksReassembled`, `PathDiversityResult` | VS Code (Windows) | WSL2: `cargo check -p mcn-router-sim` |
| `prototype/gbn-proto/crates/mcn-router-sim/src/swarm.rs` | P2-4: `register_publisher_pubkey_in_cloudmap()` | VS Code (Windows) | WSL2: `cargo check -p mcn-router-sim` |
| `prototype/gbn-proto/infra/cloudformation/phase1-scale-stack.yaml` | P2-7: ports, IAM, env vars | VS Code (Windows) | WSL2: `aws cloudformation validate-template` |
| `prototype/gbn-proto/infra/scripts/teardown-scale-test.sh` | P2-5: query new metrics | VS Code (Windows) | WSL2: `bash` syntax check |
| `prototype/gbn-proto/infra/scripts/run-chaos-upload.sh` | P2-8: SSM keygen step, circuit timeout gate | VS Code (Windows) | WSL2: `bash` |

## Runtime Diagnostic Commands

During ECS runtime, operators can dynamically interrogate any active node (Creator, Publisher, or Relay) using the newly integrated Control TCP server bound locally to port `5050` inside every container.

To access these, use ECS Execute Command to drop into a container shell, and use `nc` to send JSON payloads. The commands are currently supported by the Kademlia DHT Seed Store and `relay_engine.rs`:

```bash

#Connecting to ECS Containers in an ECS Cluster
#Step 1 get a container task ID - This changes when stack is relaunched
aws ecs list-tasks --cluster gbn-proto-phase1-scale-n100-cluster

#Step 2 get the container name (look for relay or creator etc)
aws ecs describe-tasks --cluster gbn-proto-phase1-scale-n100-cluster --tasks 9b43f1ee8bd747beb776c1e33cc402a0 --query 'tasks[0].containers[*].name'

# Step 3. Connect to an ECS Container/Task
aws ecs execute-command --cluster gbn-proto-phase1-scale-n100-cluster --task  arn:aws:ecs:us-east-1:138472308340:task/gbn-proto-phase1-scale-n100-cluster/cb5845e94382412db25ed7f48688db5e --container relay --region us-east-1 --interactive --command "echo helo world"

#Connecting to EC2 Instances running the GBN Containers in Docker environment using system session manager
#Step1 : user the instance ID to start the session
aws ssm start-session --target i-08ec6402d9bdee34c --region us-east-1

#Step2 : connect to the container environmet
sudo -i
docker ps
docker exec -it gbn-seed-relay sh

#Running commands from the container exec environment
# 2. Dump Kademlia DHT mapping & Local Gossip Directory Seed Store
echo '{"cmd":"DumpDht"}' | nc -w 1 127.0.0.1 5050
printf '%s\n' '{"cmd":"DumpDht"}' | nc -q 0 127.0.0.1 5050

# Ex Full Command for ECS Containers (Besure to replace the --container and --task to match the current deployment) Environment:
aws ecs execute-command --cluster gbn-proto-phase1-scale-n100-cluster --task  arn:aws:ecs:us-east-1:138472308340:task/gbn-proto-phase1-scale-n100-cluster/7c612a3dd0e6411ab6d1222ad6a89ab2 --container relay --region us-east-1 --interactive --command "echo '{"cmd": "DumpDht"}' | nc -w 1 127.0.0.1 5050"

# 3. Dump Packet Metadata Ring Buffer (last 100 arriving frames)
echo '{"cmd": "DumpMetadata"}' | nc -w 1 127.0.0.1 5050

aws ecs execute-command --cluster gbn-proto-phase1-scale-n100-cluster --task arn:aws:ecs:us-east-1:138472308340:task/gbn-proto-phase1-scale-n100-cluster/7c612a3dd0e6411ab6d1222ad6a89ab2 --container relay --region us-east-1 --interactive --command "python3 -c \"import socket; s=socket.create_connection(('127.0.0.1',5050),2); s.sendall(b'{\\\"cmd\\\":\\\"DumpMetadata\\\"}\\n'); print(s.recv(65535).decode()); s.close()\""


# 4. Inject a customizable Dummy byte payload over a defined Onion Routing path (gaurd->relay->exit->publisher)  (automatically fetches Node PUBKEYs via local DHT)
echo '{"cmd":"SendDummy","size":512,"path":["10.0.1.223:9001","10.0.0.8:9001","10.0.3.211:9001"]}' | nc -w 1 127.0.0.1 5050

# 5. Force a node to manually broadcast the Cloud Map Gossip Directory to the entire network
echo '{"cmd": "BroadcastSeed"}' | nc -w 1 127.0.0.1 5050
```
---


## What Is NOT Changing in Phase 2

- ECS Fargate infrastructure topology (same VPC, subnets, Security Groups, Chaos Engine)
- CloudFormation stack name convention (`gbn-proto-phase1-scale-n100`)
- Orchestration scripts structure (`deploy → run-chaos-upload → teardown`)
- PlumTree gossip layer (Phase 1 validated — untouched)
- Cloud Map peer discovery and `entrypoint.sh` IP injection
- Scale target: N=100 only. N=500/N=1000 deferred until Phase 2 circuit test passes.


export AWS_REGION=us-east-1
export REGION=us-east-1
export CLUSTER=gbn-proto-phase1-scale-n100-cluster
export HOSTILE_SVC=$(aws ecs list-services --cluster "$CLUSTER" --region "$AWS_REGION" --query 'serviceArns[?contains(@,`HostileRelayService`)]|[0]' --output text)
echo "AWS_REGION=$AWS_REGION"
echo "HOSTILE_SVC=$HOSTILE_SVC"


TD=$(aws ecs describe-services --cluster "$CLUSTER" --services "$HOSTILE_SVC" --region "$AWS_REGION" --query 'services[0].taskDefinition' --output text)
echo "TD=$TD"
aws ecs describe-task-definition --task-definition "$TD" --region "$AWS_REGION" \
  --query 'taskDefinition.containerDefinitions[0].environment[?name==`GBN_SEED_IPS` || name==`GBN_SEED_PUBKEYS` || name==`GBN_ROLE`]' \
  --output table


aws ssm send-command \
  --region us-east-1 \
  --instance-ids i-0939cf22a5923a683 \
  --document-name AWS-RunShellScript \
  --parameters commands='["docker inspect gbn-seed-relay --format \"{{range .Config.Env}}{{println .}}{{end}}\" | grep -E \"GBN_(NOISE_PRIVKEY_HEX|INSTANCE_IPV4|ROLE)\""]'


for C in ce0da5f8-8728-4d0b-9598-9658428e3a5b f88687b7-2c82-4eb3-ade9-e58f63187a4b; do
  echo "=== $C ==="
  aws ssm get-command-invocation \
    --region us-east-1 \
    --command-id "$C" \
    --instance-id "$( [ "$C" = "ce0da5f8-8728-4d0b-9598-9658428e3a5b" ] && echo i-0939cf22a5923a683 || echo i-0502b9408ffaa6f7f )" \
    --query '{Status:Status,StdOut:StandardOutputContent,StdErr:StandardErrorContent}' \
    --output json
done


CMD_ID=$(aws ssm send-command \
  --region us-east-1 \
  --instance-ids i-08ec6402d9bdee34c \
  --document-name AWS-RunShellScript \
  --parameters 'commands=["sudo docker ps","sudo docker exec -i gbn-seed-relay sh -c \"echo '\''{\"cmd\":\"DumpDht\"}'\'' | nc -w 1 127.0.0.1 5050\""]' \
  --query 'Command.CommandId' \
  --output text)

aws ssm get-command-invocation \
  --region us-east-1 \
  --command-id "$CMD_ID" \
  --instance-id i-08ec6402d9bdee34c
