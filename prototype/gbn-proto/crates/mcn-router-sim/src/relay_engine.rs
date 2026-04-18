//! # Onion Relay Engine
//!
//! Relay behavior is decrypt-and-forward:
//! - Read one framed ciphertext from upstream.
//! - Open one onion layer with local static key (Noise_N).
//! - If `next_hop` exists, forward inner bytes to that hop and relay ACK back.
//! - If `next_hop` is `None`, process terminal `ChunkPayload` and return ACK.

use crate::control::push_packet_meta_trace;
use crate::observability::publish_chunks_received_from_env;
use anyhow::{Context, Result};
use gbn_protocol::onion::{AckPayload, ChunkPayload, OnionLayer};
use mcn_crypto::noise::{open, seal};
use std::{net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
    time::timeout,
};

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

/// Write one raw frame: `[u32_be_len][bytes]`.
pub async fn write_raw_frame(stream: &mut TcpStream, data: &[u8]) -> Result<()> {
    let len = data.len() as u32;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(data).await?;
    stream.flush().await?;
    Ok(())
}

/// Read one raw frame: `[u32_be_len][bytes]`.
pub async fn read_raw_frame(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Spawn the onion relay listener.
pub async fn spawn_onion_relay(
    listen_addr: SocketAddr,
    local_priv_key: [u8; 32],
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
                            let key = local_priv_key;
                            tokio::spawn(async move {
                                if let Err(e) = handle_onion_connection(stream, key).await {
                                    let msg = format!(
                                        "relay.handle_connection ERROR node={} listen={} peer={} err={e:#}",
                                        crate::trace::node_id(), bound_addr, peer_addr
                                    );
                                    tracing::error!("{}", msg);
                                    push_packet_meta_trace("ComponentError", 0, &msg, "", "relay.error");
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

async fn handle_onion_connection(mut upstream: TcpStream, local_priv_key: [u8; 32]) -> Result<()> {
    let encrypted_layer = timeout(Duration::from_secs(30), read_raw_frame(&mut upstream))
        .await
        .context("Timeout reading upstream raw frame")?
        .context("Failed reading upstream raw frame")?;

    let layer_plain = open(&local_priv_key, &encrypted_layer)
        .context("Failed to open onion layer with local key")?;
    let layer: OnionLayer =
        serde_json::from_slice(&layer_plain).context("Failed to decode OnionLayer")?;

    match layer.next_hop {
        Some(next_hop) => {
            push_packet_meta_trace(
                "RelayData(Intermediate)",
                layer.inner.len(),
                &format!(
                    "relay.forward node={} next_hop={} bytes={}",
                    crate::trace::node_id(),
                    next_hop,
                    layer.inner.len()
                ),
                "",
                "relay.data",
            );

            let mut next_stream = timeout(Duration::from_secs(10), TcpStream::connect(next_hop))
                .await
                .context(format!("Timeout dialing next hop {}", next_hop))?
                .context(format!("Failed connecting to next hop {}", next_hop))?;

            write_raw_frame(&mut next_stream, &layer.inner)
                .await
                .context("Failed writing inner onion bytes to next hop")?;

            let ack_from_downstream = timeout(Duration::from_secs(30), read_raw_frame(&mut next_stream))
                .await
                .context("Timeout waiting for downstream ACK frame")?
                .context("Failed reading downstream ACK frame")?;

            // Peel one ACK layer (reverse onion) before relaying upstream.
            let ack_to_upstream = peel_ack_for_upstream(&local_priv_key, &ack_from_downstream)
                .unwrap_or(ack_from_downstream);

            write_raw_frame(&mut upstream, &ack_to_upstream)
                .await
                .context("Failed relaying ACK upstream")?;
        }
        None => {
            // Terminal destination for this onion message.
            let payload: ChunkPayload =
                serde_json::from_slice(&layer.inner).context("Failed to decode ChunkPayload")?;

            let got_hash = *blake3::hash(&payload.chunk).as_bytes();
            if got_hash != payload.hash {
                anyhow::bail!(
                    "Chunk hash mismatch at destination: expected {}, got {}",
                    hex::encode(payload.hash),
                    hex::encode(got_hash)
                );
            }

            // Compatibility bridge: in existing Phase-2 flow, terminal relay still
            // forwards chunk bytes to mpub-receiver when publisher address is known.
            if let Ok(publisher_addr) = crate::swarm::discover_publisher_addr_for_exit_relay().await {
                if let Err(e) = forward_terminal_chunk_to_publisher(publisher_addr, &payload.chunk).await {
                    tracing::warn!(
                        "Destination relay failed forwarding chunk to Publisher {}: {e:#}",
                        publisher_addr
                    );
                }
            }

            push_packet_meta_trace(
                "ExitDelivery",
                payload.chunk.len(),
                &format!(
                    "relay.destination node={} chunk_id={} bytes={}",
                    crate::trace::node_id(),
                    payload.chunk_id,
                    payload.chunk.len()
                ),
                "",
                "relay.data",
            );

            tokio::spawn(async move {
                publish_chunks_received_from_env(1).await;
            });

            let ack = build_ack_onion(&payload)?;
            write_raw_frame(&mut upstream, &ack)
                .await
                .context("Failed writing terminal ACK to upstream")?;
        }
    }

    Ok(())
}

fn peel_ack_for_upstream(local_priv_key: &[u8; 32], ack_frame: &[u8]) -> Option<Vec<u8>> {
    let opened = open(local_priv_key, ack_frame).ok()?;
    let layer: OnionLayer = serde_json::from_slice(&opened).ok()?;
    Some(layer.inner)
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Build a reverse-direction layered ACK.
///
/// `return_path` format is:
/// `[Creator, Guard, Middle, Exit, ... , Destination]`
///
/// The destination (current node) wraps for each prior hop in reverse order so
/// the ACK can be peeled hop-by-hop on the way back to the creator.
fn build_ack_onion(payload: &ChunkPayload) -> Result<Vec<u8>> {
    anyhow::ensure!(
        !payload.return_path.is_empty(),
        "ACK build failed: empty return_path"
    );

    let ack = AckPayload {
        chunk_id: payload.chunk_id,
        hash: payload.hash,
        send_timestamp_ms: payload.send_timestamp_ms,
        received_timestamp_ms: now_millis(),
        total_chunks: payload.total_chunks,
        chunk_index: payload.chunk_index,
    };
    let ack_bytes = serde_json::to_vec(&ack)?;

    let creator = &payload.return_path[0];
    let creator_layer = OnionLayer {
        next_hop: None,
        inner: ack_bytes,
    };
    let creator_layer_bytes = serde_json::to_vec(&creator_layer)?;
    let mut sealed = seal(&creator.identity_pub, &creator_layer_bytes)?;

    let dest_idx = payload.return_path.len().saturating_sub(1);
    for idx in (1..dest_idx).rev() {
        let current = &payload.return_path[idx];
        let next_addr = payload.return_path[idx - 1].addr;
        let layer = OnionLayer {
            next_hop: Some(next_addr),
            inner: sealed,
        };
        let layer_bytes = serde_json::to_vec(&layer)?;
        sealed = seal(&current.identity_pub, &layer_bytes)?;
    }

    Ok(sealed)
}

async fn forward_terminal_chunk_to_publisher(
    publisher_addr: SocketAddr,
    chunk_bytes: &[u8],
) -> Result<()> {
    let mut pub_stream = timeout(Duration::from_secs(10), TcpStream::connect(publisher_addr))
        .await
        .context(format!(
            "Timeout connecting to Publisher {} from destination relay",
            publisher_addr
        ))?
        .context(format!(
            "TCP connect to Publisher {} failed from destination relay",
            publisher_addr
        ))?;

    // mpub-receiver wire format uses little-endian length prefix.
    let len = chunk_bytes.len() as u32;
    pub_stream
        .write_all(&len.to_le_bytes())
        .await
        .context("Failed writing publisher length prefix")?;
    pub_stream
        .write_all(chunk_bytes)
        .await
        .context("Failed writing publisher chunk bytes")?;
    pub_stream.flush().await?;
    Ok(())
}

