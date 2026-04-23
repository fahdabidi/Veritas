//! Conduit ExitBridge runtime for registration, lease maintenance, punching, and seed bootstrap duties.

pub mod ack_tracker;
pub mod bootstrap;
pub mod bootstrap_bridge;
pub mod bridge;
pub mod bridge_pool;
pub mod catalog_cache;
pub mod chunk_sender;
pub mod creator;
pub mod creator_listener;
pub mod discovery;
pub mod fanout_scheduler;
pub mod forwarder;
pub mod framing;
pub mod heartbeat_loop;
pub mod hint_merge;
pub mod host_creator;
pub mod lease_state;
pub mod local_dht;
pub mod progress_reporter;
pub mod publisher_client;
pub mod punch;
pub mod punch_fanout;
pub mod seed_catalog;
pub mod selector;
pub mod session;

use gbn_bridge_protocol::{ProtocolError, ReachabilityClass};
use gbn_bridge_publisher::AuthorityError;
use thiserror::Error;

pub use ack_tracker::AckTracker;
pub use bootstrap::{
    establish_seed_tunnel, fetch_bridge_set, request_first_contact, SeedTunnelOutcome,
};
pub use bootstrap_bridge::{BootstrapBridgeState, SeedBridgeAssignment};
pub use bridge::{ExitBridgeConfig, ExitBridgeRuntime};
pub use bridge_pool::{BridgePool, BridgePoolEntry};
pub use catalog_cache::CatalogCache;
pub use chunk_sender::{ChunkSender, ChunkSenderConfig, UploadResult};
pub use creator::{CreatorConfig, CreatorRuntime};
pub use creator_listener::CreatorListener;
pub use discovery::{DiscoveryHint, DiscoveryHintSource, WeakDiscoveryConfig, WeakDiscoveryState};
pub use fanout_scheduler::{FanoutPlan, FanoutScheduler, FanoutSchedulerConfig, FrameDispatch};
pub use forwarder::{ForwardedFrame, PayloadForwarder};
pub use framing::{frame_payload, FramePayloadConfig};
pub use heartbeat_loop::HeartbeatLoop;
pub use hint_merge::{
    merge_refresh_candidates, RefreshCandidate, RefreshCandidateAuthority, RefreshCandidateSource,
};
pub use host_creator::HostCreator;
pub use lease_state::LeaseState;
pub use local_dht::{LocalDht, LocalDhtNode, LocalHintSource};
pub use progress_reporter::ProgressReporter;
pub use publisher_client::InProcessPublisherClient;
pub use punch::{ActivePunchAttempt, PunchAuthorization, PunchManager};
pub use punch_fanout::{CreatorPunchAck, CreatorPunchAttempt, FanoutSource, PunchFanout};
pub use seed_catalog::SeedCatalog;
pub use session::{UploadSession, UploadSessionConfig};

pub type RuntimeResult<T> = Result<T, RuntimeError>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    #[error("bridge `{bridge_id}` has no active lease")]
    NoActiveLease { bridge_id: String },

    #[error("bridge `{bridge_id}` ingress is disabled")]
    IngressDisabled { bridge_id: String },

    #[error(
        "bridge `{bridge_id}` cannot expose ingress when reachability class is `{reachability_class:?}`"
    )]
    NonDirectReachability {
        bridge_id: String,
        reachability_class: ReachabilityClass,
    },

    #[error("publisher-directed or refresh-authorized punching required: {reason}")]
    PunchUnauthorized { reason: &'static str },

    #[error("bootstrap session `{bootstrap_session_id}` is not tracked by this bridge")]
    BootstrapSessionNotTracked { bootstrap_session_id: String },

    #[error(
        "punch attempt for bootstrap session `{bootstrap_session_id}` expired at `{attempt_expiry_ms}` before `{now_ms}`"
    )]
    PunchAttemptExpired {
        bootstrap_session_id: String,
        attempt_expiry_ms: u64,
        now_ms: u64,
    },

    #[error(
        "probe nonce mismatch for bootstrap session `{bootstrap_session_id}`: expected `{expected}`, got `{actual}`"
    )]
    ProbeNonceMismatch {
        bootstrap_session_id: String,
        expected: u64,
        actual: u64,
    },

    #[error("bridge `{bridge_id}` has no remembered reachability class for re-registration")]
    MissingReachabilityClass { bridge_id: String },

    #[error("creator has no publisher trust root loaded")]
    MissingPublisherTrustRoot,

    #[error("publisher trust root mismatch: expected {expected:?}, got {actual:?}")]
    PublisherTrustRootMismatch {
        expected: gbn_bridge_protocol::PublicKeyBytes,
        actual: gbn_bridge_protocol::PublicKeyBytes,
    },

    #[error("creator has no valid cached catalog")]
    CatalogUnavailable,

    #[error("no valid direct bridge candidate is available")]
    NoUsableBridgeCandidate,

    #[error(
        "creator identity mismatch: expected `{expected_creator_id}`, got `{actual_creator_id}`"
    )]
    CreatorIdentityMismatch {
        expected_creator_id: String,
        actual_creator_id: String,
    },

    #[error(
        "bridge runtime mismatch: expected bridge `{expected_bridge_id}`, got `{actual_bridge_id}`"
    )]
    UnexpectedBridgeRuntime {
        expected_bridge_id: String,
        actual_bridge_id: String,
    },

    #[error(
        "bridge set session mismatch: expected `{expected_bootstrap_session_id}`, got `{actual_bootstrap_session_id}`"
    )]
    BridgeSetSessionMismatch {
        expected_bootstrap_session_id: String,
        actual_bootstrap_session_id: String,
    },

    #[error("creator fanout attempt `{bootstrap_session_id}` is not tracked by this creator")]
    CreatorBootstrapSessionNotTracked { bootstrap_session_id: String },

    #[error(
        "creator punch target mismatch for `{bootstrap_session_id}`: expected `{expected_target_id}`, got `{actual_target_id}`"
    )]
    CreatorPunchTargetMismatch {
        bootstrap_session_id: String,
        expected_target_id: String,
        actual_target_id: String,
    },

    #[error("creator has no active bridges available for upload")]
    NoActiveUploadBridge,

    #[error("upload session `{session_id}` is not tracked by this component")]
    UploadSessionNotTracked { session_id: String },

    #[error("bridge ACK for session `{session_id}` sequence `{sequence}` was unexpected")]
    UnexpectedBridgeAck { session_id: String, sequence: u32 },

    #[error("bridge ACK rejected session `{session_id}` sequence `{sequence}`")]
    RejectedBridgeAck { session_id: String, sequence: u32 },

    #[error(transparent)]
    Authority(#[from] AuthorityError),

    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}
