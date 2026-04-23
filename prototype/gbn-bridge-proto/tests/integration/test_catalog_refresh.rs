use gbn_bridge_protocol::ReachabilityClass;
use gbn_bridge_runtime::InProcessPublisherClient;

use crate::common::*;

#[test]
fn returning_creator_refresh_preserves_signed_catalog_and_direct_selection() {
    let shared_client = InProcessPublisherClient::new(publisher());
    let mut refresh_bridge =
        startup_bridge("bridge-direct", 21, "198.51.100.20", &shared_client, 1_000);
    let _brokered = startup_bridge_with_class(
        "bridge-brokered",
        22,
        "198.51.100.21",
        &shared_client,
        ReachabilityClass::Brokered,
        1_000,
    );
    let _relay_only = startup_bridge_with_class(
        "bridge-relay-only",
        23,
        "198.51.100.22",
        &shared_client,
        ReachabilityClass::RelayOnly,
        1_000,
    );

    let mut creator = creator_runtime("creator-refresh", 31, "203.0.113.10");
    prime_creator(&mut creator, &mut refresh_bridge, 1_100);

    let catalog = creator
        .catalog_cache()
        .current()
        .expect("catalog should be cached");
    assert_eq!(catalog.bridges.len(), 3);

    let selected = creator.select_refresh_bridge(1_100).unwrap();
    assert_eq!(selected.bridge_id, "bridge-direct");

    let candidates = creator.ordered_refresh_candidates(1_100).unwrap();
    assert_eq!(candidates[0].bridge_id, "bridge-direct");
}
