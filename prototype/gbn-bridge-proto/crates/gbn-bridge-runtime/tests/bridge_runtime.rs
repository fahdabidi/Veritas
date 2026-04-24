use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BootstrapDhtEntry, BridgeCapability, BridgeData, BridgeIngressEndpoint,
    BridgeSetRequest, CreatorJoinRequest, PendingCreator, ReachabilityClass,
};
use gbn_bridge_publisher::PublisherAuthority;
use gbn_bridge_runtime::{
    ExitBridgeConfig, ExitBridgeRuntime, InProcessPublisherClient, RuntimeError,
};

fn publisher_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[31_u8; 32])
}

fn publisher() -> PublisherAuthority {
    PublisherAuthority::new(publisher_signing_key())
}

fn node_public_key(seed: u8) -> gbn_bridge_protocol::PublicKeyBytes {
    publisher_identity(&SigningKey::from_bytes(&[seed; 32]))
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

fn join_request(request_id: &str, relay_bridge_id: &str, key_seed: u8) -> CreatorJoinRequest {
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

#[test]
fn bridge_registers_successfully_on_startup() {
    let config = direct_bridge_config("bridge-01", 41, "198.51.100.10");
    let client = InProcessPublisherClient::new(publisher());
    let mut bridge = ExitBridgeRuntime::new(config, client);

    let lease = bridge.startup(ReachabilityClass::Direct, 1_000).unwrap();
    lease
        .verify_authority(&bridge.publisher_client().publisher_public_key(), 1_100)
        .unwrap();
    assert!(bridge.ingress_is_exposed(1_100));
    assert_eq!(
        bridge
            .publisher_client()
            .authority()
            .active_bridge_count(1_100),
        1
    );
}

#[test]
fn bridge_starts_punching_only_after_instruction_or_valid_refresh_state() {
    let mut authority = publisher();
    authority
        .register_bridge(
            gbn_bridge_protocol::BridgeRegister {
                bridge_id: "bridge-relay".into(),
                identity_pub: node_public_key(50),
                ingress_endpoints: vec![BridgeIngressEndpoint {
                    host: "198.51.100.20".into(),
                    port: 443,
                }],
                requested_udp_punch_port: 443,
                capabilities: vec![BridgeCapability::BootstrapSeed],
            },
            ReachabilityClass::Direct,
            900,
        )
        .unwrap();

    let client = InProcessPublisherClient::new(authority);
    let mut bridge = ExitBridgeRuntime::new(
        direct_bridge_config("bridge-seed", 51, "198.51.100.21"),
        client,
    );
    bridge.startup(ReachabilityClass::Direct, 1_000).unwrap();

    let plan = bridge
        .publisher_client_mut()
        .authority_mut()
        .begin_bootstrap(join_request("join-001", "bridge-relay", 52), 2_000)
        .unwrap();

    let probe = bridge
        .begin_publisher_directed_punch(plan.seed_punch.clone(), 2_000)
        .unwrap();
    assert_eq!(
        probe.bootstrap_session_id,
        plan.seed_punch.bootstrap_session_id
    );

    let refresh_probe = bridge
        .begin_refresh_punch(plan.creator_entry.clone(), 2_001)
        .unwrap();
    assert_ne!(
        refresh_probe.bootstrap_session_id,
        probe.bootstrap_session_id
    );

    let invalid_refresh = BootstrapDhtEntry {
        publisher_sig: gbn_bridge_protocol::SignatureBytes(vec![7; 64]),
        ..plan.creator_entry
    };
    assert!(matches!(
        bridge.begin_refresh_punch(invalid_refresh, 2_002),
        Err(RuntimeError::Protocol(
            gbn_bridge_protocol::ProtocolError::InvalidSignature
        ))
    ));
}

#[test]
fn seed_bridge_establishes_acks_and_returns_bootstrap_payload() {
    let mut authority = publisher();
    authority
        .register_bridge(
            gbn_bridge_protocol::BridgeRegister {
                bridge_id: "bridge-relay".into(),
                identity_pub: node_public_key(60),
                ingress_endpoints: vec![BridgeIngressEndpoint {
                    host: "198.51.100.30".into(),
                    port: 443,
                }],
                requested_udp_punch_port: 443,
                capabilities: vec![BridgeCapability::BootstrapSeed],
            },
            ReachabilityClass::Direct,
            900,
        )
        .unwrap();

    let client = InProcessPublisherClient::new(authority);
    let mut bridge = ExitBridgeRuntime::new(
        direct_bridge_config("bridge-seed", 61, "198.51.100.31"),
        client,
    );
    bridge.startup(ReachabilityClass::Direct, 1_000).unwrap();

    let plan = bridge
        .publisher_client_mut()
        .authority_mut()
        .begin_bootstrap(join_request("join-002", "bridge-relay", 62), 2_000)
        .unwrap();
    bridge.remember_bootstrap_chain_id(
        &plan.response.bootstrap_session_id,
        "bridge-runtime-seed-test",
    );
    let ack = bridge
        .receive_next_control_command(2_000)
        .unwrap()
        .expect("seed bridge should receive seed assignment");
    assert_eq!(
        ack.status,
        gbn_bridge_protocol::BridgeCommandAckStatus::Applied
    );

    let probe = bridge
        .active_punch_attempt(&plan.response.bootstrap_session_id)
        .cloned()
        .map(|attempt| gbn_bridge_protocol::BridgePunchProbe {
            bootstrap_session_id: attempt.bootstrap_session_id.clone(),
            source_node_id: bridge.config().bridge_id.clone(),
            source_pub_key: bridge.config().identity_pub.clone(),
            source_ip_addr: bridge.config().ingress_endpoint.host.clone(),
            source_udp_punch_port: bridge.current_lease().unwrap().udp_punch_port,
            probe_nonce: attempt.probe_nonce,
        })
        .unwrap();
    let ack = bridge
        .acknowledge_tunnel(
            &probe.bootstrap_session_id,
            &plan.creator_entry.node_id,
            443,
            probe.probe_nonce,
            2_010,
        )
        .unwrap();
    assert_eq!(ack.acked_probe_nonce, probe.probe_nonce);

    let bridge_set = bridge
        .serve_bridge_set(
            &BridgeSetRequest {
                bootstrap_session_id: plan.response.bootstrap_session_id.clone(),
                creator_id: plan.creator_entry.node_id.clone(),
                requested_bridge_count: 9,
            },
            2_020,
        )
        .unwrap();
    assert_eq!(
        bridge_set.bootstrap_session_id,
        plan.response.bootstrap_session_id
    );
    assert_eq!(bridge_set.bridge_entries.len(), 2);

    let progress = bridge.publisher_client().reported_progress();
    assert_eq!(progress.len(), 2);
    assert_eq!(
        progress[0].stage,
        gbn_bridge_protocol::BootstrapProgressStage::SeedTunnelEstablished
    );
    assert_eq!(
        progress[1].stage,
        gbn_bridge_protocol::BootstrapProgressStage::SeedPayloadReceived
    );
}

#[test]
fn bridge_drops_creator_ingress_when_lease_becomes_invalid() {
    let config = direct_bridge_config("bridge-expire", 71, "198.51.100.40");
    let client = InProcessPublisherClient::new(PublisherAuthority::with_config(
        publisher_signing_key(),
        gbn_bridge_publisher::AuthorityConfig {
            lease_ttl_ms: 100,
            heartbeat_interval_ms: 10,
            ..gbn_bridge_publisher::AuthorityConfig::default()
        },
        gbn_bridge_publisher::AuthorityPolicy::default(),
    ));
    let mut bridge = ExitBridgeRuntime::new(config, client);
    bridge.startup(ReachabilityClass::Direct, 1_000).unwrap();
    assert!(bridge.ingress_is_exposed(1_050));

    let result = bridge.heartbeat_tick(0, 1_101);
    assert!(matches!(
        result,
        Err(RuntimeError::Authority(
            gbn_bridge_publisher::AuthorityError::LeaseExpired { .. }
        ))
    ));
    assert!(!bridge.ingress_is_exposed(1_101));
}

#[test]
fn bridge_reconnects_and_reregisters_after_publisher_restart() {
    let config = direct_bridge_config("bridge-reregister", 81, "198.51.100.50");
    let client = InProcessPublisherClient::new(publisher());
    let mut bridge = ExitBridgeRuntime::new(config, client);
    let original_lease = bridge.startup(ReachabilityClass::Direct, 1_000).unwrap();

    bridge.publisher_client_mut().replace_authority(publisher());
    let renewed = bridge.heartbeat_tick(0, 6_100).unwrap().unwrap();
    assert_eq!(renewed.issued_at_ms, 6_100);
    assert_eq!(
        bridge
            .publisher_client()
            .authority()
            .active_bridge_count(6_100),
        1
    );
    assert_eq!(original_lease.bridge_id, renewed.bridge_id);
}

#[test]
fn brokered_bridge_never_exposes_ingress() {
    let config = direct_bridge_config("bridge-brokered", 91, "198.51.100.60");
    let client = InProcessPublisherClient::new(publisher());
    let mut bridge = ExitBridgeRuntime::new(config, client);
    bridge.startup(ReachabilityClass::Brokered, 1_000).unwrap();

    assert!(!bridge.ingress_is_exposed(1_001));
    assert!(matches!(
        bridge.forward_creator_frame(
            BridgeData {
                session_id: "session-001".into(),
                frame_id: "frame-001".into(),
                sequence: 1,
                sent_at_ms: 1_001,
                ciphertext: vec![1, 2, 3],
                final_frame: false,
            },
            1_001,
        ),
        Err(RuntimeError::NonDirectReachability { .. })
    ));
}
