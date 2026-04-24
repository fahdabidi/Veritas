use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::bootstrap::BootstrapDhtEntry;
use crate::error::ProtocolError;
use crate::signing::{
    ensure_not_expired, sign_payload, verify_payload, PublicKeyBytes, SignatureBytes,
};
use crate::trace::validate_chain_id;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgePunchStartUnsigned {
    pub chain_id: String,
    pub bootstrap_session_id: String,
    pub initiator_id: String,
    pub target: BootstrapDhtEntry,
    pub attempt_expiry_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgePunchStart {
    pub chain_id: String,
    pub bootstrap_session_id: String,
    pub initiator_id: String,
    pub target: BootstrapDhtEntry,
    pub attempt_expiry_ms: u64,
    pub publisher_sig: SignatureBytes,
}

impl BridgePunchStart {
    pub fn sign(
        unsigned: BridgePunchStartUnsigned,
        signing_key: &SigningKey,
    ) -> Result<Self, ProtocolError> {
        validate_chain_id(&unsigned.chain_id)?;
        let publisher_sig = sign_payload(&unsigned, signing_key)?;

        Ok(Self {
            chain_id: unsigned.chain_id,
            bootstrap_session_id: unsigned.bootstrap_session_id,
            initiator_id: unsigned.initiator_id,
            target: unsigned.target,
            attempt_expiry_ms: unsigned.attempt_expiry_ms,
            publisher_sig,
        })
    }

    pub fn unsigned_payload(&self) -> BridgePunchStartUnsigned {
        BridgePunchStartUnsigned {
            chain_id: self.chain_id.clone(),
            bootstrap_session_id: self.bootstrap_session_id.clone(),
            initiator_id: self.initiator_id.clone(),
            target: self.target.clone(),
            attempt_expiry_ms: self.attempt_expiry_ms,
        }
    }

    pub fn verify_authority(
        &self,
        publisher_key: &PublicKeyBytes,
        now_ms: u64,
    ) -> Result<(), ProtocolError> {
        validate_chain_id(&self.chain_id)?;
        self.target.verify_authority(publisher_key, now_ms)?;
        verify_payload(&self.unsigned_payload(), publisher_key, &self.publisher_sig)?;
        ensure_not_expired("bridge punch start", self.attempt_expiry_ms, now_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgePunchProbe {
    pub chain_id: String,
    pub bootstrap_session_id: String,
    pub source_node_id: String,
    pub source_pub_key: PublicKeyBytes,
    pub source_ip_addr: String,
    pub source_udp_punch_port: u16,
    pub probe_nonce: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgePunchAck {
    pub chain_id: String,
    pub bootstrap_session_id: String,
    pub source_node_id: String,
    pub responder_node_id: String,
    pub observed_udp_punch_port: u16,
    pub acked_probe_nonce: u64,
    pub established_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapProgressStage {
    SeedAssigned,
    SeedTunnelEstablished,
    SeedPayloadReceived,
    FanoutStarted,
    BridgeTunnelEstablished,
    BridgeSetComplete,
    FallbackReuseActivated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapProgress {
    pub chain_id: String,
    pub bootstrap_session_id: String,
    pub reporter_id: String,
    pub stage: BootstrapProgressStage,
    pub active_bridge_count: u16,
    pub reported_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchAssignment {
    pub chain_id: String,
    pub bootstrap_session_id: String,
    pub creator: BootstrapDhtEntry,
    pub requested_bridge_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeBatchAssignUnsigned {
    pub chain_id: String,
    pub batch_id: String,
    pub bridge_id: String,
    pub window_started_at_ms: u64,
    pub window_length_ms: u64,
    pub assignments: Vec<BatchAssignment>,
}

impl BridgeBatchAssignUnsigned {
    pub fn validate_shape(&self) -> Result<(), ProtocolError> {
        validate_chain_id(&self.chain_id)?;
        if self.assignments.is_empty() {
            return Err(ProtocolError::EmptyBatchAssignments);
        }
        for assignment in &self.assignments {
            validate_chain_id(&assignment.chain_id)?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeBatchAssign {
    pub chain_id: String,
    pub batch_id: String,
    pub bridge_id: String,
    pub window_started_at_ms: u64,
    pub window_length_ms: u64,
    pub assignments: Vec<BatchAssignment>,
    pub publisher_sig: SignatureBytes,
}

impl BridgeBatchAssign {
    pub fn sign(
        unsigned: BridgeBatchAssignUnsigned,
        signing_key: &SigningKey,
    ) -> Result<Self, ProtocolError> {
        unsigned.validate_shape()?;
        let publisher_sig = sign_payload(&unsigned, signing_key)?;

        Ok(Self {
            chain_id: unsigned.chain_id,
            batch_id: unsigned.batch_id,
            bridge_id: unsigned.bridge_id,
            window_started_at_ms: unsigned.window_started_at_ms,
            window_length_ms: unsigned.window_length_ms,
            assignments: unsigned.assignments,
            publisher_sig,
        })
    }

    pub fn unsigned_payload(&self) -> BridgeBatchAssignUnsigned {
        BridgeBatchAssignUnsigned {
            chain_id: self.chain_id.clone(),
            batch_id: self.batch_id.clone(),
            bridge_id: self.bridge_id.clone(),
            window_started_at_ms: self.window_started_at_ms,
            window_length_ms: self.window_length_ms,
            assignments: self.assignments.clone(),
        }
    }

    pub fn verify_authority(
        &self,
        publisher_key: &PublicKeyBytes,
        now_ms: u64,
    ) -> Result<(), ProtocolError> {
        let unsigned = self.unsigned_payload();
        unsigned.validate_shape()?;
        for assignment in &self.assignments {
            assignment.creator.verify_authority(publisher_key, now_ms)?;
        }
        verify_payload(&unsigned, publisher_key, &self.publisher_sig)
    }
}
