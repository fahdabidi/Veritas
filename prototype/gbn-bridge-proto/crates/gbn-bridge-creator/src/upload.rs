use gbn_bridge_protocol::{BridgeAck, BridgeClose, BridgeData, BridgeOpen};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum CreatorBridgeRequest {
    Open(BridgeOpen),
    Frame(BridgeData),
    FrameFragment(CreatorBridgeFrameFragment),
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
    FrameFragmentAccepted {
        chain_id: String,
        session_id: String,
        frame_id: String,
        fragment_index: u16,
        total_fragments: u16,
    },
    Closed {
        chain_id: String,
        session_id: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatorBridgeFrameFragment {
    pub chain_id: String,
    pub session_id: String,
    pub frame_id: String,
    pub sequence: u32,
    pub fragment_index: u16,
    pub total_fragments: u16,
    pub frame_bytes_b64: String,
}

impl CreatorBridgeFrameFragment {
    pub fn new(
        frame: &BridgeData,
        fragment_index: u16,
        total_fragments: u16,
        frame_bytes: &[u8],
    ) -> Self {
        Self {
            chain_id: frame.chain_id.clone(),
            session_id: frame.session_id.clone(),
            frame_id: frame.frame_id.clone(),
            sequence: frame.sequence,
            fragment_index,
            total_fragments,
            frame_bytes_b64: base64_encode(frame_bytes),
        }
    }

    pub fn decoded_frame_bytes(&self) -> Result<Vec<u8>, String> {
        base64_decode(&self.frame_bytes_b64)
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut bits = 0_u32;
    let mut bit_count = 0_u8;
    let mut out = Vec::new();
    for byte in input.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        if byte == b'=' {
            break;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return Err(format!("invalid base64 byte `{}`", byte as char)),
        } as u32;
        bits = (bits << 6) | value;
        bit_count += 6;
        if bit_count >= 8 {
            bit_count -= 8;
            out.push((bits >> bit_count) as u8);
            bits &= (1 << bit_count) - 1;
        }
    }
    Ok(out)
}
