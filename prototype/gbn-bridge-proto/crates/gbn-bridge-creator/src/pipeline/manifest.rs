use gbn_bridge_protocol::PublicKeyBytes;
use serde::{Deserialize, Serialize};

use super::sanitizer::SanitizationReport;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadManifest {
    pub session_id: Vec<u8>,
    pub creator_ephemeral_pubkey: PublicKeyBytes,
    pub publisher_key_id: String,
    pub total_chunks: u32,
    pub content_hash: Vec<u8>,
    pub sanitization_profile: String,
    pub sanitization_report: SanitizationReport,
    pub created_at_ms: u64,
    pub chunk_size: u32,
    pub total_bytes: u64,
}
