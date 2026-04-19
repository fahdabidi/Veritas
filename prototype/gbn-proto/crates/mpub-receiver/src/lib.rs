//! # MPub Receiver
//!
//! Publisher-side component that receives encrypted chunks from the MCN relay
//! network, buffers them, performs ECDH key derivation, decrypts each chunk,
//! verifies BLAKE3 integrity, and reassembles the original video.
//!
//! ## Sentinel Frame Protocol
//!
//! Before sending data chunks through an onion circuit, the Creator sends a
//! sentinel frame carrying the `UploadSessionInit` (Creator's ephemeral X25519
//! pubkey required for Publisher decryption). The sentinel is identified by a
//! 7-byte magic prefix `b"GBNINIT"` followed by JSON-serialised `UploadSessionInit`.
//! The Publisher stores the init keyed by `session_id` for later use during
//! chunk reassembly.
// Workaround for rustc 1.94.1 ICE in `check_mod_deathness` (dead-code liveness
// analysis panics on `pub type` + async fn combinations in this crate).
// Remove this once the toolchain is upgraded beyond the affected version.
#![allow(dead_code)]

use std::{collections::HashMap, net::SocketAddr, path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result as AnyResult};
use gbn_protocol::{
    chunk::{ChunkManifest, EncryptedChunkPacket, SessionId},
    crypto::UploadSessionInit,
    error::ProtocolError,
    onion::{AckPayload, ChunkPayload, HopInfo, OnionLayer},
};

use mcn_chunker::{reassemble_chunks, verify_chunk_hash, ChunkerError};
use mcn_crypto::{
    decrypt_chunk,
    noise::{open, seal},
    PublisherSecret,
};
use mcn_router_sim::control::push_packet_meta_trace;
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, Mutex},
    task::JoinHandle,
};

/// Magic prefix identifying a sentinel `UploadSessionInit` frame.
/// Sent through the onion circuit before data chunks so the Publisher
/// can reconstruct the session key for decryption.
pub const SENTINEL_MAGIC: &[u8] = b"GBNINIT";

/// chunk_index value used for sentinel frames (never a real chunk index).
pub const SENTINEL_CHUNK_INDEX: u32 = u32::MAX;

/// Thread-safe map of SessionId → UploadSessionInit,
/// populated by sentinel frames and consumed during reassembly.
pub type SessionInitStore = Arc<Mutex<HashMap<SessionId, UploadSessionInit>>>;

#[derive(Debug, Error)]
pub enum ReceiverError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    #[error("Chunker error: {0}")]
    Chunker(#[from] ChunkerError),

    #[error("Session {0} timed out waiting for chunks")]
    Timeout(String),

    #[error("Missing chunk {0} in session")]
    MissingChunk(u32),

    #[error("Incomplete session: got {0} chunks, expected {1}")]
    IncompleteSession(u32, u32),

    #[error("Chunk {0} failed BLAKE3 verification")]
    Blake3VerificationFailed(u32),

    #[error("SHA-256 verification of the completely reassembled file failed")]
    OverallVerificationFailed,

    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

// ─────────────────────────── Network Protocol ──────────────────────────────

/// Read a raw length-prefixed frame from a TCP stream.
/// Returns the raw bytes (before any JSON parsing) so callers can
/// inspect for sentinel magic prefix before parsing.
async fn recv_raw_frame_le(stream: &mut TcpStream) -> Result<Vec<u8>, ReceiverError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut data = vec![0u8; len];
    stream.read_exact(&mut data).await?;
    Ok(data)
}

/// Read a raw big-endian length-prefixed frame from a TCP stream.
async fn recv_raw_frame_be(stream: &mut TcpStream) -> Result<Vec<u8>, ReceiverError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut data = vec![0u8; len];
    stream.read_exact(&mut data).await?;
    Ok(data)
}

async fn write_raw_frame_be(stream: &mut TcpStream, data: &[u8]) -> Result<(), ReceiverError> {
    let len = data.len() as u32;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(data).await?;
    stream.flush().await?;
    Ok(())
}

fn trace_next_chain(parent: &str) -> String {
    let hop = mcn_router_sim::trace::next_hop_id();
    if parent.is_empty() {
        hop
    } else if hop.is_empty() {
        parent.to_string()
    } else {
        format!("{parent} -> {hop}")
    }
}

/// Receive a packet from a TCP stream using length-prefix framing.
async fn recv_packet(stream: &mut TcpStream) -> Result<EncryptedChunkPacket, ReceiverError> {
    let data = recv_raw_frame_le(stream).await?;
    let packet = serde_json::from_slice(&data)?;
    Ok(packet)
}

#[derive(Clone, Copy)]
enum ReceiverMode {
    PlainFrame,
    OnionTerminal { local_priv_key: [u8; 32] },
}

// ─────────────────────────── Receiver ──────────────────────────────────────

pub struct Receiver {
    listen_addrs: Vec<SocketAddr>,
    mode: ReceiverMode,
}

#[derive(Clone)]
struct ServerSharedState {
    // Maps SessionId -> (total_chunks_expected, Map<chunk_index, Packet>)
    sessions: Arc<Mutex<HashMap<SessionId, (u32, HashMap<u32, EncryptedChunkPacket>)>>>,
    // Channel to notify when a session is complete
    completed_tx: mpsc::Sender<CompletedSession>,
    // Maps SessionId -> UploadSessionInit (from sentinel frames)
    pub session_inits: SessionInitStore,
    mode: ReceiverMode,
}

pub struct ReceiverHandle {
    pub bound_addrs: Vec<SocketAddr>,
    pub session_inits: SessionInitStore,
    completed_rx: mpsc::Receiver<CompletedSession>,
    tasks: Vec<JoinHandle<()>>,
}

pub struct CompletedSession {
    pub session_id: SessionId,
    pub packets: HashMap<u32, EncryptedChunkPacket>,
}

impl Receiver {
    pub fn new(listen_addrs: Vec<SocketAddr>) -> Self {
        Self {
            listen_addrs,
            mode: ReceiverMode::PlainFrame,
        }
    }

    pub fn new_onion_terminal(
        listen_addrs: Vec<SocketAddr>,
        local_priv_key: [u8; 32],
    ) -> Self {
        Self {
            listen_addrs,
            mode: ReceiverMode::OnionTerminal { local_priv_key },
        }
    }

    pub async fn start(self) -> Result<ReceiverHandle, ReceiverError> {
        let (completed_tx, completed_rx) = mpsc::channel(100);
        let session_inits: SessionInitStore = Arc::new(Mutex::new(HashMap::new()));
        let shared_state = ServerSharedState {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            completed_tx,
            session_inits: Arc::clone(&session_inits),
            mode: self.mode,
        };

        let mut bound_addrs = Vec::new();
        let mut tasks = Vec::new();

        for addr in self.listen_addrs {
            let listener = TcpListener::bind(addr).await?;
            bound_addrs.push(listener.local_addr()?);
            let state = shared_state.clone();

            let task = tokio::spawn(async move {
                loop {
                    if let Ok((stream, _peer)) = listener.accept().await {
                        let state = state.clone();
                        tokio::spawn(handle_connection(stream, state));
                    }
                }
            });
            tasks.push(task);
        }

        Ok(ReceiverHandle {
            bound_addrs,
            session_inits,
            completed_rx,
            tasks,
        })
    }
}

/// Per-connection handler extracted into a named async function to avoid
/// a rustc ICE in `check_mod_deathness` with complex nested async closures.
async fn handle_connection(mut stream: TcpStream, state: ServerSharedState) {
    match state.mode {
        ReceiverMode::PlainFrame => {
            let frame = match recv_raw_frame_le(&mut stream).await {
                Ok(f) => f,
                Err(_) => return,
            };
            let recv_chain = trace_next_chain("");
            push_packet_meta_trace(
                "ComponentInput",
                frame.len(),
                &format!("publisher.frame INPUT mode=plain bytes={}", frame.len()),
                &recv_chain,
                "publisher.input",
            );
            if let Err(e) = handle_payload_frame(&frame, &state, &recv_chain).await {
                tracing::warn!("Publisher: failed to process plain frame: {e}");
            }
        }
        ReceiverMode::OnionTerminal { local_priv_key } => {
            let frame = match recv_raw_frame_be(&mut stream).await {
                Ok(f) => f,
                Err(_) => return,
            };
            let recv_chain = trace_next_chain("");
            push_packet_meta_trace(
                "ComponentInput",
                frame.len(),
                &format!("publisher.frame INPUT mode=onion bytes={}", frame.len()),
                &recv_chain,
                "publisher.input",
            );
            if let Err(e) =
                handle_onion_terminal_frame(&mut stream, &frame, &state, local_priv_key, &recv_chain).await
            {
                tracing::warn!("Publisher: failed to process onion terminal frame: {e:#}");
            }
        }
    }
}

async fn handle_payload_frame(
    frame: &[u8],
    state: &ServerSharedState,
    parent_chain: &str,
) -> Result<(), ReceiverError> {
    // Sentinel GBNINIT frame carries the UploadSessionInit for decryption
    if frame.starts_with(SENTINEL_MAGIC) {
        let init_chain = trace_next_chain(parent_chain);
        let init_bytes = &frame[SENTINEL_MAGIC.len()..];
        if let Ok(init) = serde_json::from_slice::<UploadSessionInit>(init_bytes) {
            let session_id = init.session_id;
            let total_chunks = init.total_chunks;
            let mut inits = state.session_inits.lock().await;
            inits.insert(session_id, init);
            push_packet_meta_trace(
                "ComponentOutput",
                init_bytes.len(),
                &format!(
                    "publisher.session_init OUTPUT session_id={} total_chunks={}",
                    hex::encode(session_id),
                    total_chunks
                ),
                &init_chain,
                "publisher.session",
            );
            tracing::debug!("Publisher: stored UploadSessionInit for session");
        } else {
            push_packet_meta_trace(
                "ComponentError",
                init_bytes.len(),
                "publisher.session_init ERROR failed to decode UploadSessionInit",
                &init_chain,
                "publisher.error",
            );
        }
        return Ok(());
    }

    let packet = match serde_json::from_slice::<EncryptedChunkPacket>(&frame) {
        Ok(p) => p,
        Err(e) => {
            push_packet_meta_trace(
                "ComponentError",
                frame.len(),
                &format!("publisher.packet ERROR failed to decode packet: {e}"),
                &trace_next_chain(parent_chain),
                "publisher.error",
            );
            return Err(e.into());
        }
    };
    let packet_chain = trace_next_chain(parent_chain);
    push_packet_meta_trace(
        "ComponentInput",
        frame.len(),
        &format!(
            "publisher.packet INPUT session_id={} chunk_index={} total_chunks={}",
            hex::encode(packet.session_id),
            packet.chunk_index,
            packet.total_chunks
        ),
        &packet_chain,
        "publisher.packet",
    );

    let mut sessions = state.sessions.lock().await;
    let session_entry = sessions
        .entry(packet.session_id)
        .or_insert_with(|| (packet.total_chunks, HashMap::new()));

    session_entry.1.insert(packet.chunk_index, packet.clone());

    if session_entry.1.len() as u32 == session_entry.0 {
        let complete = CompletedSession {
            session_id: packet.session_id,
            packets: session_entry.1.clone(),
        };
        let _ = state.completed_tx.send(complete).await;
        push_packet_meta_trace(
            "ComponentOutput",
            frame.len(),
            &format!(
                "publisher.packet OUTPUT session_complete session_id={} received={}/{} chunk_index={}",
                hex::encode(packet.session_id),
                session_entry.1.len(),
                session_entry.0,
                packet.chunk_index
            ),
            &packet_chain,
            "publisher.packet",
        );
    } else {
        push_packet_meta_trace(
            "ComponentOutput",
            frame.len(),
            &format!(
                "publisher.packet OUTPUT stored session_id={} received={}/{} chunk_index={}",
                hex::encode(packet.session_id),
                session_entry.1.len(),
                session_entry.0,
                packet.chunk_index
            ),
            &packet_chain,
            "publisher.packet",
        );
    }

    Ok(())
}

async fn handle_onion_terminal_frame(
    stream: &mut TcpStream,
    frame: &[u8],
    state: &ServerSharedState,
    local_priv_key: [u8; 32],
    root_chain: &str,
) -> Result<(), ReceiverError> {
    let opened = open(&local_priv_key, frame).context("Publisher failed opening onion frame").map_err(|e| {
        push_packet_meta_trace(
            "ComponentError",
            frame.len(),
            &format!("publisher.open_layer ERROR err={e:#}"),
            root_chain,
            "publisher.error",
        );
        e
    })?;
    let layer: OnionLayer =
        serde_json::from_slice(&opened)
            .context("Publisher failed decoding terminal OnionLayer")
            .map_err(|e| {
                push_packet_meta_trace(
                    "ComponentError",
                    opened.len(),
                    &format!("publisher.decode_layer ERROR err={e:#}"),
                    root_chain,
                    "publisher.error",
                );
                e
            })?;
    let incoming_chain = layer.trace_id.clone().unwrap_or_else(|| root_chain.to_string());
    let ingress_chain = trace_next_chain(&incoming_chain);
    push_packet_meta_trace(
        "ComponentInput",
        layer.inner.len(),
        &format!(
            "publisher.layer INPUT next_hop={:?} bytes={}",
            layer.next_hop,
            layer.inner.len()
        ),
        &ingress_chain,
        "publisher.input",
    );
    if layer.next_hop.is_some() {
        push_packet_meta_trace(
            "ComponentError",
            layer.inner.len(),
            &format!(
                "publisher.layer ERROR expected next_hop=None, got {:?}",
                layer.next_hop
            ),
            &ingress_chain,
            "publisher.error",
        );
        return Err(anyhow::anyhow!(
            "Publisher terminal layer expected next_hop=None, got {:?}",
            layer.next_hop
        )
        .into());
    }

    let payload: ChunkPayload =
        serde_json::from_slice(&layer.inner).context("Publisher failed decoding ChunkPayload")?;
    let payload_chain = payload
        .trace_id
        .clone()
        .unwrap_or_else(|| trace_next_chain(&ingress_chain));
    push_packet_meta_trace(
        "ComponentInput",
        payload.chunk.len(),
        &format!(
            "publisher.payload INPUT chunk_id={} chunk_index={} total_chunks={} bytes={}",
            payload.chunk_id,
            payload.chunk_index,
            payload.total_chunks,
            payload.chunk.len()
        ),
        &payload_chain,
        "publisher.payload",
    );
    let got_hash = *blake3::hash(&payload.chunk).as_bytes();
    if got_hash != payload.hash {
        push_packet_meta_trace(
            "ComponentError",
            payload.chunk.len(),
            &format!(
                "publisher.payload ERROR hash_mismatch chunk_id={} expected={} got={}",
                payload.chunk_id,
                hex::encode(payload.hash),
                hex::encode(got_hash)
            ),
            &payload_chain,
            "publisher.error",
        );
        return Err(anyhow::anyhow!(
            "Publisher payload hash mismatch: expected {}, got {}",
            hex::encode(payload.hash),
            hex::encode(got_hash)
        )
        .into());
    }

    let payload_stored = match handle_payload_frame(&payload.chunk, state, &payload_chain).await {
        Ok(()) => {
            push_packet_meta_trace(
                "ComponentOutput",
                payload.chunk.len(),
                &format!(
                    "publisher.payload OUTPUT accepted chunk_id={} chunk_index={} bytes={} mode=stored",
                    payload.chunk_id,
                    payload.chunk_index,
                    payload.chunk.len()
                ),
                &payload_chain,
                "publisher.payload",
            );
            true
        }
        Err(e) => {
            tracing::warn!(
                "Publisher: transport accepted but packet parse/store failed chunk_id={} chunk_index={}: {e}",
                payload.chunk_id,
                payload.chunk_index
            );
            push_packet_meta_trace(
                "ComponentOutput",
                payload.chunk.len(),
                &format!(
                    "publisher.payload OUTPUT accepted chunk_id={} chunk_index={} bytes={} mode=transport_only",
                    payload.chunk_id,
                    payload.chunk_index,
                    payload.chunk.len()
                ),
                &payload_chain,
                "publisher.payload",
            );
            false
        }
    };
    tracing::info!(
        "Publisher: accepted terminal onion payload chunk_id={} chunk_index={} bytes={}",
        payload.chunk_id,
        payload.chunk_index,
        payload.chunk.len()
    );

    let (ack, ack_trace) = build_publisher_ack_onion(&payload)?;
    push_packet_meta_trace(
        "ComponentOutput",
        ack.len(),
        &format!(
            "publisher.ack OUTPUT chunk_id={} chunk_index={} bytes={} payload_stored={}",
            payload.chunk_id,
            payload.chunk_index,
            ack.len(),
            payload_stored
        ),
        &ack_trace,
        "publisher.ack",
    );
    write_raw_frame_be(stream, &ack).await?;
    tracing::info!(
        "Publisher: sent ACK for chunk_id={} chunk_index={} bytes={}",
        payload.chunk_id,
        payload.chunk_index,
        ack.len()
    );
    Ok(())
}

fn build_publisher_ack_onion(payload: &ChunkPayload) -> AnyResult<(Vec<u8>, String)> {
    anyhow::ensure!(
        payload.return_path.len() >= 2,
        "Publisher ACK build failed: return_path must include at least creator and publisher"
    );

    let publisher = payload
        .return_path
        .last()
        .context("Publisher ACK build failed: missing publisher in return_path")?;
    let creator = &payload.return_path[0];
    let ack_origin_trace = append_trace_stage(
        payload.trace_id.as_deref().unwrap_or(""),
        &format!("ack-origin@{}", publisher.addr),
    );
    let creator_trace = response_trace_for_destination(&payload.return_path, &ack_origin_trace, 0);
    let ack = AckPayload {
        chunk_id: payload.chunk_id,
        hash: payload.hash,
        trace_id: if creator_trace.is_empty() {
            None
        } else {
            Some(creator_trace.clone())
        },
        send_timestamp_ms: payload.send_timestamp_ms,
        received_timestamp_ms: now_millis(),
        total_chunks: payload.total_chunks,
        chunk_index: payload.chunk_index,
    };
    let ack_bytes = serde_json::to_vec(&ack)?;

    let creator_layer = OnionLayer {
        next_hop: None,
        inner: ack_bytes,
        trace_id: if creator_trace.is_empty() {
            None
        } else {
            Some(creator_trace.clone())
        },
    };
    let creator_layer_bytes = serde_json::to_vec(&creator_layer)?;
    let mut sealed = seal(&creator.identity_pub, &creator_layer_bytes)?;

    let dest_idx = payload.return_path.len().saturating_sub(1);
    for idx in 1..dest_idx {
        let current = &payload.return_path[idx];
        let next_addr = payload.return_path[idx - 1].addr;
        let layer_trace = response_trace_for_destination(&payload.return_path, &ack_origin_trace, idx);
        let layer = OnionLayer {
            next_hop: Some(next_addr),
            inner: sealed,
            trace_id: if layer_trace.is_empty() {
                None
            } else {
                Some(layer_trace.clone())
            },
        };
        let layer_bytes = serde_json::to_vec(&layer)?;
        sealed = seal(&current.identity_pub, &layer_bytes)?;
    }

    Ok((sealed, creator_trace))
}

fn append_trace_stage(parent: &str, stage: &str) -> String {
    if parent.is_empty() {
        stage.to_string()
    } else if stage.is_empty() {
        parent.to_string()
    } else {
        format!("{parent} -> {stage}")
    }
}

fn response_trace_for_destination(
    return_path: &[HopInfo],
    ack_origin_trace: &str,
    dest_idx: usize,
) -> String {
    if return_path.is_empty() || dest_idx >= return_path.len() {
        return ack_origin_trace.to_string();
    }

    let mut chain = ack_origin_trace.to_string();
    if return_path.len() < 2 {
        return chain;
    }

    let last_relay_idx = return_path.len() - 2;
    for idx in (dest_idx.max(1)..=last_relay_idx).rev() {
        chain = append_trace_stage(&chain, &format!("ack-hop@{}", return_path[idx].addr));
    }
    if dest_idx == 0 {
        chain = append_trace_stage(&chain, &format!("ack-hop@{}", return_path[0].addr));
    }
    chain
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl ReceiverHandle {
    /// Wait for a specific session (by known SessionId) to complete.
    pub async fn await_session(
        &mut self,
        expected_session: SessionId,
        wait_timeout: Duration,
    ) -> Result<CompletedSession, ReceiverError> {
        let deadline = tokio::time::Instant::now() + wait_timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(ReceiverError::Timeout(hex::encode(expected_session)));
            }
            match tokio::time::timeout(remaining, self.completed_rx.recv()).await {
                Ok(Some(session)) if session.session_id == expected_session => return Ok(session),
                Ok(Some(_)) => {} // different session, keep waiting
                Ok(None) | Err(_) => {
                    return Err(ReceiverError::Timeout(hex::encode(expected_session)))
                }
            }
        }
    }

    /// Wait for the **next** completed session, regardless of SessionId.
    ///
    /// Used by the Publisher in ECS where the session ID is not known in advance.
    pub async fn await_any_session(
        &mut self,
        wait_timeout: Duration,
    ) -> Result<CompletedSession, ReceiverError> {
        match tokio::time::timeout(wait_timeout, self.completed_rx.recv()).await {
            Ok(Some(session)) => Ok(session),
            Ok(None) | Err(_) => Err(ReceiverError::Timeout("(unknown)".to_string())),
        }
    }

    pub fn shutdown(self) {
        for task in self.tasks {
            task.abort();
        }
    }
}

impl CompletedSession {
    /// Decrypt all chunks, verify BLAKE3 against manifest, write reassembled file.
    pub fn decrypt_and_reassemble(
        &self,
        output_path: impl AsRef<Path>,
        publisher_secret: &PublisherSecret,
        session_init: &UploadSessionInit,
        manifest: &ChunkManifest,
    ) -> Result<(), ReceiverError> {
        let expected_chunks = manifest.total_chunks;
        if self.packets.len() as u32 != expected_chunks {
            return Err(ReceiverError::IncompleteSession(
                self.packets.len() as u32,
                expected_chunks,
            ));
        }

        let mut decrypted_chunks: Vec<Vec<u8>> = vec![Vec::new(); expected_chunks as usize];

        for i in 0..expected_chunks {
            let p = self.packets.get(&i).ok_or(ReceiverError::MissingChunk(i))?;

            // Decrypt
            let plaintext = decrypt_chunk(publisher_secret, session_init, p)?;

            // Verify BLAKE3 against manifest
            let expected_hash = manifest
                .chunks
                .iter()
                .find(|c| c.index == i)
                .map(|c| c.hash)
                .unwrap_or([0u8; 32]);
            if !verify_chunk_hash(&plaintext, &expected_hash) {
                return Err(ReceiverError::Blake3VerificationFailed(i));
            }

            decrypted_chunks[i as usize] = plaintext;
        }

        // Reassemble
        reassemble_chunks(&decrypted_chunks, manifest, output_path)?;

        Ok(())
    }

    pub fn verify(
        &self,
        original_hash: [u8; 32],
        reassembled_path: impl AsRef<Path>,
    ) -> Result<bool, ReceiverError> {
        let actual = mcn_chunker::hash_file(reassembled_path)?;
        Ok(actual == original_hash)
    }
}

// ─────────────────────────────── Tests ───────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mcn_chunker::chunk_file;
    use mcn_crypto::{create_upload_session, generate_publisher_keypair};
    use std::io::Write;
    use tempfile::NamedTempFile;
    use tokio::io::AsyncWriteExt;

    async fn send_packet_test(addr: SocketAddr, packet: &EncryptedChunkPacket) {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let json = serde_json::to_vec(packet).unwrap();
        let len = json.len() as u32;
        stream.write_all(&len.to_le_bytes()).await.unwrap();
        stream.write_all(&json).await.unwrap();
        stream.flush().await.unwrap();
    }

    #[tokio::test]
    async fn test_receive_in_order() {
        // 1. Setup keys and file
        let (pub_secret, pub_key) = generate_publisher_keypair();

        let content: Vec<u8> = (0u8..=255).cycle().take(5000).collect();
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(&content).unwrap();
        f.flush().unwrap();
        let original_hash = mcn_chunker::hash_file(f.path()).unwrap();

        let (chunks, manifest) = chunk_file(f.path(), 1024).unwrap();
        let session =
            create_upload_session(&pub_key, manifest.total_chunks, original_hash).unwrap();

        let expected_session = session.session_init.session_id;

        // 2. Setup receiver
        let receiver = Receiver::new(vec!["127.0.0.1:0".parse().unwrap()]);
        let mut handle = receiver.start().await.unwrap();
        let addr = handle.bound_addrs[0];

        // 3. Send chunks in order
        for i in 0..manifest.total_chunks {
            let data = &chunks[i as usize];
            let hash = manifest.chunks[i as usize].hash;
            let mut packet = session.encrypt_chunk(i, data, hash).unwrap();
            packet.session_id = expected_session; // Sync with manifest
            send_packet_test(addr, &packet).await;
        }

        // 4. Await and reassemble
        let completed = handle
            .await_session(expected_session, Duration::from_secs(2))
            .await
            .unwrap();

        let out = NamedTempFile::new().unwrap();
        completed
            .decrypt_and_reassemble(out.path(), &pub_secret, &session.session_init, &manifest)
            .unwrap();

        assert!(completed.verify(original_hash, out.path()).unwrap());
    }

    #[tokio::test]
    async fn test_receive_out_of_order() {
        let (pub_secret, pub_key) = generate_publisher_keypair();

        let content: Vec<u8> = (0u8..=100).cycle().take(3000).collect();
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(&content).unwrap();
        f.flush().unwrap();
        let original_hash = mcn_chunker::hash_file(f.path()).unwrap();

        let (chunks, manifest) = chunk_file(f.path(), 1024).unwrap();
        let session =
            create_upload_session(&pub_key, manifest.total_chunks, original_hash).unwrap();
        let expected_session = session.session_init.session_id;

        let receiver = Receiver::new(vec!["127.0.0.1:0".parse().unwrap()]);
        let mut handle = receiver.start().await.unwrap();
        let addr = handle.bound_addrs[0];

        // Send out of order (2, 0, 1)
        let order = vec![2, 0, 1];
        for i in order {
            if i < manifest.total_chunks {
                let data = &chunks[i as usize];
                let hash = manifest.chunks[i as usize].hash;
                let mut packet = session.encrypt_chunk(i, data, hash).unwrap();
                packet.session_id = expected_session;
                send_packet_test(addr, &packet).await;
            }
        }

        let completed = handle
            .await_session(expected_session, Duration::from_secs(2))
            .await
            .unwrap();

        let out = NamedTempFile::new().unwrap();
        completed
            .decrypt_and_reassemble(out.path(), &pub_secret, &session.session_init, &manifest)
            .unwrap();
        assert!(completed.verify(original_hash, out.path()).unwrap());
    }

    #[tokio::test]
    async fn test_receive_multiport() {
        let (pub_secret, pub_key) = generate_publisher_keypair();

        let content: Vec<u8> = (0u8..=50).cycle().take(4000).collect();
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(&content).unwrap();
        f.flush().unwrap();
        let original_hash = mcn_chunker::hash_file(f.path()).unwrap();

        let (chunks, manifest) = chunk_file(f.path(), 1024).unwrap();
        let session =
            create_upload_session(&pub_key, manifest.total_chunks, original_hash).unwrap();
        let expected_session = session.session_init.session_id;

        // Receiver with 3 ports
        let receiver = Receiver::new(vec![
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.1:0".parse().unwrap(),
        ]);
        let mut handle = receiver.start().await.unwrap();

        for i in 0..manifest.total_chunks {
            let data = &chunks[i as usize];
            let hash = manifest.chunks[i as usize].hash;
            let mut packet = session.encrypt_chunk(i, data, hash).unwrap();
            packet.session_id = expected_session;

            // Cycle through available bound addresses
            let addr = handle.bound_addrs[(i as usize) % handle.bound_addrs.len()];
            send_packet_test(addr, &packet).await;
        }

        let completed = handle
            .await_session(expected_session, Duration::from_secs(2))
            .await
            .unwrap();
        let out = NamedTempFile::new().unwrap();
        completed
            .decrypt_and_reassemble(out.path(), &pub_secret, &session.session_init, &manifest)
            .unwrap();
        assert!(completed.verify(original_hash, out.path()).unwrap());
    }
}
