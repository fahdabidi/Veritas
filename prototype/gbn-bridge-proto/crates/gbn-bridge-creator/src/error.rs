use thiserror::Error;

#[derive(Debug, Error)]
pub enum CreatorError {
    #[error("authority bootstrap failed: {0}")]
    BootstrapFailed(String),

    #[error("no bridge assigned by authority")]
    NoBridgeAssigned,

    #[error("frame upload to bridge failed: {0}")]
    FrameUploadFailed(String),

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
