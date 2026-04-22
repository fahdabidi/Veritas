use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::descriptor::{BridgeCapability, BridgeIngressEndpoint, ReachabilityClass};
use crate::error::ProtocolError;
use crate::signing::{
    ensure_not_expired, sign_payload, verify_payload, PublicKeyBytes, SignatureBytes,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeRegister {
    pub bridge_id: String,
    pub identity_pub: PublicKeyBytes,
    pub ingress_endpoints: Vec<BridgeIngressEndpoint>,
    pub requested_udp_punch_port: u16,
    pub capabilities: Vec<BridgeCapability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeLeaseUnsigned {
    pub lease_id: String,
    pub bridge_id: String,
    pub udp_punch_port: u16,
    pub reachability_class: ReachabilityClass,
    pub lease_expiry_ms: u64,
    pub issued_at_ms: u64,
    pub heartbeat_interval_ms: u64,
    pub capabilities: Vec<BridgeCapability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeLease {
    pub lease_id: String,
    pub bridge_id: String,
    pub udp_punch_port: u16,
    pub reachability_class: ReachabilityClass,
    pub lease_expiry_ms: u64,
    pub issued_at_ms: u64,
    pub heartbeat_interval_ms: u64,
    pub capabilities: Vec<BridgeCapability>,
    pub publisher_sig: SignatureBytes,
}

impl BridgeLease {
    pub fn sign(
        unsigned: BridgeLeaseUnsigned,
        signing_key: &SigningKey,
    ) -> Result<Self, ProtocolError> {
        let publisher_sig = sign_payload(&unsigned, signing_key)?;

        Ok(Self {
            lease_id: unsigned.lease_id,
            bridge_id: unsigned.bridge_id,
            udp_punch_port: unsigned.udp_punch_port,
            reachability_class: unsigned.reachability_class,
            lease_expiry_ms: unsigned.lease_expiry_ms,
            issued_at_ms: unsigned.issued_at_ms,
            heartbeat_interval_ms: unsigned.heartbeat_interval_ms,
            capabilities: unsigned.capabilities,
            publisher_sig,
        })
    }

    pub fn unsigned_payload(&self) -> BridgeLeaseUnsigned {
        BridgeLeaseUnsigned {
            lease_id: self.lease_id.clone(),
            bridge_id: self.bridge_id.clone(),
            udp_punch_port: self.udp_punch_port,
            reachability_class: self.reachability_class.clone(),
            lease_expiry_ms: self.lease_expiry_ms,
            issued_at_ms: self.issued_at_ms,
            heartbeat_interval_ms: self.heartbeat_interval_ms,
            capabilities: self.capabilities.clone(),
        }
    }

    pub fn verify_authority(
        &self,
        publisher_key: &PublicKeyBytes,
        now_ms: u64,
    ) -> Result<(), ProtocolError> {
        verify_payload(&self.unsigned_payload(), publisher_key, &self.publisher_sig)?;
        ensure_not_expired("bridge lease", self.lease_expiry_ms, now_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeHeartbeat {
    pub lease_id: String,
    pub bridge_id: String,
    pub heartbeat_at_ms: u64,
    pub active_sessions: u32,
    pub observed_ingress: Option<BridgeIngressEndpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevocationReason {
    LeaseExpired,
    PolicyViolation,
    OperatorDisabled,
    ReachabilityDowngraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeRevokeUnsigned {
    pub lease_id: String,
    pub bridge_id: String,
    pub revoked_at_ms: u64,
    pub reason: RevocationReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeRevoke {
    pub lease_id: String,
    pub bridge_id: String,
    pub revoked_at_ms: u64,
    pub reason: RevocationReason,
    pub publisher_sig: SignatureBytes,
}

impl BridgeRevoke {
    pub fn sign(
        unsigned: BridgeRevokeUnsigned,
        signing_key: &SigningKey,
    ) -> Result<Self, ProtocolError> {
        let publisher_sig = sign_payload(&unsigned, signing_key)?;

        Ok(Self {
            lease_id: unsigned.lease_id,
            bridge_id: unsigned.bridge_id,
            revoked_at_ms: unsigned.revoked_at_ms,
            reason: unsigned.reason,
            publisher_sig,
        })
    }

    pub fn unsigned_payload(&self) -> BridgeRevokeUnsigned {
        BridgeRevokeUnsigned {
            lease_id: self.lease_id.clone(),
            bridge_id: self.bridge_id.clone(),
            revoked_at_ms: self.revoked_at_ms,
            reason: self.reason.clone(),
        }
    }

    pub fn verify_signature(&self, publisher_key: &PublicKeyBytes) -> Result<(), ProtocolError> {
        verify_payload(&self.unsigned_payload(), publisher_key, &self.publisher_sig)
    }
}
