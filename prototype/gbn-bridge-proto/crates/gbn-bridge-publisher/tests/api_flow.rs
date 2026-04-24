use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BootstrapJoinReply, BootstrapProgress, BootstrapProgressStage,
    BridgeCapability, BridgeCatalogRequest, BridgeIngressEndpoint, BridgeRegister, PendingCreator,
    PublicKeyBytes, ReachabilityClass,
};
use gbn_bridge_publisher::{
    api::{
        AuthorityApiRequest, AuthorityApiRequestUnsigned, AuthorityApiResponse, BootstrapJoinBody,
        BootstrapProgressBody, BridgeRegisterBody, CreatorCatalogBody, EmptyResponse,
        HealthResponse,
    },
    AuthorityServer, BootstrapProgressReceipt, CreatorCatalogResponse, PublisherAuthority,
    PublisherServiceConfig,
};

fn publisher_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[31_u8; 32])
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

fn authority_server() -> (gbn_bridge_publisher::AuthorityServerHandle, PublicKeyBytes) {
    let authority = PublisherAuthority::new(publisher_signing_key());
    let publisher_pub = authority.publisher_public_key().clone();
    let server = AuthorityServer::new(
        authority,
        PublisherServiceConfig {
            bind_addr: "127.0.0.1:0".into(),
            ..PublisherServiceConfig::default()
        },
    );
    let handle = server.bind().unwrap().spawn().unwrap();
    (handle, publisher_pub)
}

fn default_capabilities() -> Vec<BridgeCapability> {
    vec![
        BridgeCapability::BootstrapSeed,
        BridgeCapability::CatalogRefresh,
        BridgeCapability::BatchAssignment,
        BridgeCapability::ProgressReporting,
    ]
}

fn bridge_register(
    bridge_id: &str,
    key_seed: u8,
    host: &str,
    udp_punch_port: u16,
) -> BridgeRegister {
    BridgeRegister {
        bridge_id: bridge_id.into(),
        identity_pub: node_public_key(key_seed),
        ingress_endpoints: vec![BridgeIngressEndpoint {
            host: host.into(),
            port: 443,
        }],
        requested_udp_punch_port: udp_punch_port,
        capabilities: default_capabilities(),
    }
}

fn post_json<T, R>(addr: std::net::SocketAddr, path: &str, payload: &T) -> (u16, R)
where
    T: serde::Serialize,
    R: for<'de> serde::Deserialize<'de>,
{
    let body = serde_json::to_vec(payload).unwrap();
    let mut stream = TcpStream::connect(addr).unwrap();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        addr,
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    stream.write_all(&body).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();

    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    parse_http_response(&response)
}

fn get_json<R>(addr: std::net::SocketAddr, path: &str) -> (u16, R)
where
    R: for<'de> serde::Deserialize<'de>,
{
    let mut stream = TcpStream::connect(addr).unwrap();
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        addr
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
fn healthz_returns_signed_ready_status() {
    let (handle, publisher_pub) = authority_server();
    let (status, response): (u16, AuthorityApiResponse<HealthResponse>) =
        get_json(handle.local_addr(), "/healthz");
    assert_eq!(status, 200);
    assert!(response.ok);
    assert_eq!(response.chain_id, "system-healthz");
    response.verify_authority(&publisher_pub).unwrap();
    assert_eq!(response.body.as_ref().unwrap().status, "ok");
    handle.join().unwrap();
}

#[test]
fn bridge_register_route_returns_signed_lease_and_preserves_chain_id() {
    let (handle, publisher_pub) = authority_server();
    let base_now_ms = now_ms();
    let bridge_key = actor_signing_key(44);
    let request = AuthorityApiRequest::sign(
        AuthorityApiRequestUnsigned {
            chain_id: "chain-register-01".into(),
            request_id: "register-01".into(),
            sent_at_ms: base_now_ms,
            actor_id: "bridge-01".into(),
            body: BridgeRegisterBody {
                register: bridge_register("bridge-01", 44, "198.51.100.10", 443),
                reachability_class: ReachabilityClass::Direct,
                now_ms: base_now_ms,
            },
        },
        &bridge_key,
    )
    .unwrap();

    let (status, response): (u16, AuthorityApiResponse<gbn_bridge_protocol::BridgeLease>) =
        post_json(handle.local_addr(), "/v1/bridge/register", &request);
    assert_eq!(status, 200);
    assert!(response.ok);
    assert_eq!(response.chain_id, "chain-register-01");
    assert_eq!(response.request_id, "register-01");
    response.verify_authority(&publisher_pub).unwrap();
    assert_eq!(response.body.as_ref().unwrap().bridge_id, "bridge-01");
    handle.join().unwrap();
}

#[test]
fn catalog_and_join_routes_accept_signed_requests() {
    let (handle, publisher_pub) = authority_server();
    let base_now_ms = now_ms();
    let bridge_key = actor_signing_key(55);
    let bridge_request = AuthorityApiRequest::sign(
        AuthorityApiRequestUnsigned {
            chain_id: "chain-setup".into(),
            request_id: "register-seed".into(),
            sent_at_ms: base_now_ms,
            actor_id: "bridge-seed".into(),
            body: BridgeRegisterBody {
                register: bridge_register("bridge-seed", 55, "198.51.100.20", 443),
                reachability_class: ReachabilityClass::Direct,
                now_ms: base_now_ms,
            },
        },
        &bridge_key,
    )
    .unwrap();
    let (status, _): (u16, AuthorityApiResponse<gbn_bridge_protocol::BridgeLease>) =
        post_json(handle.local_addr(), "/v1/bridge/register", &bridge_request);
    assert_eq!(status, 200);

    let relay_key = actor_signing_key(56);
    let relay_request = AuthorityApiRequest::sign(
        AuthorityApiRequestUnsigned {
            chain_id: "chain-setup".into(),
            request_id: "register-relay".into(),
            sent_at_ms: base_now_ms,
            actor_id: "bridge-relay".into(),
            body: BridgeRegisterBody {
                register: bridge_register("bridge-relay", 56, "198.51.100.21", 443),
                reachability_class: ReachabilityClass::Direct,
                now_ms: base_now_ms,
            },
        },
        &relay_key,
    )
    .unwrap();
    let (status, _): (u16, AuthorityApiResponse<gbn_bridge_protocol::BridgeLease>) =
        post_json(handle.local_addr(), "/v1/bridge/register", &relay_request);
    assert_eq!(status, 200);

    let creator_key = actor_signing_key(60);
    let catalog_request = AuthorityApiRequest::sign(
        AuthorityApiRequestUnsigned {
            chain_id: "chain-catalog-01".into(),
            request_id: "catalog-01".into(),
            sent_at_ms: base_now_ms,
            actor_id: "creator-01".into(),
            body: CreatorCatalogBody {
                request: BridgeCatalogRequest {
                    creator_id: "creator-01".into(),
                    known_catalog_id: None,
                    direct_only: false,
                    refresh_hint: None,
                },
                now_ms: base_now_ms,
            },
        },
        &creator_key,
    )
    .unwrap();
    let (status, response): (u16, CreatorCatalogResponse) =
        post_json(handle.local_addr(), "/v1/creator/catalog", &catalog_request);
    assert_eq!(status, 200);
    assert_eq!(response.chain_id, "chain-catalog-01");
    response.verify_authority(&publisher_pub).unwrap();
    assert!(!response.body.as_ref().unwrap().bridges.is_empty());

    let host_key = actor_signing_key(61);
    let join_request = AuthorityApiRequest::sign(
        AuthorityApiRequestUnsigned {
            chain_id: "chain-bootstrap-01".into(),
            request_id: "join-01".into(),
            sent_at_ms: base_now_ms,
            actor_id: "host-creator-01".into(),
            body: BootstrapJoinBody {
                request: gbn_bridge_protocol::CreatorJoinRequest {
                    chain_id: "chain-bootstrap-01".into(),
                    request_id: "join-01".into(),
                    host_creator_id: "host-creator-01".into(),
                    relay_bridge_id: "bridge-relay".into(),
                    creator: PendingCreator {
                        node_id: "creator-boot".into(),
                        ip_addr: "203.0.113.44".into(),
                        pub_key: node_public_key(62),
                        udp_punch_port: 443,
                    },
                },
                now_ms: base_now_ms,
            },
        },
        &host_key,
    )
    .unwrap();
    let (status, response): (u16, AuthorityApiResponse<BootstrapJoinReply>) =
        post_json(handle.local_addr(), "/v1/bootstrap/join", &join_request);
    assert_eq!(status, 200);
    assert_eq!(response.chain_id, "chain-bootstrap-01");
    response.verify_authority(&publisher_pub).unwrap();
    assert_eq!(
        response
            .body
            .as_ref()
            .unwrap()
            .response
            .bootstrap_session_id,
        "bootstrap-000001"
    );
    handle.join().unwrap();
}

#[test]
fn progress_route_records_events_and_invalid_signature_is_rejected() {
    let (handle, publisher_pub) = authority_server();
    let base_now_ms = now_ms();
    let bridge_key = actor_signing_key(70);
    let register_request = AuthorityApiRequest::sign(
        AuthorityApiRequestUnsigned {
            chain_id: "chain-progress".into(),
            request_id: "register-progress".into(),
            sent_at_ms: base_now_ms,
            actor_id: "bridge-progress".into(),
            body: BridgeRegisterBody {
                register: bridge_register("bridge-progress", 70, "198.51.100.30", 443),
                reachability_class: ReachabilityClass::Direct,
                now_ms: base_now_ms,
            },
        },
        &bridge_key,
    )
    .unwrap();
    let (status, _): (u16, AuthorityApiResponse<gbn_bridge_protocol::BridgeLease>) = post_json(
        handle.local_addr(),
        "/v1/bridge/register",
        &register_request,
    );
    assert_eq!(status, 200);

    let host_key = actor_signing_key(71);
    let join_request = AuthorityApiRequest::sign(
        AuthorityApiRequestUnsigned {
            chain_id: "chain-progress".into(),
            request_id: "join-progress".into(),
            sent_at_ms: base_now_ms,
            actor_id: "host-creator-01".into(),
            body: BootstrapJoinBody {
                request: gbn_bridge_protocol::CreatorJoinRequest {
                    chain_id: "chain-progress".into(),
                    request_id: "join-progress".into(),
                    host_creator_id: "host-creator-01".into(),
                    relay_bridge_id: "bridge-progress".into(),
                    creator: PendingCreator {
                        node_id: "creator-progress".into(),
                        ip_addr: "203.0.113.50".into(),
                        pub_key: node_public_key(72),
                        udp_punch_port: 443,
                    },
                },
                now_ms: base_now_ms,
            },
        },
        &host_key,
    )
    .unwrap();
    let (status, join_response): (u16, AuthorityApiResponse<BootstrapJoinReply>) =
        post_json(handle.local_addr(), "/v1/bootstrap/join", &join_request);
    assert_eq!(status, 200);
    let bootstrap_session_id = join_response.body.unwrap().response.bootstrap_session_id;

    let progress_request = AuthorityApiRequest::sign(
        AuthorityApiRequestUnsigned {
            chain_id: "chain-progress".into(),
            request_id: "progress-01".into(),
            sent_at_ms: base_now_ms,
            actor_id: "bridge-progress".into(),
            body: BootstrapProgressBody {
                progress: BootstrapProgress {
                    chain_id: "chain-progress".into(),
                    bootstrap_session_id: bootstrap_session_id.clone(),
                    reporter_id: "bridge-progress".into(),
                    stage: BootstrapProgressStage::SeedTunnelEstablished,
                    active_bridge_count: 1,
                    reported_at_ms: base_now_ms,
                },
            },
        },
        &bridge_key,
    )
    .unwrap();
    let (status, response): (u16, AuthorityApiResponse<BootstrapProgressReceipt>) = post_json(
        handle.local_addr(),
        "/v1/bridge/progress",
        &progress_request,
    );
    assert_eq!(status, 200);
    response.verify_authority(&publisher_pub).unwrap();
    assert_eq!(response.body.as_ref().unwrap().stored_event_count, 1);

    let mut tampered = progress_request.clone();
    tampered.chain_id = "chain-progress-tampered".into();
    let (status, error): (u16, AuthorityApiResponse<EmptyResponse>) =
        post_json(handle.local_addr(), "/v1/bridge/progress", &tampered);
    assert_eq!(status, 401);
    assert!(!error.ok);
    assert_eq!(error.error.unwrap().code, "unauthorized");
    handle.join().unwrap();
}

#[test]
fn malformed_json_is_rejected_with_structured_error() {
    let (handle, _publisher_pub) = authority_server();
    let mut stream = TcpStream::connect(handle.local_addr()).unwrap();
    let body = b"{not-json";
    let request = format!(
        "POST /v1/bridge/register HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        handle.local_addr(),
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();

    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let (status, parsed): (u16, AuthorityApiResponse<EmptyResponse>) =
        parse_http_response(&response);
    assert_eq!(status, 400);
    assert!(!parsed.ok);
    assert_eq!(parsed.error.unwrap().code, "bad_request");
    handle.join().unwrap();
}
