//! # Circuit Manager
//!
//! Creator-side onion sender:
//! - Maintains a set of relay paths.
//! - Builds full asymmetric onion layers upfront per chunk.
//! - Sends only the outermost frame to Guard.
//! - Verifies reverse ACK at the Creator after relays peel return layers.

use anyhow::{Context, Result};
use gbn_protocol::dht::RelayDescriptor;
use gbn_protocol::onion::{AckPayload, ChunkPayload, HopInfo, OnionLayer};
use mcn_crypto::noise::{open, seal};
use mcn_crypto::x25519_pubkey_from_privkey;
use rand::seq::SliceRandom;
use rand::thread_rng;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{net::TcpStream, sync::Mutex, time::timeout};

use crate::control::push_packet_meta_trace;
use crate::observability::publish_circuit_build_result_from_env;
use crate::relay_engine::{read_raw_frame, write_raw_frame};

pub type ChunkBytes = Vec<u8>;

/// Relay descriptor used in local DHT/seed-store views.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RelayNode {
    pub addr: SocketAddr,
    pub identity_pub: [u8; 32],
    pub subnet_tag: String,
    /// Unix timestamp (ms) when this node was last seen by the local seed store.
    /// Set on every insert/update; not propagated over gossip.
    #[serde(default)]
    pub last_seen_ms: u64,
}

pub fn relay_node_from_descriptor(descriptor: &RelayDescriptor) -> RelayNode {
    RelayNode {
        addr: descriptor.address,
        identity_pub: descriptor.identity_key,
        subnet_tag: descriptor.subnet_tag.clone(),
        last_seen_ms: 0,
    }
}

pub fn relay_nodes_from_descriptors(descriptors: &[RelayDescriptor]) -> Vec<RelayNode> {
    descriptors.iter().map(relay_node_from_descriptor).collect()
}

#[derive(Clone, Debug)]
pub struct OnionCircuit {
    /// Ordered forward path: [Guard, Middle, Exit]
    pub path: Vec<HopInfo>,
    pub guard_addr: SocketAddr,
    pub middle_addr: SocketAddr,
    pub exit_addr: SocketAddr,
    pub publisher_addr: SocketAddr,
    creator_priv_key: [u8; 32],
    creator_info: HopInfo,
    publisher_info: HopInfo,
    trace_id: String,
}

fn relay_to_hop(node: &RelayNode) -> HopInfo {
    HopInfo {
        addr: node.addr,
        identity_pub: node.identity_pub,
    }
}

fn creator_ack_addr_from_env() -> SocketAddr {
    std::env::var("GBN_CREATOR_ACK_ADDR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0))
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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

fn hop_stage_label(idx: usize, total_hops: usize) -> String {
    if idx == 0 {
        "guard".to_string()
    } else if idx + 1 == total_hops {
        "exit".to_string()
    } else if total_hops == 3 && idx == 1 {
        "middle".to_string()
    } else {
        format!("middle[{}]", idx)
    }
}

/// Build an onion path descriptor (no interactive circuit handshake).
pub async fn build_circuit(
    creator_priv_key: &[u8; 32],
    guard: &RelayNode,
    middle: &RelayNode,
    exit: &RelayNode,
    publisher_addr: SocketAddr,
    publisher_pub_key: [u8; 32],
    trace_id: &str,
) -> Result<OnionCircuit> {
    let started = tokio::time::Instant::now();
    let base_trace = if trace_id.is_empty() {
        next_chain("")
    } else {
        trace_id.to_string()
    };
    let build_input_chain = next_chain(&base_trace);
    push_packet_meta_trace(
        "ComponentInput",
        0,
        &format!(
            "circuit.build INPUT guard={} middle={} exit={}",
            guard.addr, middle.addr, exit.addr
        ),
        &build_input_chain,
        "circuit.build",
    );

    let result: Result<OnionCircuit> = async {
        anyhow::ensure!(
            guard.addr != middle.addr && middle.addr != exit.addr && guard.addr != exit.addr,
            "Circuit hops must be unique (guard={}, middle={}, exit={})",
            guard.addr,
            middle.addr,
            exit.addr
        );

        let creator_info = HopInfo {
            addr: creator_ack_addr_from_env(),
            identity_pub: x25519_pubkey_from_privkey(creator_priv_key),
        };

        Ok(OnionCircuit {
            path: vec![relay_to_hop(guard), relay_to_hop(middle), relay_to_hop(exit)],
            guard_addr: guard.addr,
            middle_addr: middle.addr,
            exit_addr: exit.addr,
            publisher_addr,
            creator_priv_key: *creator_priv_key,
            creator_info,
            publisher_info: HopInfo {
                addr: publisher_addr,
                identity_pub: publisher_pub_key,
            },
            trace_id: base_trace.clone(),
        })
    }
    .await;

    let latency_ms = started.elapsed().as_millis();
    match &result {
        Ok(_) => push_packet_meta_trace(
            "ComponentOutput",
            0,
            &format!(
                "circuit.build OUTPUT ok guard={} middle={} exit={} latency_ms={}",
                guard.addr, middle.addr, exit.addr, latency_ms
            ),
            &next_chain(&build_input_chain),
            "circuit.build",
        ),
        Err(e) => push_packet_meta_trace(
            "ComponentError",
            0,
            &format!(
                "circuit.build ERROR guard={} middle={} exit={} err={e:#}",
                guard.addr, middle.addr, exit.addr
            ),
            &next_chain(&build_input_chain),
            "circuit.error",
        ),
    }
    publish_circuit_build_result_from_env(result.is_ok(), latency_ms).await;
    result
}

fn seal_layer_for_hop(
    hop: &HopInfo,
    next_hop: Option<SocketAddr>,
    inner: Vec<u8>,
    trace_id: &str,
    stage: &str,
) -> Result<Vec<u8>> {
    let inner_len = inner.len();
    let layer = OnionLayer {
        next_hop,
        inner,
        trace_id: if trace_id.is_empty() {
            None
        } else {
            Some(trace_id.to_string())
        },
    };
    let bytes = serde_json::to_vec(&layer).with_context(|| {
        format!(
            "Failed serializing onion layer stage={} hop={} next_hop={:?} inner_bytes={}",
            stage, hop.addr, next_hop, inner_len
        )
    })?;
    let plaintext_len = bytes.len();
    seal(&hop.identity_pub, &bytes).with_context(|| {
        format!(
            "Failed sealing onion layer stage={} hop={} next_hop={:?} plaintext_bytes={} inner_bytes={}",
            stage, hop.addr, next_hop, plaintext_len, inner_len
        )
    })
}

async fn send_chunk_via_circuit(
    circuit: &OnionCircuit,
    chunk_id: u64,
    chunk_index: u32,
    total_chunks: u32,
    transfer_trace_root: Option<&str>,
    payload: ChunkBytes,
) -> Result<()> {
    anyhow::ensure!(
        !circuit.path.is_empty(),
        "Cannot send chunk: empty onion path"
    );

    let payload_len = payload.len();
    let hash = *blake3::hash(&payload).as_bytes();
    let send_timestamp_ms = now_millis();
    let chunk_trace_base = match transfer_trace_root {
        Some(root) if !root.is_empty() => root.to_string(),
        _ => next_chain(&circuit.trace_id),
    };
    let send_input_chain = next_chain(&chunk_trace_base);
    push_packet_meta_trace(
        "ComponentInput",
        payload_len,
        &format!(
            "circuit.send_chunk INPUT chunk_id={} chunk_index={} total_chunks={} guard={} middle={} exit={}",
            chunk_id, chunk_index, total_chunks, circuit.guard_addr, circuit.middle_addr, circuit.exit_addr
        ),
        &send_input_chain,
        "circuit.send",
    );

    let mut return_path = Vec::with_capacity(circuit.path.len() + 1);
    return_path.push(circuit.creator_info.clone());
    return_path.extend(circuit.path.clone());
    return_path.push(circuit.publisher_info.clone());

    let mut hop_layer_traces = Vec::with_capacity(circuit.path.len());
    let mut trace_cursor = chunk_trace_base.clone();
    for _ in &circuit.path {
        trace_cursor = next_chain(&trace_cursor);
        hop_layer_traces.push(trace_cursor.clone());
    }
    let payload_trace = next_chain(&trace_cursor);

    let terminal_payload = ChunkPayload {
        chunk_id,
        hash,
        chunk: payload,
        return_path,
        trace_id: Some(payload_trace.clone()),
        send_timestamp_ms,
        total_chunks,
        chunk_index,
    };
    let terminal_payload_bytes = serde_json::to_vec(&terminal_payload).with_context(|| {
        format!(
            "Failed serializing terminal payload chunk_id={} chunk_index={} bytes={} return_hops={} guard={} middle={} exit={}",
            chunk_id,
            chunk_index,
            payload_len,
            terminal_payload.return_path.len(),
            circuit.guard_addr,
            circuit.middle_addr,
            circuit.exit_addr
        )
    })?;

    let publisher_stage = "publisher";
    let mut sealed = seal_layer_for_hop(
        &circuit.publisher_info,
        None,
        terminal_payload_bytes,
        &payload_trace,
        publisher_stage,
    )
    .map_err(|e| {
        push_packet_meta_trace(
            "ComponentError",
            payload_len,
            &format!(
                "circuit.send_chunk ERROR stage={} hop={} chunk_id={} err={e:#}",
                publisher_stage, circuit.publisher_addr, chunk_id
            ),
            &next_chain(&send_input_chain),
            "circuit.error",
        );
        e
    })?;

    for idx in (0..circuit.path.len()).rev() {
        let hop = &circuit.path[idx];
        let next_addr = if idx + 1 < circuit.path.len() {
            circuit.path[idx + 1].addr
        } else {
            circuit.publisher_addr
        };
        let stage = hop_stage_label(idx, circuit.path.len());
        sealed = seal_layer_for_hop(
            hop,
            Some(next_addr),
            sealed,
            hop_layer_traces.get(idx).map(String::as_str).unwrap_or(""),
            &stage,
        )
        .map_err(|e| {
            push_packet_meta_trace(
                "ComponentError",
                payload_len,
                &format!(
                    "circuit.send_chunk ERROR stage={} hop={} next_hop={} chunk_id={} err={e:#}",
                    stage, hop.addr, next_addr, chunk_id
                ),
                &next_chain(&send_input_chain),
                "circuit.error",
            );
            e
        })?;
    }

    let guard_addr = circuit.path[0].addr;
    let mut stream = timeout(Duration::from_secs(10), TcpStream::connect(guard_addr))
        .await
        .context(format!("Timeout connecting to Guard {}", guard_addr))?
        .context(format!("Failed connecting to Guard {}", guard_addr))
        .map_err(|e| {
            push_packet_meta_trace(
                "ComponentError",
                payload_len,
                &format!(
                    "circuit.send_chunk ERROR connect_guard={} chunk_id={} err={e:#}",
                    guard_addr, chunk_id
                ),
                &next_chain(&send_input_chain),
                "circuit.error",
            );
            e
        })?;

    write_raw_frame(&mut stream, &sealed)
        .await
        .context("Failed writing outer onion frame to Guard")
        .map_err(|e| {
            push_packet_meta_trace(
                "ComponentError",
                payload_len,
                &format!(
                    "circuit.send_chunk ERROR write_guard={} chunk_id={} err={e:#}",
                    guard_addr, chunk_id
                ),
                &next_chain(&send_input_chain),
                "circuit.error",
            );
            e
        })?;

    let ack_frame = timeout(Duration::from_secs(45), read_raw_frame(&mut stream))
        .await
        .context("Timeout waiting for reverse ACK frame")?
        .context("Failed reading reverse ACK frame")
        .map_err(|e| {
            push_packet_meta_trace(
                "ComponentError",
                payload_len,
                &format!(
                    "circuit.send_chunk ERROR read_ack guard={} chunk_id={} err={e:#}",
                    guard_addr, chunk_id
                ),
                &next_chain(&send_input_chain),
                "circuit.error",
            );
            e
        })?;

    let ack_open = open(&circuit.creator_priv_key, &ack_frame)
        .context("Failed opening creator ACK layer")
        .map_err(|e| {
            push_packet_meta_trace(
                "ComponentError",
                payload_len,
                &format!(
                    "circuit.send_chunk ERROR open_ack chunk_id={} err={e:#}",
                    chunk_id
                ),
                &next_chain(&send_input_chain),
                "circuit.error",
            );
            e
        })?;
    let ack_layer: OnionLayer =
        serde_json::from_slice(&ack_open)
            .context("Failed decoding creator ACK OnionLayer")
            .map_err(|e| {
                push_packet_meta_trace(
                    "ComponentError",
                    payload_len,
                    &format!(
                        "circuit.send_chunk ERROR decode_ack_layer chunk_id={} err={e:#}",
                        chunk_id
                    ),
                    &next_chain(&send_input_chain),
                    "circuit.error",
                );
                e
            })?;
    anyhow::ensure!(
        ack_layer.next_hop.is_none(),
        "Creator ACK layer expected next_hop=None, got {:?}",
        ack_layer.next_hop
    );

    let ack: AckPayload =
        serde_json::from_slice(&ack_layer.inner).context("Failed decoding AckPayload")?;
    if ack.chunk_id != chunk_id {
        let msg = format!(
            "ACK chunk_id mismatch: expected {}, got {}",
            chunk_id, ack.chunk_id
        );
        push_packet_meta_trace(
            "ComponentError",
            payload_len,
            &format!("circuit.send_chunk ERROR {}", msg),
            &next_chain(&send_input_chain),
            "circuit.error",
        );
        anyhow::bail!(msg);
    }
    if ack.hash != hash {
        let msg = format!(
            "ACK hash mismatch: expected {}, got {}",
            hex::encode(hash),
            hex::encode(ack.hash)
        );
        push_packet_meta_trace(
            "ComponentError",
            payload_len,
            &format!("circuit.send_chunk ERROR {}", msg),
            &next_chain(&send_input_chain),
            "circuit.error",
        );
        anyhow::bail!(msg);
    }
    if ack.chunk_index != chunk_index {
        let msg = format!(
            "ACK chunk_index mismatch: expected {}, got {}",
            chunk_index, ack.chunk_index
        );
        push_packet_meta_trace(
            "ComponentError",
            payload_len,
            &format!("circuit.send_chunk ERROR {}", msg),
            &next_chain(&send_input_chain),
            "circuit.error",
        );
        anyhow::bail!(msg);
    }

    let ack_chain = ack
        .trace_id
        .clone()
        .or_else(|| ack_layer.trace_id.clone())
        .unwrap_or_else(|| next_chain(&chunk_trace_base));

    tracing::info!(
        "ACK received for chunk_id={} via guard={} middle={} exit={} trace_id={}",
        chunk_id,
        circuit.guard_addr,
        circuit.middle_addr,
        circuit.exit_addr,
        ack_chain
    );
    push_packet_meta_trace(
        "ComponentOutput",
        payload_len,
        &format!(
            "circuit.send_chunk OUTPUT ack chunk_id={} chunk_index={} total_chunks={} guard={} middle={} exit={}",
            chunk_id, chunk_index, total_chunks, circuit.guard_addr, circuit.middle_addr, circuit.exit_addr
        ),
        &next_chain(&ack_chain),
        "circuit.send",
    );
    Ok(())
}

/// Manages a pool of pre-selected onion paths.
pub struct CircuitManager {
    circuits: Arc<Mutex<Vec<OnionCircuit>>>,
    used_guards: Arc<Mutex<HashSet<SocketAddr>>>,
}

impl CircuitManager {
    pub fn new() -> Self {
        Self {
            circuits: Arc::new(Mutex::new(Vec::new())),
            used_guards: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub async fn add_circuit(&self, circuit: OnionCircuit) {
        self.used_guards.lock().await.insert(circuit.guard_addr);
        self.circuits.lock().await.push(circuit);
    }

    /// Send a payload through one path (round-robin by chunk index modulo path count).
    pub async fn send_chunk(&self, chunk_index: u32, payload: ChunkBytes) -> Result<()> {
        self.send_chunk_with_meta(chunk_index as u64, chunk_index, 0, None, payload)
            .await
    }

    pub async fn send_chunk_with_meta(
        &self,
        chunk_id: u64,
        chunk_index: u32,
        total_chunks: u32,
        transfer_trace_root: Option<&str>,
        payload: ChunkBytes,
    ) -> Result<()> {
        let circuits = self.circuits.lock().await;
        if circuits.is_empty() {
            anyhow::bail!("No active circuits available for chunk {}", chunk_index);
        }

        let idx = chunk_index as usize % circuits.len();
        let circuit = circuits[idx].clone();
        drop(circuits);

        send_chunk_via_circuit(
            &circuit,
            chunk_id,
            chunk_index,
            total_chunks,
            transfer_trace_root,
            payload,
        )
        .await
    }

    pub async fn ack_chunk(&self, _chunk_index: u32) {}

    pub async fn drain_failures(&self) -> Vec<(u32, ChunkBytes)> {
        Vec::new()
    }

    pub async fn process_failures_with_rebuild(
        &self,
        _creator_priv_key: &[u8; 32],
        _all_peers: &[RelayNode],
        _exit_candidates: &[RelayNode],
    ) -> Result<usize> {
        Ok(0)
    }

    pub async fn process_failures_with_rebuild_from_descriptors(
        &self,
        _creator_priv_key: &[u8; 32],
        _descriptors: &[RelayDescriptor],
    ) -> Result<usize> {
        Ok(0)
    }
}

pub fn select_exit_candidates(all_peers: &[RelayNode]) -> Vec<RelayNode> {
    all_peers
        .iter()
        .filter(|p| p.subnet_tag == "FreeSubnet")
        .cloned()
        .collect()
}

pub fn select_exit_candidates_from_descriptors(descriptors: &[RelayDescriptor]) -> Vec<RelayNode> {
    let all_peers = relay_nodes_from_descriptors(descriptors);
    select_exit_candidates(&all_peers)
}

/// Verify that all circuits use disjoint relay sets (no relay IP appears twice).
pub fn log_path_diversity(circuits: &[OnionCircuit]) -> bool {
    let mut all_addrs: Vec<SocketAddr> = Vec::with_capacity(circuits.len() * 3);
    for (i, c) in circuits.iter().enumerate() {
        tracing::info!(
            "Circuit {}: guard={} middle={} exit={}",
            i,
            c.guard_addr,
            c.middle_addr,
            c.exit_addr
        );
        all_addrs.push(c.guard_addr);
        all_addrs.push(c.middle_addr);
        all_addrs.push(c.exit_addr);
    }

    let unique: HashSet<_> = all_addrs.iter().collect();
    let ok = unique.len() == all_addrs.len();
    tracing::info!(
        "Path diversity: {} (unique={}/{})",
        if ok { "PASS" } else { "FAIL" },
        unique.len(),
        all_addrs.len()
    );
    ok
}

pub async fn build_circuits_speculative(
    creator_priv_key: &[u8; 32],
    all_peers: &[RelayNode],
    exit_candidates: &[RelayNode],
    publisher_addr: SocketAddr,
    publisher_pub_key: [u8; 32],
    target_count: usize,
    max_concurrent: usize,
) -> Result<Vec<OnionCircuit>> {
    if target_count == 0 {
        return Ok(Vec::new());
    }

    let candidates = enumerate_speculative_candidates(all_peers, exit_candidates, max_concurrent);
    if candidates.is_empty() {
        anyhow::bail!("No candidate relay triplets available for speculative path selection");
    }

    let mut winners = Vec::new();
    let mut used_relay_addrs = HashSet::new();

    for (guard, middle, exit) in candidates {
        let candidate_trace = format!(
            "{} [speculative guard={} middle={} exit={}]",
            next_chain(""),
            guard.addr,
            middle.addr,
            exit.addr
        );
        let candidate =
            build_circuit(
                creator_priv_key,
                &guard,
                &middle,
                &exit,
                publisher_addr,
                publisher_pub_key,
                &candidate_trace,
            )
            .await?;
        let addrs = [candidate.guard_addr, candidate.middle_addr, candidate.exit_addr];
        if addrs.iter().any(|addr| used_relay_addrs.contains(addr)) {
            continue;
        }
        for addr in addrs {
            used_relay_addrs.insert(addr);
        }
        winners.push(candidate);
        if winners.len() >= target_count {
            break;
        }
    }

    if winners.is_empty() {
        anyhow::bail!("Speculative path selection produced zero circuits");
    }
    if winners.len() < target_count {
        tracing::warn!(
            "Speculative path selection: got {}/{} circuits (partial success)",
            winners.len(),
            target_count
        );
    }

    Ok(winners)
}

pub async fn build_circuits_speculative_from_descriptors(
    creator_priv_key: &[u8; 32],
    descriptors: &[RelayDescriptor],
    publisher_addr: SocketAddr,
    publisher_pub_key: [u8; 32],
    target_count: usize,
    max_concurrent: usize,
) -> Result<Vec<OnionCircuit>> {
    let all_peers = relay_nodes_from_descriptors(descriptors);
    let exit_candidates = select_exit_candidates(&all_peers);
    build_circuits_speculative(
        creator_priv_key,
        &all_peers,
        &exit_candidates,
        publisher_addr,
        publisher_pub_key,
        target_count,
        max_concurrent,
    )
    .await
}

fn enumerate_speculative_candidates(
    all_peers: &[RelayNode],
    exit_candidates: &[RelayNode],
    max_concurrent: usize,
) -> Vec<(RelayNode, RelayNode, RelayNode)> {
    let mut out = Vec::new();
    for guard in all_peers {
        for middle in all_peers {
            if middle.addr == guard.addr {
                continue;
            }
            for exit in exit_candidates {
                if exit.addr == guard.addr || exit.addr == middle.addr {
                    continue;
                }
                out.push((guard.clone(), middle.clone(), exit.clone()));
            }
        }
    }
    out.shuffle(&mut thread_rng());
    if out.len() > max_concurrent {
        out.truncate(max_concurrent);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_subnet_filter_works() {
        let mk = |tag: &str, port: u16| RelayNode {
            addr: format!("127.0.0.1:{}", port).parse().unwrap(),
            identity_pub: [0u8; 32],
            subnet_tag: tag.to_string(),
            last_seen_ms: 0,
        };
        let peers = vec![
            mk("HostileSubnet", 1),
            mk("FreeSubnet", 2),
            mk("FreeSubnet", 3),
        ];
        let exits = select_exit_candidates(&peers);
        assert_eq!(exits.len(), 2);
        assert!(exits.iter().all(|p| p.subnet_tag == "FreeSubnet"));
    }

    #[test]
    fn speculative_candidate_generation_respects_constraints() {
        let mk = |tag: &str, port: u16, key: u8| RelayNode {
            addr: format!("127.0.0.1:{}", port).parse().unwrap(),
            identity_pub: [key; 32],
            subnet_tag: tag.to_string(),
            last_seen_ms: 0,
        };
        let all = vec![
            mk("HostileSubnet", 1101, 1),
            mk("HostileSubnet", 1102, 2),
            mk("HostileSubnet", 1103, 3),
            mk("FreeSubnet", 1201, 4),
            mk("FreeSubnet", 1202, 5),
        ];
        let exits = select_exit_candidates(&all);
        let candidates = enumerate_speculative_candidates(&all, &exits, 10);

        assert!(!candidates.is_empty());
        assert!(candidates.len() <= 10);
        for (g, m, e) in candidates {
            assert_ne!(g.addr, m.addr);
            assert_ne!(g.addr, e.addr);
            assert_ne!(m.addr, e.addr);
            assert_eq!(e.subnet_tag, "FreeSubnet");
        }
    }

    #[test]
    fn free_subnet_filter_from_descriptors_works() {
        let mk = |tag: &str, port: u16, key: u8| RelayDescriptor {
            identity_key: [key; 32],
            address: format!("127.0.0.1:{}", port).parse().unwrap(),
            subnet_tag: tag.to_string(),
            timestamp: 0,
            signature: [0u8; 64],
        };
        let descriptors = vec![
            mk("HostileSubnet", 2101, 1),
            mk("FreeSubnet", 2201, 2),
            mk("FreeSubnet", 2202, 3),
        ];
        let exits = select_exit_candidates_from_descriptors(&descriptors);
        assert_eq!(exits.len(), 2);
        assert!(exits.iter().all(|p| p.subnet_tag == "FreeSubnet"));
    }
}
