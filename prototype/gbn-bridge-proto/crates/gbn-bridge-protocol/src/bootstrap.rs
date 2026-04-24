use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::error::ProtocolError;
use crate::punch::BridgePunchStart;
use crate::signing::{
    ensure_not_expired, sign_payload, verify_payload, PublicKeyBytes, SignatureBytes,
};
use crate::trace::validate_chain_id;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingCreator {
    pub node_id: String,
    pub ip_addr: String,
    pub pub_key: PublicKeyBytes,
    pub udp_punch_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapDhtEntryUnsigned {
    pub node_id: String,
    pub ip_addr: String,
    pub pub_key: PublicKeyBytes,
    pub udp_punch_port: u16,
    pub entry_expiry_ms: u64,
}

impl BootstrapDhtEntryUnsigned {
    pub fn validate_shape(&self) -> Result<(), ProtocolError> {
        if self.udp_punch_port == 0 {
            return Err(ProtocolError::InvalidUdpPunchPort);
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapDhtEntry {
    pub node_id: String,
    pub ip_addr: String,
    pub pub_key: PublicKeyBytes,
    pub udp_punch_port: u16,
    pub entry_expiry_ms: u64,
    pub publisher_sig: SignatureBytes,
}

impl BootstrapDhtEntry {
    pub fn sign(
        unsigned: BootstrapDhtEntryUnsigned,
        signing_key: &SigningKey,
    ) -> Result<Self, ProtocolError> {
        unsigned.validate_shape()?;
        let publisher_sig = sign_payload(&unsigned, signing_key)?;

        Ok(Self {
            node_id: unsigned.node_id,
            ip_addr: unsigned.ip_addr,
            pub_key: unsigned.pub_key,
            udp_punch_port: unsigned.udp_punch_port,
            entry_expiry_ms: unsigned.entry_expiry_ms,
            publisher_sig,
        })
    }

    pub fn unsigned_payload(&self) -> BootstrapDhtEntryUnsigned {
        BootstrapDhtEntryUnsigned {
            node_id: self.node_id.clone(),
            ip_addr: self.ip_addr.clone(),
            pub_key: self.pub_key.clone(),
            udp_punch_port: self.udp_punch_port,
            entry_expiry_ms: self.entry_expiry_ms,
        }
    }

    pub fn verify_authority(
        &self,
        publisher_key: &PublicKeyBytes,
        now_ms: u64,
    ) -> Result<(), ProtocolError> {
        let unsigned = self.unsigned_payload();
        unsigned.validate_shape()?;
        verify_payload(&unsigned, publisher_key, &self.publisher_sig)?;
        ensure_not_expired("bootstrap entry", self.entry_expiry_ms, now_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatorJoinRequest {
    pub chain_id: String,
    pub request_id: String,
    pub host_creator_id: String,
    pub relay_bridge_id: String,
    pub creator: PendingCreator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatorBootstrapResponseUnsigned {
    pub chain_id: String,
    pub bootstrap_session_id: String,
    pub seed_bridge: BootstrapDhtEntry,
    pub publisher_pub: PublicKeyBytes,
    pub response_expiry_ms: u64,
    pub assigned_bridge_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatorBootstrapResponse {
    pub chain_id: String,
    pub bootstrap_session_id: String,
    pub seed_bridge: BootstrapDhtEntry,
    pub publisher_pub: PublicKeyBytes,
    pub response_expiry_ms: u64,
    pub assigned_bridge_count: u16,
    pub publisher_sig: SignatureBytes,
}

impl CreatorBootstrapResponse {
    pub fn sign(
        unsigned: CreatorBootstrapResponseUnsigned,
        signing_key: &SigningKey,
    ) -> Result<Self, ProtocolError> {
        validate_chain_id(&unsigned.chain_id)?;
        let publisher_sig = sign_payload(&unsigned, signing_key)?;

        Ok(Self {
            chain_id: unsigned.chain_id,
            bootstrap_session_id: unsigned.bootstrap_session_id,
            seed_bridge: unsigned.seed_bridge,
            publisher_pub: unsigned.publisher_pub,
            response_expiry_ms: unsigned.response_expiry_ms,
            assigned_bridge_count: unsigned.assigned_bridge_count,
            publisher_sig,
        })
    }

    pub fn unsigned_payload(&self) -> CreatorBootstrapResponseUnsigned {
        CreatorBootstrapResponseUnsigned {
            chain_id: self.chain_id.clone(),
            bootstrap_session_id: self.bootstrap_session_id.clone(),
            seed_bridge: self.seed_bridge.clone(),
            publisher_pub: self.publisher_pub.clone(),
            response_expiry_ms: self.response_expiry_ms,
            assigned_bridge_count: self.assigned_bridge_count,
        }
    }

    pub fn verify_authority(
        &self,
        publisher_key: &PublicKeyBytes,
        now_ms: u64,
    ) -> Result<(), ProtocolError> {
        validate_chain_id(&self.chain_id)?;
        verify_payload(&self.unsigned_payload(), publisher_key, &self.publisher_sig)?;
        self.seed_bridge.verify_authority(publisher_key, now_ms)?;
        ensure_not_expired(
            "creator bootstrap response",
            self.response_expiry_ms,
            now_ms,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapJoinReply {
    pub chain_id: String,
    pub creator_entry: BootstrapDhtEntry,
    pub response: CreatorBootstrapResponse,
}

impl BootstrapJoinReply {
    pub fn verify_authority(
        &self,
        publisher_key: &PublicKeyBytes,
        now_ms: u64,
    ) -> Result<(), ProtocolError> {
        validate_chain_id(&self.chain_id)?;
        if self.response.chain_id != self.chain_id {
            return Err(ProtocolError::Serialization(
                "bootstrap join reply chain_id mismatch".into(),
            ));
        }
        self.creator_entry.verify_authority(publisher_key, now_ms)?;
        self.response.verify_authority(publisher_key, now_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeSetRequest {
    pub chain_id: String,
    pub bootstrap_session_id: String,
    pub creator_id: String,
    pub requested_bridge_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeSetResponseUnsigned {
    pub chain_id: String,
    pub bootstrap_session_id: String,
    pub bridge_entries: Vec<BootstrapDhtEntry>,
    pub response_expiry_ms: u64,
}

impl BridgeSetResponseUnsigned {
    pub fn validate_shape(&self) -> Result<(), ProtocolError> {
        validate_chain_id(&self.chain_id)?;
        if self.bridge_entries.is_empty() {
            return Err(ProtocolError::EmptyBridgeSet);
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeSetResponse {
    pub chain_id: String,
    pub bootstrap_session_id: String,
    pub bridge_entries: Vec<BootstrapDhtEntry>,
    pub response_expiry_ms: u64,
    pub publisher_sig: SignatureBytes,
}

impl BridgeSetResponse {
    pub fn sign(
        unsigned: BridgeSetResponseUnsigned,
        signing_key: &SigningKey,
    ) -> Result<Self, ProtocolError> {
        unsigned.validate_shape()?;
        let publisher_sig = sign_payload(&unsigned, signing_key)?;

        Ok(Self {
            chain_id: unsigned.chain_id,
            bootstrap_session_id: unsigned.bootstrap_session_id,
            bridge_entries: unsigned.bridge_entries,
            response_expiry_ms: unsigned.response_expiry_ms,
            publisher_sig,
        })
    }

    pub fn unsigned_payload(&self) -> BridgeSetResponseUnsigned {
        BridgeSetResponseUnsigned {
            chain_id: self.chain_id.clone(),
            bootstrap_session_id: self.bootstrap_session_id.clone(),
            bridge_entries: self.bridge_entries.clone(),
            response_expiry_ms: self.response_expiry_ms,
        }
    }

    pub fn verify_authority(
        &self,
        publisher_key: &PublicKeyBytes,
        now_ms: u64,
    ) -> Result<(), ProtocolError> {
        let unsigned = self.unsigned_payload();
        unsigned.validate_shape()?;
        for entry in &self.bridge_entries {
            entry.verify_authority(publisher_key, now_ms)?;
        }
        verify_payload(&unsigned, publisher_key, &self.publisher_sig)?;
        ensure_not_expired("bridge set response", self.response_expiry_ms, now_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeSeedAssignUnsigned {
    pub chain_id: String,
    pub bootstrap_session_id: String,
    pub seed_bridge_id: String,
    pub creator_entry: BootstrapDhtEntry,
    pub bridge_set: BridgeSetResponse,
    pub seed_punch: BridgePunchStart,
    pub assignment_expiry_ms: u64,
}

impl BridgeSeedAssignUnsigned {
    pub fn validate_shape(&self) -> Result<(), ProtocolError> {
        validate_chain_id(&self.chain_id)?;
        if self.bootstrap_session_id.trim().is_empty() || self.seed_bridge_id.trim().is_empty() {
            return Err(ProtocolError::Serialization(
                "bridge seed assignment requires non-empty identifiers".into(),
            ));
        }
        if self.bridge_set.chain_id != self.chain_id {
            return Err(ProtocolError::Serialization(
                "bridge seed assignment bridge set chain mismatch".into(),
            ));
        }
        if self.bridge_set.bootstrap_session_id != self.bootstrap_session_id {
            return Err(ProtocolError::Serialization(
                "bridge seed assignment bridge set session mismatch".into(),
            ));
        }
        if self.seed_punch.chain_id != self.chain_id {
            return Err(ProtocolError::Serialization(
                "bridge seed assignment punch chain mismatch".into(),
            ));
        }
        if self.seed_punch.bootstrap_session_id != self.bootstrap_session_id {
            return Err(ProtocolError::Serialization(
                "bridge seed assignment punch session mismatch".into(),
            ));
        }
        if self.seed_punch.initiator_id != self.seed_bridge_id {
            return Err(ProtocolError::Serialization(
                "bridge seed assignment punch initiator mismatch".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeSeedAssign {
    pub chain_id: String,
    pub bootstrap_session_id: String,
    pub seed_bridge_id: String,
    pub creator_entry: BootstrapDhtEntry,
    pub bridge_set: BridgeSetResponse,
    pub seed_punch: BridgePunchStart,
    pub assignment_expiry_ms: u64,
    pub publisher_sig: SignatureBytes,
}

impl BridgeSeedAssign {
    pub fn sign(
        unsigned: BridgeSeedAssignUnsigned,
        signing_key: &SigningKey,
    ) -> Result<Self, ProtocolError> {
        unsigned.validate_shape()?;
        let publisher_sig = sign_payload(&unsigned, signing_key)?;
        Ok(Self {
            chain_id: unsigned.chain_id,
            bootstrap_session_id: unsigned.bootstrap_session_id,
            seed_bridge_id: unsigned.seed_bridge_id,
            creator_entry: unsigned.creator_entry,
            bridge_set: unsigned.bridge_set,
            seed_punch: unsigned.seed_punch,
            assignment_expiry_ms: unsigned.assignment_expiry_ms,
            publisher_sig,
        })
    }

    pub fn unsigned_payload(&self) -> BridgeSeedAssignUnsigned {
        BridgeSeedAssignUnsigned {
            chain_id: self.chain_id.clone(),
            bootstrap_session_id: self.bootstrap_session_id.clone(),
            seed_bridge_id: self.seed_bridge_id.clone(),
            creator_entry: self.creator_entry.clone(),
            bridge_set: self.bridge_set.clone(),
            seed_punch: self.seed_punch.clone(),
            assignment_expiry_ms: self.assignment_expiry_ms,
        }
    }

    pub fn verify_authority(
        &self,
        publisher_key: &PublicKeyBytes,
        now_ms: u64,
    ) -> Result<(), ProtocolError> {
        let unsigned = self.unsigned_payload();
        unsigned.validate_shape()?;
        self.creator_entry.verify_authority(publisher_key, now_ms)?;
        self.bridge_set.verify_authority(publisher_key, now_ms)?;
        self.seed_punch.verify_authority(publisher_key, now_ms)?;
        verify_payload(&unsigned, publisher_key, &self.publisher_sig)?;
        ensure_not_expired("bridge seed assignment", self.assignment_expiry_ms, now_ms)
    }
}
