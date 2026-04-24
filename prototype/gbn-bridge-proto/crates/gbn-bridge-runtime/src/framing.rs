use gbn_bridge_protocol::BridgeData;

use crate::{RuntimeError, RuntimeResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramePayloadConfig {
    pub frame_size_bytes: usize,
}

impl Default for FramePayloadConfig {
    fn default() -> Self {
        Self {
            frame_size_bytes: 1024,
        }
    }
}

pub fn frame_payload(
    chain_id: &str,
    session_id: &str,
    payload: &[u8],
    sent_at_ms: u64,
    config: FramePayloadConfig,
) -> RuntimeResult<Vec<BridgeData>> {
    if config.frame_size_bytes == 0 {
        return Err(RuntimeError::NoActiveUploadBridge);
    }

    let chunks: Vec<Vec<u8>> = if payload.is_empty() {
        vec![Vec::new()]
    } else {
        payload
            .chunks(config.frame_size_bytes)
            .map(|chunk| chunk.to_vec())
            .collect()
    };

    Ok(chunks
        .into_iter()
        .enumerate()
        .map(|(index, ciphertext)| BridgeData {
            chain_id: chain_id.to_string(),
            session_id: session_id.to_string(),
            frame_id: format!("{session_id}-frame-{index:06}"),
            sequence: index as u32,
            sent_at_ms: sent_at_ms + index as u64,
            final_frame: index == chunks_len_minus_one(payload, config.frame_size_bytes),
            ciphertext,
        })
        .collect())
}

fn chunks_len_minus_one(payload: &[u8], frame_size_bytes: usize) -> usize {
    let frame_count = if payload.is_empty() {
        1
    } else {
        payload.len().div_ceil(frame_size_bytes)
    };
    frame_count - 1
}
