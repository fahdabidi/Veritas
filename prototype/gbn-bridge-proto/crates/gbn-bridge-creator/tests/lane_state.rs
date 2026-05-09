use gbn_bridge_creator::{LaneState, LaneStatus};

#[test]
fn pending_to_active_on_open_ack() {
    let mut lane = LaneState::pending("exit-bridge-0", 1_000);
    lane.mark_active(1_010);
    assert!(matches!(lane.status, LaneStatus::Active));
    assert_eq!(lane.active_at_ms, Some(1_010));
}

#[test]
fn sending_chunk_returns_to_active_on_ack() {
    let mut lane = LaneState::pending("exit-bridge-0", 1_000);
    lane.mark_active(1_010);
    lane.mark_sending(42, 1_020);
    assert!(matches!(
        lane.status,
        LaneStatus::SendingChunk {
            chunk_index: 42,
            sent_at_ms: 1_020
        }
    ));
    lane.mark_acked(42, 1_030);
    assert!(matches!(lane.status, LaneStatus::Active));
    assert_eq!(lane.chunks_acked, vec![42]);
    assert_eq!(lane.last_ack_at_ms, Some(1_030));
}

#[test]
fn any_state_can_transition_to_failed() {
    let mut lane = LaneState::pending("exit-bridge-0", 1_000);
    lane.mark_active(1_010);
    lane.mark_sending(7, 1_020);
    lane.mark_failed(1_025);
    assert!(matches!(lane.status, LaneStatus::Failed));
    assert_eq!(lane.failed_at_ms, Some(1_025));
}

#[test]
fn active_lane_drains_when_session_completes() {
    let mut lane = LaneState::pending("exit-bridge-0", 1_000);
    lane.mark_active(1_010);
    lane.mark_drained();
    assert!(matches!(lane.status, LaneStatus::Drained));
}
