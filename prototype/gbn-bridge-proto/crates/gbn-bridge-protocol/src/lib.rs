//! Canonical Conduit V2 protocol schema set for bridge-mode transport.

pub mod bootstrap;
pub mod catalog;
pub mod descriptor;
pub mod error;
pub mod lease;
pub mod messages;
pub mod punch;
pub mod session;
pub mod signing;

pub use bootstrap::{
    BootstrapDhtEntry, BootstrapDhtEntryUnsigned, BridgeSetRequest, BridgeSetResponse,
    BridgeSetResponseUnsigned, CreatorBootstrapResponse, CreatorBootstrapResponseUnsigned,
    CreatorJoinRequest, PendingCreator,
};
pub use catalog::{
    BridgeCatalogRequest, BridgeCatalogResponse, BridgeCatalogResponseUnsigned, BridgeRefreshHint,
    RefreshHintReason,
};
pub use descriptor::{
    BridgeCapability, BridgeDescriptor, BridgeDescriptorUnsigned, BridgeIngressEndpoint,
    ReachabilityClass,
};
pub use error::ProtocolError;
pub use lease::{
    BridgeHeartbeat, BridgeLease, BridgeLeaseUnsigned, BridgeRegister, BridgeRevoke,
    BridgeRevokeUnsigned, RevocationReason,
};
pub use messages::{
    ProtocolEnvelope, ProtocolMessage, ProtocolVersion, ReplayProtection, CURRENT_PROTOCOL_VERSION,
};
pub use punch::{
    BatchAssignment, BootstrapProgress, BootstrapProgressStage, BridgeBatchAssign,
    BridgeBatchAssignUnsigned, BridgePunchAck, BridgePunchProbe, BridgePunchStart,
    BridgePunchStartUnsigned,
};
pub use session::{
    BridgeAck, BridgeAckStatus, BridgeClose, BridgeCloseReason, BridgeData, BridgeOpen,
};
pub use signing::{
    canonical_json_bytes, ensure_not_expired, publisher_identity, sign_payload, verify_payload,
    PublicKeyBytes, SignatureBytes,
};

/// Default UDP punch port reserved for early Conduit bridge sessions.
pub const DEFAULT_UDP_PUNCH_PORT: u16 = 443;

/// Shared millisecond timestamp representation used throughout the Phase 2 wire model.
pub type UnixTimestampMs = u64;
