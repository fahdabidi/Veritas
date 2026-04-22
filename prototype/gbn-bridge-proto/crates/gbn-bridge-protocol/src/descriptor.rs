use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::error::ProtocolError;
use crate::signing::{
    ensure_not_expired, sign_payload, verify_payload, PublicKeyBytes, SignatureBytes,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeIngressEndpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachabilityClass {
    Direct,
    Brokered,
    RelayOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeCapability {
    BootstrapSeed,
    CatalogRefresh,
    SessionRelay,
    BatchAssignment,
    ProgressReporting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeDescriptorUnsigned {
    pub bridge_id: String,
    pub identity_pub: PublicKeyBytes,
    pub ingress_endpoints: Vec<BridgeIngressEndpoint>,
    pub udp_punch_port: u16,
    pub reachability_class: ReachabilityClass,
    pub lease_expiry_ms: u64,
    pub capabilities: Vec<BridgeCapability>,
}

impl BridgeDescriptorUnsigned {
    pub fn validate_shape(&self) -> Result<(), ProtocolError> {
        if self.ingress_endpoints.is_empty() {
            return Err(ProtocolError::EmptyIngressEndpoints);
        }

        if self.udp_punch_port == 0 {
            return Err(ProtocolError::InvalidUdpPunchPort);
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeDescriptor {
    pub bridge_id: String,
    pub identity_pub: PublicKeyBytes,
    pub ingress_endpoints: Vec<BridgeIngressEndpoint>,
    pub udp_punch_port: u16,
    pub reachability_class: ReachabilityClass,
    pub lease_expiry_ms: u64,
    pub capabilities: Vec<BridgeCapability>,
    pub publisher_sig: SignatureBytes,
}

impl BridgeDescriptor {
    pub fn sign(
        unsigned: BridgeDescriptorUnsigned,
        signing_key: &SigningKey,
    ) -> Result<Self, ProtocolError> {
        unsigned.validate_shape()?;
        let publisher_sig = sign_payload(&unsigned, signing_key)?;

        Ok(Self {
            bridge_id: unsigned.bridge_id,
            identity_pub: unsigned.identity_pub,
            ingress_endpoints: unsigned.ingress_endpoints,
            udp_punch_port: unsigned.udp_punch_port,
            reachability_class: unsigned.reachability_class,
            lease_expiry_ms: unsigned.lease_expiry_ms,
            capabilities: unsigned.capabilities,
            publisher_sig,
        })
    }

    pub fn unsigned_payload(&self) -> BridgeDescriptorUnsigned {
        BridgeDescriptorUnsigned {
            bridge_id: self.bridge_id.clone(),
            identity_pub: self.identity_pub.clone(),
            ingress_endpoints: self.ingress_endpoints.clone(),
            udp_punch_port: self.udp_punch_port,
            reachability_class: self.reachability_class.clone(),
            lease_expiry_ms: self.lease_expiry_ms,
            capabilities: self.capabilities.clone(),
        }
    }

    pub fn verify_signature(&self, publisher_key: &PublicKeyBytes) -> Result<(), ProtocolError> {
        let unsigned = self.unsigned_payload();
        unsigned.validate_shape()?;
        verify_payload(&unsigned, publisher_key, &self.publisher_sig)
    }

    pub fn verify_authority(
        &self,
        publisher_key: &PublicKeyBytes,
        now_ms: u64,
    ) -> Result<(), ProtocolError> {
        self.verify_signature(publisher_key)?;
        ensure_not_expired("bridge descriptor", self.lease_expiry_ms, now_ms)
    }
}
