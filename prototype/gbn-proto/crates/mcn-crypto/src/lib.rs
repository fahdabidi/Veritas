//! # MCN Crypto
//!
//! Implements the cryptographic pipeline for the Media Creation Network:
//!
//! 1. **Key Generation**: Publisher generates a long-term Ed25519 keypair. The
//!    X25519 key is derived from the Ed25519 seed so only one keypair needs to
//!    be stored and distributed.
//! 2. **Session Creation**: Creator generates an ephemeral X25519 keypair per
//!    upload session.
//! 3. **Key Agreement**: X25519 ECDH → shared secret → HKDF-SHA256 → AES-256-GCM
//!    session key + 12-byte nonce base.
//! 4. **Per-Chunk Encryption**: AES-256-GCM with per-chunk nonce derived as
//!    `nonce_base XOR (chunk_index as little-endian u32 in first 4 bytes)`.
//! 5. **Key Destruction**: Ephemeral keys and session keys implement `Zeroize`
//!    and are zeroed on drop.
//!
//! ## Architecture Decision: Chunk-Then-Encrypt
//!
//! The MCN chunks the plaintext video *before* encrypting each chunk independently.
//! This enables:
//! - Out-of-order decryption at the Publisher (no sequential dependency)
//! - Per-chunk error isolation (one corrupted chunk doesn't invalidate the rest)
//! - Streaming encryption on memory-constrained devices (no full-file buffering)

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use gbn_protocol::{
    chunk::EncryptedChunkPacket,
    crypto::{EphemeralPublicKey, SessionKey, UploadSessionInit, X25519PublicKey},
    error::ProtocolError,
};
use hkdf::Hkdf;
use rand::{rngs::OsRng, RngCore};
use sha2::Sha256;
use x25519_dalek::{EphemeralSecret, PublicKey as X25519PubKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

// HKDF info string — encodes the protocol and version. Changing this is a
// breaking change that requires a protocol version bump.
const HKDF_INFO: &[u8] = b"GBN-MCN-v1";

// AES key length (32 bytes = 256 bits) + GCM nonce base length (12 bytes)
const AES_KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

// ─────────────────────────── Publisher Secret ─────────────────────────────

/// Publisher's long-term secret key material.
///
/// Derives a stable X25519 static secret from a 32-byte seed so that the
/// Publisher only needs to manage a single secret value.
#[derive(ZeroizeOnDrop)]
pub struct PublisherSecret {
    seed: [u8; 32],
}

impl PublisherSecret {
    /// Construct from raw seed bytes (e.g., loaded from disk).
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self { seed }
    }

    /// Extract the raw seed bytes (e.g., for saving to disk).
    pub fn to_seed(&self) -> [u8; 32] {
        self.seed
    }

    /// Derive the X25519 static secret from the seed.
    fn x25519_secret(&self) -> StaticSecret {
        StaticSecret::from(self.seed)
    }

    /// Return the corresponding X25519 public key (what the Creator uses).
    pub fn x25519_public_key(&self) -> X25519PublicKey {
        X25519PubKey::from(&self.x25519_secret()).to_bytes()
    }
}

// ─────────────────────────── Key Generation ───────────────────────────────

/// Generate a new Publisher keypair.
///
/// Returns `(secret, x25519_public_key_bytes)`. The Publisher distributes the
/// public key out-of-band (e.g., via their website or a trust anchor). The
/// secret is saved to disk and never transmitted.
pub fn generate_publisher_keypair() -> (PublisherSecret, X25519PublicKey) {
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let secret = PublisherSecret::from_seed(seed);
    let pubkey = secret.x25519_public_key();
    (secret, pubkey)
}

// ─────────────────────────── Upload Session ───────────────────────────────

/// Creator-side upload session — holds all ephemeral key material for one upload.
///
/// Dropped (and zeroized) after the upload completes. The Creator sends
/// `session_init` to the Publisher so it can reconstruct the session key.
pub struct UploadSession {
    /// The AES-256-GCM session key derived from ECDH.
    session_key: SessionKey,
    /// Base nonce — per-chunk nonce = nonce_base XOR chunk_index (first 4 bytes).
    nonce_base: [u8; NONCE_LEN],
    /// Public information sent to the Publisher to bootstrap decryption.
    pub session_init: UploadSessionInit,
}

impl Drop for UploadSession {
    fn drop(&mut self) {
        // Zeroize only the secret key material; session_init is public info.
        self.nonce_base.zeroize();
        // SessionKey itself implements ZeroizeOnDrop via its own derive.
    }
}

/// Establish a new upload session targeting a specific Publisher.
///
/// Generates an ephemeral X25519 keypair, performs ECDH with the Publisher's
/// static X25519 key, and derives the AES session key + nonce base via
/// HKDF-SHA256. The Creator's ephemeral private key is consumed in this call.
pub fn create_upload_session(
    publisher_x25519_pubkey: &X25519PublicKey,
    total_chunks: u32,
    content_hash: [u8; 32],
) -> Result<UploadSession, ProtocolError> {
    // Generate ephemeral keypair — EphemeralSecret is consumed on DH and zeroized
    let ephemeral_secret = EphemeralSecret::random_from_rng(OsRng);
    let ephemeral_pubkey: EphemeralPublicKey = X25519PubKey::from(&ephemeral_secret).to_bytes();

    // ECDH
    let publisher_pub = X25519PubKey::from(*publisher_x25519_pubkey);
    let shared_secret = ephemeral_secret.diffie_hellman(&publisher_pub);

    // Unique session ID
    let mut session_id = [0u8; 16];
    OsRng.fill_bytes(&mut session_id);

    // HKDF: extract + expand
    // IKM  = shared_secret bytes
    // Salt = session_id (adds per-session entropy even if shared secret repeats)
    // Info = HKDF_INFO constant
    // OKM  = 44 bytes: 32 (AES key) + 12 (nonce base)
    let hk = Hkdf::<Sha256>::new(Some(&session_id), shared_secret.as_bytes());
    let mut okm = [0u8; AES_KEY_LEN + NONCE_LEN];
    hk.expand(HKDF_INFO, &mut okm)
        .map_err(|_| ProtocolError::KeyDerivationFailure {
            reason: "HKDF expand failed (OKM too long)".into(),
        })?;

    let mut key_bytes = [0u8; AES_KEY_LEN];
    let mut nonce_base = [0u8; NONCE_LEN];
    key_bytes.copy_from_slice(&okm[..AES_KEY_LEN]);
    nonce_base.copy_from_slice(&okm[AES_KEY_LEN..]);
    okm.zeroize();

    let session_key = SessionKey(key_bytes);

    let session_init = UploadSessionInit {
        ephemeral_pubkey,
        publisher_pubkey: *publisher_x25519_pubkey,
        session_id,
        total_chunks,
        content_hash,
    };

    Ok(UploadSession {
        session_key,
        nonce_base,
        session_init,
    })
}

impl UploadSession {
    /// Encrypt a single plaintext chunk.
    ///
    /// `chunk_index` must be unique per session. The nonce is derived as
    /// `nonce_base XOR chunk_index` (XOR applied to the first 4 bytes only,
    /// matching little-endian u32 representation).
    pub fn encrypt_chunk(
        &self,
        chunk_index: u32,
        plaintext: &[u8],
        plaintext_hash: [u8; 32],
    ) -> Result<EncryptedChunkPacket, ProtocolError> {
        let nonce_bytes = derive_nonce(&self.nonce_base, chunk_index);
        let key = Key::<Aes256Gcm>::from_slice(&self.session_key.0);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|_| ProtocolError::DecryptionAuthFailure)?;

        Ok(EncryptedChunkPacket {
            session_id: self.session_init.session_id,
            chunk_index,
            total_chunks: self.session_init.total_chunks,
            plaintext_hash,
            nonce: nonce_bytes,
            ciphertext,
        })
    }
}

// ─────────────────────────── Publisher Decryption ─────────────────────────

/// Decrypt a single chunk on the Publisher side.
///
/// The Publisher reconstructs the shared secret using its long-term X25519
/// static secret and the Creator's ephemeral public key from `session_init`,
/// then re-derives the identical session key via HKDF, and decrypts the chunk.
pub fn decrypt_chunk(
    publisher_secret: &PublisherSecret,
    session_init: &UploadSessionInit,
    packet: &EncryptedChunkPacket,
) -> Result<Vec<u8>, ProtocolError> {
    // Re-derive session key from the ephemeral public key
    let ephemeral_pub = X25519PubKey::from(session_init.ephemeral_pubkey);
    let static_secret = publisher_secret.x25519_secret();
    let shared_secret = static_secret.diffie_hellman(&ephemeral_pub);

    let hk = Hkdf::<Sha256>::new(Some(&session_init.session_id), shared_secret.as_bytes());
    let mut okm = [0u8; AES_KEY_LEN + NONCE_LEN];
    hk.expand(HKDF_INFO, &mut okm)
        .map_err(|_| ProtocolError::KeyDerivationFailure {
            reason: "HKDF expand failed on Publisher side".into(),
        })?;

    let key_bytes = &okm[..AES_KEY_LEN];
    let key = Key::<Aes256Gcm>::from_slice(key_bytes);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(&packet.nonce);

    let plaintext = cipher
        .decrypt(nonce, packet.ciphertext.as_ref())
        .map_err(|_| ProtocolError::DecryptionAuthFailure)?;

    okm.zeroize();
    Ok(plaintext)
}

// ─────────────────────────── Nonce Derivation ─────────────────────────────

/// Derive a per-chunk nonce by XOR-ing the first 4 bytes of `nonce_base` with
/// the little-endian representation of `chunk_index`.
///
/// This guarantees unique nonces for up to 2^32 chunks per session.
fn derive_nonce(nonce_base: &[u8; NONCE_LEN], chunk_index: u32) -> [u8; NONCE_LEN] {
    let mut nonce = *nonce_base;
    let index_bytes = chunk_index.to_le_bytes();
    nonce[0] ^= index_bytes[0];
    nonce[1] ^= index_bytes[1];
    nonce[2] ^= index_bytes[2];
    nonce[3] ^= index_bytes[3];
    nonce
}

// ─────────────────────────────── Tests ───────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_hash() -> [u8; 32] {
        [0xab; 32]
    }

    fn make_session(
        publisher_pub: &X25519PublicKey,
        chunks: u32,
    ) -> UploadSession {
        create_upload_session(publisher_pub, chunks, dummy_hash()).unwrap()
    }

    // T-CRYPTO-1: Basic encrypt → decrypt roundtrip for a single chunk.
    #[test]
    fn test_roundtrip_single_chunk() {
        let (secret, pubkey) = generate_publisher_keypair();
        let session = make_session(&pubkey, 1);

        let plaintext = b"Hello, GBN! This is a test chunk.";
        let hash = dummy_hash();
        let packet = session.encrypt_chunk(0, plaintext, hash).unwrap();

        let decrypted = decrypt_chunk(&secret, &session.session_init, &packet).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    // T-CRYPTO-2: 100 chunks with 1KB of random data each all roundtrip correctly.
    #[test]
    fn test_roundtrip_100_chunks() {
        let (secret, pubkey) = generate_publisher_keypair();
        let num_chunks = 100u32;
        let session = make_session(&pubkey, num_chunks);

        let mut chunks: Vec<Vec<u8>> = Vec::new();
        let mut packets: Vec<EncryptedChunkPacket> = Vec::new();

        for i in 0..num_chunks {
            let mut data = vec![0u8; 1024];
            OsRng.fill_bytes(&mut data);
            let hash = dummy_hash();
            let packet = session.encrypt_chunk(i, &data, hash).unwrap();
            packets.push(packet);
            chunks.push(data);
        }

        // Decrypt in reverse order to prove order independence
        for i in (0..num_chunks as usize).rev() {
            let decrypted = decrypt_chunk(&secret, &session.session_init, &packets[i]).unwrap();
            assert_eq!(decrypted, chunks[i], "Chunk {i} failed to decrypt correctly");
        }
    }

    // T-CRYPTO-3: Decrypting with a different Publisher key must fail.
    #[test]
    fn test_wrong_key_fails() {
        let (_correct_secret, pubkey) = generate_publisher_keypair();
        let (wrong_secret, _) = generate_publisher_keypair();
        let session = make_session(&pubkey, 1);

        let plaintext = b"Secret data";
        let packet = session.encrypt_chunk(0, plaintext, dummy_hash()).unwrap();

        let result = decrypt_chunk(&wrong_secret, &session.session_init, &packet);
        assert!(
            matches!(result, Err(ProtocolError::DecryptionAuthFailure)),
            "Expected DecryptionAuthFailure, got: {:?}",
            result
        );
    }

    // T-CRYPTO-4: Flipping any single bit in the ciphertext must fail GCM auth.
    #[test]
    fn test_tampered_ciphertext_fails() {
        let (secret, pubkey) = generate_publisher_keypair();
        let session = make_session(&pubkey, 1);

        let plaintext = b"Tamper me if you dare";
        let mut packet = session.encrypt_chunk(0, plaintext, dummy_hash()).unwrap();

        // Flip a bit in the middle of the ciphertext
        let mid = packet.ciphertext.len() / 2;
        packet.ciphertext[mid] ^= 0x01;

        let result = decrypt_chunk(&secret, &session.session_init, &packet);
        assert!(
            matches!(result, Err(ProtocolError::DecryptionAuthFailure)),
            "Expected auth failure after tampering, got: {:?}",
            result
        );
    }

    // T-CRYPTO-5: Each chunk must have a unique nonce — no nonce reuse within a session.
    #[test]
    fn test_nonce_uniqueness() {
        let (_secret, pubkey) = generate_publisher_keypair();
        let num_chunks = 10u32;
        let session = make_session(&pubkey, num_chunks);

        let data = vec![0u8; 64];
        let nonces: Vec<[u8; 12]> = (0..num_chunks)
            .map(|i| {
                session
                    .encrypt_chunk(i, &data, dummy_hash())
                    .unwrap()
                    .nonce
            })
            .collect();

        // All nonces must be distinct
        for i in 0..nonces.len() {
            for j in (i + 1)..nonces.len() {
                assert_ne!(
                    nonces[i], nonces[j],
                    "Nonce collision between chunk {i} and chunk {j}"
                );
            }
        }
    }

    // T-CRYPTO-6: Two independent sessions produce different session keys;
    // chunks from session A cannot be decrypted using session B's init params.
    #[test]
    fn test_session_isolation() {
        let (secret, pubkey) = generate_publisher_keypair();
        let session_a = make_session(&pubkey, 1);
        let session_b = make_session(&pubkey, 1);

        let plaintext = b"Session isolation test";
        let packet_a = session_a.encrypt_chunk(0, plaintext, dummy_hash()).unwrap();

        // Try to decrypt packet_a using session_b's init — must fail
        let result = decrypt_chunk(&secret, &session_b.session_init, &packet_a);
        assert!(
            matches!(result, Err(ProtocolError::DecryptionAuthFailure)),
            "Cross-session decryption should fail, got: {:?}",
            result
        );
    }

    // T-CRYPTO-7: Large chunk (1 MB) roundtrips correctly — validates no off-by-one
    // in ciphertext buffer handling.
    #[test]
    fn test_large_chunk_1mb() {
        let (secret, pubkey) = generate_publisher_keypair();
        let session = make_session(&pubkey, 1);

        let mut data = vec![0u8; 1024 * 1024]; // 1 MB
        OsRng.fill_bytes(&mut data);

        let packet = session.encrypt_chunk(0, &data, dummy_hash()).unwrap();
        let decrypted = decrypt_chunk(&secret, &session.session_init, &packet).unwrap();

        assert_eq!(decrypted.len(), data.len());
        assert_eq!(decrypted, data);
    }
}
