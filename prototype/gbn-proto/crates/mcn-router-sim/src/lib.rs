//! # MCN Router Simulator
//!
//! Simulates the multipath relay network for prototype testing.
//!
//! - `RelayNode`: A single TCP proxy that adds jitter.
//! - `Circuit`: A chain of `RelayNode`s.
//! - `MultipathRouter`: Manages multiple circuits and distributes traffic
//!   round-robin to simulate multipath routing.

// Workaround for rustc 1.94.1 ICE in `check_mod_deathness` / `early_lint_checks`.
// The ice is triggered when the lint system attempts to compute suggestion spans
// for grouped `use {}` imports. Suppressing both lints avoids the ICE.
// Remove once toolchain is upgraded beyond the affected version.
#![allow(dead_code, unused_imports)]

pub mod gossip;
pub mod relay_engine;
pub mod swarm;

pub mod circuit_manager;
pub mod observability;

pub use gossip::{GossipRequest, PlumTreeBehaviour};

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use gbn_protocol::chunk::EncryptedChunkPacket;
use rand::Rng;
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
};

#[derive(Debug, Error)]
pub enum RouterError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("No circuits available")]
    NoCircuitsAvailable,
}

// ─────────────────────────── Network Protocol ──────────────────────────────

/// Send a packet over a TCP stream using length-prefix framing.
/// Format: `[4-byte little-endian length][JSON serialized packet]`
pub async fn send_packet(
    stream: &mut TcpStream,
    packet: &EncryptedChunkPacket,
) -> Result<(), RouterError> {
    let json = serde_json::to_vec(packet)?;
    let len = json.len() as u32;
    stream.write_all(&len.to_le_bytes()).await?;
    stream.write_all(&json).await?;
    stream.flush().await?;
    Ok(())
}

/// Receive a packet from a TCP stream using length-prefix framing.
pub async fn recv_packet(stream: &mut TcpStream) -> Result<EncryptedChunkPacket, RouterError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;

    let mut data = vec![0u8; len];
    stream.read_exact(&mut data).await?;

    let packet = serde_json::from_slice(&data)?;
    Ok(packet)
}

// ─────────────────────────── Relay Node ───────────────────────────────────

/// Handle to a running relay node.
pub struct RelayHandle {
    pub listen_addr: SocketAddr,
    #[allow(dead_code)]
    forward_addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl RelayHandle {
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = &mut self.task.await;
    }
}

/// Spawn a relay node that forwards traffic to `forward_addr` with random jitter.
pub async fn spawn_relay(
    listen_addr: SocketAddr,
    forward_addr: SocketAddr,
    min_jitter_ms: u64,
    max_jitter_ms: u64,
) -> Result<RelayHandle, RouterError> {
    let listener = TcpListener::bind(listen_addr).await?;
    let bound_addr = listener.local_addr()?;

    tracing::debug!(
        "Relay spawned: {} -> {} (jitter: {}-{}ms)",
        bound_addr,
        forward_addr,
        min_jitter_ms,
        max_jitter_ms
    );

    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    tracing::debug!("Relay {} shutting down", bound_addr);
                    break;
                }
                accept_res = listener.accept() => {
                    match accept_res {
                        Ok((mut client_stream, peer_addr)) => {
                            tracing::trace!("Relay {} accepted connection from {}", bound_addr, peer_addr);
                            let fwd = forward_addr;
                            tokio::spawn(async move {
                                // 1. Receive from client
                                let packet = match recv_packet(&mut client_stream).await {
                                    Ok(p) => p,
                                    Err(e) => {
                                        tracing::error!("Relay {} failed to receive packet: {}", bound_addr, e);
                                        return;
                                    }
                                };

                                // 2. Add Jitter
                                let jitter = rand::thread_rng().gen_range(min_jitter_ms..=max_jitter_ms);
                                tokio::time::sleep(Duration::from_millis(jitter)).await;

                                // 3. Forward to next hop
                                match TcpStream::connect(fwd).await {
                                    Ok(mut fwd_stream) => {
                                        if let Err(e) = send_packet(&mut fwd_stream, &packet).await {
                                            tracing::error!("Relay {} failed to forward packet: {}", bound_addr, e);
                                        } else {
                                            tracing::trace!("Relay {} forwarded chunk {} to {}", bound_addr, packet.chunk_index, fwd);
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!("Relay {} failed to connect to next hop {}: {}", bound_addr, fwd, e);
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            tracing::error!("Relay {} listener error: {}", bound_addr, e);
                        }
                    }
                }
            }
        }
    });

    Ok(RelayHandle {
        listen_addr: bound_addr,
        forward_addr,
        shutdown_tx: Some(shutdown_tx),
        task,
    })
}

// ─────────────────────────── Circuit ──────────────────────────────────────

pub struct CircuitHandle {
    pub entry_addr: SocketAddr,
    pub exit_addr: SocketAddr, // The original destination passed in
    relays: Vec<RelayHandle>,
}

impl CircuitHandle {
    pub async fn shutdown(mut self) {
        // Shutdown all relays
        // Best to shutdown in reverse order, but parallel is faster
        let mut futures = Vec::new();
        for relay in self.relays.drain(..) {
            futures.push(tokio::spawn(relay.shutdown()));
        }
        for f in futures {
            let _ = f.await;
        }
    }
}

/// Construct a circuit of N hops backwards from the destination.
pub async fn spawn_circuit(
    destination: SocketAddr,
    num_hops: usize,
    min_jitter_ms: u64,
    max_jitter_ms: u64,
) -> Result<CircuitHandle, RouterError> {
    if num_hops == 0 {
        return Ok(CircuitHandle {
            entry_addr: destination,
            exit_addr: destination,
            relays: Vec::new(),
        });
    }

    let mut relays = Vec::new();
    let mut next_hop = destination;

    // Build backwards from destination to entry
    for _ in 0..num_hops {
        let relay = spawn_relay(
            "127.0.0.1:0".parse().unwrap(),
            next_hop,
            min_jitter_ms,
            max_jitter_ms,
        )
        .await?;
        next_hop = relay.listen_addr;
        relays.push(relay);
    }

    // Now relays[0] points to destination, relays[1] points to relays[0], etc.
    // The entry point is the last relay we created.
    let entry_addr = next_hop;

    Ok(CircuitHandle {
        entry_addr,
        exit_addr: destination, // the ultimate target
        relays,
    })
}

// ─────────────────────────── Multipath Router ─────────────────────────────

pub struct MultipathRouter {
    circuits: Vec<CircuitHandle>,
    next_circuit: Arc<Mutex<usize>>,
}

impl MultipathRouter {
    pub async fn shutdown(mut self) {
        let mut futures = Vec::new();
        for circuit in self.circuits.drain(..) {
            futures.push(tokio::spawn(circuit.shutdown()));
        }
        for f in futures {
            let _ = f.await;
        }
    }

    /// Round-robin send a chunk through one of the circuits
    pub async fn send_chunk(&self, packet: &EncryptedChunkPacket) -> Result<(), RouterError> {
        if self.circuits.is_empty() {
            return Err(RouterError::NoCircuitsAvailable);
        }

        let circuit_idx = {
            let mut guard = self.next_circuit.lock().unwrap();
            let idx = *guard;
            *guard = (idx + 1) % self.circuits.len();
            idx
        };

        let circuit = &self.circuits[circuit_idx];
        tracing::debug!(
            "Routing chunk {} via circuit entering {}",
            packet.chunk_index,
            circuit.entry_addr
        );

        let mut stream = TcpStream::connect(circuit.entry_addr).await?;
        send_packet(&mut stream, packet).await?;

        Ok(())
    }
}

pub async fn create_multipath_router(
    destinations: Vec<SocketAddr>,
    hops_per_path: usize,
    min_jitter_ms: u64,
    max_jitter_ms: u64,
) -> Result<MultipathRouter, RouterError> {
    let mut circuits = Vec::new();

    for dest in destinations {
        let circuit = spawn_circuit(dest, hops_per_path, min_jitter_ms, max_jitter_ms).await?;
        circuits.push(circuit);
    }

    Ok(MultipathRouter {
        circuits,
        next_circuit: Arc::new(Mutex::new(0)),
    })
}

// ─────────────────────────────── Tests ───────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    fn dummy_packet(index: u32) -> EncryptedChunkPacket {
        EncryptedChunkPacket {
            session_id: [0u8; 16],
            chunk_index: index,
            total_chunks: 10,
            plaintext_hash: [0u8; 32],
            nonce: [0u8; 12],
            ciphertext: b"dummy ciphertext".to_vec(),
        }
    }

    /// Helper to spawn a dummy destination that collects received packets
    async fn spawn_receiver() -> (
        SocketAddr,
        tokio::sync::mpsc::Receiver<EncryptedChunkPacket>,
        JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(100);

        let handle = tokio::spawn(async move {
            loop {
                // Short timeout to allow clean shutdown during tests if needed,
                // but usually we just drop it or let the test end.
                let accept_res = tokio::time::timeout(
                    tokio::time::Duration::from_millis(500),
                    listener.accept(),
                )
                .await;
                if let Ok(Ok((mut stream, _))) = accept_res {
                    if let Ok(packet) = recv_packet(&mut stream).await {
                        if tx.send(packet).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        (addr, rx, handle)
    }

    // T-ROUTER-1: Single relay hop forwards successfully.
    #[tokio::test]
    async fn test_single_hop_relay() {
        let (dest_addr, mut rx, _rx_handle) = spawn_receiver().await;

        let relay = spawn_relay("127.0.0.1:0".parse().unwrap(), dest_addr, 10, 50)
            .await
            .unwrap();

        let mut client_stream = TcpStream::connect(relay.listen_addr).await.unwrap();
        let packet = dummy_packet(42);
        send_packet(&mut client_stream, &packet).await.unwrap();
        let received = res_timeout(rx.recv()).await.unwrap().unwrap();
        assert_eq!(received.chunk_index, 42);

        relay.shutdown().await;
    }

    // T-ROUTER-2: 3-hop circuit forwards successfully.
    #[tokio::test]
    async fn test_three_hop_circuit() {
        let (dest_addr, mut rx, _) = spawn_receiver().await;

        let circuit = spawn_circuit(dest_addr, 3, 5, 20).await.unwrap();
        assert_eq!(circuit.relays.len(), 3);

        let mut client_stream = TcpStream::connect(circuit.entry_addr).await.unwrap();
        let packet = dummy_packet(7);
        send_packet(&mut client_stream, &packet).await.unwrap();

        let received = res_timeout(rx.recv()).await.unwrap().unwrap();
        assert_eq!(received.chunk_index, 7);

        circuit.shutdown().await;
    }

    // T-ROUTER-3: Multipath router routes over 3 paths.
    #[tokio::test]
    async fn test_multipath_10_chunks() {
        // Spawn 3 destinations
        let (d1, mut rx1, _) = spawn_receiver().await;
        let (d2, mut rx2, _) = spawn_receiver().await;
        let (d3, mut rx3, _) = spawn_receiver().await;

        let router = create_multipath_router(vec![d1, d2, d3], 2, 5, 10)
            .await
            .unwrap();
        assert_eq!(router.circuits.len(), 3);

        // Send 6 chunks
        for i in 0..6 {
            router.send_chunk(&dummy_packet(i)).await.unwrap();
        }

        // They are sent round-robin, so each dest should get 2 chunks
        let mut d1_count = 0;
        let mut d2_count = 0;
        let mut d3_count = 0;

        // Collect 6 chunks total
        for _ in 0..6 {
            tokio::select! {
                Some(_) = rx1.recv() => d1_count += 1,
                Some(_) = rx2.recv() => d2_count += 1,
                Some(_) = rx3.recv() => d3_count += 1,
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(1500)) => {
                    panic!("Timeout waiting for chunks");
                }
            }
        }

        assert_eq!(d1_count, 2);
        assert_eq!(d2_count, 2);
        assert_eq!(d3_count, 2);

        router.shutdown().await;
    }

    // Helper for test timeouts
    async fn res_timeout<T>(fut: impl std::future::Future<Output = T>) -> Option<T> {
        tokio::time::timeout(tokio::time::Duration::from_secs(2), fut)
            .await
            .ok()
    }
}
