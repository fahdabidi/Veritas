use gbn_bridge_runtime::{
    establish_seed_tunnel, fetch_bridge_set, request_first_contact, InProcessPublisherClient,
};

use crate::common::*;

#[test]
fn first_creator_bootstrap_completes_seed_and_bridge_set_flow() {
    let shared_client = InProcessPublisherClient::new(publisher());
    let mut relay_bridge =
        startup_bridge("bridge-relay", 41, "198.51.100.30", &shared_client, 1_000);
    let mut seed_bridge =
        startup_bridge("bridge-a-seed", 42, "198.51.100.31", &shared_client, 1_000);
    let _extra_a = startup_bridge(
        "bridge-z-extra-a",
        43,
        "198.51.100.32",
        &shared_client,
        1_000,
    );
    let _extra_b = startup_bridge(
        "bridge-z-extra-b",
        44,
        "198.51.100.33",
        &shared_client,
        1_000,
    );

    let mut creator = creator_runtime("creator-bootstrap", 45, "203.0.113.20");
    let mut host_creator = host_creator();

    let plan = request_first_contact(
        &mut creator,
        &mut host_creator,
        &mut relay_bridge,
        "join-bootstrap",
        2_000,
    )
    .unwrap();
    assert_eq!(plan.reply.response.seed_bridge.node_id, "bridge-a-seed");
    assert!(creator.publisher_trust_root().is_some());
    assert_eq!(creator.self_entry().unwrap().node_id, "creator-bootstrap");

    let seed_tunnel = establish_seed_tunnel(&mut creator, &mut seed_bridge, &plan, 2_010).unwrap();
    assert_eq!(
        seed_tunnel.bridge_ack.responder_node_id,
        "creator-bootstrap"
    );

    let bridge_set = fetch_bridge_set(&mut creator, &mut seed_bridge, &plan, 2_020).unwrap();
    assert_eq!(
        bridge_set.bootstrap_session_id,
        plan.reply.response.bootstrap_session_id
    );
    assert!(!bridge_set.bridge_entries.is_empty());
    assert!(creator.local_dht().len() >= 2);
}
