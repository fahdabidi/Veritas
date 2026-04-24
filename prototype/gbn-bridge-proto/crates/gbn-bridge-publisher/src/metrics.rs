#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
