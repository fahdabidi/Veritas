use gbn_bridge_protocol::{
    BridgeBatchAssign, BridgeCatalogResponse, BridgeCommandAckStatus, BridgeCommandPayload,
    BridgeControlCommand, BridgePunchStart, BridgeRevoke, BridgeSeedAssign,
};

use crate::storage::{BridgeCommandRecord, InMemoryAuthorityStorage};

pub fn queue_seed_punch_command(
    storage: &mut InMemoryAuthorityStorage,
    chain_id: &str,
    issued_at_ms: u64,
    payload: BridgePunchStart,
) -> BridgeCommandRecord {
    let bridge_id = payload.initiator_id.clone();
    queue_bridge_command(
        storage,
        &bridge_id,
        chain_id,
        issued_at_ms,
        BridgeCommandPayload::PunchStart(payload),
    )
}

pub fn queue_seed_assignment_command(
    storage: &mut InMemoryAuthorityStorage,
    chain_id: &str,
    issued_at_ms: u64,
    payload: BridgeSeedAssign,
) -> BridgeCommandRecord {
    let bridge_id = payload.seed_bridge_id.clone();
    queue_bridge_command(
        storage,
        &bridge_id,
        chain_id,
        issued_at_ms,
        BridgeCommandPayload::SeedAssign(payload),
    )
}

pub fn queue_batch_assignment_command(
    storage: &mut InMemoryAuthorityStorage,
    chain_id: &str,
    issued_at_ms: u64,
    payload: BridgeBatchAssign,
) -> BridgeCommandRecord {
    let bridge_id = payload.bridge_id.clone();
    queue_bridge_command(
        storage,
        &bridge_id,
        chain_id,
        issued_at_ms,
        BridgeCommandPayload::BatchAssign(payload),
    )
}

pub fn queue_revoke_command(
    storage: &mut InMemoryAuthorityStorage,
    chain_id: &str,
    issued_at_ms: u64,
    payload: BridgeRevoke,
) -> BridgeCommandRecord {
    let bridge_id = payload.bridge_id.clone();
    queue_bridge_command(
        storage,
        &bridge_id,
        chain_id,
        issued_at_ms,
        BridgeCommandPayload::Revoke(payload),
    )
}

pub fn queue_catalog_refresh_command(
    storage: &mut InMemoryAuthorityStorage,
    bridge_id: &str,
    chain_id: &str,
    issued_at_ms: u64,
    payload: BridgeCatalogResponse,
) -> BridgeCommandRecord {
    queue_bridge_command(
        storage,
        bridge_id,
        chain_id,
        issued_at_ms,
        BridgeCommandPayload::CatalogRefresh(payload),
    )
}

pub fn wire_command(session_id: &str, record: &BridgeCommandRecord) -> BridgeControlCommand {
    BridgeControlCommand {
        session_id: session_id.to_string(),
        bridge_id: record.bridge_id.clone(),
        command_id: record.command_id.clone(),
        seq_no: record.seq_no,
        issued_at_ms: record.issued_at_ms,
        chain_id: record.chain_id.clone(),
        payload: record.payload.clone(),
    }
}

pub fn mark_command_sent(record: &mut BridgeCommandRecord, sent_at_ms: u64) {
    record.sent_count = record.sent_count.saturating_add(1);
    record.last_sent_at_ms = Some(sent_at_ms);
}

pub fn mark_command_acked(
    record: &mut BridgeCommandRecord,
    status: BridgeCommandAckStatus,
    acked_at_ms: u64,
) {
    record.ack_status = Some(status);
    record.acked_at_ms = Some(acked_at_ms);
}

fn queue_bridge_command(
    storage: &mut InMemoryAuthorityStorage,
    bridge_id: &str,
    chain_id: &str,
    issued_at_ms: u64,
    payload: BridgeCommandPayload,
) -> BridgeCommandRecord {
    let seq_no = storage.next_bridge_command_seq(bridge_id);
    let command_id = format!("cmd-{bridge_id}-{seq_no:06}");
    let record = BridgeCommandRecord {
        command_id: command_id.clone(),
        bridge_id: bridge_id.to_string(),
        seq_no,
        issued_at_ms,
        chain_id: chain_id.to_string(),
        payload,
        sent_count: 0,
        last_sent_at_ms: None,
        acked_at_ms: None,
        ack_status: None,
    };
    storage.bridge_commands.insert(command_id, record.clone());
    record
}
