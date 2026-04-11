use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;
use std::net::SocketAddr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DhtError {
    #[error("Invalid signature on descriptor")]
    InvalidSignature,
    #[error("Signature verification error: {0}")]
    DalekError(#[from] ed25519_dalek::SignatureError),
}

/// A node's descriptor as advertised in the DHT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayDescriptor {
    /// The Ed25519 public key is the node's authoritative identity.
    pub identity_key: [u8; 32],
    /// The globally reachable IP and port.
    pub address: SocketAddr,
    /// Network/geofence tag advertised by the node (e.g. HostileSubnet/FreeSubnet).
    pub subnet_tag: String,
    /// The timestamp when this was published (prevent replay attacks).
    pub timestamp: u64,
    /// Signature of the (identity_key + address + subnet_tag + timestamp) bytes.
    #[serde(with = "BigArray")]
    pub signature: [u8; 64],
}

impl RelayDescriptor {
    /// Verify that the RelayDescriptor represents a cryptographically sound record.
    pub fn verify(&self) -> Result<(), DhtError> {
        let public_key = VerifyingKey::from_bytes(&self.identity_key).map_err(DhtError::DalekError)?;
        let sig = Signature::from_bytes(&self.signature);

        let mut signed_data = Vec::new();
        signed_data.extend_from_slice(&self.identity_key);
        signed_data.extend_from_slice(self.address.to_string().as_bytes());
        signed_data.extend_from_slice(self.subnet_tag.as_bytes());
        signed_data.extend_from_slice(&self.timestamp.to_le_bytes());

        public_key
            .verify(&signed_data, &sig)
            .map_err(|_| DhtError::InvalidSignature)?;
        Ok(())
    }
}
