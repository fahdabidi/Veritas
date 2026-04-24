use gbn_bridge_protocol::{BridgeCatalogRequest, ReachabilityClass, RefreshHintReason};

use crate::common::{now_ms, DistributedHarness};

#[test]
fn returning_creator_refresh_and_catalog_notifications_use_network_and_control_paths() {
    let harness = DistributedHarness::in_memory();
    let started_at_ms = now_ms();

    let mut bridge_a = harness.start_bridge(
        "bridge-refresh-a",
        71,
        "198.51.100.71",
        ReachabilityClass::Direct,
        started_at_ms,
    );
    let mut bridge_b = harness.start_bridge(
        "bridge-refresh-b",
        72,
        "198.51.100.72",
        ReachabilityClass::Direct,
        started_at_ms + 1,
    );
    let _bridge_brokered = harness.start_bridge(
        "bridge-refresh-brokered",
        73,
        "198.51.100.73",
        ReachabilityClass::Brokered,
        started_at_ms + 2,
    );

    let refresh_notice = {
        let service = harness.service_handle();
        let mut service = service.lock().unwrap();
        service
            .publisher_authority_mut()
            .queue_catalog_refresh_notification(
                "bridge-refresh-a",
                "chain-refresh-e2e-01",
                &BridgeCatalogRequest {
                    creator_id: "creator-refresh-e2e".into(),
                    known_catalog_id: None,
                    direct_only: false,
                    refresh_hint: None,
                },
                started_at_ms + 5,
            )
            .unwrap()
    };
    bridge_a.send_control_keepalive(started_at_ms + 6).unwrap();

    let refresh_ack = bridge_a
        .receive_next_control_command(started_at_ms + 10)
        .unwrap()
        .expect("bridge should receive refresh notification");
    assert_eq!(refresh_ack.chain_id, "chain-refresh-e2e-01");
    assert!(bridge_a
        .received_catalog_refreshes()
        .iter()
        .any(|catalog| catalog.catalog_id == refresh_notice.catalog_id));

    let mut creator = harness.creator("creator-refresh-e2e", 81, "203.0.113.81");
    let refreshed = creator
        .refresh_catalog_via_bridge(
            &mut bridge_a,
            RefreshHintReason::Startup,
            started_at_ms + 20,
        )
        .unwrap();
    assert_eq!(
        harness.issue_catalog_record(&refreshed.catalog_id).response,
        refreshed
    );
    assert!(refreshed
        .bridges
        .iter()
        .any(|bridge| bridge.bridge_id == "bridge-refresh-b"));
    assert!(refreshed
        .bridges
        .iter()
        .any(|bridge| bridge.bridge_id == "bridge-refresh-brokered"));

    creator.record_refresh_failure("bridge-refresh-a");
    let ordered = creator
        .ordered_refresh_candidates(started_at_ms + 21)
        .unwrap();
    assert_eq!(ordered[0].bridge_id, "bridge-refresh-b");
    assert!(!ordered
        .iter()
        .any(|candidate| candidate.bridge_id == "bridge-refresh-a"));

    let second_refresh = creator
        .refresh_catalog_via_bridge(
            &mut bridge_b,
            RefreshHintReason::ManualRefresh,
            started_at_ms + 30,
        )
        .unwrap();
    assert!(second_refresh
        .bridges
        .iter()
        .any(|bridge| bridge.bridge_id == "bridge-refresh-b"));
}
