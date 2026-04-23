use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BootstrapDhtEntry, BootstrapDhtEntryUnsigned, BridgeCapability,
    BridgeCatalogRequest, BridgeIngressEndpoint, CreatorBootstrapResponse,
    CreatorBootstrapResponseUnsigned, PublicKeyBytes, ReachabilityClass, RefreshHintReason,
};
use gbn_bridge_publisher::PublisherAuthority;
use gbn_bridge_runtime::{
    CreatorConfig, CreatorRuntime, DiscoveryHint, DiscoveryHintSource, ExitBridgeConfig,
    ExitBridgeRuntime, InProcessPublisherClient, RefreshCandidateSource, RuntimeError, SeedCatalog,
};

fn publisher_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[91_u8; 32])
}

fn publisher_public_key(signing_key: &SigningKey) -> PublicKeyBytes {
    publisher_identity(signing_key)
}

fn publisher() -> PublisherAuthority {
    PublisherAuthority::new(publisher_signing_key())
}

fn node_public_key(seed: u8) -> PublicKeyBytes {
    publisher_identity(&SigningKey::from_bytes(&[seed; 32]))
}

fn bridge_register(
    bridge_id: &str,
    key_seed: u8,
    host: &str,
) -> gbn_bridge_protocol::BridgeRegister {
    gbn_bridge_protocol::BridgeRegister {
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

fn creator_runtime(creator_id: &str, key_seed: u8, host: &str) -> CreatorRuntime {
    CreatorRuntime::new(CreatorConfig {
        creator_id: creator_id.into(),
        ip_addr: host.into(),
        pub_key: node_public_key(key_seed),
        udp_punch_port: 443,
    })
}

fn weak_hint(
    bridge_id: &str,
    host: &str,
    observed_at_ms: u64,
    source: DiscoveryHintSource,
) -> DiscoveryHint {
    DiscoveryHint {
        bridge_id: bridge_id.into(),
        host: host.into(),
        port: 443,
        observed_at_ms,
        source,
    }
}

fn bootstrap_seed_entry(
    bridge_id: &str,
    key_seed: u8,
    host: &str,
    entry_expiry_ms: u64,
    signing_key: &SigningKey,
) -> BootstrapDhtEntry {
    BootstrapDhtEntry::sign(
        BootstrapDhtEntryUnsigned {
            node_id: bridge_id.into(),
            ip_addr: host.into(),
            pub_key: node_public_key(key_seed),
            udp_punch_port: 443,
            entry_expiry_ms,
        },
        signing_key,
    )
    .unwrap()
}

fn bootstrap_response(
    seed_bridge: BootstrapDhtEntry,
    response_expiry_ms: u64,
    signing_key: &SigningKey,
) -> CreatorBootstrapResponse {
    CreatorBootstrapResponse::sign(
        CreatorBootstrapResponseUnsigned {
            bootstrap_session_id: "bootstrap-001".into(),
            seed_bridge,
            publisher_pub: publisher_public_key(signing_key),
            response_expiry_ms,
            assigned_bridge_count: 1,
        },
        signing_key,
    )
    .unwrap()
}

#[test]
fn weak_discovery_candidate_without_signed_catalog_is_not_transport_eligible() {
    let signing_key = publisher_signing_key();
    let mut creator = creator_runtime("creator-discovery-01", 101, "203.0.113.50");
    creator
        .load_publisher_trust_root(publisher_public_key(&signing_key))
        .unwrap();
    creator.seed_discovery(&SeedCatalog::new(vec![weak_hint(
        "bridge-weak",
        "198.51.100.50",
        2_000,
        DiscoveryHintSource::SeedCatalog,
    )]));

    let candidate = creator.select_refresh_candidate(2_100).unwrap();
    assert_eq!(candidate.bridge_id, "bridge-weak");
    assert!(!candidate.transport_eligible());
    assert!(matches!(
        candidate.source,
        RefreshCandidateSource::WeakDiscovery(DiscoveryHintSource::SeedCatalog)
    ));
    assert!(matches!(
        creator.select_refresh_bridge(2_100),
        Err(RuntimeError::CatalogUnavailable)
    ));
}

#[test]
fn weak_discovery_cannot_override_active_bootstrap_entries_for_new_creator_session() {
    let signing_key = publisher_signing_key();
    let seed_bridge = bootstrap_seed_entry(
        "bridge-bootstrap",
        102,
        "198.51.100.51",
        12_000,
        &signing_key,
    );
    let response = bootstrap_response(seed_bridge, 12_000, &signing_key);

    let mut creator = creator_runtime("creator-discovery-02", 103, "203.0.113.51");
    creator.apply_bootstrap_response(&response, 2_000).unwrap();
    creator.seed_discovery(&SeedCatalog::new(vec![weak_hint(
        "bridge-weak",
        "198.51.100.52",
        2_500,
        DiscoveryHintSource::SeedCatalog,
    )]));

    let candidates = creator.ordered_refresh_candidates(2_500).unwrap();
    assert_eq!(candidates[0].bridge_id, "bridge-bootstrap");
    assert!(candidates[0].transport_eligible());
    assert_eq!(candidates[0].source, RefreshCandidateSource::Bootstrap);
    assert_eq!(candidates[1].bridge_id, "bridge-weak");
    assert!(!candidates[1].transport_eligible());
}

#[test]
fn weak_discovery_can_seed_later_signed_catalog_refresh() {
    let mut authority = publisher();
    authority
        .register_bridge(
            bridge_register("bridge-refresh", 104, "198.51.100.53"),
            ReachabilityClass::Direct,
            1_000,
        )
        .unwrap();

    let shared_client = InProcessPublisherClient::new(authority);
    let mut bridge = ExitBridgeRuntime::new(
        bridge_config("bridge-refresh", 104, "198.51.100.53"),
        shared_client,
    );
    let mut creator = creator_runtime("creator-discovery-03", 105, "203.0.113.52");
    creator
        .load_publisher_trust_root(bridge.publisher_client().publisher_public_key())
        .unwrap();
    creator.ingest_weak_discovery_hints(&[weak_hint(
        "bridge-refresh",
        "198.51.100.53",
        2_000,
        DiscoveryHintSource::WeakDiscovery,
    )]);

    assert!(matches!(
        creator.select_refresh_bridge(2_000),
        Err(RuntimeError::CatalogUnavailable)
    ));

    let candidate = creator.select_refresh_candidate(2_000).unwrap();
    assert_eq!(candidate.bridge_id, "bridge-refresh");
    assert!(!candidate.transport_eligible());

    let refreshed = creator
        .refresh_catalog_via_candidate(
            &candidate,
            &mut bridge,
            RefreshHintReason::ManualRefresh,
            2_100,
        )
        .unwrap();
    assert!(refreshed
        .bridges
        .iter()
        .any(|descriptor| descriptor.bridge_id == "bridge-refresh"));

    let selected = creator.select_refresh_bridge(2_100).unwrap();
    assert_eq!(selected.bridge_id, "bridge-refresh");

    let candidates = creator.ordered_refresh_candidates(2_100).unwrap();
    assert_eq!(candidates[0].bridge_id, "bridge-refresh");
    assert!(candidates[0].transport_eligible());
    assert_eq!(candidates[0].source, RefreshCandidateSource::Catalog);
}

#[test]
fn stale_weak_discovery_does_not_override_fresher_signed_data() {
    let mut authority = publisher();
    authority
        .register_bridge(
            bridge_register("bridge-signed", 106, "198.51.100.54"),
            ReachabilityClass::Direct,
            380_000,
        )
        .unwrap();

    let catalog = authority
        .issue_catalog(
            &BridgeCatalogRequest {
                creator_id: "creator-discovery-04".into(),
                known_catalog_id: None,
                direct_only: true,
                refresh_hint: None,
            },
            390_000,
        )
        .unwrap();

    let mut creator = creator_runtime("creator-discovery-04", 107, "203.0.113.53");
    creator
        .load_publisher_trust_root(authority.publisher_public_key().clone())
        .unwrap();
    creator.ingest_catalog(catalog, 390_000).unwrap();
    creator.ingest_weak_discovery_hints(&[weak_hint(
        "bridge-stale",
        "198.51.100.55",
        1_000,
        DiscoveryHintSource::WeakDiscovery,
    )]);

    let candidates = creator.ordered_refresh_candidates(400_000).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].bridge_id, "bridge-signed");
    assert!(candidates[0].transport_eligible());
}

#[test]
fn creator_still_functions_when_discovery_is_disabled_but_cached_catalog_exists() {
    let mut authority = publisher();
    authority
        .register_bridge(
            bridge_register("bridge-cached", 108, "198.51.100.56"),
            ReachabilityClass::Direct,
            1_000,
        )
        .unwrap();

    let catalog = authority
        .issue_catalog(
            &BridgeCatalogRequest {
                creator_id: "creator-discovery-05".into(),
                known_catalog_id: None,
                direct_only: true,
                refresh_hint: None,
            },
            2_000,
        )
        .unwrap();

    let mut creator = creator_runtime("creator-discovery-05", 109, "203.0.113.54");
    creator
        .load_publisher_trust_root(authority.publisher_public_key().clone())
        .unwrap();
    creator.ingest_catalog(catalog, 2_000).unwrap();
    creator.seed_discovery(&SeedCatalog::new(vec![weak_hint(
        "bridge-ignored",
        "198.51.100.57",
        2_100,
        DiscoveryHintSource::SeedCatalog,
    )]));
    creator.set_discovery_enabled(false);

    let selected = creator.select_refresh_bridge(2_100).unwrap();
    assert_eq!(selected.bridge_id, "bridge-cached");

    let candidates = creator.ordered_refresh_candidates(2_100).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].bridge_id, "bridge-cached");
    assert_eq!(candidates[0].source, RefreshCandidateSource::Catalog);
}
