use gbn_bridge_creator::chunk;
use sha2::{Digest, Sha256};

#[test]
fn one_mebibyte_chunks_into_sixteen_full_chunks() {
    let input = vec![7_u8; 1024 * 1024];
    let chunked = chunk(&input, 64 * 1024).unwrap();
    assert_eq!(chunked.chunks.len(), 16);
    assert_eq!(chunked.chunks.last().unwrap().plaintext.len(), 64 * 1024);
    assert_eq!(chunked.content_hash, Sha256::digest(&input).to_vec());
}

#[test]
fn one_mebibyte_plus_one_has_one_byte_tail() {
    let input = vec![9_u8; 1024 * 1024 + 1];
    let chunked = chunk(&input, 64 * 1024).unwrap();
    assert_eq!(chunked.chunks.len(), 17);
    assert_eq!(chunked.chunks.last().unwrap().plaintext.len(), 1);
}

#[test]
fn empty_input_is_rejected() {
    assert!(chunk(&[], 64 * 1024).is_err());
}

#[test]
fn chunking_is_deterministic_and_hashes_each_plaintext() {
    let input = (0..200_000)
        .map(|idx| (idx % 251) as u8)
        .collect::<Vec<_>>();
    let left = chunk(&input, 8192).unwrap();
    let right = chunk(&input, 8192).unwrap();
    assert_eq!(left, right);
    for chunk in &left.chunks {
        assert_eq!(
            chunk.plaintext_hash,
            Sha256::digest(&chunk.plaintext).to_vec()
        );
    }
}
