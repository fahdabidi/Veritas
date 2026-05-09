use std::collections::BTreeSet;

use gbn_bridge_protocol::{
    BridgeAckStatus, BridgeClose, BridgeCloseReason, BridgeData, BridgeDhtEntry, BridgeOpen,
    EncryptedFrame, PublicKeyBytes,
};
use serde::{Deserialize, Serialize};

use crate::upload::{CreatorBridgeRequest, CreatorBridgeResponse};
use crate::CreatorError;

use super::lane_planner::LanePlan;
use super::lane_state::{ChunkAssignment, LaneState};
use super::session::{
    EncryptedUploadSession, UploadDispatchPlan, UploadSessionStatus, MANIFEST_CHUNK_INDEX,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchUploadOptions {
    pub chain_id: String,
    pub actor_id: String,
    pub actor_pub: PublicKeyBytes,
    pub lane_open_timeout_ms: u64,
    pub chunk_ack_timeout_ms: u64,
    pub suspect_ttl_ms: u64,
    pub force_lane_failure: Vec<String>,
    pub now_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendUploadSessionResult {
    pub session_id: String,
    pub chain_id: String,
    pub session_status: UploadSessionStatus,
    pub total_chunks: u32,
    pub completed_chunks: u32,
    pub failed_chunks: Vec<u32>,
    pub lanes_used: Vec<String>,
    pub lane_count_at_first_dispatch: u32,
    pub lane_count_at_completion: u32,
    pub ciphertext_only_at_bridge: bool,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_lane: Option<String>,
    pub force_lane_failure_used: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_chunk_dispatched_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_lanes_active_at_ms: Option<u64>,
    pub reused_lane_events: u32,
    pub failover_events: u32,
}

pub fn dispatch_upload_session<F>(
    session: &mut EncryptedUploadSession,
    lane_plan: LanePlan,
    options: DispatchUploadOptions,
    mut round_trip: F,
) -> Result<SendUploadSessionResult, CreatorError>
where
    F: FnMut(&str, CreatorBridgeRequest) -> Result<CreatorBridgeResponse, CreatorError>,
{
    let started_at = options.now_ms;
    let mut clock = LogicalClock::new(started_at);
    let force_lane_failure = options
        .force_lane_failure
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut dispatch_plan = UploadDispatchPlan {
        plan_started_at_ms: started_at,
        target_lane_count: lane_plan.target_lane_count,
        overflow_pool: lane_plan
            .overflow_pool
            .iter()
            .map(|entry| entry.bridge_id.clone())
            .collect(),
        session_status: UploadSessionStatus::Dispatching,
        ..UploadDispatchPlan::default()
    };
    session.status = UploadSessionStatus::Dispatching;

    let mut selected = lane_plan.selected_bridges;
    let mut overflow = lane_plan.overflow_pool;
    let mut active_lanes = Vec::<usize>::new();
    let mut attempted_lanes = 0_usize;
    let mut next_lane = 0_usize;
    let mut next_chunk = 0_usize;
    let mut manifest_sent = false;
    let mut rr_cursor = 0_usize;

    while next_lane < selected.len() && active_lanes.is_empty() {
        open_lane(
            &selected[next_lane],
            &session.session_id,
            &options,
            &mut dispatch_plan,
            &mut active_lanes,
            &mut attempted_lanes,
            &force_lane_failure,
            &mut clock,
            &mut round_trip,
            session.manifest.total_chunks,
        )?;
        next_lane += 1;
        if active_lanes.is_empty() {
            continue;
        }
        manifest_sent = try_send_manifest(
            session,
            &options,
            &mut dispatch_plan,
            &mut active_lanes,
            &mut rr_cursor,
            &mut clock,
            &mut round_trip,
        )?;
        if manifest_sent && next_chunk < session.chunk_ciphertexts.len() {
            let sent = send_next_chunk(
                session,
                &options,
                &mut dispatch_plan,
                &mut selected,
                &mut overflow,
                &mut active_lanes,
                &mut rr_cursor,
                &mut clock,
                &mut round_trip,
                &mut next_chunk,
            )?;
            if !sent && next_lane >= selected.len() && overflow.is_empty() {
                break;
            }
        }
    }

    while next_lane < selected.len() {
        let active_lane_count_before_open = active_lanes.len();
        open_lane(
            &selected[next_lane],
            &session.session_id,
            &options,
            &mut dispatch_plan,
            &mut active_lanes,
            &mut attempted_lanes,
            &force_lane_failure,
            &mut clock,
            &mut round_trip,
            session.manifest.total_chunks,
        )?;
        next_lane += 1;
        if active_lanes.len() > active_lane_count_before_open {
            rr_cursor = active_lanes.len().saturating_sub(1);
        }
        if !manifest_sent && !active_lanes.is_empty() {
            manifest_sent = try_send_manifest(
                session,
                &options,
                &mut dispatch_plan,
                &mut active_lanes,
                &mut rr_cursor,
                &mut clock,
                &mut round_trip,
            )?;
        }
        if manifest_sent && next_chunk < session.chunk_ciphertexts.len() {
            let sent = send_next_chunk(
                session,
                &options,
                &mut dispatch_plan,
                &mut selected,
                &mut overflow,
                &mut active_lanes,
                &mut rr_cursor,
                &mut clock,
                &mut round_trip,
                &mut next_chunk,
            )?;
            if !sent && next_lane >= selected.len() && overflow.is_empty() {
                break;
            }
        }
    }
    if dispatch_plan.all_lanes_active_at_ms.is_none() {
        dispatch_plan.all_lanes_active_at_ms = Some(clock.tick());
    }

    while manifest_sent && next_chunk < session.chunk_ciphertexts.len() {
        let sent = send_next_chunk(
            session,
            &options,
            &mut dispatch_plan,
            &mut selected,
            &mut overflow,
            &mut active_lanes,
            &mut rr_cursor,
            &mut clock,
            &mut round_trip,
            &mut next_chunk,
        )?;
        if !sent {
            break;
        }
    }

    if !manifest_sent {
        dispatch_plan.failed_chunks =
            all_remaining_chunks(next_chunk, session.manifest.total_chunks);
        dispatch_plan.session_status = UploadSessionStatus::Failed;
    } else if dispatch_plan.completed_chunks == session.manifest.total_chunks {
        dispatch_plan.session_status = UploadSessionStatus::Completed;
    } else if dispatch_plan.completed_chunks > 0 {
        dispatch_plan.failed_chunks =
            all_remaining_chunks(next_chunk, session.manifest.total_chunks);
        dispatch_plan.session_status = UploadSessionStatus::Partial;
    } else {
        dispatch_plan.failed_chunks =
            all_remaining_chunks(next_chunk, session.manifest.total_chunks);
        dispatch_plan.session_status = UploadSessionStatus::Failed;
    }

    dispatch_plan.lane_count_at_completion = active_lanes
        .iter()
        .filter(|idx| {
            dispatch_plan
                .lanes
                .get(**idx)
                .is_some_and(|lane| !lane.is_failed())
        })
        .count() as u32;
    for lane in &mut dispatch_plan.lanes {
        lane.mark_drained();
    }
    if matches!(dispatch_plan.session_status, UploadSessionStatus::Completed) {
        close_active_lanes(
            session,
            &options,
            &dispatch_plan,
            &mut round_trip,
            &mut clock,
        );
    }

    session.status = dispatch_plan.session_status.clone();
    session.plan = dispatch_plan.clone();
    let lanes_used = dispatch_plan
        .chunk_assignments
        .iter()
        .filter(|assignment| assignment.ack_at_ms.is_some())
        .map(|assignment| assignment.assigned_bridge_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    Ok(SendUploadSessionResult {
        session_id: session.session_id.clone(),
        chain_id: options.chain_id,
        session_status: dispatch_plan.session_status,
        total_chunks: session.manifest.total_chunks,
        completed_chunks: dispatch_plan.completed_chunks,
        failed_chunks: dispatch_plan.failed_chunks,
        lanes_used,
        lane_count_at_first_dispatch: dispatch_plan.lane_count_at_first_dispatch,
        lane_count_at_completion: dispatch_plan.lane_count_at_completion,
        ciphertext_only_at_bridge: true,
        elapsed_ms: clock.current().saturating_sub(started_at),
        manifest_lane: dispatch_plan.manifest_lane,
        force_lane_failure_used: dispatch_plan.force_lane_failure_used,
        first_chunk_dispatched_at_ms: dispatch_plan.first_chunk_dispatched_at_ms,
        all_lanes_active_at_ms: dispatch_plan.all_lanes_active_at_ms,
        reused_lane_events: dispatch_plan.reused_lane_events,
        failover_events: dispatch_plan.failover_events,
    })
}

#[allow(clippy::too_many_arguments)]
fn open_lane<F>(
    bridge: &BridgeDhtEntry,
    session_id: &str,
    options: &DispatchUploadOptions,
    plan: &mut UploadDispatchPlan,
    active_lanes: &mut Vec<usize>,
    attempted_lanes: &mut usize,
    force_lane_failure: &BTreeSet<String>,
    clock: &mut LogicalClock,
    round_trip: &mut F,
    total_chunks: u32,
) -> Result<(), CreatorError>
where
    F: FnMut(&str, CreatorBridgeRequest) -> Result<CreatorBridgeResponse, CreatorError>,
{
    let opened_at_ms = clock.tick();
    let lane_index = plan.lanes.len();
    plan.lanes
        .push(LaneState::pending(bridge.bridge_id.clone(), opened_at_ms));
    *attempted_lanes += 1;
    let expected_chunks = total_chunks.saturating_add(1).min(u16::MAX as u32) as u16;
    let open = BridgeOpen {
        chain_id: options.chain_id.clone(),
        session_id: session_id.to_string(),
        creator_id: options.actor_id.clone(),
        bridge_id: bridge.bridge_id.clone(),
        creator_session_pub: options.actor_pub.clone(),
        opened_at_ms,
        expected_chunks: Some(expected_chunks),
    };
    let address = match bridge_upload_address(bridge) {
        Ok(address) => address,
        Err(_) => {
            plan.lanes[lane_index].mark_failed(clock.tick());
            plan.failover_events = plan.failover_events.saturating_add(1);
            return Ok(());
        }
    };
    match round_trip(&address, CreatorBridgeRequest::Open(open)) {
        Ok(CreatorBridgeResponse::Opened { .. }) => {
            if force_lane_failure.contains(&bridge.bridge_id) {
                plan.lanes[lane_index].mark_failed(clock.tick());
                plan.force_lane_failure_used.push(bridge.bridge_id.clone());
                plan.failover_events = plan.failover_events.saturating_add(1);
            } else {
                plan.lanes[lane_index].mark_active(clock.tick());
                active_lanes.push(lane_index);
            }
        }
        Ok(CreatorBridgeResponse::Error { message }) => {
            plan.lanes[lane_index].mark_failed(clock.tick());
            plan.failover_events = plan.failover_events.saturating_add(1);
            let _ = message;
        }
        Ok(other) => {
            plan.lanes[lane_index].mark_failed(clock.tick());
            plan.failover_events = plan.failover_events.saturating_add(1);
            let _ = other;
        }
        Err(error) => {
            plan.lanes[lane_index].mark_failed(clock.tick());
            plan.failover_events = plan.failover_events.saturating_add(1);
            let _ = error;
        }
    }
    Ok(())
}

fn try_send_manifest<F>(
    session: &EncryptedUploadSession,
    options: &DispatchUploadOptions,
    plan: &mut UploadDispatchPlan,
    active_lanes: &mut Vec<usize>,
    rr_cursor: &mut usize,
    clock: &mut LogicalClock,
    round_trip: &mut F,
) -> Result<bool, CreatorError>
where
    F: FnMut(&str, CreatorBridgeRequest) -> Result<CreatorBridgeResponse, CreatorError>,
{
    while !active_lanes.is_empty() {
        let lane_index = active_lanes[*rr_cursor % active_lanes.len()];
        let bridge_id = plan.lanes[lane_index].bridge_id.clone();
        let sent_at_ms = clock.tick();
        match send_frame(
            session,
            options,
            &bridge_id,
            &session.manifest_ciphertext,
            MANIFEST_CHUNK_INDEX,
            false,
            sent_at_ms,
            round_trip,
        ) {
            Ok(_) => {
                plan.manifest_lane = Some(bridge_id);
                return Ok(true);
            }
            Err(error) => {
                plan.lanes[lane_index].mark_failed(clock.tick());
                plan.failover_events = plan.failover_events.saturating_add(1);
                active_lanes.retain(|idx| *idx != lane_index);
                if active_lanes.is_empty() {
                    let _ = error;
                    return Ok(false);
                }
            }
        }
    }
    Ok(false)
}

#[allow(clippy::too_many_arguments)]
fn send_next_chunk<F>(
    session: &EncryptedUploadSession,
    options: &DispatchUploadOptions,
    plan: &mut UploadDispatchPlan,
    selected: &mut Vec<BridgeDhtEntry>,
    overflow: &mut Vec<BridgeDhtEntry>,
    active_lanes: &mut Vec<usize>,
    rr_cursor: &mut usize,
    clock: &mut LogicalClock,
    round_trip: &mut F,
    next_chunk: &mut usize,
) -> Result<bool, CreatorError>
where
    F: FnMut(&str, CreatorBridgeRequest) -> Result<CreatorBridgeResponse, CreatorError>,
{
    while active_lanes.is_empty() {
        let Some(bridge) = overflow.pop() else {
            return Ok(false);
        };
        selected.push(bridge.clone());
        open_lane(
            &bridge,
            &session.session_id,
            options,
            plan,
            active_lanes,
            &mut 0,
            &BTreeSet::new(),
            clock,
            round_trip,
            session.manifest.total_chunks,
        )?;
    }

    let chunk_index = *next_chunk as u32;
    let frame = &session.chunk_ciphertexts[*next_chunk];
    loop {
        if active_lanes.is_empty() {
            return Ok(false);
        }
        let lane_index = active_lanes[*rr_cursor % active_lanes.len()];
        *rr_cursor = rr_cursor.saturating_add(1);
        let bridge_id = plan.lanes[lane_index].bridge_id.clone();
        let was_reuse = !plan.lanes[lane_index].chunks_sent.is_empty();
        let sent_at_ms = clock.tick();
        if plan.first_chunk_dispatched_at_ms.is_none() {
            plan.first_chunk_dispatched_at_ms = Some(sent_at_ms);
            plan.lane_count_at_first_dispatch = active_lanes.len() as u32;
        }
        plan.lanes[lane_index].mark_sending(chunk_index, sent_at_ms);
        let attempts = plan
            .chunk_assignments
            .iter()
            .filter(|assignment| assignment.chunk_index == chunk_index)
            .count() as u32
            + 1;
        let assignment_index = plan.chunk_assignments.len();
        plan.chunk_assignments.push(ChunkAssignment {
            chunk_index,
            assigned_bridge_id: bridge_id.clone(),
            attempts,
            first_dispatch_at_ms: sent_at_ms,
            ack_at_ms: None,
        });
        if was_reuse {
            plan.reused_lane_events = plan.reused_lane_events.saturating_add(1);
        }
        let final_frame = chunk_index + 1 == session.manifest.total_chunks;
        match send_frame(
            session,
            options,
            &bridge_id,
            frame,
            chunk_index,
            final_frame,
            sent_at_ms,
            round_trip,
        ) {
            Ok(ack_at_ms) => {
                plan.lanes[lane_index].mark_acked(chunk_index, ack_at_ms);
                plan.chunk_assignments[assignment_index].ack_at_ms = Some(ack_at_ms);
                plan.completed_chunks = plan.completed_chunks.saturating_add(1);
                *next_chunk += 1;
                return Ok(true);
            }
            Err(error) => {
                plan.lanes[lane_index].mark_failed(clock.tick());
                plan.failover_events = plan.failover_events.saturating_add(1);
                active_lanes.retain(|idx| *idx != lane_index);
                if active_lanes.is_empty() && overflow.is_empty() {
                    let _ = error;
                    return Ok(false);
                }
            }
        }
    }
}

fn send_frame<F>(
    session: &EncryptedUploadSession,
    options: &DispatchUploadOptions,
    bridge_id: &str,
    encrypted: &EncryptedFrame,
    sequence: u32,
    final_frame: bool,
    sent_at_ms: u64,
    round_trip: &mut F,
) -> Result<u64, CreatorError>
where
    F: FnMut(&str, CreatorBridgeRequest) -> Result<CreatorBridgeResponse, CreatorError>,
{
    let ciphertext = serde_json::to_vec(encrypted).map_err(|error| {
        CreatorError::Protocol(format!(
            "failed to serialize encrypted upload frame: {error}"
        ))
    })?;
    let frame = BridgeData {
        chain_id: options.chain_id.clone(),
        session_id: session.session_id.clone(),
        frame_id: if sequence == MANIFEST_CHUNK_INDEX {
            format!("{}-manifest", session.session_id)
        } else {
            format!("{}-chunk-{sequence:06}", session.session_id)
        },
        sequence,
        sent_at_ms,
        ciphertext,
        final_frame,
    };
    let bridge = session
        .local_dht_snapshot
        .bridge_entries
        .iter()
        .find(|entry| entry.bridge_id == bridge_id)
        .ok_or_else(|| {
            CreatorError::FrameUploadFailed(format!(
                "selected bridge `{bridge_id}` is missing from session DHT snapshot"
            ))
        })?;
    let address = bridge_upload_address(bridge)?;
    match round_trip(&address, CreatorBridgeRequest::Frame(frame))? {
        CreatorBridgeResponse::Ack(ack) if !matches!(ack.status, BridgeAckStatus::Rejected) => {
            Ok(ack.acked_at_ms.max(sent_at_ms))
        }
        CreatorBridgeResponse::Ack(ack) => Err(CreatorError::FrameUploadFailed(format!(
            "bridge `{bridge_id}` rejected upload chunk sequence {} for session {}",
            ack.acked_sequence, ack.session_id
        ))),
        CreatorBridgeResponse::Error { message } => Err(CreatorError::FrameUploadFailed(message)),
        other => Err(CreatorError::FrameUploadFailed(format!(
            "unexpected bridge frame response: {other:?}"
        ))),
    }
}

fn close_active_lanes<F>(
    session: &EncryptedUploadSession,
    options: &DispatchUploadOptions,
    plan: &UploadDispatchPlan,
    round_trip: &mut F,
    clock: &mut LogicalClock,
) where
    F: FnMut(&str, CreatorBridgeRequest) -> Result<CreatorBridgeResponse, CreatorError>,
{
    let mut closed = BTreeSet::new();
    for lane in &plan.lanes {
        if lane.is_failed() || !closed.insert(lane.bridge_id.clone()) {
            continue;
        }
        let Some(bridge) = session
            .local_dht_snapshot
            .bridge_entries
            .iter()
            .find(|entry| entry.bridge_id == lane.bridge_id)
        else {
            continue;
        };
        let Ok(address) = bridge_upload_address(bridge) else {
            continue;
        };
        let close = BridgeClose {
            chain_id: options.chain_id.clone(),
            session_id: session.session_id.clone(),
            closed_at_ms: clock.tick(),
            reason: BridgeCloseReason::Completed,
        };
        let _ = round_trip(&address, CreatorBridgeRequest::Close(close));
    }
}

fn all_remaining_chunks(next_chunk: usize, total_chunks: u32) -> Vec<u32> {
    (next_chunk as u32..total_chunks).collect()
}

fn bridge_upload_address(entry: &BridgeDhtEntry) -> Result<String, CreatorError> {
    let endpoint = entry
        .ingress_endpoints
        .iter()
        .find(|endpoint| {
            endpoint.port != 0
                && !endpoint.ip_addr.trim().is_empty()
                && matches!(
                    endpoint.kind,
                    gbn_bridge_protocol::BridgeIngressEndpointKind::Direct
                        | gbn_bridge_protocol::BridgeIngressEndpointKind::Brokered
                )
        })
        .or_else(|| entry.ingress_endpoints.first())
        .ok_or_else(|| {
            CreatorError::Protocol(format!(
                "bridge `{}` has no ingress endpoint in local DHT",
                entry.bridge_id
            ))
        })?;
    Ok(format!("{}:{}", endpoint.ip_addr, endpoint.port))
}

struct LogicalClock {
    current_ms: u64,
}

impl LogicalClock {
    fn new(start_ms: u64) -> Self {
        Self {
            current_ms: start_ms,
        }
    }

    fn tick(&mut self) -> u64 {
        self.current_ms = self.current_ms.saturating_add(1);
        self.current_ms
    }

    fn current(&self) -> u64 {
        self.current_ms
    }
}
