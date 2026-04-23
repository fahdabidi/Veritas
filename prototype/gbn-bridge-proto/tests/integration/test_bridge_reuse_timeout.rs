use gbn_bridge_protocol::BridgeAckStatus;
use gbn_bridge_runtime::{
    AckTracker, BridgePool, FanoutScheduler, FanoutSchedulerConfig, InProcessPublisherClient,
};

use crate::common::*;

#[test]
fn insufficient_fanout_reuses_active_bridge_after_timeout() {
    let shared_client = InProcessPublisherClient::new(publisher());
    let mut bridge = startup_bridge("bridge-a", 81, "198.51.100.70", &shared_client, 1_000);
    let mut creator = creator_runtime("creator-reuse", 82, "203.0.113.50");
    prime_creator(&mut creator, &mut bridge, 1_100);

    let pool = BridgePool::from_creator(&creator, 10).unwrap();
    let mut sender = chunk_sender(4);
    let session = sender
        .begin_session(&creator, b"bridge-reuse-timeout", 2_000)
        .unwrap();
    let mut tracker = AckTracker::new(&session);
    let scheduler = FanoutScheduler::new(
        &pool,
        FanoutSchedulerConfig {
            target_bridge_count: 10,
            reuse_timeout_ms: 50,
        },
        2_000,
    );

    sender
        .open_selected_bridges(
            &session,
            &pool.selected_bridge_ids(),
            std::slice::from_mut(&mut bridge),
            2_000,
        )
        .unwrap();

    let plan = scheduler.initial_plan(session.frames()).unwrap();
    assert_eq!(plan.initial.len(), 1);
    assert!(!plan.pending.is_empty());
    assert!(scheduler
        .reuse_pending(&plan.pending, 2_010)
        .unwrap()
        .is_empty());

    let mut acks = sender
        .send_dispatches(
            &plan.initial,
            std::slice::from_mut(&mut bridge),
            &mut tracker,
            2_010,
        )
        .unwrap();
    let reused = scheduler.reuse_pending(&plan.pending, 2_060).unwrap();
    assert!(reused.iter().all(|dispatch| dispatch.reused_bridge));
    acks.extend(
        sender
            .send_dispatches(
                &reused,
                std::slice::from_mut(&mut bridge),
                &mut tracker,
                2_060,
            )
            .unwrap(),
    );

    assert!(tracker.all_acked());
    assert!(acks
        .iter()
        .any(|ack| ack.status == BridgeAckStatus::Complete));
}
