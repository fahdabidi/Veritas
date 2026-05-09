use ed25519_dalek::SigningKey;
use gbn_bridge_creator::{plan_lanes, LanePlanError};
use gbn_bridge_protocol::{
    publisher_identity, BridgeDhtEntry, BridgeDhtEntryUnsigned,
    DhtBridgeIngressEndpoint as BridgeIngressEndpoint, LocalDiscoveryTable, PublicKeyBytes,
    ReachabilityClass, SelfOnboardingState, TunnelPeerRole, TunnelState,
};

fn bridge_entry(
    signing_key: &SigningKey,
    id: &str,
    port: u16,
    active: bool,
    reachability_class: ReachabilityClass,
    suspect_until_ms: Option<u64>,
    now_ms: u64,
) -> BridgeDhtEntry {
    let mut entry = BridgeDhtEntry::sign(
        BridgeDhtEntryUnsigned {
            bridge_id: id.to_string(),
            identity_pub: PublicKeyBytes(vec![port as u8; 32]),
            ingress_endpoints: vec![BridgeIngressEndpoint::direct("127.0.0.1", port)],
            udp_punch_port: port,
            reachability_class,
            lease_expiry_ms: now_ms + 300_000,
            entry_expiry_ms: now_ms + 300_000,
            capabilities: vec!["session_relay".to_string()],
        },
        signing_key,
        active,
    )
    .unwrap();
    entry.suspect_until_ms = suspect_until_ms;
    entry
}

fn table(entries: Vec<BridgeDhtEntry>, now_ms: u64) -> LocalDiscoveryTable {
    let mut table = LocalDiscoveryTable::empty("new-creator", now_ms);
    table.self_onboarding_state = SelfOnboardingState::Onboarded;
    table.active_tunnels = entries
        .iter()
        .enumerate()
        .map(|(idx, entry)| TunnelState {
            peer_id: entry.bridge_id.clone(),
            peer_role: TunnelPeerRole::ExitBridge,
            established_at_ms: now_ms,
            last_seen_ms: now_ms + idx as u64,
            bootstrap_session_id: None,
        })
        .collect();
    table.bridge_entries = entries;
    table
}

#[test]
fn ten_active_bridges_selects_ten_lanes() {
    let now = 1_000;
    let signing_key = SigningKey::from_bytes(&[9; 32]);
    let publisher = publisher_identity(&signing_key);
    let entries = (0..10)
        .map(|idx| {
            bridge_entry(
                &signing_key,
                &format!("exit-bridge-{idx}"),
                40_000 + idx,
                true,
                ReachabilityClass::Direct,
                None,
                now,
            )
        })
        .collect();
    let plan = plan_lanes(&table(entries, now), &publisher, 10, now).unwrap();
    assert_eq!(plan.selected_bridges.len(), 10);
    assert!(plan.overflow_pool.is_empty());
}

#[test]
fn fewer_active_bridges_returns_reusable_smaller_plan() {
    let now = 1_000;
    let signing_key = SigningKey::from_bytes(&[9; 32]);
    let publisher = publisher_identity(&signing_key);
    let entries = (0..5)
        .map(|idx| {
            bridge_entry(
                &signing_key,
                &format!("exit-bridge-{idx}"),
                40_000 + idx,
                true,
                ReachabilityClass::Direct,
                None,
                now,
            )
        })
        .collect();
    let plan = plan_lanes(&table(entries, now), &publisher, 10, now).unwrap();
    assert_eq!(plan.selected_bridges.len(), 5);
    assert!(plan.overflow_pool.is_empty());
}

#[test]
fn overflow_pool_keeps_extra_candidates_for_failover() {
    let now = 1_000;
    let signing_key = SigningKey::from_bytes(&[9; 32]);
    let publisher = publisher_identity(&signing_key);
    let entries = (0..12)
        .map(|idx| {
            bridge_entry(
                &signing_key,
                &format!("exit-bridge-{idx}"),
                40_000 + idx,
                true,
                ReachabilityClass::Direct,
                None,
                now,
            )
        })
        .collect();
    let plan = plan_lanes(&table(entries, now), &publisher, 10, now).unwrap();
    assert_eq!(plan.selected_bridges.len(), 10);
    assert_eq!(plan.overflow_pool.len(), 2);
}

#[test]
fn relay_only_and_suspect_bridges_are_filtered() {
    let now = 1_000;
    let signing_key = SigningKey::from_bytes(&[9; 32]);
    let publisher = publisher_identity(&signing_key);
    let entries = vec![
        bridge_entry(
            &signing_key,
            "relay-only",
            40_001,
            true,
            ReachabilityClass::RelayOnly,
            None,
            now,
        ),
        bridge_entry(
            &signing_key,
            "suspect",
            40_002,
            true,
            ReachabilityClass::Direct,
            Some(now + 10_000),
            now,
        ),
        bridge_entry(
            &signing_key,
            "eligible",
            40_003,
            true,
            ReachabilityClass::Direct,
            None,
            now,
        ),
    ];
    let plan = plan_lanes(&table(entries, now), &publisher, 10, now).unwrap();
    assert_eq!(plan.selected_bridges.len(), 1);
    assert_eq!(plan.selected_bridges[0].bridge_id, "eligible");
    assert_eq!(plan.filter_drops.relay_only, 1);
    assert_eq!(plan.filter_drops.suspect, 1);
}

#[test]
fn no_surviving_bridge_returns_no_eligible_bridges() {
    let now = 1_000;
    let signing_key = SigningKey::from_bytes(&[9; 32]);
    let publisher = publisher_identity(&signing_key);
    let entries = vec![bridge_entry(
        &signing_key,
        "inactive",
        40_001,
        false,
        ReachabilityClass::Direct,
        None,
        now,
    )];
    let error = plan_lanes(&table(entries, now), &publisher, 10, now).unwrap_err();
    assert!(matches!(error, LanePlanError::NoEligibleBridges { .. }));
}
