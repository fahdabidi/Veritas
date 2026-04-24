use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BridgeCapability, BridgeIngressEndpoint, PublicKeyBytes, ReachabilityClass,
    RefreshHintReason,
};
use gbn_bridge_publisher::{AuthorityServer, PublisherAuthority, PublisherServiceConfig};
use gbn_bridge_runtime::{
    establish_seed_tunnel, fetch_bridge_set, request_first_contact, BridgeControlClient,
    CreatorConfig, CreatorRuntime, ExitBridgeConfig, ExitBridgeRuntime, HostCreator,
    HostCreatorClient, HttpJsonTransport, HttpTransportConfig, PublisherApiClient,
};
use std::time::{SystemTime, UNIX_EPOCH};

fn publisher_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[12_u8; 32])
}

fn actor_signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn node_public_key(seed: u8) -> PublicKeyBytes {
    publisher_identity(&actor_signing_key(seed))
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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn transport(handle: &gbn_bridge_publisher::AuthorityServerHandle) -> HttpJsonTransport {
    HttpJsonTransport::new(HttpTransportConfig::new(format!(
        "http://{}",
        handle.local_addr()
    )))
    .unwrap()
}

fn control_url(handle: &gbn_bridge_publisher::AuthorityServerHandle) -> String {
    format!("ws://{}/v1/bridge/control", handle.local_addr())
}

fn default_capabilities() -> Vec<BridgeCapability> {
    vec![
        BridgeCapability::BootstrapSeed,
        BridgeCapability::CatalogRefresh,
        BridgeCapability::SessionRelay,
        BridgeCapability::BatchAssignment,
        BridgeCapability::ProgressReporting,
    ]
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
        capabilities: default_capabilities(),
    }
}

#[test]
fn bridge_runtime_startup_and_heartbeat_use_network_client() {
    let (handle, publisher_pub) = authority_server();
    let client = PublisherApiClient::new(
        "bridge-net-01",
        actor_signing_key(41),
        publisher_pub,
        transport(&handle),
    );
    let mut bridge =
        ExitBridgeRuntime::new(bridge_config("bridge-net-01", 41, "198.51.100.41"), client);

    let startup_now_ms = now_ms();
    let lease = bridge
        .startup(ReachabilityClass::Direct, startup_now_ms)
        .unwrap();
    assert_eq!(lease.bridge_id, "bridge-net-01");
    assert!(!bridge.has_simulation_publisher_client());
    assert!(bridge.ingress_is_exposed(startup_now_ms));

    let heartbeat_now_ms = now_ms();
    let renewed = bridge.heartbeat_tick(0, heartbeat_now_ms).unwrap().unwrap();
    assert_eq!(renewed.bridge_id, "bridge-net-01");

    handle.join().unwrap();
}

#[test]
fn creator_and_host_creator_can_bootstrap_over_network_clients_without_simulation() {
    let (handle, publisher_pub) = authority_server();

    let relay_client = PublisherApiClient::new(
        "bridge-relay",
        actor_signing_key(51),
        publisher_pub.clone(),
        transport(&handle),
    );
    let seed_client = PublisherApiClient::new(
        "bridge-seed",
        actor_signing_key(52),
        publisher_pub.clone(),
        transport(&handle),
    );
    let extra_client = PublisherApiClient::new(
        "bridge-extra",
        actor_signing_key(53),
        publisher_pub.clone(),
        transport(&handle),
    );

    let mut relay_bridge = ExitBridgeRuntime::new(
        bridge_config("bridge-relay", 51, "198.51.100.51"),
        relay_client,
    );
    let mut seed_bridge = ExitBridgeRuntime::new(
        bridge_config("bridge-seed", 52, "198.51.100.52"),
        seed_client,
    );
    let mut extra_bridge = ExitBridgeRuntime::new(
        bridge_config("bridge-extra", 53, "198.51.100.53"),
        extra_client,
    );
    let relay_startup_now_ms = now_ms();
    relay_bridge
        .startup(ReachabilityClass::Direct, relay_startup_now_ms)
        .unwrap();
    let seed_startup_now_ms = now_ms();
    seed_bridge
        .startup(ReachabilityClass::Direct, seed_startup_now_ms)
        .unwrap();
    let extra_startup_now_ms = now_ms();
    extra_bridge
        .startup(ReachabilityClass::Direct, extra_startup_now_ms)
        .unwrap();
    let control_url = control_url(&handle);
    seed_bridge.attach_control_client(
        BridgeControlClient::connect(
            &control_url,
            "bridge-seed",
            &seed_bridge.current_lease().unwrap().lease_id,
            &node_public_key(52),
            &actor_signing_key(52),
            &publisher_pub.clone(),
            "control-chain-seed-001",
            "control-hello-seed-001",
            now_ms(),
            None,
            30_000,
        )
        .unwrap(),
    );
    extra_bridge.attach_control_client(
        BridgeControlClient::connect(
            &control_url,
            "bridge-extra",
            &extra_bridge.current_lease().unwrap().lease_id,
            &node_public_key(53),
            &actor_signing_key(53),
            &publisher_pub.clone(),
            "control-chain-extra-001",
            "control-hello-extra-001",
            now_ms(),
            None,
            30_000,
        )
        .unwrap(),
    );

    let mut creator = CreatorRuntime::new(CreatorConfig {
        creator_id: "creator-net-01".into(),
        ip_addr: "203.0.113.70".into(),
        pub_key: node_public_key(61),
        udp_punch_port: 443,
    });
    creator.attach_publisher_client(PublisherApiClient::new(
        "creator-net-01",
        actor_signing_key(61),
        publisher_pub.clone(),
        transport(&handle),
    ));

    let mut host_creator = HostCreator::new("host-creator-01");
    host_creator
        .attach_client(
            HostCreatorClient::new(
                "host-creator-01",
                PublisherApiClient::new(
                    "host-creator-01",
                    actor_signing_key(62),
                    publisher_pub,
                    transport(&handle),
                ),
            )
            .unwrap(),
        )
        .unwrap();

    let plan = request_first_contact(
        &mut creator,
        &mut host_creator,
        &mut relay_bridge,
        "join-net-01",
        now_ms(),
    )
    .unwrap();

    assert_eq!(
        plan.chain_id,
        "bootstrap-host-creator-01-join-net-01".to_string()
    );
    let selected_seed_bridge_id = plan.reply.response.seed_bridge.node_id.clone();
    assert_ne!(selected_seed_bridge_id, "bridge-relay");
    assert!(!relay_bridge.has_simulation_publisher_client());
    assert!(!seed_bridge.has_simulation_publisher_client());

    let seed_tunnel = if selected_seed_bridge_id == "bridge-seed" {
        establish_seed_tunnel(&mut creator, &mut seed_bridge, &plan, now_ms()).unwrap()
    } else {
        establish_seed_tunnel(&mut creator, &mut extra_bridge, &plan, now_ms()).unwrap()
    };
    assert_eq!(seed_tunnel.bridge_ack.responder_node_id, "creator-net-01");

    let bridge_set = if selected_seed_bridge_id == "bridge-seed" {
        fetch_bridge_set(&mut creator, &mut seed_bridge, &plan, now_ms()).unwrap()
    } else {
        fetch_bridge_set(&mut creator, &mut extra_bridge, &plan, now_ms()).unwrap()
    };
    assert_eq!(
        bridge_set.bootstrap_session_id,
        plan.reply.response.bootstrap_session_id
    );
    let follow_on_ack = if selected_seed_bridge_id == "bridge-seed" {
        extra_bridge.receive_next_control_command(now_ms()).unwrap()
    } else {
        seed_bridge.receive_next_control_command(now_ms()).unwrap()
    }
    .expect("publisher should activate remaining bridge fanout");
    assert_eq!(
        follow_on_ack.status,
        gbn_bridge_protocol::BridgeCommandAckStatus::Applied
    );

    let attempts = creator
        .begin_bootstrap_fanout(
            &plan.reply.response.bootstrap_session_id,
            &bridge_set,
            now_ms(),
        )
        .unwrap();
    let expected_fanout_target = if selected_seed_bridge_id == "bridge-seed" {
        "bridge-extra"
    } else {
        "bridge-seed"
    };
    assert!(attempts
        .iter()
        .any(|attempt| attempt.target_node_id == expected_fanout_target));

    let refreshed = creator
        .refresh_catalog_via_bridge(&mut relay_bridge, RefreshHintReason::Startup, now_ms())
        .unwrap();
    assert!(refreshed
        .bridges
        .iter()
        .any(|bridge| bridge.bridge_id == "bridge-seed"));

    handle.join().unwrap();
}
