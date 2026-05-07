use std::time::Instant;

use gbn_bridge_protocol::ChainId;

#[derive(Debug, Clone)]
pub struct CreatorSession {
    pub session_id: String,
    pub bridge_id: String,
    pub bridge_address: String,
    pub bootstrap_chain_id: ChainId,
    pub started_at: Instant,
}
