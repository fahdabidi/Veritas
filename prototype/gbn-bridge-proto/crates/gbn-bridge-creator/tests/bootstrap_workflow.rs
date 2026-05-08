use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_creator::{LocalDhtMutation, LocalDhtStore};
use gbn_bridge_protocol::{
    publisher_identity, BootstrapSession, BridgeDhtEntry, BridgeDhtEntryUnsigned, CreatorDhtEntry,
    CreatorDhtEntryUnsigned, DhtBridgeIngressEndpoint, LocalDiscoveryTable, PublicKeyBytes,
    ReachabilityClass, SelfOnboardingState, TunnelPeerRole, TunnelState,
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
        "veritas-creator-bootstrap-workflow-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ))
}

fn creator_entry(now_ms: u64) -> CreatorDhtEntry {
    CreatorDhtEntry::sign(
        CreatorDhtEntryUnsigned {
            node_id: "creator-new".to_string(),
            ip_addr: "127.0.0.1".to_string(),
            pub_key: public_key(20),
            udp_punch_port: 4443,
            entry_expiry_ms: now_ms + 60_000,
        },
        &signing_key(9),
        true,
    )
    .expect("creator entry should sign")
}

fn bridge_entry(index: usize, active: bool, now_ms: u64) -> BridgeDhtEntry {
    BridgeDhtEntry::sign(
        BridgeDhtEntryUnsigned {
            bridge_id: format!("exit-bridge-{index:02}"),
            identity_pub: public_key(40 + index as u8),
            ingress_endpoints: vec![DhtBridgeIngressEndpoint::direct(
                "127.0.0.1",
                4443 + index as u16,
            )],
            udp_punch_port: 4443 + index as u16,
            reachability_class: ReachabilityClass::Direct,
            lease_expiry_ms: now_ms + 60_000,
            entry_expiry_ms: now_ms + 60_000,
            capabilities: vec!["bootstrap_seed".to_string(), "session_relay".to_string()],
        },
        &signing_key(9),
        active,
    )
    .expect("bridge entry should sign")
}

#[test]
fn creator_local_dht_can_complete_bootstrap_workflow_from_signed_payload() {
    let dir = unique_test_dir("complete");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("local_dht.json");
    let now = now_ms();
    let store = LocalDhtStore::start(
        "creator-new",
        &path,
        LocalDiscoveryTable::empty("creator-new", now),
    );
    let session_id = "bootstrap-000001";

    store
        .mutate(
            LocalDhtMutation::SetSelfOnboardingState(SelfOnboardingState::SeedBridgeAssigned),
            now + 1,
        )
        .unwrap();
    store
        .mutate(
            LocalDhtMutation::SetBootstrapSession(Some(BootstrapSession {
                session_id: session_id.to_string(),
                chain_id: Some("bootstrap-local-dht".to_string()),
                started_at_ms: now,
                last_event_ms: now + 1,
                last_state: "seed_bridge_assigned".to_string(),
            })),
            now + 1,
        )
        .unwrap();
    store
        .mutate(
            LocalDhtMutation::SetCreatorEntry(Some(creator_entry(now))),
            now + 2,
        )
        .unwrap();
    for index in 1..=10 {
        store
            .mutate(
                LocalDhtMutation::UpsertBridgeEntry(bridge_entry(index, false, now)),
                now + 3,
            )
            .unwrap();
    }
    store
        .mutate(
            LocalDhtMutation::SetSelfOnboardingState(SelfOnboardingState::BridgeSetReceived),
            now + 4,
        )
        .unwrap();
    for index in 1..=10 {
        store
            .mutate(
                LocalDhtMutation::UpsertBridgeEntry(bridge_entry(index, true, now)),
                now + 5,
            )
            .unwrap();
    }
    let tunnels = (1..=10)
        .map(|index| TunnelState {
            peer_id: format!("exit-bridge-{index:02}"),
            peer_role: TunnelPeerRole::ExitBridge,
            established_at_ms: now + 5,
            last_seen_ms: now + 5,
            bootstrap_session_id: Some(session_id.to_string()),
        })
        .collect::<Vec<_>>();
    store
        .mutate(LocalDhtMutation::SetActiveTunnels(tunnels), now + 5)
        .unwrap();
    store
        .mutate(
            LocalDhtMutation::SetBootstrapSession(Some(BootstrapSession {
                session_id: session_id.to_string(),
                chain_id: Some("bootstrap-local-dht".to_string()),
                started_at_ms: now,
                last_event_ms: now + 6,
                last_state: "onboarded".to_string(),
            })),
            now + 6,
        )
        .unwrap();
    let table = store
        .mutate(
            LocalDhtMutation::SetSelfOnboardingState(SelfOnboardingState::Onboarded),
            now + 6,
        )
        .unwrap();

    assert_eq!(table.self_onboarding_state, SelfOnboardingState::Onboarded);
    assert_eq!(
        table
            .creator_entry
            .as_ref()
            .map(|entry| entry.node_id.as_str()),
        Some("creator-new")
    );
    assert_eq!(table.bridge_entries.len(), 10);
    assert!(table.bridge_entries.iter().all(|entry| entry.active));
    assert_eq!(table.active_tunnels.len(), 10);
    assert_eq!(
        table
            .current_bootstrap_session
            .as_ref()
            .map(|session| session.last_state.as_str()),
        Some("onboarded")
    );

    let raw = std::fs::read_to_string(&path).unwrap();
    let reloaded: LocalDiscoveryTable = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        reloaded.self_onboarding_state,
        SelfOnboardingState::Onboarded
    );
    assert_eq!(reloaded.bridge_entries.len(), 10);

    let _ = std::fs::remove_dir_all(dir);
}
