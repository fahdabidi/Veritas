use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::error::ProtocolError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PublicKeyBytes(pub Vec<u8>);

impl PublicKeyBytes {
    pub fn from_verifying_key(value: &VerifyingKey) -> Self {
        Self(value.to_bytes().to_vec())
    }

    pub fn to_verifying_key(&self) -> Result<VerifyingKey, ProtocolError> {
        let bytes: [u8; 32] =
            self.0
                .as_slice()
                .try_into()
                .map_err(|_| ProtocolError::InvalidPublicKeyLength {
                    actual: self.0.len(),
                })?;
        VerifyingKey::from_bytes(&bytes).map_err(|_| ProtocolError::InvalidSignature)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SignatureBytes(pub Vec<u8>);

impl SignatureBytes {
    pub fn from_signature(value: &Signature) -> Self {
        Self(value.to_bytes().to_vec())
    }

    pub fn to_signature(&self) -> Result<Signature, ProtocolError> {
        Signature::from_slice(&self.0).map_err(|_| ProtocolError::InvalidSignatureLength {
            actual: self.0.len(),
        })
    }
}

pub fn publisher_identity(signing_key: &SigningKey) -> PublicKeyBytes {
    PublicKeyBytes::from_verifying_key(&signing_key.verifying_key())
}

pub fn canonical_json_bytes<T>(payload: &T) -> Result<Vec<u8>, ProtocolError>
where
    T: Serialize,
{
    serde_json::to_vec(payload).map_err(Into::into)
}

pub fn sign_payload<T>(
    payload: &T,
    signing_key: &SigningKey,
) -> Result<SignatureBytes, ProtocolError>
where
    T: Serialize,
{
    let bytes = canonical_json_bytes(payload)?;
    Ok(SignatureBytes::from_signature(&signing_key.sign(&bytes)))
}

pub fn verify_payload<T>(
    payload: &T,
    verifying_key: &PublicKeyBytes,
    signature: &SignatureBytes,
) -> Result<(), ProtocolError>
where
    T: Serialize,
{
    let bytes = canonical_json_bytes(payload)?;
    let verifying_key = verifying_key.to_verifying_key()?;
    let signature = signature.to_signature()?;
    verifying_key
        .verify(&bytes, &signature)
        .map_err(|_| ProtocolError::InvalidSignature)
}

pub fn ensure_not_expired(
    object: &'static str,
    expiry_ms: u64,
    now_ms: u64,
) -> Result<(), ProtocolError> {
    if now_ms > expiry_ms {
        return Err(ProtocolError::Expired {
            object,
            expiry_ms,
            now_ms,
        });
    }

    Ok(())
}
