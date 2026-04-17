# Runtime Control API Integration Plan

This plan details how we will introduce a runtime control interface into the GBN nodes (Relay, Creator, Publisher, Hostile) to allow dynamic inspection and stimulation while running as ECS tasks.

## 1. Control Interface Architecture
We will introduce a lightweight TCP control server bound to `127.0.0.1:GBN_CONTROL_PORT` (default 5050). It will consume line-delimited JSON messages representing control commands and stream back responses. This allows us to use `aws ecs execute-command` + local `nc` (Netcat) to interact with running containers.

No heavy HTTP framework dependencies will be added; we will leverage standard `tokio::net::TcpListener` and `serde_json`.

### Protocol Definition
```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "cmd")]
pub enum ControlCommand {
    /// Dump the current Kademlia DHT routing table
    DumpDht,
    /// Send a dummy data payload of `size` bytes over a manually specified array of IPv4/Port strings.
    /// If `path` is empty, target the publisher directly.
    SendDummy { size: usize, path: Vec<String> },
    /// Dump the last N packet metadata records
    DumpMetadata,
}
```

## 2. Shared State & Subsystem Wiring

### A. DHT Table Dumps
* The `libp2p::Swarm` is tightly held by the `run_swarm_until_ctrl_c` loop.
* **Modification**: We will add a `mpsc::Receiver<SwarmControl>` channel to `run_swarm_until_ctrl_c`. The control server will send a `DumpDht(oneshot::Sender<Vec<String>>)` request.
* Inside the swarm loop, it will read `kademlia.kbuckets()`, format the connected peers into a vector, and reply through the oneshot channel.

### B. Packet Metadata Ring Buffer
* **Modification**: We will introduce a global static bounded ring buffer using `std::sync::Mutex<VecDeque<PacketMeta>>`.
* Tracking struct:
  ```rust
  pub struct PacketMeta {
      pub timestamp_ms: u64,
      pub action: String, // "RelayData", "ExitDelivery", "PublisherRecv"
      pub size_bytes: usize,
      pub info: String,
  }
  ```
* **Injection Points**:
  * `mcn-router-sim/src/relay_engine.rs`: Log on `RelayData` (forwarding) and Exit Delivery (to publisher).
  * `mpub-receiver/src/lib.rs`: Log in `recv_raw_frame` when a payload chunk arrives at the publisher.

### C. Sending Arbitrary Dummy Data
* **Modification**: We will add a helper in the Control Server task. When `SendDummy` is received, the task will:
  1. Generate a deterministic dummy payload of `size`.
  2. If `path` is provided, fetch Cloud Map relay records via `discover_relay_nodes_from_cloudmap()`, filter matching relays by IP to obtain their `Noise_XX` public keys, manually construct a `Circuit` sequence, and use `CircuitManager` to transmit.
  3. If `path` is empty, simulate a direct hop by querying `discover_publisher_addr_for_exit_relay()` and opening a direct TCP raw chunk stream exactly like the exit relay does.

## 3. ECS Usage Flow for Testing
Once shipped, an operator can SSH onto any ECS task:
```bash
aws ecs execute-command --cluster x --task y --interactive --command "/bin/bash"
# Inside container:
echo '{"cmd": "DumpDht"}' | nc 127.0.0.1 5050
echo '{"cmd": "SendDummy", "size": 1024, "path": ["10.0.3.45:9001", "10.0.2.14:9001"]}' | nc 127.0.0.1 5050
echo '{"cmd": "DumpMetadata"}' | nc 127.0.0.1 5050
```

## 4. Required Package Changes
* **proto-cli**: Added `control_server.rs` module and spawn the tokio listener in `main.rs` before `run_swarm_until_ctrl_c`.
* **mcn-router-sim**: Add bounded ring buffer logic, update `relay_engine` to log arrivals, update `swarm::run_swarm_...` to handle MPSC channels.
* **mpub-receiver**: Update arrival logs to push to the ring buffer. 

## User Review Required
> [!IMPORTANT]
> The `SendDummy` feature relies on the requested `path` IPs being actively registered in Cloud Map so we can fetch their required cryptograhic Noise keys. (A relay IP cannot be dialed natively via Onion without its public key). Will this constraint work for your tests, or do you expect to provide raw `<IP>,<PUBKEY>` pairs directly in the JSON command?
