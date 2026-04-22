use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::descriptor::BridgeDescriptor;
use crate::error::ProtocolError;
use crate::signing::{
    ensure_not_expired, sign_payload, verify_payload, PublicKeyBytes, SignatureBytes,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefreshHintReason {
    Startup,
    LeaseExpiring,
    BridgeFailure,
    BootstrapCompleted,
    ManualRefresh,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeRefreshHint {
    pub bridge_id: Option<String>,
    pub reason: RefreshHintReason,
    pub last_success_ms: Option<u64>,
    pub stale_after_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeCatalogRequest {
    pub creator_id: String,
    pub known_catalog_id: Option<String>,
    pub direct_only: bool,
    pub refresh_hint: Option<BridgeRefreshHint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeCatalogResponseUnsigned {
    pub catalog_id: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub bridges: Vec<BridgeDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeCatalogResponse {
    pub catalog_id: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub bridges: Vec<BridgeDescriptor>,
    pub publisher_sig: SignatureBytes,
}

impl BridgeCatalogResponse {
    pub fn sign(
        unsigned: BridgeCatalogResponseUnsigned,
        signing_key: &SigningKey,
    ) -> Result<Self, ProtocolError> {
        let publisher_sig = sign_payload(&unsigned, signing_key)?;

        Ok(Self {
            catalog_id: unsigned.catalog_id,
            issued_at_ms: unsigned.issued_at_ms,
            expires_at_ms: unsigned.expires_at_ms,
            bridges: unsigned.bridges,
            publisher_sig,
        })
    }

    pub fn unsigned_payload(&self) -> BridgeCatalogResponseUnsigned {
        BridgeCatalogResponseUnsigned {
            catalog_id: self.catalog_id.clone(),
            issued_at_ms: self.issued_at_ms,
            expires_at_ms: self.expires_at_ms,
            bridges: self.bridges.clone(),
        }
    }

    pub fn verify_authority(
        &self,
        publisher_key: &PublicKeyBytes,
        now_ms: u64,
    ) -> Result<(), ProtocolError> {
        for bridge in &self.bridges {
            bridge.verify_authority(publisher_key, now_ms)?;
        }
        verify_payload(&self.unsigned_payload(), publisher_key, &self.publisher_sig)?;
        ensure_not_expired("bridge catalog", self.expires_at_ms, now_ms)
    }
}
