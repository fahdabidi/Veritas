use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    BootstrapJoinReply, BridgeSeedAssign, BridgeSeedAssignUnsigned, BridgeSetResponse,
    CreatorBootstrapResponse,
};

use crate::AuthorityResult;

pub fn join_reply(
    creator_entry: gbn_bridge_protocol::BootstrapDhtEntry,
    response: CreatorBootstrapResponse,
) -> BootstrapJoinReply {
    BootstrapJoinReply {
        creator_entry,
        response,
    }
}

pub fn issue_seed_assignment(
    signing_key: &SigningKey,
    seed_bridge_id: &str,
    creator_entry: gbn_bridge_protocol::BootstrapDhtEntry,
    bridge_set: BridgeSetResponse,
    seed_punch: gbn_bridge_protocol::BridgePunchStart,
    assignment_expiry_ms: u64,
) -> AuthorityResult<BridgeSeedAssign> {
    BridgeSeedAssign::sign(
        BridgeSeedAssignUnsigned {
            bootstrap_session_id: bridge_set.bootstrap_session_id.clone(),
            seed_bridge_id: seed_bridge_id.to_string(),
            creator_entry,
            bridge_set,
            seed_punch,
            assignment_expiry_ms,
        },
        signing_key,
    )
    .map_err(Into::into)
}
