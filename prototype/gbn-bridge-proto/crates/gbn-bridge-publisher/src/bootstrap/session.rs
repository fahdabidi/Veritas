use gbn_bridge_protocol::{BridgeSetResponse, CreatorBootstrapResponse};

use crate::storage::{BootstrapSessionRecord, BootstrapSessionState};
use crate::AuthorityConfig;

pub fn build_session_record(
    config: &AuthorityConfig,
    chain_id: &str,
    creator_request_id: &str,
    creator_entry: gbn_bridge_protocol::BootstrapDhtEntry,
    creator_response: CreatorBootstrapResponse,
    bridge_set: BridgeSetResponse,
    host_creator_id: String,
    relay_bridge_id: String,
    seed_bridge_id: String,
    bridge_ids: Vec<String>,
    created_at_ms: u64,
) -> BootstrapSessionRecord {
    BootstrapSessionRecord {
        bootstrap_session_id: creator_response.bootstrap_session_id.clone(),
        chain_id: chain_id.to_string(),
        creator_request_id: creator_request_id.to_string(),
        creator_entry,
        creator_response,
        bridge_set,
        host_creator_id,
        relay_bridge_id,
        seed_bridge_id: seed_bridge_id.clone(),
        bridge_ids,
        attempted_seed_bridge_ids: vec![seed_bridge_id],
        state: BootstrapSessionState::SeedAssigned,
        created_at_ms,
        seed_assigned_at_ms: Some(created_at_ms),
        seed_acknowledged_at_ms: None,
        response_returned_at_ms: None,
        seed_tunnel_reported_at_ms: None,
        bridge_set_delivered_at_ms: None,
        fanout_activated_at_ms: None,
        completed_at_ms: None,
        expired_at_ms: None,
        failed_at_ms: None,
        reassigned_at_ms: None,
        response_expiry_ms: created_at_ms + config.bootstrap_response_ttl_ms,
        seed_ack_deadline_ms: created_at_ms + config.bootstrap_seed_ack_timeout_ms,
        seed_tunnel_deadline_ms: created_at_ms + config.bootstrap_seed_tunnel_timeout_ms,
        bridge_set_delivery_deadline_ms: created_at_ms + config.bootstrap_bridge_set_timeout_ms,
        reassignment_count: 0,
        max_reassignment_count: config.bootstrap_max_reassignments,
        progress_events: Vec::new(),
    }
}

pub fn mark_response_returned(record: &mut BootstrapSessionRecord, returned_at_ms: u64) {
    record.response_returned_at_ms = Some(returned_at_ms);
    if matches!(
        record.state,
        BootstrapSessionState::Created | BootstrapSessionState::SeedAssigned
    ) {
        record.state = BootstrapSessionState::BootstrapResponseReturned;
    }
}

pub fn mark_seed_acknowledged(record: &mut BootstrapSessionRecord, acked_at_ms: u64) {
    record.seed_acknowledged_at_ms = Some(acked_at_ms);
    record.state = BootstrapSessionState::SeedAcknowledged;
}

pub fn mark_seed_tunnel_reported(record: &mut BootstrapSessionRecord, reported_at_ms: u64) {
    record.seed_tunnel_reported_at_ms = Some(reported_at_ms);
    record.state = BootstrapSessionState::SeedTunnelReported;
}

pub fn mark_bridge_set_delivered(record: &mut BootstrapSessionRecord, delivered_at_ms: u64) {
    record.bridge_set_delivered_at_ms = Some(delivered_at_ms);
    record.state = BootstrapSessionState::BridgeSetDelivered;
}

pub fn mark_fanout_activated(record: &mut BootstrapSessionRecord, activated_at_ms: u64) {
    record.fanout_activated_at_ms = Some(activated_at_ms);
    record.state = BootstrapSessionState::FanoutActivated;
}

pub fn mark_completed(record: &mut BootstrapSessionRecord, completed_at_ms: u64) {
    record.completed_at_ms = Some(completed_at_ms);
    record.state = BootstrapSessionState::Completed;
}

pub fn mark_failed(record: &mut BootstrapSessionRecord, failed_at_ms: u64) {
    record.failed_at_ms = Some(failed_at_ms);
    record.state = BootstrapSessionState::Failed;
}

pub fn mark_reassigned(
    record: &mut BootstrapSessionRecord,
    new_seed_bridge_id: &str,
    reassigned_at_ms: u64,
    seed_ack_deadline_ms: u64,
    seed_tunnel_deadline_ms: u64,
) {
    record.seed_bridge_id = new_seed_bridge_id.to_string();
    if !record
        .attempted_seed_bridge_ids
        .iter()
        .any(|bridge_id| bridge_id == new_seed_bridge_id)
    {
        record
            .attempted_seed_bridge_ids
            .push(new_seed_bridge_id.to_string());
    }
    record.reassignment_count = record.reassignment_count.saturating_add(1);
    record.reassigned_at_ms = Some(reassigned_at_ms);
    record.seed_acknowledged_at_ms = None;
    record.seed_tunnel_reported_at_ms = None;
    record.bridge_set_delivered_at_ms = None;
    record.fanout_activated_at_ms = None;
    record.seed_ack_deadline_ms = seed_ack_deadline_ms;
    record.seed_tunnel_deadline_ms = seed_tunnel_deadline_ms;
    record.state = BootstrapSessionState::Reassigned;
}

pub fn should_reassign_seed(record: &BootstrapSessionRecord, now_ms: u64) -> bool {
    matches!(
        record.state,
        BootstrapSessionState::SeedAssigned
            | BootstrapSessionState::BootstrapResponseReturned
            | BootstrapSessionState::SeedAcknowledged
            | BootstrapSessionState::Reassigned
    ) && (record.seed_acknowledged_at_ms.is_none() && now_ms > record.seed_ack_deadline_ms
        || record.seed_acknowledged_at_ms.is_some()
            && record.seed_tunnel_reported_at_ms.is_none()
            && now_ms > record.seed_tunnel_deadline_ms)
}
