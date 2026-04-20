use crate::circuit_manager::{
    bootstrap_snapshot_for_peer, select_default_bootstrap_path, CircuitManager, RelayNode,
    ValidationState,
};
use crate::swarm::SwarmControlCmd;
use anyhow::{Context, Result};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chain_id: Option<String>,
    },
    BroadcastSeed,
    UnicastDHT {
        target_addr: String,
    },
    SendScale {
        chunk_count: usize,
        chunk_size: usize,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ScaleChunkResult {
    pub chunk_index: u32,
    pub chunk_id: u64,
    pub guard_addr: String,
    pub middle_addr: String,
    pub exit_addr: String,
    pub acked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ControlResponse {
    Ok {
        msg: String,
    },
    DhtData {
        store: Vec<RelayNode>,
        kademlia_buckets: Vec<String>,
    },
    Metadata {
        packets: Vec<PacketMeta>,
    },
    Error {
        reason: String,
    },
    TraceId {
        chain_id: String,
    },
    ScaleResult {
        chunks: Vec<ScaleChunkResult>,
        acked: usize,
        total: usize,
        elapsed_ms: u64,
    },
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

const SEND_DUMMY_FRAGMENT_SIZE: usize = 32 * 1024;
const SEND_DUMMY_MAX_ATTEMPTS: usize = 8;
#[cfg(feature = "dht-validation-policy")]
const NODE_VALIDATION_COMPLETE_SCORE: u32 = 20;

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn is_relay_node(node: &RelayNode) -> bool {
    node.subnet_tag == "HostileSubnet" || node.subnet_tag == "FreeSubnet"
}

fn path_key(
    guard: &RelayNode,
    middle: &RelayNode,
    exit: &RelayNode,
) -> (SocketAddr, SocketAddr, SocketAddr) {
    (guard.addr, middle.addr, exit.addr)
}

fn build_candidate_paths(
    explicit_route: Option<(RelayNode, RelayNode, RelayNode)>,
    peers: &[RelayNode],
) -> Vec<(RelayNode, RelayNode, RelayNode)> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();

    if peers.len() < 3 {
        return paths;
    }

    let mut exit_candidates: Vec<RelayNode> = peers
        .iter()
        .filter(|p| p.subnet_tag == "FreeSubnet")
        .cloned()
        .collect();
    if exit_candidates.is_empty() {
        exit_candidates = peers.to_vec();
    }

    let mut add_path = |guard: RelayNode, middle: RelayNode, exit: RelayNode| {
        let key = path_key(&guard, &middle, &exit);
        if seen.insert(key) {
            paths.push((guard, middle, exit));
        }
    };

    if let Some((guard, middle, exit)) = explicit_route {
        if peers.iter().any(|p| p.addr == guard.addr)
            && peers.iter().any(|p| p.addr == middle.addr)
            && peers.iter().any(|p| p.addr == exit.addr)
        {
            if guard.addr != middle.addr && middle.addr != exit.addr && guard.addr != exit.addr {
                add_path(guard, middle, exit);
            }
        }
    }

    for guard in peers {
        for middle in peers {
            if guard.addr == middle.addr {
                continue;
            }
            for exit in &exit_candidates {
                if guard.addr == exit.addr || middle.addr == exit.addr {
                    continue;
                }
                add_path(guard.clone(), middle.clone(), exit.clone());
            }
        }
    }

    paths
}

fn live_relay_nodes(seed_store: &Arc<RwLock<HashMap<SocketAddr, RelayNode>>>) -> Vec<RelayNode> {
    let store = seed_store.read().unwrap();
    let mut peers = store
        .values()
        .filter(|node| is_relay_node(node))
        .cloned()
        .collect::<Vec<_>>();
    #[cfg(feature = "dht-validation-policy")]
    {
        peers.retain(is_path_eligible_node);
        peers.sort_unstable_by(|a, b| {
            let a_state = if a.validation_state == ValidationState::Complete {
                1
            } else {
                0
            };
            let b_state = if b.validation_state == ValidationState::Complete {
                1
            } else {
                0
            };
            b_state
                .cmp(&a_state)
                .then_with(|| b.validation_score.cmp(&a.validation_score))
                .then_with(|| {
                    let a_ts = a.last_direct_seen_ms.unwrap_or(0);
                    let b_ts = b.last_direct_seen_ms.unwrap_or(0);
                    b_ts.cmp(&a_ts)
                })
                .then_with(|| a.addr.cmp(&b.addr))
        });
    }
    #[cfg(not(feature = "dht-validation-policy"))]
    {
        peers.sort_unstable_by_key(|p| p.addr);
    }
    peers
}

#[cfg(feature = "dht-validation-policy")]
fn is_path_eligible_node(node: &RelayNode) -> bool {
    crate::circuit_manager::is_bootstrap_eligible_node(node)
}

#[cfg(feature = "dht-validation-policy")]
fn apply_node_path_result(
    seed_store: &Arc<RwLock<HashMap<SocketAddr, RelayNode>>>,
    addrs: &[SocketAddr],
    acked: bool,
) {
    let mut store = seed_store.write().unwrap();
    for addr in addrs {
        if let Some(node) = store.get_mut(&addr) {
            if acked {
                node.validation_score = node.validation_score.saturating_add(1);
                match node.validation_state {
                    ValidationState::Unvalidated | ValidationState::Direct => {
                        if node.validation_score > NODE_VALIDATION_COMPLETE_SCORE {
                            node.validation_state = ValidationState::Complete;
                        } else {
                            node.validation_state = ValidationState::Direct;
                        }
                    }
                    ValidationState::Complete => {}
                    ValidationState::Isolated => {}
                    ValidationState::PropagatedOnly => {}
                }
            } else {
                node.validation_score = node.validation_score.saturating_sub(1);
                if node.validation_score == 0 {
                    node.validation_state = ValidationState::Isolated;
                }
            }
        }
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
                                    let req_res: Result<ControlRequest, _> =
                                        serde_json::from_str(line.trim());
                                    match req_res {
                                        Ok(req) => {
                                            // For SendDummy: emit chain_id immediately before async work begins.
                                            let pre_chain = match &req {
                                                ControlRequest::SendDummy { .. }
                                                | ControlRequest::SendScale { .. } => {
                                                    let chain = next_chain("");
                                                    let pre_json = serde_json::to_string(
                                                        &ControlResponse::TraceId {
                                                            chain_id: chain.clone(),
                                                        },
                                                    )
                                                    .unwrap_or_default();
                                                    let _ = write
                                                        .write_all(
                                                            format!("{}\n", pre_json).as_bytes(),
                                                        )
                                                        .await;
                                                    let _ = write.flush().await;
                                                    Some(chain)
                                                }
                                                _ => None,
                                            };
                                            let resp = handle_request(
                                                req,
                                                &seed_store,
                                                &swarm_tx,
                                                priv_key,
                                                pre_chain,
                                            )
                                            .await;
                                            let resp_json =
                                                serde_json::to_string(&resp).unwrap_or_default();
                                            if let Err(e) = write
                                                .write_all(format!("{}\n", resp_json).as_bytes())
                                                .await
                                            {
                                                warn!("Failed writing to control client: {e}");
                                                break;
                                            }
                                        }
                                        Err(e) => {
                                            let chain = next_chain("");
                                            push_packet_meta_trace(
                                                "ComponentError",
                                                line.len(),
                                                &format!(
                                                    "control.parse_request ERROR err={e} raw={}",
                                                    line.trim()
                                                ),
                                                &chain,
                                                "component.error",
                                            );
                                            let err = ControlResponse::Error {
                                                reason: format!("Bad request: {e}"),
                                            };
                                            let _ = write
                                                .write_all(
                                                    format!(
                                                        "{}\n",
                                                        serde_json::to_string(&err).unwrap()
                                                    )
                                                    .as_bytes(),
                                                )
                                                .await;
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
    pre_chain: Option<String>,
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
                return ControlResponse::Error {
                    reason: "Swarm loop disconnected".into(),
                };
            }

            let kademlia_buckets = match rx.recv().await {
                Some(buckets) => buckets,
                None => vec!["Failed to fetch Kademlia buckets".into()],
            };

            let mut store: Vec<RelayNode> = seed_store.read().unwrap().values().cloned().collect();
            #[cfg(feature = "dht-validation-policy")]
            {
                store.sort_unstable_by(|a, b| {
                    let rank = |node: &RelayNode| match node.validation_state {
                        ValidationState::Complete => 3,
                        ValidationState::Direct => 2,
                        ValidationState::Unvalidated => 1,
                        ValidationState::PropagatedOnly => 0,
                        ValidationState::Isolated => -1,
                    };
                    rank(b)
                        .cmp(&rank(a))
                        .then_with(|| b.validation_score.cmp(&a.validation_score))
                        .then_with(|| {
                            let a_direct = a.last_direct_seen_ms.unwrap_or(0);
                            let b_direct = b.last_direct_seen_ms.unwrap_or(0);
                            b_direct.cmp(&a_direct)
                        })
                        .then_with(|| b.last_observed_ms.cmp(&a.last_observed_ms))
                });
            }
            #[cfg(not(feature = "dht-validation-policy"))]
            {
                store.sort_unstable_by(|a, b| b.last_seen_ms.cmp(&a.last_seen_ms));
            }
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
        ControlRequest::DumpMetadata { limit, chain_id } => {
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
                    "control.DumpMetadata INPUT limit={:?} effective_limit={} chain_filter={:?}",
                    limit, effective_limit, chain_id
                ),
                &chain,
                "component.input",
            );
            let (packets, total_in_ring): (Vec<PacketMeta>, usize) = {
                let ring = get_ring().lock().unwrap();
                let total = ring.len();
                // ring is newest-first (push_front); take the first N = the N most recent
                let packets = match &chain_id {
                    Some(cid) => ring
                        .iter()
                        .filter(|p| {
                            #[cfg(feature = "distributed-trace")]
                            {
                                p.id_chain.contains(cid.as_str())
                            }
                            #[cfg(not(feature = "distributed-trace"))]
                            {
                                let _ = cid;
                                true
                            }
                        })
                        .take(effective_limit)
                        .cloned()
                        .collect(),
                    None => ring.iter().take(effective_limit).cloned().collect(),
                };
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
            ControlResponse::Ok {
                msg: "CloudMap local seed broadcast queued".to_string(),
            }
        }
        ControlRequest::UnicastDHT { target_addr } => {
            let chain = next_chain("");
            push_packet_meta_trace(
                "ComponentInput",
                0,
                &format!("control.UnicastDHT INPUT target={target_addr}"),
                &chain,
                "component.input",
            );
            if swarm_tx
                .send(SwarmControlCmd::UnicastDHT {
                    target_addr: target_addr.clone(),
                })
                .await
                .is_err()
            {
                push_packet_meta_trace(
                    "ComponentError",
                    0,
                    "control.UnicastDHT ERROR swarm loop disconnected",
                    &chain,
                    "component.error",
                );
                return ControlResponse::Error {
                    reason: "Swarm loop disconnected".into(),
                };
            }
            push_packet_meta_trace(
                "ComponentOutput",
                0,
                &format!("control.UnicastDHT OUTPUT queued target={target_addr}"),
                &chain,
                "component.output",
            );
            ControlResponse::Ok {
                msg: format!("UnicastDHT NodeAnnounce to {} queued", target_addr),
            }
        }
        ControlRequest::SendScale {
            chunk_count,
            chunk_size,
        } => {
            let chain = pre_chain.unwrap_or_else(|| next_chain(""));
            push_packet_meta_trace(
                "ComponentInput",
                chunk_count * chunk_size,
                &format!(
                    "control.SendScale INPUT chunk_count={chunk_count} chunk_size={chunk_size}"
                ),
                &chain,
                "component.input",
            );
            match execute_send_scale(chunk_count, chunk_size, seed_store, noise_priv_key, &chain)
                .await
            {
                Ok(resp) => {
                    if let ControlResponse::ScaleResult {
                        acked,
                        total,
                        elapsed_ms,
                        ..
                    } = &resp
                    {
                        push_packet_meta_trace(
                            "ComponentOutput",
                            acked * chunk_size,
                            &format!(
                                "control.SendScale OUTPUT acked={acked}/{total} elapsed_ms={elapsed_ms}"
                            ),
                            &chain,
                            "component.output",
                        );
                    }
                    resp
                }
                Err(e) => {
                    let reason = format!(
                        "SendScale failed chunk_count={chunk_count} chunk_size={chunk_size} err={e:#}"
                    );
                    push_packet_meta_trace(
                        "ComponentError",
                        0,
                        &format!("control.SendScale ERROR {reason}"),
                        &chain,
                        "component.error",
                    );
                    ControlResponse::Error { reason }
                }
            }
        }
        ControlRequest::SendDummy { size, path } => {
            let chain = pre_chain.unwrap_or_else(|| next_chain(""));
            let path_summary = path.join("->");
            push_packet_meta_trace(
                "ComponentInput",
                size,
                &format!(
                    "control.SendDummy INPUT size={} path={}",
                    size, path_summary
                ),
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
                    let reason = format!(
                        "SendDummy failed stage=execute_send_dummy size={} path={} err={:#}",
                        size, path_summary, e
                    );
                    push_packet_meta_trace(
                        "ComponentError",
                        size,
                        &format!("control.SendDummy ERROR err={reason}"),
                        &chain,
                        "component.error",
                    );
                    ControlResponse::Error { reason }
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
        &format!(
            "execute_send_dummy INPUT size={} path={}",
            size,
            path.join("->")
        ),
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
        return Err(anyhow::anyhow!(
            "Direct dummy send to publisher without circuit not currently implemented in this demo"
        ));
    }

    // 1. Build the user-provided explicit triplet from local seed store.
    chain = next_chain(&chain);
    push_packet_meta_trace(
        "ComponentInput",
        path.len(),
        "execute_send_dummy.lookup_seed_store INPUT",
        &chain,
        "component.input",
    );
    let mut explicit_triplet = Vec::new();
    {
        let store = seed_store.read().unwrap();
        for hop_str in &path {
            let addr: SocketAddr = hop_str
                .parse()
                .context(format!("Invalid SocketAddr: {hop_str}"))?;
            if let Some(node) = store.get(&addr) {
                if !is_relay_node(node) {
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
                explicit_triplet.push(node.clone());
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
        explicit_triplet.len(),
        &format!(
            "execute_send_dummy.lookup_seed_store OUTPUT hops={}",
            explicit_triplet.len()
        ),
        &chain,
        "component.output",
    );

    if explicit_triplet.len() < 3 {
        push_packet_meta_trace(
            "ComponentError",
            explicit_triplet.len(),
            &format!(
                "execute_send_dummy ERROR insufficient_hops={} (need>=3)",
                explicit_triplet.len()
            ),
            &chain,
            "component.error",
        );
        anyhow::bail!(
            "Circuit requires at least 3 hops (guard, middle, exit), got {}",
            explicit_triplet.len()
        );
    }

    let exit = explicit_triplet.pop().unwrap();
    let middle = explicit_triplet.pop().unwrap();
    let guard = explicit_triplet.pop().unwrap();
    let explicit_route = (guard, middle, exit);

    let (publisher_addr, publisher_pub_key) = crate::swarm::discover_publisher_static()
        .await
        .context("SendDummy: failed to discover Publisher static endpoint")?;

    let dummy_payload = vec![0x42u8; size]; // "B"s
    let total_chunks = dummy_payload.len().div_ceil(SEND_DUMMY_FRAGMENT_SIZE) as u32;
    let transfer_chunk_id = now_millis();
    let transfer_chain = next_chain(&chain);
    push_packet_meta_trace(
        "ComponentInput",
        size,
        &format!(
            "circuit_manager.send_chunk INPUT chunk_id={} total_chunks={} size={} fragment_size={}",
            transfer_chunk_id, total_chunks, size, SEND_DUMMY_FRAGMENT_SIZE
        ),
        &transfer_chain,
        "component.input",
    );

    for (chunk_index, fragment) in dummy_payload.chunks(SEND_DUMMY_FRAGMENT_SIZE).enumerate() {
        send_chunk_with_retries(
            &noise_priv_key,
            seed_store,
            Some(&explicit_route),
            publisher_addr,
            publisher_pub_key,
            transfer_chain.clone(),
            transfer_chunk_id,
            chunk_index as u32,
            total_chunks,
            fragment.to_vec(),
        )
        .await
        .with_context(|| {
            format!(
                "SendDummy send_chunk failed size={} chunk_index={} total_chunks={} chunk_id={}",
                size, chunk_index, total_chunks, transfer_chunk_id
            )
        })
        .map_err(|e| {
            push_packet_meta_trace(
                "ComponentError",
                fragment.len(),
                &format!(
                    "circuit_manager.send_chunk ERROR chunk_id={} chunk_index={} total_chunks={} err={e:#}",
                    transfer_chunk_id, chunk_index, total_chunks
                ),
                &transfer_chain,
                "component.error",
            );
            e
        })?;
        push_packet_meta_trace(
            "ComponentOutput",
            fragment.len(),
            &format!(
                "circuit_manager.send_chunk OUTPUT sent_bytes={} chunk_id={} chunk_index={} total_chunks={}",
                fragment.len(),
                transfer_chunk_id,
                chunk_index,
                total_chunks
            ),
            &transfer_chain,
            "component.output",
        );
    }
    push_packet_meta_trace(
        "ComponentOutput",
        size,
        &format!(
            "circuit_manager.send_chunk OUTPUT sent_bytes={} chunk_id={} total_chunks={}",
            size, transfer_chunk_id, total_chunks
        ),
        &transfer_chain,
        "component.output",
    );

    info!(
        "SendDummy: Successfully sent {} bytes over explicit circuit in {} chunk(s), chunk_id={}.",
        size, total_chunks, transfer_chunk_id
    );
    push_packet_meta_trace(
        "ComponentOutput",
        size,
        &format!(
            "execute_send_dummy OUTPUT success size={} chunk_id={} total_chunks={}",
            size, transfer_chunk_id, total_chunks
        ),
        &transfer_chain,
        "component.output",
    );
    Ok(format!(
        "Successfully built circuit and sent {} bytes across {} chunk(s).",
        size, total_chunks
    ))
}

async fn send_chunk_with_retries(
    noise_priv_key: &[u8; 32],
    seed_store: &Arc<RwLock<HashMap<SocketAddr, RelayNode>>>,
    explicit_path: Option<&(RelayNode, RelayNode, RelayNode)>,
    publisher_addr: SocketAddr,
    publisher_pub_key: [u8; 32],
    transfer_chain: String,
    chunk_id: u64,
    chunk_index: u32,
    total_chunks: u32,
    fragment: Vec<u8>,
) -> Result<()> {
    if explicit_path.is_none() {
        anyhow::bail!("SendDummy retry logic requires an explicit first path");
    }

    let mut attempts = 0usize;
    let mut attempted = HashSet::new();
    let explicit = explicit_path.cloned();
    let mut last_error: Option<String> = None;

    while attempts < SEND_DUMMY_MAX_ATTEMPTS {
        let chain_attempt = next_chain(&transfer_chain);
        let peers = live_relay_nodes(seed_store);
        if peers.is_empty() {
            anyhow::bail!("No relay peers found in local DHT seed store");
        }

        let candidate_paths = build_candidate_paths(explicit.clone(), &peers);
        if candidate_paths.is_empty() {
            anyhow::bail!("No valid relay triplets available in local DHT");
        }

        let mut selected: Option<(RelayNode, RelayNode, RelayNode)> = None;
        for path in &candidate_paths {
            if !attempted.contains(&path_key(&path.0, &path.1, &path.2)) {
                selected = Some(path.clone());
                break;
            }
        }
        let used_recycled = selected.is_none();
        if selected.is_none() {
            let reuse_idx = if candidate_paths.is_empty() {
                0
            } else {
                attempts % candidate_paths.len()
            };
            selected = candidate_paths.get(reuse_idx).cloned();
        }

        let Some((guard, middle, exit)) = selected else {
            anyhow::bail!("Unable to select relay path");
        };

        let new_key = path_key(&guard, &middle, &exit);
        if !used_recycled {
            attempted.insert(new_key);
        }

        attempts += 1;
        let path = format!(
            "guard={} middle={} exit={}",
            guard.addr, middle.addr, exit.addr
        );
        push_packet_meta_trace(
            "ChunkRouteSelected",
            0,
            &format!(
                "execute_send_dummy.route attempt={} path={} recycled={}",
                attempts, path, used_recycled
            ),
            &chain_attempt,
            "circuit.route",
        );

        let chunk_result = crate::circuit_manager::send_chunk_via_path(
            noise_priv_key,
            &guard,
            &middle,
            &exit,
            publisher_addr,
            publisher_pub_key,
            chunk_id,
            chunk_index,
            total_chunks,
            Some(&transfer_chain),
            fragment.clone(),
        )
        .await;

        if let Err(error) = chunk_result {
            #[cfg(feature = "dht-validation-policy")]
            apply_node_path_result(seed_store, &[guard.addr, middle.addr, exit.addr], false);
            last_error = Some(error.to_string());
            push_packet_meta_trace(
                "ComponentError",
                fragment.len(),
                &format!(
                    "circuit_manager.send_chunk ERROR attempt={} path={} guard={} middle={} exit={} err={error:#}",
                    attempts, path, guard.addr, middle.addr, exit.addr
                ),
                &chain_attempt,
                "circuit.error",
            );
            continue;
        }

        #[cfg(feature = "dht-validation-policy")]
        apply_node_path_result(seed_store, &[guard.addr, middle.addr, exit.addr], true);
        push_packet_meta_trace(
            "ComponentOutput",
            fragment.len(),
            &format!(
                "circuit_manager.send_chunk OUTPUT sent_bytes={} chunk_id={} chunk_index={} total_chunks={} attempt={}",
                fragment.len(),
                chunk_id,
                chunk_index,
                total_chunks,
                attempts
            ),
            &chain_attempt,
            "circuit.send",
        );
        return Ok(());
    }

    anyhow::bail!(
        "Failed to send chunk after {} attempts: {}",
        attempts,
        last_error.unwrap_or_else(|| "unknown error".to_string())
    )
}

pub(crate) async fn bootstrap_validate_unvalidated_node(
    seed_store: &Arc<RwLock<HashMap<SocketAddr, RelayNode>>>,
    noise_priv_key: [u8; 32],
    guard_addr: SocketAddr,
    parent_chain: &str,
) -> Result<bool> {
    let (guard, middle, exit) = {
        let store = seed_store.read().unwrap();
        let all_peers: Vec<RelayNode> = store
            .values()
            .filter(|node| node.validation_state != ValidationState::Isolated)
            .cloned()
            .collect();
        let Some(guard) = store.get(&guard_addr).cloned() else {
            return Ok(false);
        };
        if guard.validation_state != ValidationState::Unvalidated || guard.validation_score == 0 {
            return Ok(false);
        }
        let Some((middle, exit)) = select_default_bootstrap_path(&all_peers, guard_addr) else {
            return Ok(false);
        };
        (guard, middle, exit)
    };

    let chain = next_chain(parent_chain);
    let payload_len = rand::thread_rng().gen_range(256..=2048);
    let payload = vec![0x42u8; payload_len];
    let chunk_id = now_millis();

    push_packet_meta_trace(
        "ComponentInput",
        payload_len,
        &format!(
            "bootstrap.validate INPUT guard={} middle={} exit={} size={}",
            guard.addr, middle.addr, exit.addr, payload_len
        ),
        &chain,
        "bootstrap.validate",
    );

    let (publisher_addr, publisher_pub_key) = crate::swarm::discover_publisher_static()
        .await
        .context("Bootstrap validation: failed to discover Publisher static endpoint")?;

    let send_result = crate::circuit_manager::send_chunk_via_path(
        &noise_priv_key,
        &guard,
        &middle,
        &exit,
        publisher_addr,
        publisher_pub_key,
        chunk_id,
        0,
        1,
        Some(&chain),
        payload,
    )
    .await;

    match send_result {
        Ok(()) => {
            apply_node_path_result(seed_store, &[guard.addr, middle.addr, exit.addr], true);
            push_packet_meta_trace(
                "ComponentOutput",
                payload_len,
                &format!(
                    "bootstrap.validate OUTPUT promoted guard={} middle={} exit={}",
                    guard.addr, middle.addr, exit.addr
                ),
                &chain,
                "bootstrap.validate",
            );
            Ok(true)
        }
        Err(error) => {
            push_packet_meta_trace(
                "ComponentError",
                payload_len,
                &format!(
                    "bootstrap.validate ERROR guard={} middle={} exit={} err={error:#}",
                    guard.addr, middle.addr, exit.addr
                ),
                &chain,
                "bootstrap.error",
            );
            Err(error)
        }
    }
}

pub(crate) fn minimal_bootstrap_snapshot(
    seed_store: &Arc<RwLock<HashMap<SocketAddr, RelayNode>>>,
    target_addr: Option<SocketAddr>,
    target_validation_state: Option<&ValidationState>,
    full_limit: usize,
) -> Vec<RelayNode> {
    let store = seed_store.read().unwrap();
    let all_peers: Vec<RelayNode> = store.values().cloned().collect();
    bootstrap_snapshot_for_peer(&all_peers, target_validation_state, target_addr, full_limit)
}

async fn execute_send_scale(
    chunk_count: usize,
    chunk_size: usize,
    seed_store: &Arc<RwLock<HashMap<SocketAddr, RelayNode>>>,
    noise_priv_key: [u8; 32],
    parent_chain: &str,
) -> Result<ControlResponse> {
    let chain = next_chain(parent_chain);

    let all_peers: Vec<RelayNode> = {
        let store = seed_store.read().unwrap();
        #[cfg(feature = "dht-validation-policy")]
        {
            store
                .values()
                .filter(|node| node.validation_state != ValidationState::Isolated)
                .cloned()
                .collect()
        }
        #[cfg(not(feature = "dht-validation-policy"))]
        {
            store.values().cloned().collect()
        }
    };

    if all_peers.is_empty() {
        anyhow::bail!("DHT is empty — gossip has not populated any peers yet");
    }

    push_packet_meta_trace(
        "ComponentInput",
        0,
        &format!(
            "execute_send_scale INPUT chunk_count={chunk_count} chunk_size={chunk_size} peers={}",
            all_peers.len()
        ),
        &chain,
        "component.input",
    );

    let (publisher_addr, publisher_pub_key) = crate::swarm::discover_publisher_static()
        .await
        .context("SendScale: failed to discover Publisher static endpoint")?;

    let (outcomes, elapsed_ms) = crate::circuit_manager::send_scale_multipath(
        &noise_priv_key,
        all_peers,
        publisher_addr,
        publisher_pub_key,
        chunk_count,
        chunk_size,
        &chain,
    )
    .await?;

    let total = outcomes.len();
    let acked = outcomes.iter().filter(|o| o.acked).count();

    #[cfg(feature = "dht-validation-policy")]
    for outcome in &outcomes {
        let middle_is_unvalidated = seed_store
            .read()
            .unwrap()
            .get(&outcome.middle_addr)
            .map(|node| node.validation_state == ValidationState::Unvalidated)
            .unwrap_or(false);
        let shared_pair_validated_elsewhere = outcomes.iter().any(|other| {
            other.chunk_index != outcome.chunk_index
                && other.guard_addr == outcome.guard_addr
                && other.exit_addr == outcome.exit_addr
                && other.middle_addr != outcome.middle_addr
                && other.acked
        });

        if !outcome.acked && middle_is_unvalidated && shared_pair_validated_elsewhere {
            apply_node_path_result(seed_store, &[outcome.middle_addr], false);
        } else {
            apply_node_path_result(
                seed_store,
                &[outcome.guard_addr, outcome.middle_addr, outcome.exit_addr],
                outcome.acked,
            );
        }
    }

    let chunks: Vec<ScaleChunkResult> = outcomes
        .into_iter()
        .map(|o| ScaleChunkResult {
            chunk_index: o.chunk_index,
            chunk_id: o.chunk_id,
            guard_addr: o.guard_addr.to_string(),
            middle_addr: o.middle_addr.to_string(),
            exit_addr: o.exit_addr.to_string(),
            acked: o.acked,
            error: o.error_msg,
        })
        .collect();

    Ok(ControlResponse::ScaleResult {
        chunks,
        acked,
        total,
        elapsed_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit_manager::ValidationState;

    fn test_node(addr: &str, tag: &str, state: ValidationState, score: u32) -> RelayNode {
        RelayNode {
            addr: addr.parse().unwrap(),
            identity_pub: [addr.as_bytes()[0]; 32],
            subnet_tag: tag.to_string(),
            announce_ts_ms: 1,
            last_direct_seen_ms: Some(1),
            last_propagated_seen_ms: None,
            last_observed_ms: 1,
            validation_state: state,
            validation_score: score,
            last_seen_ms: 1,
        }
    }

    #[tokio::test]
    async fn bootstrap_validation_skips_direct_guards() {
        let guard = test_node(
            "127.0.0.1:9101",
            "HostileSubnet",
            ValidationState::Direct,
            25,
        );
        let middle = test_node(
            "127.0.0.1:9102",
            "HostileSubnet",
            ValidationState::Complete,
            30,
        );
        let exit = test_node(
            "127.0.0.1:9201",
            "FreeSubnet",
            ValidationState::Complete,
            35,
        );
        let store = Arc::new(RwLock::new(HashMap::from([
            (guard.addr, guard.clone()),
            (middle.addr, middle),
            (exit.addr, exit),
        ])));

        let promoted = bootstrap_validate_unvalidated_node(&store, [0u8; 32], guard.addr, "")
            .await
            .unwrap();

        assert!(!promoted);
    }

    #[tokio::test]
    async fn bootstrap_validation_skips_zero_score_guards() {
        let guard = test_node(
            "127.0.0.1:9301",
            "HostileSubnet",
            ValidationState::Unvalidated,
            0,
        );
        let middle = test_node(
            "127.0.0.1:9302",
            "HostileSubnet",
            ValidationState::Complete,
            30,
        );
        let exit = test_node(
            "127.0.0.1:9401",
            "FreeSubnet",
            ValidationState::Complete,
            35,
        );
        let store = Arc::new(RwLock::new(HashMap::from([
            (guard.addr, guard.clone()),
            (middle.addr, middle),
            (exit.addr, exit),
        ])));

        let promoted = bootstrap_validate_unvalidated_node(&store, [0u8; 32], guard.addr, "")
            .await
            .unwrap();

        assert!(!promoted);
    }
}
