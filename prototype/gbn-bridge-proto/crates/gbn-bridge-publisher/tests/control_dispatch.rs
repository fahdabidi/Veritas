use std::sync::mpsc;

use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BridgeCapability, BridgeCatalogRequest, BridgeControlHello,
    BridgeControlHelloUnsigned, BridgeControlKeepalive, BridgeIngressEndpoint, BridgeRegister,
    CreatorJoinRequest, PendingCreator, ReachabilityClass, RevocationReason,
};
use gbn_bridge_publisher::{
    AuthorityConfig, AuthorityPolicy, AuthorityService, PublisherAuthority, PublisherServiceConfig,
};

fn publisher_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[91_u8; 32])
}

fn node_public_key(seed: u8) -> gbn_bridge_protocol::PublicKeyBytes {
    publisher_identity(&SigningKey::from_bytes(&[seed; 32]))
}

fn authority() -> PublisherAuthority {
    PublisherAuthority::with_config(
        publisher_signing_key(),
        AuthorityConfig::default(),
        AuthorityPolicy::default(),
    )
}

fn service() -> AuthorityService {
    AuthorityService::new(authority(), &PublisherServiceConfig::default())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn bridge_register(
    bridge_id: &str,
    key_seed: u8,
    host: &str,
    requested_udp_punch_port: u16,
) -> BridgeRegister {
    BridgeRegister {
        bridge_id: bridge_id.into(),
        identity_pub: node_public_key(key_seed),
        ingress_endpoints: vec![BridgeIngressEndpoint {
            host: host.into(),
            port: 443,
        }],
        requested_udp_punch_port,
        capabilities: vec![
            BridgeCapability::BootstrapSeed,
            BridgeCapability::CatalogRefresh,
            BridgeCapability::BatchAssignment,
        ],
    }
}

fn creator_join_request(
    request_id: &str,
    relay_bridge_id: &str,
    key_seed: u8,
) -> CreatorJoinRequest {
    CreatorJoinRequest {
        chain_id: format!("chain-{request_id}"),
        request_id: request_id.into(),
        host_creator_id: "host-creator-01".into(),
        relay_bridge_id: relay_bridge_id.into(),
        creator: PendingCreator {
            node_id: format!("creator-{request_id}"),
            ip_addr: "203.0.113.44".into(),
            pub_key: node_public_key(key_seed),
            udp_punch_port: 443,
        },
    }
}

#[test]
fn authority_queues_control_commands_for_seed_batch_revoke_and_refresh() {
    let mut authority = authority();
    authority
        .register_bridge(
            bridge_register("bridge-seed", 101, "198.51.100.40", 443),
            ReachabilityClass::Direct,
            1_000,
        )
        .unwrap();
    authority
        .register_bridge(
            bridge_register("bridge-relay", 102, "198.51.100.41", 443),
            ReachabilityClass::Direct,
            1_000,
        )
        .unwrap();
    authority
        .register_bridge(
            bridge_register("bridge-batch", 103, "198.51.100.42", 443),
            ReachabilityClass::Direct,
            1_000,
        )
        .unwrap();

    let bootstrap = authority
        .begin_bootstrap_with_chain_id(
            "chain-bootstrap-seed",
            creator_join_request("join-seed", "bridge-relay", 111),
            2_000,
        )
        .unwrap();
    let seed_commands = authority.pending_bridge_commands(&bootstrap.seed_punch.initiator_id);
    assert_eq!(seed_commands.len(), 1);
    assert_eq!(seed_commands[0].chain_id, "chain-bootstrap-seed");
    assert!(matches!(
        seed_commands[0].payload,
        gbn_bridge_protocol::BridgeCommandPayload::SeedAssign(_)
    ));
    assert_eq!(
        bootstrap.response.seed_bridge.node_id,
        seed_commands[0].bridge_id
    );

    for index in 0..11 {
        let _ = authority
            .enqueue_join_request_for_batch_with_chain_id(
                Some(&format!("chain-batch-{index:02}")),
                creator_join_request(
                    &format!("join-batch-{index:02}"),
                    "bridge-relay",
                    120 + index as u8,
                ),
                5_000,
            )
            .unwrap();
    }
    let batch_commands = authority.pending_bridge_commands("bridge-seed");
    assert!(batch_commands.iter().any(|record| matches!(
        record.payload,
        gbn_bridge_protocol::BridgeCommandPayload::BatchAssign(_)
    )));

    let refresh = authority
        .queue_catalog_refresh_notification(
            "bridge-seed",
            "chain-refresh-01",
            &BridgeCatalogRequest {
                creator_id: "creator-refresh".into(),
                known_catalog_id: None,
                direct_only: false,
                refresh_hint: None,
            },
            6_000,
        )
        .unwrap();
    let refresh_commands = authority.pending_bridge_commands("bridge-seed");
    assert!(refresh_commands.iter().any(|record| {
        record.chain_id == "chain-refresh-01"
            && matches!(
                &record.payload,
                gbn_bridge_protocol::BridgeCommandPayload::CatalogRefresh(payload)
                    if payload.catalog_id == refresh.catalog_id
            )
    }));

    let revoke = authority
        .revoke_bridge("bridge-batch", RevocationReason::OperatorDisabled, 7_000)
        .unwrap();
    let revoke_commands = authority.pending_bridge_commands("bridge-batch");
    assert!(revoke_commands.iter().any(|record| matches!(
        &record.payload,
        gbn_bridge_protocol::BridgeCommandPayload::Revoke(payload)
            if payload.lease_id == revoke.lease_id
    )));
}

#[test]
fn latest_control_session_replaces_the_previous_one() {
    let mut service = service();
    service
        .publisher_authority_mut()
        .register_bridge(
            bridge_register("bridge-seed", 121, "198.51.100.50", 443),
            ReachabilityClass::Direct,
            now_ms(),
        )
        .unwrap();
    let bridge_key = SigningKey::from_bytes(&[121_u8; 32]);

    let hello1 = BridgeControlHello::sign(
        BridgeControlHelloUnsigned {
            bridge_id: "bridge-seed".into(),
            lease_id: service
                .publisher_authority()
                .bridge_record("bridge-seed")
                .unwrap()
                .current_lease
                .lease_id
                .clone(),
            bridge_pub: node_public_key(121),
            sent_at_ms: now_ms(),
            request_id: "hello-1".into(),
            resume_acked_seq_no: None,
            chain_id: "control-chain-1".into(),
        },
        &bridge_key,
    )
    .unwrap();
    let (tx1, _rx1) = mpsc::channel();
    let (welcome1, _) = service.accept_control_hello(hello1, tx1).unwrap();

    let hello2 = BridgeControlHello::sign(
        BridgeControlHelloUnsigned {
            bridge_id: "bridge-seed".into(),
            lease_id: service
                .publisher_authority()
                .bridge_record("bridge-seed")
                .unwrap()
                .current_lease
                .lease_id
                .clone(),
            bridge_pub: node_public_key(121),
            sent_at_ms: now_ms(),
            request_id: "hello-2".into(),
            resume_acked_seq_no: None,
            chain_id: "control-chain-2".into(),
        },
        &bridge_key,
    )
    .unwrap();
    let (tx2, _rx2) = mpsc::channel();
    let (welcome2, _) = service.accept_control_hello(hello2, tx2).unwrap();
    assert_ne!(welcome1.session_id, welcome2.session_id);

    let error = service
        .handle_control_keepalive(BridgeControlKeepalive {
            session_id: welcome1.session_id,
            bridge_id: "bridge-seed".into(),
            sent_at_ms: now_ms(),
            chain_id: "control-chain-1".into(),
            last_acked_seq_no: None,
        })
        .unwrap_err();
    assert_eq!(error.code(), "unauthorized");
}
