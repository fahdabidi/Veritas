use gbn_bridge_runtime::{AckTracker, BridgePool, ChunkSender, InProcessPublisherClient};

use crate::common::*;

#[test]
fn bridge_and_publisher_preserve_opaque_frame_payload_without_transformation() {
    let shared_client = InProcessPublisherClient::new(publisher());
    let mut bridge = startup_bridge("bridge-a", 91, "198.51.100.80", &shared_client, 1_000);
    let mut creator = creator_runtime("creator-confidentiality", 92, "203.0.113.60");
    prime_creator(&mut creator, &mut bridge, 1_100);

    let pool = BridgePool::from_creator(&creator, 10).unwrap();
    let mut sender = ChunkSender::default();
    let payload = b"\x9f\x82opaque-frame-vector\x00\x7f".to_vec();
    let session = sender.begin_session(&creator, &payload, 2_000).unwrap();
    let mut tracker = AckTracker::new(&session);

    sender
        .open_selected_bridges(
            &session,
            &pool.selected_bridge_ids(),
            std::slice::from_mut(&mut bridge),
            2_000,
        )
        .unwrap();

    let dispatch = gbn_bridge_runtime::FrameDispatch {
        bridge_id: "bridge-a".into(),
        frame: session.frames()[0].clone(),
        reused_bridge: false,
    };
    let _ = sender
        .send_dispatches(
            &[dispatch],
            std::slice::from_mut(&mut bridge),
            &mut tracker,
            2_010,
        )
        .unwrap();

    let forwarded = &bridge.local_forwarded_frames()[0].frame;
    let authority = shared_client.authority();
    let session_record = authority.upload_session(session.session_id()).unwrap();
    let ingested = &session_record.frames_by_sequence[&0].frame;

    assert_eq!(forwarded.ciphertext, session.frames()[0].ciphertext);
    assert_eq!(ingested.ciphertext, session.frames()[0].ciphertext);
    assert_eq!(forwarded.ciphertext, ingested.ciphertext);
}
