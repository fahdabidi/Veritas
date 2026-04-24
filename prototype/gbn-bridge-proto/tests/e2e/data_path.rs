use gbn_bridge_protocol::{BridgeAckStatus, ReachabilityClass, RefreshHintReason};
use gbn_bridge_runtime::{
    AckTracker, BridgePool, ChunkSender, ChunkSenderConfig, FanoutScheduler, FanoutSchedulerConfig,
    FrameDispatch, FramePayloadConfig, UploadSessionConfig,
};

use crate::common::{now_ms, DistributedHarness};

#[test]
fn data_path_duplicate_retry_and_ack_flow_cross_real_receiver_boundary() {
    let harness = DistributedHarness::in_memory();
    let started_at_ms = now_ms();

    let mut bridge = harness.start_bridge(
        "bridge-data-e2e",
        91,
        "198.51.100.91",
        ReachabilityClass::Direct,
        started_at_ms,
    );
    let mut creator = harness.creator("creator-data-e2e", 92, "203.0.113.92");

    let catalog = creator
        .refresh_catalog_via_bridge(&mut bridge, RefreshHintReason::Startup, started_at_ms + 10)
        .unwrap();
    assert!(catalog
        .bridges
        .iter()
        .any(|descriptor| descriptor.bridge_id == "bridge-data-e2e"));
    creator.mark_bridge_active("bridge-data-e2e", started_at_ms + 10);

    let pool = BridgePool::from_creator(&creator, 10).unwrap();
    let mut sender = ChunkSender::with_config(ChunkSenderConfig {
        upload_session: UploadSessionConfig {
            frame_payload: FramePayloadConfig {
                frame_size_bytes: 4,
            },
        },
    });
    let session = sender
        .begin_session(&creator, b"phase-nine-e2e-data-path", started_at_ms + 20)
        .unwrap();
    let mut tracker = AckTracker::new(&session);
    let scheduler = FanoutScheduler::new(
        &pool,
        FanoutSchedulerConfig {
            target_bridge_count: 10,
            reuse_timeout_ms: 25,
        },
        started_at_ms + 20,
    );

    sender
        .open_selected_bridges(
            &session,
            &pool.selected_bridge_ids(),
            std::slice::from_mut(&mut bridge),
            started_at_ms + 20,
        )
        .unwrap();

    let plan = scheduler.initial_plan(session.frames()).unwrap();
    let first_dispatch = FrameDispatch {
        bridge_id: "bridge-data-e2e".into(),
        frame: session.frames()[0].clone(),
        reused_bridge: false,
    };
    let first_ack = sender
        .send_dispatches(
            &[first_dispatch.clone()],
            std::slice::from_mut(&mut bridge),
            &mut tracker,
            started_at_ms + 30,
        )
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(first_ack.status, BridgeAckStatus::Accepted);

    let duplicate_ack = bridge
        .forward_session_frame(first_dispatch.frame.clone(), started_at_ms + 31)
        .unwrap();
    assert_eq!(duplicate_ack.status, BridgeAckStatus::Duplicate);

    let reused = scheduler
        .reuse_pending(&plan.pending, started_at_ms + 60)
        .unwrap();
    let acks = sender
        .send_dispatches(
            &reused,
            std::slice::from_mut(&mut bridge),
            &mut tracker,
            started_at_ms + 60,
        )
        .unwrap();
    assert!(acks
        .iter()
        .any(|ack| ack.status == BridgeAckStatus::Complete));

    sender
        .close_selected_bridges(
            &session,
            &pool.selected_bridge_ids(),
            std::slice::from_mut(&mut bridge),
            started_at_ms + 90,
        )
        .unwrap();
    assert!(tracker.all_acked());

    let stored = harness.upload_session_record(session.session_id());
    assert_eq!(stored.chain_id.as_deref(), Some(session.chain_id()));
    assert_eq!(stored.frames_by_sequence.len(), session.frame_count());
    assert_eq!(
        stored.frames_by_sequence.get(&0).unwrap().via_bridge_id,
        "bridge-data-e2e"
    );
    assert!(stored
        .frames_by_sequence
        .values()
        .all(|record| record.chain_id.as_deref() == Some(session.chain_id())));
}
