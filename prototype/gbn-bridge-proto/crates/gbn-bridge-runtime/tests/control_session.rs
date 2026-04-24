use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BootstrapJoinReply, BridgeCapability, BridgeIngressEndpoint, BridgeLease,
    BridgeRegister, CreatorJoinRequest, PendingCreator, PublicKeyBytes, ReachabilityClass,
};
use gbn_bridge_publisher::{
    api::{
        AuthorityApiRequest, AuthorityApiRequestUnsigned, AuthorityApiResponse, BootstrapJoinBody,
        BridgeRegisterBody,
    },
    AuthorityServer, PublisherAuthority, PublisherServiceConfig,
};
use gbn_bridge_runtime::{
    BridgeControlClient, ExitBridgeConfig, ExitBridgeRuntime, InProcessPublisherClient,
};

fn publisher_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[41_u8; 32])
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

fn authority_server() -> (
    gbn_bridge_publisher::AuthorityServerHandle,
    PublicKeyBytes,
    SigningKey,
) {
    let signing_key = publisher_signing_key();
    let authority = PublisherAuthority::new(signing_key.clone());
    let publisher_pub = authority.publisher_public_key().clone();
    let server = AuthorityServer::new(
        authority,
        PublisherServiceConfig {
            bind_addr: "127.0.0.1:0".into(),
            ..PublisherServiceConfig::default()
        },
    );
    let handle = server.bind().unwrap().spawn().unwrap();
    (handle, publisher_pub, signing_key)
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
        capabilities: vec![
            BridgeCapability::BootstrapSeed,
            BridgeCapability::CatalogRefresh,
            BridgeCapability::BatchAssignment,
            BridgeCapability::ProgressReporting,
        ],
    }
}

fn direct_bridge_config(bridge_id: &str, key_seed: u8, host: &str) -> ExitBridgeConfig {
    ExitBridgeConfig {
        bridge_id: bridge_id.into(),
        identity_pub: node_public_key(key_seed),
        ingress_endpoint: BridgeIngressEndpoint {
            host: host.into(),
            port: 443,
        },
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

fn creator_join_request(
    request_id: &str,
    relay_bridge_id: &str,
    key_seed: u8,
) -> CreatorJoinRequest {
    CreatorJoinRequest {
        request_id: request_id.into(),
        host_creator_id: "host-creator-01".into(),
        relay_bridge_id: relay_bridge_id.into(),
        creator: PendingCreator {
            node_id: format!("creator-{request_id}"),
            ip_addr: "203.0.113.55".into(),
            pub_key: node_public_key(key_seed),
            udp_punch_port: 443,
        },
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

fn register_bridge_via_api(
    addr: std::net::SocketAddr,
    chain_id: &str,
    request_id: &str,
    bridge_id: &str,
    key_seed: u8,
    host: &str,
    now_ms: u64,
) -> BridgeLease {
    let bridge_key = actor_signing_key(key_seed);
    let request = AuthorityApiRequest::sign(
        AuthorityApiRequestUnsigned {
            chain_id: chain_id.into(),
            request_id: request_id.into(),
            sent_at_ms: now_ms,
            actor_id: bridge_id.into(),
            body: BridgeRegisterBody {
                register: bridge_register(bridge_id, key_seed, host, 443),
                reachability_class: ReachabilityClass::Direct,
                now_ms,
            },
        },
        &bridge_key,
    )
    .unwrap();
    let (status, response): (u16, AuthorityApiResponse<BridgeLease>) =
        post_json(addr, "/v1/bridge/register", &request);
    assert_eq!(status, 200);
    response.body.unwrap()
}

#[test]
fn control_session_receives_seed_commands_and_resumes_after_reconnect() {
    let (handle, publisher_pub, signing_key) = authority_server();
    let base_now_ms = now_ms();

    let seed_lease = register_bridge_via_api(
        handle.local_addr(),
        "chain-seed-register",
        "register-seed",
        "bridge-seed",
        61,
        "198.51.100.11",
        base_now_ms,
    );
    let _relay_lease = register_bridge_via_api(
        handle.local_addr(),
        "chain-relay-register",
        "register-relay",
        "bridge-relay",
        62,
        "198.51.100.12",
        base_now_ms,
    );

    let local_authority = PublisherAuthority::new(signing_key.clone());
    let mut bridge = ExitBridgeRuntime::new(
        direct_bridge_config("bridge-seed", 61, "198.51.100.11"),
        InProcessPublisherClient::new(local_authority),
    );
    bridge.apply_remote_lease(seed_lease.clone(), base_now_ms);

    let ws_url = format!("ws://{}/v1/bridge/control", handle.local_addr());
    let bridge_key = actor_signing_key(61);
    let control_client = BridgeControlClient::connect(
        &ws_url,
        "bridge-seed",
        &seed_lease.lease_id,
        &node_public_key(61),
        &bridge_key,
        &publisher_pub,
        "control-chain-001",
        "control-hello-001",
        base_now_ms,
        None,
        30_000,
    )
    .unwrap();
    bridge.attach_control_client(control_client);

    let host_key = actor_signing_key(70);
    let join_request = AuthorityApiRequest::sign(
        AuthorityApiRequestUnsigned {
            chain_id: "chain-bootstrap-001".into(),
            request_id: "join-001".into(),
            sent_at_ms: base_now_ms,
            actor_id: "host-creator-01".into(),
            body: BootstrapJoinBody {
                request: creator_join_request("join-001", "bridge-relay", 71),
                now_ms: base_now_ms,
            },
        },
        &host_key,
    )
    .unwrap();
    let (status, response): (u16, AuthorityApiResponse<BootstrapJoinReply>) =
        post_json(handle.local_addr(), "/v1/bootstrap/join", &join_request);
    assert_eq!(status, 200);
    let plan = response.body.unwrap();

    let ack = bridge
        .receive_next_control_command(base_now_ms + 50)
        .unwrap()
        .expect("seed bridge should receive a publisher command");
    assert_eq!(ack.bridge_id, "bridge-seed");
    assert_eq!(ack.seq_no, 1);
    assert_eq!(ack.chain_id, "chain-bootstrap-001");
    assert_eq!(bridge.publisher_client().reported_progress().len(), 0);

    let active_attempt = bridge
        .active_punch_attempt(&plan.response.bootstrap_session_id)
        .cloned()
        .expect("control command should start a punch attempt");
    bridge
        .acknowledge_tunnel(
            &plan.response.bootstrap_session_id,
            &plan.creator_entry.node_id,
            443,
            active_attempt.probe_nonce,
            base_now_ms + 100,
        )
        .unwrap();

    let last_acked_seq_no = bridge
        .control_client()
        .and_then(|client| client.last_acked_seq_no())
        .unwrap();
    let replacement_client = BridgeControlClient::connect(
        &ws_url,
        "bridge-seed",
        &seed_lease.lease_id,
        &node_public_key(61),
        &bridge_key,
        &publisher_pub,
        "control-chain-002",
        "control-hello-002",
        now_ms(),
        Some(last_acked_seq_no),
        30_000,
    )
    .unwrap();
    bridge.attach_control_client(replacement_client);

    let second_join_now_ms = now_ms();
    let second_join = AuthorityApiRequest::sign(
        AuthorityApiRequestUnsigned {
            chain_id: "chain-bootstrap-002".into(),
            request_id: "join-002".into(),
            sent_at_ms: second_join_now_ms,
            actor_id: "host-creator-01".into(),
            body: BootstrapJoinBody {
                request: creator_join_request("join-002", "bridge-relay", 72),
                now_ms: second_join_now_ms,
            },
        },
        &host_key,
    )
    .unwrap();
    let (status, _response): (u16, AuthorityApiResponse<BootstrapJoinReply>) =
        post_json(handle.local_addr(), "/v1/bootstrap/join", &second_join);
    assert_eq!(status, 200);

    let second_ack = bridge
        .receive_next_control_command(base_now_ms + 250)
        .unwrap()
        .expect("reconnected bridge should receive the next command");
    assert_eq!(second_ack.seq_no, 2);
    assert_eq!(second_ack.chain_id, "chain-bootstrap-002");

    handle.join().unwrap();
}
