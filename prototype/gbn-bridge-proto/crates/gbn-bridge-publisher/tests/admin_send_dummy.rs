use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_creator::{
    CreatorBridgeRequest, CreatorBridgeResponse, DiscoveryProbeResult, LocalDhtStore,
    SendDummyResult,
};
use gbn_bridge_protocol::{
    publisher_identity, BridgeCapability, BridgeDhtEntry, BridgeDhtEntryUnsigned,
    BridgeIngressEndpoint, BridgeRegister, DhtBridgeIngressEndpoint, EncryptedFrame,
    LocalDiscoveryTable, PublicKeyBytes, PublisherDhtEntry, ReachabilityClass, SelfOnboardingState,
    TunnelPeerRole, TunnelState, DEFAULT_UDP_PUNCH_PORT,
};
use gbn_bridge_publisher::{
    admin::{AdminCreatorConfig, AdminHttpServer, AdminState, FramesResponse, MetricsResponse},
    api::AuthorityRoute,
    AuthorityServer, PublisherAuthority, PublisherServiceConfig,
};

fn publisher_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[93_u8; 32])
}

fn actor_signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn node_public_key(seed: u8) -> PublicKeyBytes {
    publisher_identity(&actor_signing_key(seed))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn bridge_register(bridge_id: &str, key_seed: u8, ingress: SocketAddr) -> BridgeRegister {
    BridgeRegister {
        bridge_id: bridge_id.into(),
        identity_pub: node_public_key(key_seed),
        ingress_endpoints: vec![BridgeIngressEndpoint {
            host: ingress.ip().to_string(),
            port: ingress.port(),
        }],
        requested_udp_punch_port: ingress.port(),
        capabilities: vec![
            BridgeCapability::BootstrapSeed,
            BridgeCapability::CatalogRefresh,
            BridgeCapability::SessionRelay,
            BridgeCapability::BatchAssignment,
            BridgeCapability::ProgressReporting,
        ],
    }
}

fn bridge_dht_entry(
    publisher_key: &SigningKey,
    bridge_id: &str,
    key_seed: u8,
    ingress: SocketAddr,
    now_ms: u64,
) -> BridgeDhtEntry {
    BridgeDhtEntry::sign(
        BridgeDhtEntryUnsigned {
            bridge_id: bridge_id.into(),
            identity_pub: node_public_key(key_seed),
            ingress_endpoints: vec![DhtBridgeIngressEndpoint::direct(
                ingress.ip().to_string(),
                ingress.port(),
            )],
            udp_punch_port: ingress.port(),
            reachability_class: ReachabilityClass::Direct,
            lease_expiry_ms: now_ms + 300_000,
            entry_expiry_ms: now_ms + 300_000,
            capabilities: vec!["session_relay".into()],
        },
        publisher_key,
        true,
    )
    .unwrap()
}

struct TestTopology {
    authority: gbn_bridge_publisher::AuthorityServerHandle,
    admin: gbn_bridge_publisher::admin::AdminHttpServerHandle,
    service: Arc<Mutex<gbn_bridge_publisher::AuthorityService>>,
    fake_bridges: Vec<FakeBridgeHandle>,
    dummy_bridge_addr: SocketAddr,
    local_dht: Option<LocalDhtStore>,
}

impl TestTopology {
    fn shutdown(self) {
        self.admin.join().unwrap();
        self.authority.join().unwrap();
        for bridge in self.fake_bridges {
            bridge.join();
        }
    }
}

struct FakeBridgeHandle {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    join: thread::JoinHandle<()>,
}

impl FakeBridgeHandle {
    fn addr(&self) -> SocketAddr {
        self.addr
    }

    fn join(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .send_to(&[], self.addr);
        self.join.join().unwrap();
    }
}

fn start_fake_bridge(
    bridge_id: impl Into<String>,
    service: Arc<Mutex<gbn_bridge_publisher::AuthorityService>>,
) -> FakeBridgeHandle {
    let bridge_id = bridge_id.into();
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let addr = socket.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = stop.clone();
    let join = thread::spawn(move || {
        let mut buffer = vec![0_u8; 60 * 1024];
        while !stop_for_thread.load(Ordering::Relaxed) {
            let (read, peer) = match socket.recv_from(&mut buffer) {
                Ok(received) => received,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(_) => continue,
            };
            if read == 0 {
                continue;
            }
            let response = fake_bridge_response(&bridge_id, &service, &buffer[..read]);
            let payload = serde_json::to_vec(&response).unwrap();
            let _ = socket.send_to(&payload, peer);
        }
    });
    FakeBridgeHandle { addr, stop, join }
}

fn fake_bridge_response(
    bridge_id: &str,
    service: &Arc<Mutex<gbn_bridge_publisher::AuthorityService>>,
    payload: &[u8],
) -> CreatorBridgeResponse {
    let request = match serde_json::from_slice::<CreatorBridgeRequest>(payload) {
        Ok(request) => request,
        Err(error) => {
            return CreatorBridgeResponse::Error {
                message: error.to_string(),
            }
        }
    };
    let mut service = service.lock().unwrap();
    match request {
        CreatorBridgeRequest::Open(open) => {
            let chain_id = open.chain_id.clone();
            let session_id = open.session_id.clone();
            match service
                .publisher_authority_mut()
                .open_bridge_session_with_chain_id(Some(&chain_id), open)
            {
                Ok(()) => CreatorBridgeResponse::Opened {
                    chain_id,
                    session_id,
                },
                Err(error) => CreatorBridgeResponse::Error {
                    message: error.to_string(),
                },
            }
        }
        CreatorBridgeRequest::Frame(frame) => {
            let chain_id = frame.chain_id.clone();
            match service
                .publisher_authority_mut()
                .ingest_bridge_frame_with_chain_id(Some(&chain_id), bridge_id, frame, now_ms())
            {
                Ok(ack) => CreatorBridgeResponse::Ack(ack),
                Err(error) => CreatorBridgeResponse::Error {
                    message: error.to_string(),
                },
            }
        }
        CreatorBridgeRequest::FrameFragment(_) => CreatorBridgeResponse::Error {
            message: "fragmented upload frames are not used by send-dummy tests".to_string(),
        },
        CreatorBridgeRequest::Close(close) => {
            let chain_id = close.chain_id.clone();
            let session_id = close.session_id.clone();
            match service
                .publisher_authority_mut()
                .close_bridge_session_with_chain_id(Some(&chain_id), close)
            {
                Ok(()) => CreatorBridgeResponse::Closed {
                    chain_id,
                    session_id,
                },
                Err(error) => CreatorBridgeResponse::Error {
                    message: error.to_string(),
                },
            }
        }
    }
}

fn start_topology(state_kind: AdminStateKind) -> TestTopology {
    let publisher_key = publisher_signing_key();
    let publisher_pub = publisher_identity(&publisher_key);
    let authority = PublisherAuthority::new(publisher_key.clone());
    let mut config = PublisherServiceConfig::default();
    config.bind_addr = "127.0.0.1:0".into();
    let server = AuthorityServer::new(authority, config);
    let service = server.service_handle();
    let fake_bridge = start_fake_bridge("bridge-dummy", service.clone());
    let fake_bridge_extra = start_fake_bridge("bridge-extra", service.clone());
    let registered_at_ms = now_ms();
    service
        .lock()
        .unwrap()
        .publisher_authority_mut()
        .register_bridge(
            bridge_register("bridge-dummy", 54, fake_bridge.addr()),
            ReachabilityClass::Direct,
            registered_at_ms,
        )
        .unwrap();
    service
        .lock()
        .unwrap()
        .publisher_authority_mut()
        .register_bridge(
            bridge_register("bridge-extra", 55, fake_bridge_extra.addr()),
            ReachabilityClass::Direct,
            registered_at_ms,
        )
        .unwrap();
    let bound = server.bind().unwrap();
    let authority_url = format!("http://{}", bound.local_addr());
    let authority_handle = bound.spawn().unwrap();
    let creator = AdminCreatorConfig {
        actor_id: state_kind.actor_id().into(),
        signing_key: actor_signing_key(state_kind.creator_key_seed()),
        publisher_pub: publisher_pub.clone(),
        authority_url: authority_url.clone(),
        creator_ip_addr: "127.0.0.1".into(),
        udp_punch_port: DEFAULT_UDP_PUNCH_PORT,
        timeout: Duration::from_secs(5),
    };
    let mut local_dht = None;
    let admin_state = match state_kind {
        AdminStateKind::Authority => AdminState::authority_with_creator(service.clone(), creator),
        AdminStateKind::Receiver => AdminState::receiver_with_creator(
            Arc::new(Mutex::new(gbn_bridge_publisher::ReceiverMetrics::default())),
            creator,
        ),
        AdminStateKind::Bridge => AdminState::bridge_with_creator(
            Arc::new(Mutex::new(gbn_bridge_publisher::BridgeMetrics::default())),
            creator,
        ),
        AdminStateKind::CreatorOnboarded
        | AdminStateKind::CreatorNoEligibleBridge
        | AdminStateKind::CreatorEmpty => {
            let now = now_ms();
            let mut table = LocalDiscoveryTable::empty(state_kind.actor_id(), now);
            table.publisher_entry = Some(PublisherDhtEntry {
                node_id: "publisher".into(),
                authority_url: authority_url.clone(),
                receiver_url: authority_url.clone(),
                pub_key: publisher_pub.clone(),
                entry_expiry_ms: now + 300_000,
            });
            if matches!(
                state_kind,
                AdminStateKind::CreatorOnboarded | AdminStateKind::CreatorNoEligibleBridge
            ) {
                table.self_onboarding_state = SelfOnboardingState::Onboarded;
                let mut bridge_dummy =
                    bridge_dht_entry(&publisher_key, "bridge-dummy", 54, fake_bridge.addr(), now);
                let mut bridge_extra = bridge_dht_entry(
                    &publisher_key,
                    "bridge-extra",
                    55,
                    fake_bridge_extra.addr(),
                    now,
                );
                if matches!(state_kind, AdminStateKind::CreatorNoEligibleBridge) {
                    bridge_dummy.active = false;
                    bridge_extra.active = false;
                }
                table.bridge_entries = vec![bridge_dummy, bridge_extra];
                table.active_tunnels = vec![
                    TunnelState {
                        peer_id: "bridge-dummy".into(),
                        peer_role: TunnelPeerRole::ExitBridge,
                        established_at_ms: now - 10,
                        last_seen_ms: now,
                        bootstrap_session_id: Some("bootstrap-test".into()),
                    },
                    TunnelState {
                        peer_id: "bridge-extra".into(),
                        peer_role: TunnelPeerRole::ExitBridge,
                        established_at_ms: now - 20,
                        last_seen_ms: now - 5,
                        bootstrap_session_id: Some("bootstrap-test".into()),
                    },
                ];
            }
            let path = std::env::temp_dir().join(format!(
                "gbn-admin-send-dummy-{}-{now}.json",
                state_kind.actor_id()
            ));
            let store = LocalDhtStore::start(state_kind.actor_id(), path, table);
            local_dht = Some(store.clone());
            AdminState::creator_with_config(
                gbn_bridge_publisher::admin::AdminNodeMetadata::from_env(
                    state_kind.actor_id(),
                    "creator",
                )
                .with_public_key(&publisher_identity(&actor_signing_key(
                    state_kind.creator_key_seed(),
                )))
                .with_publisher_public_key(&publisher_pub)
                .with_authority_url(authority_url.clone()),
                store,
                creator,
            )
        }
    };
    let admin = AdminHttpServer::bind("127.0.0.1:0".parse().unwrap(), admin_state, 1_048_576)
        .unwrap()
        .spawn()
        .unwrap();

    TestTopology {
        authority: authority_handle,
        admin,
        service,
        dummy_bridge_addr: fake_bridge.addr(),
        fake_bridges: vec![fake_bridge, fake_bridge_extra],
        local_dht,
    }
}

#[derive(Debug, Clone, Copy)]
enum AdminStateKind {
    Authority,
    Receiver,
    Bridge,
    CreatorOnboarded,
    CreatorNoEligibleBridge,
    CreatorEmpty,
}

impl AdminStateKind {
    fn actor_id(self) -> &'static str {
        match self {
            Self::Authority => "publisher-authority",
            Self::Receiver => "publisher-receiver",
            Self::Bridge => "bridge-dummy",
            Self::CreatorOnboarded | Self::CreatorNoEligibleBridge | Self::CreatorEmpty => {
                "creator-new"
            }
        }
    }

    fn creator_key_seed(self) -> u8 {
        match self {
            Self::Authority => 93,
            Self::Receiver => 72,
            Self::Bridge => 54,
            Self::CreatorOnboarded | Self::CreatorNoEligibleBridge | Self::CreatorEmpty => 80,
        }
    }
}

fn post_json<R>(addr: SocketAddr, path: &str, body: &str) -> (u16, R)
where
    R: for<'de> serde::Deserialize<'de>,
{
    let mut stream = TcpStream::connect(addr).unwrap();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();

    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    parse_http_response(&response)
}

fn get_json<R>(addr: SocketAddr, path: &str) -> (u16, R)
where
    R: for<'de> serde::Deserialize<'de>,
{
    let mut stream = TcpStream::connect(addr).unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
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
fn send_dummy_from_authority_fails_creator_not_onboarded() {
    let topology = start_topology(AdminStateKind::Authority);

    let (status, error): (u16, gbn_bridge_publisher::admin::AdminErrorResponse) = post_json(
        topology.admin.local_addr(),
        "/v1/admin/send-dummy",
        r#"{"size":32}"#,
    );

    assert_eq!(status, 409);
    assert_eq!(error.error.code, "creator_not_onboarded");
    assert_eq!(error.error.current_state.as_deref(), Some("not_applicable"));

    topology.shutdown();
}

#[test]
fn discovery_probe_from_authority_returns_catalog_without_persisting_frames() {
    let topology = start_topology(AdminStateKind::Authority);

    let (status, result): (u16, DiscoveryProbeResult) = post_json(
        topology.admin.local_addr(),
        "/v1/admin/discovery-probe",
        r#"{}"#,
    );

    assert_eq!(status, 200);
    assert!(result.chain_id.starts_with("discovery-probe-"));
    assert_eq!(result.actor_id, "publisher-authority");
    assert_eq!(result.assigned_bridge_id, "bridge-dummy");
    assert_eq!(result.known_bridge_count, 2);
    assert_eq!(
        result.known_bridge_ids,
        vec!["bridge-dummy".to_string(), "bridge-extra".to_string()]
    );
    assert!(result
        .bridge_address
        .ends_with(&format!(":{}", topology.dummy_bridge_addr.port())));

    let path = format!("/v1/admin/frames?chain_id={}&limit=10", result.chain_id);
    let (status, frames): (u16, FramesResponse) = get_json(topology.admin.local_addr(), &path);
    assert_eq!(status, 200);
    assert!(frames.frames.is_empty());

    topology.shutdown();
}

#[test]
fn discovery_probe_from_receiver_returns_catalog() {
    let topology = start_topology(AdminStateKind::Receiver);

    let (status, result): (u16, DiscoveryProbeResult) = post_json(
        topology.admin.local_addr(),
        "/v1/admin/discovery-probe",
        r#"{}"#,
    );

    assert_eq!(status, 200);
    assert!(result.chain_id.starts_with("discovery-probe-"));
    assert_eq!(result.actor_id, "publisher-receiver");
    assert_eq!(result.assigned_bridge_id, "bridge-dummy");
    assert_eq!(
        result.known_bridge_ids,
        vec!["bridge-dummy".to_string(), "bridge-extra".to_string()]
    );

    topology.shutdown();
}

#[test]
fn send_dummy_from_receiver_fails_creator_not_onboarded() {
    let topology = start_topology(AdminStateKind::Receiver);

    let (status, error): (u16, gbn_bridge_publisher::admin::AdminErrorResponse) =
        post_json(topology.admin.local_addr(), "/v1/admin/send-dummy", r#"{}"#);

    assert_eq!(status, 409);
    assert_eq!(error.error.code, "creator_not_onboarded");
    let (status, metrics): (u16, MetricsResponse) = get_json(
        topology.admin.local_addr(),
        AuthorityRoute::AdminMetrics.path(),
    );
    assert_eq!(status, 200);
    assert!(matches!(metrics, MetricsResponse::Receiver(_)));

    topology.shutdown();
}

#[test]
fn send_dummy_from_bridge_fails_creator_not_onboarded() {
    let topology = start_topology(AdminStateKind::Bridge);

    let (status, error): (u16, gbn_bridge_publisher::admin::AdminErrorResponse) = post_json(
        topology.admin.local_addr(),
        "/v1/admin/send-dummy",
        r#"{"size":1}"#,
    );

    assert_eq!(status, 409);
    assert_eq!(error.error.code, "creator_not_onboarded");
    let (status, metrics): (u16, MetricsResponse) = get_json(
        topology.admin.local_addr(),
        AuthorityRoute::AdminMetrics.path(),
    );
    assert_eq!(status, 200);
    assert!(matches!(metrics, MetricsResponse::Bridge(_)));

    topology.shutdown();
}

#[test]
fn send_dummy_from_onboarded_creator_uses_local_dht_route_and_envelope() {
    let topology = start_topology(AdminStateKind::CreatorOnboarded);

    let (status, result): (u16, SendDummyResult) = post_json(
        topology.admin.local_addr(),
        "/v1/admin/send-dummy",
        r#"{"size":32}"#,
    );

    assert_eq!(status, 200);
    assert_eq!(result.actor_id, "creator-new");
    assert_eq!(result.route_source, "local_dht");
    assert_eq!(
        result.candidate_bridge_ids,
        vec!["bridge-dummy".to_string(), "bridge-extra".to_string()]
    );
    assert_eq!(result.selected_bridge_ids, vec!["bridge-dummy".to_string()]);
    assert_eq!(result.assigned_bridge_id, "bridge-dummy");
    assert_eq!(
        result.encryption_envelope,
        "publisher_x25519_hkdf_aes256gcm_v1"
    );
    assert!(result.ciphertext_only_at_bridge);
    assert_eq!(result.frames, 1);

    let frames = topology
        .service
        .lock()
        .unwrap()
        .publisher_authority()
        .list_frames(Some(&result.chain_id), 10);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].via_bridge_id, "bridge-dummy");
    let encrypted: EncryptedFrame = serde_json::from_slice(&frames[0].frame.ciphertext).unwrap();
    assert_eq!(encrypted.publisher_key_id, "publisher");
    assert!(!encrypted.ciphertext.is_empty());
    assert!(!encrypted.auth_tag.is_empty());

    topology.shutdown();
}

#[test]
fn send_dummy_from_onboarded_creator_preserves_operator_chain_id() {
    let topology = start_topology(AdminStateKind::CreatorOnboarded);
    let chain_id = "operator-send-dummy-chain";
    let path = format!("/v1/admin/send-dummy?chain_id={chain_id}");

    let (status, result): (u16, SendDummyResult) =
        post_json(topology.admin.local_addr(), &path, r#"{"size":32}"#);

    assert_eq!(status, 200);
    assert_eq!(result.chain_id, chain_id);
    assert_eq!(result.route_source, "local_dht");

    let frames = topology
        .service
        .lock()
        .unwrap()
        .publisher_authority()
        .list_frames(Some(chain_id), 10);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].chain_id.as_deref(), Some(chain_id));

    topology.shutdown();
}

#[test]
fn send_dummy_rejects_mismatched_query_and_body_chain_id() {
    let topology = start_topology(AdminStateKind::CreatorOnboarded);

    let (status, error): (u16, gbn_bridge_publisher::admin::AdminErrorResponse) = post_json(
        topology.admin.local_addr(),
        "/v1/admin/send-dummy?chain_id=query-chain",
        r#"{"size":32,"chain_id":"body-chain"}"#,
    );

    assert_eq!(status, 400);
    assert_eq!(error.error.code, "bad_query");
    assert!(error
        .error
        .message
        .contains("chain_id query parameter and request body chain_id must match"));

    topology.shutdown();
}

#[test]
fn send_dummy_force_bridge_failure_selects_second_local_dht_bridge() {
    let topology = start_topology(AdminStateKind::CreatorOnboarded);

    let (status, result): (u16, SendDummyResult) = post_json(
        topology.admin.local_addr(),
        "/v1/admin/send-dummy",
        r#"{"size":32,"force_bridge_failure":true}"#,
    );

    assert_eq!(status, 200);
    assert_eq!(result.route_source, "local_dht");
    assert_eq!(
        result.candidate_bridge_ids,
        vec!["bridge-dummy".to_string(), "bridge-extra".to_string()]
    );
    assert_eq!(result.selected_bridge_ids, vec!["bridge-extra".to_string()]);
    assert_eq!(result.assigned_bridge_id, "bridge-extra");
    assert!(result.force_bridge_failure_used);

    let table = topology.local_dht.as_ref().unwrap().snapshot();
    let failed = table
        .bridge_entries
        .iter()
        .find(|entry| entry.bridge_id == "bridge-dummy")
        .unwrap();
    assert!(failed
        .suspect_until_ms
        .is_some_and(|suspect| suspect > now_ms()));

    topology.shutdown();
}

#[test]
fn send_dummy_from_non_onboarded_creator_returns_current_state() {
    let topology = start_topology(AdminStateKind::CreatorEmpty);

    let (status, error): (u16, gbn_bridge_publisher::admin::AdminErrorResponse) =
        post_json(topology.admin.local_addr(), "/v1/admin/send-dummy", r#"{}"#);

    assert_eq!(status, 409);
    assert_eq!(error.error.code, "creator_not_onboarded");
    assert_eq!(error.error.current_state.as_deref(), Some("none"));

    topology.shutdown();
}

#[test]
fn send_dummy_from_onboarded_creator_without_eligible_bridge_returns_filter_drops() {
    let topology = start_topology(AdminStateKind::CreatorNoEligibleBridge);

    let (status, error): (u16, gbn_bridge_publisher::admin::AdminErrorResponse) =
        post_json(topology.admin.local_addr(), "/v1/admin/send-dummy", r#"{}"#);

    assert_eq!(status, 409);
    assert_eq!(error.error.code, "no_eligible_bridge");
    let drops = error.error.filter_drops.unwrap();
    assert_eq!(drops.inactive, 2);

    topology.shutdown();
}

#[test]
fn send_dummy_without_creator_config_returns_501() {
    let admin = AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::stub(),
        1_048_576,
    )
    .unwrap()
    .spawn()
    .unwrap();

    let (status, error): (u16, gbn_bridge_publisher::admin::AdminErrorResponse) =
        post_json(admin.local_addr(), "/v1/admin/send-dummy", r#"{}"#);

    assert_eq!(status, 501);
    assert_eq!(error.error.code, "not_supported");
    admin.join().unwrap();
}

#[test]
fn discovery_probe_without_creator_config_returns_501() {
    let admin = AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::stub(),
        1_048_576,
    )
    .unwrap()
    .spawn()
    .unwrap();

    let (status, error): (u16, gbn_bridge_publisher::admin::AdminErrorResponse) =
        post_json(admin.local_addr(), "/v1/admin/discovery-probe", r#"{}"#);

    assert_eq!(status, 501);
    assert_eq!(error.error.code, "not_supported");
    admin.join().unwrap();
}
