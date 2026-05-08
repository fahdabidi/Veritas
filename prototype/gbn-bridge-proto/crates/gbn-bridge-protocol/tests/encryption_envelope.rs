use gbn_bridge_protocol::{
    decrypt_from_creator, encrypt_for_publisher, EnvelopeKeyDerivation, PublicKeyBytes,
};
use x25519_dalek::{PublicKey, StaticSecret};

fn public_key(private: [u8; 32]) -> PublicKeyBytes {
    let secret = StaticSecret::from(private);
    PublicKeyBytes(PublicKey::from(&secret).as_bytes().to_vec())
}

#[test]
fn encrypt_for_publisher_decrypt_from_creator_round_trip_succeeds() {
    let publisher_private = [9_u8; 32];
    let publisher_public = public_key(publisher_private);
    let creator_private = [7_u8; 32];
    let session_id = [1_u8; 16];
    let plaintext = b"VERITAS-SMOKE-3-PLAINTEXT";

    let encrypted = encrypt_for_publisher(
        plaintext,
        &publisher_public,
        "publisher",
        session_id,
        0,
        1,
        creator_private,
    )
    .unwrap();
    assert_eq!(
        encrypted.key_derivation,
        EnvelopeKeyDerivation::PublisherX25519HkdfAes256GcmV1
    );
    assert_ne!(encrypted.ciphertext, plaintext);

    let decrypted = decrypt_from_creator(&encrypted, publisher_private).unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn encrypted_frame_aad_mismatch_fails_decryption() {
    let publisher_private = [9_u8; 32];
    let publisher_public = public_key(publisher_private);
    let mut encrypted = encrypt_for_publisher(
        b"payload",
        &publisher_public,
        "publisher",
        [1_u8; 16],
        0,
        1,
        [7_u8; 32],
    )
    .unwrap();
    encrypted.chunk_index = 1;

    assert!(decrypt_from_creator(&encrypted, publisher_private).is_err());
}

#[test]
fn encrypted_frame_plaintext_hash_mismatch_fails_decryption() {
    let publisher_private = [9_u8; 32];
    let publisher_public = public_key(publisher_private);
    let mut encrypted = encrypt_for_publisher(
        b"payload",
        &publisher_public,
        "publisher",
        [1_u8; 16],
        0,
        1,
        [7_u8; 32],
    )
    .unwrap();
    encrypted.plaintext_hash[0] ^= 0xff;

    assert!(decrypt_from_creator(&encrypted, publisher_private).is_err());
}

#[test]
fn bridge_key_cannot_decrypt_publisher_encrypted_frame() {
    let publisher_private = [9_u8; 32];
    let publisher_public = public_key(publisher_private);
    let bridge_private = [8_u8; 32];
    let encrypted = encrypt_for_publisher(
        b"payload",
        &publisher_public,
        "publisher",
        [1_u8; 16],
        0,
        1,
        [7_u8; 32],
    )
    .unwrap();

    assert!(decrypt_from_creator(&encrypted, bridge_private).is_err());
}
