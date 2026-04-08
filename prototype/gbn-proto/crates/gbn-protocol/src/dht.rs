use ed25519_dalek::{PublicKey, Signature, Verifier};
use serde::{Deserialize, Serialize};
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
    /// The timestamp when this was published (prevent replay attacks).
    pub timestamp: u64,
    /// Signature of the (identity_key + address + timestamp) bytes.
    pub signature: [u8; 64],
}

impl RelayDescriptor {
    /// Verify that the RelayDescriptor represents a cryptographically sound record.
    pub fn verify(&self) -> Result<(), DhtError> {
        let pub_key = PublicKey::from_bytes(&self.identity_key)?;
        let sig = Signature::from_bytes(&self.signature)?;

        let mut signed_data = Vec::new();
        signed_data.extend_from_slice(&self.identity_key);
        signed_data.extend_from_slice(self.address.to_string().as_bytes());
        signed_data.extend_from_slice(&self.timestamp.to_le_bytes());

        pub_key.verify(&signed_data, &sig)?;
        Ok(())
    }
}
