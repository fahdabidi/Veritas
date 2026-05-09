use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_creator::{LocalDhtMutation, LocalDhtStore};
use gbn_bridge_protocol::{
    publisher_identity, BridgeCapability, BridgeDhtEntry, BridgeDhtEntryUnsigned,
    BridgeIngressEndpoint, BridgeRegister, DhtBridgeIngressEndpoint, HostRoleState,
    LocalDiscoveryTable, PublicKeyBytes, PublisherDhtEntry, ReachabilityClass, SelfOnboardingState,
};
use gbn_bridge_publisher::{
    admin::{
        AdminErrorResponse, AdminHttpServer, AdminHttpServerHandle, AdminNodeMetadata, AdminState,
        BridgeDhtEntryResponse, SeedHostCreatorRequest, SeedHostCreatorResponse,
    },
    api::AuthorityRoute,
    AuthorityServer, PublisherAuthority, PublisherServiceConfig,
};

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn public_key(seed: u8) -> PublicKeyBytes {
    publisher_identity(&signing_key(seed))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_millis() as u64
}

fn unique_test_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "veritas-admin-seed-host-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ))
}

fn publisher_entry(publisher_pub: PublicKeyBytes, now_ms: u64) -> PublisherDhtEntry {
    PublisherDhtEntry {
        node_id: "publisher".to_string(),
        authority_url: "http://publisher-authority:8080".to_string(),
        receiver_url: "http://publisher-receiver:8081".to_string(),
        pub_key: publisher_pub,
        encryption_pub_key: None,
        entry_expiry_ms: now_ms + 60_000,
    }
}

fn bridge_entry(publisher: &SigningKey, now_ms: u64) -> BridgeDhtEntry {
    bridge_entry_with_id(publisher, "exit-bridge-a", now_ms)
}

fn bridge_entry_with_id(publisher: &SigningKey, bridge_id: &str, now_ms: u64) -> BridgeDhtEntry {
    bridge_entry_custom(
        publisher,
        bridge_id,
        ReachabilityClass::Direct,
        now_ms + 60_000,
        now_ms + 60_000,
    )
}

fn bridge_entry_custom(
    publisher: &SigningKey,
    bridge_id: &str,
    reachability_class: ReachabilityClass,
    lease_expiry_ms: u64,
    entry_expiry_ms: u64,
) -> BridgeDhtEntry {
    BridgeDhtEntry::sign(
        BridgeDhtEntryUnsigned {
            bridge_id: bridge_id.to_string(),
            identity_pub: public_key(22),
            ingress_endpoints: vec![DhtBridgeIngressEndpoint::direct("10.0.0.10", 4443)],
            udp_punch_port: 4443,
            reachability_class,
            lease_expiry_ms,
            entry_expiry_ms,
            capabilities: vec!["bootstrap_seed".to_string(), "session_relay".to_string()],
        },
        publisher,
        true,
    )
    .expect("bridge dht entry should sign")
}

fn seed_request(
    publisher: &SigningKey,
    host_creator_id: &str,
    now_ms: u64,
) -> SeedHostCreatorRequest {
    SeedHostCreatorRequest {
        host_creator_id: host_creator_id.to_string(),
        publisher_entry: publisher_entry(publisher_identity(publisher), now_ms),
        exit_bridge_a_entry: bridge_entry(publisher, now_ms),
        bootstrap_genesis: false,
        force: false,
    }
}

fn creator_admin_server(actor_id: &str, store: LocalDhtStore) -> AdminHttpServerHandle {
    let metadata = AdminNodeMetadata::from_env(actor_id, "creator")
        .with_public_key(&public_key(11))
        .with_publisher_public_key(&public_key(9))
        .with_creator_transport("10.0.0.20", 4443);
    AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::creator(metadata, store),
        1_048_576,
    )
    .unwrap()
    .spawn()
    .unwrap()
}

fn authority_admin_server_with_bridge(now_ms: u64) -> AdminHttpServerHandle {
    let mut authority = PublisherAuthority::new(signing_key(9));
    authority
        .register_bridge(
            BridgeRegister {
                bridge_id: "exit-bridge-a".to_string(),
                identity_pub: public_key(22),
                ingress_endpoints: vec![BridgeIngressEndpoint {
                    host: "10.0.0.10".to_string(),
                    port: 4443,
                }],
                requested_udp_punch_port: 4443,
                capabilities: vec![
                    BridgeCapability::BootstrapSeed,
                    BridgeCapability::SessionRelay,
                ],
            },
            ReachabilityClass::Direct,
            now_ms,
        )
        .expect("bridge should register");
    let server = AuthorityServer::new(authority, PublisherServiceConfig::default());
    AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::authority(server.service_handle()),
        1_048_576,
    )
    .unwrap()
    .spawn()
    .unwrap()
}

fn get_json<R>(addr: SocketAddr, path: &str) -> (u16, R)
where
    R: for<'de> serde::Deserialize<'de>,
{
    request_json(addr, "GET", path, "")
}

fn post_json<R>(addr: SocketAddr, path: &str, body: &str) -> (u16, R)
where
    R: for<'de> serde::Deserialize<'de>,
{
    request_json(addr, "POST", path, body)
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
    parse_http_response(&response)
}

fn parse_http_response<R>(response: &[u8]) -> (u16, R)
where
    R: for<'de> serde::Deserialize<'de>,
{
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
    let body = &response[header_end + 4..];
    (status, serde_json::from_slice(body).unwrap())
}

#[test]
fn authority_admin_signs_active_bridge_dht_entry() {
    let now = now_ms();
    let handle = authority_admin_server_with_bridge(now);
    let (status, response): (u16, BridgeDhtEntryResponse) = get_json(
        handle.local_addr(),
        "/v1/admin/bridges/exit-bridge-a/dht-entry",
    );
    assert_eq!(status, 200);
    assert_eq!(response.bridge.bridge_id, "exit-bridge-a");
    response
        .bridge
        .verify_authority(&public_key(9), now)
        .expect("authority-signed bridge DHT entry should validate");
    handle.join().unwrap();
}

#[test]
fn genesis_seed_stores_host_creator_state_and_idempotent_replay_keeps_chain() {
    let dir = unique_test_dir("genesis");
    let path = dir.join("local_dht.json");
    let store = LocalDhtStore::load_or_create("creator-host", &path, None, now_ms())
        .expect("store should start");
    let handle = creator_admin_server("creator-host", store.clone());
    let mut request = seed_request(&signing_key(9), "creator-host", now_ms());
    request.bootstrap_genesis = true;
    let body = serde_json::to_string(&request).unwrap();

    let (status, response): (u16, SeedHostCreatorResponse) = post_json(
        handle.local_addr(),
        AuthorityRoute::AdminSeedHostCreator.path(),
        &body,
    );
    assert_eq!(status, 200);
    assert_eq!(
        response.self_onboarding_state,
        SelfOnboardingState::Onboarded
    );
    assert_eq!(response.host_role_state, HostRoleState::HostSeeded);
    assert_eq!(response.seeded_bridge_id, "exit-bridge-a");
    assert!(response.genesis);
    assert!(!response.idempotent);

    let (status, replay): (u16, SeedHostCreatorResponse) = post_json(
        handle.local_addr(),
        AuthorityRoute::AdminSeedHostCreator.path(),
        &body,
    );
    assert_eq!(status, 200);
    assert_eq!(replay.chain_id, response.chain_id);
    assert!(replay.idempotent);

    let (status, table): (u16, LocalDiscoveryTable) =
        get_json(handle.local_addr(), AuthorityRoute::AdminLocalDht.path());
    assert_eq!(status, 200);
    assert_eq!(table.self_onboarding_state, SelfOnboardingState::Onboarded);
    assert_eq!(table.host_role_state, HostRoleState::HostSeeded);
    assert_eq!(table.bridge_entries.len(), 1);
    assert_eq!(
        table
            .host_seed_state
            .as_ref()
            .map(|seed| seed.chain_id.as_str()),
        Some(response.chain_id.as_str())
    );

    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn non_genesis_seed_requires_onboarded_creator() {
    let dir = unique_test_dir("not-onboarded");
    let path = dir.join("local_dht.json");
    let store = LocalDhtStore::load_or_create("creator-host", &path, None, now_ms())
        .expect("store should start");
    let handle = creator_admin_server("creator-host", store);
    let request = seed_request(&signing_key(9), "creator-host", now_ms());
    let body = serde_json::to_string(&request).unwrap();

    let (status, error): (u16, AdminErrorResponse) = post_json(
        handle.local_addr(),
        AuthorityRoute::AdminSeedHostCreator.path(),
        &body,
    );
    assert_eq!(status, 409);
    assert_eq!(error.error.code, "host_creator_not_onboarded");

    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn force_replaces_existing_seed_with_fresh_chain() {
    let dir = unique_test_dir("force");
    let path = dir.join("local_dht.json");
    let store = LocalDhtStore::load_or_create("creator-host", &path, None, now_ms())
        .expect("store should start");
    store
        .mutate(
            LocalDhtMutation::SetSelfOnboardingState(SelfOnboardingState::Onboarded),
            now_ms(),
        )
        .expect("state mutation should persist");
    let handle = creator_admin_server("creator-host", store);

    let first = seed_request(&signing_key(9), "creator-host", now_ms());
    let first_body = serde_json::to_string(&first).unwrap();
    let (status, first_response): (u16, SeedHostCreatorResponse) = post_json(
        handle.local_addr(),
        AuthorityRoute::AdminSeedHostCreator.path(),
        &first_body,
    );
    assert_eq!(status, 200);

    std::thread::sleep(std::time::Duration::from_millis(2));
    let mut second = seed_request(&signing_key(9), "creator-host", now_ms());
    second.exit_bridge_a_entry = bridge_entry_with_id(&signing_key(9), "exit-bridge-b", now_ms());
    second.force = true;
    let second_body = serde_json::to_string(&second).unwrap();
    let (status, second_response): (u16, SeedHostCreatorResponse) = post_json(
        handle.local_addr(),
        AuthorityRoute::AdminSeedHostCreator.path(),
        &second_body,
    );
    assert_eq!(status, 200);
    assert_ne!(second_response.chain_id, first_response.chain_id);
    assert_eq!(second_response.seeded_bridge_id, "exit-bridge-b");
    assert!(second_response.forced);

    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn seed_validation_rejects_trust_mismatch_and_tampered_bridge_entry() {
    let dir = unique_test_dir("validation");
    let path = dir.join("local_dht.json");
    let store = LocalDhtStore::load_or_create("creator-host", &path, None, now_ms())
        .expect("store should start");
    store
        .mutate(
            LocalDhtMutation::SetSelfOnboardingState(SelfOnboardingState::Onboarded),
            now_ms(),
        )
        .expect("state mutation should persist");
    let handle = creator_admin_server("creator-host", store);

    let mut trust_mismatch = seed_request(&signing_key(9), "creator-host", now_ms());
    trust_mismatch.publisher_entry.pub_key = public_key(7);
    let body = serde_json::to_string(&trust_mismatch).unwrap();
    let (status, error): (u16, AdminErrorResponse) = post_json(
        handle.local_addr(),
        AuthorityRoute::AdminSeedHostCreator.path(),
        &body,
    );
    assert_eq!(status, 409);
    assert_eq!(error.error.code, "publisher_trust_mismatch");

    let mut tampered = seed_request(&signing_key(9), "creator-host", now_ms());
    tampered
        .exit_bridge_a_entry
        .capabilities
        .push("tampered".to_string());
    let body = serde_json::to_string(&tampered).unwrap();
    let (status, error): (u16, AdminErrorResponse) = post_json(
        handle.local_addr(),
        AuthorityRoute::AdminSeedHostCreator.path(),
        &body,
    );
    assert_eq!(status, 409);
    assert_eq!(error.error.code, "bridge_signature_invalid");

    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn seed_validation_rejects_mismatch_expiry_relay_only_and_non_force_conflict() {
    let dir = unique_test_dir("more-validation");
    let path = dir.join("local_dht.json");
    let store = LocalDhtStore::load_or_create("creator-host", &path, None, now_ms())
        .expect("store should start");
    store
        .mutate(
            LocalDhtMutation::SetSelfOnboardingState(SelfOnboardingState::Onboarded),
            now_ms(),
        )
        .expect("state mutation should persist");
    let handle = creator_admin_server("creator-host", store);

    let mismatch = seed_request(&signing_key(9), "different-host", now_ms());
    let body = serde_json::to_string(&mismatch).unwrap();
    let (status, error): (u16, AdminErrorResponse) = post_json(
        handle.local_addr(),
        AuthorityRoute::AdminSeedHostCreator.path(),
        &body,
    );
    assert_eq!(status, 409);
    assert_eq!(error.error.code, "host_creator_id_mismatch");

    let mut expired = seed_request(&signing_key(9), "creator-host", now_ms());
    expired.exit_bridge_a_entry = bridge_entry_custom(
        &signing_key(9),
        "exit-bridge-a",
        ReachabilityClass::Direct,
        1,
        1,
    );
    let body = serde_json::to_string(&expired).unwrap();
    let (status, error): (u16, AdminErrorResponse) = post_json(
        handle.local_addr(),
        AuthorityRoute::AdminSeedHostCreator.path(),
        &body,
    );
    assert_eq!(status, 409);
    assert_eq!(error.error.code, "bridge_expired");

    let mut relay_only = seed_request(&signing_key(9), "creator-host", now_ms());
    relay_only.exit_bridge_a_entry = bridge_entry_custom(
        &signing_key(9),
        "exit-bridge-a",
        ReachabilityClass::RelayOnly,
        now_ms() + 60_000,
        now_ms() + 60_000,
    );
    let body = serde_json::to_string(&relay_only).unwrap();
    let (status, error): (u16, AdminErrorResponse) = post_json(
        handle.local_addr(),
        AuthorityRoute::AdminSeedHostCreator.path(),
        &body,
    );
    assert_eq!(status, 409);
    assert_eq!(error.error.code, "bridge_relay_only_ineligible");

    let first = seed_request(&signing_key(9), "creator-host", now_ms());
    let first_body = serde_json::to_string(&first).unwrap();
    let (status, _): (u16, SeedHostCreatorResponse) = post_json(
        handle.local_addr(),
        AuthorityRoute::AdminSeedHostCreator.path(),
        &first_body,
    );
    assert_eq!(status, 200);

    let mut conflicting = seed_request(&signing_key(9), "creator-host", now_ms());
    conflicting.exit_bridge_a_entry =
        bridge_entry_with_id(&signing_key(9), "exit-bridge-b", now_ms());
    let body = serde_json::to_string(&conflicting).unwrap();
    let (status, error): (u16, AdminErrorResponse) = post_json(
        handle.local_addr(),
        AuthorityRoute::AdminSeedHostCreator.path(),
        &body,
    );
    assert_eq!(status, 409);
    assert_eq!(error.error.code, "seed_already_present");

    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
