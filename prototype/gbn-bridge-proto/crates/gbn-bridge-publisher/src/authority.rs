use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BootstrapJoinReply, BootstrapProgress, BootstrapProgressStage, BridgeAck,
    BridgeCatalogRequest, BridgeCatalogResponse, BridgeClose, BridgeCommandAck,
    BridgeCommandAckStatus, BridgeCommandPayload, BridgeData, BridgeHeartbeat, BridgeLease,
    BridgeOpen, BridgeRegister, BridgeRevoke, CreatorJoinRequest, PublicKeyBytes,
    ReachabilityClass, RevocationReason,
};
use serde::Serialize;

use crate::api::{AuthorityApiResponse, AuthorityApiResponseUnsigned};
use crate::assignment;
use crate::batching::{self, FinalizedBatch};
use crate::bootstrap::{self, session as bootstrap_session, AuthorityBootstrapPlan};
use crate::catalog;
use crate::ingest;
use crate::lease;
use crate::metrics::{AuthorityMetrics, AuthorityMetricsSnapshot};
use crate::storage::{
    postgres::{PostgresAuthorityStorage, PostgresStorageConfig},
    recovery::RecoverySummary,
    BootstrapSessionRecord, BridgeCommandRecord, BridgeRecord, CatalogIssuanceRecord,
    InMemoryAuthorityStorage, UploadSessionRecord,
};
use crate::{AuthorityConfig, AuthorityError, AuthorityPolicy, AuthorityResult};

#[derive(Debug)]
pub struct PublisherAuthority {
    signing_key: SigningKey,
    publisher_pub: PublicKeyBytes,
    config: AuthorityConfig,
    policy: AuthorityPolicy,
    storage: InMemoryAuthorityStorage,
    durable_storage: Option<PostgresAuthorityStorage>,
    last_recovery_summary: RecoverySummary,
    metrics: AuthorityMetrics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapProgressUpdate {
    pub stored_event_count: usize,
    pub activated_bridge_ids: Vec<String>,
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
            durable_storage: None,
            last_recovery_summary: RecoverySummary::default(),
            metrics: AuthorityMetrics::default(),
        }
    }

    pub fn with_postgres(
        signing_key: SigningKey,
        config: AuthorityConfig,
        policy: AuthorityPolicy,
        postgres_config: PostgresStorageConfig,
        now_ms: u64,
    ) -> AuthorityResult<Self> {
        let publisher_pub = publisher_identity(&signing_key);
        let mut durable_storage = PostgresAuthorityStorage::connect(&postgres_config)?;
        let (storage, last_recovery_summary) = durable_storage.load_state(now_ms)?;
        Ok(Self {
            signing_key,
            publisher_pub,
            config,
            policy,
            storage,
            durable_storage: Some(durable_storage),
            last_recovery_summary,
            metrics: AuthorityMetrics::default(),
        })
    }

    pub fn publisher_public_key(&self) -> &PublicKeyBytes {
        &self.publisher_pub
    }

    pub fn signing_key(&self) -> &SigningKey {
        &self.signing_key
    }

    pub fn storage_is_durable(&self) -> bool {
        self.durable_storage.is_some()
    }

    pub fn last_recovery_summary(&self) -> RecoverySummary {
        self.last_recovery_summary
    }

    pub fn durable_store_healthcheck(&mut self) -> AuthorityResult<()> {
        if let Some(storage) = &mut self.durable_storage {
            storage.is_healthy()?;
        }
        Ok(())
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

    pub fn bridge_record(&self, bridge_id: &str) -> Option<&BridgeRecord> {
        self.storage.bridges.get(bridge_id)
    }

    pub fn last_bridge_command_seq(&self, bridge_id: &str) -> Option<u64> {
        self.storage
            .bridge_commands
            .values()
            .filter(|record| record.bridge_id == bridge_id)
            .map(|record| record.seq_no)
            .max()
    }

    pub fn pending_bridge_commands(&self, bridge_id: &str) -> Vec<BridgeCommandRecord> {
        let mut commands = self
            .storage
            .bridge_commands
            .values()
            .filter(|record| record.bridge_id == bridge_id && record.acked_at_ms.is_none())
            .cloned()
            .collect::<Vec<_>>();
        commands.sort_by_key(|record| record.seq_no);
        commands
    }

    pub fn current_batch_size(&self) -> usize {
        self.storage
            .current_batch
            .as_ref()
            .map(|batch| batch.assignments.len())
            .unwrap_or(0)
    }

    pub fn bootstrap_session(&self, bootstrap_session_id: &str) -> Option<&BootstrapSessionRecord> {
        self.storage.bootstrap_sessions.get(bootstrap_session_id)
    }

    pub fn catalog_issuance(&self, catalog_id: &str) -> Option<&CatalogIssuanceRecord> {
        self.storage.catalog_issuance.get(catalog_id)
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
        if result.is_ok() {
            self.persist_state()?;
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
        let result = lease::reclassify_bridge(
            &mut self.storage,
            &self.signing_key,
            &self.config,
            bridge_id,
            reachability_class,
            udp_punch_port,
            now_ms,
        );
        if result.is_ok() {
            self.persist_state()?;
        }
        result
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
            self.persist_state()?;
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
        self.queue_revoke_command(revoke.clone(), now_ms);
        self.persist_state()?;
        Ok(revoke)
    }

    pub fn issue_catalog(
        &mut self,
        request: &BridgeCatalogRequest,
        now_ms: u64,
    ) -> AuthorityResult<BridgeCatalogResponse> {
        self.issue_catalog_with_chain_id(None, request, now_ms)
    }

    pub fn issue_catalog_with_chain_id(
        &mut self,
        chain_id: Option<&str>,
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
        self.storage
            .record_catalog_issuance(chain_id.map(ToOwned::to_owned), response.clone());
        self.metrics.record_catalog();
        self.persist_state()?;
        Ok(response)
    }

    pub fn begin_bootstrap(
        &mut self,
        request: CreatorJoinRequest,
        now_ms: u64,
    ) -> AuthorityResult<AuthorityBootstrapPlan> {
        let chain_id = request.request_id.clone();
        self.begin_bootstrap_with_chain_id(&chain_id, request, now_ms)
    }

    pub fn begin_bootstrap_with_chain_id(
        &mut self,
        chain_id: &str,
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
            chain_id,
            request,
            now_ms,
        );
        if result.is_err() {
            self.metrics.record_bootstrap_rejection();
        } else {
            if let Ok(plan) = &result {
                self.queue_seed_assignment_command(chain_id, now_ms, plan.seed_assignment.clone());
            }
            self.persist_state()?;
        }
        result
    }

    pub fn begin_bootstrap_reply(
        &mut self,
        request: CreatorJoinRequest,
        now_ms: u64,
    ) -> AuthorityResult<BootstrapJoinReply> {
        let chain_id = request.request_id.clone();
        self.begin_bootstrap_reply_with_chain_id(&chain_id, request, now_ms)
    }

    pub fn begin_bootstrap_reply_with_chain_id(
        &mut self,
        chain_id: &str,
        request: CreatorJoinRequest,
        now_ms: u64,
    ) -> AuthorityResult<BootstrapJoinReply> {
        let plan = self.begin_bootstrap_with_chain_id(chain_id, request, now_ms)?;
        Ok(plan.join_reply())
    }

    pub fn mark_bootstrap_response_returned(
        &mut self,
        chain_id: &str,
        bootstrap_session_id: &str,
        returned_at_ms: u64,
    ) -> AuthorityResult<()> {
        let session = self
            .storage
            .bootstrap_sessions
            .get_mut(bootstrap_session_id)
            .ok_or_else(|| AuthorityError::BootstrapSessionNotFound {
                bootstrap_session_id: bootstrap_session_id.to_string(),
            })?;
        if session.chain_id != chain_id {
            return Err(AuthorityError::ChainIdMismatch {
                context: "bootstrap response return",
                expected: session.chain_id.clone(),
                actual: chain_id.to_string(),
            });
        }
        bootstrap_session::mark_response_returned(session, returned_at_ms);
        self.persist_state()?;
        Ok(())
    }

    pub fn enqueue_join_request_for_batch(
        &mut self,
        request: CreatorJoinRequest,
        now_ms: u64,
    ) -> AuthorityResult<Option<FinalizedBatch>> {
        self.enqueue_join_request_for_batch_with_chain_id(None, request, now_ms)
    }

    pub fn enqueue_join_request_for_batch_with_chain_id(
        &mut self,
        chain_id: Option<&str>,
        request: CreatorJoinRequest,
        now_ms: u64,
    ) -> AuthorityResult<Option<FinalizedBatch>> {
        let result = batching::enqueue_join_request(
            &mut self.storage,
            &self.signing_key,
            &self.config,
            &self.policy,
            chain_id,
            request,
            now_ms,
        )?;
        if result.is_some() {
            self.metrics.record_batch_rollover();
            self.metrics.record_batch_emitted();
            if let Some(batch) = &result {
                self.queue_batch_commands(now_ms, batch);
            }
        }
        self.persist_state()?;
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
            if let Some(batch) = &result {
                self.queue_batch_commands(now_ms, batch);
            }
        }
        if result.is_some() || self.storage.current_batch.is_none() {
            self.persist_state()?;
        }
        Ok(result)
    }

    pub fn open_bridge_session(&mut self, open: BridgeOpen) -> AuthorityResult<()> {
        self.open_bridge_session_with_chain_id(None, open)
    }

    pub fn open_bridge_session_with_chain_id(
        &mut self,
        chain_id: Option<&str>,
        open: BridgeOpen,
    ) -> AuthorityResult<()> {
        let result = ingest::open_session_with_chain_id(&mut self.storage, chain_id, open);
        if result.is_ok() {
            self.persist_state()?;
        }
        result
    }

    pub fn ingest_bridge_frame(
        &mut self,
        via_bridge_id: &str,
        frame: BridgeData,
        received_at_ms: u64,
    ) -> AuthorityResult<BridgeAck> {
        self.ingest_bridge_frame_with_chain_id(None, via_bridge_id, frame, received_at_ms)
    }

    pub fn ingest_bridge_frame_with_chain_id(
        &mut self,
        chain_id: Option<&str>,
        via_bridge_id: &str,
        frame: BridgeData,
        received_at_ms: u64,
    ) -> AuthorityResult<BridgeAck> {
        let ack = ingest::ingest_frame_with_chain_id(
            &mut self.storage,
            chain_id,
            via_bridge_id,
            frame,
            received_at_ms,
        )?;
        self.persist_state()?;
        Ok(ack)
    }

    pub fn close_bridge_session(&mut self, close: BridgeClose) -> AuthorityResult<()> {
        self.close_bridge_session_with_chain_id(None, close)
    }

    pub fn close_bridge_session_with_chain_id(
        &mut self,
        chain_id: Option<&str>,
        close: BridgeClose,
    ) -> AuthorityResult<()> {
        let result = ingest::close_session_with_chain_id(&mut self.storage, chain_id, close);
        if result.is_ok() {
            self.persist_state()?;
        }
        result
    }

    pub fn report_bootstrap_progress(
        &mut self,
        progress: BootstrapProgress,
    ) -> AuthorityResult<BootstrapProgressUpdate> {
        let chain_id = self
            .storage
            .bootstrap_sessions
            .get(&progress.bootstrap_session_id)
            .map(|session| session.chain_id.clone())
            .ok_or_else(|| AuthorityError::BootstrapSessionNotFound {
                bootstrap_session_id: progress.bootstrap_session_id.clone(),
            })?;
        self.report_bootstrap_progress_with_chain_id(&chain_id, progress)
    }

    pub fn report_bootstrap_progress_with_chain_id(
        &mut self,
        chain_id: &str,
        progress: BootstrapProgress,
    ) -> AuthorityResult<BootstrapProgressUpdate> {
        let reported_at_ms = progress.reported_at_ms;
        let mut fanout_session = None;
        let stored_event_count = {
            let session = self
                .storage
                .bootstrap_sessions
                .get_mut(&progress.bootstrap_session_id)
                .ok_or_else(|| crate::AuthorityError::BootstrapSessionNotFound {
                    bootstrap_session_id: progress.bootstrap_session_id.clone(),
                })?;
            if session.chain_id != chain_id {
                return Err(AuthorityError::ChainIdMismatch {
                    context: "bootstrap progress",
                    expected: session.chain_id.clone(),
                    actual: chain_id.to_string(),
                });
            }
            session.progress_events.push(progress);
            let latest_progress = session
                .progress_events
                .last()
                .cloned()
                .expect("progress event was just pushed");
            match latest_progress.stage {
                BootstrapProgressStage::SeedTunnelEstablished
                    if latest_progress.reporter_id == session.seed_bridge_id =>
                {
                    bootstrap_session::mark_seed_tunnel_reported(
                        session,
                        latest_progress.reported_at_ms,
                    );
                    if session.fanout_activated_at_ms.is_none() {
                        bootstrap_session::mark_fanout_activated(
                            session,
                            latest_progress.reported_at_ms,
                        );
                        fanout_session = Some(session.clone());
                    }
                }
                BootstrapProgressStage::SeedPayloadReceived
                    if latest_progress.reporter_id == session.seed_bridge_id =>
                {
                    bootstrap_session::mark_bridge_set_delivered(
                        session,
                        latest_progress.reported_at_ms,
                    );
                }
                BootstrapProgressStage::BridgeSetComplete => {
                    bootstrap_session::mark_completed(session, latest_progress.reported_at_ms);
                }
                _ => {}
            }
            session.progress_events.len()
        };
        let mut activated_bridge_ids = Vec::new();
        if let Some(session) = fanout_session {
            let fanout_commands = bootstrap::fanout::build_remaining_bridge_assignments(
                &self.signing_key,
                &self.config,
                &session,
                reported_at_ms,
            )?;
            for command in fanout_commands {
                activated_bridge_ids.push(command.bridge_id.clone());
                assignment::queue_batch_assignment_command(
                    &mut self.storage,
                    &session.chain_id,
                    reported_at_ms,
                    command,
                );
            }
        }
        self.metrics.record_progress_report();
        self.persist_state()?;
        Ok(BootstrapProgressUpdate {
            stored_event_count,
            activated_bridge_ids,
        })
    }

    pub fn queue_catalog_refresh_notification(
        &mut self,
        bridge_id: &str,
        chain_id: &str,
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
        self.storage
            .record_catalog_issuance(Some(chain_id.to_string()), response.clone());
        assignment::queue_catalog_refresh_command(
            &mut self.storage,
            bridge_id,
            chain_id,
            now_ms,
            response.clone(),
        );
        self.persist_state()?;
        Ok(response)
    }

    pub fn reconcile_bridge_command_resume(
        &mut self,
        bridge_id: &str,
        resume_acked_seq_no: Option<u64>,
        acked_at_ms: u64,
    ) -> AuthorityResult<Option<u64>> {
        let Some(resume_acked_seq_no) = resume_acked_seq_no else {
            return Ok(self
                .storage
                .bridge_commands
                .values()
                .filter(|record| record.bridge_id == bridge_id)
                .filter(|record| record.acked_at_ms.is_some())
                .map(|record| record.seq_no)
                .max());
        };

        let mut changed = false;
        for record in self.storage.bridge_commands.values_mut() {
            if record.bridge_id == bridge_id
                && record.seq_no <= resume_acked_seq_no
                && record.acked_at_ms.is_none()
            {
                assignment::mark_command_acked(
                    record,
                    BridgeCommandAckStatus::Applied,
                    acked_at_ms,
                );
                changed = true;
            }
        }
        if changed {
            self.persist_state()?;
        }
        Ok(Some(resume_acked_seq_no))
    }

    pub fn mark_bridge_command_dispatched(
        &mut self,
        bridge_id: &str,
        command_id: &str,
        sent_at_ms: u64,
    ) -> AuthorityResult<()> {
        let record = self
            .storage
            .bridge_commands
            .get_mut(command_id)
            .ok_or_else(|| AuthorityError::BridgeCommandNotFound {
                bridge_id: bridge_id.to_string(),
                command_id: command_id.to_string(),
            })?;
        if record.bridge_id != bridge_id {
            return Err(AuthorityError::BridgeCommandNotFound {
                bridge_id: bridge_id.to_string(),
                command_id: command_id.to_string(),
            });
        }
        assignment::mark_command_sent(record, sent_at_ms);
        self.persist_state()?;
        Ok(())
    }

    pub fn acknowledge_bridge_command(&mut self, ack: &BridgeCommandAck) -> AuthorityResult<()> {
        let (payload, chain_id) = {
            let record = self
                .storage
                .bridge_commands
                .get_mut(&ack.command_id)
                .ok_or_else(|| AuthorityError::BridgeCommandNotFound {
                    bridge_id: ack.bridge_id.clone(),
                    command_id: ack.command_id.clone(),
                })?;
            if record.bridge_id != ack.bridge_id || record.seq_no != ack.seq_no {
                return Err(AuthorityError::BridgeCommandNotFound {
                    bridge_id: ack.bridge_id.clone(),
                    command_id: ack.command_id.clone(),
                });
            }
            if record.chain_id != ack.chain_id {
                return Err(AuthorityError::ChainIdMismatch {
                    context: "bridge command ack",
                    expected: record.chain_id.clone(),
                    actual: ack.chain_id.clone(),
                });
            }
            let payload = record.payload.clone();
            let chain_id = record.chain_id.clone();
            assignment::mark_command_acked(record, ack.status, ack.acked_at_ms);
            (payload, chain_id)
        };
        if matches!(ack.status, BridgeCommandAckStatus::Applied) {
            if let BridgeCommandPayload::SeedAssign(seed_assign) = payload {
                let session = self
                    .storage
                    .bootstrap_sessions
                    .get_mut(&seed_assign.bootstrap_session_id)
                    .ok_or_else(|| AuthorityError::BootstrapSessionNotFound {
                        bootstrap_session_id: seed_assign.bootstrap_session_id.clone(),
                    })?;
                if session.chain_id != chain_id {
                    return Err(AuthorityError::ChainIdMismatch {
                        context: "seed assignment ack",
                        expected: session.chain_id.clone(),
                        actual: chain_id,
                    });
                }
                bootstrap_session::mark_seed_acknowledged(session, ack.acked_at_ms);
            }
        }
        self.persist_state()?;
        Ok(())
    }

    pub fn upload_session(&self, session_id: &str) -> Option<&UploadSessionRecord> {
        self.storage.upload_sessions.get(session_id)
    }

    pub fn process_bootstrap_timeouts(&mut self, now_ms: u64) -> AuthorityResult<Vec<String>> {
        let timed_out_sessions = self
            .storage
            .bootstrap_sessions
            .values()
            .filter(|session| bootstrap_session::should_reassign_seed(session, now_ms))
            .map(|session| session.bootstrap_session_id.clone())
            .collect::<Vec<_>>();

        let mut reassigned_bridges = Vec::new();
        let had_timed_out_sessions = !timed_out_sessions.is_empty();
        for bootstrap_session_id in timed_out_sessions {
            let Some(current) = self
                .storage
                .bootstrap_sessions
                .get(&bootstrap_session_id)
                .cloned()
            else {
                continue;
            };

            if current.reassignment_count >= current.max_reassignment_count {
                if let Some(session) = self
                    .storage
                    .bootstrap_sessions
                    .get_mut(&bootstrap_session_id)
                {
                    bootstrap_session::mark_failed(session, now_ms);
                }
                continue;
            }

            let Some(new_seed_record) =
                crate::policy::bootstrap_candidates(&self.storage, now_ms, &self.policy)
                    .into_iter()
                    .find(|record| {
                        !current
                            .attempted_seed_bridge_ids
                            .iter()
                            .any(|attempted| attempted == &record.bridge_id)
                            && record.bridge_id != current.relay_bridge_id
                    })
            else {
                if let Some(session) = self
                    .storage
                    .bootstrap_sessions
                    .get_mut(&bootstrap_session_id)
                {
                    bootstrap_session::mark_failed(session, now_ms);
                }
                continue;
            };

            let creator_entry = current.creator_entry.clone();
            let new_seed_entry = bootstrap::bridge_bootstrap_entry(
                &new_seed_record,
                &self.signing_key,
                &self.config,
                now_ms,
            )?;
            let creator_response = gbn_bridge_protocol::CreatorBootstrapResponse::sign(
                gbn_bridge_protocol::CreatorBootstrapResponseUnsigned {
                    bootstrap_session_id: bootstrap_session_id.clone(),
                    seed_bridge: new_seed_entry,
                    publisher_pub: self.publisher_pub.clone(),
                    response_expiry_ms: now_ms + self.config.bootstrap_response_ttl_ms,
                    assigned_bridge_count: current.creator_response.assigned_bridge_count,
                },
                &self.signing_key,
            )?;
            let seed_punch = crate::punch::issue_seed_punch_instruction(
                &self.signing_key,
                &bootstrap_session_id,
                &new_seed_record.bridge_id,
                creator_entry.clone(),
                &self.config,
                now_ms,
            )?;
            let seed_assignment = crate::bootstrap::distribution::issue_seed_assignment(
                &self.signing_key,
                &new_seed_record.bridge_id,
                creator_entry,
                current.bridge_set.clone(),
                seed_punch,
                now_ms + self.config.bootstrap_response_ttl_ms,
            )?;
            if let Some(session) = self
                .storage
                .bootstrap_sessions
                .get_mut(&bootstrap_session_id)
            {
                session.creator_response = creator_response;
                bootstrap_session::mark_reassigned(
                    session,
                    &new_seed_record.bridge_id,
                    now_ms,
                    now_ms + self.config.bootstrap_seed_ack_timeout_ms,
                    now_ms + self.config.bootstrap_seed_tunnel_timeout_ms,
                );
            }
            self.queue_seed_assignment_command(&current.chain_id, now_ms, seed_assignment);
            reassigned_bridges.push(new_seed_record.bridge_id);
        }

        if !reassigned_bridges.is_empty() || had_timed_out_sessions {
            self.persist_state()?;
        }
        Ok(reassigned_bridges)
    }

    fn queue_seed_assignment_command(
        &mut self,
        chain_id: &str,
        issued_at_ms: u64,
        payload: gbn_bridge_protocol::BridgeSeedAssign,
    ) {
        assignment::queue_seed_assignment_command(
            &mut self.storage,
            chain_id,
            issued_at_ms,
            payload,
        );
    }

    fn queue_batch_commands(&mut self, issued_at_ms: u64, batch: &FinalizedBatch) {
        for assignment in &batch.bridge_assignments {
            let chain_id = format!("batch-{}", batch.batch_id);
            assignment::queue_batch_assignment_command(
                &mut self.storage,
                &chain_id,
                issued_at_ms,
                assignment.clone(),
            );
        }
    }

    fn queue_revoke_command(&mut self, revoke: BridgeRevoke, now_ms: u64) {
        let chain_id = format!("bridge-revoke-{}", revoke.bridge_id);
        assignment::queue_revoke_command(&mut self.storage, &chain_id, now_ms, revoke);
    }

    fn persist_state(&mut self) -> AuthorityResult<()> {
        if let Some(storage) = &mut self.durable_storage {
            storage.persist_state(&self.storage)?;
        }
        Ok(())
    }
}
