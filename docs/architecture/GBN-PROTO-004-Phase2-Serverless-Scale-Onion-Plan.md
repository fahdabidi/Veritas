# Plan: Correct Onion Routing — Asymmetric Layer Encryption

## Context

The current implementation is wrong in two ways:
1. It used Noise_XX *interactive handshakes* to build circuits hop-by-hop, which is not onion routing — it requires the Creator to interactively negotiate sessions with each hop, and Guard can observe that process.
2. The "bad fix" session removed nested encryption entirely.

The correct design (per user): Creator builds ALL encryption layers upfront before sending anything. Each layer is sealed with the next hop's public key. Each relay simply decrypts its own layer with its private key, reads the next hop address, and forwards the inner ciphertext. No circuit setup phase, no Noise_XX handshakes. This is classic onion routing.

---

## Architecture

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

## DHT Population And Validation

The DHT is now populated through separate discovery, direct-liveness, and routing-validation mechanisms. A node appearing in the DHT does not automatically mean it is trusted for routing.

### Population paths

1. NodeAnnounce
   Periodic PlumTree self-announcement used for wide discovery and eventual convergence.

2. DirectNodeAnnounce
   Immediate direct self-announcement exchanged when two peers connect on the gossip plane.

3. DirectNodePropagate
   Every 10 seconds, a node sends a sampled batch of its freshest live DHT entries to a sampled subset of neighbors.

4. DirectNodeProbe / DirectNodeProbeResponse
   A node first learned through propagation is queued for direct validation. Only the direct probe response populates last_direct_seen_ms.

### Local DHT entry fields

~~~text
+-------------------------+----------------------------------------------+
| Field                   | Meaning                                      |
+-------------------------+----------------------------------------------+
| addr                    | Onion ingress socket address                 |
| identity_pub            | Public key used for onion encryption         |
| subnet_tag              | Hostile / Free / Seed / Creator / Publisher  |
| announce_ts_ms          | Node's own latest self-announced timestamp   |
| last_direct_seen_ms     | Last time this node was heard directly       |
| last_propagated_seen_ms | Last time this node was heard via propagate  |
| last_observed_ms        | Most recent local observation of any kind    |
| validation_state        | propagated_only / unvalidated / direct /     |
|                         | complete / isolated                          |
| validation_score        | Routing confidence score                     |
+-------------------------+----------------------------------------------+
~~~

### Validation states

~~~text
propagated_only
  Discovered indirectly. Not trusted for routing. Inbound propagated DHT
  updates from this node are ignored.

unvalidated
  Directly seen for the first time. validation_score is seeded to 10, but the
  node is still not trusted for general path selection.

direct
  The node has participated in at least one successful chunk path that produced
  a publisher ACK. It is usable, but still in the preliminary validation period.

complete
  validation_score > 20. The node is fully trusted for routing and its
  DirectNodePropagate updates are accepted for DHT growth.

isolated
  validation_score == 0. The node remains in the DHT until stale cleanup, but
  it is excluded from path construction.
~~~

### Validation score rules

~~~text
First direct sighting              -> validation_score := max(score, 10)
First direct sighting              -> validation_state = unvalidated
Successful ACKed chunk             -> validation_score += 1
First ACK while unvalidated        -> validation_state = direct
Score > 20                         -> validation_state = complete
Failed routed chunk                -> validation_score -= 1
Score == 0                         -> validation_state = isolated
~~~

### Lazy validation during payload sends

New nodes are validated using real traffic, not a separate synthetic test circuit.

- Guard and Exit stay validated nodes.
- An unvalidated node is introduced only as a **middle** relay in a canary path.
- The Creator pairs that canary chunk with a sibling chunk that uses the same
  Guard and Exit but swaps in a validated middle.
- If both chunks succeed, the candidate middle gains score and can be promoted.
- If the baseline succeeds and the canary fails, only the candidate middle is
  penalized.

### Trust boundary for DHT growth

- PlumTree broadcast is discovery.
- Direct probe/response is direct liveness evidence.
- Publisher ACKed chunk delivery is routing evidence.
- Only complete nodes are allowed to influence DHT growth through accepted
  DirectNodePropagate batches.
---

## Encryption Primitive

Use Noise "N" one-shot pattern (`Noise_N_25519_ChaChaPoly_BLAKE2s`) from the existing `snow` crate — initiator knows recipient's static key, sends one message, no reply needed.

```rust
// seal: one-shot encrypt for recipient
pub fn seal(recipient_pub: &[u8; 32], plaintext: &[u8]) -> anyhow::Result<Vec<u8>> {
    let builder = snow::Builder::new("Noise_N_25519_ChaChaPoly_BLAKE2s".parse()?);
    let mut hs = builder.remote_public_key(recipient_pub).build_initiator()?;
    let mut buf = vec![0u8; plaintext.len() + 128];
    let len = hs.write_message(plaintext, &mut buf)?;
    Ok(buf[..len].to_vec())
}

// open: one-shot decrypt with own private key
pub fn open(local_priv: &[u8; 32], ciphertext: &[u8]) -> anyhow::Result<Vec<u8>> {
    let builder = snow::Builder::new("Noise_N_25519_ChaChaPoly_BLAKE2s".parse()?);
    let mut hs = builder.local_private_key(local_priv).build_responder()?;
    let mut buf = vec![0u8; ciphertext.len()];
    let len = hs.read_message(ciphertext, &mut buf)?;
    Ok(buf[..len].to_vec())
}
```

---

## Files to Modify

### 1. `crates/mcn-crypto/src/noise.rs`
Add `seal` and `open` functions above. Keep existing Noise_XX functions (used for transport elsewhere) but these new functions are what the onion routing uses.

### 2. `crates/gbn-protocol/src/onion.rs`
Replace the existing `OnionCell` with simpler structs:

```rust
// Deserialized inner content after opening a layer
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OnionLayer {
    pub next_hop: Option<SocketAddr>,  // None = I am the destination
    pub inner: Vec<u8>,               // next sealed OnionLayer, or ChunkPayload if None
}

// Innermost payload (Publisher decrypts to this)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChunkPayload {
    pub chunk_id: u64,
    pub hash: [u8; 32],
    pub chunk: Vec<u8>,
    pub return_path: Vec<HopInfo>,    // full path for ACK routing back to Creator
}

// HopInfo: one node in the path
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HopInfo {
    pub addr: SocketAddr,
    pub identity_pub: [u8; 32],
}

// ACK payload (Creator decrypts to this via reverse onion)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AckPayload {
    pub chunk_id: u64,
    pub hash: [u8; 32],
}
```

Keep `RelayHeartbeat` if it's used by PlumTree gossip; remove all circuit-building variants (`RelayExtend`, `RelayExtended`, etc.).

### 3. `crates/mcn-router-sim/src/relay_engine.rs`
Rewrite to a simple decrypt-and-forward loop. Remove Noise_XX transport entirely (relay just uses raw TCP):

```rust
pub async fn spawn_onion_relay(listen_addr: SocketAddr, local_priv_key: [u8; 32]) {
    let listener = TcpListener::bind(listen_addr).await?;
    loop {
        let (stream, _) = listener.accept().await?;
        let key = local_priv_key;
        tokio::spawn(async move { handle_onion_connection(stream, key).await });
    }
}

async fn handle_onion_connection(mut stream: TcpStream, local_priv: [u8; 32]) -> anyhow::Result<()> {
    let ciphertext = read_raw_frame(&mut stream).await?;
    let inner_bytes = mcn_crypto::noise::open(&local_priv, &ciphertext)?;
    let layer: OnionLayer = serde_json::from_slice(&inner_bytes)?;

    match layer.next_hop {
        Some(next_addr) => {
            // Relay: forward inner ciphertext to next hop
            let mut next_stream = TcpStream::connect(next_addr).await?;
            write_raw_frame(&mut next_stream, &layer.inner).await?;
            // ACK: read response from next hop and pipe back upstream
            let ack = read_raw_frame(&mut next_stream).await?;
            write_raw_frame(&mut stream, &ack).await?;
        }
        None => {
            // Publisher destination: decode chunk, send ACK back
            let payload: ChunkPayload = serde_json::from_slice(&layer.inner)?;
            // verify hash, store/process chunk...
            let ack = build_ack_onion(&payload)?;
            write_raw_frame(&mut stream, &ack).await?;
        }
    }
    Ok(())
}
```

**Raw frame helpers** (length-prefix, no Noise):
```rust
async fn write_raw_frame(stream: &mut TcpStream, data: &[u8]) -> anyhow::Result<()> {
    stream.write_all(&(data.len() as u32).to_be_bytes()).await?;
    stream.write_all(data).await?;
    Ok(())
}

async fn read_raw_frame(stream: &mut TcpStream) -> anyhow::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let mut buf = vec![0u8; u32::from_be_bytes(len_buf) as usize];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}
```

**ACK build** (Publisher builds reverse onion back to Creator):
```rust
fn build_ack_onion(payload: &ChunkPayload) -> anyhow::Result<Vec<u8>> {
    // return_path is [Creator, Guard, Middle, Exit, Publisher]
    // ACK travels: Publisher → Exit → Middle → Guard → Creator
    let ack = AckPayload { chunk_id: payload.chunk_id, hash: payload.hash, send_timestamp: payload.send_timestamp, total_chunks: payload.total_chunks, chunk_index: payload.chunk_index  }; 
    let mut inner = serde_json::to_vec(&ack)?;
    // Wrap innermost for Creator (next_hop = None)
    let creator = &payload.return_path[0];
    let mut layer = serde_json::to_vec(&OnionLayer { next_hop: None, inner })?;
    let mut sealed = seal(&creator.identity_pub, &layer)?;
    // Wrap for Guard, Middle, Exit (indexes 1, 2, 3) — reverse order
    for hop in payload.return_path[1..=3].iter().rev() {
        let next = // the hop BEFORE this one in the forward path = after this one in reverse
        // build layer pointing to next in reverse chain
    }
    // Note: exact index math shown below in implementation notes
    Ok(sealed)
}
```

**ACK wrapping (exact order)**:
- `return_path` = [Creator(0), Guard(1), Middle(2), Exit(3), Publisher(4)]
- ACK goes: Publisher → Exit(3) → Middle(2) → Guard(1) → Creator(0)
- Build innermost first (for Creator):
  ```
  ack_bytes = serialize(AckPayload)
  layer_creator = seal(creator.pub, serialize(OnionLayer { next_hop: None, inner: ack_bytes }))
  layer_guard   = seal(guard.pub,   serialize(OnionLayer { next_hop: Some(creator.addr), inner: layer_creator }))
  layer_middle  = seal(middle.pub,  serialize(OnionLayer { next_hop: Some(guard.addr),  inner: layer_guard   }))
  layer_exit    = seal(exit.pub,    serialize(OnionLayer { next_hop: Some(middle.addr), inner: layer_middle  }))
  ```
- Publisher sends `layer_exit` to Exit

### 4. `crates/mcn-router-sim/src/circuit_manager.rs`
Replace `build_circuit` and `send_chunk` with a single `send_chunk` function:

```rust
pub async fn send_chunk(
    path: &[HopInfo],   // [Guard, Middle, Exit, Publisher]
    creator_priv: &[u8; 32],
    creator_info: HopInfo,  // Creator's own addr + pub for return_path
    chunk: &[u8],
    chunk_id: u64,
) -> anyhow::Result<()> {
    let hash = blake3::hash(chunk).as_bytes().clone();

    // Build return_path for ACK
    let mut return_path = vec![creator_info.clone()];
    return_path.extend_from_slice(path);  // [Creator, Guard, Middle, Exit, Publisher]

    // Innermost layer: Publisher payload (next_hop = None)
    let pub_payload = ChunkPayload { chunk_id, hash, chunk: chunk.to_vec(), return_path };
    let inner_bytes = serde_json::to_vec(&pub_payload)?;
    let sealed_pub = seal(&path[3].identity_pub, &inner_bytes)?;

    // Layer for Exit: next_hop = Publisher
    let exit_layer = serde_json::to_vec(&OnionLayer { next_hop: Some(path[3].addr), inner: sealed_pub })?;
    let sealed_exit = seal(&path[2].identity_pub, &exit_layer)?;

    // Layer for Middle: next_hop = Exit
    let mid_layer = serde_json::to_vec(&OnionLayer { next_hop: Some(path[2].addr), inner: sealed_exit })?;
    let sealed_mid = seal(&path[1].identity_pub, &mid_layer)?;

    // Layer for Guard: next_hop = Middle
    let grd_layer = serde_json::to_vec(&OnionLayer { next_hop: Some(path[1].addr), inner: sealed_mid })?;
    let sealed_grd = seal(&path[0].identity_pub, &grd_layer)?;

    // Send to Guard, await ACK
    let mut stream = TcpStream::connect(path[0].addr).await?;
    write_raw_frame(&mut stream, &sealed_grd).await?;

    // Read ACK (reverse onion arrives here as sealed bytes for Creator)
    let ack_sealed = read_raw_frame(&mut stream).await?;
    let ack_inner = open(creator_priv, &ack_sealed)?;
    let ack_layer: OnionLayer = serde_json::from_slice(&ack_inner)?;
    // ack_layer.next_hop is None; ack_layer.inner is AckPayload
    let ack: AckPayload = serde_json::from_slice(&ack_layer.inner)?;
    assert_eq!(ack.chunk_id, chunk_id);
    assert_eq!(ack.hash, hash);
    tracing::info!(chunk_id, "ACK received — chunk delivered");
    Ok(())
}
```

**Note on path indexing** (path = [Guard(0), Middle(1), Exit(2), Publisher(3)]):
- `path[0]` = Guard, `path[1]` = Middle, `path[2]` = Exit, `path[3]` = Publisher
- Outermost seal uses `path[0].identity_pub` (Guard decrypts first)
- Innermost seal uses `path[3].identity_pub` (Publisher decrypts last)

### 5. `crates/proto-cli/src/main.rs`
- Update `spawn_onion_relay` call: remove `seed_store.clone()` argument (relay no longer needs DHT for onion routing)
- Update `send_chunk` / circuit builder call with new signature

---

## ACK Flow (Connection-level)

Each relay reads the ACK from downstream before sending back upstream:
```
Creator → Guard:  write sealed_grd, read ack_sealed
Guard → Middle:   write inner of sealed_grd, read ack
Middle → Exit:    write inner, read ack
Exit → Publisher: write inner, read ack
Publisher:        processes chunk, writes ack_layer_exit back
```

This works because each relay's `handle_onion_connection` does:
```
write_raw_frame(next_stream, inner)
let ack = read_raw_frame(next_stream)   // wait for response
write_raw_frame(upstream, ack)          // relay ACK back
```

---

## What to Remove

- `RelayExtend`, `RelayExtended`, `RelayExtendContinue`, `RelayForwardRequest` variants
- `complete_handshake` calls in relay_engine.rs (Noise_XX transport for onion port)
- `downstream_transport` / `downstream_pipe` state machine
- All circuit-building handshake logic in `circuit_manager.rs` but don't remove logic that builds DHT entries
- `seed_store` parameter from `spawn_onion_relay` but only if this does not interfere with the gossip protocol from populating the DHT table on the seedrelay or the DHT broadcasts

---

## Verification

1. `cargo build --workspace` — must compile clean
2. `cargo test -p mcn-router-sim`
3. `docker build` + push both images to ECR
4. `./restart-static-nodes.sh <stack>` redeploy EC2 nodes
5. `./relay-control-interactive.sh` → `SendDummy` → expect ACK in ring buffer with matching `chunk_id` and `hash`
6. CloudWatch logs should show Guard/Middle/Exit each logging "forwarding to next_hop" and Publisher logging "chunk received, sending ACK"


docker build --no-cache -t gbn-relay -f Dockerfile.relay .
