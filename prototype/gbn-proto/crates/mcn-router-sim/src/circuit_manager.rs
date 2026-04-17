//! # Circuit Manager (Step 5)
//!
//! The Creator-side orchestrator for building and maintaining Telescopic Onion
//! Circuits. Responsibilities:
//!
//! 1. **Telescopic Build** — dials the Guard, sends `RelayExtend` toward the
//!    Middle (via Guard), sends another `RelayExtend` toward the Exit (via
//!    Guard → Middle). Each hop independently validates its Noise_XX handshake
//!    with the next hop before returning `RelayExtended`.
//!
//! 2. **Heartbeat Watchdog** — sends periodic `RelayHeartbeat` PINGs through
//!    the Guard. If an echo is not received within the timeout window, the
//!    circuit is declared dead.
//!
//! 3. **Chunk Queue & Fallback** — un-ACKed chunks are retained in an in-flight
//!    queue. If the heartbeat watchdog fires, it immediately kicks off circuit
//!    rebuild using a **disjoint** Guard (queried from the DHT) to prevent
//!    Temporal Circuit Correlation.

use anyhow::{Context, Result};
use gbn_protocol::dht::RelayDescriptor;
use gbn_protocol::onion::{
    DataPayload, ExtendPayload, ExtendedPayload, HeartbeatPayload, OnionCell,
};
use mcn_crypto::noise::{build_initiator, complete_handshake, encrypt_frame};
use rand::seq::SliceRandom;
use rand::thread_rng;
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};
use tokio::time::Instant;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{mpsc, Mutex},
    time::timeout,
};

use crate::observability::publish_circuit_build_result_from_env;
use crate::relay_engine::{recv_cell_secure, send_cell_secure};

// ─────────────────────────── Types ─────────────────────────────────────────

pub type ChunkBytes = Vec<u8>;

use serde::{Deserialize, Serialize};

/// A descriptor for a relay node (simplified — real impl uses DHT records).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RelayNode {
    pub addr: SocketAddr,
    pub identity_pub: [u8; 32],
    pub subnet_tag: String,
    /// Unix timestamp (ms) when this node was last seen by the local seed store.
    /// Set on every insert/update; never propagated over gossip so it reflects
    /// local observation time, not the sender's clock.
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

/// A fully built Telescopic Circuit: Guard → Middle → Exit.
/// The Creator holds a single TCP stream to the Guard; everything else is
/// tunnelled through nested Noise_XX sessions.
pub struct OnionCircuit {
    /// The TCP connection open to the Guard node.
    guard_stream: TcpStream,
    /// Transport states in order: [guard_transport, middle_transport, exit_transport]
    /// The Creator stacks these to produce nested encryption when sending data.
    transports: Vec<snow::TransportState>,
    /// Address of the Guard (used for disjoint rebuild check).
    pub guard_addr: SocketAddr,
    /// Address of the Middle relay.
    pub middle_addr: SocketAddr,
    /// Address of the Exit relay (FreeSubnet only).
    pub exit_addr: SocketAddr,
}

// ─────────────────────────── Circuit Builder ───────────────────────────────

/// Build a complete Guard → Middle → Exit telescopic circuit.
///
/// Steps:
///   1. TCP connect + Noise_XX handshake with Guard.
///   2. Send `RelayExtend{Middle}` through Guard; await `RelayExtended`.
///   3. Send `RelayExtend{Exit}` through Guard→Middle; await `RelayExtended`.
///
/// Each `RelayExtended` response contains the next-hop's static public key
/// (the handshake hash), so the Creator can verify the correct node responded.
pub async fn build_circuit(
    creator_priv_key: &[u8; 32],
    guard: &RelayNode,
    middle: &RelayNode,
    exit: &RelayNode,
    trace_id: &str,
) -> Result<OnionCircuit> {
    let started = Instant::now();
    let result: Result<OnionCircuit> = async {
        // ── Step 1: Connect and handshake with Guard ───────────────────────────
        tracing::info!("Building circuit: connecting to Guard {}", guard.addr);
        let mut guard_stream = timeout(Duration::from_secs(10), TcpStream::connect(guard.addr))
            .await
            .context(format!("Timeout connecting to Guard {}", guard.addr))?
            .context(format!("Failed to TCP-connect to Guard {}", guard.addr))?;

        let guard_hs = build_initiator(creator_priv_key, &guard.identity_pub)
            .context(format!("Failed to build initiator HS for Guard {}", guard.addr))?;
        let mut guard_transport = complete_handshake(&mut guard_stream, guard_hs, true)
            .await
            .context(format!("Noise_XX handshake with Guard {} failed", guard.addr))?;
        tracing::debug!("Guard handshake complete");

        // ── Step 2: Extend to Middle through Guard ────────────────────────────
        tracing::info!("Extending circuit to Middle {}", middle.addr);
        let middle_hs = build_initiator(creator_priv_key, &middle.identity_pub)
            .context("Failed to build initiator HS for Middle")?;

        // Capture the first handshake message to embed in RelayExtend
        let mut hs_buf = vec![0u8; 65535];
        let mut middle_hs = middle_hs; // rebind as mut
        let hs_len = middle_hs.write_message(&[], &mut hs_buf)?;
        let initial_hs_payload = hs_buf[..hs_len].to_vec();

        send_cell_secure(
            &mut guard_stream,
            &mut guard_transport,
            &OnionCell::RelayExtend(ExtendPayload {
                next_hop: middle.addr,
                next_identity_key: middle.identity_pub,
                handshake_payload: initial_hs_payload,
                trace_id: if trace_id.is_empty() { None } else { Some(trace_id.to_string()) },
            }),
        )
        .await
        .context(format!("Failed to send RelayExtend(middle={}) to Guard {}", middle.addr, guard.addr))?;

        let response = timeout(
            Duration::from_secs(10),
            recv_cell_secure(&mut guard_stream, &mut guard_transport),
        )
            .await
            .context(format!("Timeout waiting for RelayExtended(middle={}) from Guard {}", middle.addr, guard.addr))?
            .context(format!("Failed to read RelayExtended(middle={}) from Guard {}", middle.addr, guard.addr))?;

        let ExtendedPayload {
            handshake_response: middle_hs_response,
        } = match response {
            OnionCell::RelayExtended(p) => p,
            other => anyhow::bail!("Expected RelayExtended for Middle, got {:?}", other),
        };
        tracing::debug!(
            "Middle extension confirmed; remote static: {} bytes",
            middle_hs_response.len()
        );

        // Complete the Middle handshake state (remaining turns after initial message)
        let middle_transport = complete_handshake(&mut guard_stream, middle_hs, false)
            .await
            .context(format!("Noise_XX handshake continuation for Middle {} failed", middle.addr))?;

        // ── Step 3: Extend to Exit through Guard→Middle ───────────────────────
        tracing::info!("Extending circuit to Exit {}", exit.addr);
        let exit_hs = build_initiator(creator_priv_key, &exit.identity_pub)
            .context("Failed to build initiator HS for Exit")?;
        let mut exit_hs = exit_hs;
        let hs_len = exit_hs.write_message(&[], &mut hs_buf)?;
        let initial_exit_payload = hs_buf[..hs_len].to_vec();

        send_cell_secure(
            &mut guard_stream,
            &mut guard_transport,
            &OnionCell::RelayExtend(ExtendPayload {
                next_hop: exit.addr,
                next_identity_key: exit.identity_pub,
                handshake_payload: initial_exit_payload,
                trace_id: if trace_id.is_empty() { None } else { Some(trace_id.to_string()) },
            }),
        )
        .await
        .context(format!("Failed to send RelayExtend(exit={}) through Guard {}", exit.addr, guard.addr))?;

        let response = timeout(
            Duration::from_secs(10),
            recv_cell_secure(&mut guard_stream, &mut guard_transport),
        )
            .await
            .context(format!("Timeout waiting for RelayExtended(exit={}) via Guard {}", exit.addr, guard.addr))?
            .context(format!("Failed to read RelayExtended(exit={}) via Guard {}", exit.addr, guard.addr))?;

        match response {
            OnionCell::RelayExtended(_) => {}
            other => anyhow::bail!("Expected RelayExtended for exit={}, got {:?}", exit.addr, other),
        };

        let exit_transport = complete_handshake(&mut guard_stream, exit_hs, false)
            .await
            .context(format!("Noise_XX handshake continuation for Exit {} failed", exit.addr))?;

        tracing::info!(
            "Circuit built: {} → {} → {}",
            guard.addr,
            middle.addr,
            exit.addr
        );

        Ok(OnionCircuit {
            guard_stream,
            transports: vec![guard_transport, middle_transport, exit_transport],
            guard_addr: guard.addr,
            middle_addr: middle.addr,
            exit_addr: exit.addr,
        })
    }
    .await;

    let latency_ms = started.elapsed().as_millis();
    publish_circuit_build_result_from_env(result.is_ok(), latency_ms).await;
    result
}

// ─────────────────────────── Circuit Manager ───────────────────────────────

/// Manages multiple active circuits, the in-flight chunk queue, and the
/// heartbeat watchdog. Hands chunks to circuits round-robin.
pub struct CircuitManager {
    /// All live circuits.
    circuits: Arc<Mutex<Vec<OnionCircuit>>>,
    /// Chunks that have been sent but not yet ACKed by the Publisher.
    /// On circuit failure, these are re-queued to a new circuit.
    inflight_queue: Arc<Mutex<Vec<(u32, ChunkBytes)>>>,
    /// Set of Guard addresses used so far — new circuits MUST NOT reuse them
    /// to prevent Temporal Circuit Correlation.
    used_guards: Arc<Mutex<HashSet<SocketAddr>>>,
    /// Channel the heartbeat watchdog uses to signal a dead circuit.
    failure_tx: mpsc::Sender<usize>,
    failure_rx: Arc<Mutex<mpsc::Receiver<usize>>>,
    retry_counts: Arc<Mutex<HashMap<u32, u8>>>,
}

impl CircuitManager {
    pub fn new() -> Self {
        let (failure_tx, failure_rx) = mpsc::channel(32);
        Self {
            circuits: Arc::new(Mutex::new(Vec::new())),
            inflight_queue: Arc::new(Mutex::new(Vec::new())),
            used_guards: Arc::new(Mutex::new(HashSet::new())),
            failure_tx,
            failure_rx: Arc::new(Mutex::new(failure_rx)),
            retry_counts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a newly built circuit and launch its heartbeat watchdog.
    pub async fn add_circuit(&self, circuit: OnionCircuit) {
        let guard_addr = circuit.guard_addr;
        {
            let mut used = self.used_guards.lock().await;
            used.insert(guard_addr);
        }
        let mut circuits = self.circuits.lock().await;
        let idx = circuits.len();
        circuits.push(circuit);
        drop(circuits);

        // Launch heartbeat watchdog for this circuit index
        let failure_tx = self.failure_tx.clone();
        let circuits_ref = Arc::clone(&self.circuits);
        tokio::spawn(async move {
            heartbeat_watchdog(idx, circuits_ref, failure_tx).await;
        });
    }

    /// Send an encrypted chunk through the next available circuit (round-robin).
    /// Stores the chunk in the in-flight queue until ACKed.
    pub async fn send_chunk(&self, chunk_index: u32, payload: ChunkBytes) -> Result<()> {
        // Push to in-flight queue before sending (so we can re-queue on failure)
        {
            let mut q = self.inflight_queue.lock().await;
            q.push((chunk_index, payload.clone()));
        }

        let mut circuits = self.circuits.lock().await;
        if circuits.is_empty() {
            anyhow::bail!("No active circuits available for chunk {}", chunk_index);
        }

        // Round-robin selection
        let idx = chunk_index as usize % circuits.len();
        let circuit = &mut circuits[idx];

        // Wrap payload in nested encryption layers (Exit → Middle → Guard)
        let mut wrapped = payload;
        for transport in circuit.transports.iter_mut().rev() {
            wrapped =
                encrypt_frame(transport, &wrapped).context("Failed to encrypt chunk layer")?;
        }

        let guard_link = circuit
            .transports
            .get_mut(0)
            .context("Circuit transport state missing guard link")?;
        send_cell_secure(
            &mut circuit.guard_stream,
            guard_link,
            &OnionCell::RelayData(DataPayload {
                ciphertext: wrapped,
            }),
        )
        .await
        .context("Failed to send RelayData to Guard")?;

        Ok(())
    }

    /// Acknowledge a delivered chunk, removing it from the in-flight queue.
    pub async fn ack_chunk(&self, chunk_index: u32) {
        let mut q = self.inflight_queue.lock().await;
        q.retain(|(idx, _)| *idx != chunk_index);
        tracing::debug!(
            "ACKed chunk {}; in-flight remaining: {}",
            chunk_index,
            q.len()
        );
    }

    /// Process any pending circuit-failure signals.
    ///
    /// In Tests: call this after killing a relay to verify the manager re-queues
    /// and could route through replacement circuits.
    pub async fn drain_failures(&self) -> Vec<(u32, ChunkBytes)> {
        let mut requeued = Vec::new();
        let mut rx = self.failure_rx.lock().await;
        let mut dead = Vec::new();

        while let Ok(dead_idx) = rx.try_recv() {
            tracing::warn!(
                "Circuit {} declared dead — collecting in-flight chunks",
                dead_idx
            );
            dead.push(dead_idx);
        }

        if !dead.is_empty() {
            dead.sort_unstable();
            dead.dedup();
            dead.reverse();
            let mut circuits = self.circuits.lock().await;
            for idx in dead {
                if idx < circuits.len() {
                    circuits.swap_remove(idx);
                }
            }
            drop(circuits);

            let mut q = self.inflight_queue.lock().await;
            requeued.extend(q.drain(..));
        }
        requeued
    }

    pub async fn process_failures_with_rebuild(
        &self,
        creator_priv_key: &[u8; 32],
        all_peers: &[RelayNode],
        exit_candidates: &[RelayNode],
    ) -> Result<usize> {
        let requeued = self.drain_failures().await;
        if requeued.is_empty() {
            return Ok(0);
        }

        let used = self.used_guards.lock().await.clone();
        let guard_pool: Vec<_> = all_peers
            .iter()
            .filter(|p| !used.contains(&p.addr))
            .cloned()
            .collect();

        if guard_pool.is_empty() {
            anyhow::bail!("No disjoint guards available for rebuild");
        }

        let guard = &guard_pool[0];
        let middle = all_peers
            .iter()
            .find(|p| p.addr != guard.addr)
            .cloned()
            .context("No middle peer available for rebuild")?;
        let exit = exit_candidates
            .iter()
            .find(|p| p.addr != guard.addr && p.addr != middle.addr)
            .cloned()
            .context("No exit candidate available for rebuild")?;

        let circuit = build_circuit(creator_priv_key, guard, &middle, &exit, "").await?;
        self.add_circuit(circuit).await;

        let mut resent = 0usize;
        for (chunk_idx, payload) in requeued {
            let should_send = {
                let mut retries = self.retry_counts.lock().await;
                let count = retries.entry(chunk_idx).or_insert(0);
                if *count >= 3 {
                    false
                } else {
                    *count += 1;
                    true
                }
            };

            if should_send && self.send_chunk(chunk_idx, payload).await.is_ok() {
                resent += 1;
            }
        }
        Ok(resent)
    }

    pub async fn process_failures_with_rebuild_from_descriptors(
        &self,
        creator_priv_key: &[u8; 32],
        descriptors: &[RelayDescriptor],
    ) -> Result<usize> {
        let all_peers = relay_nodes_from_descriptors(descriptors);
        let exit_candidates = select_exit_candidates(&all_peers);
        self.process_failures_with_rebuild(creator_priv_key, &all_peers, &exit_candidates)
            .await
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
///
/// Logs each circuit's hops in the format required by test-spec §5.5:
/// `Circuit N: guard=<ip> middle=<ip> exit=<ip>`
/// then logs `Path diversity: PASS/FAIL (unique=X / total=Y)`.
///
/// Returns `true` if all guard/middle/exit addresses across all circuits are unique.
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
    target_count: usize,
    max_concurrent: usize,
) -> Result<Vec<OnionCircuit>> {
    use tokio::task::JoinSet;

    if target_count == 0 {
        return Ok(Vec::new());
    }

    let candidates = enumerate_speculative_candidates(all_peers, exit_candidates, max_concurrent);
    let mut joins = JoinSet::new();

    for (guard, middle, exit) in candidates {
        let key = *creator_priv_key;
        joins.spawn(async move { build_circuit(&key, &guard, &middle, &exit, "").await });
    }

    let mut winners = Vec::new();
    let mut used_relay_addrs = HashSet::new();
    while let Some(joined) = joins.join_next().await {
        if let Ok(Ok(c)) = joined {
            let addrs = [c.guard_addr, c.middle_addr, c.exit_addr];
            if addrs.iter().any(|addr| used_relay_addrs.contains(addr)) {
                continue;
            }

            for addr in &addrs {
                used_relay_addrs.insert(*addr);
            }
            winners.push(c);
            if winners.len() >= target_count {
                break;
            }
        }
    }

    // Explicitly cancel unfinished speculative dials once target is reached (or no more useful work remains).
    joins.abort_all();

    if winners.is_empty() {
        anyhow::bail!("Speculative dialing produced zero successful circuits");
    }
    if winners.len() < target_count {
        tracing::warn!(
            "Speculative dialing: got {}/{} circuits (partial success)",
            winners.len(),
            target_count
        );
    }
    Ok(winners)
}

pub async fn build_circuits_speculative_from_descriptors(
    creator_priv_key: &[u8; 32],
    descriptors: &[RelayDescriptor],
    target_count: usize,
    max_concurrent: usize,
) -> Result<Vec<OnionCircuit>> {
    let all_peers = relay_nodes_from_descriptors(descriptors);
    let exit_candidates = select_exit_candidates(&all_peers);
    build_circuits_speculative(
        creator_priv_key,
        &all_peers,
        &exit_candidates,
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
    fn descriptor_geofence_filter_works() {
        let mk = |tag: &str, port: u16, key: u8| RelayDescriptor {
            identity_key: [key; 32],
            address: format!("127.0.0.1:{}", port).parse().unwrap(),
            subnet_tag: tag.to_string(),
            timestamp: 1,
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

// ─────────────────────────── Heartbeat Watchdog ────────────────────────────

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(10);

/// Continuously pings the circuit's Guard via `RelayHeartbeat`.
/// Declares the circuit dead (and notifies via `failure_tx`) if no echo
/// arrives within `HEARTBEAT_TIMEOUT`.
async fn heartbeat_watchdog(
    circuit_idx: usize,
    circuits: Arc<Mutex<Vec<OnionCircuit>>>,
    failure_tx: mpsc::Sender<usize>,
) {
    let mut seq_id: u64 = 0;
    loop {
        tokio::time::sleep(HEARTBEAT_INTERVAL).await;
        seq_id += 1;

        // Send PING
        let send_result: Result<()> = {
            let mut locked = circuits.lock().await;
            if let Some(circuit) = locked.get_mut(circuit_idx) {
                if let Some(guard_link) = circuit.transports.get_mut(0) {
                    send_cell_secure(
                        &mut circuit.guard_stream,
                        guard_link,
                        &OnionCell::RelayHeartbeat(HeartbeatPayload { seq_id }),
                    )
                    .await
                } else {
                    Err(anyhow::anyhow!("Missing guard transport state"))
                }
            } else {
                // Circuit already removed
                return;
            }
        };

        if send_result.is_err() {
            tracing::warn!(
                "Heartbeat SEND failed for circuit {} — declaring dead",
                circuit_idx
            );
            let _ = failure_tx.send(circuit_idx).await;
            return;
        }

        // Await PONG
        let pong_result = {
            let mut locked = circuits.lock().await;
            if let Some(circuit) = locked.get_mut(circuit_idx) {
                let maybe_guard_link = circuit.transports.get_mut(0);
                let guard_link = match maybe_guard_link {
                    Some(t) => t,
                    None => return,
                };
                timeout(
                    HEARTBEAT_TIMEOUT,
                    recv_cell_secure(&mut circuit.guard_stream, guard_link),
                )
                .await
            } else {
                return;
            }
        };

        match pong_result {
            Ok(Ok(OnionCell::RelayHeartbeat(p))) if p.seq_id == seq_id => {
                tracing::trace!("Heartbeat PONG seq={} for circuit {}", seq_id, circuit_idx);
            }
            _ => {
                tracing::warn!(
                    "Heartbeat PONG timeout/mismatch for circuit {} — declaring dead",
                    circuit_idx
                );
                let _ = failure_tx.send(circuit_idx).await;
                return;
            }
        }
    }
}
