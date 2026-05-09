use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_creator::{
    CreatorBridgeRequest, CreatorBridgeResponse, SendUploadSessionResult, UploadDispatchPlan,
};
use gbn_bridge_protocol::{
    publisher_identity, BridgeAck, BridgeAckStatus, BridgeDhtEntry, BridgeDhtEntryUnsigned,
    DhtBridgeIngressEndpoint as BridgeIngressEndpoint, LocalDiscoveryTable, PublicKeyBytes,
    PublisherDhtEntry, ReachabilityClass, SelfOnboardingState, TunnelPeerRole, TunnelState,
};
use gbn_bridge_publisher::admin::{
    AdminCreatorConfig, AdminHttpServer, AdminNodeMetadata, AdminState, BuildUploadSessionResponse,
};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_dir(name: &str) -> PathBuf {
    let counter = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "veritas-admin-send-upload-{name}-{}-{}-{counter}",
        std::process::id(),
        now_ms()
    ))
}

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

struct FakeBridgeHandle {
    id: String,
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl FakeBridgeHandle {
    fn start(id: impl Into<String>) -> Self {
        let id = id.into();
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let addr = socket.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = stop.clone();
        let join = thread::spawn(move || {
            let mut buffer = vec![0_u8; 60 * 1024];
            let mut fragments = BTreeMap::new();
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
                let response = fake_bridge_response(&buffer[..read], &mut fragments);
                let payload = serde_json::to_vec(&response).unwrap();
                let _ = socket.send_to(&payload, peer);
            }
        });
        Self {
            id,
            addr,
            stop,
            join: Some(join),
        }
    }
}

impl Drop for FakeBridgeHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = UdpSocket::bind("127.0.0.1:0")
            .and_then(|socket| socket.send_to(&[0], self.addr).map(|_| ()));
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[derive(Debug)]
struct PendingTestFrame {
    sequence: u32,
    total_fragments: u16,
    fragments: Vec<Option<Vec<u8>>>,
}

fn fake_bridge_response(
    payload: &[u8],
    fragments: &mut BTreeMap<(String, String, String), PendingTestFrame>,
) -> CreatorBridgeResponse {
    let request = match serde_json::from_slice::<CreatorBridgeRequest>(payload) {
        Ok(request) => request,
        Err(error) => {
            return CreatorBridgeResponse::Error {
                message: error.to_string(),
            }
        }
    };
    match request {
        CreatorBridgeRequest::Open(open) => CreatorBridgeResponse::Opened {
            chain_id: open.chain_id,
            session_id: open.session_id,
        },
        CreatorBridgeRequest::Frame(frame) => CreatorBridgeResponse::Ack(BridgeAck {
            chain_id: frame.chain_id,
            session_id: frame.session_id,
            acked_sequence: frame.sequence,
            status: if frame.final_frame {
                BridgeAckStatus::Complete
            } else {
                BridgeAckStatus::Accepted
            },
            acked_at_ms: now_ms(),
        }),
        CreatorBridgeRequest::FrameFragment(fragment) => {
            let key = (
                fragment.chain_id.clone(),
                fragment.session_id.clone(),
                fragment.frame_id.clone(),
            );
            let frame_bytes = match fragment.decoded_frame_bytes() {
                Ok(bytes) => bytes,
                Err(message) => return CreatorBridgeResponse::Error { message },
            };
            let pending = fragments
                .entry(key.clone())
                .or_insert_with(|| PendingTestFrame {
                    sequence: fragment.sequence,
                    total_fragments: fragment.total_fragments,
                    fragments: vec![None; fragment.total_fragments as usize],
                });
            if pending.sequence != fragment.sequence
                || pending.total_fragments != fragment.total_fragments
                || fragment.fragment_index >= pending.total_fragments
            {
                return CreatorBridgeResponse::Error {
                    message: "bad test fragment metadata".to_string(),
                };
            }
            pending.fragments[fragment.fragment_index as usize] = Some(frame_bytes);
            if pending.fragments.iter().any(Option::is_none) {
                return CreatorBridgeResponse::FrameFragmentAccepted {
                    chain_id: fragment.chain_id,
                    session_id: fragment.session_id,
                    frame_id: fragment.frame_id,
                    fragment_index: fragment.fragment_index,
                    total_fragments: fragment.total_fragments,
                };
            }
            let completed = fragments.remove(&key).unwrap();
            let mut bytes = Vec::new();
            for fragment in completed.fragments.into_iter().flatten() {
                bytes.extend(fragment);
            }
            match serde_json::from_slice::<gbn_bridge_protocol::BridgeData>(&bytes) {
                Ok(frame) => CreatorBridgeResponse::Ack(BridgeAck {
                    chain_id: frame.chain_id,
                    session_id: frame.session_id,
                    acked_sequence: frame.sequence,
                    status: if frame.final_frame {
                        BridgeAckStatus::Complete
                    } else {
                        BridgeAckStatus::Accepted
                    },
                    acked_at_ms: now_ms(),
                }),
                Err(error) => CreatorBridgeResponse::Error {
                    message: error.to_string(),
                },
            }
        }
        CreatorBridgeRequest::Close(close) => CreatorBridgeResponse::Closed {
            chain_id: close.chain_id,
            session_id: close.session_id,
        },
    }
}

fn bridge_entry(
    signing_key: &SigningKey,
    bridge: &FakeBridgeHandle,
    now_ms: u64,
) -> BridgeDhtEntry {
    BridgeDhtEntry::sign(
        BridgeDhtEntryUnsigned {
            bridge_id: bridge.id.clone(),
            identity_pub: PublicKeyBytes(vec![bridge.addr.port() as u8; 32]),
            ingress_endpoints: vec![BridgeIngressEndpoint::direct(
                bridge.addr.ip().to_string(),
                bridge.addr.port(),
            )],
            udp_punch_port: bridge.addr.port(),
            reachability_class: ReachabilityClass::Direct,
            lease_expiry_ms: now_ms + 300_000,
            entry_expiry_ms: now_ms + 300_000,
            capabilities: vec!["session_relay".to_string()],
        },
        signing_key,
        true,
    )
    .unwrap()
}

fn start_creator_admin(
    bridges: &[FakeBridgeHandle],
) -> (gbn_bridge_publisher::admin::AdminHttpServerHandle, PathBuf) {
    let state_dir = unique_dir("creator");
    std::fs::create_dir_all(&state_dir).unwrap();
    let publisher_key = signing_key(9);
    let publisher_pub = publisher_identity(&publisher_key);
    let now = now_ms();
    let mut table = LocalDiscoveryTable::empty("new-creator", now);
    table.self_onboarding_state = SelfOnboardingState::Onboarded;
    table.publisher_entry = Some(PublisherDhtEntry {
        node_id: "publisher".to_string(),
        authority_url: "http://publisher-authority:8080".to_string(),
        receiver_url: "http://publisher-receiver:8081".to_string(),
        pub_key: publisher_pub.clone(),
        encryption_pub_key: None,
        entry_expiry_ms: now + 300_000,
    });
    table.bridge_entries = bridges
        .iter()
        .map(|bridge| bridge_entry(&publisher_key, bridge, now))
        .collect();
    table.active_tunnels = table
        .bridge_entries
        .iter()
        .enumerate()
        .map(|(idx, entry)| TunnelState {
            peer_id: entry.bridge_id.clone(),
            peer_role: TunnelPeerRole::ExitBridge,
            established_at_ms: now,
            last_seen_ms: now + idx as u64,
            bootstrap_session_id: None,
        })
        .collect();

    let store = gbn_bridge_creator::LocalDhtStore::start(
        "new-creator",
        state_dir.join("local_dht.json"),
        table,
    );
    let creator_key = signing_key(4);
    let mut metadata = AdminNodeMetadata::from_env("creator-new", "creator")
        .with_public_key(&publisher_identity(&creator_key))
        .with_publisher_public_key(&publisher_pub)
        .with_creator_transport("127.0.0.1", 4443);
    metadata.state_dir = Some(state_dir.to_string_lossy().into_owned());
    let admin = AdminHttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdminState::creator_with_config(
            metadata,
            store,
            AdminCreatorConfig {
                actor_id: "new-creator".to_string(),
                signing_key: creator_key,
                publisher_pub,
                authority_url: "http://publisher-authority:8080".to_string(),
                creator_ip_addr: "127.0.0.1".to_string(),
                udp_punch_port: 4443,
                timeout: Duration::from_secs(5),
            },
        ),
        1_048_576,
    )
    .unwrap()
    .spawn()
    .unwrap();
    (admin, state_dir)
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
fn send_upload_endpoint_dispatches_session_and_persists_plan() {
    let bridges = vec![
        FakeBridgeHandle::start("exit-bridge-0"),
        FakeBridgeHandle::start("exit-bridge-1"),
    ];
    let (handle, state_dir) = start_creator_admin(&bridges);
    let build_body = r#"{
        "input_source":"synthetic",
        "synthetic_size_bytes":32768,
        "synthetic_marker":"VERITAS-SMOKE-4-PLAINTEXT",
        "chunk_size_bytes":4096,
        "sanitization_profile":"v3-default-no-visual-anon"
    }"#;
    let (status, built): (u16, BuildUploadSessionResponse) = request_json(
        handle.local_addr(),
        "POST",
        "/v1/admin/build-upload-session?chain_id=phase11-build",
        build_body,
    );
    assert_eq!(status, 200);
    assert_eq!(built.manifest.total_chunks, 8);

    let send_body = format!(
        r#"{{"session_id":"{}","target_lane_count":2}}"#,
        built.session_id
    );
    let (status, sent): (u16, SendUploadSessionResult) = request_json(
        handle.local_addr(),
        "POST",
        "/v1/admin/send-upload?chain_id=phase11-send",
        &send_body,
    );
    assert_eq!(status, 200);
    assert_eq!(sent.chain_id, "phase11-send");
    assert_eq!(
        sent.session_status,
        gbn_bridge_creator::UploadSessionStatus::Completed
    );
    assert_eq!(sent.completed_chunks, 8);
    assert_eq!(sent.lanes_used.len(), 2);
    assert!(sent.first_chunk_dispatched_at_ms < sent.all_lanes_active_at_ms);

    let plan_path = format!(
        "/v1/admin/upload-sessions/{}/dispatch-plan",
        built.session_id
    );
    let (status, plan): (u16, UploadDispatchPlan) =
        request_json(handle.local_addr(), "GET", &plan_path, "");
    assert_eq!(status, 200);
    assert_eq!(plan.completed_chunks, 8);
    assert_eq!(plan.chunk_assignments.len(), 8);
    assert_eq!(
        plan.session_status,
        gbn_bridge_creator::UploadSessionStatus::Completed
    );

    let _ = std::fs::remove_dir_all(state_dir);
}
