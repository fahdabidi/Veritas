use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gbn_bridge_creator::{
    build_upload_session, build_upload_session_to_disk, get_upload_session, list_upload_sessions,
    BuildUploadSessionOptions, SanitizerFormatHint, UploadManifest, MANIFEST_CHUNK_INDEX,
};
use gbn_bridge_protocol::{
    decrypt_from_creator, LocalDiscoveryTable, PublicKeyBytes, PublisherDhtEntry,
    SelfOnboardingState,
};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn unique_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "veritas-upload-session-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ))
}

fn publisher_entry() -> (PublisherDhtEntry, [u8; 32]) {
    let private = [9_u8; 32];
    let public = PublicKey::from(&StaticSecret::from(private));
    (
        PublisherDhtEntry {
            node_id: "publisher".to_string(),
            authority_url: "http://publisher-authority:8080".to_string(),
            receiver_url: "http://publisher-receiver:8081".to_string(),
            pub_key: PublicKeyBytes(public.as_bytes().to_vec()),
            encryption_pub_key: None,
            entry_expiry_ms: now_ms() + 300_000,
        },
        private,
    )
}

fn onboarded_table(publisher: PublisherDhtEntry) -> LocalDiscoveryTable {
    let mut table = LocalDiscoveryTable::empty("new-creator", now_ms());
    table.self_onboarding_state = SelfOnboardingState::Onboarded;
    table.publisher_entry = Some(publisher);
    table
}

fn options(input: Vec<u8>, now: u64) -> BuildUploadSessionOptions {
    BuildUploadSessionOptions {
        chain_id: format!("build-upload-session-test-{now}"),
        actor_id: "new-creator".to_string(),
        plaintext: input,
        format_hint: SanitizerFormatHint::Opaque,
        chunk_size: 64 * 1024,
        sanitization_profile: "v3-default-no-visual-anon".to_string(),
        now_ms: now,
    }
}

#[test]
fn build_session_encrypts_manifest_and_four_chunks() {
    let (publisher, private) = publisher_entry();
    let input = vec![42_u8; 256 * 1024];
    let table = onboarded_table(publisher.clone());
    let result =
        build_upload_session(options(input.clone(), now_ms()), &publisher, &table).unwrap();

    assert_eq!(result.summary.total_chunks, 4);
    assert_eq!(result.summary.content_hash, Sha256::digest(&input).to_vec());
    assert_eq!(result.session.chunk_ciphertexts.len(), 4);
    assert_eq!(
        result.session.manifest_ciphertext.chunk_index,
        MANIFEST_CHUNK_INDEX
    );

    let manifest_plaintext =
        decrypt_from_creator(&result.session.manifest_ciphertext, private).unwrap();
    let manifest: UploadManifest = serde_json::from_slice(&manifest_plaintext).unwrap();
    assert_eq!(manifest.total_chunks, 4);
    assert_eq!(manifest.content_hash, Sha256::digest(&input).to_vec());

    let first_chunk = decrypt_from_creator(&result.session.chunk_ciphertexts[0], private).unwrap();
    assert_eq!(first_chunk, vec![42_u8; 64 * 1024]);
}

#[test]
fn aad_tampering_causes_decryption_failure() {
    let (publisher, private) = publisher_entry();
    let table = onboarded_table(publisher.clone());
    let result = build_upload_session(
        options(vec![1_u8; 128 * 1024], now_ms()),
        &publisher,
        &table,
    )
    .unwrap();
    let mut frame = result.session.chunk_ciphertexts[0].clone();
    frame.chunk_index += 1;
    assert!(decrypt_from_creator(&frame, private).is_err());
}

#[test]
fn session_persists_and_summaries_reload_without_ciphertext_bytes() {
    let (publisher, _) = publisher_entry();
    let table = onboarded_table(publisher.clone());
    let dir = unique_dir("persist");
    fs::create_dir_all(&dir).unwrap();

    let result = build_upload_session_to_disk(
        &dir,
        options(vec![5_u8; 96 * 1024], now_ms()),
        &publisher,
        &table,
    )
    .unwrap();
    let sessions = list_upload_sessions(&dir).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, result.summary.session_id);
    let summary = get_upload_session(&dir, &result.summary.session_id).unwrap();
    assert_eq!(summary.ciphertext_chunk_count, 2);
    assert_eq!(summary.total_bytes, 96 * 1024);
    assert!(dir
        .join("upload_sessions")
        .join(&summary.session_id)
        .join("chunks")
        .join("000000.bin")
        .exists());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn non_onboarded_creator_is_rejected() {
    let (publisher, _) = publisher_entry();
    let table = LocalDiscoveryTable::empty("new-creator", now_ms());
    let error =
        build_upload_session(options(vec![1_u8; 1024], now_ms()), &publisher, &table).unwrap_err();
    assert!(error.to_string().contains("not onboarded"));
}

#[test]
fn repeated_builds_with_same_input_create_unique_sessions() {
    let (publisher, _) = publisher_entry();
    let table = onboarded_table(publisher.clone());
    let now = now_ms();
    let input = vec![7_u8; 64 * 1024];
    let first = build_upload_session(options(input.clone(), now), &publisher, &table).unwrap();
    let second = build_upload_session(options(input, now), &publisher, &table).unwrap();

    assert_ne!(first.summary.session_id, second.summary.session_id);
    assert_eq!(first.summary.content_hash, second.summary.content_hash);
    assert_eq!(first.summary.total_chunks, second.summary.total_chunks);
}
