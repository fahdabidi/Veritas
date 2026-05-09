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
    get_upload_session, list_upload_sessions, sanitize, BuildUploadSessionOptions,
    BuildUploadSessionResult, Chunk, ChunkedContent, EncryptedUploadSession, SanitizationReport,
    SanitizedBytes, SanitizerFormatHint, SessionBuildError, UploadManifest, UploadSessionStatus,
    UploadSessionSummary, MANIFEST_CHUNK_INDEX,
};
pub use session::CreatorSession;
pub use upload::{CreatorBridgeRequest, CreatorBridgeResponse};
