use gbn_bridge_protocol::{BridgeAck, BridgeAckStatus};

pub fn build_ack(
    chain_id: &str,
    session_id: &str,
    acked_sequence: u32,
    status: BridgeAckStatus,
    acked_at_ms: u64,
) -> BridgeAck {
    BridgeAck {
        chain_id: chain_id.to_string(),
        session_id: session_id.to_string(),
        acked_sequence,
        status,
        acked_at_ms,
    }
}
