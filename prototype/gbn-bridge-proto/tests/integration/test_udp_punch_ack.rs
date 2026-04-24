use gbn_bridge_runtime::{
    establish_seed_tunnel, fetch_bridge_set, request_first_contact, InProcessPublisherClient,
};

use crate::common::*;

#[test]
fn seed_and_follow_on_punch_acks_are_correlated_to_the_right_targets() {
    let shared_client = InProcessPublisherClient::new(publisher());
    let mut relay_bridge =
        startup_bridge("bridge-relay", 51, "198.51.100.40", &shared_client, 1_000);
    let mut seed_bridge =
        startup_bridge("bridge-a-seed", 52, "198.51.100.41", &shared_client, 1_000);
    let _extra = startup_bridge("bridge-z-extra", 53, "198.51.100.42", &shared_client, 1_000);

    let mut creator = creator_runtime("creator-punch", 54, "203.0.113.30");
    let mut host_creator = host_creator();
    let plan = request_first_contact(
        &mut creator,
        &mut host_creator,
        &mut relay_bridge,
        "join-punch",
        2_000,
    )
    .unwrap();

    let seed_tunnel = establish_seed_tunnel(&mut creator, &mut seed_bridge, &plan, 2_010).unwrap();
    assert_eq!(
        seed_tunnel.probe.probe_nonce,
        seed_tunnel.bridge_ack.acked_probe_nonce
    );

    let bridge_set = fetch_bridge_set(&mut creator, &mut seed_bridge, &plan, 2_020).unwrap();
    let attempts = creator
        .begin_bootstrap_fanout(
            &plan.reply.response.bootstrap_session_id,
            &bridge_set,
            2_030,
        )
        .unwrap();
    let follow_on = attempts
        .into_iter()
        .find(|attempt| attempt.target_node_id != "bridge-a-seed")
        .expect("at least one follow-on bridge should be assigned");

    let ack = creator.acknowledge_tunnel(&follow_on, 2_040).unwrap();
    assert_eq!(ack.target_node_id, follow_on.target_node_id);
    assert_eq!(ack.acked_probe_nonce, follow_on.probe_nonce);
}
