use std::collections::BTreeMap;

use gbn_bridge_protocol::{BridgeSeedAssign, BridgeSetRequest, BridgeSetResponse, PublicKeyBytes};

use crate::{RuntimeError, RuntimeResult};

#[derive(Debug, Clone)]
pub struct SeedBridgeAssignment {
    pub bridge_set: BridgeSetResponse,
}

#[derive(Debug, Clone, Default)]
pub struct BootstrapBridgeState {
    assignments: BTreeMap<String, SeedBridgeAssignment>,
}

impl BootstrapBridgeState {
    pub fn assign_from_command(
        &mut self,
        bridge_id: &str,
        publisher_key: &PublicKeyBytes,
        assignment: &BridgeSeedAssign,
        now_ms: u64,
    ) -> RuntimeResult<bool> {
        assignment.verify_authority(publisher_key, now_ms)?;

        if assignment.seed_bridge_id != bridge_id || assignment.seed_punch.initiator_id != bridge_id
        {
            return Ok(false);
        }

        self.assignments.insert(
            assignment.bootstrap_session_id.clone(),
            SeedBridgeAssignment {
                bridge_set: assignment.bridge_set.clone(),
            },
        );
        Ok(true)
    }

    pub fn has_assignment(&self, bootstrap_session_id: &str) -> bool {
        self.assignments.contains_key(bootstrap_session_id)
    }

    pub fn serve_bridge_set(
        &self,
        request: &BridgeSetRequest,
        publisher_key: &PublicKeyBytes,
        now_ms: u64,
    ) -> RuntimeResult<BridgeSetResponse> {
        let assignment = self
            .assignments
            .get(&request.bootstrap_session_id)
            .ok_or_else(|| RuntimeError::BootstrapSessionNotTracked {
                bootstrap_session_id: request.bootstrap_session_id.clone(),
            })?;

        assignment
            .bridge_set
            .verify_authority(publisher_key, now_ms)?;
        Ok(assignment.bridge_set.clone())
    }
}
