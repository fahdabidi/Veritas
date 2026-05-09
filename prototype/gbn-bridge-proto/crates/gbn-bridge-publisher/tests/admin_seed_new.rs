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
        AdminNodeMetadata, AdminState, CreatorDhtEntryResponse, CreatorDhtEntrySignRequest,
        HostJoinRelayBody, SeedNewCreatorRequest, SeedNewCreatorResponse,
    },
    api::{AuthorityApiRequest, AuthorityApiRequestUnsigned, AuthorityRoute},
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
        "veritas-admin-seed-new-{name}-{}-{}",
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
        encryption_pub_key: None,
        entry_expiry_ms: now_ms + 60_000,
    }
}

fn bridge_dht_entry(bridge_id: &str, now_ms: u64) -> BridgeDhtEntry {
    BridgeDhtEntry::sign(
        BridgeDhtEntryUnsigned {
            bridge_id: bridge_id.to_string(),
            identity_pub: public_key(44),
            ingress_endpoints: vec![DhtBridgeIngressEndpoint::direct("127.0.0.1", 4443)],
            udp_punch_port: 4443,
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
    with_config: bool,
) -> AdminHttpServerHandle {
    let metadata = creator_metadata(actor_id, seed);
    let state = if with_config {
        AdminState::creator_with_config(
            metadata,
            store,
            creator_config(actor_id, seed, authority_url),
        )
    } else {
        AdminState::creator(metadata, store)
    };
    AdminHttpServer::bind("127.0.0.1:0".parse().unwrap(), state, 1_048_576)
        .unwrap()
        .spawn()
        .unwrap()
}

fn seed_host_store(actor_id: &str, authority_url: String) -> (PathBuf, LocalDhtStore) {
    let dir = unique_test_dir(actor_id);
    let path = dir.join("local_dht.json");
    let now = now_ms();
    let mut table = LocalDiscoveryTable::empty(actor_id, now);
    let publisher = publisher_entry(authority_url, now);
    let bridge = bridge_dht_entry("exit-bridge-a", now);
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
    let store = LocalDhtStore::start(actor_id, &path, table);
    (dir, store)
}

fn empty_store(actor_id: &str) -> (PathBuf, LocalDhtStore) {
    let dir = unique_test_dir(actor_id);
    let path = dir.join("local_dht.json");
    std::fs::create_dir_all(&dir).unwrap();
    (
        dir,
        LocalDhtStore::start(
            actor_id,
            &path,
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
    fn start() -> Self {
        let mut authority = PublisherAuthority::new(signing_key(9));
        let now = now_ms();
        authority
            .register_bridge(
                bridge_register("exit-bridge-a", 44, 4443),
                ReachabilityClass::Direct,
                now,
            )
            .unwrap();
        authority
            .register_bridge(
                bridge_register("exit-bridge-b", 45, 4444),
                ReachabilityClass::Direct,
                now,
            )
            .unwrap();
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

fn seed_request(host_entry: CreatorDhtEntry, start_bootstrap: bool) -> SeedNewCreatorRequest {
    SeedNewCreatorRequest {
        new_creator_id: "creator-new".to_string(),
        host_creator_entry: host_entry,
        start_bootstrap,
        force: false,
        host_admin_url: None,
    }
}

#[test]
fn authority_admin_signs_creator_dht_entry() {
    let server = AuthorityServer::new(PublisherAuthority::new(signing_key(9)), {
        let mut config = PublisherServiceConfig::default();
        config.bind_addr = "127.0.0.1:0".to_string();
        config
    });
    let admin = AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::authority(server.service_handle()),
        1_048_576,
    )
    .unwrap()
    .spawn()
    .unwrap();
    let request = CreatorDhtEntrySignRequest {
        creator: CreatorDhtEntryUnsigned {
            node_id: "creator-host".to_string(),
            ip_addr: "127.0.0.1".to_string(),
            pub_key: public_key(10),
            udp_punch_port: 4443,
            entry_expiry_ms: now_ms() + 60_000,
        },
        active: true,
    };
    let body = serde_json::to_string(&request).unwrap();
    let (status, response): (u16, CreatorDhtEntryResponse) = post_json(
        admin.local_addr(),
        AuthorityRoute::AdminCreatorDhtEntry.path(),
        &body,
    );
    assert_eq!(status, 200);
    response
        .creator
        .verify_authority(&public_key(9), now_ms())
        .expect("signed creator entry should validate");
    admin.join().unwrap();
}

#[test]
fn valid_seed_without_bootstrap_stores_new_creator_seeded_state() {
    let (dir, store) = empty_store("creator-new");
    let admin = creator_admin_server(
        "creator-new",
        20,
        "http://127.0.0.1:1".to_string(),
        store,
        false,
    );
    let request = seed_request(creator_dht_entry("creator-host", 10, now_ms()), false);
    let body = serde_json::to_string(&request).unwrap();
    let (status, response): (u16, SeedNewCreatorResponse) = post_json(
        admin.local_addr(),
        AuthorityRoute::AdminSeedNewCreator.path(),
        &body,
    );
    assert_eq!(status, 200);
    assert_eq!(
        response.self_onboarding_state,
        SelfOnboardingState::NewCreatorSeeded
    );
    assert!(!response.started_bootstrap);

    let (status, table): (u16, LocalDiscoveryTable) =
        get_json(admin.local_addr(), AuthorityRoute::AdminLocalDht.path());
    assert_eq!(status, 200);
    assert_eq!(
        table.self_onboarding_state,
        SelfOnboardingState::NewCreatorSeeded
    );
    assert_eq!(
        table
            .new_creator_seed_state
            .as_ref()
            .map(|seed| seed.host_creator_entry.node_id.as_str()),
        Some("creator-host")
    );

    admin.join().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn valid_seed_with_bootstrap_relays_join_through_host_and_records_session() {
    let authority = AuthorityHarness::start();
    let (host_dir, host_store) = seed_host_store("creator-host", authority.url.clone());
    let host_admin =
        creator_admin_server("creator-host", 10, authority.url.clone(), host_store, true);
    let (new_dir, new_store) = empty_store("creator-new");
    let new_admin = creator_admin_server("creator-new", 20, authority.url.clone(), new_store, true);
    let mut request = seed_request(creator_dht_entry("creator-host", 10, now_ms()), true);
    request.host_admin_url = Some(format!("http://{}", host_admin.local_addr()));
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
    let bootstrap_session_id = response
        .bootstrap_session_id
        .clone()
        .expect("bootstrap session id should be returned");

    let (status, table): (u16, LocalDiscoveryTable) =
        get_json(new_admin.local_addr(), AuthorityRoute::AdminLocalDht.path());
    assert_eq!(status, 200);
    assert_eq!(table.self_onboarding_state, SelfOnboardingState::Onboarded);
    assert_eq!(
        table
            .current_bootstrap_session
            .as_ref()
            .map(|session| session.session_id.as_str()),
        Some(bootstrap_session_id.as_str())
    );
    assert_eq!(
        table
            .current_bootstrap_session
            .as_ref()
            .map(|session| session.last_state.as_str()),
        Some("onboarded")
    );
    assert_eq!(
        table
            .creator_entry
            .as_ref()
            .map(|entry| entry.node_id.as_str()),
        Some("creator-new")
    );
    assert_eq!(table.bridge_entries.len(), 2);
    assert!(table
        .bridge_entries
        .iter()
        .any(|entry| entry.bridge_id == "exit-bridge-a"));
    assert!(table
        .bridge_entries
        .iter()
        .any(|entry| entry.bridge_id == "exit-bridge-b"));
    assert!(table.bridge_entries.iter().all(|entry| entry.active));
    assert_eq!(table.active_tunnels.len(), 2);
    assert!(table
        .active_tunnels
        .iter()
        .any(|tunnel| tunnel.peer_id == "exit-bridge-a"));
    assert!(table
        .active_tunnels
        .iter()
        .any(|tunnel| tunnel.peer_id == "exit-bridge-b"));

    let service = authority.service.lock().unwrap();
    let session = service
        .publisher_authority()
        .bootstrap_session(&bootstrap_session_id)
        .expect("publisher should record bootstrap session");
    assert_eq!(session.creator_entry.node_id, "creator-new");
    assert_eq!(session.host_creator_id, "creator-host");
    assert_eq!(session.relay_bridge_id, "exit-bridge-a");
    assert_eq!(session.seed_bridge_id, "exit-bridge-b");
    assert_eq!(
        session.bridge_ids,
        vec!["exit-bridge-a".to_string(), "exit-bridge-b".to_string()]
    );
    assert_eq!(session.bridge_set.bridge_dht_entries.len(), 2);
    assert_eq!(
        session.state,
        gbn_bridge_publisher::storage::BootstrapSessionState::Completed
    );
    assert!(session.completed_at_ms.is_some());
    assert!(session
        .progress_events
        .iter()
        .any(|event| event.reporter_id == "exit-bridge-b"
            && event.stage == BootstrapProgressStage::SeedTunnelEstablished));
    assert!(session
        .progress_events
        .iter()
        .any(|event| event.reporter_id == "creator-new"
            && event.stage == BootstrapProgressStage::BridgeSetComplete));
    assert_ne!(session.creator_entry.node_id, session.host_creator_id);
    assert_ne!(session.creator_entry.node_id, session.relay_bridge_id);
    assert_ne!(session.host_creator_id, session.relay_bridge_id);
    assert_eq!(session.chain_id, response.chain_id);
    drop(service);

    new_admin.join().unwrap();
    host_admin.join().unwrap();
    authority.join();
    let _ = std::fs::remove_dir_all(host_dir);
    let _ = std::fs::remove_dir_all(new_dir);
}

#[test]
fn seed_new_validation_rejects_mismatch_expiry_bad_signature_and_conflict() {
    let (dir, store) = empty_store("creator-new");
    let admin = creator_admin_server(
        "creator-new",
        20,
        "http://127.0.0.1:1".to_string(),
        store.clone(),
        false,
    );

    let mut mismatch = seed_request(creator_dht_entry("creator-host", 10, now_ms()), false);
    mismatch.new_creator_id = "other-creator".to_string();
    let body = serde_json::to_string(&mismatch).unwrap();
    let (status, error): (u16, AdminErrorResponse) = post_json(
        admin.local_addr(),
        AuthorityRoute::AdminSeedNewCreator.path(),
        &body,
    );
    assert_eq!(status, 409);
    assert_eq!(error.error.code, "new_creator_id_mismatch");

    let expired = seed_request(creator_dht_entry("creator-host", 10, 1), false);
    let body = serde_json::to_string(&expired).unwrap();
    let (status, error): (u16, AdminErrorResponse) = post_json(
        admin.local_addr(),
        AuthorityRoute::AdminSeedNewCreator.path(),
        &body,
    );
    assert_eq!(status, 409);
    assert_eq!(error.error.code, "host_creator_expired");

    let mut bad_sig = seed_request(creator_dht_entry("creator-host", 10, now_ms()), false);
    bad_sig.host_creator_entry.ip_addr = "127.0.0.2".to_string();
    let body = serde_json::to_string(&bad_sig).unwrap();
    let (status, error): (u16, AdminErrorResponse) = post_json(
        admin.local_addr(),
        AuthorityRoute::AdminSeedNewCreator.path(),
        &body,
    );
    assert_eq!(status, 409);
    assert_eq!(error.error.code, "host_creator_signature_invalid");

    let first = seed_request(creator_dht_entry("creator-host", 10, now_ms()), false);
    let body = serde_json::to_string(&first).unwrap();
    let (status, response): (u16, SeedNewCreatorResponse) = post_json(
        admin.local_addr(),
        AuthorityRoute::AdminSeedNewCreator.path(),
        &body,
    );
    assert_eq!(status, 200);

    let (status, replay): (u16, SeedNewCreatorResponse) = post_json(
        admin.local_addr(),
        AuthorityRoute::AdminSeedNewCreator.path(),
        &body,
    );
    assert_eq!(status, 200);
    assert_eq!(replay.chain_id, response.chain_id);
    assert!(replay.idempotent);

    let mut conflicting = seed_request(creator_dht_entry("creator-host-2", 11, now_ms()), false);
    let body = serde_json::to_string(&conflicting).unwrap();
    let (status, error): (u16, AdminErrorResponse) = post_json(
        admin.local_addr(),
        AuthorityRoute::AdminSeedNewCreator.path(),
        &body,
    );
    assert_eq!(status, 409);
    assert_eq!(error.error.code, "seed_already_present");

    std::thread::sleep(Duration::from_millis(2));
    conflicting.force = true;
    let body = serde_json::to_string(&conflicting).unwrap();
    let (status, forced): (u16, SeedNewCreatorResponse) = post_json(
        admin.local_addr(),
        AuthorityRoute::AdminSeedNewCreator.path(),
        &body,
    );
    assert_eq!(status, 200);
    assert!(forced.forced);
    assert_ne!(forced.chain_id, response.chain_id);

    admin.join().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bootstrapping_state_rejects_new_payload_without_force() {
    let dir = unique_test_dir("creator-new-bootstrapping");
    let path = dir.join("local_dht.json");
    std::fs::create_dir_all(&dir).unwrap();
    let mut table = LocalDiscoveryTable::empty("creator-new", now_ms());
    table.self_onboarding_state = SelfOnboardingState::Bootstrapping;
    let store = LocalDhtStore::start("creator-new", &path, table);
    let admin = creator_admin_server(
        "creator-new",
        20,
        "http://127.0.0.1:1".to_string(),
        store,
        false,
    );
    let request = seed_request(creator_dht_entry("creator-host", 10, now_ms()), false);
    let body = serde_json::to_string(&request).unwrap();
    let (status, error): (u16, AdminErrorResponse) = post_json(
        admin.local_addr(),
        AuthorityRoute::AdminSeedNewCreator.path(),
        &body,
    );
    assert_eq!(status, 409);
    assert_eq!(error.error.code, "new_creator_already_seeded");

    admin.join().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unseeded_host_creator_rejects_join_request() {
    let authority_url = "http://127.0.0.1:1".to_string();
    let (host_dir, host_store) = empty_store("creator-host");
    let host_admin = creator_admin_server("creator-host", 10, authority_url, host_store, true);
    let now = now_ms();
    let relay = AuthorityApiRequest::sign(
        AuthorityApiRequestUnsigned {
            chain_id: format!("seed-new-creator-creator-new-{now}"),
            request_id: format!("join-{now}"),
            sent_at_ms: now,
            actor_id: "creator-new".to_string(),
            body: HostJoinRelayBody {
                host_creator_id: "creator-host".to_string(),
                creator: PendingCreator {
                    node_id: "creator-new".to_string(),
                    ip_addr: "127.0.0.1".to_string(),
                    pub_key: public_key(20),
                    udp_punch_port: 4443,
                },
                now_ms: now,
            },
        },
        &signing_key(20),
    )
    .unwrap();
    let body = serde_json::to_string(&relay).unwrap();
    let (status, error): (u16, AdminErrorResponse) = post_json(
        host_admin.local_addr(),
        AuthorityRoute::AdminHostJoinRelay.path(),
        &body,
    );
    assert_eq!(status, 409);
    assert_eq!(error.error.code, "host_creator_not_seeded");

    host_admin.join().unwrap();
    let _ = std::fs::remove_dir_all(host_dir);
}
