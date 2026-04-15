//! Shared error types for the GBN protocol layer.

use thiserror::Error;

/// Errors that can occur at the protocol level.
#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("Chunk integrity verification failed: expected {expected}, got {actual}")]
    ChunkIntegrityFailure { expected: String, actual: String },

    #[error("AES-GCM decryption failed: authentication tag mismatch (chunk may be tampered)")]
    DecryptionAuthFailure,

    #[error("Invalid Publisher signature on manifest")]
    InvalidSignature,

    #[error("Session key derivation failed: {reason}")]
    KeyDerivationFailure { reason: String },

    #[error("Chunk index {index} out of range (total: {total})")]
    ChunkIndexOutOfRange { index: u32, total: u32 },

    #[error("Missing chunks: received {received} of {expected}")]
    IncompleteSession { received: u32, expected: u32 },

    #[error("Protocol version mismatch: local={local}, remote={remote}")]
    VersionMismatch { local: u32, remote: u32 },
}
