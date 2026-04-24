use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::bootstrap::BridgeSeedAssign;
use crate::catalog::BridgeCatalogResponse;
use crate::error::ProtocolError;
use crate::lease::BridgeRevoke;
use crate::punch::{BootstrapProgress, BridgeBatchAssign, BridgePunchStart};
use crate::signing::{
    ensure_not_expired, sign_payload, verify_payload, PublicKeyBytes, SignatureBytes,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeControlHelloUnsigned {
    pub bridge_id: String,
    pub lease_id: String,
    pub bridge_pub: PublicKeyBytes,
    pub sent_at_ms: u64,
    pub request_id: String,
    pub resume_acked_seq_no: Option<u64>,
    pub chain_id: String,
}

impl BridgeControlHelloUnsigned {
    pub fn validate_shape(&self) -> Result<(), ProtocolError> {
        if self.bridge_id.trim().is_empty()
            || self.lease_id.trim().is_empty()
            || self.request_id.trim().is_empty()
            || self.chain_id.trim().is_empty()
        {
            return Err(ProtocolError::Serialization(
                "bridge control hello requires non-empty identifiers".into(),
            ));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeControlHello {
    pub bridge_id: String,
    pub lease_id: String,
    pub bridge_pub: PublicKeyBytes,
    pub sent_at_ms: u64,
    pub request_id: String,
    pub resume_acked_seq_no: Option<u64>,
    pub chain_id: String,
    pub bridge_sig: SignatureBytes,
}

impl BridgeControlHello {
    pub fn sign(
        unsigned: BridgeControlHelloUnsigned,
        signing_key: &SigningKey,
    ) -> Result<Self, ProtocolError> {
        unsigned.validate_shape()?;
        let bridge_sig = sign_payload(&unsigned, signing_key)?;
        Ok(Self {
            bridge_id: unsigned.bridge_id,
            lease_id: unsigned.lease_id,
            bridge_pub: unsigned.bridge_pub,
            sent_at_ms: unsigned.sent_at_ms,
            request_id: unsigned.request_id,
            resume_acked_seq_no: unsigned.resume_acked_seq_no,
            chain_id: unsigned.chain_id,
            bridge_sig,
        })
    }

    pub fn unsigned_payload(&self) -> BridgeControlHelloUnsigned {
        BridgeControlHelloUnsigned {
            bridge_id: self.bridge_id.clone(),
            lease_id: self.lease_id.clone(),
            bridge_pub: self.bridge_pub.clone(),
            sent_at_ms: self.sent_at_ms,
            request_id: self.request_id.clone(),
            resume_acked_seq_no: self.resume_acked_seq_no,
            chain_id: self.chain_id.clone(),
        }
    }

    pub fn verify_bridge(&self, now_ms: u64, max_age_ms: u64) -> Result<(), ProtocolError> {
        let unsigned = self.unsigned_payload();
        unsigned.validate_shape()?;
        if unsigned.sent_at_ms > now_ms {
            return Err(ProtocolError::ReplayTimestampInFuture {
                sent_at_ms: unsigned.sent_at_ms,
                now_ms,
            });
        }
        if now_ms.saturating_sub(unsigned.sent_at_ms) > max_age_ms {
            return Err(ProtocolError::ReplayWindowExpired {
                sent_at_ms: unsigned.sent_at_ms,
                now_ms,
                max_age_ms,
            });
        }
        verify_payload(&unsigned, &self.bridge_pub, &self.bridge_sig)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeControlWelcomeUnsigned {
    pub bridge_id: String,
    pub session_id: String,
    pub accepted_at_ms: u64,
    pub heartbeat_interval_ms: u64,
    pub idle_timeout_ms: u64,
    pub last_publisher_seq_no: Option<u64>,
    pub chain_id: String,
}

impl BridgeControlWelcomeUnsigned {
    pub fn validate_shape(&self) -> Result<(), ProtocolError> {
        if self.bridge_id.trim().is_empty()
            || self.session_id.trim().is_empty()
            || self.chain_id.trim().is_empty()
        {
            return Err(ProtocolError::Serialization(
                "bridge control welcome requires non-empty identifiers".into(),
            ));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeControlWelcome {
    pub bridge_id: String,
    pub session_id: String,
    pub accepted_at_ms: u64,
    pub heartbeat_interval_ms: u64,
    pub idle_timeout_ms: u64,
    pub last_publisher_seq_no: Option<u64>,
    pub chain_id: String,
    pub publisher_sig: SignatureBytes,
}

impl BridgeControlWelcome {
    pub fn sign(
        unsigned: BridgeControlWelcomeUnsigned,
        signing_key: &SigningKey,
    ) -> Result<Self, ProtocolError> {
        unsigned.validate_shape()?;
        let publisher_sig = sign_payload(&unsigned, signing_key)?;
        Ok(Self {
            bridge_id: unsigned.bridge_id,
            session_id: unsigned.session_id,
            accepted_at_ms: unsigned.accepted_at_ms,
            heartbeat_interval_ms: unsigned.heartbeat_interval_ms,
            idle_timeout_ms: unsigned.idle_timeout_ms,
            last_publisher_seq_no: unsigned.last_publisher_seq_no,
            chain_id: unsigned.chain_id,
            publisher_sig,
        })
    }

    pub fn unsigned_payload(&self) -> BridgeControlWelcomeUnsigned {
        BridgeControlWelcomeUnsigned {
            bridge_id: self.bridge_id.clone(),
            session_id: self.session_id.clone(),
            accepted_at_ms: self.accepted_at_ms,
            heartbeat_interval_ms: self.heartbeat_interval_ms,
            idle_timeout_ms: self.idle_timeout_ms,
            last_publisher_seq_no: self.last_publisher_seq_no,
            chain_id: self.chain_id.clone(),
        }
    }

    pub fn verify_authority(
        &self,
        publisher_key: &PublicKeyBytes,
        now_ms: u64,
        max_age_ms: u64,
    ) -> Result<(), ProtocolError> {
        let unsigned = self.unsigned_payload();
        unsigned.validate_shape()?;
        verify_payload(&unsigned, publisher_key, &self.publisher_sig)?;
        ensure_not_expired(
            "bridge control welcome",
            self.accepted_at_ms + max_age_ms,
            now_ms,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command_type", content = "body", rename_all = "snake_case")]
pub enum BridgeCommandPayload {
    SeedAssign(BridgeSeedAssign),
    PunchStart(BridgePunchStart),
    BatchAssign(BridgeBatchAssign),
    Revoke(BridgeRevoke),
    CatalogRefresh(BridgeCatalogResponse),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeControlCommand {
    pub session_id: String,
    pub bridge_id: String,
    pub command_id: String,
    pub seq_no: u64,
    pub issued_at_ms: u64,
    pub chain_id: String,
    pub payload: BridgeCommandPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeCommandAckStatus {
    Applied,
    Duplicate,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeCommandAck {
    pub session_id: String,
    pub bridge_id: String,
    pub command_id: String,
    pub seq_no: u64,
    pub acked_at_ms: u64,
    pub chain_id: String,
    pub status: BridgeCommandAckStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeControlKeepalive {
    pub session_id: String,
    pub bridge_id: String,
    pub sent_at_ms: u64,
    pub chain_id: String,
    pub last_acked_seq_no: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeControlProgress {
    pub session_id: String,
    pub chain_id: String,
    pub progress: BootstrapProgress,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeControlError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "frame_type", content = "body", rename_all = "snake_case")]
pub enum BridgeControlFrame {
    Hello(BridgeControlHello),
    Welcome(BridgeControlWelcome),
    Command(BridgeControlCommand),
    Ack(BridgeCommandAck),
    Progress(BridgeControlProgress),
    Keepalive(BridgeControlKeepalive),
    Error(BridgeControlError),
}
