use gbn_bridge_protocol::{BridgeAck, BridgeClose, BridgeData, BridgeOpen};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum CreatorBridgeRequest {
    Open(BridgeOpen),
    Frame(BridgeData),
    Close(BridgeClose),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum CreatorBridgeResponse {
    Opened {
        chain_id: String,
        session_id: String,
    },
    Ack(BridgeAck),
    Closed {
        chain_id: String,
        session_id: String,
    },
    Error {
        message: String,
    },
}
