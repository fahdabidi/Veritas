use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BootstrapProgress, BootstrapProgressStage, BridgeCapability,
    BridgeCatalogRequest, BridgeClose, BridgeCloseReason, BridgeData, BridgeHeartbeat,
    BridgeIngressEndpoint, BridgeOpen, BridgeRegister, PendingCreator, PublicKeyBytes,
    ReachabilityClass,
};
use gbn_bridge_publisher::{
    AuthorityConfig, AuthorityPolicy, PostgresStorageConfig, PublisherAuthority,
};
use postgres::{Client, NoTls};

fn publisher_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[41_u8; 32])
}

fn bridge_signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn node_public_key(seed: u8) -> PublicKeyBytes {
    publisher_identity(&bridge_signing_key(seed))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn postgres_url() -> String {
    std::env::var("GBN_BRIDGE_TEST_POSTGRES_URL").unwrap_or_else(|_| {
        "host=127.0.0.1 port=5432 user=postgres password=postgres dbname=veritas_proto006".into()
    })
}

fn unique_schema(prefix: &str) -> String {
    format!("{prefix}_{}", now_ms())
}

fn cleanup_schema(schema: &str) {
    let mut client = Client::connect(&postgres_url(), NoTls).unwrap();
    client
        .batch_execute(&format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE;"))
        .unwrap();
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
            BridgeCapability::ProgressReporting,
        ],
    }
}

#[test]
fn postgres_backed_authority_recovers_bridges_bootstrap_catalog_and_upload_sessions() {
    let schema = unique_schema("proto006_phase2");
    let storage_config = PostgresStorageConfig {
        connection_string: postgres_url(),
        schema: schema.clone(),
    };
    let publisher_key = publisher_signing_key();
    let base_now_ms = now_ms();

    let mut authority = PublisherAuthority::with_postgres(
        publisher_key.clone(),
        AuthorityConfig::default(),
        AuthorityPolicy::default(),
        storage_config.clone(),
        base_now_ms,
    )
    .unwrap();
    assert!(authority.storage_is_durable());
    assert_eq!(authority.last_recovery_summary().expired_bridges, 0);

    let bridge_a_lease = authority
        .register_bridge(
            bridge_register("bridge-a", 51, "198.51.100.10", 443),
            ReachabilityClass::Direct,
            base_now_ms,
        )
        .unwrap();
    let bridge_b_lease = authority
        .register_bridge(
            bridge_register("bridge-b", 52, "198.51.100.11", 443),
            ReachabilityClass::Direct,
            base_now_ms,
        )
        .unwrap();
    assert_eq!(bridge_a_lease.bridge_id, "bridge-a");
    assert_eq!(bridge_b_lease.bridge_id, "bridge-b");

    let catalog = authority
        .issue_catalog_with_chain_id(
            Some("catalog-chain-01"),
            &BridgeCatalogRequest {
                creator_id: "creator-a".into(),
                known_catalog_id: None,
                direct_only: false,
                refresh_hint: None,
            },
            base_now_ms,
        )
        .unwrap();

    let bootstrap_plan = authority
        .begin_bootstrap_with_chain_id(
            "bootstrap-chain-01",
            gbn_bridge_protocol::CreatorJoinRequest {
                chain_id: "bootstrap-chain-01".into(),
                request_id: "join-01".into(),
                host_creator_id: "host-a".into(),
                relay_bridge_id: "bridge-a".into(),
                creator: PendingCreator {
                    node_id: "creator-new".into(),
                    ip_addr: "203.0.113.44".into(),
                    pub_key: node_public_key(61),
                    udp_punch_port: 443,
                },
            },
            base_now_ms,
        )
        .unwrap();

    authority
        .report_bootstrap_progress_with_chain_id(
            "bootstrap-chain-01",
            BootstrapProgress {
                chain_id: "bootstrap-chain-01".into(),
                bootstrap_session_id: bootstrap_plan.response.bootstrap_session_id.clone(),
                reporter_id: "bridge-b".into(),
                stage: BootstrapProgressStage::SeedTunnelEstablished,
                active_bridge_count: 1,
                reported_at_ms: base_now_ms + 5,
            },
        )
        .unwrap();

    authority
        .handle_heartbeat(BridgeHeartbeat {
            lease_id: bridge_a_lease.lease_id.clone(),
            bridge_id: "bridge-a".into(),
            heartbeat_at_ms: base_now_ms + 1_000,
            active_sessions: 1,
            observed_ingress: None,
        })
        .unwrap();

    authority
        .open_bridge_session(BridgeOpen {
            chain_id: "upload-chain-01".into(),
            session_id: "session-01".into(),
            creator_id: "creator-a".into(),
            bridge_id: "bridge-a".into(),
            creator_session_pub: node_public_key(71),
            opened_at_ms: base_now_ms + 2_000,
            expected_chunks: Some(1),
        })
        .unwrap();
    authority
        .ingest_bridge_frame(
            "bridge-a",
            BridgeData {
                chain_id: "upload-chain-01".into(),
                session_id: "session-01".into(),
                frame_id: "frame-01".into(),
                sequence: 0,
                sent_at_ms: base_now_ms + 2_001,
                ciphertext: vec![1, 2, 3, 4],
                final_frame: true,
            },
            base_now_ms + 2_002,
        )
        .unwrap();
    authority
        .close_bridge_session(BridgeClose {
            chain_id: "upload-chain-01".into(),
            session_id: "session-01".into(),
            closed_at_ms: base_now_ms + 2_003,
            reason: BridgeCloseReason::Completed,
        })
        .unwrap();

    drop(authority);

    let mut recovered = PublisherAuthority::with_postgres(
        publisher_key,
        AuthorityConfig::default(),
        AuthorityPolicy::default(),
        storage_config,
        base_now_ms + 2_500,
    )
    .unwrap();
    recovered.durable_store_healthcheck().unwrap();

    assert_eq!(recovered.active_bridge_count(base_now_ms + 2_500), 2);

    let recovered_bootstrap = recovered
        .bootstrap_session(&bootstrap_plan.response.bootstrap_session_id)
        .unwrap();
    assert_eq!(recovered_bootstrap.chain_id, "bootstrap-chain-01");
    assert_eq!(recovered_bootstrap.progress_events.len(), 1);
    assert_eq!(
        recovered_bootstrap.progress_events[0].stage,
        BootstrapProgressStage::SeedTunnelEstablished
    );

    let recovered_catalog = recovered.catalog_issuance(&catalog.catalog_id).unwrap();
    assert_eq!(
        recovered_catalog.chain_id.as_deref(),
        Some("catalog-chain-01")
    );
    assert_eq!(recovered_catalog.response.catalog_id, catalog.catalog_id);

    let recovered_upload = recovered.upload_session("session-01").unwrap();
    assert_eq!(recovered_upload.creator_id, "creator-a");
    assert_eq!(recovered_upload.frames_by_sequence.len(), 1);
    assert_eq!(
        recovered_upload.close_reason,
        Some(BridgeCloseReason::Completed)
    );
    assert_eq!(
        recovered_upload
            .frames_by_sequence
            .get(&0)
            .unwrap()
            .frame
            .frame_id,
        "frame-01"
    );

    cleanup_schema(&schema);
}
