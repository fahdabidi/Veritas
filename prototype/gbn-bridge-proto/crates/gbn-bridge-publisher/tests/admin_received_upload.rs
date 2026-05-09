use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_creator::{
    build_upload_session, BuildUploadSessionOptions, SanitizerFormatHint, MANIFEST_CHUNK_INDEX,
};
use gbn_bridge_protocol::{
    publisher_encryption_identity, publisher_identity, BridgeData, BridgeOpen, LocalDiscoveryTable,
    PublisherDhtEntry, SelfOnboardingState,
};
use gbn_bridge_publisher::{
    admin::{AdminHttpServer, AdminState, ReceivedUploadSessionSummary},
    AuthorityService, PublisherAuthority, PublisherServiceConfig,
};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
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
fn received_upload_session_summary_decrypts_and_reassembles_chunks() {
    let publisher_key = signing_key(9);
    let publisher_pub = publisher_identity(&publisher_key);
    let publisher_encryption_pub = publisher_encryption_identity(&publisher_key);
    let now = now_ms();
    let publisher_entry = PublisherDhtEntry {
        node_id: "publisher".to_string(),
        authority_url: "http://publisher-authority:8080".to_string(),
        receiver_url: "http://publisher-receiver:8081".to_string(),
        pub_key: publisher_pub.clone(),
        encryption_pub_key: Some(publisher_encryption_pub),
        entry_expiry_ms: now + 300_000,
    };
    let mut table = LocalDiscoveryTable::empty("new-creator", now);
    table.self_onboarding_state = SelfOnboardingState::Onboarded;
    table.publisher_entry = Some(publisher_entry.clone());

    let mut plaintext = b"VERITAS-SMOKE-4-PLAINTEXT".to_vec();
    plaintext.extend(std::iter::repeat(7_u8).take(4096));
    let built = build_upload_session(
        BuildUploadSessionOptions {
            chain_id: "phase12-build".to_string(),
            actor_id: "new-creator".to_string(),
            plaintext,
            format_hint: SanitizerFormatHint::Synthetic,
            chunk_size: 1024,
            sanitization_profile: "v3-default-no-visual-anon".to_string(),
            now_ms: now,
        },
        &publisher_entry,
        &table,
    )
    .unwrap();

    let send_chain_id = "phase12-send";
    let mut authority = PublisherAuthority::new(publisher_key);
    authority
        .open_bridge_session_with_chain_id(
            Some(send_chain_id),
            BridgeOpen {
                chain_id: send_chain_id.to_string(),
                session_id: built.session.session_id.clone(),
                creator_id: "new-creator".to_string(),
                bridge_id: "exit-bridge-0".to_string(),
                creator_session_pub: built.session.manifest.creator_ephemeral_pubkey.clone(),
                opened_at_ms: now,
                expected_chunks: Some(built.session.manifest.total_chunks as u16),
            },
        )
        .unwrap();

    let mut frames = vec![(&built.session.manifest_ciphertext, MANIFEST_CHUNK_INDEX)];
    for (idx, frame) in built.session.chunk_ciphertexts.iter().enumerate() {
        frames.push((frame, idx as u32));
    }
    for (encrypted, sequence) in frames {
        let final_frame =
            sequence != MANIFEST_CHUNK_INDEX && sequence + 1 == built.session.manifest.total_chunks;
        authority
            .ingest_bridge_frame_with_chain_id(
                Some(send_chain_id),
                "exit-bridge-0",
                BridgeData {
                    chain_id: send_chain_id.to_string(),
                    session_id: built.session.session_id.clone(),
                    frame_id: if sequence == MANIFEST_CHUNK_INDEX {
                        format!("{}-manifest", built.session.session_id)
                    } else {
                        format!("{}-chunk-{sequence:06}", built.session.session_id)
                    },
                    sequence,
                    sent_at_ms: now
                        + if sequence == MANIFEST_CHUNK_INDEX {
                            0
                        } else {
                            1
                        },
                    ciphertext: serde_json::to_vec(encrypted).unwrap(),
                    final_frame,
                },
                now + 1,
            )
            .unwrap();
    }

    let service = AuthorityService::new(authority, &PublisherServiceConfig::default());
    let admin = AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::authority(Arc::new(Mutex::new(service))),
        1_048_576,
    )
    .unwrap()
    .spawn()
    .unwrap();

    let path = format!(
        "/v1/admin/received-upload-sessions/{}",
        built.session.session_id
    );
    let (status, summary): (u16, ReceivedUploadSessionSummary) =
        request_json(admin.local_addr(), "GET", &path, "");
    assert_eq!(status, 200);
    assert_eq!(summary.session_id, built.session.session_id);
    assert_eq!(
        summary.chunks_received,
        built.session.manifest.total_chunks as usize
    );
    assert!(summary.manifest_received);
    assert_eq!(summary.content_hash_match, Some(true));
    assert_eq!(summary.synthetic_marker_zeroed_at_start, Some(true));
    assert!(
        summary.decrypt_errors.is_empty(),
        "{:?}",
        summary.decrypt_errors
    );
}
