use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("protocol version {actual} is not supported; expected {expected}")]
    UnsupportedProtocolVersion { actual: u16, expected: u16 },

    #[error("{object} expired at {expiry_ms}; now={now_ms}")]
    Expired {
        object: &'static str,
        expiry_ms: u64,
        now_ms: u64,
    },

    #[error(
        "message replay window expired; sent_at={sent_at_ms}, now={now_ms}, max_age={max_age_ms}"
    )]
    ReplayWindowExpired {
        sent_at_ms: u64,
        now_ms: u64,
        max_age_ms: u64,
    },

    #[error("message timestamp {sent_at_ms} is ahead of now={now_ms}")]
    ReplayTimestampInFuture { sent_at_ms: u64, now_ms: u64 },

    #[error("signature is missing")]
    MissingSignature,

    #[error("invalid public key length {actual}; expected 32 bytes")]
    InvalidPublicKeyLength { actual: usize },

    #[error("invalid signature length {actual}; expected 64 bytes")]
    InvalidSignatureLength { actual: usize },

    #[error("signature verification failed")]
    InvalidSignature,

    #[error("bridge descriptor must include at least one ingress endpoint")]
    EmptyIngressEndpoints,

    #[error("bridge set response must include at least one bridge entry")]
    EmptyBridgeSet,

    #[error("batch assignment must include at least one creator assignment")]
    EmptyBatchAssignments,

    #[error("udp punch port must be non-zero")]
    InvalidUdpPunchPort,

    #[error("serialization error: {0}")]
    Serialization(String),
}

impl From<serde_json::Error> for ProtocolError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value.to_string())
    }
}
