use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::SigningKey;
use gbn_bridge_creator::{
    build_upload_session, dispatch_upload_session, plan_lanes, BuildUploadSessionOptions,
    DispatchUploadOptions, SanitizerFormatHint, UploadSessionStatus,
};
use gbn_bridge_protocol::{
    publisher_identity, BridgeAck, BridgeAckStatus, BridgeDhtEntry, BridgeDhtEntryUnsigned,
    DhtBridgeIngressEndpoint as BridgeIngressEndpoint, LocalDiscoveryTable, PublicKeyBytes,
    PublisherDhtEntry, ReachabilityClass, SelfOnboardingState, TunnelPeerRole, TunnelState,
};

fn now_ms() -> u64 {
    10_000
}

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[9; 32])
}

fn bridge_entry(signing_key: &SigningKey, id: &str, port: u16, now_ms: u64) -> BridgeDhtEntry {
    BridgeDhtEntry::sign(
        BridgeDhtEntryUnsigned {
            bridge_id: id.to_string(),
            identity_pub: PublicKeyBytes(vec![port as u8; 32]),
            ingress_endpoints: vec![BridgeIngressEndpoint::direct("127.0.0.1", port)],
            udp_punch_port: port,
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

fn local_dht(bridge_count: u16, now_ms: u64) -> (LocalDiscoveryTable, PublicKeyBytes) {
    let signing_key = signing_key();
    let publisher_pub = publisher_identity(&signing_key);
    let mut table = LocalDiscoveryTable::empty("new-creator", now_ms);
    table.self_onboarding_state = SelfOnboardingState::Onboarded;
    table.publisher_entry = Some(PublisherDhtEntry {
        node_id: "publisher".to_string(),
        authority_url: "http://publisher-authority:8080".to_string(),
        receiver_url: "http://publisher-receiver:8081".to_string(),
        pub_key: publisher_pub.clone(),
        entry_expiry_ms: now_ms + 300_000,
    });
    table.bridge_entries = (0..bridge_count)
        .map(|idx| {
            bridge_entry(
                &signing_key,
                &format!("exit-bridge-{idx}"),
                40_000 + idx,
                now_ms,
            )
        })
        .collect();
    table.active_tunnels = table
        .bridge_entries
        .iter()
        .enumerate()
        .map(|(idx, entry)| TunnelState {
            peer_id: entry.bridge_id.clone(),
            peer_role: TunnelPeerRole::ExitBridge,
            established_at_ms: now_ms,
            last_seen_ms: now_ms + idx as u64,
            bootstrap_session_id: None,
        })
        .collect();
    (table, publisher_pub)
}

fn upload_session(
    bridge_count: u16,
    total_chunks: usize,
) -> (
    gbn_bridge_creator::EncryptedUploadSession,
    PublicKeyBytes,
    LocalDiscoveryTable,
) {
    let now = now_ms();
    let (table, publisher_pub) = local_dht(bridge_count, now);
    let publisher_entry = table.publisher_entry.clone().unwrap();
    let result = build_upload_session(
        BuildUploadSessionOptions {
            chain_id: format!("build-upload-session-test-{bridge_count}-{total_chunks}"),
            actor_id: "new-creator".to_string(),
            plaintext: vec![42; total_chunks * 1024],
            format_hint: SanitizerFormatHint::Opaque,
            chunk_size: 1024,
            sanitization_profile: "v3-default-no-visual-anon".to_string(),
            now_ms: now,
        },
        &publisher_entry,
        &table,
    )
    .unwrap();
    (result.session, publisher_pub, table)
}

#[derive(Default)]
struct FakeTransport {
    frames_by_bridge: BTreeMap<String, Vec<u32>>,
    fail_open_for: BTreeSet<String>,
    fail_first_data_frame_for: BTreeSet<String>,
}

impl FakeTransport {
    fn round_trip(
        &mut self,
        bridge_address: &str,
        request: gbn_bridge_creator::CreatorBridgeRequest,
    ) -> Result<gbn_bridge_creator::CreatorBridgeResponse, gbn_bridge_creator::CreatorError> {
        let bridge_id = bridge_id_for_address(bridge_address);
        match request {
            gbn_bridge_creator::CreatorBridgeRequest::Open(open) => {
                if self.fail_open_for.contains(&bridge_id) {
                    return Ok(gbn_bridge_creator::CreatorBridgeResponse::Error {
                        message: format!("forced open failure for {bridge_id}"),
                    });
                }
                Ok(gbn_bridge_creator::CreatorBridgeResponse::Opened {
                    chain_id: open.chain_id,
                    session_id: open.session_id,
                })
            }
            gbn_bridge_creator::CreatorBridgeRequest::Frame(frame) => {
                if frame.sequence != gbn_bridge_creator::MANIFEST_CHUNK_INDEX
                    && self.fail_first_data_frame_for.remove(&bridge_id)
                {
                    return Ok(gbn_bridge_creator::CreatorBridgeResponse::Error {
                        message: format!("forced frame failure for {bridge_id}"),
                    });
                }
                self.frames_by_bridge
                    .entry(bridge_id)
                    .or_default()
                    .push(frame.sequence);
                Ok(gbn_bridge_creator::CreatorBridgeResponse::Ack(BridgeAck {
                    chain_id: frame.chain_id,
                    session_id: frame.session_id,
                    acked_sequence: frame.sequence,
                    status: if frame.final_frame {
                        BridgeAckStatus::Complete
                    } else {
                        BridgeAckStatus::Accepted
                    },
                    acked_at_ms: frame.sent_at_ms + 1,
                }))
            }
            gbn_bridge_creator::CreatorBridgeRequest::FrameFragment(_) => {
                Ok(gbn_bridge_creator::CreatorBridgeResponse::Error {
                    message: "fragmented transport is covered by client integration tests"
                        .to_string(),
                })
            }
            gbn_bridge_creator::CreatorBridgeRequest::Close(close) => {
                Ok(gbn_bridge_creator::CreatorBridgeResponse::Closed {
                    chain_id: close.chain_id,
                    session_id: close.session_id,
                })
            }
        }
    }
}

fn bridge_id_for_address(address: &str) -> String {
    let port = address.rsplit(':').next().unwrap().parse::<u16>().unwrap();
    format!("exit-bridge-{}", port - 40_000)
}

fn dispatch(
    bridge_count: u16,
    target_lane_count: u32,
    force_lane_failure: Vec<String>,
) -> gbn_bridge_creator::SendUploadSessionResult {
    dispatch_with_open_failures(
        bridge_count,
        target_lane_count,
        force_lane_failure,
        Vec::new(),
    )
}

fn dispatch_with_open_failures(
    bridge_count: u16,
    target_lane_count: u32,
    force_lane_failure: Vec<String>,
    fail_open_for: Vec<String>,
) -> gbn_bridge_creator::SendUploadSessionResult {
    dispatch_with_failures(
        bridge_count,
        target_lane_count,
        force_lane_failure,
        fail_open_for,
        Vec::new(),
    )
}

fn dispatch_with_frame_failures(
    bridge_count: u16,
    target_lane_count: u32,
    fail_first_data_frame_for: Vec<String>,
) -> gbn_bridge_creator::SendUploadSessionResult {
    dispatch_with_failures(
        bridge_count,
        target_lane_count,
        Vec::new(),
        Vec::new(),
        fail_first_data_frame_for,
    )
}

fn dispatch_with_failures(
    bridge_count: u16,
    target_lane_count: u32,
    force_lane_failure: Vec<String>,
    fail_open_for: Vec<String>,
    fail_first_data_frame_for: Vec<String>,
) -> gbn_bridge_creator::SendUploadSessionResult {
    dispatch_with_failures_and_chunks(
        bridge_count,
        target_lane_count,
        128,
        force_lane_failure,
        fail_open_for,
        fail_first_data_frame_for,
    )
}

fn dispatch_with_failures_and_chunks(
    bridge_count: u16,
    target_lane_count: u32,
    total_chunks: usize,
    force_lane_failure: Vec<String>,
    fail_open_for: Vec<String>,
    fail_first_data_frame_for: Vec<String>,
) -> gbn_bridge_creator::SendUploadSessionResult {
    let (mut session, publisher_pub, _) = upload_session(bridge_count, total_chunks);
    let plan = plan_lanes(
        &session.local_dht_snapshot,
        &publisher_pub,
        target_lane_count,
        now_ms(),
    )
    .unwrap();
    let mut transport = FakeTransport {
        fail_open_for: fail_open_for.into_iter().collect(),
        fail_first_data_frame_for: fail_first_data_frame_for.into_iter().collect(),
        ..FakeTransport::default()
    };
    dispatch_upload_session(
        &mut session,
        plan,
        DispatchUploadOptions {
            chain_id: "send-upload-test".to_string(),
            actor_id: "new-creator".to_string(),
            actor_pub: PublicKeyBytes(vec![7; 32]),
            lane_open_timeout_ms: 30_000,
            chunk_ack_timeout_ms: 15_000,
            suspect_ttl_ms: 300_000,
            force_lane_failure,
            now_ms: now_ms(),
        },
        |address, request| transport.round_trip(address, request),
    )
    .unwrap()
}

#[test]
fn ten_active_bridges_spreads_chunks_across_lanes() {
    let result = dispatch(10, 10, Vec::new());
    assert_eq!(result.session_status, UploadSessionStatus::Completed);
    assert_eq!(result.completed_chunks, 128);
    assert_eq!(result.lanes_used.len(), 10);
    assert!(result.first_chunk_dispatched_at_ms < result.all_lanes_active_at_ms);
}

#[test]
fn five_active_bridges_reuse_lanes_for_all_chunks() {
    let result = dispatch(5, 10, Vec::new());
    assert_eq!(result.session_status, UploadSessionStatus::Completed);
    assert_eq!(result.lanes_used.len(), 5);
    assert!(result.reused_lane_events > 0);
}

#[test]
fn one_active_bridge_reuses_single_lane_for_every_chunk_after_first() {
    let result = dispatch(1, 10, Vec::new());
    assert_eq!(result.session_status, UploadSessionStatus::Completed);
    assert_eq!(result.lanes_used.len(), 1);
    assert_eq!(result.reused_lane_events, 127);
}

#[test]
fn forced_lane_failure_reroutes_and_completes() {
    let result = dispatch(10, 10, vec!["exit-bridge-9".to_string()]);
    assert_eq!(result.session_status, UploadSessionStatus::Completed);
    assert!(result.failover_events >= 1);
    assert_eq!(result.force_lane_failure_used, vec!["exit-bridge-9"]);
    assert!(!result.lanes_used.contains(&"exit-bridge-9".to_string()));
}

#[test]
fn forced_second_lane_failure_still_spreads_small_upload() {
    let result = dispatch_with_failures_and_chunks(
        10,
        10,
        8,
        vec!["exit-bridge-8".to_string()],
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(result.session_status, UploadSessionStatus::Completed);
    assert_eq!(result.completed_chunks, 8);
    assert!(result.failover_events >= 1);
    assert_eq!(result.force_lane_failure_used, vec!["exit-bridge-8"]);
    assert!(result.lanes_used.len() >= 2, "{result:?}");
}

#[test]
fn transient_open_failures_do_not_abort_remaining_lanes() {
    let result = dispatch_with_open_failures(
        10,
        10,
        Vec::new(),
        vec!["exit-bridge-9".to_string(), "exit-bridge-8".to_string()],
    );
    assert_eq!(result.session_status, UploadSessionStatus::Completed);
    assert_eq!(result.completed_chunks, 128);
    assert!(result.failover_events >= 2);
    assert_eq!(result.lanes_used.len(), 8);
    assert!(!result.lanes_used.contains(&"exit-bridge-9".to_string()));
    assert!(!result.lanes_used.contains(&"exit-bridge-8".to_string()));
}

#[test]
fn transient_frame_failures_do_not_abort_unopened_lanes() {
    let result = dispatch_with_frame_failures(
        10,
        10,
        vec!["exit-bridge-9".to_string(), "exit-bridge-8".to_string()],
    );
    assert_eq!(result.session_status, UploadSessionStatus::Completed);
    assert_eq!(result.completed_chunks, 128);
    assert!(result.failover_events >= 2);
    assert!(!result.lanes_used.contains(&"exit-bridge-9".to_string()));
    assert!(!result.lanes_used.contains(&"exit-bridge-8".to_string()));
    assert!(result.lanes_used.len() >= 2);
}
