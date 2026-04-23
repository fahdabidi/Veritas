use gbn_bridge_protocol::ReachabilityClass;
use gbn_bridge_runtime::InProcessPublisherClient;

use crate::common::*;

#[test]
fn non_direct_bridges_stay_signed_but_are_excluded_from_creator_ingress_paths() {
    let shared_client = InProcessPublisherClient::new(publisher());
    let mut direct_bridge =
        startup_bridge("bridge-direct", 101, "198.51.100.90", &shared_client, 1_000);
    let _brokered = startup_bridge_with_class(
        "bridge-brokered",
        102,
        "198.51.100.91",
        &shared_client,
        ReachabilityClass::Brokered,
        1_000,
    );
    let _relay_only = startup_bridge_with_class(
        "bridge-relay-only",
        103,
        "198.51.100.92",
        &shared_client,
        ReachabilityClass::RelayOnly,
        1_000,
    );

    let mut creator = creator_runtime("creator-reachability", 104, "203.0.113.70");
    prime_creator(&mut creator, &mut direct_bridge, 1_100);

    let catalog = creator.catalog_cache().current().unwrap();
    assert_eq!(catalog.bridges.len(), 3);

    let selected = creator.select_refresh_bridge(1_100).unwrap();
    assert_eq!(selected.bridge_id, "bridge-direct");

    let fanout = creator.begin_refresh_fanout(1_100).unwrap();
    assert_eq!(fanout.len(), 1);
    assert_eq!(fanout[0].target_node_id, "bridge-direct");
}
