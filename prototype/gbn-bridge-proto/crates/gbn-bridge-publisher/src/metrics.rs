use serde::{Deserialize, Serialize};

use crate::metrics_emitter::metric_data;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityMetricsSnapshot {
    pub successful_registrations: u64,
    pub rejected_registrations: u64,
    pub heartbeats: u64,
    pub revocations: u64,
    pub issued_catalogs: u64,
    pub bootstrap_requests: u64,
    pub rejected_bootstrap_requests: u64,
    pub bootstrap_progress_reports: u64,
    pub issued_batches: u64,
    pub batch_rollovers: u64,
}

impl AuthorityMetricsSnapshot {
    pub fn cloudwatch_data(
        &self,
        service: &str,
        stack: &str,
    ) -> Vec<aws_sdk_cloudwatch::types::MetricDatum> {
        [
            (
                "SuccessfulRegistrations",
                self.successful_registrations as f64,
            ),
            ("RejectedRegistrations", self.rejected_registrations as f64),
            ("Heartbeats", self.heartbeats as f64),
            ("Revocations", self.revocations as f64),
            ("IssuedCatalogs", self.issued_catalogs as f64),
            ("BootstrapRequests", self.bootstrap_requests as f64),
            (
                "RejectedBootstrapRequests",
                self.rejected_bootstrap_requests as f64,
            ),
            (
                "BootstrapProgressReports",
                self.bootstrap_progress_reports as f64,
            ),
            ("IssuedBatches", self.issued_batches as f64),
            ("BatchRollovers", self.batch_rollovers as f64),
        ]
        .into_iter()
        .map(|(name, value)| metric_data(name, value, service, stack))
        .collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct AuthorityMetrics {
    snapshot: AuthorityMetricsSnapshot,
}

impl AuthorityMetrics {
    pub fn snapshot(&self) -> AuthorityMetricsSnapshot {
        self.snapshot
    }

    pub fn record_registration_success(&mut self) {
        self.snapshot.successful_registrations += 1;
    }

    pub fn record_registration_rejection(&mut self) {
        self.snapshot.rejected_registrations += 1;
    }

    pub fn record_heartbeat(&mut self) {
        self.snapshot.heartbeats += 1;
    }

    pub fn record_revocation(&mut self) {
        self.snapshot.revocations += 1;
    }

    pub fn record_catalog(&mut self) {
        self.snapshot.issued_catalogs += 1;
    }

    pub fn record_bootstrap_request(&mut self) {
        self.snapshot.bootstrap_requests += 1;
    }

    pub fn record_bootstrap_rejection(&mut self) {
        self.snapshot.rejected_bootstrap_requests += 1;
    }

    pub fn record_progress_report(&mut self) {
        self.snapshot.bootstrap_progress_reports += 1;
    }

    pub fn record_batch_emitted(&mut self) {
        self.snapshot.issued_batches += 1;
    }

    pub fn record_batch_rollover(&mut self) {
        self.snapshot.batch_rollovers += 1;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiverMetricsSnapshot {
    pub frames_accepted: u64,
    pub frames_rejected: u64,
    pub bytes_ingested: u64,
    pub sessions_opened: u64,
    pub sessions_closed: u64,
}

impl ReceiverMetricsSnapshot {
    pub fn cloudwatch_data(
        &self,
        service: &str,
        stack: &str,
    ) -> Vec<aws_sdk_cloudwatch::types::MetricDatum> {
        [
            ("FramesAccepted", self.frames_accepted as f64),
            ("FramesRejected", self.frames_rejected as f64),
            ("BytesIngested", self.bytes_ingested as f64),
            ("SessionsOpened", self.sessions_opened as f64),
            ("SessionsClosed", self.sessions_closed as f64),
        ]
        .into_iter()
        .map(|(name, value)| metric_data(name, value, service, stack))
        .collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ReceiverMetrics {
    snapshot: ReceiverMetricsSnapshot,
}

impl ReceiverMetrics {
    pub fn snapshot(&self) -> ReceiverMetricsSnapshot {
        self.snapshot
    }

    pub fn record_session_opened(&mut self) {
        self.snapshot.sessions_opened += 1;
    }

    pub fn record_session_closed(&mut self) {
        self.snapshot.sessions_closed += 1;
    }

    pub fn record_frame_accepted(&mut self, bytes: usize) {
        self.snapshot.frames_accepted += 1;
        self.snapshot.bytes_ingested = self.snapshot.bytes_ingested.saturating_add(bytes as u64);
    }

    pub fn record_frame_rejected(&mut self) {
        self.snapshot.frames_rejected += 1;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeMetricsSnapshot {
    pub commands_received: u64,
    pub commands_acked: u64,
    pub commands_rejected: u64,
    pub frames_forwarded: u64,
    pub bytes_forwarded: u64,
    pub control_reconnects: u64,
}

impl BridgeMetricsSnapshot {
    pub fn cloudwatch_data(
        &self,
        service: &str,
        stack: &str,
    ) -> Vec<aws_sdk_cloudwatch::types::MetricDatum> {
        [
            ("CommandsReceived", self.commands_received as f64),
            ("CommandsAcked", self.commands_acked as f64),
            ("CommandsRejected", self.commands_rejected as f64),
            ("FramesForwarded", self.frames_forwarded as f64),
            ("BytesForwarded", self.bytes_forwarded as f64),
            ("ControlReconnects", self.control_reconnects as f64),
        ]
        .into_iter()
        .map(|(name, value)| metric_data(name, value, service, stack))
        .collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct BridgeMetrics {
    snapshot: BridgeMetricsSnapshot,
}

impl BridgeMetrics {
    pub fn snapshot(&self) -> BridgeMetricsSnapshot {
        self.snapshot
    }

    pub fn record_command_ack(&mut self, rejected: bool) {
        self.snapshot.commands_received += 1;
        self.snapshot.commands_acked += 1;
        if rejected {
            self.snapshot.commands_rejected += 1;
        }
    }

    pub fn record_control_reconnect(&mut self) {
        self.snapshot.control_reconnects += 1;
    }

    pub fn record_frame_forwarded(&mut self, bytes: usize) {
        self.snapshot.frames_forwarded += 1;
        self.snapshot.bytes_forwarded = self.snapshot.bytes_forwarded.saturating_add(bytes as u64);
    }
}
