use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_creator::{LocalDhtMutation, LocalDhtStore, ResetCreatorStateResponse};
use gbn_bridge_protocol::{
    publisher_identity, LocalDiscoveryTable, ReachabilityClass, SelfOnboardingState,
};
use gbn_bridge_publisher::{
    admin::{
        AdminCreatorConfig, AdminErrorResponse, AdminHttpServer, AdminHttpServerHandle,
        AdminLocalDhtResponse, AdminNodeMetadata, AdminState,
    },
    api::AuthorityRoute,
    AuthorityServer, PublisherAuthority, PublisherServiceConfig,
};

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_millis() as u64
}

fn unique_test_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "veritas-admin-local-dht-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ))
}

fn authority_admin_server() -> AdminHttpServerHandle {
    let authority = PublisherAuthority::new(signing_key(9));
    let server = AuthorityServer::new(authority, PublisherServiceConfig::default());
    let service_handle = server.service_handle();
    AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::authority(service_handle),
        1_048_576,
    )
    .unwrap()
    .spawn()
    .unwrap()
}

fn receiver_admin_server() -> AdminHttpServerHandle {
    let metrics = Arc::new(Mutex::new(gbn_bridge_publisher::ReceiverMetrics::default()));
    AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::receiver(metrics),
        1_048_576,
    )
    .unwrap()
    .spawn()
    .unwrap()
}

fn bridge_admin_server() -> AdminHttpServerHandle {
    let metrics = Arc::new(Mutex::new(gbn_bridge_publisher::BridgeMetrics::default()));
    AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::bridge(metrics),
        1_048_576,
    )
    .unwrap()
    .spawn()
    .unwrap()
}

fn admin_creator_config(actor_id: &str) -> AdminCreatorConfig {
    let signing_key = signing_key(9);
    AdminCreatorConfig {
        actor_id: actor_id.to_string(),
        signing_key: signing_key.clone(),
        publisher_pub: publisher_identity(&signing_key),
        authority_url: "http://publisher-authority:8080".to_string(),
        creator_ip_addr: "10.0.0.10".to_string(),
        udp_punch_port: 4_443,
        timeout: Duration::from_secs(5),
    }
}

fn authority_admin_server_with_creator() -> AdminHttpServerHandle {
    let authority = PublisherAuthority::new(signing_key(9));
    let server = AuthorityServer::new(authority, PublisherServiceConfig::default());
    let service_handle = server.service_handle();
    AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::authority_with_creator(
            service_handle,
            admin_creator_config("publisher-authority"),
        ),
        1_048_576,
    )
    .unwrap()
    .spawn()
    .unwrap()
}

fn receiver_admin_server_with_creator() -> AdminHttpServerHandle {
    let metrics = Arc::new(Mutex::new(gbn_bridge_publisher::ReceiverMetrics::default()));
    AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::receiver_with_creator(metrics, admin_creator_config("publisher-receiver")),
        1_048_576,
    )
    .unwrap()
    .spawn()
    .unwrap()
}

fn bridge_admin_server_with_creator() -> AdminHttpServerHandle {
    let metrics = Arc::new(Mutex::new(gbn_bridge_publisher::BridgeMetrics::default()));
    AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::bridge_with_creator(metrics, admin_creator_config("exit-bridge-3")),
        1_048_576,
    )
    .unwrap()
    .spawn()
    .unwrap()
}

fn creator_admin_server(store: LocalDhtStore) -> AdminHttpServerHandle {
    let creator_key = signing_key(11);
    let publisher_key = signing_key(9);
    let metadata = AdminNodeMetadata::from_env("creator-new", "creator")
        .with_public_key(&publisher_identity(&creator_key))
        .with_publisher_public_key(&publisher_identity(&publisher_key))
        .with_creator_transport("10.0.0.20", 4_443);
    AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::creator(metadata, store),
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
fn node_metadata_route_returns_role_specific_seed_fields() {
    let authority = authority_admin_server_with_creator();
    let (status, metadata): (u16, AdminNodeMetadata) = get_json(
        authority.local_addr(),
        AuthorityRoute::AdminNodeMetadata.path(),
    );
    assert_eq!(status, 200);
    assert_eq!(metadata.node_id, "publisher-authority");
    assert_eq!(metadata.role, "publisher");
    assert_eq!(metadata.publisher_surface.as_deref(), Some("authority"));
    assert_eq!(
        metadata.authority_url.as_deref(),
        Some("http://publisher-authority:8080")
    );
    assert!(metadata.public_key.is_some());
    assert!(metadata.publisher_public_key.is_some());
    authority.join().unwrap();

    let receiver = receiver_admin_server_with_creator();
    let (status, metadata): (u16, AdminNodeMetadata) = get_json(
        receiver.local_addr(),
        AuthorityRoute::AdminNodeMetadata.path(),
    );
    assert_eq!(status, 200);
    assert_eq!(metadata.node_id, "publisher-receiver");
    assert_eq!(metadata.role, "publisher");
    assert_eq!(metadata.publisher_surface.as_deref(), Some("receiver"));
    assert!(metadata.public_key.is_some());
    assert!(metadata.publisher_public_key.is_some());
    receiver.join().unwrap();

    let bridge = bridge_admin_server_with_creator();
    let (status, metadata): (u16, AdminNodeMetadata) = get_json(
        bridge.local_addr(),
        AuthorityRoute::AdminNodeMetadata.path(),
    );
    assert_eq!(status, 200);
    assert_eq!(metadata.node_id, "exit-bridge-3");
    assert_eq!(metadata.role, "exit_bridge");
    assert_eq!(metadata.udp_punch_port, Some(4_443));
    assert_eq!(metadata.reachability_class, Some(ReachabilityClass::Direct));
    assert_eq!(
        metadata
            .ingress_endpoints
            .as_ref()
            .and_then(|endpoints| endpoints.first())
            .map(|endpoint| endpoint.ip_addr.as_str()),
        Some("10.0.0.10")
    );
    assert!(metadata
        .capabilities
        .as_ref()
        .is_some_and(|capabilities| capabilities.iter().any(|value| value == "session_relay")));
    assert!(metadata.public_key.is_some());
    assert!(metadata.publisher_public_key.is_some());
    bridge.join().unwrap();

    let dir = unique_test_dir("creator-metadata");
    let path = dir.join("local_dht.json");
    let store = LocalDhtStore::load_or_create("creator-new", &path, None, 1_000)
        .expect("store should start");
    let creator = creator_admin_server(store);
    let (status, metadata): (u16, AdminNodeMetadata) = get_json(
        creator.local_addr(),
        AuthorityRoute::AdminNodeMetadata.path(),
    );
    assert_eq!(status, 200);
    assert_eq!(metadata.node_id, "creator-new");
    assert_eq!(metadata.role, "creator");
    assert_eq!(metadata.ip_addr.as_deref(), Some("10.0.0.20"));
    assert_eq!(metadata.creator_udp_punch_port, Some(4_443));
    assert!(metadata.public_key.is_some());
    assert!(metadata.publisher_public_key.is_some());
    creator.join().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn non_creator_admin_surfaces_return_not_applicable_local_dht_and_reject_reset() {
    let cases = [
        (authority_admin_server(), "publisher", Some("authority")),
        (receiver_admin_server(), "publisher", Some("receiver")),
        (bridge_admin_server(), "exit_bridge", None),
    ];

    for (handle, role, surface) in cases {
        let (status, response): (u16, AdminLocalDhtResponse) =
            get_json(handle.local_addr(), AuthorityRoute::AdminLocalDht.path());
        assert_eq!(status, 200);
        assert_eq!(response.role, role);
        assert_eq!(response.state, "not_applicable");
        assert_eq!(response.publisher_surface.as_deref(), surface);

        let (status, error): (u16, AdminErrorResponse) = post_json(
            handle.local_addr(),
            AuthorityRoute::AdminResetCreatorState.path(),
            "{}",
        );
        assert_eq!(status, 405);
        assert_eq!(error.error.code, "method_not_allowed");

        handle.join().unwrap();
    }
}

#[test]
fn creator_admin_surface_returns_full_table_and_reset_clears_state() {
    let dir = unique_test_dir("creator-reset");
    let path = dir.join("local_dht.json");
    let store = LocalDhtStore::load_or_create("creator-new", &path, None, 1_000)
        .expect("store should start");
    store
        .mutate(
            LocalDhtMutation::SetSelfOnboardingState(SelfOnboardingState::FanoutPartial),
            1_500,
        )
        .expect("state mutation should persist");
    let handle = creator_admin_server(store.clone());

    let (status, table): (u16, LocalDiscoveryTable) =
        get_json(handle.local_addr(), AuthorityRoute::AdminLocalDht.path());
    assert_eq!(status, 200);
    assert_eq!(
        table.self_onboarding_state,
        SelfOnboardingState::FanoutPartial
    );

    let (status, reset): (u16, ResetCreatorStateResponse) = post_json(
        handle.local_addr(),
        AuthorityRoute::AdminResetCreatorState.path(),
        "{}",
    );
    assert_eq!(status, 200);
    assert_eq!(reset.actor_id, "creator-new");
    assert_eq!(
        reset.prior_self_onboarding_state,
        SelfOnboardingState::FanoutPartial
    );

    let (status, table): (u16, LocalDiscoveryTable) =
        get_json(handle.local_addr(), AuthorityRoute::AdminLocalDht.path());
    assert_eq!(status, 200);
    assert_eq!(table.self_onboarding_state, SelfOnboardingState::None);

    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
