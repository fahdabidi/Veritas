use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::bootstrap::{
    BootstrapDhtEntry, BootstrapDhtEntryUnsigned, BridgeSetRequest, BridgeSetResponse,
    BridgeSetResponseUnsigned, CreatorBootstrapResponse, CreatorBootstrapResponseUnsigned,
    CreatorJoinRequest, PendingCreator,
};
use gbn_bridge_protocol::catalog::{
    BridgeCatalogRequest, BridgeCatalogResponse, BridgeCatalogResponseUnsigned, BridgeRefreshHint,
    RefreshHintReason,
};
use gbn_bridge_protocol::descriptor::{
    BridgeCapability, BridgeDescriptor, BridgeDescriptorUnsigned, BridgeIngressEndpoint,
    ReachabilityClass,
};
use gbn_bridge_protocol::lease::{
    BridgeHeartbeat, BridgeLease, BridgeLeaseUnsigned, BridgeRegister, BridgeRevoke,
    BridgeRevokeUnsigned, RevocationReason,
};
use gbn_bridge_protocol::messages::{
    ProtocolEnvelope, ProtocolMessage, ProtocolVersion, ReplayProtection, CURRENT_PROTOCOL_VERSION,
};
use gbn_bridge_protocol::punch::{
    BatchAssignment, BootstrapProgress, BootstrapProgressStage, BridgeBatchAssign,
    BridgeBatchAssignUnsigned, BridgePunchAck, BridgePunchProbe, BridgePunchStart,
    BridgePunchStartUnsigned,
};
use gbn_bridge_protocol::session::{
    BridgeAck, BridgeAckStatus, BridgeClose, BridgeCloseReason, BridgeData, BridgeOpen,
};
use gbn_bridge_protocol::{publisher_identity, ProtocolError, PublicKeyBytes};
use serde::de::DeserializeOwned;
use serde::Serialize;

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7_u8; 32])
}

fn publisher_pub() -> PublicKeyBytes {
    publisher_identity(&signing_key())
}

fn bridge_identity_pub() -> PublicKeyBytes {
    publisher_identity(&SigningKey::from_bytes(&[9_u8; 32]))
}

fn creator_identity_pub() -> PublicKeyBytes {
    publisher_identity(&SigningKey::from_bytes(&[11_u8; 32]))
}

fn sample_endpoint() -> BridgeIngressEndpoint {
    BridgeIngressEndpoint {
        host: "198.51.100.20".into(),
        port: 443,
    }
}

fn sample_bridge_descriptor_unsigned() -> BridgeDescriptorUnsigned {
    BridgeDescriptorUnsigned {
        bridge_id: "bridge-01".into(),
        identity_pub: bridge_identity_pub(),
        ingress_endpoints: vec![sample_endpoint()],
        udp_punch_port: 443,
        reachability_class: ReachabilityClass::Direct,
        lease_expiry_ms: 50_000,
        capabilities: vec![
            BridgeCapability::BootstrapSeed,
            BridgeCapability::CatalogRefresh,
            BridgeCapability::SessionRelay,
        ],
    }
}

fn sample_bridge_descriptor() -> BridgeDescriptor {
    BridgeDescriptor::sign(sample_bridge_descriptor_unsigned(), &signing_key()).unwrap()
}

fn sample_bootstrap_entry_unsigned(
    node_id: &str,
    ip_addr: &str,
    pub_key: PublicKeyBytes,
) -> BootstrapDhtEntryUnsigned {
    BootstrapDhtEntryUnsigned {
        node_id: node_id.into(),
        ip_addr: ip_addr.into(),
        pub_key,
        udp_punch_port: 443,
        entry_expiry_ms: 40_000,
    }
}

fn sample_seed_bridge_entry() -> BootstrapDhtEntry {
    BootstrapDhtEntry::sign(
        sample_bootstrap_entry_unsigned("bridge-01", "198.51.100.20", bridge_identity_pub()),
        &signing_key(),
    )
    .unwrap()
}

fn sample_creator_entry() -> BootstrapDhtEntry {
    BootstrapDhtEntry::sign(
        sample_bootstrap_entry_unsigned("creator-01", "203.0.113.44", creator_identity_pub()),
        &signing_key(),
    )
    .unwrap()
}

fn sample_creator_join_request() -> CreatorJoinRequest {
    CreatorJoinRequest {
        chain_id: "chain-bootstrap-001".into(),
        request_id: "join-001".into(),
        host_creator_id: "host-creator-01".into(),
        relay_bridge_id: "bridge-a".into(),
        creator: PendingCreator {
            node_id: "creator-01".into(),
            ip_addr: "203.0.113.44".into(),
            pub_key: creator_identity_pub(),
            udp_punch_port: 443,
        },
    }
}

fn sample_creator_bootstrap_response() -> CreatorBootstrapResponse {
    CreatorBootstrapResponse::sign(
        CreatorBootstrapResponseUnsigned {
            chain_id: "chain-bootstrap-001".into(),
            bootstrap_session_id: "bootstrap-001".into(),
            seed_bridge: sample_seed_bridge_entry(),
            publisher_pub: publisher_pub(),
            response_expiry_ms: 45_000,
            assigned_bridge_count: 9,
        },
        &signing_key(),
    )
    .unwrap()
}

fn sample_bridge_set_response() -> BridgeSetResponse {
    BridgeSetResponse::sign(
        BridgeSetResponseUnsigned {
            chain_id: "chain-bootstrap-001".into(),
            bootstrap_session_id: "bootstrap-001".into(),
            bridge_entries: vec![sample_seed_bridge_entry(), sample_creator_entry()],
            response_expiry_ms: 45_000,
        },
        &signing_key(),
    )
    .unwrap()
}

fn sample_catalog_response() -> BridgeCatalogResponse {
    BridgeCatalogResponse::sign(
        BridgeCatalogResponseUnsigned {
            catalog_id: "catalog-001".into(),
            issued_at_ms: 10_000,
            expires_at_ms: 45_000,
            bridges: vec![sample_bridge_descriptor()],
        },
        &signing_key(),
    )
    .unwrap()
}

fn sample_bridge_lease() -> BridgeLease {
    BridgeLease::sign(
        BridgeLeaseUnsigned {
            lease_id: "lease-001".into(),
            bridge_id: "bridge-01".into(),
            udp_punch_port: 443,
            reachability_class: ReachabilityClass::Direct,
            lease_expiry_ms: 45_000,
            issued_at_ms: 10_000,
            heartbeat_interval_ms: 5_000,
            capabilities: vec![BridgeCapability::SessionRelay],
        },
        &signing_key(),
    )
    .unwrap()
}

fn sample_bridge_revoke() -> BridgeRevoke {
    BridgeRevoke::sign(
        BridgeRevokeUnsigned {
            lease_id: "lease-001".into(),
            bridge_id: "bridge-01".into(),
            revoked_at_ms: 12_000,
            reason: RevocationReason::PolicyViolation,
        },
        &signing_key(),
    )
    .unwrap()
}

fn sample_bridge_punch_start() -> BridgePunchStart {
    BridgePunchStart::sign(
        BridgePunchStartUnsigned {
            chain_id: "chain-bootstrap-001".into(),
            bootstrap_session_id: "bootstrap-001".into(),
            initiator_id: "bridge-01".into(),
            target: sample_creator_entry(),
            attempt_expiry_ms: 30_000,
        },
        &signing_key(),
    )
    .unwrap()
}

fn sample_bridge_batch_assign() -> BridgeBatchAssign {
    BridgeBatchAssign::sign(
        BridgeBatchAssignUnsigned {
            chain_id: "chain-bootstrap-001".into(),
            batch_id: "batch-001".into(),
            bridge_id: "bridge-01".into(),
            window_started_at_ms: 20_000,
            window_length_ms: 500,
            assignments: vec![BatchAssignment {
                chain_id: "chain-bootstrap-001".into(),
                bootstrap_session_id: "bootstrap-001".into(),
                creator: sample_creator_entry(),
                requested_bridge_count: 9,
            }],
        },
        &signing_key(),
    )
    .unwrap()
}

fn sample_protocol_message() -> ProtocolMessage {
    ProtocolMessage::BridgeCatalogResponse(sample_catalog_response())
}

fn assert_round_trip<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + core::fmt::Debug,
{
    let json = serde_json::to_string(value).unwrap();
    let decoded: T = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, *value);
}

#[test]
fn serde_round_trip_covers_phase_two_types() {
    assert_round_trip(&sample_bridge_descriptor_unsigned());
    assert_round_trip(&sample_bridge_descriptor());
    assert_round_trip(&sample_seed_bridge_entry());
    assert_round_trip(&sample_creator_join_request());
    assert_round_trip(&sample_creator_bootstrap_response());
    assert_round_trip(&BridgeSetRequest {
        chain_id: "chain-bootstrap-001".into(),
        bootstrap_session_id: "bootstrap-001".into(),
        creator_id: "creator-01".into(),
        requested_bridge_count: 9,
    });
    assert_round_trip(&sample_bridge_set_response());
    assert_round_trip(&BridgeCatalogRequest {
        creator_id: "creator-01".into(),
        known_catalog_id: Some("catalog-000".into()),
        direct_only: true,
        refresh_hint: Some(BridgeRefreshHint {
            bridge_id: Some("bridge-01".into()),
            reason: RefreshHintReason::LeaseExpiring,
            last_success_ms: Some(9_000),
            stale_after_ms: Some(15_000),
        }),
    });
    assert_round_trip(&sample_catalog_response());
    assert_round_trip(&BridgeRegister {
        bridge_id: "bridge-01".into(),
        identity_pub: bridge_identity_pub(),
        ingress_endpoints: vec![sample_endpoint()],
        requested_udp_punch_port: 443,
        capabilities: vec![BridgeCapability::SessionRelay],
    });
    assert_round_trip(&sample_bridge_lease());
    assert_round_trip(&BridgeHeartbeat {
        lease_id: "lease-001".into(),
        bridge_id: "bridge-01".into(),
        heartbeat_at_ms: 15_000,
        active_sessions: 2,
        observed_ingress: Some(sample_endpoint()),
    });
    assert_round_trip(&sample_bridge_revoke());
    assert_round_trip(&sample_bridge_punch_start());
    assert_round_trip(&BridgePunchProbe {
        chain_id: "chain-bootstrap-001".into(),
        bootstrap_session_id: "bootstrap-001".into(),
        source_node_id: "bridge-01".into(),
        source_pub_key: bridge_identity_pub(),
        source_ip_addr: "198.51.100.20".into(),
        source_udp_punch_port: 443,
        probe_nonce: 44,
    });
    assert_round_trip(&BridgePunchAck {
        chain_id: "chain-bootstrap-001".into(),
        bootstrap_session_id: "bootstrap-001".into(),
        source_node_id: "bridge-01".into(),
        responder_node_id: "creator-01".into(),
        observed_udp_punch_port: 443,
        acked_probe_nonce: 44,
        established_at_ms: 21_000,
    });
    assert_round_trip(&BootstrapProgress {
        chain_id: "chain-bootstrap-001".into(),
        bootstrap_session_id: "bootstrap-001".into(),
        reporter_id: "bridge-01".into(),
        stage: BootstrapProgressStage::BridgeTunnelEstablished,
        active_bridge_count: 4,
        reported_at_ms: 21_500,
    });
    assert_round_trip(&sample_bridge_batch_assign());
    assert_round_trip(&BridgeOpen {
        chain_id: "chain-upload-001".into(),
        session_id: "session-001".into(),
        creator_id: "creator-01".into(),
        bridge_id: "bridge-01".into(),
        creator_session_pub: creator_identity_pub(),
        opened_at_ms: 30_000,
        expected_chunks: Some(10),
    });
    assert_round_trip(&BridgeData {
        chain_id: "chain-upload-001".into(),
        session_id: "session-001".into(),
        frame_id: "frame-001".into(),
        sequence: 1,
        sent_at_ms: 30_100,
        ciphertext: vec![1, 2, 3, 4],
        final_frame: false,
    });
    assert_round_trip(&BridgeAck {
        chain_id: "chain-upload-001".into(),
        session_id: "session-001".into(),
        acked_sequence: 1,
        status: BridgeAckStatus::Accepted,
        acked_at_ms: 30_200,
    });
    assert_round_trip(&BridgeClose {
        chain_id: "chain-upload-001".into(),
        session_id: "session-001".into(),
        closed_at_ms: 31_000,
        reason: BridgeCloseReason::Completed,
    });
    assert_round_trip(&sample_protocol_message());
    assert_round_trip(&ProtocolEnvelope::with_replay(
        sample_protocol_message(),
        ReplayProtection {
            message_id: "msg-001".into(),
            nonce: 77,
            sent_at_ms: 15_000,
        },
    ));
}

#[test]
fn valid_and_tampered_publisher_signatures_are_detected() {
    let publisher = publisher_pub();
    let descriptor = sample_bridge_descriptor();
    descriptor.verify_authority(&publisher, 20_000).unwrap();

    let mut tampered = descriptor.clone();
    tampered.udp_punch_port = 8443;
    let error = tampered.verify_authority(&publisher, 20_000).unwrap_err();
    assert_eq!(error, ProtocolError::InvalidSignature);
}

#[test]
fn lease_expiry_is_rejected() {
    let lease = sample_bridge_lease();
    let publisher = publisher_pub();
    let error = lease.verify_authority(&publisher, 50_000).unwrap_err();
    assert_eq!(
        error,
        ProtocolError::Expired {
            object: "bridge lease",
            expiry_ms: 45_000,
            now_ms: 50_000,
        }
    );
}

#[test]
fn bootstrap_entry_expiry_and_signature_validation_are_enforced() {
    let entry = sample_seed_bridge_entry();
    let publisher = publisher_pub();
    entry.verify_authority(&publisher, 20_000).unwrap();

    let mut tampered = entry.clone();
    tampered.ip_addr = "198.51.100.99".into();
    assert_eq!(
        tampered.verify_authority(&publisher, 20_000).unwrap_err(),
        ProtocolError::InvalidSignature
    );

    assert_eq!(
        entry.verify_authority(&publisher, 50_000).unwrap_err(),
        ProtocolError::Expired {
            object: "bootstrap entry",
            expiry_ms: 40_000,
            now_ms: 50_000,
        }
    );
}

#[test]
fn udp_punch_messages_round_trip_cleanly() {
    assert_round_trip(&sample_bridge_punch_start());
    assert_round_trip(&BridgePunchProbe {
        chain_id: "chain-bootstrap-001".into(),
        bootstrap_session_id: "bootstrap-001".into(),
        source_node_id: "creator-01".into(),
        source_pub_key: creator_identity_pub(),
        source_ip_addr: "203.0.113.44".into(),
        source_udp_punch_port: 443,
        probe_nonce: 99,
    });
    assert_round_trip(&BridgePunchAck {
        chain_id: "chain-bootstrap-001".into(),
        bootstrap_session_id: "bootstrap-001".into(),
        source_node_id: "creator-01".into(),
        responder_node_id: "bridge-01".into(),
        observed_udp_punch_port: 443,
        acked_probe_nonce: 99,
        established_at_ms: 21_000,
    });
}

#[test]
fn protocol_version_mismatch_is_rejected() {
    let envelope = ProtocolEnvelope {
        version: ProtocolVersion(99),
        replay: Some(ReplayProtection {
            message_id: "msg-001".into(),
            nonce: 1,
            sent_at_ms: 10_000,
        }),
        body: sample_protocol_message(),
    };

    let error = envelope.validate(12_000, 5_000).unwrap_err();
    assert_eq!(
        error,
        ProtocolError::UnsupportedProtocolVersion {
            actual: 99,
            expected: CURRENT_PROTOCOL_VERSION.0,
        }
    );
}

#[test]
fn replay_window_validation_rejects_stale_envelopes() {
    let envelope = ProtocolEnvelope::with_replay(
        sample_protocol_message(),
        ReplayProtection {
            message_id: "msg-002".into(),
            nonce: 55,
            sent_at_ms: 1_000,
        },
    );

    let error = envelope.validate(10_000, 5_000).unwrap_err();
    assert_eq!(
        error,
        ProtocolError::ReplayWindowExpired {
            sent_at_ms: 1_000,
            now_ms: 10_000,
            max_age_ms: 5_000,
        }
    );
}
