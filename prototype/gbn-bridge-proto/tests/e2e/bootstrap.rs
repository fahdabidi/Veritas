use gbn_bridge_protocol::{BridgeCommandAckStatus, ReachabilityClass};
use gbn_bridge_runtime::{establish_seed_tunnel, fetch_bridge_set, request_first_contact};

use crate::common::{now_ms, DistributedHarness};

#[test]
fn first_contact_bootstrap_and_fanout_cross_real_service_boundaries() {
    let harness = DistributedHarness::in_memory();
    let started_at_ms = now_ms();

    let mut relay_bridge = harness.start_bridge(
        "bridge-relay-e2e",
        51,
        "198.51.100.51",
        ReachabilityClass::Direct,
        started_at_ms,
    );
    let mut bridge_seed = harness.start_bridge(
        "bridge-seed-e2e",
        52,
        "198.51.100.52",
        ReachabilityClass::Direct,
        started_at_ms + 1,
    );
    let mut bridge_extra = harness.start_bridge(
        "bridge-extra-e2e",
        53,
        "198.51.100.53",
        ReachabilityClass::Direct,
        started_at_ms + 2,
    );

    let mut creator = harness.creator("creator-e2e-bootstrap", 61, "203.0.113.61");
    let mut host_creator = harness.host_creator("host-creator-e2e", 62);

    let plan = request_first_contact(
        &mut creator,
        &mut host_creator,
        &mut relay_bridge,
        "join-e2e-bootstrap-01",
        started_at_ms + 10,
    )
    .unwrap();

    let selected_seed_id = plan.reply.response.seed_bridge.node_id.clone();

    let initial_commands = harness.pending_commands(&selected_seed_id);
    assert_eq!(initial_commands.len(), 1);
    assert_eq!(initial_commands[0].chain_id, plan.chain_id);

    let outcome = if selected_seed_id == "bridge-seed-e2e" {
        establish_seed_tunnel(&mut creator, &mut bridge_seed, &plan, started_at_ms + 20).unwrap()
    } else {
        establish_seed_tunnel(&mut creator, &mut bridge_extra, &plan, started_at_ms + 20).unwrap()
    };
    assert_eq!(outcome.probe.chain_id, plan.chain_id);
    assert_eq!(outcome.bridge_ack.chain_id, plan.chain_id);

    let bridge_set = if selected_seed_id == "bridge-seed-e2e" {
        fetch_bridge_set(&mut creator, &mut bridge_seed, &plan, started_at_ms + 30).unwrap()
    } else {
        fetch_bridge_set(&mut creator, &mut bridge_extra, &plan, started_at_ms + 30).unwrap()
    };
    assert_eq!(
        bridge_set.bootstrap_session_id,
        plan.reply.response.bootstrap_session_id
    );

    let fanout_ack = if selected_seed_id == "bridge-seed-e2e" {
        bridge_extra
            .receive_next_control_command(started_at_ms + 40)
            .unwrap()
            .expect("remaining bridge should receive fanout activation")
    } else {
        bridge_seed
            .receive_next_control_command(started_at_ms + 40)
            .unwrap()
            .expect("remaining bridge should receive fanout activation")
    };
    assert_eq!(fanout_ack.chain_id, plan.chain_id);
    assert_eq!(fanout_ack.status, BridgeCommandAckStatus::Applied);

    let attempts = creator
        .begin_bootstrap_fanout(
            &plan.reply.response.bootstrap_session_id,
            &bridge_set,
            started_at_ms + 45,
        )
        .unwrap();
    let follow_on_bridge_id = if selected_seed_id == "bridge-seed-e2e" {
        "bridge-extra-e2e"
    } else {
        "bridge-seed-e2e"
    };
    assert!(attempts
        .iter()
        .any(|attempt| attempt.target_node_id == follow_on_bridge_id));

    let session = harness.bootstrap_session_record(&plan.reply.response.bootstrap_session_id);
    assert_eq!(session.chain_id, plan.chain_id);
    assert_eq!(
        session.seed_bridge_id,
        plan.reply.response.seed_bridge.node_id
    );
    assert!(session.seed_tunnel_reported_at_ms.is_some());
    assert!(session.bridge_set_delivered_at_ms.is_some());
    assert!(session.fanout_activated_at_ms.is_some());
}
