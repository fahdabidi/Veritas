use crate::circuit_manager::{CircuitManager, RelayNode};
use crate::swarm::SwarmControlCmd;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

// ─────────────────────────── Protocol Types ───────────────────────────────

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "cmd")]
pub enum ControlRequest {
    DumpDht,
    SendDummy {
        size: usize,
        path: Vec<String>,
    },
    DumpMetadata {
        limit: Option<usize>,
    },
    BroadcastSeed,
    UnicastDHT {
        target_addr: String,
    },
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ControlResponse {
    Ok { msg: String },
    DhtData { 
        store: Vec<RelayNode>,
        kademlia_buckets: Vec<String>,
    },
    Metadata { packets: Vec<PacketMeta> },
    Error { reason: String },
}

// ─────────────────────────── Packet Metadata Tracker ──────────────────────

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct PacketMeta {
    pub timestamp_ms: u64,
    pub action: String,
    pub size_bytes: usize,
    pub info: String,
    #[cfg(feature = "distributed-trace")]
    pub id_chain: String,
    #[cfg(feature = "distributed-trace")]
    pub phase: String,
    #[cfg(feature = "distributed-trace")]
    pub node_id: String,
}

static PACKET_METADATA_RING: OnceLock<Mutex<VecDeque<PacketMeta>>> = OnceLock::new();
const PACKET_METADATA_RING_CAPACITY: usize = 1000;
const PACKET_METADATA_DEFAULT_LIMIT: usize = 20;

fn get_ring() -> &'static Mutex<VecDeque<PacketMeta>> {
    PACKET_METADATA_RING
        .get_or_init(|| Mutex::new(VecDeque::with_capacity(PACKET_METADATA_RING_CAPACITY)))
}

fn next_chain(parent: &str) -> String {
    let hop = crate::trace::next_hop_id();
    if parent.is_empty() {
        hop
    } else if hop.is_empty() {
        parent.to_string()
    } else {
        format!("{parent} -> {hop}")
    }
}

pub fn push_packet_meta(action: &str, size_bytes: usize, info: &str) {
    #[cfg(not(feature = "distributed-trace"))]
    {
        let mut ring = get_ring().lock().unwrap();
        if ring.len() >= PACKET_METADATA_RING_CAPACITY {
            ring.pop_back();
        }
        ring.push_front(PacketMeta {
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            action: action.to_string(),
            size_bytes,
            info: info.to_string(),
        });
    }

    #[cfg(feature = "distributed-trace")]
    {
        push_packet_meta_trace(action, size_bytes, info, "", "legacy");
    }
}

#[cfg(feature = "distributed-trace")]
pub fn push_packet_meta_trace(
    action: &str,
    size_bytes: usize,
    info: &str,
    id_chain: &str,
    phase: &str,
) {
    let mut ring = get_ring().lock().unwrap();
    if ring.len() >= PACKET_METADATA_RING_CAPACITY {
        ring.pop_back();
    }
    ring.push_front(PacketMeta {
        timestamp_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        action: action.to_string(),
        size_bytes,
        info: info.to_string(),
        id_chain: id_chain.to_string(),
        phase: phase.to_string(),
        node_id: crate::trace::node_id(),
    });
}

#[cfg(not(feature = "distributed-trace"))]
pub fn push_packet_meta_trace(
    action: &str,
    size_bytes: usize,
    info: &str,
    _id_chain: &str,
    _phase: &str,
) {
    push_packet_meta(action, size_bytes, info);
}

// ─────────────────────────── Control Server ───────────────────────────────

pub async fn spawn_control_server(
    listen_port: u16,
    seed_store: Arc<RwLock<HashMap<SocketAddr, RelayNode>>>,
    swarm_tx: mpsc::Sender<SwarmControlCmd>,
    noise_priv_key: [u8; 32],
) -> Result<()> {
    let addr = format!("0.0.0.0:{}", listen_port);
    let listener = TcpListener::bind(&addr).await?;
    info!("Control API listening on {}", addr);

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((mut socket, peer)) => {
                    debug!("Control connection from {}", peer);
                    let seed_store = seed_store.clone();
                    let swarm_tx = swarm_tx.clone();
                    let priv_key = noise_priv_key;
                    tokio::spawn(async move {
                        let (read, mut write) = socket.split();
                        let mut reader = BufReader::new(read);
                        let mut line = String::new();

                        loop {
                            line.clear();
                            match reader.read_line(&mut line).await {
                                Ok(0) => break, // EOF
                                Ok(_) => {
                                    if line.trim().is_empty() {
                                        continue;
                                    }
                                    let req_res: Result<ControlRequest, _> = serde_json::from_str(line.trim());
                                    match req_res {
                                        Ok(req) => {
                                            let resp = handle_request(req, &seed_store, &swarm_tx, priv_key).await;
                                            let resp_json = serde_json::to_string(&resp).unwrap_or_default();
                                            if let Err(e) = write.write_all(format!("{}\n", resp_json).as_bytes()).await {
                                                warn!("Failed writing to control client: {e}");
                                                break;
                                            }
                                        }
                                        Err(e) => {
                                            let chain = next_chain("");
                                            push_packet_meta_trace(
                                                "ComponentError",
                                                line.len(),
                                                &format!("control.parse_request ERROR err={e} raw={}", line.trim()),
                                                &chain,
                                                "component.error",
                                            );
                                            let err = ControlResponse::Error { reason: format!("Bad request: {e}") };
                                            let _ = write.write_all(format!("{}\n", serde_json::to_string(&err).unwrap()).as_bytes()).await;
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!("Error reading from control client: {e}");
                                    break;
                                }
                            }
                        }
                    });
                }
                Err(e) => error!("Control API accept error: {e}"),
            }
        }
    });

    Ok(())
}

async fn handle_request(
    req: ControlRequest,
    seed_store: &Arc<RwLock<HashMap<SocketAddr, RelayNode>>>,
    swarm_tx: &mpsc::Sender<SwarmControlCmd>,
    noise_priv_key: [u8; 32],
) -> ControlResponse {
    match req {
        ControlRequest::DumpDht => {
            let chain = next_chain("");
            push_packet_meta_trace(
                "ComponentInput",
                0,
                "control.DumpDht INPUT",
                &chain,
                "component.input",
            );
            let (tx, mut rx) = tokio::sync::mpsc::channel(1);
            if swarm_tx.send(SwarmControlCmd::DumpDht(tx)).await.is_err() {
                push_packet_meta_trace(
                    "ComponentError",
                    0,
                    "control.DumpDht ERROR swarm loop disconnected",
                    &chain,
                    "component.error",
                );
                return ControlResponse::Error { reason: "Swarm loop disconnected".into() };
            }

            let kademlia_buckets = match rx.recv().await {
                Some(buckets) => buckets,
                None => vec!["Failed to fetch Kademlia buckets".into()],
            };

            let mut store: Vec<RelayNode> = seed_store.read().unwrap().values().cloned().collect();
            store.sort_unstable_by(|a, b| b.last_seen_ms.cmp(&a.last_seen_ms));
            push_packet_meta_trace(
                "ComponentOutput",
                store.len(),
                &format!(
                    "control.DumpDht OUTPUT store_nodes={} kademlia_buckets={}",
                    store.len(),
                    kademlia_buckets.len()
                ),
                &chain,
                "component.output",
            );

            ControlResponse::DhtData {
                store,
                kademlia_buckets,
            }
        }
        ControlRequest::DumpMetadata { limit } => {
            let chain = next_chain("");
            // 0 or absent → return all entries in the ring
            let effective_limit = match limit {
                None | Some(0) => PACKET_METADATA_RING_CAPACITY,
                Some(n) => n.min(PACKET_METADATA_RING_CAPACITY),
            };
            push_packet_meta_trace(
                "ComponentInput",
                effective_limit,
                &format!(
                    "control.DumpMetadata INPUT limit={:?} effective_limit={}",
                    limit, effective_limit
                ),
                &chain,
                "component.input",
            );
            let (packets, total_in_ring): (Vec<PacketMeta>, usize) = {
                let ring = get_ring().lock().unwrap();
                let total = ring.len();
                // ring is newest-first (push_front); take the first N = the N most recent
                let packets = ring.iter().take(effective_limit).cloned().collect();
                (packets, total)
            };
            push_packet_meta_trace(
                "ComponentOutput",
                packets.len(),
                &format!(
                    "control.DumpMetadata OUTPUT packets={} total_in_ring={} requested_limit={:?} effective_limit={}",
                    packets.len(), total_in_ring, limit, effective_limit
                ),
                &chain,
                "component.output",
            );
            ControlResponse::Metadata { packets }
        }
        ControlRequest::BroadcastSeed => {
            let chain = next_chain("");
            push_packet_meta_trace(
                "ComponentInput",
                0,
                "control.BroadcastSeed INPUT",
                &chain,
                "component.input",
            );
            let _ = swarm_tx.send(SwarmControlCmd::BroadcastSeed).await;
            push_packet_meta_trace(
                "ComponentOutput",
                0,
                "control.BroadcastSeed OUTPUT queued",
                &chain,
                "component.output",
            );
            ControlResponse::Ok { msg: "CloudMap local seed broadcast queued".to_string() }
        }
        ControlRequest::UnicastDHT { target_addr } => {
            let chain = next_chain("");
            push_packet_meta_trace(
                "ComponentInput", 0,
                &format!("control.UnicastDHT INPUT target={target_addr}"),
                &chain, "component.input",
            );
            if swarm_tx.send(SwarmControlCmd::UnicastDHT { target_addr: target_addr.clone() }).await.is_err() {
                push_packet_meta_trace(
                    "ComponentError", 0,
                    "control.UnicastDHT ERROR swarm loop disconnected",
                    &chain, "component.error",
                );
                return ControlResponse::Error { reason: "Swarm loop disconnected".into() };
            }
            push_packet_meta_trace(
                "ComponentOutput", 0,
                &format!("control.UnicastDHT OUTPUT queued target={target_addr}"),
                &chain, "component.output",
            );
            ControlResponse::Ok { msg: format!("UnicastDHT NodeAnnounce to {} queued", target_addr) }
        }
        ControlRequest::SendDummy { size, path } => {
            let chain = next_chain("");
            push_packet_meta_trace(
                "ComponentInput",
                size,
                &format!("control.SendDummy INPUT size={} path={}", size, path.join("->")),
                &chain,
                "component.input",
            );
            match execute_send_dummy(size, path, seed_store, noise_priv_key, &chain).await {
                Ok(msg) => {
                    push_packet_meta_trace(
                        "ComponentOutput",
                        size,
                        &format!("control.SendDummy OUTPUT msg={msg}"),
                        &chain,
                        "component.output",
                    );
                    ControlResponse::Ok { msg }
                }
                Err(e) => {
                    push_packet_meta_trace(
                        "ComponentError",
                        size,
                        &format!("control.SendDummy ERROR err={e:#}"),
                        &chain,
                        "component.error",
                    );
                    ControlResponse::Error { reason: format!("{e:#}") }
                }
            }
        }
    }
}

async fn execute_send_dummy(
    size: usize,
    path: Vec<String>,
    seed_store: &Arc<RwLock<HashMap<SocketAddr, RelayNode>>>,
    noise_priv_key: [u8; 32],
    parent_chain: &str,
) -> Result<String> {
    let mut chain = next_chain(parent_chain);
    push_packet_meta_trace(
        "ComponentInput",
        size,
        &format!("execute_send_dummy INPUT size={} path={}", size, path.join("->")),
        &chain,
        "component.input",
    );

    if path.is_empty() {
        push_packet_meta_trace(
            "ComponentError",
            0,
            "execute_send_dummy ERROR empty path",
            &chain,
            "component.error",
        );
        return Err(anyhow::anyhow!("Direct dummy send to publisher without circuit not currently implemented in this demo"));
    }

    // 1. Build circuit path from Local Seed Store
    chain = next_chain(&chain);
    push_packet_meta_trace(
        "ComponentInput",
        path.len(),
        "execute_send_dummy.lookup_seed_store INPUT",
        &chain,
        "component.input",
    );
    let mut explicit_route = Vec::new();
    {
        let store = seed_store.read().unwrap();
        for hop_str in &path {
            let addr: SocketAddr = hop_str.parse().context(format!("Invalid SocketAddr: {hop_str}"))?;
            if let Some(node) = store.get(&addr) {
                if node.subnet_tag != "HostileSubnet" && node.subnet_tag != "FreeSubnet" {
                    push_packet_meta_trace(
                        "ComponentError",
                        0,
                        &format!(
                            "execute_send_dummy.lookup_seed_store ERROR non_relay_hop={} subnet_tag={}",
                            hop_str, node.subnet_tag
                        ),
                        &chain,
                        "component.error",
                    );
                    anyhow::bail!(
                        "IP {} has subnet_tag '{}' and is not relay-capable (expected HostileSubnet/FreeSubnet)",
                        hop_str,
                        node.subnet_tag
                    );
                }
                explicit_route.push(node.clone());
            } else {
                push_packet_meta_trace(
                    "ComponentError",
                    0,
                    &format!("execute_send_dummy.lookup_seed_store ERROR missing_hop={hop_str}"),
                    &chain,
                    "component.error",
                );
                anyhow::bail!("IP {} not found in local DHT Seed Store", hop_str);
            }
        }
    }
    push_packet_meta_trace(
        "ComponentOutput",
        explicit_route.len(),
        &format!("execute_send_dummy.lookup_seed_store OUTPUT hops={}", explicit_route.len()),
        &chain,
        "component.output",
    );

    if explicit_route.len() < 3 {
        push_packet_meta_trace(
            "ComponentError",
            explicit_route.len(),
            &format!(
                "execute_send_dummy ERROR insufficient_hops={} (need>=3)",
                explicit_route.len()
            ),
            &chain,
            "component.error",
        );
        anyhow::bail!("Circuit requires at least 3 hops (guard, middle, exit), got {}", explicit_route.len());
    }

    // 2. Extract guard, middle, exit
    let exit = explicit_route.pop().unwrap();
    let middle = explicit_route.pop().unwrap();
    let guard = explicit_route.pop().unwrap();

    let cm = CircuitManager::new();

    // 3. Build circuit
    chain = next_chain(&chain);
    push_packet_meta_trace(
        "ComponentInput",
        0,
        &format!(
            "circuit_manager.build_circuit INPUT guard={} middle={} exit={}",
            guard.addr, middle.addr, exit.addr
        ),
        &chain,
        "component.input",
    );
    info!("SendDummy: Building explicit circuit guard={}, middle={}, exit={}", guard.addr, middle.addr, exit.addr);
    let circuit = crate::circuit_manager::build_circuit(
        &noise_priv_key,
        &guard,
        &middle,
        &exit,
        &chain,
    ).await.map_err(|e| {
        push_packet_meta_trace(
            "ComponentError",
            0,
            &format!("circuit_manager.build_circuit ERROR err={e:#}"),
            &chain,
            "component.error",
        );
        e
    })?;
    push_packet_meta_trace(
        "ComponentOutput",
        0,
        "circuit_manager.build_circuit OUTPUT ok",
        &chain,
        "component.output",
    );
    
    cm.add_circuit(circuit).await;

    // 4. Send dummy packet
    chain = next_chain(&chain);
    push_packet_meta_trace(
        "ComponentInput",
        size,
        &format!("circuit_manager.send_chunk INPUT chunk_index=0 size={size}"),
        &chain,
        "component.input",
    );
    let dummy_payload = vec![0x42u8; size]; // "B"s

    cm.send_chunk(0, dummy_payload).await.map_err(|e| {
        push_packet_meta_trace(
            "ComponentError",
            size,
            &format!("circuit_manager.send_chunk ERROR err={e:#}"),
            &chain,
            "component.error",
        );
        e
    })?;
    push_packet_meta_trace(
        "ComponentOutput",
        size,
        &format!("circuit_manager.send_chunk OUTPUT sent_bytes={size}"),
        &chain,
        "component.output",
    );

    info!("SendDummy: Successfully sent {} bytes over explicit circuit.", size);
    push_packet_meta_trace(
        "ComponentOutput",
        size,
        &format!("execute_send_dummy OUTPUT success size={size}"),
        &chain,
        "component.output",
    );
    Ok(format!("Successfully built circuit and sent {} bytes.", size))
}
