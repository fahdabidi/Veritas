use gbn_bridge_protocol::{BridgeAck, BridgeData};

use crate::{AuthorityResult, PublisherAuthority};

pub fn ingest_frame(
    authority: &mut PublisherAuthority,
    chain_id: &str,
    via_bridge_id: &str,
    frame: BridgeData,
    received_at_ms: u64,
) -> AuthorityResult<BridgeAck> {
    authority.ingest_bridge_frame_with_chain_id(
        Some(chain_id),
        via_bridge_id,
        frame,
        received_at_ms,
    )
}
