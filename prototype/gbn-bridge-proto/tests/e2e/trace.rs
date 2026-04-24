use gbn_bridge_protocol::{ReachabilityClass, RefreshHintReason};
use gbn_bridge_runtime::{
    establish_seed_tunnel, fetch_bridge_set, request_first_contact, AckTracker, BridgePool,
    ChunkSender, ChunkSenderConfig, FanoutScheduler, FanoutSchedulerConfig, FrameDispatch,
    FramePayloadConfig, UploadSessionConfig,
};

use crate::common::{now_ms, DistributedHarness};

#[test]
fn bootstrap_chain_id_stays_consistent_across_host_creator_authority_and_bridge_control() {
    let harness = DistributedHarness::in_memory();
    let started_at_ms = now_ms();

    let mut relay_bridge = harness.start_bridge(
        "bridge-trace-relay",
        151,
        "198.51.100.151",
        ReachabilityClass::Direct,
        started_at_ms,
    );
    let mut seed_bridge = harness.start_bridge(
        "bridge-trace-seed",
        152,
        "198.51.100.152",
        ReachabilityClass::Direct,
        started_at_ms + 1,
    );
    let mut extra_bridge = harness.start_bridge(
        "bridge-trace-extra",
        153,
        "198.51.100.153",
        ReachabilityClass::Direct,
        started_at_ms + 2,
    );

    let mut creator = harness.creator("creator-trace-bootstrap", 161, "203.0.113.161");
    let mut host_creator = harness.host_creator("host-trace-bootstrap", 162);
    let plan = request_first_contact(
        &mut creator,
        &mut host_creator,
        &mut relay_bridge,
        "join-trace-bootstrap",
        started_at_ms + 10,
    )
    .unwrap();

    let seed_bridge_runtime = if plan.reply.response.seed_bridge.node_id == "bridge-trace-seed" {
        &mut seed_bridge
    } else {
        &mut extra_bridge
    };
    let seed_outcome =
        establish_seed_tunnel(&mut creator, seed_bridge_runtime, &plan, started_at_ms + 20)
            .unwrap();
    let bridge_set =
        fetch_bridge_set(&mut creator, seed_bridge_runtime, &plan, started_at_ms + 30).unwrap();

    assert_eq!(seed_outcome.probe.chain_id, plan.chain_id);
    assert_eq!(seed_outcome.bridge_ack.chain_id, plan.chain_id);
    assert_eq!(bridge_set.chain_id, plan.chain_id);
    assert!(seed_bridge_runtime
        .local_progress_events()
        .iter()
        .all(|progress| progress.chain_id == plan.chain_id));

    let session = harness.bootstrap_session_record(&plan.reply.response.bootstrap_session_id);
    assert_eq!(session.chain_id, plan.chain_id);
    assert!(session
        .progress_events
        .iter()
        .all(|progress| progress.chain_id == plan.chain_id));
}

#[test]
fn upload_chain_id_stays_consistent_across_bridge_receiver_and_authority_storage() {
    let harness = DistributedHarness::in_memory();
    let started_at_ms = now_ms();

    let mut bridge = harness.start_bridge(
        "bridge-trace-upload",
        171,
        "198.51.100.171",
        ReachabilityClass::Direct,
        started_at_ms,
    );
    let mut creator = harness.creator("creator-trace-upload", 172, "203.0.113.172");
    creator
        .refresh_catalog_via_bridge(&mut bridge, RefreshHintReason::Startup, started_at_ms + 10)
        .unwrap();
    creator.mark_bridge_active("bridge-trace-upload", started_at_ms + 10);

    let pool = BridgePool::from_creator(&creator, 10).unwrap();
    let mut sender = ChunkSender::with_config(ChunkSenderConfig {
        upload_session: UploadSessionConfig {
            frame_payload: FramePayloadConfig {
                frame_size_bytes: 6,
            },
        },
    });
    let session = sender
        .begin_session(&creator, b"trace-upload-payload", started_at_ms + 20)
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
        bridge_id: "bridge-trace-upload".into(),
        frame: session.frames()[0].clone(),
        reused_bridge: false,
    };
    let first_ack = sender
        .send_dispatches(
            &[first_dispatch],
            std::slice::from_mut(&mut bridge),
            &mut tracker,
            started_at_ms + 30,
        )
        .unwrap()
        .pop()
        .unwrap();
    let reused = scheduler
        .reuse_pending(&plan.pending, started_at_ms + 60)
        .unwrap();
    let reused_acks = sender
        .send_dispatches(
            &reused,
            std::slice::from_mut(&mut bridge),
            &mut tracker,
            started_at_ms + 60,
        )
        .unwrap();

    assert_eq!(first_ack.chain_id, session.chain_id());
    assert!(reused_acks
        .iter()
        .all(|ack| ack.chain_id == session.chain_id()));

    let stored = harness.upload_session_record(session.session_id());
    assert_eq!(stored.chain_id.as_deref(), Some(session.chain_id()));
    assert!(stored
        .frames_by_sequence
        .values()
        .all(|record| record.chain_id.as_deref() == Some(session.chain_id())));
    assert!(bridge
        .local_forwarded_frames()
        .iter()
        .all(|record| record.frame.chain_id == session.chain_id()));
}
