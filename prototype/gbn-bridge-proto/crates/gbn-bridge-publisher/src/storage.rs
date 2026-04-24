use std::collections::BTreeMap;

use gbn_bridge_protocol::{
    BootstrapDhtEntry, BootstrapProgress, BridgeCapability, BridgeCloseReason, BridgeData,
    BridgeHeartbeat, BridgeIngressEndpoint, BridgeLease, BridgeOpen, CreatorJoinRequest,
    PublicKeyBytes, ReachabilityClass, RevocationReason, UnixTimestampMs,
};

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct BootstrapSessionRecord {
    pub bootstrap_session_id: String,
    pub creator_entry: BootstrapDhtEntry,
    pub host_creator_id: String,
    pub relay_bridge_id: String,
    pub seed_bridge_id: String,
    pub bridge_ids: Vec<String>,
    pub created_at_ms: UnixTimestampMs,
    pub response_expiry_ms: UnixTimestampMs,
    pub progress_events: Vec<BootstrapProgress>,
}

#[derive(Debug, Clone)]
pub struct PendingBatchAssignment {
    pub bootstrap_session_id: String,
    pub join_request: CreatorJoinRequest,
    pub creator_entry: BootstrapDhtEntry,
}

#[derive(Debug, Clone)]
pub struct BatchWindowState {
    pub batch_id: String,
    pub window_started_at_ms: UnixTimestampMs,
    pub assignments: Vec<PendingBatchAssignment>,
}

#[derive(Debug, Clone)]
pub struct IngestedFrameRecord {
    pub via_bridge_id: String,
    pub frame: BridgeData,
    pub received_at_ms: UnixTimestampMs,
}

#[derive(Debug, Clone)]
pub struct UploadSessionRecord {
    pub session_id: String,
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

#[derive(Debug, Default)]
pub struct InMemoryAuthorityStorage {
    pub bridges: BTreeMap<String, BridgeRecord>,
    pub bootstrap_sessions: BTreeMap<String, BootstrapSessionRecord>,
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
}
