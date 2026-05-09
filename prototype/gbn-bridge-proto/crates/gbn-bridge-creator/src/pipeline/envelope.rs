use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn session_id_bytes(
    actor_id: &str,
    chain_id: &str,
    content_hash: &[u8],
    chunk_size: usize,
    now_ms: u64,
) -> [u8; 16] {
    let counter = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut hasher = Sha256::new();
    hasher.update(b"veritas/conduit/v2/upload-session-id");
    hasher.update(actor_id.as_bytes());
    hasher.update(chain_id.as_bytes());
    hasher.update(content_hash);
    hasher.update(chunk_size.to_le_bytes());
    hasher.update(now_ms.to_le_bytes());
    hasher.update(counter.to_le_bytes());
    let digest = hasher.finalize();
    let mut out = [0_u8; 16];
    out.copy_from_slice(&digest[..16]);
    out
}

pub fn upload_ephemeral_private(
    actor_id: &str,
    session_id: &[u8; 16],
    content_hash: &[u8],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"veritas/conduit/v2/upload-session-ephemeral");
    hasher.update(actor_id.as_bytes());
    hasher.update(session_id);
    hasher.update(content_hash);
    hasher.finalize().into()
}

pub fn session_id_hex(session_id: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(session_id.len() * 2);
    for byte in session_id {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
