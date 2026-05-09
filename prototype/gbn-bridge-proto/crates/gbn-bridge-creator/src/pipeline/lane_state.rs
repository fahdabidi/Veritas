use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneStatus {
    Pending,
    Active,
    SendingChunk { chunk_index: u32, sent_at_ms: u64 },
    Failed,
    Drained,
}

impl Default for LaneStatus {
    fn default() -> Self {
        Self::Pending
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneState {
    pub bridge_id: String,
    #[serde(default)]
    pub status: LaneStatus,
    #[serde(default)]
    pub chunks_sent: Vec<u32>,
    #[serde(default)]
    pub chunks_acked: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_sent_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ack_at_ms: Option<u64>,
}

impl LaneState {
    pub fn pending(bridge_id: impl Into<String>, open_sent_at_ms: u64) -> Self {
        Self {
            bridge_id: bridge_id.into(),
            status: LaneStatus::Pending,
            chunks_sent: Vec::new(),
            chunks_acked: Vec::new(),
            open_sent_at_ms: Some(open_sent_at_ms),
            active_at_ms: None,
            failed_at_ms: None,
            last_ack_at_ms: None,
        }
    }

    pub fn mark_active(&mut self, active_at_ms: u64) {
        self.status = LaneStatus::Active;
        self.active_at_ms = Some(active_at_ms);
    }

    pub fn mark_sending(&mut self, chunk_index: u32, sent_at_ms: u64) {
        self.status = LaneStatus::SendingChunk {
            chunk_index,
            sent_at_ms,
        };
        self.chunks_sent.push(chunk_index);
    }

    pub fn mark_acked(&mut self, chunk_index: u32, ack_at_ms: u64) {
        if !self.chunks_acked.contains(&chunk_index) {
            self.chunks_acked.push(chunk_index);
        }
        self.last_ack_at_ms = Some(ack_at_ms);
        self.status = LaneStatus::Active;
    }

    pub fn mark_failed(&mut self, failed_at_ms: u64) {
        self.status = LaneStatus::Failed;
        self.failed_at_ms = Some(failed_at_ms);
    }

    pub fn mark_drained(&mut self) {
        if !matches!(self.status, LaneStatus::Failed) {
            self.status = LaneStatus::Drained;
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self.status, LaneStatus::Active)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self.status, LaneStatus::Failed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkAssignment {
    pub chunk_index: u32,
    pub assigned_bridge_id: String,
    pub attempts: u32,
    pub first_dispatch_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ack_at_ms: Option<u64>,
}
