use gbn_bridge_protocol::{BatchAssignment, BridgeBatchAssign};

use crate::punch;
use crate::storage::BootstrapSessionRecord;
use crate::{AuthorityConfig, AuthorityResult};

pub fn build_remaining_bridge_assignments(
    signing_key: &ed25519_dalek::SigningKey,
    config: &AuthorityConfig,
    session: &BootstrapSessionRecord,
    now_ms: u64,
) -> AuthorityResult<Vec<BridgeBatchAssign>> {
    let bridge_ids = session
        .bridge_ids
        .iter()
        .filter(|bridge_id| *bridge_id != &session.seed_bridge_id)
        .cloned()
        .collect::<Vec<_>>();
    if bridge_ids.is_empty() {
        return Ok(Vec::new());
    }

    punch::sign_batch_assignments(
        signing_key,
        &session.chain_id,
        &format!("bootstrap-fanout-{}", session.bootstrap_session_id),
        &bridge_ids,
        &[BatchAssignment {
            chain_id: session.chain_id.clone(),
            bootstrap_session_id: session.bootstrap_session_id.clone(),
            creator: session.creator_entry.clone(),
            requested_bridge_count: session.creator_response.assigned_bridge_count,
        }],
        config,
        now_ms,
    )
}
