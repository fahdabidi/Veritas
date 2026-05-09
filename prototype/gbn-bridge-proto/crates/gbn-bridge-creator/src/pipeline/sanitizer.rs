use serde::{Deserialize, Serialize};

const JPEG_SOI: &[u8] = &[0xff, 0xd8];
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
const SYNTHETIC_MARKER: &[u8] = b"VERITAS-SMOKE-4-PLAINTEXT";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SanitizerFormatHint {
    Synthetic,
    Jpeg,
    Png,
    Mp4,
    Opaque,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SanitizationReport {
    pub exif_segments_stripped: u32,
    pub container_metadata_blocks_stripped: u32,
    pub encoder_id_strings_stripped: u32,
    pub timestamps_normalized: u32,
    pub synthetic_marker_zeroed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SanitizedBytes {
    pub bytes: Vec<u8>,
    pub report: SanitizationReport,
}

pub fn sanitize(input: &[u8], format_hint: SanitizerFormatHint) -> SanitizedBytes {
    match format_hint {
        SanitizerFormatHint::Synthetic => sanitize_synthetic(input),
        SanitizerFormatHint::Jpeg => sanitize_jpeg(input),
        SanitizerFormatHint::Png => sanitize_png(input),
        SanitizerFormatHint::Mp4 => sanitize_mp4(input),
        SanitizerFormatHint::Opaque => SanitizedBytes {
            bytes: input.to_vec(),
            report: SanitizationReport::default(),
        },
    }
}

fn sanitize_synthetic(input: &[u8]) -> SanitizedBytes {
    let mut bytes = input.to_vec();
    let marker_len = SYNTHETIC_MARKER.len().min(bytes.len());
    let synthetic_marker_zeroed =
        marker_len > 0 && bytes[..marker_len] == SYNTHETIC_MARKER[..marker_len];
    if synthetic_marker_zeroed {
        for byte in &mut bytes[..marker_len] {
            *byte = 0;
        }
    }
    SanitizedBytes {
        bytes,
        report: SanitizationReport {
            synthetic_marker_zeroed,
            ..SanitizationReport::default()
        },
    }
}

fn sanitize_jpeg(input: &[u8]) -> SanitizedBytes {
    if !input.starts_with(JPEG_SOI) {
        return SanitizedBytes {
            bytes: input.to_vec(),
            report: SanitizationReport::default(),
        };
    }

    let mut output = Vec::with_capacity(input.len());
    output.extend_from_slice(JPEG_SOI);
    let mut i = 2;
    let mut report = SanitizationReport::default();

    while i + 1 < input.len() {
        if input[i] != 0xff {
            output.extend_from_slice(&input[i..]);
            break;
        }
        let marker = input[i + 1];
        if marker == 0xd9 {
            output.extend_from_slice(&input[i..]);
            break;
        }
        if marker == 0xda {
            output.extend_from_slice(&input[i..]);
            break;
        }
        if i + 4 > input.len() {
            output.extend_from_slice(&input[i..]);
            break;
        }
        let len = u16::from_be_bytes([input[i + 2], input[i + 3]]) as usize;
        if len < 2 || i + 2 + len > input.len() {
            output.extend_from_slice(&input[i..]);
            break;
        }
        if marker == 0xe1 {
            report.exif_segments_stripped += 1;
        } else {
            output.extend_from_slice(&input[i..i + 2 + len]);
        }
        i += 2 + len;
    }

    SanitizedBytes {
        bytes: output,
        report,
    }
}

fn sanitize_png(input: &[u8]) -> SanitizedBytes {
    if !input.starts_with(PNG_SIGNATURE) {
        return SanitizedBytes {
            bytes: input.to_vec(),
            report: SanitizationReport::default(),
        };
    }

    let mut output = Vec::with_capacity(input.len());
    output.extend_from_slice(PNG_SIGNATURE);
    let mut i = PNG_SIGNATURE.len();
    let mut report = SanitizationReport::default();
    while i + 12 <= input.len() {
        let len = u32::from_be_bytes([input[i], input[i + 1], input[i + 2], input[i + 3]]) as usize;
        let end = i + 12 + len;
        if end > input.len() {
            output.extend_from_slice(&input[i..]);
            break;
        }
        let chunk_type = &input[i + 4..i + 8];
        if matches!(chunk_type, b"tEXt" | b"iTXt" | b"zTXt") {
            report.container_metadata_blocks_stripped += 1;
        } else {
            output.extend_from_slice(&input[i..end]);
        }
        i = end;
    }
    if i < input.len() {
        output.extend_from_slice(&input[i..]);
    }

    SanitizedBytes {
        bytes: output,
        report,
    }
}

fn sanitize_mp4(input: &[u8]) -> SanitizedBytes {
    let mut output = Vec::with_capacity(input.len());
    let mut i = 0;
    let mut report = SanitizationReport::default();
    while i + 8 <= input.len() {
        let size =
            u32::from_be_bytes([input[i], input[i + 1], input[i + 2], input[i + 3]]) as usize;
        if size < 8 || i + size > input.len() {
            output.extend_from_slice(&input[i..]);
            break;
        }
        let box_type = &input[i + 4..i + 8];
        if box_type == b"udta" {
            report.container_metadata_blocks_stripped += 1;
            report.timestamps_normalized += 1;
        } else {
            output.extend_from_slice(&input[i..i + size]);
        }
        i += size;
    }
    if i < input.len() {
        output.extend_from_slice(&input[i..]);
    }

    SanitizedBytes {
        bytes: output,
        report,
    }
}
