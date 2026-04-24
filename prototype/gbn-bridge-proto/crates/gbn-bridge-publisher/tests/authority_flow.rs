use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BridgeCapability, BridgeCatalogRequest, BridgeHeartbeat,
    BridgeIngressEndpoint, BridgeRegister, CreatorJoinRequest, PendingCreator, ProtocolError,
    ReachabilityClass, RevocationReason,
};
use gbn_bridge_publisher::{AuthorityConfig, AuthorityError, AuthorityPolicy, PublisherAuthority};

fn publisher_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[21_u8; 32])
}

fn node_public_key(seed: u8) -> gbn_bridge_protocol::PublicKeyBytes {
    publisher_identity(&SigningKey::from_bytes(&[seed; 32]))
}

fn authority() -> PublisherAuthority {
    PublisherAuthority::new(publisher_signing_key())
}

fn authority_with_config(config: AuthorityConfig) -> PublisherAuthority {
    PublisherAuthority::with_config(publisher_signing_key(), config, AuthorityPolicy::default())
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
fn registration_rejection_and_heartbeat_renewal_work() {
    let mut authority = authority();

    let invalid = BridgeRegister {
        bridge_id: "bridge-invalid".into(),
        identity_pub: node_public_key(41),
        ingress_endpoints: Vec::new(),
        requested_udp_punch_port: 0,
        capabilities: vec![BridgeCapability::SessionRelay],
    };
    let error = authority
        .register_bridge(invalid, ReachabilityClass::Direct, 1_000)
        .unwrap_err();
    assert_eq!(
        error,
        AuthorityError::InvalidBridgeRegistration {
            reason: "bridge ingress endpoints are required",
        }
    );

    let lease = authority
        .register_bridge(
            bridge_register("bridge-01", 42, "198.51.100.10", 0),
            ReachabilityClass::Direct,
            1_500,
        )
        .unwrap();
    assert_eq!(lease.udp_punch_port, 443);
    lease
        .verify_authority(authority.publisher_public_key(), 1_600)
        .unwrap();

    let renewed = authority
        .handle_heartbeat(BridgeHeartbeat {
            lease_id: lease.lease_id.clone(),
            bridge_id: "bridge-01".into(),
            heartbeat_at_ms: 6_000,
            active_sessions: 3,
            observed_ingress: None,
        })
        .unwrap();
    assert!(renewed.lease_expiry_ms > lease.lease_expiry_ms);

    let metrics = authority.metrics_snapshot();
    assert_eq!(metrics.successful_registrations, 1);
    assert_eq!(metrics.rejected_registrations, 1);
    assert_eq!(metrics.heartbeats, 1);
    assert_eq!(authority.active_bridge_count(6_000), 1);
}

#[test]
fn heartbeat_after_expiry_is_rejected() {
    let mut authority = authority_with_config(AuthorityConfig {
        lease_ttl_ms: 100,
        ..AuthorityConfig::default()
    });

    let lease = authority
        .register_bridge(
            bridge_register("bridge-expiring", 50, "198.51.100.20", 443),
            ReachabilityClass::Direct,
            1_000,
        )
        .unwrap();

    let error = authority
        .handle_heartbeat(BridgeHeartbeat {
            lease_id: lease.lease_id.clone(),
            bridge_id: "bridge-expiring".into(),
            heartbeat_at_ms: 1_101,
            active_sessions: 0,
            observed_ingress: None,
        })
        .unwrap_err();

    assert_eq!(
        error,
        AuthorityError::LeaseExpired {
            bridge_id: "bridge-expiring".into(),
            lease_id: lease.lease_id,
            lease_expiry_ms: 1_100,
            heartbeat_at_ms: 1_101,
        }
    );
}

#[test]
fn expired_and_revoked_bridges_are_filtered_from_catalogs() {
    let mut authority = authority_with_config(AuthorityConfig {
        lease_ttl_ms: 500,
        catalog_ttl_ms: 200,
        ..AuthorityConfig::default()
    });

    authority
        .register_bridge(
            bridge_register("bridge-active", 61, "198.51.100.30", 443),
            ReachabilityClass::Direct,
            1_000,
        )
        .unwrap();
    authority
        .register_bridge(
            bridge_register("bridge-revoked", 62, "198.51.100.31", 443),
            ReachabilityClass::Direct,
            1_000,
        )
        .unwrap();
    authority
        .revoke_bridge("bridge-revoked", RevocationReason::OperatorDisabled, 1_100)
        .unwrap();

    let catalog = authority
        .issue_catalog(
            &BridgeCatalogRequest {
                creator_id: "creator-01".into(),
                known_catalog_id: None,
                direct_only: false,
                refresh_hint: None,
            },
            1_200,
        )
        .unwrap();
    catalog
        .verify_authority(authority.publisher_public_key(), 1_200)
        .unwrap();
    assert_eq!(catalog.bridges.len(), 1);
    assert_eq!(catalog.bridges[0].bridge_id, "bridge-active");

    let empty_catalog = authority
        .issue_catalog(
            &BridgeCatalogRequest {
                creator_id: "creator-01".into(),
                known_catalog_id: Some(catalog.catalog_id.clone()),
                direct_only: false,
                refresh_hint: None,
            },
            1_600,
        )
        .unwrap();
    empty_catalog
        .verify_authority(authority.publisher_public_key(), 1_600)
        .unwrap();
    assert!(empty_catalog.bridges.is_empty());
}

#[test]
fn tampered_catalogs_fail_authority_verification() {
    let mut authority = authority();
    authority
        .register_bridge(
            bridge_register("bridge-verify", 71, "198.51.100.40", 443),
            ReachabilityClass::Direct,
            1_000,
        )
        .unwrap();

    let catalog = authority
        .issue_catalog(
            &BridgeCatalogRequest {
                creator_id: "creator-verify".into(),
                known_catalog_id: None,
                direct_only: false,
                refresh_hint: None,
            },
            1_200,
        )
        .unwrap();

    let mut tampered = catalog.clone();
    tampered.bridges[0].udp_punch_port = 8443;

    assert_eq!(
        tampered
            .verify_authority(authority.publisher_public_key(), 1_200)
            .unwrap_err(),
        ProtocolError::InvalidSignature
    );
}

#[test]
fn bootstrap_selects_a_direct_seed_bridge_and_signs_outputs() {
    let mut authority = authority();
    authority
        .register_bridge(
            bridge_register("bridge-relay", 81, "198.51.100.50", 443),
            ReachabilityClass::Direct,
            1_000,
        )
        .unwrap();
    authority
        .register_bridge(
            bridge_register("bridge-seed", 82, "198.51.100.51", 443),
            ReachabilityClass::Direct,
            1_000,
        )
        .unwrap();
    authority
        .register_bridge(
            bridge_register("bridge-brokered", 83, "198.51.100.52", 443),
            ReachabilityClass::Brokered,
            1_000,
        )
        .unwrap();

    let plan = authority
        .begin_bootstrap(creator_join_request("join-001", "bridge-relay", 84), 2_000)
        .unwrap();

    plan.response
        .verify_authority(authority.publisher_public_key(), 2_000)
        .unwrap();
    plan.bridge_set
        .verify_authority(authority.publisher_public_key(), 2_000)
        .unwrap();
    plan.seed_punch
        .verify_authority(authority.publisher_public_key(), 2_000)
        .unwrap();

    assert_eq!(plan.response.seed_bridge.node_id, "bridge-seed");
    assert_eq!(plan.seed_punch.initiator_id, "bridge-seed");
    assert_eq!(plan.response.assigned_bridge_count, 2);
    assert_eq!(plan.bridge_set.bridge_entries.len(), 2);
    assert!(plan
        .bridge_set
        .bridge_entries
        .iter()
        .all(|entry| entry.node_id != "bridge-brokered"));

    let metrics = authority.metrics_snapshot();
    assert_eq!(metrics.bootstrap_requests, 1);
    assert_eq!(metrics.rejected_bootstrap_requests, 0);
}

#[test]
fn bootstrap_rejects_without_any_direct_bridge() {
    let mut authority = authority();
    authority
        .register_bridge(
            bridge_register("bridge-brokered", 91, "198.51.100.60", 443),
            ReachabilityClass::Brokered,
            1_000,
        )
        .unwrap();

    let error = authority
        .begin_bootstrap(
            creator_join_request("join-002", "bridge-brokered", 92),
            2_000,
        )
        .unwrap_err();
    assert_eq!(error, AuthorityError::NoEligibleBootstrapBridge);

    let metrics = authority.metrics_snapshot();
    assert_eq!(metrics.bootstrap_requests, 1);
    assert_eq!(metrics.rejected_bootstrap_requests, 1);
}

#[test]
fn eleventh_join_request_rolls_into_the_next_batch() {
    let mut authority = authority();
    authority
        .register_bridge(
            bridge_register("bridge-01", 101, "198.51.100.70", 443),
            ReachabilityClass::Direct,
            1_000,
        )
        .unwrap();
    authority
        .register_bridge(
            bridge_register("bridge-02", 102, "198.51.100.71", 443),
            ReachabilityClass::Direct,
            1_000,
        )
        .unwrap();

    for index in 0..10 {
        let result = authority
            .enqueue_join_request_for_batch(
                creator_join_request(&format!("join-{index:03}"), "bridge-01", 110 + index as u8),
                5_000,
            )
            .unwrap();
        assert!(result.is_none());
    }
    assert_eq!(authority.current_batch_size(), 10);

    let finalized = authority
        .enqueue_join_request_for_batch(creator_join_request("join-010", "bridge-01", 120), 5_000)
        .unwrap()
        .expect("the 11th join request should flush the first batch");
    assert_eq!(finalized.assignments.len(), 10);
    assert_eq!(finalized.bridge_assignments.len(), 2);
    for assignment in &finalized.bridge_assignments {
        assignment
            .verify_authority(authority.publisher_public_key(), 5_000)
            .unwrap();
    }
    assert_eq!(authority.current_batch_size(), 1);

    let trailing_batch = authority
        .flush_ready_batch(5_500)
        .unwrap()
        .expect("the rollover batch should flush after the 500ms window");
    assert_eq!(trailing_batch.assignments.len(), 1);
    assert_eq!(trailing_batch.bridge_assignments.len(), 2);

    let metrics = authority.metrics_snapshot();
    assert_eq!(metrics.batch_rollovers, 1);
    assert_eq!(metrics.issued_batches, 2);
}
