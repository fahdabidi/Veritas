use serde::{Deserialize, Serialize};

use crate::signing::PublicKeyBytes;
use crate::trace::validate_chain_id;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeOpen {
    pub chain_id: String,
    pub session_id: String,
    pub creator_id: String,
    pub bridge_id: String,
    pub creator_session_pub: PublicKeyBytes,
    pub opened_at_ms: u64,
    pub expected_chunks: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeData {
    pub chain_id: String,
    pub session_id: String,
    pub frame_id: String,
    pub sequence: u32,
    pub sent_at_ms: u64,
    pub ciphertext: Vec<u8>,
    pub final_frame: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeAckStatus {
    Accepted,
    Duplicate,
    Complete,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeAck {
    pub chain_id: String,
    pub session_id: String,
    pub acked_sequence: u32,
    pub status: BridgeAckStatus,
    pub acked_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeCloseReason {
    Completed,
    Timeout,
    LeaseExpired,
    PublisherRejected,
    BridgeUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeClose {
    pub chain_id: String,
    pub session_id: String,
    pub closed_at_ms: u64,
    pub reason: BridgeCloseReason,
}

impl BridgeOpen {
    pub fn validate_shape(&self) -> Result<(), crate::ProtocolError> {
        validate_chain_id(&self.chain_id)
    }
}

impl BridgeData {
    pub fn validate_shape(&self) -> Result<(), crate::ProtocolError> {
        validate_chain_id(&self.chain_id)
    }
}

impl BridgeAck {
    pub fn validate_shape(&self) -> Result<(), crate::ProtocolError> {
        validate_chain_id(&self.chain_id)
    }
}

impl BridgeClose {
    pub fn validate_shape(&self) -> Result<(), crate::ProtocolError> {
        validate_chain_id(&self.chain_id)
    }
}
