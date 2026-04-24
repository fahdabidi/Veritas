use std::time::{SystemTime, UNIX_EPOCH};

use gbn_bridge_protocol::{BootstrapProgressStage, ProtocolError};
use serde::Serialize;

use crate::api::{
    AuthorityApiErrorBody, AuthorityApiRequest, AuthorityApiResponse, AuthorityApiResponseUnsigned,
    BootstrapJoinBody, BootstrapProgressBody, BootstrapProgressReceipt, BridgeHeartbeatBody,
    BridgeRegisterBody, CreatorCatalogBody, CreatorCatalogResponse, EmptyResponse, HealthResponse,
};
use crate::auth::{AuthError, RequestAuthenticator};
use crate::{AuthorityError, PublisherAuthority, PublisherServiceConfig};

#[derive(Debug)]
pub struct AuthorityService {
    authority: PublisherAuthority,
    authenticator: RequestAuthenticator,
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
        }
    }

    pub fn publisher_public_key(&self) -> &gbn_bridge_protocol::PublicKeyBytes {
        self.authority.publisher_public_key()
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

    pub fn readyz(&self) -> Result<AuthorityApiResponse<HealthResponse>, ServiceError> {
        self.success_response(
            "system-readyz",
            "readyz",
            HealthResponse {
                status: "ready".into(),
            },
        )
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
            .issue_catalog(&request.body.request, request.body.now_ms)
            .map_err(map_authority_error)?;
        self.success_response(&request.chain_id, &request.request_id, catalog)
    }

    pub fn handle_bootstrap_join(
        &mut self,
        request: AuthorityApiRequest<BootstrapJoinBody>,
    ) -> Result<AuthorityApiResponse<crate::AuthorityBootstrapPlan>, ServiceError> {
        self.authenticator
            .verify_signed_request(&request, now_ms())
            .map_err(map_auth_error)?;
        self.authenticator
            .ensure_actor_id_matches(&request.body.request.host_creator_id, &request.actor_id)
            .map_err(map_auth_error)?;

        let plan = self
            .authority
            .begin_bootstrap(request.body.request, request.body.now_ms)
            .map_err(map_authority_error)?;
        self.success_response(&request.chain_id, &request.request_id, plan)
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
        let stored_event_count = self
            .authority
            .report_bootstrap_progress(progress.clone())
            .map_err(map_authority_error)?;
        let receipt = BootstrapProgressReceipt {
            bootstrap_session_id: progress.bootstrap_session_id,
            reporter_id: progress.reporter_id,
            stored_event_count,
            latest_stage: bootstrap_stage_name(progress.stage),
        };
        self.success_response(&request.chain_id, &request.request_id, receipt)
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
        | AuthorityError::UploadSessionNotFound { .. } => ServiceError::NotFound(error.to_string()),
        AuthorityError::BridgeRevoked { .. } => ServiceError::Forbidden(error.to_string()),
        AuthorityError::LeaseMismatch { .. } => ServiceError::Conflict(error.to_string()),
        AuthorityError::LeaseExpired { .. } => ServiceError::Expired(error.to_string()),
        AuthorityError::NoEligibleBootstrapBridge
        | AuthorityError::NoEligibleBatchBridge
        | AuthorityError::UploadSessionCreatorMismatch { .. }
        | AuthorityError::UploadSessionClosed { .. } => ServiceError::BadRequest(error.to_string()),
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
