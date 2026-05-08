use thiserror::Error;

use crate::client::BridgeFilterDrops;

#[derive(Debug, Error)]
pub enum CreatorError {
    #[error("authority bootstrap failed: {0}")]
    BootstrapFailed(String),

    #[error("no bridge assigned by authority")]
    NoBridgeAssigned,

    #[error("frame upload to bridge failed: {0}")]
    FrameUploadFailed(String),

    #[error("selected node has not completed NewCreator onboarding")]
    CreatorNotOnboarded { current_state: String },

    #[error("no active publisher-signed direct/brokered bridge available in local DHT")]
    NoEligibleBridge { filter_drops: BridgeFilterDrops },

    #[error("local DHT is missing a trusted Publisher entry")]
    MissingPublisherEntry,

    #[error("local DHT error: {0}")]
    LocalDht(String),

    #[error("transport error during {operation}: {detail}")]
    Transport {
        operation: &'static str,
        detail: String,
    },

    #[error("protocol error: {0}")]
    Protocol(String),
}

impl From<gbn_bridge_protocol::ProtocolError> for CreatorError {
    fn from(value: gbn_bridge_protocol::ProtocolError) -> Self {
        Self::Protocol(value.to_string())
    }
}
