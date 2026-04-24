use std::collections::BTreeMap;

use gbn_bridge_protocol::{
    BootstrapDhtEntry, BootstrapProgress, BridgeCapability, BridgeCatalogResponse,
    BridgeCloseReason, BridgeCommandAckStatus, BridgeCommandPayload, BridgeData, BridgeHeartbeat,
    BridgeIngressEndpoint, BridgeLease, BridgeOpen, BridgeSetResponse, CreatorBootstrapResponse,
    CreatorJoinRequest, PublicKeyBytes, ReachabilityClass, RevocationReason, UnixTimestampMs,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod postgres;
pub mod recovery;
pub mod schema;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum StorageError {
    #[error("storage configuration error: {0}")]
    Config(String),

    #[error("storage backend error: {0}")]
    Backend(String),

    #[error("storage serialization error: {0}")]
    Serialization(String),
}

pub type StorageResult<T> = Result<T, StorageError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeRecord {
    pub bridge_id: String,
    pub identity_pub: PublicKeyBytes,
    pub ingress_endpoints: Vec<BridgeIngressEndpoint>,
    pub assigned_udp_punch_port: u16,
    pub reachability_class: ReachabilityClass,
    pub capabilities: Vec<BridgeCapability>,
    pub current_lease: BridgeLease,
    pub last_heartbeat: BridgeHeartbeat,
    pub revoked_reason: Option<RevocationReason>,
    pub revoked_at_ms: Option<UnixTimestampMs>,
}

impl BridgeRecord {
    pub fn is_active(&self, now_ms: UnixTimestampMs) -> bool {
        self.revoked_reason.is_none() && self.current_lease.lease_expiry_ms >= now_ms
    }

    pub fn is_direct(&self) -> bool {
        self.reachability_class == ReachabilityClass::Direct
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapSessionState {
    Created,
    SeedAssigned,
    SeedAcknowledged,
    BootstrapResponseReturned,
    SeedTunnelReported,
    BridgeSetDelivered,
    FanoutActivated,
    Completed,
    Expired,
    Reassigned,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapSessionRecord {
    pub bootstrap_session_id: String,
    pub chain_id: String,
    pub creator_request_id: String,
    pub creator_entry: BootstrapDhtEntry,
    pub creator_response: CreatorBootstrapResponse,
    pub bridge_set: BridgeSetResponse,
    pub host_creator_id: String,
    pub relay_bridge_id: String,
    pub seed_bridge_id: String,
    pub bridge_ids: Vec<String>,
    pub attempted_seed_bridge_ids: Vec<String>,
    pub state: BootstrapSessionState,
    pub created_at_ms: UnixTimestampMs,
    pub seed_assigned_at_ms: Option<UnixTimestampMs>,
    pub seed_acknowledged_at_ms: Option<UnixTimestampMs>,
    pub response_returned_at_ms: Option<UnixTimestampMs>,
    pub seed_tunnel_reported_at_ms: Option<UnixTimestampMs>,
    pub bridge_set_delivered_at_ms: Option<UnixTimestampMs>,
    pub fanout_activated_at_ms: Option<UnixTimestampMs>,
    pub completed_at_ms: Option<UnixTimestampMs>,
    pub expired_at_ms: Option<UnixTimestampMs>,
    pub failed_at_ms: Option<UnixTimestampMs>,
    pub reassigned_at_ms: Option<UnixTimestampMs>,
    pub response_expiry_ms: UnixTimestampMs,
    pub seed_ack_deadline_ms: UnixTimestampMs,
    pub seed_tunnel_deadline_ms: UnixTimestampMs,
    pub bridge_set_delivery_deadline_ms: UnixTimestampMs,
    pub reassignment_count: u32,
    pub max_reassignment_count: u32,
    pub progress_events: Vec<BootstrapProgress>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingBatchAssignment {
    pub bootstrap_session_id: String,
    pub chain_id: Option<String>,
    pub join_request: CreatorJoinRequest,
    pub creator_entry: BootstrapDhtEntry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchWindowState {
    pub batch_id: String,
    pub window_started_at_ms: UnixTimestampMs,
    pub assignments: Vec<PendingBatchAssignment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestedFrameRecord {
    pub via_bridge_id: String,
    pub chain_id: Option<String>,
    pub frame: BridgeData,
    pub received_at_ms: UnixTimestampMs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadSessionRecord {
    pub session_id: String,
    pub chain_id: Option<String>,
    pub creator_id: String,
    pub creator_session_pub: PublicKeyBytes,
    pub expected_chunks: Option<u16>,
    pub opened_at_ms: UnixTimestampMs,
    pub opened_via_bridges: Vec<String>,
    pub frames_by_sequence: BTreeMap<u32, IngestedFrameRecord>,
    pub frame_id_to_sequence: BTreeMap<String, u32>,
    pub completed_at_ms: Option<UnixTimestampMs>,
    pub closed_at_ms: Option<UnixTimestampMs>,
    pub close_reason: Option<BridgeCloseReason>,
}

impl UploadSessionRecord {
    pub fn new(open: &BridgeOpen) -> Self {
        Self {
            session_id: open.session_id.clone(),
            chain_id: None,
            creator_id: open.creator_id.clone(),
            creator_session_pub: open.creator_session_pub.clone(),
            expected_chunks: open.expected_chunks,
            opened_at_ms: open.opened_at_ms,
            opened_via_bridges: vec![open.bridge_id.clone()],
            frames_by_sequence: BTreeMap::new(),
            frame_id_to_sequence: BTreeMap::new(),
            completed_at_ms: None,
            closed_at_ms: None,
            close_reason: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogIssuanceRecord {
    pub catalog_id: String,
    pub chain_id: Option<String>,
    pub issued_at_ms: UnixTimestampMs,
    pub expires_at_ms: UnixTimestampMs,
    pub response: BridgeCatalogResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeCommandRecord {
    pub command_id: String,
    pub bridge_id: String,
    pub seq_no: u64,
    pub issued_at_ms: UnixTimestampMs,
    pub chain_id: String,
    pub payload: BridgeCommandPayload,
    pub sent_count: u32,
    pub last_sent_at_ms: Option<UnixTimestampMs>,
    pub acked_at_ms: Option<UnixTimestampMs>,
    pub ack_status: Option<BridgeCommandAckStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SequenceState {
    pub next_lease_seq: u64,
    pub next_catalog_seq: u64,
    pub next_bootstrap_seq: u64,
    pub next_batch_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct InMemoryAuthorityStorage {
    pub bridges: BTreeMap<String, BridgeRecord>,
    pub catalog_issuance: BTreeMap<String, CatalogIssuanceRecord>,
    pub bootstrap_sessions: BTreeMap<String, BootstrapSessionRecord>,
    pub bridge_commands: BTreeMap<String, BridgeCommandRecord>,
    pub upload_sessions: BTreeMap<String, UploadSessionRecord>,
    pub current_batch: Option<BatchWindowState>,
    next_lease_seq: u64,
    next_catalog_seq: u64,
    next_bootstrap_seq: u64,
    next_batch_seq: u64,
}

impl InMemoryAuthorityStorage {
    pub fn next_lease_id(&mut self) -> String {
        self.next_lease_seq += 1;
        format!("lease-{:06}", self.next_lease_seq)
    }

    pub fn next_catalog_id(&mut self) -> String {
        self.next_catalog_seq += 1;
        format!("catalog-{:06}", self.next_catalog_seq)
    }

    pub fn next_bootstrap_id(&mut self) -> String {
        self.next_bootstrap_seq += 1;
        format!("bootstrap-{:06}", self.next_bootstrap_seq)
    }

    pub fn next_batch_id(&mut self) -> String {
        self.next_batch_seq += 1;
        format!("batch-{:06}", self.next_batch_seq)
    }

    pub fn sequence_state(&self) -> SequenceState {
        SequenceState {
            next_lease_seq: self.next_lease_seq,
            next_catalog_seq: self.next_catalog_seq,
            next_bootstrap_seq: self.next_bootstrap_seq,
            next_batch_seq: self.next_batch_seq,
        }
    }

    pub fn apply_sequence_state(&mut self, sequence_state: SequenceState) {
        self.next_lease_seq = sequence_state.next_lease_seq;
        self.next_catalog_seq = sequence_state.next_catalog_seq;
        self.next_bootstrap_seq = sequence_state.next_bootstrap_seq;
        self.next_batch_seq = sequence_state.next_batch_seq;
    }

    pub fn record_catalog_issuance(
        &mut self,
        chain_id: Option<String>,
        response: BridgeCatalogResponse,
    ) {
        self.catalog_issuance.insert(
            response.catalog_id.clone(),
            CatalogIssuanceRecord {
                catalog_id: response.catalog_id.clone(),
                chain_id,
                issued_at_ms: response.issued_at_ms,
                expires_at_ms: response.expires_at_ms,
                response,
            },
        );
    }

    pub fn next_bridge_command_seq(&self, bridge_id: &str) -> u64 {
        self.bridge_commands
            .values()
            .filter(|record| record.bridge_id == bridge_id)
            .map(|record| record.seq_no)
            .max()
            .unwrap_or(0)
            + 1
    }
}
