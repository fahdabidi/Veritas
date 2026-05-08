use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_creator::{LocalDhtMutation, LocalDhtStore};
use gbn_bridge_protocol::{
    publisher_identity, BootstrapSession, BridgeDhtEntry, BridgeDhtEntryUnsigned, CreatorDhtEntry,
    CreatorDhtEntryUnsigned, DhtBridgeIngressEndpoint, HostRoleState, LocalDiscoveryTable,
    PublicKeyBytes, PublisherDhtEntry, ReachabilityClass, SelfOnboardingState,
};

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn public_key(seed: u8) -> PublicKeyBytes {
    publisher_identity(&signing_key(seed))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_millis() as u64
}

fn unique_test_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "veritas-local-dht-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ))
}

fn bridge_entry(
    publisher: &SigningKey,
    bridge_id: &str,
    lease_expiry_ms: u64,
    entry_expiry_ms: u64,
) -> BridgeDhtEntry {
    BridgeDhtEntry::sign(
        BridgeDhtEntryUnsigned {
            bridge_id: bridge_id.to_string(),
            identity_pub: public_key(20),
            ingress_endpoints: vec![DhtBridgeIngressEndpoint::direct("10.0.0.10", 4443)],
            udp_punch_port: 4443,
            reachability_class: ReachabilityClass::Direct,
            lease_expiry_ms,
            entry_expiry_ms,
            capabilities: vec!["session_relay".to_string()],
        },
        publisher,
        true,
    )
    .expect("bridge entry should sign")
}

fn creator_entry(publisher: &SigningKey, entry_expiry_ms: u64) -> CreatorDhtEntry {
    CreatorDhtEntry::sign(
        CreatorDhtEntryUnsigned {
            node_id: "creator-new".to_string(),
            ip_addr: "10.0.0.20".to_string(),
            pub_key: public_key(21),
            udp_punch_port: 4443,
            entry_expiry_ms,
        },
        publisher,
        true,
    )
    .expect("creator entry should sign")
}

#[test]
fn empty_local_dht_dump_before_seeding() {
    let dir = unique_test_dir("empty");
    let path = dir.join("local_dht.json");
    let publisher_pub = public_key(9);

    let store = LocalDhtStore::load_or_create("creator-new", &path, Some(&publisher_pub), 1_000)
        .expect("store should start");
    let snapshot = store.snapshot();

    assert_eq!(snapshot.actor_id, "creator-new");
    assert_eq!(snapshot.role, "creator");
    assert_eq!(snapshot.self_onboarding_state, SelfOnboardingState::None);
    assert_eq!(snapshot.host_role_state, HostRoleState::NotHost);
    assert!(snapshot.bridge_entries.is_empty());
    assert!(path.exists());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn local_dht_serialization_round_trip_through_store() {
    let dir = unique_test_dir("roundtrip");
    let path = dir.join("local_dht.json");
    let publisher = signing_key(9);
    let publisher_pub = publisher_identity(&publisher);
    let mut table = LocalDiscoveryTable::empty("creator-new", 2_000);
    table.self_onboarding_state = SelfOnboardingState::Onboarded;
    table.publisher_entry = Some(PublisherDhtEntry {
        node_id: "publisher".to_string(),
        authority_url: "http://publisher-authority:8080".to_string(),
        receiver_url: "http://publisher-receiver:8081".to_string(),
        pub_key: publisher_pub.clone(),
        entry_expiry_ms: 10_000,
    });
    table.creator_entry = Some(creator_entry(&publisher, 10_000));
    table.bridge_entries = vec![bridge_entry(&publisher, "exit-bridge-0", 10_000, 10_000)];
    gbn_bridge_creator::local_dht::persist_table(&path, &table).expect("table should persist");

    let reloaded = LocalDhtStore::load_or_create("creator-new", &path, Some(&publisher_pub), 3_000)
        .expect("store should reload")
        .snapshot();

    assert_eq!(reloaded, table);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn trust_root_validation_prunes_invalid_persisted_entries() {
    let dir = unique_test_dir("validation");
    let path = dir.join("local_dht.json");
    let trusted = signing_key(9);
    let trusted_pub = publisher_identity(&trusted);
    let attacker = signing_key(10);
    let mut table = LocalDiscoveryTable::empty("creator-new", 2_000);
    table.publisher_entry = Some(PublisherDhtEntry {
        node_id: "publisher".to_string(),
        authority_url: "http://publisher-authority:8080".to_string(),
        receiver_url: "http://publisher-receiver:8081".to_string(),
        pub_key: public_key(10),
        entry_expiry_ms: 10_000,
    });
    table.creator_entry = Some(creator_entry(&attacker, 10_000));
    table.bridge_entries = vec![
        bridge_entry(&trusted, "expired-lease", 999, 10_000),
        bridge_entry(&trusted, "expired-entry", 10_000, 999),
        bridge_entry(&attacker, "bad-signature", 10_000, 10_000),
        bridge_entry(&trusted, "valid-suspect", 10_000, 10_000),
    ];
    table.bridge_entries[3].suspect_until_ms = Some(9_000);
    gbn_bridge_creator::local_dht::persist_table(&path, &table).expect("table should persist");

    let reloaded = LocalDhtStore::load_or_create("creator-new", &path, Some(&trusted_pub), 2_000)
        .expect("store should reload")
        .snapshot();

    assert!(reloaded.publisher_entry.is_none());
    assert!(reloaded.creator_entry.is_none());
    assert_eq!(reloaded.bridge_entries.len(), 1);
    assert_eq!(reloaded.bridge_entries[0].bridge_id, "valid-suspect");
    assert_eq!(reloaded.bridge_entries[0].suspect_until_ms, Some(9_000));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn unparseable_state_file_starts_empty_and_overwrites_snapshot() {
    let dir = unique_test_dir("unparseable");
    let path = dir.join("local_dht.json");
    fs::create_dir_all(&dir).expect("test dir should be created");
    fs::write(&path, "{not-json").expect("bad file should be written");
    let publisher_pub = public_key(9);

    let store = LocalDhtStore::load_or_create("creator-new", &path, Some(&publisher_pub), 1_000)
        .expect("store should recover from bad json");

    assert_eq!(
        store.snapshot().self_onboarding_state,
        SelfOnboardingState::None
    );
    let raw = fs::read_to_string(&path).expect("snapshot should be rewritten");
    let decoded: LocalDiscoveryTable =
        serde_json::from_str(&raw).expect("rewritten state should be valid json");
    assert_eq!(decoded.actor_id, "creator-new");

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn single_writer_handles_concurrent_reads_and_mutations() {
    let dir = unique_test_dir("single-writer");
    let path = dir.join("local_dht.json");
    let publisher_pub = public_key(9);
    let store = Arc::new(
        LocalDhtStore::load_or_create("creator-new", &path, Some(&publisher_pub), 1_000)
            .expect("store should start"),
    );

    let mut readers = Vec::new();
    for _ in 0..10 {
        let store = Arc::clone(&store);
        readers.push(thread::spawn(move || {
            for _ in 0..100 {
                let snapshot = store.snapshot();
                assert_eq!(snapshot.role, "creator");
            }
        }));
    }

    for index in 0..1_000 {
        store
            .mutate(
                LocalDhtMutation::SetLastError(Some(format!("mutation-{index}"))),
                1_001 + index,
            )
            .expect("mutation should persist");
    }

    for reader in readers {
        reader.join().expect("reader should not panic");
    }
    assert!(store
        .snapshot()
        .last_error
        .as_deref()
        .is_some_and(|value| value.starts_with("mutation-")));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn reset_clears_state_and_returns_prior_state() {
    let dir = unique_test_dir("reset");
    let path = dir.join("local_dht.json");
    let publisher_pub = public_key(9);
    let store = LocalDhtStore::load_or_create("creator-new", &path, Some(&publisher_pub), 1_000)
        .expect("store should start");
    let mut table = store.snapshot();
    table.self_onboarding_state = SelfOnboardingState::FanoutPartial;
    table.host_role_state = HostRoleState::HostSeeded;
    table.current_bootstrap_session = Some(BootstrapSession {
        session_id: "boot-123".to_string(),
        chain_id: Some("chain-123".to_string()),
        started_at_ms: 1_000,
        last_event_ms: 1_200,
        last_state: "fanout_partial".to_string(),
    });
    store
        .replace(table)
        .expect("state replacement should persist");

    let response = store
        .reset("reset-chain", 2_000)
        .expect("reset should persist");
    assert_eq!(response.actor_id, "creator-new");
    assert_eq!(
        response.prior_self_onboarding_state,
        SelfOnboardingState::FanoutPartial
    );
    assert_eq!(response.prior_host_role_state, HostRoleState::HostSeeded);
    assert_eq!(
        response.prior_bootstrap_session_id.as_deref(),
        Some("boot-123")
    );

    let snapshot = store.snapshot();
    assert_eq!(snapshot.self_onboarding_state, SelfOnboardingState::None);
    assert_eq!(snapshot.host_role_state, HostRoleState::NotHost);
    assert!(snapshot.current_bootstrap_session.is_none());

    let persisted: LocalDiscoveryTable = serde_json::from_str(
        &fs::read_to_string(&path).expect("persisted state should be readable"),
    )
    .expect("persisted state should deserialize");
    assert_eq!(persisted.self_onboarding_state, SelfOnboardingState::None);

    let _ = fs::remove_dir_all(dir);
}
