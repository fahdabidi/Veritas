//! # MCN Chunker
//!
//! Splits a sanitized video file into fixed-size chunks and generates a
//! BLAKE3 content-addressed manifest.
//!
//! ## Design
//!
//! - **Streaming**: reads the file in `chunk_size` increments — only 2 chunks
//!   held in memory at any moment (current chunk + hasher state).
//! - **Last-chunk padding**: the final chunk is zero-padded to `chunk_size`.
//!   `ChunkInfo.size` stores the real payload length for correct unpadding on
//!   reassembly. This prevents traffic analysis from revealing file size via
//!   the last chunk's wire size.
//! - **Dual hashing**: BLAKE3 is computed per-chunk (for post-decrypt
//!   verification) AND over the entire file in a single streaming pass (for
//!   the `ChunkManifest.content_hash` field).
//! - **Session ID**: a 16-byte random value generated fresh per chunking call,
//!   uniquely identifying one upload session.

use std::{
    fs::File,
    io::{BufReader, Read, Write},
    path::Path,
};

use blake3::Hasher;
use gbn_protocol::{
    chunk::{ChunkInfo, ChunkManifest},
    DEFAULT_MCN_CHUNK_SIZE,
};
use rand::{rngs::OsRng, RngCore};
use thiserror::Error;

// ─────────────────────────── Error Type ──────────────────────────────────

#[derive(Debug, Error)]
pub enum ChunkerError {
    #[error("I/O error during chunking: {0}")]
    Io(#[from] std::io::Error),

    #[error("Input file is empty")]
    EmptyFile,

    #[error("Input data is empty")]
    EmptyInput,

    #[error("Chunk index {index} out of range (expected < {total})")]
    IndexOutOfRange { index: usize, total: usize },

    #[error("Manifest chunk count ({manifest}) does not match data chunk count ({data})")]
    ManifestMismatch { manifest: usize, data: usize },
}

// ─────────────────────────── Chunking ────────────────────────────────────

/// Split a file into fixed-size chunks, returning all chunk data and a manifest.
///
/// This loads all chunks into memory. For large files, prefer
/// [`chunk_file_streaming`] to keep peak memory proportional to `chunk_size`,
/// not file size.
///
/// # Arguments
/// - `path` — path to the sanitized input file
/// - `chunk_size` — size of each chunk in bytes (default: `DEFAULT_MCN_CHUNK_SIZE`)
///
/// # Returns
/// `(chunks, manifest)` where `chunks[i]` corresponds to `manifest.chunks[i]`.
/// All chunks (including the last) are exactly `chunk_size` bytes due to
/// zero-padding; use `manifest.chunks[i].size` to get the actual payload length.
pub fn chunk_file(
    path: impl AsRef<Path>,
    chunk_size: usize,
) -> Result<(Vec<Vec<u8>>, ChunkManifest), ChunkerError> {
    let chunk_size = if chunk_size == 0 {
        DEFAULT_MCN_CHUNK_SIZE
    } else {
        chunk_size
    };

    let file = File::open(path)?;
    let total_size = file.metadata()?.len();

    if total_size == 0 {
        return Err(ChunkerError::EmptyFile);
    }

    let mut reader = BufReader::new(file);
    let mut session_id = [0u8; 16];
    OsRng.fill_bytes(&mut session_id);

    let mut chunks = Vec::new();
    let mut chunk_infos = Vec::new();
    let mut file_hasher = Hasher::new();
    let mut index: u32 = 0;
    let mut eof = false;

    while !eof {
        let mut buf = vec![0u8; chunk_size];
        let mut bytes_read = 0;

        // Fill the buffer, handling short reads
        while bytes_read < chunk_size {
            match reader.read(&mut buf[bytes_read..])? {
                0 => {
                    eof = true;
                    break;
                }
                n => bytes_read += n,
            }
        }

        if bytes_read == 0 {
            break;
        }

        // Update the whole-file hasher with the real (un-padded) data
        file_hasher.update(&buf[..bytes_read]);

        let actual_size = bytes_read as u32;

        // Zero-pad to chunk_size if this is the last (short) chunk
        // buf is already zeroed beyond bytes_read since we allocated with vec![0u8; chunk_size]

        // BLAKE3 hash of the padded chunk (what the Publisher will verify post-decrypt)
        let chunk_hash: [u8; 32] = *blake3::hash(&buf).as_bytes();

        chunk_infos.push(ChunkInfo {
            index,
            hash: chunk_hash,
            size: actual_size,
        });
        chunks.push(buf);
        index += 1;
    }

    let content_hash: [u8; 32] = *file_hasher.finalize().as_bytes();

    let manifest = ChunkManifest {
        session_id,
        total_chunks: index,
        content_hash,
        total_size,
        chunks: chunk_infos,
    };

    Ok((chunks, manifest))
}

/// Streaming variant of [`chunk_file`].
///
/// Calls `callback(chunk_index, chunk_data, chunk_info)` for each chunk as it
/// is produced. Only one chunk is held in memory at a time. Returns the
/// completed manifest after all chunks have been emitted.
///
/// Use this when you want to encrypt-and-send each chunk immediately rather
/// than buffering the entire file.
pub fn chunk_file_streaming<F>(
    path: impl AsRef<Path>,
    chunk_size: usize,
    mut callback: F,
) -> Result<ChunkManifest, ChunkerError>
where
    F: FnMut(u32, &[u8], &ChunkInfo) -> Result<(), ChunkerError>,
{
    let chunk_size = if chunk_size == 0 {
        DEFAULT_MCN_CHUNK_SIZE
    } else {
        chunk_size
    };

    let file = File::open(path)?;
    let total_size = file.metadata()?.len();

    if total_size == 0 {
        return Err(ChunkerError::EmptyFile);
    }

    let mut reader = BufReader::new(file);
    let mut session_id = [0u8; 16];
    OsRng.fill_bytes(&mut session_id);

    let mut chunk_infos = Vec::new();
    let mut file_hasher = Hasher::new();
    let mut index: u32 = 0;
    let mut eof = false;

    while !eof {
        let mut buf = vec![0u8; chunk_size];
        let mut bytes_read = 0;

        while bytes_read < chunk_size {
            match reader.read(&mut buf[bytes_read..])? {
                0 => {
                    eof = true;
                    break;
                }
                n => bytes_read += n,
            }
        }

        if bytes_read == 0 {
            break;
        }

        file_hasher.update(&buf[..bytes_read]);
        let actual_size = bytes_read as u32;
        let chunk_hash: [u8; 32] = *blake3::hash(&buf).as_bytes();

        let info = ChunkInfo {
            index,
            hash: chunk_hash,
            size: actual_size,
        };

        callback(index, &buf, &info)?;
        chunk_infos.push(info);
        index += 1;
    }

    let content_hash: [u8; 32] = *file_hasher.finalize().as_bytes();

    Ok(ChunkManifest {
        session_id,
        total_chunks: index,
        content_hash,
        total_size,
        chunks: chunk_infos,
    })
}

// ─────────────────────────── In-Memory Chunking ──────────────────────────

/// Split raw bytes into fixed-size chunks, returning all chunk data and a manifest.
///
/// Identical to [`chunk_file`] but operates on an in-memory `&[u8]` instead of
/// a file path. Used in ECS where the synthetic payload is generated in memory.
///
/// # Arguments
/// - `data` — raw bytes to chunk
/// - `chunk_size` — size of each chunk in bytes (default: `DEFAULT_MCN_CHUNK_SIZE`)
///
/// # Returns
/// `(chunks, manifest)` where `chunks[i]` corresponds to `manifest.chunks[i]`.
/// All chunks are exactly `chunk_size` bytes (zero-padded); use
/// `manifest.chunks[i].size` for actual payload length.
pub fn chunk_bytes(
    data: &[u8],
    chunk_size: usize,
) -> Result<(Vec<Vec<u8>>, ChunkManifest), ChunkerError> {
    let chunk_size = if chunk_size == 0 {
        DEFAULT_MCN_CHUNK_SIZE
    } else {
        chunk_size
    };

    if data.is_empty() {
        return Err(ChunkerError::EmptyInput);
    }

    let mut session_id = [0u8; 16];
    OsRng.fill_bytes(&mut session_id);

    let mut chunks = Vec::new();
    let mut chunk_infos = Vec::new();
    let mut file_hasher = Hasher::new();
    let mut index: u32 = 0;
    let mut offset = 0usize;
    let total_size = data.len() as u64;

    while offset < data.len() {
        let end = (offset + chunk_size).min(data.len());
        let actual_size = (end - offset) as u32;

        // Update whole-payload hasher with real (un-padded) data
        file_hasher.update(&data[offset..end]);

        // Zero-pad to chunk_size
        let mut buf = vec![0u8; chunk_size];
        buf[..actual_size as usize].copy_from_slice(&data[offset..end]);

        let chunk_hash: [u8; 32] = *blake3::hash(&buf).as_bytes();

        chunk_infos.push(ChunkInfo {
            index,
            hash: chunk_hash,
            size: actual_size,
        });
        chunks.push(buf);
        index += 1;
        offset = end;
    }

    let content_hash: [u8; 32] = *file_hasher.finalize().as_bytes();

    let manifest = ChunkManifest {
        session_id,
        total_chunks: index,
        content_hash,
        total_size,
        chunks: chunk_infos,
    };

    Ok((chunks, manifest))
}

/// Compute BLAKE3 hash of raw bytes.
///
/// Equivalent to [`hash_file`] but for in-memory data.
pub fn hash_bytes(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

// ─────────────────────────── Reassembly ──────────────────────────────────

/// Reassemble chunks back into the original file, stripping padding from the
/// final chunk.
///
/// Chunks can be provided in any order — they are written to `output_path` in
/// the sequence order defined by `manifest.chunks[i].index`. All chunks must
/// be present.
///
/// # Arguments
/// - `chunks` — chunk data indexed by position (chunks[i] = data for chunk index i)
/// - `manifest` — the manifest produced by [`chunk_file`]
/// - `output_path` — where to write the reassembled file
pub fn reassemble_chunks(
    chunks: &[Vec<u8>],
    manifest: &ChunkManifest,
    output_path: impl AsRef<Path>,
) -> Result<(), ChunkerError> {
    if chunks.len() != manifest.chunks.len() {
        return Err(ChunkerError::ManifestMismatch {
            manifest: manifest.chunks.len(),
            data: chunks.len(),
        });
    }

    let mut out = File::create(output_path)?;

    for info in &manifest.chunks {
        let idx = info.index as usize;
        if idx >= chunks.len() {
            return Err(ChunkerError::IndexOutOfRange {
                index: idx,
                total: chunks.len(),
            });
        }
        let data = &chunks[idx];
        // Write only the actual payload (strips zero-padding from last chunk)
        out.write_all(&data[..info.size as usize])?;
    }

    out.flush()?;
    Ok(())
}

// ─────────────────────────── Integrity Check ─────────────────────────────

/// Verify that a chunk's data matches its expected BLAKE3 hash.
///
/// Called by the Publisher after decryption to confirm the chunk hasn't been
/// corrupted in transit (separate from the GCM auth tag check).
pub fn verify_chunk_hash(data: &[u8], expected_hash: &[u8; 32]) -> bool {
    let actual: [u8; 32] = *blake3::hash(data).as_bytes();
    actual == *expected_hash
}

/// Compute BLAKE3 hash of a complete file (for final integrity check).
pub fn hash_file(path: impl AsRef<Path>) -> Result<[u8; 32], ChunkerError> {
    let mut file = BufReader::new(File::open(path)?);
    let mut hasher = Hasher::new();
    let mut buf = vec![0u8; 65536]; // 64KB read buffer
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(*hasher.finalize().as_bytes())
}

// ─────────────────────────────── Tests ───────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Create a temp file with given content and return it.
    fn make_temp_file(content: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content).unwrap();
        f.flush().unwrap();
        f
    }

    /// SHA-256 hash of a byte slice for end-to-end identity checks.
    fn sha256(data: &[u8]) -> [u8; 32] {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher as StdHasher};
        // Use blake3 for simplicity (sha2 not in chunker deps)
        *blake3::hash(data).as_bytes()
    }

    // T-CHUNK-1: chunk then reassemble returns byte-identical content.
    #[test]
    fn test_chunk_reassemble_identity() {
        let content: Vec<u8> = (0u8..=255).cycle().take(5000).collect();
        let f = make_temp_file(&content);

        let chunk_size = 1024;
        let (chunks, manifest) = chunk_file(f.path(), chunk_size).unwrap();

        let out = NamedTempFile::new().unwrap();
        reassemble_chunks(&chunks, &manifest, out.path()).unwrap();

        let reassembled = std::fs::read(out.path()).unwrap();
        assert_eq!(
            reassembled.len(),
            content.len(),
            "Reassembled length mismatch"
        );
        assert_eq!(
            sha256(&reassembled),
            sha256(&content),
            "Content hash mismatch"
        );
    }

    // T-CHUNK-2: file not divisible by chunk_size — last chunk padded, but
    // reassembly strips padding and returns original length exactly.
    #[test]
    fn test_unaligned_file_padding() {
        let chunk_size = 1000;
        // 2500 bytes = 2 full chunks + 500-byte remainder → 3 chunks
        let content: Vec<u8> = (0u8..100).cycle().take(2500).collect();
        let f = make_temp_file(&content);

        let (chunks, manifest) = chunk_file(f.path(), chunk_size).unwrap();

        assert_eq!(manifest.total_chunks, 3, "Expected 3 chunks");
        assert_eq!(chunks[0].len(), chunk_size, "Chunk 0 wrong size");
        assert_eq!(chunks[1].len(), chunk_size, "Chunk 1 wrong size");
        assert_eq!(
            chunks[2].len(),
            chunk_size,
            "Chunk 2 should be padded to chunk_size"
        );
        assert_eq!(
            manifest.chunks[2].size, 500,
            "Last chunk actual size should be 500"
        );

        let out = NamedTempFile::new().unwrap();
        reassemble_chunks(&chunks, &manifest, out.path()).unwrap();
        let reassembled = std::fs::read(out.path()).unwrap();

        assert_eq!(
            reassembled.len(),
            2500,
            "Reassembled should be exactly 2500 bytes"
        );
        assert_eq!(sha256(&reassembled), sha256(&content));
    }

    // T-CHUNK-3: single-byte file produces exactly 1 chunk.
    #[test]
    fn test_single_byte_file() {
        let f = make_temp_file(&[0x42]);
        let (chunks, manifest) = chunk_file(f.path(), 1024).unwrap();

        assert_eq!(manifest.total_chunks, 1);
        assert_eq!(chunks.len(), 1);
        assert_eq!(manifest.chunks[0].size, 1, "Actual size should be 1");

        let out = NamedTempFile::new().unwrap();
        reassemble_chunks(&chunks, &manifest, out.path()).unwrap();
        let reassembled = std::fs::read(out.path()).unwrap();
        assert_eq!(reassembled, vec![0x42u8]);
    }

    // T-CHUNK-4: empty file returns EmptyFile error.
    #[test]
    fn test_empty_file_error() {
        let f = make_temp_file(&[]);
        let result = chunk_file(f.path(), 1024);
        assert!(
            matches!(result, Err(ChunkerError::EmptyFile)),
            "Expected EmptyFile error, got: {:?}",
            result
        );
    }

    // T-CHUNK-5: manifest chunk hashes match actual BLAKE3 hash of padded chunk data.
    #[test]
    fn test_manifest_hashes_correct() {
        let content: Vec<u8> = (0u8..=255).cycle().take(3072).collect();
        let f = make_temp_file(&content);

        let (chunks, manifest) = chunk_file(f.path(), 1024).unwrap();

        for (i, info) in manifest.chunks.iter().enumerate() {
            let actual_hash: [u8; 32] = *blake3::hash(&chunks[i]).as_bytes();
            assert_eq!(
                actual_hash, info.hash,
                "Chunk {i} hash in manifest does not match actual BLAKE3"
            );
            assert!(
                verify_chunk_hash(&chunks[i], &info.hash),
                "verify_chunk_hash failed for chunk {i}"
            );
        }
    }

    // T-CHUNK-6: streaming variant produces identical manifest to batch variant.
    #[test]
    fn test_streaming_matches_batch() {
        let content: Vec<u8> = (0u8..=255).cycle().take(5000).collect();
        let f = make_temp_file(&content);

        let (batch_chunks, batch_manifest) = chunk_file(f.path(), 1024).unwrap();

        let mut streaming_chunks: Vec<Vec<u8>> = Vec::new();
        let streaming_manifest = chunk_file_streaming(f.path(), 1024, |_idx, data, _info| {
            streaming_chunks.push(data.to_vec());
            Ok(())
        })
        .unwrap();

        // Same number of chunks
        assert_eq!(batch_manifest.total_chunks, streaming_manifest.total_chunks);
        assert_eq!(batch_manifest.total_size, streaming_manifest.total_size);
        assert_eq!(batch_manifest.content_hash, streaming_manifest.content_hash);

        // Same chunk data
        for i in 0..batch_chunks.len() {
            assert_eq!(
                batch_chunks[i], streaming_chunks[i],
                "Chunk {i} data differs"
            );
        }
    }

    // T-CHUNK-7: content_hash in manifest matches hash_file() output.
    #[test]
    fn test_content_hash_matches_file() {
        let content: Vec<u8> = (42u8..=200).cycle().take(8192).collect();
        let f = make_temp_file(&content);

        let (_chunks, manifest) = chunk_file(f.path(), 1024).unwrap();
        let file_hash = hash_file(f.path()).unwrap();

        assert_eq!(
            manifest.content_hash, file_hash,
            "Manifest content_hash must match hash_file()"
        );
    }

    // T-CHUNK-8: exactly chunk_size bytes => 1 chunk, no padding needed.
    #[test]
    fn test_exact_chunk_boundary() {
        let chunk_size = 512;
        let content = vec![0xFFu8; chunk_size];
        let f = make_temp_file(&content);

        let (chunks, manifest) = chunk_file(f.path(), chunk_size).unwrap();

        assert_eq!(manifest.total_chunks, 1);
        assert_eq!(manifest.chunks[0].size, chunk_size as u32);
        assert_eq!(chunks[0].len(), chunk_size);
    }
}
