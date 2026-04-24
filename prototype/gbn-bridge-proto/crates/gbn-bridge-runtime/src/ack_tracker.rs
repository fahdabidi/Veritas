use std::collections::{BTreeMap, BTreeSet};

use gbn_bridge_protocol::{BridgeAck, BridgeAckStatus};

use crate::fanout_scheduler::FrameDispatch;
use crate::{RuntimeError, RuntimeResult, UploadSession};

#[derive(Debug, Clone)]
pub struct AckTracker {
    session_id: String,
    chain_id: String,
    expected_frame_count: usize,
    outstanding: BTreeMap<u32, String>,
    acked: BTreeSet<u32>,
    completed: bool,
}

impl AckTracker {
    pub fn new(session: &UploadSession) -> Self {
        Self {
            session_id: session.session_id().to_string(),
            chain_id: session.chain_id().to_string(),
            expected_frame_count: session.frame_count(),
            outstanding: BTreeMap::new(),
            acked: BTreeSet::new(),
            completed: false,
        }
    }

    pub fn register_dispatch(&mut self, dispatch: &FrameDispatch) {
        self.outstanding
            .insert(dispatch.frame.sequence, dispatch.bridge_id.clone());
    }

    pub fn observe_ack(&mut self, ack: &BridgeAck) -> RuntimeResult<()> {
        if ack.session_id != self.session_id {
            return Err(RuntimeError::UnexpectedBridgeAck {
                session_id: ack.session_id.clone(),
                sequence: ack.acked_sequence,
            });
        }

        if !self.outstanding.contains_key(&ack.acked_sequence)
            && !self.acked.contains(&ack.acked_sequence)
        {
            return Err(RuntimeError::UnexpectedBridgeAck {
                session_id: ack.session_id.clone(),
                sequence: ack.acked_sequence,
            });
        }

        if matches!(ack.status, BridgeAckStatus::Rejected) {
            return Err(RuntimeError::RejectedBridgeAck {
                session_id: ack.session_id.clone(),
                sequence: ack.acked_sequence,
            });
        }

        self.outstanding.remove(&ack.acked_sequence);
        self.acked.insert(ack.acked_sequence);
        if matches!(ack.status, BridgeAckStatus::Complete) {
            self.completed = true;
        }

        Ok(())
    }

    pub fn all_acked(&self) -> bool {
        self.acked.len() == self.expected_frame_count
    }

    pub fn chain_id(&self) -> &str {
        &self.chain_id
    }

    pub fn completed(&self) -> bool {
        self.completed
    }
}
