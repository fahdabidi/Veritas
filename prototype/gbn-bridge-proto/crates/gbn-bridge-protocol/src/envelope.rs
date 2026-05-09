use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use ed25519_dalek::SigningKey;
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::error::ProtocolError;
use crate::signing::PublicKeyBytes;

const KEY_INFO: &[u8] = b"veritas/conduit/v2/upload-content-key";
const NONCE_INFO: &[u8] = b"veritas/conduit/v2/nonce";
const SESSION_ID_LEN: usize = 16;
const GCM_TAG_LEN: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeKeyDerivation {
    PublisherX25519HkdfAes256GcmV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedFrame {
    pub key_derivation: EnvelopeKeyDerivation,
    pub creator_ephemeral_pubkey: PublicKeyBytes,
    pub publisher_key_id: String,
    pub session_id: Vec<u8>,
    pub chunk_index: u32,
    pub total_chunks: u32,
    pub plaintext_hash: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub auth_tag: Vec<u8>,
}

pub fn publisher_encryption_private_from_signing_key(signing_key: &SigningKey) -> [u8; 32] {
    signing_key.to_bytes()
}

pub fn publisher_encryption_identity(signing_key: &SigningKey) -> PublicKeyBytes {
    let private = publisher_encryption_private_from_signing_key(signing_key);
    let public = PublicKey::from(&StaticSecret::from(private));
    PublicKeyBytes(public.as_bytes().to_vec())
}

pub fn encrypt_for_publisher(
    plaintext: &[u8],
    publisher_x25519_pubkey: &PublicKeyBytes,
    publisher_key_id: impl Into<String>,
    session_id: [u8; SESSION_ID_LEN],
    chunk_index: u32,
    total_chunks: u32,
    creator_ephemeral_private: [u8; 32],
) -> Result<EncryptedFrame, ProtocolError> {
    let publisher_pubkey = x25519_public_key(publisher_x25519_pubkey)?;
    let creator_secret = StaticSecret::from(creator_ephemeral_private);
    let creator_pubkey = PublicKey::from(&creator_secret);
    let shared_secret = creator_secret.diffie_hellman(&publisher_pubkey);
    let key = derive_bytes(shared_secret.as_bytes(), KEY_INFO, 32)?;
    let nonce_base = derive_bytes(shared_secret.as_bytes(), NONCE_INFO, 12)?;
    let nonce = nonce_for_chunk(&nonce_base, chunk_index)?;
    let plaintext_hash = Sha256::digest(plaintext).to_vec();
    let aad = envelope_aad(&session_id, chunk_index, total_chunks, &plaintext_hash);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|error| {
        ProtocolError::Envelope(format!("failed to initialize AES-256-GCM: {error}"))
    })?;
    let mut encrypted = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| ProtocolError::Envelope("AES-256-GCM encryption failed".to_string()))?;
    if encrypted.len() < GCM_TAG_LEN {
        return Err(ProtocolError::Envelope(
            "AES-256-GCM output was shorter than authentication tag".to_string(),
        ));
    }
    let auth_tag = encrypted.split_off(encrypted.len() - GCM_TAG_LEN);

    Ok(EncryptedFrame {
        key_derivation: EnvelopeKeyDerivation::PublisherX25519HkdfAes256GcmV1,
        creator_ephemeral_pubkey: PublicKeyBytes(creator_pubkey.as_bytes().to_vec()),
        publisher_key_id: publisher_key_id.into(),
        session_id: session_id.to_vec(),
        chunk_index,
        total_chunks,
        plaintext_hash,
        ciphertext: encrypted,
        auth_tag,
    })
}

pub fn decrypt_from_creator(
    frame: &EncryptedFrame,
    publisher_x25519_private: [u8; 32],
) -> Result<Vec<u8>, ProtocolError> {
    if frame.session_id.len() != SESSION_ID_LEN {
        return Err(ProtocolError::Envelope(format!(
            "encrypted frame session_id length must be {SESSION_ID_LEN}, got {}",
            frame.session_id.len()
        )));
    }
    if frame.auth_tag.len() != GCM_TAG_LEN {
        return Err(ProtocolError::Envelope(format!(
            "encrypted frame auth_tag length must be {GCM_TAG_LEN}, got {}",
            frame.auth_tag.len()
        )));
    }
    let creator_pubkey = x25519_public_key(&frame.creator_ephemeral_pubkey)?;
    let publisher_secret = StaticSecret::from(publisher_x25519_private);
    let shared_secret = publisher_secret.diffie_hellman(&creator_pubkey);
    let key = derive_bytes(shared_secret.as_bytes(), KEY_INFO, 32)?;
    let nonce_base = derive_bytes(shared_secret.as_bytes(), NONCE_INFO, 12)?;
    let nonce = nonce_for_chunk(&nonce_base, frame.chunk_index)?;
    let session_id: [u8; SESSION_ID_LEN] =
        frame.session_id.as_slice().try_into().map_err(|_| {
            ProtocolError::Envelope("invalid encrypted frame session id".to_string())
        })?;
    let aad = envelope_aad(
        &session_id,
        frame.chunk_index,
        frame.total_chunks,
        &frame.plaintext_hash,
    );
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|error| {
        ProtocolError::Envelope(format!("failed to initialize AES-256-GCM: {error}"))
    })?;
    let mut combined = frame.ciphertext.clone();
    combined.extend_from_slice(&frame.auth_tag);
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &combined,
                aad: &aad,
            },
        )
        .map_err(|_| ProtocolError::Envelope("AES-256-GCM decryption failed".to_string()))?;
    let actual_hash = Sha256::digest(&plaintext).to_vec();
    if actual_hash != frame.plaintext_hash {
        return Err(ProtocolError::Envelope(
            "encrypted frame plaintext_hash mismatch".to_string(),
        ));
    }
    Ok(plaintext)
}

fn x25519_public_key(value: &PublicKeyBytes) -> Result<PublicKey, ProtocolError> {
    let bytes: [u8; 32] =
        value
            .0
            .as_slice()
            .try_into()
            .map_err(|_| ProtocolError::InvalidPublicKeyLength {
                actual: value.0.len(),
            })?;
    Ok(PublicKey::from(bytes))
}

fn derive_bytes(shared_secret: &[u8], info: &[u8], len: usize) -> Result<Vec<u8>, ProtocolError> {
    let hk = Hkdf::<Sha256>::new(None, shared_secret);
    let mut out = vec![0_u8; len];
    hk.expand(info, &mut out)
        .map_err(|_| ProtocolError::Envelope("HKDF expansion failed".to_string()))?;
    Ok(out)
}

fn nonce_for_chunk(nonce_base: &[u8], chunk_index: u32) -> Result<[u8; 12], ProtocolError> {
    let mut nonce: [u8; 12] = nonce_base
        .try_into()
        .map_err(|_| ProtocolError::Envelope("nonce base must be 12 bytes".to_string()))?;
    let index = chunk_index.to_be_bytes();
    for (dst, src) in nonce[8..].iter_mut().zip(index) {
        *dst ^= src;
    }
    Ok(nonce)
}

fn envelope_aad(
    session_id: &[u8; SESSION_ID_LEN],
    chunk_index: u32,
    total_chunks: u32,
    plaintext_hash: &[u8],
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(SESSION_ID_LEN + 4 + 4 + plaintext_hash.len());
    aad.extend_from_slice(session_id);
    aad.extend_from_slice(&chunk_index.to_le_bytes());
    aad.extend_from_slice(&total_chunks.to_le_bytes());
    aad.extend_from_slice(plaintext_hash);
    aad
}
