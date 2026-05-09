use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BridgeDhtEntry, BridgeDhtEntryUnsigned, CreatorDhtEntry,
    CreatorDhtEntryUnsigned, DhtBridgeIngressEndpoint, HostRoleState, LocalDiscoveryTable,
    PublicKeyBytes, PublisherDhtEntry, ReachabilityClass, SelfOnboardingState,
};

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn public_key(seed: u8) -> PublicKeyBytes {
    publisher_identity(&signing_key(seed))
}

fn bridge_entry(
    publisher: &SigningKey,
    lease_expiry_ms: u64,
    entry_expiry_ms: u64,
) -> BridgeDhtEntry {
    BridgeDhtEntry::sign(
        BridgeDhtEntryUnsigned {
            bridge_id: "exit-bridge-0".to_string(),
            identity_pub: public_key(22),
            ingress_endpoints: vec![DhtBridgeIngressEndpoint::direct("10.0.0.10", 4443)],
            udp_punch_port: 4443,
            reachability_class: ReachabilityClass::Direct,
            lease_expiry_ms,
            entry_expiry_ms,
            capabilities: vec!["session_relay".to_string(), "bootstrap_seed".to_string()],
        },
        publisher,
        true,
    )
    .expect("bridge dht entry should sign")
}

fn creator_entry(publisher: &SigningKey, entry_expiry_ms: u64) -> CreatorDhtEntry {
    CreatorDhtEntry::sign(
        CreatorDhtEntryUnsigned {
            node_id: "creator-new".to_string(),
            ip_addr: "10.0.0.20".to_string(),
            pub_key: public_key(33),
            udp_punch_port: 4443,
            entry_expiry_ms,
        },
        publisher,
        true,
    )
    .expect("creator dht entry should sign")
}

#[test]
fn local_discovery_table_round_trips_with_orthogonal_states() {
    let publisher = signing_key(9);
    let publisher_pub = publisher_identity(&publisher);
    let mut table = LocalDiscoveryTable::empty("creator-new", 1_000);
    table.self_onboarding_state = SelfOnboardingState::Onboarded;
    table.host_role_state = HostRoleState::HostSeeded;
    table.publisher_entry = Some(PublisherDhtEntry {
        node_id: "publisher".to_string(),
        authority_url: "http://publisher-authority:8080".to_string(),
        receiver_url: "http://publisher-receiver:8081".to_string(),
        pub_key: publisher_pub,
        encryption_pub_key: None,
        entry_expiry_ms: 10_000,
    });
    table.creator_entry = Some(creator_entry(&publisher, 10_000));
    table.bridge_entries = vec![bridge_entry(&publisher, 10_000, 10_000)];

    let raw = serde_json::to_vec_pretty(&table).expect("table should serialize");
    let decoded: LocalDiscoveryTable =
        serde_json::from_slice(&raw).expect("table should deserialize");

    assert_eq!(decoded, table);
}

#[test]
fn publisher_entry_has_no_publisher_signature_and_uses_trust_root() {
    let publisher_pub = public_key(9);
    let entry = PublisherDhtEntry {
        node_id: "publisher".to_string(),
        authority_url: "http://publisher-authority:8080".to_string(),
        receiver_url: "http://publisher-receiver:8081".to_string(),
        pub_key: publisher_pub.clone(),
        encryption_pub_key: None,
        entry_expiry_ms: 10_000,
    };

    let json = serde_json::to_value(&entry).expect("publisher entry should serialize");
    assert!(json.get("publisher_sig").is_none());
    entry
        .verify_trust_root(&publisher_pub, 1_000)
        .expect("matching trust root should validate");
    assert!(entry.verify_trust_root(&public_key(10), 1_000).is_err());
}

#[test]
fn bridge_dht_entry_validation_enforces_signature_and_expiry_but_allows_suspect_marker() {
    let publisher = signing_key(9);
    let publisher_pub = publisher_identity(&publisher);

    let mut valid = bridge_entry(&publisher, 10_000, 10_000);
    valid.suspect_until_ms = Some(2_000);
    valid
        .verify_authority(&publisher_pub, 1_000)
        .expect("suspect marker should not invalidate storage");
    assert!(!valid.is_route_eligible(1_500));
    assert!(valid.is_route_eligible(2_000));

    let expired_lease = bridge_entry(&publisher, 999, 10_000);
    assert!(expired_lease
        .verify_authority(&publisher_pub, 1_000)
        .is_err());

    let expired_entry = bridge_entry(&publisher, 10_000, 999);
    assert!(expired_entry
        .verify_authority(&publisher_pub, 1_000)
        .is_err());

    let mut bad_sig = bridge_entry(&publisher, 10_000, 10_000);
    bad_sig.capabilities.push("tampered".to_string());
    assert!(bad_sig.verify_authority(&publisher_pub, 1_000).is_err());
}
