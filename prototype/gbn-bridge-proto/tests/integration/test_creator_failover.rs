use gbn_bridge_runtime::{
    AckTracker, BridgePool, FanoutScheduler, FanoutSchedulerConfig, FrameDispatch,
    InProcessPublisherClient,
};

use crate::common::*;

#[test]
fn creator_failover_reassigns_pending_frame_to_another_active_bridge() {
    let shared_client = InProcessPublisherClient::new(publisher());
    let mut bridge_a = startup_bridge("bridge-a", 61, "198.51.100.50", &shared_client, 1_000);
    let bridge_b = startup_bridge("bridge-b", 62, "198.51.100.51", &shared_client, 1_000);

    let mut creator = creator_runtime("creator-failover", 63, "203.0.113.40");
    prime_creator(&mut creator, &mut bridge_a, 1_100);
    creator.mark_bridge_active("bridge-b", 1_100);

    let pool = BridgePool::from_creator(&creator, 2).unwrap();
    let mut sender = chunk_sender(4);
    let session = sender
        .begin_session(&creator, b"failover-case", 2_000)
        .unwrap();
    let mut tracker = AckTracker::new(&session);
    let mut scheduler = FanoutScheduler::new(
        &pool,
        FanoutSchedulerConfig {
            target_bridge_count: 2,
            reuse_timeout_ms: 50,
        },
        2_000,
    );

    let plan = scheduler.initial_plan(session.frames()).unwrap();
    assert!(plan.pending.is_empty() || !plan.initial.is_empty());

    let first_dispatch = plan.initial[0].clone();
    let second_dispatch = plan
        .initial
        .get(1)
        .cloned()
        .unwrap_or_else(|| FrameDispatch {
            bridge_id: "bridge-a".into(),
            frame: session.frames()[1].clone(),
            reused_bridge: false,
        });

    let mut bridges = [bridge_a, bridge_b];
    sender
        .open_selected_bridges(&session, &pool.selected_bridge_ids(), &mut bridges, 2_001)
        .unwrap();

    let _ = sender
        .send_dispatches(&[first_dispatch], &mut bridges, &mut tracker, 2_010)
        .unwrap();

    assert!(scheduler.mark_failed("bridge-a"));
    let reassigned = scheduler
        .reassign_frame(second_dispatch.frame, "bridge-a")
        .unwrap();
    assert_eq!(reassigned.bridge_id, "bridge-b");
    let _ = sender
        .send_dispatches(&[reassigned.clone()], &mut bridges, &mut tracker, 2_020)
        .unwrap();

    let session_record = shared_client
        .authority()
        .upload_session(session.session_id())
        .unwrap()
        .clone();
    assert_eq!(
        session_record.frames_by_sequence[&reassigned.frame.sequence].via_bridge_id,
        "bridge-b"
    );
}
