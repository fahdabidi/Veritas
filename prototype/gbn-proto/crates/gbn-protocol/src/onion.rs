use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// A single onion layer plaintext after successful decrypt/open.
///
/// If `next_hop` is `Some(addr)`, the current node is an intermediate relay and
/// must forward `inner` to `addr`.
///
/// If `next_hop` is `None`, this node is the terminal destination and `inner`
/// should be decoded as the destination payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnionLayer {
    pub next_hop: Option<SocketAddr>,
    pub inner: Vec<u8>,
}

/// Public identity for one hop in a route.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HopInfo {
    pub addr: SocketAddr,
    pub identity_pub: [u8; 32],
}

/// Terminal payload delivered at the final hop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkPayload {
    pub chunk_id: u64,
    pub hash: [u8; 32],
    pub chunk: Vec<u8>,
    pub return_path: Vec<HopInfo>,
    #[serde(default)]
    pub send_timestamp_ms: u64,
    #[serde(default)]
    pub total_chunks: u32,
    #[serde(default)]
    pub chunk_index: u32,
}

/// Terminal ACK payload decrypted by the creator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AckPayload {
    pub chunk_id: u64,
    pub hash: [u8; 32],
    #[serde(default)]
    pub send_timestamp_ms: u64,
    #[serde(default)]
    pub received_timestamp_ms: u64,
    #[serde(default)]
    pub total_chunks: u32,
    #[serde(default)]
    pub chunk_index: u32,
}

