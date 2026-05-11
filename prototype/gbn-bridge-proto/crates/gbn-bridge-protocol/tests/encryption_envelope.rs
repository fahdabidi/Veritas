use gbn_bridge_protocol::{
    decrypt_bootstrap_payload, decrypt_from_creator, encrypt_bootstrap_payload,
    encrypt_for_publisher, BootstrapPayloadKind, EnvelopeKeyDerivation, PublicKeyBytes,
};
use serde_json::json;
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

#[test]
fn encrypted_bootstrap_payload_round_trip_succeeds_for_new_creator_only() {
    let publisher_private = [9_u8; 32];
    let creator_private = [7_u8; 32];
    let creator_public = public_key(creator_private);
    let host_creator_private = [8_u8; 32];
    let exit_bridge_private = [11_u8; 32];
    let plaintext = json!({
        "chain_id": "bootstrap-chain",
        "bootstrap_session_id": "bootstrap-session",
        "seed_bridge_id": "exit-bridge-b"
    });

    let encrypted = encrypt_bootstrap_payload(
        BootstrapPayloadKind::CreatorBootstrap,
        "bootstrap-chain",
        "bootstrap-session",
        &plaintext,
        &creator_public,
        "creator-new",
        publisher_private,
    )
    .unwrap();

    assert_eq!(
        encrypted.payload_kind,
        BootstrapPayloadKind::CreatorBootstrap
    );
    assert_ne!(
        encrypted.ciphertext,
        serde_json::to_vec(&plaintext).unwrap()
    );

    let decrypted: serde_json::Value =
        decrypt_bootstrap_payload(&encrypted, creator_private).unwrap();
    assert_eq!(decrypted, plaintext);
    assert!(
        decrypt_bootstrap_payload::<serde_json::Value>(&encrypted, host_creator_private).is_err()
    );
    assert!(
        decrypt_bootstrap_payload::<serde_json::Value>(&encrypted, exit_bridge_private).is_err()
    );
}
