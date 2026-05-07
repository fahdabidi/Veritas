use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BridgeCapability, BridgeCatalogResponse, BridgeCatalogResponseUnsigned,
    BridgeCommandPayload, BridgeControlFrame, BridgeControlHello, BridgeControlHelloUnsigned,
    BridgeIngressEndpoint, BridgeRegister, ReachabilityClass,
};
use gbn_bridge_publisher::{
    admin::{AdminErrorResponse, AdminHttpServer, AdminState, InjectCommandRequest},
    control::BridgeAdminCommandReceipt,
    AuthorityService, PublisherAuthority, PublisherServiceConfig,
};

fn publisher_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[94_u8; 32])
}

fn bridge_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[95_u8; 32])
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn bridge_register(bridge_id: &str) -> BridgeRegister {
    BridgeRegister {
        bridge_id: bridge_id.into(),
        identity_pub: publisher_identity(&bridge_signing_key()),
        ingress_endpoints: vec![BridgeIngressEndpoint {
            host: "198.51.100.60".into(),
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

fn catalog_payload(catalog_id: &str) -> BridgeCommandPayload {
    BridgeCommandPayload::CatalogRefresh(
        BridgeCatalogResponse::sign(
            BridgeCatalogResponseUnsigned {
                catalog_id: catalog_id.into(),
                issued_at_ms: now_ms(),
                expires_at_ms: now_ms() + 30_000,
                bridges: Vec::new(),
            },
            &publisher_signing_key(),
        )
        .unwrap(),
    )
}

fn connected_admin_server() -> (
    gbn_bridge_publisher::admin::AdminHttpServerHandle,
    Receiver<BridgeControlFrame>,
) {
    let bridge_id = "bridge-admin";
    let base_now_ms = now_ms();
    let mut authority = PublisherAuthority::new(publisher_signing_key());
    let lease = authority
        .register_bridge(
            bridge_register(bridge_id),
            ReachabilityClass::Direct,
            base_now_ms,
        )
        .unwrap();
    let mut service = AuthorityService::new(authority, &PublisherServiceConfig::default());
    let hello = BridgeControlHello::sign(
        BridgeControlHelloUnsigned {
            bridge_id: bridge_id.into(),
            lease_id: lease.lease_id,
            bridge_pub: publisher_identity(&bridge_signing_key()),
            sent_at_ms: base_now_ms,
            request_id: "admin-control-hello".into(),
            resume_acked_seq_no: None,
            chain_id: "admin-control-chain".into(),
        },
        &bridge_signing_key(),
    )
    .unwrap();
    let (tx, rx) = mpsc::channel();
    let (_welcome, initial_commands) = service.accept_control_hello(hello, tx).unwrap();
    assert!(initial_commands.is_empty());

    let admin = AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::authority(Arc::new(Mutex::new(service))),
        1_048_576,
    )
    .unwrap();
    (admin.spawn().unwrap(), rx)
}

fn authority_admin_server() -> gbn_bridge_publisher::admin::AdminHttpServerHandle {
    let service = AuthorityService::new(
        PublisherAuthority::new(publisher_signing_key()),
        &PublisherServiceConfig::default(),
    );
    let admin = AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::authority(Arc::new(Mutex::new(service))),
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

fn command_path(bridge_id: &str) -> String {
    format!("/v1/admin/bridges/{bridge_id}/command")
}

fn post_json<T, R>(addr: SocketAddr, path: &str, payload: &T) -> (u16, R)
where
    T: serde::Serialize,
    R: for<'de> serde::Deserialize<'de>,
{
    let body = serde_json::to_vec(payload).unwrap();
    let mut stream = TcpStream::connect(addr).unwrap();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    stream.write_all(&body).unwrap();
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
fn admin_injects_catalog_refresh_to_connected_bridge() {
    let (handle, rx) = connected_admin_server();
    let request = InjectCommandRequest {
        payload: catalog_payload("admin-catalog-01"),
    };

    let (status, receipt): (u16, BridgeAdminCommandReceipt) =
        post_json(handle.local_addr(), &command_path("bridge-admin"), &request);

    assert_eq!(status, 200);
    assert_eq!(receipt.command_id, "cmd-bridge-admin-000001");
    assert_eq!(receipt.seq_no, 1);
    let frame = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let BridgeControlFrame::Command(command) = frame else {
        panic!("expected command frame");
    };
    assert_eq!(command.command_id, receipt.command_id);
    assert_eq!(command.seq_no, receipt.seq_no);
    assert_eq!(command.bridge_id, "bridge-admin");
    assert!(matches!(
        command.payload,
        BridgeCommandPayload::CatalogRefresh(_)
    ));
    handle.join().unwrap();
}

#[test]
fn admin_command_sequence_numbers_increment_without_collision() {
    let (handle, rx) = connected_admin_server();

    for index in 1..=2 {
        let request = InjectCommandRequest {
            payload: catalog_payload(&format!("admin-catalog-{index:02}")),
        };
        let (status, receipt): (u16, BridgeAdminCommandReceipt) =
            post_json(handle.local_addr(), &command_path("bridge-admin"), &request);
        assert_eq!(status, 200);
        assert_eq!(receipt.seq_no, index);
        let command = loop {
            let frame = rx.recv_timeout(Duration::from_secs(1)).unwrap();
            let BridgeControlFrame::Command(command) = frame else {
                panic!("expected command frame");
            };
            if command.command_id == receipt.command_id {
                break command;
            }
        };
        assert_eq!(command.seq_no, receipt.seq_no);
    }
    handle.join().unwrap();
}

#[test]
fn admin_inject_unknown_bridge_returns_404() {
    let handle = authority_admin_server();
    let request = InjectCommandRequest {
        payload: catalog_payload("admin-catalog-missing"),
    };

    let (status, error): (u16, AdminErrorResponse) = post_json(
        handle.local_addr(),
        &command_path("missing-bridge"),
        &request,
    );

    assert_eq!(status, 404);
    assert_eq!(error.error.code, "not_found");
    handle.join().unwrap();
}

#[test]
fn admin_inject_on_non_authority_returns_501() {
    let handle = stub_admin_server();
    let request = InjectCommandRequest {
        payload: catalog_payload("admin-catalog-stub"),
    };

    let (status, error): (u16, AdminErrorResponse) =
        post_json(handle.local_addr(), &command_path("bridge-admin"), &request);

    assert_eq!(status, 501);
    assert_eq!(error.error.code, "not_supported");
    handle.join().unwrap();
}
