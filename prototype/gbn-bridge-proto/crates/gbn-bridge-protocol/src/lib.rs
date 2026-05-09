//! Canonical Conduit V2 protocol schema set for bridge-mode transport.

pub mod bootstrap;
pub mod catalog;
pub mod control;
pub mod descriptor;
pub mod dht;
pub mod envelope;
pub mod error;
pub mod lease;
pub mod messages;
pub mod punch;
pub mod session;
pub mod signing;
pub mod trace;

pub use bootstrap::{
    BootstrapDhtEntry, BootstrapDhtEntryUnsigned, BootstrapJoinReply, BridgeSeedAssign,
    BridgeSeedAssignUnsigned, BridgeSetRequest, BridgeSetResponse, BridgeSetResponseUnsigned,
    CreatorBootstrapResponse, CreatorBootstrapResponseUnsigned, CreatorJoinRequest, PendingCreator,
};
pub use catalog::{
    BridgeCatalogRequest, BridgeCatalogResponse, BridgeCatalogResponseUnsigned, BridgeRefreshHint,
    RefreshHintReason,
};
pub use control::{
    BridgeCommandAck, BridgeCommandAckStatus, BridgeCommandPayload, BridgeControlCommand,
    BridgeControlError, BridgeControlFrame, BridgeControlHello, BridgeControlHelloUnsigned,
    BridgeControlKeepalive, BridgeControlProgress, BridgeControlWelcome,
    BridgeControlWelcomeUnsigned,
};
pub use descriptor::{
    BridgeCapability, BridgeDescriptor, BridgeDescriptorUnsigned, BridgeIngressEndpoint,
    ReachabilityClass,
};
pub use dht::{
    BootstrapSession, BridgeDhtEntry, BridgeDhtEntryUnsigned,
    BridgeIngressEndpoint as DhtBridgeIngressEndpoint, BridgeIngressEndpointKind, CreatorDhtEntry,
    CreatorDhtEntryUnsigned, HostCreatorSeedState, HostRoleState, LocalDiscoveryTable,
    NewCreatorSeedState, PublisherDhtEntry, SelfOnboardingState, TunnelPeerRole, TunnelState,
    LOCAL_DISCOVERY_TABLE_SCHEMA_VERSION,
};
pub use envelope::{
    decrypt_from_creator, encrypt_for_publisher, publisher_encryption_identity,
    publisher_encryption_private_from_signing_key, EncryptedFrame, EnvelopeKeyDerivation,
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
    canonical_json_bytes, ensure_not_expired, ensure_replay_window, publisher_identity,
    sign_payload, verify_payload, PublicKeyBytes, SignatureBytes,
};
pub use trace::{validate_chain_id, ChainId, CHAIN_ID_FIELD_NAME};

/// Default UDP punch port reserved for early Conduit bridge sessions.
pub const DEFAULT_UDP_PUNCH_PORT: u16 = 443;

/// Shared millisecond timestamp representation used throughout the Phase 2 wire model.
pub type UnixTimestampMs = u64;
