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

use crate::observability::publish_chunks_received_from_env;
use anyhow::{Context, Result};
use gbn_protocol::onion::{
    DataPayload, ExtendPayload, ExtendedPayload, HeartbeatPayload, OnionCell,
};
use mcn_crypto::noise::{build_responder, complete_handshake, decrypt_frame};
use std::{net::SocketAddr, time::Duration};
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
                            let min_j = min_jitter_ms;
                            let max_j = max_jitter_ms;
                            tokio::spawn(async move {
                                if let Err(e) = handle_onion_connection(
                                    stream, key, min_j, max_j
                                ).await {
                                    tracing::error!("OnionRelay {} connection error: {}", bound_addr, e);
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
    min_jitter_ms: u64,
    max_jitter_ms: u64,
) -> Result<()> {
    // Optional downstream session: set when we receive a RelayExtend.
    let mut downstream_transport: Option<(TcpStream, snow::TransportState)> = None;

    loop {
        let cell = timeout(Duration::from_secs(30), recv_cell(&mut upstream))
            .await
            .context("Timeout waiting for OnionCell from upstream")?
            .context("Failed to read OnionCell from upstream")?;

        match cell {
            // ── RelayExtend: dial next hop, perform inner handshake ────────
            OnionCell::RelayExtend(ExtendPayload {
                next_hop,
                next_identity_key,
                handshake_payload,
            }) => {
                tracing::debug!("RelayExtend → dialing {}", next_hop);

                // This relay is the responder for the next hop because it has already
                // forwarded the creator-side initiator payload above.
                let hs = build_responder(&local_priv_key)
                    .context("Failed to build Noise_XX responder for RelayExtend")?;

                let mut ds_stream = timeout(Duration::from_secs(10), TcpStream::connect(next_hop))
                    .await
                    .context("Timeout dialing next hop")?
                    .context("Failed to connect to next hop")?;

                // Send the initiator's first handshake payload forwarded by the Creator.
                ds_stream
                    .write_all(&(handshake_payload.len() as u32).to_le_bytes())
                    .await?;
                ds_stream.write_all(&handshake_payload).await?;
                ds_stream.flush().await?;

                // Complete the rest of the handshake.
                let transport = complete_handshake(&mut ds_stream, hs, false)
                    .await
                    .context("Noise_XX handshake with next hop failed")?;

                // Capture the handshake hash as proof for the reply.
                let handshake_hash = transport
                    .get_remote_static()
                    .map(|k| k.to_vec())
                    .unwrap_or_default();
                if !handshake_hash.is_empty() && handshake_hash != next_identity_key {
                    anyhow::bail!(
                        "RelayExtend target identity mismatch: expected {:?}, got {:?}",
                        next_identity_key,
                        handshake_hash
                    );
                }

                // Reply upstream: RelayExtended carries the next-hop's public key bytes
                // (proof the handshake really succeeded).
                send_cell(
                    &mut upstream,
                    &OnionCell::RelayExtended(ExtendedPayload {
                        handshake_response: handshake_hash,
                    }),
                )
                .await?;

                downstream_transport = Some((ds_stream, transport));
                tracing::debug!(
                    "RelayExtend complete — downstream link established to {}",
                    next_hop
                );
            }

            // ── RelayData: apply jitter, forward to downstream ─────────────
            OnionCell::RelayData(DataPayload { ciphertext }) => {
                // Introduce simulated network jitter
                let jitter =
                    rand::random::<u64>() % (max_jitter_ms - min_jitter_ms + 1) + min_jitter_ms;
                tokio::time::sleep(Duration::from_millis(jitter)).await;

                match &mut downstream_transport {
                    Some((ds_stream, transport)) => {
                        // Decrypt the outer layer (this relay's envelope), then re-forward
                        let decrypted = decrypt_frame(transport, &ciphertext)
                            .context("Failed to decrypt RelayData outer envelope")?;

                        // The inner content is itself an opaque blob for the next hop
                        let fwd_cell = OnionCell::RelayData(DataPayload {
                            ciphertext: decrypted,
                        });
                        send_cell(ds_stream, &fwd_cell)
                            .await
                            .context("Failed to forward RelayData to downstream")?;
                    }
                    None => {
                        // Exit node: all 3 Noise_XX onion layers have been peeled.
                        // `ciphertext` is the application-encrypted EncryptedChunkPacket bytes
                        // (still opaque to this relay — Creator→Publisher ECDH encryption).
                        // Forward to the Publisher's mpub-receiver using 4-byte LE length prefix framing.

                        let publisher_addr = crate::swarm::discover_publisher_addr_for_exit_relay()
                            .await
                            .context("Exit relay: could not resolve Publisher address")?;

                        let mut pub_stream =
                            timeout(Duration::from_secs(10), TcpStream::connect(publisher_addr))
                                .await
                                .context("Exit relay: timeout connecting to Publisher")?
                                .context("Exit relay: TCP connect to Publisher failed")?;

                        // 4-byte LE length prefix + chunk bytes
                        // Matches mpub_receiver::recv_raw_frame() framing exactly
                        let len = ciphertext.len() as u32;
                        pub_stream
                            .write_all(&len.to_le_bytes())
                            .await
                            .context("Exit relay: failed writing length prefix")?;
                        pub_stream
                            .write_all(&ciphertext)
                            .await
                            .context("Exit relay: failed writing chunk bytes")?;
                        pub_stream.flush().await?;

                        tracing::info!(
                            "Exit relay: forwarded {} bytes to Publisher {}",
                            ciphertext.len(),
                            publisher_addr
                        );

                        // Fire-and-forget ChunksReceived metric
                        tokio::spawn(async move {
                            publish_chunks_received_from_env(1).await;
                        });
                    }
                }
            }

            // ── RelayHeartbeat: echo to prove liveness ────────────────────
            OnionCell::RelayHeartbeat(HeartbeatPayload { seq_id }) => {
                tracing::trace!("Heartbeat seq={}", seq_id);
                send_cell(
                    &mut upstream,
                    &OnionCell::RelayHeartbeat(HeartbeatPayload { seq_id }),
                )
                .await?;
            }

            // ── RelayExtended: not expected on the responder side ─────────
            OnionCell::RelayExtended(_) => {
                tracing::warn!("Unexpected RelayExtended cell from upstream — ignoring");
            }
        }
    }
}
