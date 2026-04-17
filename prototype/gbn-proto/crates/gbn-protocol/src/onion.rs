use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// Outer envelope representing traffic traversing a Telescopic Circuit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OnionCell {
    /// Request the current node to dial a new downstream hop.
    RelayExtend(ExtendPayload),

    /// Inform the client that the extension succeeded or failed.
    RelayExtended(ExtendedPayload),

    /// A bi-directional heartbeat to maintain active connections.
    RelayHeartbeat(HeartbeatPayload),

    /// Standard data payload that will be forwarded to the next hop or the Exit node.
    RelayData(DataPayload),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtendPayload {
    /// The address of the next hop to dial.
    pub next_hop: SocketAddr,
    /// The identity key of the next hop (to verify their Snow handshake).
    pub next_identity_key: [u8; 32],
    /// The first stage of a Noise_XX handshake intended for the next hop.
    pub handshake_payload: Vec<u8>,
    /// Distributed trace chain ID propagated from the circuit initiator.
    /// Relays include this in their ring-buffer entries so failures on any hop
    /// can be correlated back to the originating SendDummy / Creator operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtendedPayload {
    /// The response stage of the Noise_XX handshake if successful.
    pub handshake_response: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatPayload {
    pub seq_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataPayload {
    /// The inner payload, fully opaque to non-destination relays.
    pub ciphertext: Vec<u8>,
}
