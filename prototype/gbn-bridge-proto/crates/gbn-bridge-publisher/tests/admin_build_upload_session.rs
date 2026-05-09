use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_creator::{LocalDhtStore, UploadSessionSummary};
use gbn_bridge_protocol::{
    publisher_identity, LocalDiscoveryTable, PublisherDhtEntry, SelfOnboardingState,
};
use gbn_bridge_publisher::admin::{
    AdminCreatorConfig, AdminErrorResponse, AdminHttpServer, AdminNodeMetadata, AdminState,
    BuildUploadSessionResponse, UploadSessionDeleteResponse, UploadSessionsResponse,
};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_dir(name: &str) -> PathBuf {
    let counter = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "veritas-admin-build-upload-{name}-{}-{}-{counter}",
        std::process::id(),
        now_ms()
    ))
}

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn start_creator_admin() -> (gbn_bridge_publisher::admin::AdminHttpServerHandle, PathBuf) {
    let state_dir = unique_dir("creator");
    std::fs::create_dir_all(&state_dir).unwrap();
    let publisher_key = publisher_identity(&signing_key(9));
    let publisher = PublisherDhtEntry {
        node_id: "publisher".to_string(),
        authority_url: "http://publisher-authority:8080".to_string(),
        receiver_url: "http://publisher-receiver:8081".to_string(),
        pub_key: publisher_key.clone(),
        entry_expiry_ms: now_ms() + 300_000,
    };
    let mut table = LocalDiscoveryTable::empty("new-creator", now_ms());
    table.self_onboarding_state = SelfOnboardingState::Onboarded;
    table.publisher_entry = Some(publisher);
    let store = LocalDhtStore::start("new-creator", state_dir.join("local_dht.json"), table);
    let creator_key = signing_key(4);
    let mut metadata = AdminNodeMetadata::from_env("creator-new", "creator")
        .with_public_key(&publisher_identity(&creator_key))
        .with_publisher_public_key(&publisher_key)
        .with_creator_transport("127.0.0.1", 4443);
    metadata.state_dir = Some(state_dir.to_string_lossy().into_owned());
    let admin = AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::creator_with_config(
            metadata,
            store,
            AdminCreatorConfig {
                actor_id: "new-creator".to_string(),
                signing_key: creator_key,
                publisher_pub: publisher_key,
                authority_url: "http://publisher-authority:8080".to_string(),
                creator_ip_addr: "127.0.0.1".to_string(),
                udp_punch_port: 4443,
                timeout: Duration::from_secs(5),
            },
        ),
        1_048_576,
    )
    .unwrap()
    .spawn()
    .unwrap();
    (admin, state_dir)
}

fn request_json<R>(addr: SocketAddr, method: &str, path: &str, body: &str) -> (u16, R)
where
    R: for<'de> serde::Deserialize<'de>,
{
    let mut stream = TcpStream::connect(addr).unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let header = std::str::from_utf8(&response[..header_end]).unwrap();
    let status = header
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse::<u16>()
        .unwrap();
    let body = serde_json::from_slice(&response[header_end + 4..]).unwrap();
    (status, body)
}

#[test]
fn build_upload_session_endpoint_persists_lists_gets_and_deletes() {
    let (handle, state_dir) = start_creator_admin();
    let body = r#"{
        "input_source":"synthetic",
        "synthetic_size_bytes":65536,
        "synthetic_marker":"VERITAS-SMOKE-4-PLAINTEXT",
        "chunk_size_bytes":8192,
        "sanitization_profile":"v3-default-no-visual-anon"
    }"#;
    let (status, built): (u16, BuildUploadSessionResponse) = request_json(
        handle.local_addr(),
        "POST",
        "/v1/admin/build-upload-session?chain_id=phase10-test-chain",
        body,
    );
    assert_eq!(status, 200);
    assert_eq!(built.chain_id, "phase10-test-chain");
    assert_eq!(built.manifest.total_chunks, 8);
    assert!(built.sanitization_report.synthetic_marker_zeroed);
    assert!(state_dir
        .join("upload_sessions")
        .join(&built.session_id)
        .join("manifest.json")
        .exists());

    let (status, listed): (u16, UploadSessionsResponse) =
        request_json(handle.local_addr(), "GET", "/v1/admin/upload-sessions", "");
    assert_eq!(status, 200);
    assert_eq!(listed.sessions.len(), 1);
    assert_eq!(listed.sessions[0].session_id, built.session_id);

    let path = format!("/v1/admin/upload-sessions/{}", built.session_id);
    let (status, summary): (u16, UploadSessionSummary) =
        request_json(handle.local_addr(), "GET", &path, "");
    assert_eq!(status, 200);
    assert_eq!(summary.total_chunks, 8);

    let (status, deleted): (u16, UploadSessionDeleteResponse) =
        request_json(handle.local_addr(), "DELETE", &path, "");
    assert_eq!(status, 200);
    assert!(deleted.deleted);
    assert!(!state_dir
        .join("upload_sessions")
        .join(&built.session_id)
        .exists());

    let _ = std::fs::remove_dir_all(state_dir);
}

#[test]
fn repeated_admin_builds_keep_content_hash_but_create_distinct_sessions() {
    let (handle, state_dir) = start_creator_admin();
    let body = r#"{
        "input_source":"synthetic",
        "synthetic_size_bytes":65536,
        "synthetic_marker":"VERITAS-SMOKE-4-PLAINTEXT",
        "chunk_size_bytes":8192,
        "sanitization_profile":"v3-default-no-visual-anon"
    }"#;
    let (status, first): (u16, BuildUploadSessionResponse) = request_json(
        handle.local_addr(),
        "POST",
        "/v1/admin/build-upload-session?chain_id=phase10-idempotent-a",
        body,
    );
    assert_eq!(status, 200);
    let (status, second): (u16, BuildUploadSessionResponse) = request_json(
        handle.local_addr(),
        "POST",
        "/v1/admin/build-upload-session?chain_id=phase10-idempotent-b",
        body,
    );
    assert_eq!(status, 200);
    assert_ne!(first.session_id, second.session_id);
    assert_eq!(first.manifest.content_hash, second.manifest.content_hash);
    assert_eq!(first.manifest.total_chunks, second.manifest.total_chunks);

    let _ = std::fs::remove_dir_all(state_dir);
}

#[test]
fn synthetic_input_over_one_mib_is_rejected() {
    let (handle, state_dir) = start_creator_admin();
    let body = r#"{
        "input_source":"synthetic",
        "synthetic_size_bytes":1048577,
        "synthetic_marker":"VERITAS-SMOKE-4-PLAINTEXT",
        "chunk_size_bytes":8192,
        "sanitization_profile":"v3-default-no-visual-anon"
    }"#;
    let (status, error): (u16, AdminErrorResponse) = request_json(
        handle.local_addr(),
        "POST",
        "/v1/admin/build-upload-session?chain_id=phase10-too-large",
        body,
    );
    assert_eq!(status, 400);
    assert_eq!(error.error.code, "synthetic_size_too_large");

    let _ = std::fs::remove_dir_all(state_dir);
}
