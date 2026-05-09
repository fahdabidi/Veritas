//! Creator-side V2 send-dummy implementation.
//!
//! This crate is library-only so any Conduit service binary can act as a creator when
//! its local admin listener is asked to synthesize a test payload.

pub mod client;
pub mod error;
pub mod local_dht;
pub mod pipeline;
pub mod session;
pub mod upload;

pub use client::{BridgeFilterDrops, CreatorClient, DiscoveryProbeResult, SendDummyResult};
pub use error::CreatorError;
pub use local_dht::{
    LocalDhtCommand, LocalDhtError, LocalDhtMutation, LocalDhtStore, ResetCreatorStateResponse,
};
pub use pipeline::{
    build_upload_session, build_upload_session_to_disk, chunk, delete_upload_session,
    dispatch_upload_session, get_upload_dispatch_plan, get_upload_session, list_upload_sessions,
    load_upload_session, plan_lanes, sanitize, save_upload_session, BuildUploadSessionOptions,
    BuildUploadSessionResult, Chunk, ChunkAssignment, ChunkedContent, DispatchUploadOptions,
    EncryptedUploadSession, LanePlan, LanePlanError, LaneState, LaneStatus, SanitizationReport,
    SanitizedBytes, SanitizerFormatHint, SendUploadSessionResult, SessionBuildError,
    UploadDispatchPlan, UploadManifest, UploadSessionStatus, UploadSessionSummary,
    MANIFEST_CHUNK_INDEX,
};
pub use session::CreatorSession;
pub use upload::{CreatorBridgeFrameFragment, CreatorBridgeRequest, CreatorBridgeResponse};
