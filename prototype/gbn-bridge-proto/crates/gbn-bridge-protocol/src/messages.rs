use serde::{Deserialize, Serialize};

use crate::bootstrap::{
    BridgeSetRequest, BridgeSetResponse, CreatorBootstrapResponse, CreatorJoinRequest,
};
use crate::catalog::{BridgeCatalogRequest, BridgeCatalogResponse};
use crate::error::ProtocolError;
use crate::lease::{BridgeHeartbeat, BridgeLease, BridgeRegister, BridgeRevoke};
use crate::punch::{
    BootstrapProgress, BridgeBatchAssign, BridgePunchAck, BridgePunchProbe, BridgePunchStart,
};
use crate::session::{BridgeAck, BridgeClose, BridgeData, BridgeOpen};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProtocolVersion(pub u16);

pub const CURRENT_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion(1);

impl Default for ProtocolVersion {
    fn default() -> Self {
        CURRENT_PROTOCOL_VERSION
    }
}

impl ProtocolVersion {
    pub fn ensure_supported(self) -> Result<(), ProtocolError> {
        if self == CURRENT_PROTOCOL_VERSION {
            return Ok(());
        }

        Err(ProtocolError::UnsupportedProtocolVersion {
            actual: self.0,
            expected: CURRENT_PROTOCOL_VERSION.0,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayProtection {
    pub message_id: String,
    pub nonce: u64,
    pub sent_at_ms: u64,
}

impl ReplayProtection {
    pub fn validate(&self, now_ms: u64, max_age_ms: u64) -> Result<(), ProtocolError> {
        if self.sent_at_ms > now_ms {
            return Err(ProtocolError::ReplayTimestampInFuture {
                sent_at_ms: self.sent_at_ms,
                now_ms,
            });
        }

        if now_ms.saturating_sub(self.sent_at_ms) > max_age_ms {
            return Err(ProtocolError::ReplayWindowExpired {
                sent_at_ms: self.sent_at_ms,
                now_ms,
                max_age_ms,
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolEnvelope<T> {
    pub version: ProtocolVersion,
    pub replay: Option<ReplayProtection>,
    pub body: T,
}

impl<T> ProtocolEnvelope<T> {
    pub fn new(body: T) -> Self {
        Self {
            version: CURRENT_PROTOCOL_VERSION,
            replay: None,
            body,
        }
    }

    pub fn with_replay(body: T, replay: ReplayProtection) -> Self {
        Self {
            version: CURRENT_PROTOCOL_VERSION,
            replay: Some(replay),
            body,
        }
    }

    pub fn validate(&self, now_ms: u64, max_age_ms: u64) -> Result<(), ProtocolError> {
        self.version.ensure_supported()?;
        if let Some(replay) = &self.replay {
            replay.validate(now_ms, max_age_ms)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "message_type", content = "body", rename_all = "snake_case")]
pub enum ProtocolMessage {
    BridgeRegister(BridgeRegister),
    BridgeLease(BridgeLease),
    BridgeHeartbeat(BridgeHeartbeat),
    BridgeRevoke(BridgeRevoke),
    BridgeCatalogRequest(BridgeCatalogRequest),
    BridgeCatalogResponse(BridgeCatalogResponse),
    CreatorJoinRequest(CreatorJoinRequest),
    CreatorBootstrapResponse(CreatorBootstrapResponse),
    BridgeSetRequest(BridgeSetRequest),
    BridgeSetResponse(BridgeSetResponse),
    BridgePunchStart(BridgePunchStart),
    BridgePunchProbe(BridgePunchProbe),
    BridgePunchAck(BridgePunchAck),
    BootstrapProgress(BootstrapProgress),
    BridgeBatchAssign(BridgeBatchAssign),
    BridgeOpen(BridgeOpen),
    BridgeData(BridgeData),
    BridgeAck(BridgeAck),
    BridgeClose(BridgeClose),
}
