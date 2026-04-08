//! # MPub Receiver
//!
//! Publisher-side component that receives encrypted chunks from the MCN relay
//! network, buffers them, performs ECDH key derivation, decrypts each chunk,
//! verifies BLAKE3 integrity, and reassembles the original video.

use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use gbn_protocol::{
    chunk::{ChunkManifest, EncryptedChunkPacket, SessionId},
    crypto::UploadSessionInit,
    error::ProtocolError,
};
use mcn_chunker::{ChunkerError, reassemble_chunks, verify_chunk_hash};
use mcn_crypto::{decrypt_chunk, PublisherSecret};
use thiserror::Error;
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
    sync::{mpsc, Mutex},
    task::JoinHandle,
    time::timeout,
};

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

    #[error("Session {0:?} timed out waiting for chunks")]
    Timeout(SessionId),

    #[error("Missing chunk {0} in session")]
    MissingChunk(u32),

    #[error("Incomplete session: got {0} chunks, expected {1}")]
    IncompleteSession(u32, u32),

    #[error("Chunk {0} failed BLAKE3 verification")]
    Blake3VerificationFailed(u32),

    #[error("SHA-256 verification of the completely reassembled file failed")]
    OverallVerificationFailed,
}

// ─────────────────────────── Network Protocol ──────────────────────────────

/// Receive a packet from a TCP stream using length-prefix framing.
async fn recv_packet(stream: &mut TcpStream) -> Result<EncryptedChunkPacket, ReceiverError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;

    let mut data = vec![0u8; len];
    stream.read_exact(&mut data).await?;

    let packet = serde_json::from_slice(&data)?;
    Ok(packet)
}

// ─────────────────────────── Receiver ──────────────────────────────────────

pub struct Receiver {
    listen_addrs: Vec<SocketAddr>,
}

#[derive(Clone)]
struct ServerSharedState {
    // Maps SessionId -> (total_chunks_expected, Map<chunk_index, Packet>)
    sessions: Arc<Mutex<HashMap<SessionId, (u32, HashMap<u32, EncryptedChunkPacket>)>>>,
    // Channel to notify when a session is complete
    completed_tx: mpsc::Sender<CompletedSession>,
}

pub struct ReceiverHandle {
    pub bound_addrs: Vec<SocketAddr>,
    completed_rx: mpsc::Receiver<CompletedSession>,
    tasks: Vec<JoinHandle<()>>,
}

pub struct CompletedSession {
    pub session_id: SessionId,
    pub packets: HashMap<u32, EncryptedChunkPacket>,
}

impl Receiver {
    pub fn new(listen_addrs: Vec<SocketAddr>) -> Self {
        Self { listen_addrs }
    }

    pub async fn start(self) -> Result<ReceiverHandle, ReceiverError> {
        let (completed_tx, completed_rx) = mpsc::channel(100);
        let shared_state = ServerSharedState {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            completed_tx,
        };

        let mut bound_addrs = Vec::new();
        let mut tasks = Vec::new();

        for addr in self.listen_addrs {
            let listener = TcpListener::bind(addr).await?;
            bound_addrs.push(listener.local_addr()?);
            let state = shared_state.clone();

            let task = tokio::spawn(async move {
                loop {
                    if let Ok((mut stream, _peer)) = listener.accept().await {
                        let state = state.clone();
                        tokio::spawn(async move {
                            if let Ok(packet) = recv_packet(&mut stream).await {
                                let mut sessions = state.sessions.lock().await;
                                let session_entry = sessions.entry(packet.session_id).or_insert_with(|| {
                                    (packet.total_chunks, HashMap::new())
                                });
                                
                                session_entry.1.insert(packet.chunk_index, packet.clone());

                                // Check if complete
                                if session_entry.1.len() as u32 == session_entry.0 {
                                    let complete = CompletedSession {
                                        session_id: packet.session_id,
                                        packets: session_entry.1.clone(),
                                    };
                                    // Ignore send error if nobody is listening
                                    let _ = state.completed_tx.send(complete).await;
                                }
                            }
                        });
                    }
                }
            });
            tasks.push(task);
        }

        Ok(ReceiverHandle {
            bound_addrs,
            completed_rx,
            tasks,
        })
    }
}

impl ReceiverHandle {
    pub async fn await_session(&mut self, expected_session: SessionId, wait_timeout: Duration) -> Result<CompletedSession, ReceiverError> {
        let res = timeout(wait_timeout, async {
            loop {
                if let Some(session) = self.completed_rx.recv().await {
                    if session.session_id == expected_session {
                        return session;
                    }
                } else {
                    // channel closed
                    std::future::pending::<()>().await;
                }
            }
        }).await;

        match res {
            Ok(session) => Ok(session),
            Err(_) => Err(ReceiverError::Timeout(expected_session)),
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
            return Err(ReceiverError::IncompleteSession(self.packets.len() as u32, expected_chunks));
        }

        let mut decrypted_chunks: Vec<Vec<u8>> = vec![Vec::new(); expected_chunks as usize];

        for i in 0..expected_chunks {
            let p = self.packets.get(&i).ok_or(ReceiverError::MissingChunk(i))?;
            
            // Decrypt
            let plaintext = decrypt_chunk(publisher_secret, session_init, p)?;

            // Verify BLAKE3 against manifest
            let expected_hash = manifest.chunks.iter().find(|c| c.index == i).map(|c| c.hash).unwrap_or([0u8; 32]);
            if !verify_chunk_hash(&plaintext, &expected_hash) {
                return Err(ReceiverError::Blake3VerificationFailed(i));
            }

            decrypted_chunks[i as usize] = plaintext;
        }

        // Reassemble
        reassemble_chunks(&decrypted_chunks, manifest, output_path)?;

        Ok(())
    }

    pub fn verify(&self, original_hash: [u8; 32], reassembled_path: impl AsRef<Path>) -> Result<bool, ReceiverError> {
        let actual = mcn_chunker::hash_file(reassembled_path)?;
        Ok(actual == original_hash)
    }
}

// ─────────────────────────────── Tests ───────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mcn_crypto::{generate_publisher_keypair, create_upload_session};
    use mcn_chunker::chunk_file;
    use tempfile::NamedTempFile;
    use tokio::io::AsyncWriteExt;
    use std::io::Write;

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
        let session = create_upload_session(&pub_key, manifest.total_chunks, original_hash).unwrap();
        
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
        let completed = handle.await_session(expected_session, Duration::from_secs(2)).await.unwrap();
        
        let out = NamedTempFile::new().unwrap();
        completed.decrypt_and_reassemble(out.path(), &pub_secret, &session.session_init, &manifest).unwrap();

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
        let session = create_upload_session(&pub_key, manifest.total_chunks, original_hash).unwrap();
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

        let completed = handle.await_session(expected_session, Duration::from_secs(2)).await.unwrap();
        
        let out = NamedTempFile::new().unwrap();
        completed.decrypt_and_reassemble(out.path(), &pub_secret, &session.session_init, &manifest).unwrap();
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
        let session = create_upload_session(&pub_key, manifest.total_chunks, original_hash).unwrap();
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

        let completed = handle.await_session(expected_session, Duration::from_secs(2)).await.unwrap();
        let out = NamedTempFile::new().unwrap();
        completed.decrypt_and_reassemble(out.path(), &pub_secret, &session.session_init, &manifest).unwrap();
        assert!(completed.verify(original_hash, out.path()).unwrap());
    }
}
