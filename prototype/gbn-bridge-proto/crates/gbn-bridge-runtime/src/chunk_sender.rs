use gbn_bridge_protocol::{BridgeAck, BridgeCloseReason};

use crate::fanout_scheduler::FrameDispatch;
use crate::{
    AckTracker, CreatorRuntime, ExitBridgeRuntime, RuntimeResult, UploadSession,
    UploadSessionConfig,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkSenderConfig {
    pub upload_session: UploadSessionConfig,
}

impl Default for ChunkSenderConfig {
    fn default() -> Self {
        Self {
            upload_session: UploadSessionConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadResult {
    pub session_id: String,
    pub acks: Vec<BridgeAck>,
}

#[derive(Debug, Clone)]
pub struct ChunkSender {
    config: ChunkSenderConfig,
    next_session_seq: u64,
}

impl Default for ChunkSender {
    fn default() -> Self {
        Self {
            config: ChunkSenderConfig::default(),
            next_session_seq: 0,
        }
    }
}

impl ChunkSender {
    pub fn with_config(config: ChunkSenderConfig) -> Self {
        Self {
            config,
            next_session_seq: 0,
        }
    }

    pub fn begin_session(
        &mut self,
        creator: &CreatorRuntime,
        payload: &[u8],
        now_ms: u64,
    ) -> RuntimeResult<UploadSession> {
        self.next_session_seq += 1;
        UploadSession::new(
            format!("upload-{:06}", self.next_session_seq),
            creator,
            payload,
            now_ms,
            self.config.upload_session,
        )
    }

    pub fn open_selected_bridges(
        &self,
        session: &UploadSession,
        bridge_ids: &[String],
        bridges: &mut [ExitBridgeRuntime],
        now_ms: u64,
    ) -> RuntimeResult<()> {
        for bridge_id in bridge_ids {
            let bridge = find_bridge_mut(bridges, bridge_id)?;
            bridge.open_data_session_with_chain_id(
                session.chain_id(),
                session.open_for_bridge(bridge_id),
                now_ms,
            )?;
        }

        Ok(())
    }

    pub fn send_dispatches(
        &self,
        dispatches: &[FrameDispatch],
        bridges: &mut [ExitBridgeRuntime],
        ack_tracker: &mut AckTracker,
        now_ms: u64,
    ) -> RuntimeResult<Vec<BridgeAck>> {
        let mut acks = Vec::with_capacity(dispatches.len());
        for dispatch in dispatches {
            ack_tracker.register_dispatch(dispatch);
            let bridge = find_bridge_mut(bridges, &dispatch.bridge_id)?;
            let ack = bridge.forward_session_frame_with_chain_id(
                ack_tracker.chain_id(),
                dispatch.frame.clone(),
                now_ms + dispatch.frame.sequence as u64,
            )?;
            ack_tracker.observe_ack(&ack)?;
            acks.push(ack);
        }

        Ok(acks)
    }

    pub fn close_selected_bridges(
        &self,
        session: &UploadSession,
        bridge_ids: &[String],
        bridges: &mut [ExitBridgeRuntime],
        now_ms: u64,
    ) -> RuntimeResult<()> {
        let close = session.close(BridgeCloseReason::Completed, now_ms);
        for bridge_id in bridge_ids {
            let bridge = find_bridge_mut(bridges, bridge_id)?;
            let _ =
                bridge.close_data_session_with_chain_id(session.chain_id(), close.clone(), now_ms);
        }

        Ok(())
    }
}

fn find_bridge_mut<'a>(
    bridges: &'a mut [ExitBridgeRuntime],
    bridge_id: &str,
) -> RuntimeResult<&'a mut ExitBridgeRuntime> {
    bridges
        .iter_mut()
        .find(|bridge| bridge.config().bridge_id == bridge_id)
        .ok_or_else(|| crate::RuntimeError::UnexpectedBridgeRuntime {
            expected_bridge_id: bridge_id.to_string(),
            actual_bridge_id: "<missing>".to_string(),
        })
}
