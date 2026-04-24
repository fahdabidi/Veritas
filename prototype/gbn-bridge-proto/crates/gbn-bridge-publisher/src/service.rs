use std::sync::mpsc::Sender;
use std::time::{SystemTime, UNIX_EPOCH};

use gbn_bridge_protocol::{
    BootstrapJoinReply, BootstrapProgressStage, BridgeCommandAck, BridgeControlCommand,
    BridgeControlFrame, BridgeControlHello, BridgeControlKeepalive, BridgeControlProgress,
    BridgeControlWelcome, BridgeControlWelcomeUnsigned, ProtocolError,
};
use serde::Serialize;

use crate::api::{
    AuthorityApiErrorBody, AuthorityApiRequest, AuthorityApiResponse, AuthorityApiResponseUnsigned,
    BootstrapJoinBody, BootstrapProgressBody, BootstrapProgressReceipt, BridgeHeartbeatBody,
    BridgeRegisterBody, CreatorCatalogBody, CreatorCatalogResponse, EmptyResponse, HealthResponse,
    ReceiverCloseBody, ReceiverFrameBody, ReceiverOpenBody,
};
use crate::auth::{AuthError, RequestAuthenticator};
use crate::control::ControlSessionRegistry;
use crate::dispatcher;
use crate::{ack_service, receiver};
use crate::{AuthorityError, PublisherAuthority, PublisherServiceConfig};

#[derive(Debug)]
pub struct AuthorityService {
    authority: PublisherAuthority,
    authenticator: RequestAuthenticator,
    control_authenticator: RequestAuthenticator,
    control_sessions: ControlSessionRegistry,
    control_heartbeat_interval_ms: u64,
    control_idle_timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceError {
    BadRequest(String),
    Unauthorized(String),
    Forbidden(String),
    NotFound(String),
    Conflict(String),
    Expired(String),
    Internal(String),
}

impl ServiceError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Expired(_) => "expired",
            Self::Internal(_) => "internal",
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::BadRequest(message)
            | Self::Unauthorized(message)
            | Self::Forbidden(message)
            | Self::NotFound(message)
            | Self::Conflict(message)
            | Self::Expired(message)
            | Self::Internal(message) => message,
        }
    }

    pub fn http_status(&self) -> u16 {
        match self {
            Self::BadRequest(_) => 400,
            Self::Unauthorized(_) => 401,
            Self::Forbidden(_) => 403,
            Self::NotFound(_) => 404,
            Self::Conflict(_) => 409,
            Self::Expired(_) => 410,
            Self::Internal(_) => 500,
        }
    }
}

impl AuthorityService {
    pub fn new(authority: PublisherAuthority, config: &PublisherServiceConfig) -> Self {
        Self {
            authority,
            authenticator: RequestAuthenticator::new(config.auth_max_skew_ms, config.replay_ttl_ms),
            control_authenticator: RequestAuthenticator::new(
                config.auth_max_skew_ms,
                config.replay_ttl_ms,
            ),
            control_sessions: ControlSessionRegistry::default(),
            control_heartbeat_interval_ms: config.control_heartbeat_interval_ms,
            control_idle_timeout_ms: config.control_idle_timeout_ms,
        }
    }

    pub fn publisher_public_key(&self) -> &gbn_bridge_protocol::PublicKeyBytes {
        self.authority.publisher_public_key()
    }

    pub fn publisher_authority(&self) -> &PublisherAuthority {
        &self.authority
    }

    pub fn publisher_authority_mut(&mut self) -> &mut PublisherAuthority {
        &mut self.authority
    }

    pub fn healthz(&self) -> Result<AuthorityApiResponse<HealthResponse>, ServiceError> {
        self.success_response(
            "system-healthz",
            "healthz",
            HealthResponse {
                status: "ok".into(),
            },
        )
    }

    pub fn readyz(&mut self) -> Result<AuthorityApiResponse<HealthResponse>, ServiceError> {
        self.authority
            .durable_store_healthcheck()
            .map_err(map_authority_error)?;
        self.success_response(
            "system-readyz",
            "readyz",
            HealthResponse {
                status: "ready".into(),
            },
        )
    }

    pub fn control_timeouts(&self) -> (u64, u64) {
        (
            self.control_idle_timeout_ms,
            self.control_heartbeat_interval_ms,
        )
    }

    pub fn accept_control_hello(
        &mut self,
        hello: BridgeControlHello,
        sender: Sender<BridgeControlFrame>,
    ) -> Result<(BridgeControlWelcome, Vec<BridgeControlCommand>), ServiceError> {
        let accepted_at_ms = now_ms();
        self.control_authenticator
            .verify_bridge_control_hello(&hello, accepted_at_ms)
            .map_err(map_auth_error)?;

        let record = self
            .authority
            .bridge_record(&hello.bridge_id)
            .cloned()
            .ok_or_else(|| {
                ServiceError::NotFound(format!("bridge `{}` not found", hello.bridge_id))
            })?;
        self.control_authenticator
            .ensure_actor_key_matches(&hello.bridge_id, &record.identity_pub, &hello.bridge_pub)
            .map_err(map_auth_error)?;
        if record.current_lease.lease_id != hello.lease_id {
            return Err(ServiceError::Conflict(format!(
                "control hello lease mismatch for bridge `{}`: expected `{}`, got `{}`",
                hello.bridge_id, record.current_lease.lease_id, hello.lease_id
            )));
        }
        if record.revoked_reason.is_some() {
            return Err(ServiceError::Forbidden(format!(
                "bridge `{}` is revoked",
                hello.bridge_id
            )));
        }
        if record.current_lease.lease_expiry_ms < accepted_at_ms {
            return Err(ServiceError::Expired(format!(
                "bridge `{}` lease `{}` expired at `{}` before `{}`",
                hello.bridge_id,
                record.current_lease.lease_id,
                record.current_lease.lease_expiry_ms,
                accepted_at_ms
            )));
        }

        let resume_acked_seq_no = self
            .authority
            .reconcile_bridge_command_resume(
                &hello.bridge_id,
                hello.resume_acked_seq_no,
                accepted_at_ms,
            )
            .map_err(map_authority_error)?;
        let session = self.control_sessions.replace_session(
            &hello.bridge_id,
            &hello.lease_id,
            accepted_at_ms,
            resume_acked_seq_no,
            sender,
        );
        let welcome = BridgeControlWelcome::sign(
            BridgeControlWelcomeUnsigned {
                bridge_id: hello.bridge_id.clone(),
                session_id: session.session_id.clone(),
                accepted_at_ms,
                heartbeat_interval_ms: self.control_heartbeat_interval_ms,
                idle_timeout_ms: self.control_idle_timeout_ms,
                last_publisher_seq_no: self.authority.last_bridge_command_seq(&hello.bridge_id),
                chain_id: hello.chain_id,
            },
            self.authority.signing_key(),
        )
        .map_err(map_protocol_error)?;
        let pending =
            dispatcher::dispatch_pending_commands(&mut self.authority, &session, accepted_at_ms)
                .map_err(map_authority_error)?;
        Ok((welcome, pending))
    }

    pub fn handle_control_ack(
        &mut self,
        ack: BridgeCommandAck,
    ) -> Result<Vec<BridgeControlCommand>, ServiceError> {
        let seen_at_ms = ack.acked_at_ms;
        let session = self
            .control_sessions
            .session(&ack.session_id)
            .cloned()
            .ok_or_else(|| {
                ServiceError::Unauthorized(format!("unknown control session `{}`", ack.session_id))
            })?;
        if session.bridge_id != ack.bridge_id {
            return Err(ServiceError::Unauthorized(format!(
                "control ACK bridge mismatch: expected `{}`, got `{}`",
                session.bridge_id, ack.bridge_id
            )));
        }
        self.authority
            .acknowledge_bridge_command(&ack)
            .map_err(map_authority_error)?;
        let _ = self
            .control_sessions
            .touch_session(&ack.session_id, seen_at_ms);
        let _ = self
            .control_sessions
            .update_acked_seq_no(&ack.session_id, ack.seq_no);
        let session = self
            .control_sessions
            .session(&ack.session_id)
            .cloned()
            .expect("control session should still exist");
        dispatcher::dispatch_pending_commands(&mut self.authority, &session, seen_at_ms)
            .map_err(map_authority_error)
    }

    pub fn handle_control_progress(
        &mut self,
        progress: BridgeControlProgress,
    ) -> Result<Vec<BridgeControlCommand>, ServiceError> {
        let seen_at_ms = progress.progress.reported_at_ms;
        let session = self
            .control_sessions
            .session(&progress.session_id)
            .cloned()
            .ok_or_else(|| {
                ServiceError::Unauthorized(format!(
                    "unknown control session `{}`",
                    progress.session_id
                ))
            })?;
        if session.bridge_id != progress.progress.reporter_id {
            return Err(ServiceError::Unauthorized(format!(
                "control progress reporter mismatch: expected `{}`, got `{}`",
                session.bridge_id, progress.progress.reporter_id
            )));
        }
        let update = self
            .authority
            .report_bootstrap_progress_with_chain_id(&progress.chain_id, progress.progress.clone())
            .map_err(map_authority_error)?;
        let _ = self
            .control_sessions
            .touch_session(&progress.session_id, seen_at_ms);
        for bridge_id in update.activated_bridge_ids {
            self.push_pending_commands_for_bridge(&bridge_id, seen_at_ms)
                .map_err(map_authority_error)?;
        }
        dispatcher::dispatch_pending_commands(&mut self.authority, &session, seen_at_ms)
            .map_err(map_authority_error)
    }

    pub fn handle_control_keepalive(
        &mut self,
        keepalive: BridgeControlKeepalive,
    ) -> Result<Vec<BridgeControlCommand>, ServiceError> {
        let session = self
            .control_sessions
            .session(&keepalive.session_id)
            .cloned()
            .ok_or_else(|| {
                ServiceError::Unauthorized(format!(
                    "unknown control session `{}`",
                    keepalive.session_id
                ))
            })?;
        if session.bridge_id != keepalive.bridge_id {
            return Err(ServiceError::Unauthorized(format!(
                "control keepalive bridge mismatch: expected `{}`, got `{}`",
                session.bridge_id, keepalive.bridge_id
            )));
        }
        let _ = self
            .control_sessions
            .touch_session(&keepalive.session_id, keepalive.sent_at_ms);
        dispatcher::dispatch_pending_commands(&mut self.authority, &session, keepalive.sent_at_ms)
            .map_err(map_authority_error)
    }

    pub fn remove_control_session(&mut self, session_id: &str) {
        self.control_sessions.remove_session(session_id);
    }

    pub fn error_response(
        &self,
        chain_id: &str,
        request_id: &str,
        error: ServiceError,
    ) -> Result<AuthorityApiResponse<EmptyResponse>, ServiceError> {
        let unsigned = AuthorityApiResponseUnsigned {
            chain_id: chain_id.to_string(),
            request_id: request_id.to_string(),
            served_at_ms: now_ms(),
            ok: false,
            body: None,
            error: Some(AuthorityApiErrorBody {
                code: error.code().to_string(),
                message: error.message().to_string(),
            }),
        };

        self.authority
            .sign_api_response(unsigned)
            .map_err(map_authority_error)
    }

    pub fn handle_bridge_register(
        &mut self,
        request: AuthorityApiRequest<BridgeRegisterBody>,
    ) -> Result<AuthorityApiResponse<gbn_bridge_protocol::BridgeLease>, ServiceError> {
        self.authenticator
            .verify_signed_request(&request, now_ms())
            .map_err(map_auth_error)?;
        self.authenticator
            .ensure_actor_id_matches(&request.body.register.bridge_id, &request.actor_id)
            .map_err(map_auth_error)?;
        self.authenticator
            .ensure_actor_key_matches(
                &request.actor_id,
                &request.body.register.identity_pub,
                &request.auth.actor_pub,
            )
            .map_err(map_auth_error)?;

        let lease = self
            .authority
            .register_bridge(
                request.body.register,
                request.body.reachability_class,
                request.body.now_ms,
            )
            .map_err(map_authority_error)?;
        self.success_response(&request.chain_id, &request.request_id, lease)
    }

    pub fn handle_bridge_heartbeat(
        &mut self,
        request: AuthorityApiRequest<BridgeHeartbeatBody>,
    ) -> Result<AuthorityApiResponse<gbn_bridge_protocol::BridgeLease>, ServiceError> {
        self.authenticator
            .verify_signed_request(&request, now_ms())
            .map_err(map_auth_error)?;
        self.authenticator
            .ensure_actor_id_matches(&request.body.heartbeat.bridge_id, &request.actor_id)
            .map_err(map_auth_error)?;
        if let Some(expected_key) = self.authority.bridge_identity_pub(&request.actor_id) {
            self.authenticator
                .ensure_actor_key_matches(&request.actor_id, &expected_key, &request.auth.actor_pub)
                .map_err(map_auth_error)?;
        }

        let lease = self
            .authority
            .handle_heartbeat(request.body.heartbeat)
            .map_err(map_authority_error)?;
        self.success_response(&request.chain_id, &request.request_id, lease)
    }

    pub fn handle_creator_catalog(
        &mut self,
        request: AuthorityApiRequest<CreatorCatalogBody>,
    ) -> Result<CreatorCatalogResponse, ServiceError> {
        self.authenticator
            .verify_signed_request(&request, now_ms())
            .map_err(map_auth_error)?;
        self.authenticator
            .ensure_actor_id_matches(&request.body.request.creator_id, &request.actor_id)
            .map_err(map_auth_error)?;

        let catalog = self
            .authority
            .issue_catalog_with_chain_id(
                Some(&request.chain_id),
                &request.body.request,
                request.body.now_ms,
            )
            .map_err(map_authority_error)?;
        self.success_response(&request.chain_id, &request.request_id, catalog)
    }

    pub fn handle_bootstrap_join(
        &mut self,
        request: AuthorityApiRequest<BootstrapJoinBody>,
    ) -> Result<AuthorityApiResponse<BootstrapJoinReply>, ServiceError> {
        self.authenticator
            .verify_signed_request(&request, now_ms())
            .map_err(map_auth_error)?;
        self.authenticator
            .ensure_actor_id_matches(&request.body.request.host_creator_id, &request.actor_id)
            .map_err(map_auth_error)?;

        let plan = self
            .authority
            .begin_bootstrap_with_chain_id(
                &request.chain_id,
                request.body.request,
                request.body.now_ms,
            )
            .map_err(map_authority_error)?;
        self.push_pending_commands_for_bridge(
            &plan.seed_assignment.seed_bridge_id,
            request.body.now_ms,
        )
        .map_err(map_authority_error)?;
        self.authority
            .mark_bootstrap_response_returned(
                &request.chain_id,
                &plan.response.bootstrap_session_id,
                request.body.now_ms,
            )
            .map_err(map_authority_error)?;
        self.success_response(&request.chain_id, &request.request_id, plan.join_reply())
    }

    pub fn handle_progress_report(
        &mut self,
        request: AuthorityApiRequest<BootstrapProgressBody>,
    ) -> Result<AuthorityApiResponse<BootstrapProgressReceipt>, ServiceError> {
        self.authenticator
            .verify_signed_request(&request, now_ms())
            .map_err(map_auth_error)?;
        self.authenticator
            .ensure_actor_id_matches(&request.body.progress.reporter_id, &request.actor_id)
            .map_err(map_auth_error)?;
        if let Some(expected_key) = self.authority.bridge_identity_pub(&request.actor_id) {
            self.authenticator
                .ensure_actor_key_matches(&request.actor_id, &expected_key, &request.auth.actor_pub)
                .map_err(map_auth_error)?;
        }

        let progress = request.body.progress;
        let update = self
            .authority
            .report_bootstrap_progress_with_chain_id(&request.chain_id, progress.clone())
            .map_err(map_authority_error)?;
        for bridge_id in &update.activated_bridge_ids {
            self.push_pending_commands_for_bridge(bridge_id, progress.reported_at_ms)
                .map_err(map_authority_error)?;
        }
        let receipt = BootstrapProgressReceipt {
            bootstrap_session_id: progress.bootstrap_session_id,
            reporter_id: progress.reporter_id,
            stored_event_count: update.stored_event_count,
            latest_stage: bootstrap_stage_name(progress.stage),
        };
        self.success_response(&request.chain_id, &request.request_id, receipt)
    }

    pub fn handle_receiver_open(
        &mut self,
        request: AuthorityApiRequest<ReceiverOpenBody>,
    ) -> Result<AuthorityApiResponse<EmptyResponse>, ServiceError> {
        self.authenticator
            .verify_signed_request(&request, now_ms())
            .map_err(map_auth_error)?;
        self.authenticator
            .ensure_actor_id_matches(&request.body.open.bridge_id, &request.actor_id)
            .map_err(map_auth_error)?;
        if let Some(expected_key) = self.authority.bridge_identity_pub(&request.actor_id) {
            self.authenticator
                .ensure_actor_key_matches(&request.actor_id, &expected_key, &request.auth.actor_pub)
                .map_err(map_auth_error)?;
        }

        receiver::open_session(&mut self.authority, &request.chain_id, request.body.open)
            .map_err(map_authority_error)?;
        self.success_response(&request.chain_id, &request.request_id, EmptyResponse)
    }

    pub fn handle_receiver_frame(
        &mut self,
        request: AuthorityApiRequest<ReceiverFrameBody>,
    ) -> Result<AuthorityApiResponse<gbn_bridge_protocol::BridgeAck>, ServiceError> {
        self.authenticator
            .verify_signed_request(&request, now_ms())
            .map_err(map_auth_error)?;
        self.authenticator
            .ensure_actor_id_matches(&request.body.via_bridge_id, &request.actor_id)
            .map_err(map_auth_error)?;
        if let Some(expected_key) = self.authority.bridge_identity_pub(&request.actor_id) {
            self.authenticator
                .ensure_actor_key_matches(&request.actor_id, &expected_key, &request.auth.actor_pub)
                .map_err(map_auth_error)?;
        }

        let ack = ack_service::ingest_frame(
            &mut self.authority,
            &request.chain_id,
            &request.body.via_bridge_id,
            request.body.frame,
            request.body.received_at_ms,
        )
        .map_err(map_authority_error)?;
        self.success_response(&request.chain_id, &request.request_id, ack)
    }

    pub fn handle_receiver_close(
        &mut self,
        request: AuthorityApiRequest<ReceiverCloseBody>,
    ) -> Result<AuthorityApiResponse<EmptyResponse>, ServiceError> {
        self.authenticator
            .verify_signed_request(&request, now_ms())
            .map_err(map_auth_error)?;
        self.authenticator
            .ensure_actor_id_matches(&request.body.bridge_id, &request.actor_id)
            .map_err(map_auth_error)?;
        if let Some(expected_key) = self.authority.bridge_identity_pub(&request.actor_id) {
            self.authenticator
                .ensure_actor_key_matches(&request.actor_id, &expected_key, &request.auth.actor_pub)
                .map_err(map_auth_error)?;
        }

        receiver::close_session(&mut self.authority, &request.chain_id, request.body.close)
            .map_err(map_authority_error)?;
        self.success_response(&request.chain_id, &request.request_id, EmptyResponse)
    }

    fn success_response<T>(
        &self,
        chain_id: &str,
        request_id: &str,
        body: T,
    ) -> Result<AuthorityApiResponse<T>, ServiceError>
    where
        T: Serialize + Clone,
    {
        let unsigned = AuthorityApiResponseUnsigned {
            chain_id: chain_id.to_string(),
            request_id: request_id.to_string(),
            served_at_ms: now_ms(),
            ok: true,
            body: Some(body),
            error: None,
        };
        self.authority
            .sign_api_response(unsigned)
            .map_err(map_authority_error)
    }

    fn push_pending_commands_for_bridge(
        &mut self,
        bridge_id: &str,
        sent_at_ms: u64,
    ) -> crate::AuthorityResult<()> {
        let Some(session) = self.control_sessions.bridge_session(bridge_id).cloned() else {
            return Ok(());
        };
        let commands =
            dispatcher::dispatch_pending_commands(&mut self.authority, &session, sent_at_ms)?;
        for command in commands {
            let _ = session.sender.send(BridgeControlFrame::Command(command));
        }
        Ok(())
    }
}

fn bootstrap_stage_name(stage: BootstrapProgressStage) -> String {
    match stage {
        BootstrapProgressStage::SeedAssigned => "seed_assigned",
        BootstrapProgressStage::SeedTunnelEstablished => "seed_tunnel_established",
        BootstrapProgressStage::SeedPayloadReceived => "seed_payload_received",
        BootstrapProgressStage::FanoutStarted => "fanout_started",
        BootstrapProgressStage::BridgeTunnelEstablished => "bridge_tunnel_established",
        BootstrapProgressStage::BridgeSetComplete => "bridge_set_complete",
        BootstrapProgressStage::FallbackReuseActivated => "fallback_reuse_activated",
    }
    .into()
}

fn map_auth_error(error: AuthError) -> ServiceError {
    match error {
        AuthError::EmptyChainId | AuthError::EmptyRequestId | AuthError::EmptyActorId => {
            ServiceError::BadRequest(error.to_string())
        }
        AuthError::ReplayDetected { .. }
        | AuthError::ActorIdMismatch { .. }
        | AuthError::ActorKeyMismatch { .. }
        | AuthError::Protocol(
            ProtocolError::ReplayWindowExpired { .. }
            | ProtocolError::ReplayTimestampInFuture { .. }
            | ProtocolError::InvalidSignature
            | ProtocolError::InvalidSignatureLength { .. }
            | ProtocolError::InvalidPublicKeyLength { .. },
        ) => ServiceError::Unauthorized(error.to_string()),
        AuthError::Protocol(other) => map_protocol_error(other),
    }
}

fn map_authority_error(error: AuthorityError) -> ServiceError {
    match error {
        AuthorityError::InvalidBridgeRegistration { .. }
        | AuthorityError::InvalidCreatorJoin { .. } => ServiceError::BadRequest(error.to_string()),
        AuthorityError::BridgeAlreadyRegistered { .. } => ServiceError::Conflict(error.to_string()),
        AuthorityError::BridgeNotFound { .. }
        | AuthorityError::BootstrapSessionNotFound { .. }
        | AuthorityError::BridgeCommandNotFound { .. }
        | AuthorityError::UploadSessionNotFound { .. } => ServiceError::NotFound(error.to_string()),
        AuthorityError::BridgeRevoked { .. } => ServiceError::Forbidden(error.to_string()),
        AuthorityError::LeaseMismatch { .. } => ServiceError::Conflict(error.to_string()),
        AuthorityError::LeaseExpired { .. } => ServiceError::Expired(error.to_string()),
        AuthorityError::NoEligibleBootstrapBridge
        | AuthorityError::NoEligibleBatchBridge
        | AuthorityError::UploadSessionCreatorMismatch { .. }
        | AuthorityError::UploadSessionClosed { .. }
        | AuthorityError::ChainIdMismatch { .. } => ServiceError::BadRequest(error.to_string()),
        AuthorityError::Storage(_) => ServiceError::Internal(error.to_string()),
        AuthorityError::Protocol(error) => map_protocol_error(error),
    }
}

fn map_protocol_error(error: ProtocolError) -> ServiceError {
    match error {
        ProtocolError::Expired { .. } => ServiceError::Expired(error.to_string()),
        ProtocolError::ReplayWindowExpired { .. }
        | ProtocolError::ReplayTimestampInFuture { .. }
        | ProtocolError::MissingSignature
        | ProtocolError::InvalidPublicKeyLength { .. }
        | ProtocolError::InvalidSignatureLength { .. }
        | ProtocolError::InvalidSignature => ServiceError::Unauthorized(error.to_string()),
        ProtocolError::UnsupportedProtocolVersion { .. }
        | ProtocolError::EmptyIngressEndpoints
        | ProtocolError::EmptyBridgeSet
        | ProtocolError::EmptyBatchAssignments
        | ProtocolError::InvalidUdpPunchPort
        | ProtocolError::Serialization(_) => ServiceError::BadRequest(error.to_string()),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis() as u64
}
