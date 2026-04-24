use std::collections::BTreeMap;

use gbn_bridge_protocol::{BridgeClose, BridgeCloseReason, BridgeData, BridgeOpen, PublicKeyBytes};

use crate::creator::CreatorRuntime;
use crate::framing::{frame_payload, FramePayloadConfig};
use crate::network_transport::default_chain_id;
use crate::{RuntimeError, RuntimeResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UploadSessionConfig {
    pub frame_payload: FramePayloadConfig,
}

impl Default for UploadSessionConfig {
    fn default() -> Self {
        Self {
            frame_payload: FramePayloadConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadSession {
    session_id: String,
    chain_id: String,
    creator_id: String,
    creator_session_pub: PublicKeyBytes,
    opened_at_ms: u64,
    frames: Vec<BridgeData>,
}

impl UploadSession {
    pub fn new(
        session_id: String,
        creator: &CreatorRuntime,
        payload: &[u8],
        opened_at_ms: u64,
        config: UploadSessionConfig,
    ) -> RuntimeResult<Self> {
        let frames = frame_payload(&session_id, payload, opened_at_ms, config.frame_payload)?;
        let chain_id = default_chain_id("upload", &creator.config().creator_id, &session_id);

        Ok(Self {
            session_id,
            chain_id,
            creator_id: creator.config().creator_id.clone(),
            creator_session_pub: creator.config().pub_key.clone(),
            opened_at_ms,
            frames,
        })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn chain_id(&self) -> &str {
        &self.chain_id
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn frames(&self) -> &[BridgeData] {
        &self.frames
    }

    pub fn frame_by_sequence(&self, sequence: u32) -> RuntimeResult<&BridgeData> {
        self.frames
            .iter()
            .find(|frame| frame.sequence == sequence)
            .ok_or_else(|| RuntimeError::UnexpectedBridgeAck {
                session_id: self.session_id.clone(),
                sequence,
            })
    }

    pub fn open_for_bridge(&self, bridge_id: &str) -> BridgeOpen {
        BridgeOpen {
            session_id: self.session_id.clone(),
            creator_id: self.creator_id.clone(),
            bridge_id: bridge_id.to_string(),
            creator_session_pub: self.creator_session_pub.clone(),
            opened_at_ms: self.opened_at_ms,
            expected_chunks: Some(self.frames.len() as u16),
        }
    }

    pub fn close(&self, reason: BridgeCloseReason, closed_at_ms: u64) -> BridgeClose {
        BridgeClose {
            session_id: self.session_id.clone(),
            closed_at_ms,
            reason,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct BridgeSessionRegistry {
    sessions: BTreeMap<String, BridgeSessionRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BridgeSessionRecord {
    chain_id: String,
    open: BridgeOpen,
}

impl BridgeSessionRegistry {
    pub fn active_session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn open(&mut self, chain_id: impl Into<String>, open: BridgeOpen) {
        self.sessions.insert(
            open.session_id.clone(),
            BridgeSessionRecord {
                chain_id: chain_id.into(),
                open,
            },
        );
    }

    pub fn require_session(&self, session_id: &str) -> RuntimeResult<&BridgeOpen> {
        self.sessions
            .get(session_id)
            .map(|record| &record.open)
            .ok_or_else(|| RuntimeError::UploadSessionNotTracked {
                session_id: session_id.to_string(),
            })
    }

    pub fn require_chain_id(&self, session_id: &str) -> RuntimeResult<&str> {
        self.sessions
            .get(session_id)
            .map(|record| record.chain_id.as_str())
            .ok_or_else(|| RuntimeError::UploadSessionNotTracked {
                session_id: session_id.to_string(),
            })
    }

    pub fn close(&mut self, close: &BridgeClose) -> RuntimeResult<()> {
        self.sessions.remove(&close.session_id).ok_or_else(|| {
            RuntimeError::UploadSessionNotTracked {
                session_id: close.session_id.clone(),
            }
        })?;
        Ok(())
    }
}
