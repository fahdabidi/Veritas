use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BridgeCapability, BridgeData, BridgeIngressEndpoint, BridgeOpen,
    BridgeRegister, PublicKeyBytes, ReachabilityClass,
};
use gbn_bridge_publisher::{
    admin::{
        AdminErrorResponse, AdminHttpServer, AdminState, BridgesResponse, FramesResponse,
        MetricsResponse,
    },
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

fn bridge_register(bridge_id: &str, key_seed: u8) -> BridgeRegister {
    BridgeRegister {
        bridge_id: bridge_id.into(),
        identity_pub: node_public_key(key_seed),
        ingress_endpoints: vec![BridgeIngressEndpoint {
            host: "198.51.100.10".into(),
            port: 443,
        }],
        requested_udp_punch_port: 443,
        capabilities: vec![
            BridgeCapability::BootstrapSeed,
            BridgeCapability::CatalogRefresh,
            BridgeCapability::SessionRelay,
            BridgeCapability::BatchAssignment,
            BridgeCapability::ProgressReporting,
        ],
    }
}

fn bridge_frame(chain_id: &str, session_id: &str, frame_id: &str, sequence: u32) -> BridgeData {
    BridgeData {
        chain_id: chain_id.into(),
        session_id: session_id.into(),
        frame_id: frame_id.into(),
        sequence,
        sent_at_ms: now_ms(),
        ciphertext: vec![1, 2, 3, sequence as u8],
        final_frame: true,
    }
}

fn authority_admin_server(
    authority: PublisherAuthority,
) -> gbn_bridge_publisher::admin::AdminHttpServerHandle {
    let server = AuthorityServer::new(authority, PublisherServiceConfig::default());
    let service_handle = server.service_handle();
    let admin = AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::authority(service_handle),
        1_048_576,
    )
    .unwrap();
    admin.spawn().unwrap()
}

fn stub_admin_server() -> gbn_bridge_publisher::admin::AdminHttpServerHandle {
    let admin = AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::stub(),
        1_048_576,
    )
    .unwrap();
    admin.spawn().unwrap()
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
fn admin_list_bridges_returns_registered_bridges() {
    let mut authority = PublisherAuthority::new(publisher_signing_key());
    authority
        .register_bridge(
            bridge_register("bridge-b", 22),
            ReachabilityClass::Direct,
            now_ms(),
        )
        .unwrap();
    authority
        .register_bridge(
            bridge_register("bridge-a", 23),
            ReachabilityClass::Direct,
            now_ms(),
        )
        .unwrap();
    let handle = authority_admin_server(authority);

    let (status, response): (u16, BridgesResponse) =
        get_json(handle.local_addr(), AuthorityRoute::AdminBridges.path());

    assert_eq!(status, 200);
    assert_eq!(response.bridges.len(), 2);
    assert_eq!(response.bridges[0].bridge_id, "bridge-a");
    assert_eq!(response.bridges[1].bridge_id, "bridge-b");
    handle.join().unwrap();
}

#[test]
fn admin_list_frames_filters_by_chain_id_and_limit() {
    let base_now_ms = now_ms();
    let mut authority = PublisherAuthority::new(publisher_signing_key());
    authority
        .register_bridge(
            bridge_register("bridge-frames", 42),
            ReachabilityClass::Direct,
            base_now_ms,
        )
        .unwrap();

    for (chain_id, session_id, frame_id, sequence) in [
        ("chain-a", "session-a1", "frame-a1", 0),
        ("chain-b", "session-b", "frame-b1", 0),
        ("chain-a", "session-a2", "frame-a2", 1),
    ] {
        authority
            .open_bridge_session_with_chain_id(
                Some(chain_id),
                BridgeOpen {
                    chain_id: chain_id.into(),
                    session_id: session_id.into(),
                    creator_id: "creator-a".into(),
                    bridge_id: "bridge-frames".into(),
                    creator_session_pub: node_public_key(51),
                    opened_at_ms: base_now_ms,
                    expected_chunks: None,
                },
            )
            .unwrap();
        authority
            .ingest_bridge_frame_with_chain_id(
                Some(chain_id),
                "bridge-frames",
                bridge_frame(chain_id, session_id, frame_id, sequence),
                base_now_ms + u64::from(sequence),
            )
            .unwrap();
    }

    let handle = authority_admin_server(authority);
    let path = format!(
        "{}?chain_id=chain-a&limit=1",
        AuthorityRoute::AdminFrames.path()
    );
    let (status, response): (u16, FramesResponse) = get_json(handle.local_addr(), &path);

    assert_eq!(status, 200);
    assert_eq!(response.frames.len(), 1);
    assert_eq!(response.frames[0].chain_id.as_deref(), Some("chain-a"));
    assert_eq!(response.frames[0].frame.frame_id, "frame-a2");
    handle.join().unwrap();
}

#[test]
fn admin_metrics_returns_authority_snapshot() {
    let mut authority = PublisherAuthority::new(publisher_signing_key());
    authority
        .register_bridge(
            bridge_register("bridge-metrics", 61),
            ReachabilityClass::Direct,
            now_ms(),
        )
        .unwrap();
    let handle = authority_admin_server(authority);

    let (status, response): (u16, MetricsResponse) =
        get_json(handle.local_addr(), AuthorityRoute::AdminMetrics.path());

    assert_eq!(status, 200);
    assert_eq!(response.authority.successful_registrations, 1);
    assert_eq!(response.authority.rejected_registrations, 0);
    handle.join().unwrap();
}

#[test]
fn admin_stub_metrics_returns_zero_snapshot_and_authority_only_routes_501() {
    let handle = stub_admin_server();

    let (status, metrics): (u16, MetricsResponse) =
        get_json(handle.local_addr(), AuthorityRoute::AdminMetrics.path());
    assert_eq!(status, 200);
    assert_eq!(metrics.authority.successful_registrations, 0);

    let (status, error): (u16, AdminErrorResponse) =
        get_json(handle.local_addr(), AuthorityRoute::AdminBridges.path());
    assert_eq!(status, 501);
    assert_eq!(error.error.code, "not_supported");
    handle.join().unwrap();
}

#[test]
fn admin_frames_rejects_invalid_limit() {
    let authority = PublisherAuthority::new(publisher_signing_key());
    let handle = authority_admin_server(authority);
    let path = format!("{}?limit=not-a-number", AuthorityRoute::AdminFrames.path());

    let (status, error): (u16, AdminErrorResponse) = get_json(handle.local_addr(), &path);

    assert_eq!(status, 400);
    assert_eq!(error.error.code, "bad_query");
    handle.join().unwrap();
}
