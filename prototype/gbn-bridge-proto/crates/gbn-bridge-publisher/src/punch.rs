use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    BatchAssignment, BootstrapDhtEntry, BridgeBatchAssign, BridgeBatchAssignUnsigned,
    BridgePunchStart, BridgePunchStartUnsigned,
};

use crate::{AuthorityConfig, AuthorityResult};

pub fn issue_seed_punch_instruction(
    signing_key: &SigningKey,
    chain_id: &str,
    bootstrap_session_id: &str,
    seed_bridge_id: &str,
    creator_entry: BootstrapDhtEntry,
    config: &AuthorityConfig,
    now_ms: u64,
) -> AuthorityResult<BridgePunchStart> {
    BridgePunchStart::sign(
        BridgePunchStartUnsigned {
            chain_id: chain_id.to_string(),
            bootstrap_session_id: bootstrap_session_id.to_string(),
            initiator_id: seed_bridge_id.to_string(),
            target: creator_entry,
            attempt_expiry_ms: now_ms + config.punch_instruction_ttl_ms,
        },
        signing_key,
    )
    .map_err(Into::into)
}

pub fn sign_batch_assignments(
    signing_key: &SigningKey,
    chain_id: &str,
    batch_id: &str,
    bridge_ids: &[String],
    assignments: &[BatchAssignment],
    config: &AuthorityConfig,
    now_ms: u64,
) -> AuthorityResult<Vec<BridgeBatchAssign>> {
    bridge_ids
        .iter()
        .cloned()
        .map(|bridge_id| {
            BridgeBatchAssign::sign(
                BridgeBatchAssignUnsigned {
                    chain_id: chain_id.to_string(),
                    batch_id: batch_id.to_string(),
                    bridge_id,
                    window_started_at_ms: now_ms,
                    window_length_ms: config.batch_window_ms,
                    assignments: assignments.to_vec(),
                },
                signing_key,
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}
