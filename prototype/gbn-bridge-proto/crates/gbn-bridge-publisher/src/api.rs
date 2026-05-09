use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    sign_payload, verify_payload, BootstrapJoinReply, BootstrapProgress, BridgeAck,
    BridgeCatalogRequest, BridgeCatalogResponse, BridgeClose, BridgeData, BridgeHeartbeat,
    BridgeLease, BridgeOpen, BridgeRegister, CreatorJoinRequest, ProtocolError, PublicKeyBytes,
    ReachabilityClass, SignatureBytes,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityApiAuth {
    pub actor_pub: PublicKeyBytes,
    pub signature: SignatureBytes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityApiRequestUnsigned<T> {
    pub chain_id: String,
    pub request_id: String,
    pub sent_at_ms: u64,
    pub actor_id: String,
    pub body: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityApiRequest<T> {
    pub chain_id: String,
    pub request_id: String,
    pub sent_at_ms: u64,
    pub actor_id: String,
    pub body: T,
    pub auth: AuthorityApiAuth,
}

impl<T> AuthorityApiRequest<T>
where
    T: Serialize + Clone,
{
    pub fn sign(
        unsigned: AuthorityApiRequestUnsigned<T>,
        signing_key: &SigningKey,
    ) -> Result<Self, ProtocolError> {
        let signature = sign_payload(&unsigned, signing_key)?;
        Ok(Self {
            chain_id: unsigned.chain_id,
            request_id: unsigned.request_id,
            sent_at_ms: unsigned.sent_at_ms,
            actor_id: unsigned.actor_id,
            body: unsigned.body,
            auth: AuthorityApiAuth {
                actor_pub: PublicKeyBytes::from_verifying_key(&signing_key.verifying_key()),
                signature,
            },
        })
    }

    pub fn unsigned_payload(&self) -> AuthorityApiRequestUnsigned<T> {
        AuthorityApiRequestUnsigned {
            chain_id: self.chain_id.clone(),
            request_id: self.request_id.clone(),
            sent_at_ms: self.sent_at_ms,
            actor_id: self.actor_id.clone(),
            body: self.body.clone(),
        }
    }

    pub fn verify_signature(&self) -> Result<(), ProtocolError> {
        verify_payload(
            &self.unsigned_payload(),
            &self.auth.actor_pub,
            &self.auth.signature,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityApiErrorBody {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EmptyResponse;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityApiResponseUnsigned<T> {
    pub chain_id: String,
    pub request_id: String,
    pub served_at_ms: u64,
    pub ok: bool,
    pub body: Option<T>,
    pub error: Option<AuthorityApiErrorBody>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityApiResponse<T> {
    pub chain_id: String,
    pub request_id: String,
    pub served_at_ms: u64,
    pub ok: bool,
    pub body: Option<T>,
    pub error: Option<AuthorityApiErrorBody>,
    pub publisher_sig: SignatureBytes,
}

impl<T> AuthorityApiResponse<T>
where
    T: Serialize + Clone,
{
    pub fn sign(
        unsigned: AuthorityApiResponseUnsigned<T>,
        signing_key: &SigningKey,
    ) -> Result<Self, ProtocolError> {
        let publisher_sig = sign_payload(&unsigned, signing_key)?;
        Ok(Self {
            chain_id: unsigned.chain_id,
            request_id: unsigned.request_id,
            served_at_ms: unsigned.served_at_ms,
            ok: unsigned.ok,
            body: unsigned.body,
            error: unsigned.error,
            publisher_sig,
        })
    }

    pub fn unsigned_payload(&self) -> AuthorityApiResponseUnsigned<T> {
        AuthorityApiResponseUnsigned {
            chain_id: self.chain_id.clone(),
            request_id: self.request_id.clone(),
            served_at_ms: self.served_at_ms,
            ok: self.ok,
            body: self.body.clone(),
            error: self.error.clone(),
        }
    }

    pub fn verify_authority(&self, publisher_key: &PublicKeyBytes) -> Result<(), ProtocolError> {
        verify_payload(&self.unsigned_payload(), publisher_key, &self.publisher_sig)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeRegisterBody {
    pub register: BridgeRegister,
    pub reachability_class: ReachabilityClass,
    pub now_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeHeartbeatBody {
    pub heartbeat: BridgeHeartbeat,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatorCatalogBody {
    pub request: BridgeCatalogRequest,
    pub now_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapJoinBody {
    pub request: CreatorJoinRequest,
    pub now_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapProgressBody {
    pub progress: BootstrapProgress,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiverOpenBody {
    pub open: BridgeOpen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiverFrameBody {
    pub via_bridge_id: String,
    pub frame: BridgeData,
    pub received_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiverCloseBody {
    pub bridge_id: String,
    pub close: BridgeClose,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapProgressReceipt {
    pub bootstrap_session_id: String,
    pub reporter_id: String,
    pub stored_event_count: usize,
    pub latest_stage: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityRoute {
    Healthz,
    Readyz,
    BridgeRegister,
    BridgeHeartbeat,
    BridgeProgress,
    CreatorCatalog,
    BootstrapJoin,
    ReceiverOpen,
    ReceiverFrame,
    ReceiverClose,
    AdminBridges,
    AdminFrames,
    AdminMetrics,
    AdminNodeMetadata,
    AdminEchoChainId,
    AdminBootstrapSession,
    AdminLocalDht,
    AdminResetCreatorState,
    AdminSeedHostCreator,
    AdminSeedNewCreator,
    AdminStartBootstrap,
    AdminHostJoinRelay,
    AdminBridgeCommand,
    AdminBridgeDhtEntry,
    AdminInitializePublisherDht,
    AdminCreatorDhtEntry,
    AdminSendDummy,
    AdminDiscoveryProbe,
}

impl AuthorityRoute {
    pub fn path(self) -> &'static str {
        match self {
            Self::Healthz => "/healthz",
            Self::Readyz => "/readyz",
            Self::BridgeRegister => "/v1/bridge/register",
            Self::BridgeHeartbeat => "/v1/bridge/heartbeat",
            Self::BridgeProgress => "/v1/bridge/progress",
            Self::CreatorCatalog => "/v1/creator/catalog",
            Self::BootstrapJoin => "/v1/bootstrap/join",
            Self::ReceiverOpen => "/v1/receiver/open",
            Self::ReceiverFrame => "/v1/receiver/frame",
            Self::ReceiverClose => "/v1/receiver/close",
            Self::AdminBridges => "/v1/admin/bridges",
            Self::AdminFrames => "/v1/admin/frames",
            Self::AdminMetrics => "/v1/admin/metrics",
            Self::AdminNodeMetadata => "/v1/admin/node-metadata",
            Self::AdminEchoChainId => "/v1/admin/echo-chain-id",
            Self::AdminBootstrapSession => "/v1/admin/bootstrap-session",
            Self::AdminLocalDht => "/v1/admin/local-dht",
            Self::AdminResetCreatorState => "/v1/admin/reset-creator-state",
            Self::AdminSeedHostCreator => "/v1/admin/seed-host-creator",
            Self::AdminSeedNewCreator => "/v1/admin/seed-new-creator",
            Self::AdminStartBootstrap => "/v1/admin/start-bootstrap",
            Self::AdminHostJoinRelay => "/v1/admin/host/join",
            Self::AdminBridgeCommand => "/v1/admin/bridges/:bridge_id/command",
            Self::AdminBridgeDhtEntry => "/v1/admin/bridges/:bridge_id/dht-entry",
            Self::AdminInitializePublisherDht => "/v1/admin/publisher-dht/initialize",
            Self::AdminCreatorDhtEntry => "/v1/admin/creator-dht-entry",
            Self::AdminSendDummy => "/v1/admin/send-dummy",
            Self::AdminDiscoveryProbe => "/v1/admin/discovery-probe",
        }
    }
}

pub type RegisterBridgeResponse = AuthorityApiResponse<BridgeLease>;
pub type HeartbeatResponse = AuthorityApiResponse<BridgeLease>;
pub type CreatorCatalogResponse = AuthorityApiResponse<BridgeCatalogResponse>;
pub type BootstrapJoinResponse = AuthorityApiResponse<BootstrapJoinReply>;
pub type BootstrapProgressResponse = AuthorityApiResponse<BootstrapProgressReceipt>;
pub type ReceiverOpenResponse = AuthorityApiResponse<EmptyResponse>;
pub type ReceiverFrameResponse = AuthorityApiResponse<BridgeAck>;
pub type ReceiverCloseResponse = AuthorityApiResponse<EmptyResponse>;
