use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BootstrapProgress, BridgeAck, BridgeCatalogRequest, BridgeCatalogResponse,
    BridgeClose, BridgeData, BridgeHeartbeat, BridgeLease, BridgeOpen, BridgeRegister,
    BridgeRevoke, CreatorJoinRequest, PublicKeyBytes, ReachabilityClass, RevocationReason,
};
use serde::Serialize;

use crate::api::{AuthorityApiResponse, AuthorityApiResponseUnsigned};
use crate::batching::{self, FinalizedBatch};
use crate::bootstrap::{self, AuthorityBootstrapPlan};
use crate::catalog;
use crate::ingest;
use crate::lease;
use crate::metrics::{AuthorityMetrics, AuthorityMetricsSnapshot};
use crate::storage::{InMemoryAuthorityStorage, UploadSessionRecord};
use crate::{AuthorityConfig, AuthorityPolicy, AuthorityResult};

#[derive(Debug)]
pub struct PublisherAuthority {
    signing_key: SigningKey,
    publisher_pub: PublicKeyBytes,
    config: AuthorityConfig,
    policy: AuthorityPolicy,
    storage: InMemoryAuthorityStorage,
    metrics: AuthorityMetrics,
}

impl PublisherAuthority {
    pub fn new(signing_key: SigningKey) -> Self {
        Self::with_config(
            signing_key,
            AuthorityConfig::default(),
            AuthorityPolicy::default(),
        )
    }

    pub fn with_config(
        signing_key: SigningKey,
        config: AuthorityConfig,
        policy: AuthorityPolicy,
    ) -> Self {
        let publisher_pub = publisher_identity(&signing_key);
        Self {
            signing_key,
            publisher_pub,
            config,
            policy,
            storage: InMemoryAuthorityStorage::default(),
            metrics: AuthorityMetrics::default(),
        }
    }

    pub fn publisher_public_key(&self) -> &PublicKeyBytes {
        &self.publisher_pub
    }

    pub fn sign_api_response<T>(
        &self,
        unsigned: AuthorityApiResponseUnsigned<T>,
    ) -> AuthorityResult<AuthorityApiResponse<T>>
    where
        T: Serialize + Clone,
    {
        AuthorityApiResponse::sign(unsigned, &self.signing_key).map_err(Into::into)
    }

    pub fn metrics_snapshot(&self) -> AuthorityMetricsSnapshot {
        self.metrics.snapshot()
    }

    pub fn active_bridge_count(&self, now_ms: u64) -> usize {
        crate::registry::active_bridge_records(&self.storage, now_ms, false).len()
    }

    pub fn bridge_identity_pub(&self, bridge_id: &str) -> Option<PublicKeyBytes> {
        self.storage
            .bridges
            .get(bridge_id)
            .map(|record| record.identity_pub.clone())
    }

    pub fn current_batch_size(&self) -> usize {
        self.storage
            .current_batch
            .as_ref()
            .map(|batch| batch.assignments.len())
            .unwrap_or(0)
    }

    pub fn register_bridge(
        &mut self,
        request: BridgeRegister,
        reachability_class: ReachabilityClass,
        now_ms: u64,
    ) -> AuthorityResult<BridgeLease> {
        let result = lease::register_bridge(
            &mut self.storage,
            &self.signing_key,
            &self.config,
            request,
            reachability_class,
            now_ms,
        );
        match &result {
            Ok(_) => self.metrics.record_registration_success(),
            Err(_) => self.metrics.record_registration_rejection(),
        }
        result
    }

    pub fn reclassify_bridge(
        &mut self,
        bridge_id: &str,
        reachability_class: ReachabilityClass,
        udp_punch_port: Option<u16>,
        now_ms: u64,
    ) -> AuthorityResult<BridgeLease> {
        lease::reclassify_bridge(
            &mut self.storage,
            &self.signing_key,
            &self.config,
            bridge_id,
            reachability_class,
            udp_punch_port,
            now_ms,
        )
    }

    pub fn handle_heartbeat(&mut self, heartbeat: BridgeHeartbeat) -> AuthorityResult<BridgeLease> {
        let result = lease::handle_heartbeat(
            &mut self.storage,
            &self.signing_key,
            &self.config,
            heartbeat,
        );
        if result.is_ok() {
            self.metrics.record_heartbeat();
        }
        result
    }

    pub fn revoke_bridge(
        &mut self,
        bridge_id: &str,
        reason: RevocationReason,
        now_ms: u64,
    ) -> AuthorityResult<BridgeRevoke> {
        let revoke = lease::revoke_bridge(
            &mut self.storage,
            &self.signing_key,
            bridge_id,
            reason,
            now_ms,
        )?;
        self.metrics.record_revocation();
        Ok(revoke)
    }

    pub fn issue_catalog(
        &mut self,
        request: &BridgeCatalogRequest,
        now_ms: u64,
    ) -> AuthorityResult<BridgeCatalogResponse> {
        let response = catalog::issue_catalog(
            &mut self.storage,
            &self.signing_key,
            &self.config,
            &self.policy,
            request,
            now_ms,
        )?;
        self.metrics.record_catalog();
        Ok(response)
    }

    pub fn begin_bootstrap(
        &mut self,
        request: CreatorJoinRequest,
        now_ms: u64,
    ) -> AuthorityResult<AuthorityBootstrapPlan> {
        self.metrics.record_bootstrap_request();
        let result = bootstrap::begin_bootstrap(
            &mut self.storage,
            &self.signing_key,
            &self.publisher_pub,
            &self.config,
            &self.policy,
            request,
            now_ms,
        );
        if result.is_err() {
            self.metrics.record_bootstrap_rejection();
        }
        result
    }

    pub fn enqueue_join_request_for_batch(
        &mut self,
        request: CreatorJoinRequest,
        now_ms: u64,
    ) -> AuthorityResult<Option<FinalizedBatch>> {
        let result = batching::enqueue_join_request(
            &mut self.storage,
            &self.signing_key,
            &self.config,
            &self.policy,
            request,
            now_ms,
        )?;
        if result.is_some() {
            self.metrics.record_batch_rollover();
            self.metrics.record_batch_emitted();
        }
        Ok(result)
    }

    pub fn flush_ready_batch(&mut self, now_ms: u64) -> AuthorityResult<Option<FinalizedBatch>> {
        let result = batching::flush_ready_batch(
            &mut self.storage,
            &self.signing_key,
            &self.config,
            &self.policy,
            now_ms,
        )?;
        if result.is_some() {
            self.metrics.record_batch_emitted();
        }
        Ok(result)
    }

    pub fn open_bridge_session(&mut self, open: BridgeOpen) -> AuthorityResult<()> {
        ingest::open_session(&mut self.storage, open)
    }

    pub fn ingest_bridge_frame(
        &mut self,
        via_bridge_id: &str,
        frame: BridgeData,
        received_at_ms: u64,
    ) -> AuthorityResult<BridgeAck> {
        ingest::ingest_frame(&mut self.storage, via_bridge_id, frame, received_at_ms)
    }

    pub fn close_bridge_session(&mut self, close: BridgeClose) -> AuthorityResult<()> {
        ingest::close_session(&mut self.storage, close)
    }

    pub fn report_bootstrap_progress(
        &mut self,
        progress: BootstrapProgress,
    ) -> AuthorityResult<usize> {
        let session = self
            .storage
            .bootstrap_sessions
            .get_mut(&progress.bootstrap_session_id)
            .ok_or_else(|| crate::AuthorityError::BootstrapSessionNotFound {
                bootstrap_session_id: progress.bootstrap_session_id.clone(),
            })?;
        session.progress_events.push(progress);
        self.metrics.record_progress_report();
        Ok(session.progress_events.len())
    }

    pub fn upload_session(&self, session_id: &str) -> Option<&UploadSessionRecord> {
        self.storage.upload_sessions.get(session_id)
    }
}
