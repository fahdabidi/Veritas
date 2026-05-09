use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ChunkError {
    #[error("upload input must not be empty")]
    EmptyInput,

    #[error("chunk_size must be greater than zero")]
    InvalidChunkSize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    pub index: u32,
    pub total: u32,
    pub plaintext_hash: Vec<u8>,
    pub plaintext: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkedContent {
    pub chunks: Vec<Chunk>,
    pub content_hash: Vec<u8>,
    pub total_bytes: u64,
    pub chunk_size: usize,
}

pub fn chunk(input: &[u8], chunk_size: usize) -> Result<ChunkedContent, ChunkError> {
    if input.is_empty() {
        return Err(ChunkError::EmptyInput);
    }
    if chunk_size == 0 {
        return Err(ChunkError::InvalidChunkSize);
    }

    let total = input.chunks(chunk_size).count() as u32;
    let chunks = input
        .chunks(chunk_size)
        .enumerate()
        .map(|(index, plaintext)| Chunk {
            index: index as u32,
            total,
            plaintext_hash: Sha256::digest(plaintext).to_vec(),
            plaintext: plaintext.to_vec(),
        })
        .collect::<Vec<_>>();

    Ok(ChunkedContent {
        chunks,
        content_hash: Sha256::digest(input).to_vec(),
        total_bytes: input.len() as u64,
        chunk_size,
    })
}
