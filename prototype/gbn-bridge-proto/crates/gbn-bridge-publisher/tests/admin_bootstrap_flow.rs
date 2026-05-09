use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_creator::LocalDhtStore;
use gbn_bridge_protocol::{
    publisher_identity, BootstrapProgressStage, BridgeCapability, BridgeDhtEntry,
    BridgeDhtEntryUnsigned, BridgeIngressEndpoint, BridgeRegister, CreatorDhtEntry,
    CreatorDhtEntryUnsigned, DhtBridgeIngressEndpoint, HostCreatorSeedState, HostRoleState,
    LocalDiscoveryTable, PendingCreator, PublicKeyBytes, PublisherDhtEntry, ReachabilityClass,
    SelfOnboardingState,
};
use gbn_bridge_publisher::{
    admin::{
        AdminCreatorConfig, AdminErrorResponse, AdminHttpServer, AdminHttpServerHandle,
        AdminNodeMetadata, AdminState, InitializePublisherDhtResponse, SeedNewCreatorRequest,
        SeedNewCreatorResponse,
    },
    api::AuthorityRoute,
    storage::BootstrapSessionState,
    AuthorityError, AuthorityServer, PublisherAuthority, PublisherServiceConfig,
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
        "veritas-admin-bootstrap-flow-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ))
}

fn publisher_entry(authority_url: String, now_ms: u64) -> PublisherDhtEntry {
    PublisherDhtEntry {
        node_id: "publisher-authority".to_string(),
        authority_url,
        receiver_url: "http://publisher-receiver:8081".to_string(),
        pub_key: public_key(9),
        entry_expiry_ms: now_ms + 60_000,
    }
}

fn bridge_dht_entry(bridge_id: &str, seed: u8, port: u16, now_ms: u64) -> BridgeDhtEntry {
    BridgeDhtEntry::sign(
        BridgeDhtEntryUnsigned {
            bridge_id: bridge_id.to_string(),
            identity_pub: public_key(seed),
            ingress_endpoints: vec![DhtBridgeIngressEndpoint::direct("127.0.0.1", port)],
            udp_punch_port: port,
            reachability_class: ReachabilityClass::Direct,
            lease_expiry_ms: now_ms + 60_000,
            entry_expiry_ms: now_ms + 60_000,
            capabilities: vec!["bootstrap_seed".to_string(), "session_relay".to_string()],
        },
        &signing_key(9),
        true,
    )
    .expect("bridge entry should sign")
}

fn creator_dht_entry(node_id: &str, seed: u8, now_ms: u64) -> CreatorDhtEntry {
    CreatorDhtEntry::sign(
        CreatorDhtEntryUnsigned {
            node_id: node_id.to_string(),
            ip_addr: "127.0.0.1".to_string(),
            pub_key: public_key(seed),
            udp_punch_port: 4443,
            entry_expiry_ms: now_ms + 60_000,
        },
        &signing_key(9),
        true,
    )
    .expect("creator entry should sign")
}

fn bridge_register(bridge_id: &str, seed: u8, port: u16) -> BridgeRegister {
    BridgeRegister {
        bridge_id: bridge_id.to_string(),
        identity_pub: public_key(seed),
        ingress_endpoints: vec![BridgeIngressEndpoint {
            host: "127.0.0.1".to_string(),
            port,
        }],
        requested_udp_punch_port: port,
        capabilities: vec![
            BridgeCapability::BootstrapSeed,
            BridgeCapability::CatalogRefresh,
            BridgeCapability::SessionRelay,
            BridgeCapability::BatchAssignment,
            BridgeCapability::ProgressReporting,
        ],
    }
}

fn creator_config(actor_id: &str, seed: u8, authority_url: String) -> AdminCreatorConfig {
    AdminCreatorConfig {
        actor_id: actor_id.to_string(),
        signing_key: signing_key(seed),
        publisher_pub: public_key(9),
        authority_url,
        creator_ip_addr: "127.0.0.1".to_string(),
        udp_punch_port: 4443,
        timeout: Duration::from_secs(5),
    }
}

fn creator_metadata(actor_id: &str, seed: u8) -> AdminNodeMetadata {
    AdminNodeMetadata::from_env(actor_id, "creator")
        .with_public_key(&public_key(seed))
        .with_publisher_public_key(&public_key(9))
        .with_creator_transport("127.0.0.1", 4443)
}

fn creator_admin_server(
    actor_id: &str,
    seed: u8,
    authority_url: String,
    store: LocalDhtStore,
) -> AdminHttpServerHandle {
    AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::creator_with_config(
            creator_metadata(actor_id, seed),
            store,
            creator_config(actor_id, seed, authority_url),
        ),
        1_048_576,
    )
    .unwrap()
    .spawn()
    .unwrap()
}

fn seed_host_store(actor_id: &str, authority_url: String) -> (PathBuf, LocalDhtStore) {
    let dir = unique_test_dir(actor_id);
    let path = dir.join("local_dht.json");
    let now = now_ms();
    let publisher = publisher_entry(authority_url, now);
    let bridge = bridge_dht_entry("exit-bridge-a", 44, 4443, now);
    let mut table = LocalDiscoveryTable::empty(actor_id, now);
    table.self_onboarding_state = SelfOnboardingState::Onboarded;
    table.host_role_state = HostRoleState::HostSeeded;
    table.publisher_entry = Some(publisher.clone());
    table.bridge_entries = vec![bridge.clone()];
    table.host_seed_state = Some(HostCreatorSeedState {
        host_creator_actor_id: actor_id.to_string(),
        chain_id: format!("seed-host-{actor_id}-{now}"),
        publisher_entry: publisher,
        exit_bridge_a_entry: bridge,
        seeded_at_ms: now,
        bootstrap_genesis: true,
    });
    (dir, LocalDhtStore::start(actor_id, path, table))
}

fn empty_store(actor_id: &str) -> (PathBuf, LocalDhtStore) {
    let dir = unique_test_dir(actor_id);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("local_dht.json");
    (
        dir,
        LocalDhtStore::start(
            actor_id,
            path,
            LocalDiscoveryTable::empty(actor_id, now_ms()),
        ),
    )
}

struct AuthorityHarness {
    handle: gbn_bridge_publisher::AuthorityServerHandle,
    service: Arc<Mutex<gbn_bridge_publisher::AuthorityService>>,
    url: String,
}

impl AuthorityHarness {
    fn start_with_ten_bridges() -> Self {
        let mut authority = PublisherAuthority::new(signing_key(9));
        let now = now_ms();
        authority
            .register_bridge(
                bridge_register("exit-bridge-a", 44, 4443),
                ReachabilityClass::Direct,
                now,
            )
            .unwrap();
        for index in 1..=9 {
            authority
                .register_bridge(
                    bridge_register(
                        &format!("exit-bridge-{index:02}"),
                        44 + index as u8,
                        4443 + index as u16,
                    ),
                    ReachabilityClass::Direct,
                    now,
                )
                .unwrap();
        }

        let mut config = PublisherServiceConfig::default();
        config.bind_addr = "127.0.0.1:0".to_string();
        let server = AuthorityServer::new(authority, config);
        let service = server.service_handle();
        let bound = server.bind().unwrap();
        let url = format!("http://{}", bound.local_addr());
        let handle = bound.spawn().unwrap();
        Self {
            handle,
            service,
            url,
        }
    }

    fn join(self) {
        self.handle.join().unwrap();
    }
}

fn post_json<R>(addr: SocketAddr, path: &str, body: &str) -> (u16, R)
where
    R: for<'de> serde::Deserialize<'de>,
{
    request_json(addr, "POST", path, body)
}

fn get_json<R>(addr: SocketAddr, path: &str) -> (u16, R)
where
    R: for<'de> serde::Deserialize<'de>,
{
    request_json(addr, "GET", path, "")
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

fn seed_request(host_entry: CreatorDhtEntry, host_admin_url: String) -> SeedNewCreatorRequest {
    SeedNewCreatorRequest {
        new_creator_id: "creator-new".to_string(),
        host_creator_entry: host_entry,
        start_bootstrap: true,
        force: false,
        host_admin_url: Some(host_admin_url),
    }
}

#[test]
fn initialize_publisher_dht_admin_command_materializes_registered_exit_bridges() {
    let authority = AuthorityHarness::start_with_ten_bridges();
    let admin = AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::authority(authority.service.clone()),
        1_048_576,
    )
    .unwrap()
    .spawn()
    .unwrap();

    let chain_id = "phase4-publisher-dht-chain";
    let path = format!(
        "{}?chain_id={chain_id}",
        AuthorityRoute::AdminInitializePublisherDht.path()
    );
    let (status, response): (u16, InitializePublisherDhtResponse) =
        post_json(admin.local_addr(), &path, "{}");
    assert_eq!(status, 200);
    assert_eq!(response.chain_id, chain_id);
    assert_eq!(response.active_bridge_count, 10);
    assert_eq!(response.initialized_bridge_count, 10);
    assert_eq!(response.publisher_dht_entry_count, 10);
    assert_eq!(response.stale_entry_count, 0);
    assert!(response
        .bridge_ids
        .iter()
        .any(|bridge_id| bridge_id == "exit-bridge-a"));
    assert_eq!(
        authority
            .service
            .lock()
            .unwrap()
            .publisher_authority()
            .publisher_bridge_dht_entry_count(),
        10
    );

    admin.join().unwrap();
    authority.join();
}

#[test]
fn full_bootstrap_payload_populates_local_dht_and_records_progress() {
    let authority = AuthorityHarness::start_with_ten_bridges();
    assert_eq!(
        authority
            .service
            .lock()
            .unwrap()
            .publisher_authority()
            .publisher_bridge_dht_entry_count(),
        10
    );
    let (host_dir, host_store) = seed_host_store("creator-host", authority.url.clone());
    let host_admin = creator_admin_server("creator-host", 10, authority.url.clone(), host_store);
    let (new_dir, new_store) = empty_store("creator-new");
    let new_admin = creator_admin_server("creator-new", 20, authority.url.clone(), new_store);
    let request = seed_request(
        creator_dht_entry("creator-host", 10, now_ms()),
        format!("http://{}", host_admin.local_addr()),
    );
    let body = serde_json::to_string(&request).unwrap();

    let (status, response): (u16, SeedNewCreatorResponse) = post_json(
        new_admin.local_addr(),
        AuthorityRoute::AdminSeedNewCreator.path(),
        &body,
    );
    assert_eq!(status, 200);
    assert_eq!(
        response.self_onboarding_state,
        SelfOnboardingState::Onboarded
    );
    let bootstrap_session_id = response.bootstrap_session_id.unwrap();

    let (status, table): (u16, LocalDiscoveryTable) =
        get_json(new_admin.local_addr(), AuthorityRoute::AdminLocalDht.path());
    assert_eq!(status, 200);
    assert_eq!(table.self_onboarding_state, SelfOnboardingState::Onboarded);
    assert_eq!(
        table
            .creator_entry
            .as_ref()
            .map(|entry| entry.node_id.as_str()),
        Some("creator-new")
    );
    assert_eq!(table.bridge_entries.len(), 10);
    assert!(table.bridge_entries.iter().all(|entry| entry.active));
    assert!(table
        .bridge_entries
        .iter()
        .any(|entry| entry.bridge_id == "exit-bridge-a"));
    assert_eq!(table.active_tunnels.len(), 10);
    for entry in &table.bridge_entries {
        entry
            .verify_authority(&public_key(9), now_ms())
            .expect("stored bridge entry should remain signed by Publisher");
    }

    let service = authority.service.lock().unwrap();
    let session = service
        .publisher_authority()
        .bootstrap_session(&bootstrap_session_id)
        .expect("publisher should record bootstrap session");
    assert_eq!(session.state, BootstrapSessionState::Completed);
    assert_eq!(session.relay_bridge_id, "exit-bridge-a");
    assert_ne!(session.seed_bridge_id, session.relay_bridge_id);
    assert_eq!(session.bridge_ids.len(), 10);
    assert!(session
        .bridge_ids
        .iter()
        .any(|bridge_id| bridge_id == "exit-bridge-a"));
    assert_eq!(session.bridge_set.bridge_dht_entries.len(), 10);
    assert!(session
        .progress_events
        .iter()
        .any(|event| event.reporter_id == session.seed_bridge_id
            && event.stage == BootstrapProgressStage::SeedPayloadReceived));
    assert!(session
        .progress_events
        .iter()
        .any(|event| event.reporter_id == session.seed_bridge_id
            && event.stage == BootstrapProgressStage::SeedTunnelEstablished));
    assert!(session
        .progress_events
        .iter()
        .any(|event| event.reporter_id == "creator-new"
            && event.stage == BootstrapProgressStage::BridgeSetComplete));
    drop(service);

    new_admin.join().unwrap();
    host_admin.join().unwrap();
    authority.join();
    let _ = std::fs::remove_dir_all(host_dir);
    let _ = std::fs::remove_dir_all(new_dir);
}

#[test]
fn bootstrap_rejects_when_relay_bridge_is_the_only_direct_bridge() {
    let mut authority = PublisherAuthority::new(signing_key(9));
    let now = now_ms();
    authority
        .register_bridge(
            bridge_register("exit-bridge-a", 44, 4443),
            ReachabilityClass::Direct,
            now,
        )
        .unwrap();

    let error = authority
        .begin_bootstrap(
            gbn_bridge_protocol::CreatorJoinRequest {
                chain_id: "bootstrap-insufficient".to_string(),
                request_id: "join-001".to_string(),
                host_creator_id: "creator-host".to_string(),
                relay_bridge_id: "exit-bridge-a".to_string(),
                creator: PendingCreator {
                    node_id: "creator-new".to_string(),
                    ip_addr: "127.0.0.1".to_string(),
                    pub_key: public_key(20),
                    udp_punch_port: 4443,
                },
            },
            now + 1,
        )
        .unwrap_err();
    assert_eq!(
        error,
        AuthorityError::InsufficientBootstrapBridges {
            active_bridge_count: 1,
            relay_bridge_id: "exit-bridge-a".to_string()
        }
    );
}

#[test]
fn seed_new_surfaces_insufficient_bootstrap_bridge_rejection() {
    let mut authority = PublisherAuthority::new(signing_key(9));
    let now = now_ms();
    authority
        .register_bridge(
            bridge_register("exit-bridge-a", 44, 4443),
            ReachabilityClass::Direct,
            now,
        )
        .unwrap();
    let mut config = PublisherServiceConfig::default();
    config.bind_addr = "127.0.0.1:0".to_string();
    let server = AuthorityServer::new(authority, config);
    let bound = server.bind().unwrap();
    let authority_url = format!("http://{}", bound.local_addr());
    let authority_handle = bound.spawn().unwrap();

    let (host_dir, host_store) = seed_host_store("creator-host", authority_url.clone());
    let host_admin = creator_admin_server("creator-host", 10, authority_url.clone(), host_store);
    let (new_dir, new_store) = empty_store("creator-new");
    let new_admin = creator_admin_server("creator-new", 20, authority_url, new_store);
    let request = seed_request(
        creator_dht_entry("creator-host", 10, now_ms()),
        format!("http://{}", host_admin.local_addr()),
    );
    let body = serde_json::to_string(&request).unwrap();

    let (status, response): (u16, AdminErrorResponse) = post_json(
        new_admin.local_addr(),
        AuthorityRoute::AdminSeedNewCreator.path(),
        &body,
    );
    assert_eq!(status, 502);
    assert_eq!(response.error.code, "host_creator_join_failed");
    assert!(response
        .error
        .message
        .contains("bootstrap payload insufficient bridges"));

    let (status, table): (u16, LocalDiscoveryTable) =
        get_json(new_admin.local_addr(), AuthorityRoute::AdminLocalDht.path());
    assert_eq!(status, 200);
    assert_eq!(
        table.self_onboarding_state,
        SelfOnboardingState::FanoutFailed
    );
    assert!(table
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("bootstrap payload insufficient bridges")));

    new_admin.join().unwrap();
    host_admin.join().unwrap();
    authority_handle.join().unwrap();
    let _ = std::fs::remove_dir_all(host_dir);
    let _ = std::fs::remove_dir_all(new_dir);
}
