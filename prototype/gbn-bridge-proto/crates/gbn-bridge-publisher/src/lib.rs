//! Conduit publisher authority plane for registration, lease, catalog, and bootstrap issuance.

pub mod ack;
pub mod api;
pub mod auth;
pub mod authority;
pub mod batching;
pub mod bootstrap;
pub mod bridge_scoring;
pub mod catalog;
pub mod config;
pub mod http;
pub mod ingest;
pub mod lease;
pub mod metrics;
pub mod policy;
pub mod punch;
pub mod registry;
pub mod server;
pub mod service;
pub mod storage;

use gbn_bridge_protocol::{ProtocolError, DEFAULT_UDP_PUNCH_PORT};
use thiserror::Error;

pub use api::{
    AuthorityApiAuth, AuthorityApiErrorBody, AuthorityApiRequest, AuthorityApiRequestUnsigned,
    AuthorityApiResponse, AuthorityApiResponseUnsigned, BootstrapJoinBody, BootstrapJoinResponse,
    BootstrapProgressBody, BootstrapProgressReceipt, BootstrapProgressResponse,
    BridgeHeartbeatBody, BridgeRegisterBody, CreatorCatalogBody, CreatorCatalogResponse,
    EmptyResponse, HealthResponse, HeartbeatResponse, RegisterBridgeResponse,
};
pub use authority::PublisherAuthority;
pub use batching::FinalizedBatch;
pub use bootstrap::AuthorityBootstrapPlan;
pub use config::PublisherServiceConfig;
pub use metrics::{AuthorityMetrics, AuthorityMetricsSnapshot};
pub use server::{AuthorityServer, AuthorityServerHandle, BoundAuthorityServer};
pub use service::{AuthorityService, ServiceError};
pub use storage::{
    BatchWindowState, BootstrapSessionRecord, BridgeRecord, InMemoryAuthorityStorage,
    IngestedFrameRecord, UploadSessionRecord,
};

pub type AuthorityResult<T> = Result<T, AuthorityError>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AuthorityError {
    #[error("invalid bridge registration: {reason}")]
    InvalidBridgeRegistration { reason: &'static str },

    #[error("invalid creator join request: {reason}")]
    InvalidCreatorJoin { reason: &'static str },

    #[error("bridge `{bridge_id}` is already registered and active")]
    BridgeAlreadyRegistered { bridge_id: String },

    #[error("bridge `{bridge_id}` not found")]
    BridgeNotFound { bridge_id: String },

    #[error("bridge `{bridge_id}` is revoked")]
    BridgeRevoked { bridge_id: String },

    #[error(
        "heartbeat lease mismatch for bridge `{bridge_id}`: expected `{expected}`, got `{actual}`"
    )]
    LeaseMismatch {
        bridge_id: String,
        expected: String,
        actual: String,
    },

    #[error(
        "bridge `{bridge_id}` lease `{lease_id}` expired at `{lease_expiry_ms}` before heartbeat `{heartbeat_at_ms}`"
    )]
    LeaseExpired {
        bridge_id: String,
        lease_id: String,
        lease_expiry_ms: u64,
        heartbeat_at_ms: u64,
    },

    #[error("no eligible direct bridge is available for bootstrap")]
    NoEligibleBootstrapBridge,

    #[error("no eligible direct bridge is available for batch assignment")]
    NoEligibleBatchBridge,

    #[error("bootstrap session `{bootstrap_session_id}` not found")]
    BootstrapSessionNotFound { bootstrap_session_id: String },

    #[error("upload session `{session_id}` not found")]
    UploadSessionNotFound { session_id: String },

    #[error("upload session `{session_id}` was opened by `{expected_creator_id}` not `{actual_creator_id}`")]
    UploadSessionCreatorMismatch {
        session_id: String,
        expected_creator_id: String,
        actual_creator_id: String,
    },

    #[error("upload session `{session_id}` is already closed")]
    UploadSessionClosed { session_id: String },

    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityConfig {
    pub default_udp_punch_port: u16,
    pub lease_ttl_ms: u64,
    pub heartbeat_interval_ms: u64,
    pub catalog_ttl_ms: u64,
    pub bootstrap_entry_ttl_ms: u64,
    pub bootstrap_response_ttl_ms: u64,
    pub punch_instruction_ttl_ms: u64,
    pub bootstrap_bridge_count: usize,
    pub batch_window_ms: u64,
    pub batch_capacity: usize,
}

impl Default for AuthorityConfig {
    fn default() -> Self {
        Self {
            default_udp_punch_port: DEFAULT_UDP_PUNCH_PORT,
            lease_ttl_ms: 30_000,
            heartbeat_interval_ms: 5_000,
            catalog_ttl_ms: 15_000,
            bootstrap_entry_ttl_ms: 20_000,
            bootstrap_response_ttl_ms: 20_000,
            punch_instruction_ttl_ms: 20_000,
            bootstrap_bridge_count: 9,
            batch_window_ms: 500,
            batch_capacity: 10,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorityPolicy {
    pub direct_only_bootstrap: bool,
    pub allow_non_direct_catalog_entries: bool,
}

impl Default for AuthorityPolicy {
    fn default() -> Self {
        Self {
            direct_only_bootstrap: true,
            allow_non_direct_catalog_entries: true,
        }
    }
}
