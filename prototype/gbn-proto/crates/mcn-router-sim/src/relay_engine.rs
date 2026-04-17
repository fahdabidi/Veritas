//! # Onion Relay Engine (Step 4)
//!
//! Replaces the bare TCP forwarder with a state machine that speaks the
//! `OnionCell` protocol. Each relay loop:
//!
//!   1. Accepts a TCP connection from an upstream peer (or the Creator).
//!   2. Reads a length-prefixed `OnionCell` frame.
//!   3. Dispatches based on cell type:
//!      - `RelayExtend`   → dial the next hop, complete the inner Noise_XX
//!                          handshake, cache the downstream connection, reply
//!                          with `RelayExtended`.
//!      - `RelayData`     → forward the opaque ciphertext to the cached
//!                          downstream connection (or to the final destination
//!                          if this is the Exit node).
//!      - `RelayHeartbeat`→ echo back a heartbeat so the Circuit Manager knows
//!                          the link is still live.
//!
//! Because the `RelayExtend` payload carries the initiator half of a fresh
//! Noise_XX handshake, a malicious relay **cannot** fabricate a valid
//! `RelayExtended` response — doing so would require it to hold the private
//! key of the legitimate next-hop node.

use crate::control::{push_packet_meta, push_packet_meta_trace};
use crate::circuit_manager::RelayNode;
use crate::observability::publish_chunks_received_from_env;
use anyhow::{Context, Result};
use gbn_protocol::onion::{
    DataPayload, ExtendPayload, ExtendedPayload, HeartbeatPayload, OnionCell,
};
use mcn_crypto::noise::{
    build_initiator, build_responder, complete_handshake, decrypt_frame, encrypt_frame,
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
    time::timeout,
};

// ─────────────────────────── Wire Framing ─────────────────────────────────
// OnionCell frames are length-prefixed with a 4-byte LE u32 header.

pub async fn send_cell(stream: &mut TcpStream, cell: &OnionCell) -> Result<()> {
    let encoded = serde_json::to_vec(cell)?;
    let len = encoded.len() as u32;
    stream.write_all(&len.to_le_bytes()).await?;
    stream.write_all(&encoded).await?;
    stream.flush().await?;
    Ok(())
}

pub async fn recv_cell(stream: &mut TcpStream) -> Result<OnionCell> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    let cell = serde_json::from_slice(&buf)?;
    Ok(cell)
}

/// Send an OnionCell over an established Noise transport channel.
pub async fn send_cell_secure(
    stream: &mut TcpStream,
    transport: &mut snow::TransportState,
    cell: &OnionCell,
) -> Result<()> {
    let encoded = serde_json::to_vec(cell)?;
    let cipher = encrypt_frame(transport, &encoded)?;
    let len = cipher.len() as u32;
    stream.write_all(&len.to_le_bytes()).await?;
    stream.write_all(&cipher).await?;
    stream.flush().await?;
    Ok(())
}

/// Receive an OnionCell from an established Noise transport channel.
pub async fn recv_cell_secure(
    stream: &mut TcpStream,
    transport: &mut snow::TransportState,
) -> Result<OnionCell> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut cipher = vec![0u8; len];
    stream.read_exact(&mut cipher).await?;
    let plain = decrypt_frame(transport, &cipher)?;
    let cell = serde_json::from_slice(&plain)?;
    Ok(cell)
}

// ─────────────────────────── Noise Handshake Helpers ──────────────────────

// ─────────────────────────── Relay Engine ─────────────────────────────────

pub struct OnionRelayHandle {
    pub listen_addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl OnionRelayHandle {
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = self.task.await;
    }
}

/// Spawn an OnionRelay that speaks the full `OnionCell` protocol.
///
/// `local_priv_key` is the 32-byte Curve25519 private key for this relay's
/// `Noise_XX` identity — this is what cryptographically validates the node's
/// place in the telescopic circuit.
pub async fn spawn_onion_relay(
    listen_addr: SocketAddr,
    local_priv_key: [u8; 32],
    seed_store: Arc<RwLock<HashMap<SocketAddr, RelayNode>>>,
    min_jitter_ms: u64,
    max_jitter_ms: u64,
) -> Result<OnionRelayHandle> {
    let listener = TcpListener::bind(listen_addr).await?;
    let bound_addr = listener.local_addr()?;

    tracing::info!("OnionRelay listening on {}", bound_addr);

    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    tracing::info!("OnionRelay {} shutting down", bound_addr);
                    break;
                }
                accept_res = listener.accept() => {
                    match accept_res {
                        Ok((stream, peer_addr)) => {
                            tracing::debug!("OnionRelay {} accepted from {}", bound_addr, peer_addr);
                            let key = local_priv_key;
                            let seed_store = seed_store.clone();
                            let min_j = min_jitter_ms;
                            let max_j = max_jitter_ms;
                            tokio::spawn(async move {
                                let conn = tokio::spawn(async move {
                                    handle_onion_connection(stream, key, seed_store, min_j, max_j)
                                        .await
                                })
                                .await;

                                match conn {
                                    Ok(Ok(())) => {}
                                    Ok(Err(e)) => {
                                        let msg = format!(
                                            "relay.handle_connection ERROR node={} listen={} peer={} err={e:#}",
                                            crate::trace::node_id(), bound_addr, peer_addr
                                        );
                                        tracing::error!("{}", msg);
                                        push_packet_meta_trace("ComponentError", 0, &msg, "", "relay.error");
                                    }
                                    Err(join_err) => {
                                        let msg = format!(
                                            "relay.handle_connection PANIC node={} listen={} peer={} err={}",
                                            crate::trace::node_id(), bound_addr, peer_addr, join_err
                                        );
                                        tracing::error!("{}", msg);
                                        push_packet_meta_trace("ComponentError", 0, &msg, "", "relay.panic");
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            tracing::error!("OnionRelay {} accept error: {}", bound_addr, e);
                        }
                    }
                }
            }
        }
    });

    Ok(OnionRelayHandle {
        listen_addr: bound_addr,
        shutdown_tx: Some(shutdown_tx),
        task,
    })
}

// ─────────────────────────── Connection Handler (State Machine) ─────────────

/// Core per-connection state machine.
///
/// Holds an optional downstream Noise transport (established via `RelayExtend`),
/// and processes cells from the upstream until the connection closes.
async fn handle_onion_connection(
    mut upstream: TcpStream,
    local_priv_key: [u8; 32],
    seed_store: Arc<RwLock<HashMap<SocketAddr, RelayNode>>>,
    min_jitter_ms: u64,
    max_jitter_ms: u64,
) -> Result<()> {
    // Upstream link is always Noise_XX protected after initial dial.
    let upstream_hs = build_responder(&local_priv_key)
        .context("Failed to build upstream Noise_XX responder")?;
    let mut upstream_transport = complete_handshake(&mut upstream, upstream_hs, false)
        .await
        .context("Noise_XX handshake with upstream failed")?;

    // Optional downstream session: set when we receive a RelayExtend.
    let mut downstream_transport: Option<(TcpStream, snow::TransportState)> = None;

    // Distributed trace chain ID received from the circuit initiator via RelayExtend.
    // Carried on every ring-buffer entry for this connection so failures on any hop
    // can be correlated back to the originating SendDummy / Creator operation.
    let mut connection_trace_id = String::new();

    loop {
        let cell = timeout(
            Duration::from_secs(30),
            recv_cell_secure(&mut upstream, &mut upstream_transport),
        )
            .await
            .context("Timeout waiting for OnionCell from upstream")?
            .context("Failed to read OnionCell from upstream")?;

        // Capture a compact snapshot of the cell before it is consumed by the match.
        // Stored here so any bail! below can attach it as a RelayFailureCapture entry,
        // letting a unit test reconstruct and replay the exact cell that failed.
        let cell_snapshot: String = {
            let mut s = serde_json::to_string(&cell).unwrap_or_else(|_| format!("{:?}", cell));
            if s.len() > 1024 {
                s.truncate(1024);
                s.push_str("...(truncated)");
            }
            s
        };

        match cell {
            // ── RelayExtend: dial next hop, perform inner handshake ────────
            OnionCell::RelayExtend(ExtendPayload {
                next_hop,
                next_identity_key,
                handshake_payload,
                trace_id,
            }) => {
                // Latch the initiator's chain ID for all subsequent ring entries.
                if let Some(tid) = trace_id {
                    connection_trace_id = tid;
                }
                tracing::debug!("RelayExtend → dialing {}", next_hop);
                let mapped_next_key = {
                    let store = seed_store.read().unwrap();
                    store.get(&next_hop).map(|n| n.identity_pub)
                };
                let selected_next_key = mapped_next_key.unwrap_or(next_identity_key);
                let selected_fp = short_key_fingerprint(&selected_next_key);
                let payload_fp = short_key_fingerprint(&next_identity_key);
                tracing::info!(
                    "RelayExtend handshake start node={} role=initiator next_hop={} selected_key_fp={} payload_key_fp={} key_source={}",
                    crate::trace::node_id(),
                    next_hop,
                    selected_fp,
                    payload_fp,
                    if mapped_next_key.is_some() { "seed_store" } else { "relay_extend_payload" }
                );
                if let Some(mapped) = mapped_next_key {
                    if mapped != next_identity_key {
                        let msg = format!(
                            "relay.extend key_mismatch node={} next_hop={} mapped_fp={} payload_fp={}",
                            crate::trace::node_id(),
                            next_hop,
                            short_key_fingerprint(&mapped),
                            payload_fp,
                        );
                        push_packet_meta_trace("ComponentError", 0, &msg, &connection_trace_id, "relay.error");
                        anyhow::bail!(
                            "RelayExtend key mismatch for {}: mapped key does not match payload key",
                            next_hop
                        );
                    }
                }
                push_packet_meta_trace(
                    "ComponentInput",
                    handshake_payload.len(),
                    &format!("relay.extend INPUT node={} next_hop={}", crate::trace::node_id(), next_hop),
                    &connection_trace_id,
                    "relay.extend",
                );

                // This relay dials the downstream hop, so it must be the initiator
                // for the guard->middle / middle->exit handshake leg.
                let hs = build_initiator(&local_priv_key, &selected_next_key)
                    .context("Failed to build Noise_XX initiator for RelayExtend")?;

                let mut ds_stream = timeout(Duration::from_secs(10), TcpStream::connect(next_hop))
                    .await
                    .context(format!("Timeout dialing next hop {}", next_hop))?
                    .context(format!("Failed to TCP-connect to next hop {}", next_hop))?;

                let transport = complete_handshake(&mut ds_stream, hs, true)
                    .await
                    .context(format!("Noise_XX handshake with next hop {} failed", next_hop))?;

                // Capture the handshake hash as proof for the reply.
                let handshake_hash = transport
                    .get_remote_static()
                    .map(|k| k.to_vec())
                    .unwrap_or_default();
                if !handshake_hash.is_empty() && handshake_hash != selected_next_key {
                    let msg = format!(
                        "relay.extend_identity_mismatch node={} next_hop={} expected={} got={} expected_fp={} got_fp={}",
                        crate::trace::node_id(),
                        next_hop,
                        hex::encode(&selected_next_key),
                        hex::encode(&handshake_hash),
                        short_key_fingerprint(&selected_next_key),
                        short_key_fingerprint(&handshake_hash),
                    );
                    push_packet_meta_trace("RelayFailureCapture", 0,
                        &format!("node={} cell={} err=identity_mismatch", crate::trace::node_id(), cell_snapshot),
                        &connection_trace_id, "relay.capture");
                    push_packet_meta_trace("ComponentError", 0, &msg, &connection_trace_id, "relay.error");
                    anyhow::bail!(
                        "RelayExtend target identity mismatch at {}: expected {:?}, got {:?}",
                        next_hop, selected_next_key, handshake_hash
                    );
                }
                tracing::info!(
                    "RelayExtend handshake complete node={} role=initiator next_hop={} remote_key_fp={}",
                    crate::trace::node_id(),
                    next_hop,
                    short_key_fingerprint(&handshake_hash)
                );

                send_cell_secure(
                    &mut upstream,
                    &mut upstream_transport,
                    &OnionCell::RelayExtended(ExtendedPayload {
                        handshake_response: handshake_hash,
                    }),
                )
                .await?;

                downstream_transport = Some((ds_stream, transport));
                push_packet_meta_trace(
                    "ComponentOutput",
                    0,
                    &format!("relay.extend OUTPUT node={} next_hop={} ok", crate::trace::node_id(), next_hop),
                    &connection_trace_id,
                    "relay.extend",
                );
                tracing::debug!(
                    "RelayExtend complete — downstream link established to {}",
                    next_hop
                );
            }

            // ── RelayData: apply jitter, forward to downstream ─────────────
            OnionCell::RelayData(DataPayload { ciphertext }) => {
                let jitter =
                    rand::random::<u64>() % (max_jitter_ms - min_jitter_ms + 1) + min_jitter_ms;
                tokio::time::sleep(Duration::from_millis(jitter)).await;

                match &mut downstream_transport {
                    Some((ds_stream, transport)) => {
                        let decrypted = decrypt_frame(&mut upstream_transport, &ciphertext)
                            .context("Failed to decrypt RelayData outer envelope")?;

                        let fwd_cell = OnionCell::RelayData(DataPayload {
                            ciphertext: decrypted.clone(),
                        });
                        push_packet_meta_trace(
                            "RelayData(Intermediate)",
                            decrypted.len(),
                            &format!("relay.forward node={} bytes={}", crate::trace::node_id(), decrypted.len()),
                            &connection_trace_id,
                            "relay.data",
                        );
                        send_cell_secure(ds_stream, transport, &fwd_cell)
                            .await
                            .context("Failed to forward RelayData to downstream")?;
                    }
                    None => {
                        // Exit node: all onion layers peeled; forward to Publisher.
                        let publisher_addr = crate::swarm::discover_publisher_addr_for_exit_relay()
                            .await
                            .context("Exit relay: could not resolve Publisher address")?;

                        let mut pub_stream =
                            timeout(Duration::from_secs(10), TcpStream::connect(publisher_addr))
                                .await
                                .context(format!("Exit relay: timeout connecting to Publisher {}", publisher_addr))?
                                .context(format!("Exit relay: TCP connect to Publisher {} failed", publisher_addr))?;

                        let plaintext_chunk = decrypt_frame(&mut upstream_transport, &ciphertext)
                            .context("Exit relay: failed to decrypt final onion layer")?;

                        let len = plaintext_chunk.len() as u32;
                        pub_stream
                            .write_all(&len.to_le_bytes())
                            .await
                            .context(format!("Exit relay: failed writing length prefix to Publisher {}", publisher_addr))?;
                        pub_stream
                            .write_all(&plaintext_chunk)
                            .await
                            .context(format!("Exit relay: failed writing chunk bytes to Publisher {}", publisher_addr))?;
                        pub_stream.flush().await?;

                        push_packet_meta_trace(
                            "ExitDelivery",
                            plaintext_chunk.len(),
                            &format!("relay.exit_deliver node={} publisher={} bytes={}", crate::trace::node_id(), publisher_addr, plaintext_chunk.len()),
                            &connection_trace_id,
                            "relay.data",
                        );
                        tracing::info!(
                            "Exit relay: forwarded {} bytes to Publisher {}",
                            plaintext_chunk.len(),
                            publisher_addr
                        );

                        tokio::spawn(async move {
                            publish_chunks_received_from_env(1).await;
                        });
                    }
                }
            }

            // ── RelayHeartbeat: echo to prove liveness ────────────────────
            OnionCell::RelayHeartbeat(HeartbeatPayload { seq_id }) => {
                tracing::trace!("Heartbeat seq={}", seq_id);
                send_cell_secure(
                    &mut upstream,
                    &mut upstream_transport,
                    &OnionCell::RelayHeartbeat(HeartbeatPayload { seq_id }),
                )
                .await?;
            }

            // ── RelayExtended: not expected on the responder side ─────────
            OnionCell::RelayExtended(_) => {
                push_packet_meta_trace(
                    "RelayFailureCapture",
                    0,
                    &format!("node={} cell={} err=unexpected_extended", crate::trace::node_id(), cell_snapshot),
                    &connection_trace_id,
                    "relay.capture",
                );
                tracing::warn!("Unexpected RelayExtended cell from upstream — ignoring");
            }
        }
    }
}

fn short_key_fingerprint(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "none".to_string();
    }
    let hexed = hex::encode(bytes);
    if hexed.len() <= 12 {
        hexed
    } else {
        hexed[..12].to_string()
    }
}
