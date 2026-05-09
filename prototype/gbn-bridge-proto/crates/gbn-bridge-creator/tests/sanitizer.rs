use gbn_bridge_creator::{sanitize, SanitizerFormatHint};

fn png_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out.extend_from_slice(&0_u32.to_be_bytes());
    out
}

fn mp4_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

#[test]
fn jpeg_exif_app1_segment_is_stripped() {
    let input = [
        vec![0xff, 0xd8],
        vec![0xff, 0xe1, 0x00, 0x08],
        b"Exif!!".to_vec(),
        vec![0xff, 0xe0, 0x00, 0x04, 0x11, 0x22],
        vec![0xff, 0xda, 0x00, 0x04, 0x33, 0x44, 0xff, 0xd9],
    ]
    .concat();
    let sanitized = sanitize(&input, SanitizerFormatHint::Jpeg);
    assert_eq!(sanitized.report.exif_segments_stripped, 1);
    assert!(sanitized.bytes.starts_with(&[0xff, 0xd8]));
    assert!(!sanitized
        .bytes
        .windows(2)
        .any(|window| window == [0xff, 0xe1]));
    assert!(sanitized
        .bytes
        .windows(2)
        .any(|window| window == [0xff, 0xda]));
}

#[test]
fn png_text_chunks_are_removed_and_idat_is_kept() {
    let input = [
        b"\x89PNG\r\n\x1a\n".to_vec(),
        png_chunk(b"tEXt", b"camera=serial"),
        png_chunk(b"IDAT", b"pixels"),
        png_chunk(b"IEND", b""),
    ]
    .concat();
    let sanitized = sanitize(&input, SanitizerFormatHint::Png);
    assert_eq!(sanitized.report.container_metadata_blocks_stripped, 1);
    assert!(!sanitized.bytes.windows(4).any(|window| window == b"tEXt"));
    assert!(sanitized.bytes.windows(4).any(|window| window == b"IDAT"));
}

#[test]
fn mp4_udta_box_is_removed_and_mdat_is_kept() {
    let input = [
        mp4_box(b"ftyp", b"isom"),
        mp4_box(b"udta", b"device timestamp"),
        mp4_box(b"mdat", b"media bytes"),
    ]
    .concat();
    let sanitized = sanitize(&input, SanitizerFormatHint::Mp4);
    assert_eq!(sanitized.report.container_metadata_blocks_stripped, 1);
    assert_eq!(sanitized.report.timestamps_normalized, 1);
    assert!(!sanitized.bytes.windows(4).any(|window| window == b"udta"));
    assert!(sanitized.bytes.windows(4).any(|window| window == b"mdat"));
}

#[test]
fn synthetic_mode_zeroes_smoke_marker_prefix() {
    let input = b"VERITAS-SMOKE-4-PLAINTEXT-payload".to_vec();
    let sanitized = sanitize(&input, SanitizerFormatHint::Synthetic);
    assert!(sanitized.report.synthetic_marker_zeroed);
    assert_eq!(
        &sanitized.bytes[..b"VERITAS-SMOKE-4-PLAINTEXT".len()],
        vec![0_u8; b"VERITAS-SMOKE-4-PLAINTEXT".len()].as_slice()
    );
    assert!(sanitized.bytes.ends_with(b"-payload"));
}

#[test]
fn sanitizer_is_idempotent_for_supported_formats() {
    let input = [
        b"\x89PNG\r\n\x1a\n".to_vec(),
        png_chunk(b"tEXt", b"camera=serial"),
        png_chunk(b"IDAT", b"pixels"),
        png_chunk(b"IEND", b""),
    ]
    .concat();
    let once = sanitize(&input, SanitizerFormatHint::Png);
    let twice = sanitize(&once.bytes, SanitizerFormatHint::Png);
    assert_eq!(once.bytes, twice.bytes);
}
