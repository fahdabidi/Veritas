use gbn_bridge_protocol::{BridgeCommandAckStatus, BridgeSetRequest, ReachabilityClass};
use gbn_bridge_runtime::request_first_contact;

use crate::common::{now_ms, DistributedHarness};

#[test]
fn seed_timeout_reassigns_bootstrap_to_an_alternate_bridge() {
    let harness = DistributedHarness::in_memory();
    let started_at_ms = now_ms();

    let mut relay_bridge = harness.start_bridge(
        "bridge-timeout-relay",
        111,
        "198.51.100.111",
        ReachabilityClass::Direct,
        started_at_ms,
    );
    let mut bridge_seed = harness.start_bridge(
        "bridge-timeout-seed",
        112,
        "198.51.100.112",
        ReachabilityClass::Direct,
        started_at_ms + 1,
    );
    let mut bridge_extra = harness.start_bridge(
        "bridge-timeout-extra",
        113,
        "198.51.100.113",
        ReachabilityClass::Direct,
        started_at_ms + 2,
    );

    let mut creator = harness.creator("creator-timeout-e2e", 121, "203.0.113.121");
    let mut host_creator = harness.host_creator("host-timeout-e2e", 122);
    let plan = request_first_contact(
        &mut creator,
        &mut host_creator,
        &mut relay_bridge,
        "join-timeout-e2e",
        started_at_ms + 10,
    )
    .unwrap();

    let initial_session =
        harness.bootstrap_session_record(&plan.reply.response.bootstrap_session_id);
    let reassigned = harness.process_bootstrap_timeouts(initial_session.seed_ack_deadline_ms + 1);
    assert_eq!(reassigned.len(), 1);

    let new_seed_bridge = if reassigned[0] == "bridge-timeout-seed" {
        &mut bridge_seed
    } else {
        &mut bridge_extra
    };
    new_seed_bridge
        .send_control_keepalive(initial_session.seed_ack_deadline_ms + 2)
        .unwrap();
    let reassigned_ack = new_seed_bridge
        .receive_next_control_command(initial_session.seed_ack_deadline_ms + 3)
        .unwrap()
        .expect("reassigned seed bridge should receive seed assignment");
    assert_eq!(reassigned_ack.status, BridgeCommandAckStatus::Applied);

    let updated_session =
        harness.bootstrap_session_record(&plan.reply.response.bootstrap_session_id);
    assert_eq!(updated_session.seed_bridge_id, reassigned[0]);
    assert_eq!(updated_session.reassignment_count, 1);
    assert_eq!(updated_session.attempted_seed_bridge_ids.len(), 2);

    creator
        .apply_bootstrap_response(
            &updated_session.creator_response,
            initial_session.seed_ack_deadline_ms + 4,
        )
        .unwrap();
    new_seed_bridge.remember_bootstrap_chain_id(
        &updated_session.bootstrap_session_id,
        &updated_session.chain_id,
    );
    let active_attempt = new_seed_bridge
        .active_punch_attempt(&updated_session.bootstrap_session_id)
        .cloned()
        .expect("reassigned seed punch should be active");
    let bridge_ack = new_seed_bridge
        .acknowledge_tunnel(
            &updated_session.bootstrap_session_id,
            creator.config().creator_id.as_str(),
            creator.config().udp_punch_port,
            active_attempt.probe_nonce,
            initial_session.seed_ack_deadline_ms + 5,
        )
        .unwrap();
    assert_eq!(bridge_ack.chain_id, updated_session.chain_id);

    let bridge_set = new_seed_bridge
        .serve_bridge_set(
            &BridgeSetRequest {
                chain_id: updated_session.chain_id.clone(),
                bootstrap_session_id: updated_session.bootstrap_session_id.clone(),
                creator_id: creator.config().creator_id.clone(),
                requested_bridge_count: updated_session.creator_response.assigned_bridge_count,
            },
            initial_session.seed_ack_deadline_ms + 6,
        )
        .unwrap();
    creator
        .store_bridge_set(&bridge_set, initial_session.seed_ack_deadline_ms + 6)
        .unwrap();
}

#[test]
fn authority_restart_recovers_pending_seed_assignment_and_replays_it_on_reconnect() {
    let mut harness = DistributedHarness::durable("proto006_phase9_restart");
    let started_at_ms = now_ms();

    let mut relay_bridge = harness.start_bridge(
        "bridge-restart-relay",
        131,
        "198.51.100.131",
        ReachabilityClass::Direct,
        started_at_ms,
    );
    let _seed_bridge = harness.start_bridge(
        "bridge-restart-seed",
        132,
        "198.51.100.132",
        ReachabilityClass::Direct,
        started_at_ms + 1,
    );

    let mut creator = harness.creator("creator-restart-e2e", 141, "203.0.113.141");
    let mut host_creator = harness.host_creator("host-restart-e2e", 142);
    let plan = request_first_contact(
        &mut creator,
        &mut host_creator,
        &mut relay_bridge,
        "join-restart-e2e",
        started_at_ms + 10,
    )
    .unwrap();

    let session_before_restart =
        harness.bootstrap_session_record(&plan.reply.response.bootstrap_session_id);
    assert_eq!(session_before_restart.chain_id, plan.chain_id);
    assert_eq!(
        harness
            .pending_commands(&session_before_restart.seed_bridge_id)
            .len(),
        1
    );

    harness.restart_authority_and_receiver(started_at_ms + 50);

    let mut recovered_seed = harness.reconnect_bridge(
        &session_before_restart.seed_bridge_id,
        132,
        "198.51.100.132",
        started_at_ms + 60,
    );
    let replayed_ack = recovered_seed
        .receive_next_control_command(started_at_ms + 61)
        .unwrap()
        .expect("recovered seed bridge should receive replayed seed assignment");
    assert_eq!(replayed_ack.chain_id, plan.chain_id);
    assert_eq!(replayed_ack.status, BridgeCommandAckStatus::Applied);

    let session_after_restart =
        harness.bootstrap_session_record(&plan.reply.response.bootstrap_session_id);
    assert_eq!(session_after_restart.chain_id, plan.chain_id);
    assert_eq!(
        session_after_restart.seed_bridge_id,
        session_before_restart.seed_bridge_id
    );
    assert!(recovered_seed
        .active_punch_attempt(&session_after_restart.bootstrap_session_id)
        .is_some());
}
