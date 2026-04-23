use crate::common::*;

#[test]
fn direct_bridge_registration_exposes_ingress_and_updates_authority_state() {
    let shared_client = InProcessPublisherClient::new(publisher());
    let mut bridge = startup_bridge("bridge-a", 11, "198.51.100.10", &shared_client, 1_000);

    assert!(bridge.ingress_is_exposed(1_001));
    assert_eq!(shared_client.authority().active_bridge_count(1_001), 1);
    assert_eq!(
        shared_client
            .authority()
            .metrics_snapshot()
            .successful_registrations,
        1
    );
}

use gbn_bridge_runtime::InProcessPublisherClient;
