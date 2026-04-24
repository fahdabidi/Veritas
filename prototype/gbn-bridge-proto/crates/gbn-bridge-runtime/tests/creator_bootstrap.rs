use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BridgeCapability, BridgeCatalogRequest, BridgeIngressEndpoint,
    BridgeRegister, PublicKeyBytes, ReachabilityClass, RefreshHintReason,
};
use gbn_bridge_publisher::PublisherAuthority;
use gbn_bridge_runtime::{
    establish_seed_tunnel, fetch_bridge_set, request_first_contact, CreatorConfig, CreatorRuntime,
    ExitBridgeConfig, ExitBridgeRuntime, HostCreator, InProcessPublisherClient, RuntimeError,
};

fn publisher_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[41_u8; 32])
}

fn publisher() -> PublisherAuthority {
    PublisherAuthority::new(publisher_signing_key())
}

fn node_public_key(seed: u8) -> PublicKeyBytes {
    publisher_identity(&SigningKey::from_bytes(&[seed; 32]))
}

fn bridge_register(bridge_id: &str, key_seed: u8, host: &str) -> BridgeRegister {
    BridgeRegister {
        bridge_id: bridge_id.into(),
        identity_pub: node_public_key(key_seed),
        ingress_endpoints: vec![BridgeIngressEndpoint {
            host: host.into(),
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

fn bridge_config(bridge_id: &str, key_seed: u8, host: &str) -> ExitBridgeConfig {
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

fn startup_bridge(bridge_id: &str, key_seed: u8, host: &str, now_ms: u64) -> ExitBridgeRuntime {
    let client = InProcessPublisherClient::new(publisher());
    let mut runtime = ExitBridgeRuntime::new(bridge_config(bridge_id, key_seed, host), client);
    runtime.startup(ReachabilityClass::Direct, now_ms).unwrap();
    runtime
}

fn creator_runtime(creator_id: &str, key_seed: u8, host: &str) -> CreatorRuntime {
    CreatorRuntime::new(CreatorConfig {
        creator_id: creator_id.into(),
        ip_addr: host.into(),
        pub_key: node_public_key(key_seed),
        udp_punch_port: 443,
    })
}

#[test]
fn returning_creator_refresh_uses_cached_catalog_and_retries_next_bridge() {
    let mut authority = publisher();
    authority
        .register_bridge(
            bridge_register("bridge-a", 51, "198.51.100.10"),
            ReachabilityClass::Direct,
            1_000,
        )
        .unwrap();
    authority
        .register_bridge(
            bridge_register("bridge-b", 52, "198.51.100.11"),
            ReachabilityClass::Direct,
            1_500,
        )
        .unwrap();

    let catalog = authority
        .issue_catalog(
            &BridgeCatalogRequest {
                creator_id: "creator-refresh".into(),
                known_catalog_id: None,
                direct_only: true,
                refresh_hint: None,
            },
            2_000,
        )
        .unwrap();

    let mut creator = creator_runtime("creator-refresh", 61, "203.0.113.10");
    creator
        .load_publisher_trust_root(authority.publisher_public_key().clone())
        .unwrap();
    creator.ingest_catalog(catalog, 2_000).unwrap();

    let selected = creator.select_refresh_bridge(2_000).unwrap();
    assert_eq!(selected.bridge_id, "bridge-b");

    creator.record_refresh_failure("bridge-b");
    let retry = creator.select_refresh_bridge(2_000).unwrap();
    assert_eq!(retry.bridge_id, "bridge-a");

    let mut relay_bridge = startup_bridge("bridge-a", 51, "198.51.100.10", 2_050);
    let refreshed = creator
        .refresh_catalog_via_bridge(&mut relay_bridge, RefreshHintReason::BridgeFailure, 2_100)
        .unwrap();
    assert!(refreshed
        .bridges
        .iter()
        .any(|bridge| bridge.bridge_id == "bridge-a"));

    let fanout = creator.begin_refresh_fanout(2_100).unwrap();
    assert_eq!(fanout.len(), 1);
    assert_eq!(fanout[0].target_node_id, "bridge-a");
    assert_eq!(creator.local_dht().len(), 2);
    assert!(creator.local_dht().node("bridge-a").is_some());
}

#[test]
fn creator_rejects_invalid_signature_and_expired_cached_catalogs() {
    let mut authority = PublisherAuthority::with_config(
        publisher_signing_key(),
        gbn_bridge_publisher::AuthorityConfig {
            catalog_ttl_ms: 100,
            ..gbn_bridge_publisher::AuthorityConfig::default()
        },
        gbn_bridge_publisher::AuthorityPolicy::default(),
    );
    authority
        .register_bridge(
            bridge_register("bridge-expiring", 71, "198.51.100.20"),
            ReachabilityClass::Direct,
            1_000,
        )
        .unwrap();

    let catalog = authority
        .issue_catalog(
            &BridgeCatalogRequest {
                creator_id: "creator-expiring".into(),
                known_catalog_id: None,
                direct_only: true,
                refresh_hint: None,
            },
            1_050,
        )
        .unwrap();

    let mut creator = creator_runtime("creator-expiring", 72, "203.0.113.20");
    creator
        .load_publisher_trust_root(authority.publisher_public_key().clone())
        .unwrap();

    let mut tampered = catalog.clone();
    tampered.publisher_sig = gbn_bridge_protocol::SignatureBytes(vec![9; 64]);
    assert!(matches!(
        creator.ingest_catalog(tampered, 1_051),
        Err(RuntimeError::Protocol(
            gbn_bridge_protocol::ProtocolError::InvalidSignature
        ))
    ));

    creator.ingest_catalog(catalog, 1_051).unwrap();
    assert!(matches!(
        creator.select_refresh_bridge(1_151),
        Err(RuntimeError::Protocol(
            gbn_bridge_protocol::ProtocolError::Expired { .. }
        ))
    ));
}

#[test]
fn host_creator_bootstrap_establishes_seed_tunnel_and_updates_local_dht() {
    let relay_client = InProcessPublisherClient::new(publisher());
    let seed_client = relay_client.clone();
    let extra_client = relay_client.clone();
    let mut relay_bridge = ExitBridgeRuntime::new(
        bridge_config("bridge-relay", 83, "198.51.100.32"),
        relay_client,
    );
    relay_bridge
        .startup(ReachabilityClass::Direct, 1_500)
        .unwrap();

    let mut seed_bridge = ExitBridgeRuntime::new(
        bridge_config("bridge-a-seed", 81, "198.51.100.30"),
        seed_client,
    );
    seed_bridge
        .startup(ReachabilityClass::Direct, 1_500)
        .unwrap();
    let mut extra_bridge = ExitBridgeRuntime::new(
        bridge_config("bridge-z-extra", 82, "198.51.100.31"),
        extra_client,
    );
    extra_bridge
        .startup(ReachabilityClass::Direct, 1_500)
        .unwrap();

    let mut creator = creator_runtime("creator-join-001", 84, "203.0.113.30");
    let mut host_creator = HostCreator::new("host-creator-01");

    let plan = request_first_contact(
        &mut creator,
        &mut host_creator,
        &mut relay_bridge,
        "join-001",
        2_000,
    )
    .unwrap();
    assert_eq!(plan.reply.response.seed_bridge.node_id, "bridge-a-seed");
    assert!(creator.publisher_trust_root().is_some());
    assert_eq!(creator.self_entry().unwrap().node_id, "creator-join-001");
    assert!(creator.local_dht().node("bridge-a-seed").is_some());

    let seed_tunnel = establish_seed_tunnel(&mut creator, &mut seed_bridge, &plan, 2_010).unwrap();
    assert_eq!(seed_tunnel.probe.source_node_id, "bridge-a-seed");
    assert_eq!(seed_tunnel.bridge_ack.responder_node_id, "creator-join-001");
    assert_eq!(
        creator
            .local_dht()
            .node("bridge-a-seed")
            .and_then(|node| node.active_tunnel_since_ms),
        Some(2_010)
    );

    let bridge_set = fetch_bridge_set(&mut creator, &mut seed_bridge, &plan, 2_020).unwrap();
    assert_eq!(
        bridge_set.bootstrap_session_id,
        plan.reply.response.bootstrap_session_id
    );
    assert!(creator.local_dht().node("bridge-z-extra").is_some());

    let attempts = creator
        .begin_bootstrap_fanout(
            &plan.reply.response.bootstrap_session_id,
            &bridge_set,
            2_030,
        )
        .unwrap();
    let extra_attempt = attempts
        .iter()
        .find(|attempt| attempt.target_node_id == "bridge-z-extra")
        .unwrap()
        .clone();
    let probe = extra_bridge
        .begin_refresh_punch(creator.self_entry().unwrap().clone(), 2_031)
        .unwrap();
    let creator_ack = creator.acknowledge_tunnel(&extra_attempt, 2_040).unwrap();
    let bridge_ack = extra_bridge
        .acknowledge_tunnel(
            &probe.bootstrap_session_id,
            "creator-join-001",
            443,
            probe.probe_nonce,
            2_040,
        )
        .unwrap();

    assert_eq!(creator_ack.target_node_id, "bridge-z-extra");
    assert_eq!(bridge_ack.responder_node_id, "creator-join-001");
    assert_eq!(
        creator
            .local_dht()
            .node("bridge-z-extra")
            .and_then(|node| node.active_tunnel_since_ms),
        Some(2_040)
    );
}

#[test]
fn creator_rejects_tampered_bridge_sets() {
    let relay_client = InProcessPublisherClient::new(publisher());
    let seed_client = relay_client.clone();
    let mut relay_bridge = ExitBridgeRuntime::new(
        bridge_config("bridge-relay", 92, "198.51.100.41"),
        relay_client,
    );
    let mut seed_bridge = ExitBridgeRuntime::new(
        bridge_config("bridge-a-seed", 91, "198.51.100.40"),
        seed_client,
    );
    relay_bridge
        .startup(ReachabilityClass::Direct, 1_500)
        .unwrap();
    seed_bridge
        .startup(ReachabilityClass::Direct, 1_500)
        .unwrap();

    let mut creator = creator_runtime("creator-join-002", 93, "203.0.113.40");
    let mut host_creator = HostCreator::new("host-creator-02");
    let plan = request_first_contact(
        &mut creator,
        &mut host_creator,
        &mut relay_bridge,
        "join-002",
        2_000,
    )
    .unwrap();

    establish_seed_tunnel(&mut creator, &mut seed_bridge, &plan, 2_001).unwrap();
    let mut tampered = fetch_bridge_set(&mut creator, &mut seed_bridge, &plan, 2_002).unwrap();
    tampered.publisher_sig = gbn_bridge_protocol::SignatureBytes(vec![7; 64]);

    assert!(matches!(
        creator.store_bridge_set(&tampered, 2_002),
        Err(RuntimeError::Protocol(
            gbn_bridge_protocol::ProtocolError::InvalidSignature
        ))
    ));
}
