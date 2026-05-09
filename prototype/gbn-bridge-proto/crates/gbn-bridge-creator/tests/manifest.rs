use gbn_bridge_creator::{SanitizationReport, UploadManifest};
use gbn_bridge_protocol::PublicKeyBytes;

#[test]
fn manifest_serialization_round_trip_is_stable() {
    let manifest = UploadManifest {
        session_id: vec![1; 16],
        creator_ephemeral_pubkey: PublicKeyBytes(vec![2; 32]),
        publisher_key_id: "publisher".to_string(),
        total_chunks: 4,
        content_hash: vec![3; 32],
        sanitization_profile: "v3-default-no-visual-anon".to_string(),
        sanitization_report: SanitizationReport {
            exif_segments_stripped: 1,
            ..SanitizationReport::default()
        },
        created_at_ms: 1234,
        chunk_size: 8192,
        total_bytes: 32768,
    };
    let encoded = serde_json::to_vec(&manifest).unwrap();
    let decoded: UploadManifest = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, manifest);
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), encoded);
}
