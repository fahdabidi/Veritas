use gbn_bridge_protocol::BridgeCloseReason;
use gbn_bridge_runtime::{
    establish_seed_tunnel, fetch_bridge_set, request_first_contact, ChunkSender,
    InProcessPublisherClient,
};

use crate::common::*;

#[test]
fn bootstrap_chain_id_is_preserved_end_to_end() {
    let shared_client = InProcessPublisherClient::new(publisher());
    let mut relay_bridge =
        startup_bridge("bridge-relay", 61, "198.51.100.60", &shared_client, 1_000);
    let mut seed_bridge = startup_bridge("bridge-seed", 62, "198.51.100.61", &shared_client, 1_000);
    let _extra_bridge =
        startup_bridge("bridge-z-extra", 63, "198.51.100.62", &shared_client, 1_000);

    let mut creator = creator_runtime("creator-chain-01", 64, "203.0.113.60");
    let mut host_creator = host_creator();

    let plan = request_first_contact(
        &mut creator,
        &mut host_creator,
        &mut relay_bridge,
        "join-chain-e2e",
        2_000,
    )
    .unwrap();

    assert_eq!(plan.chain_id, "bootstrap-host-creator-01-join-chain-e2e");
    assert_eq!(plan.reply.chain_id, plan.chain_id);
    assert_eq!(plan.reply.response.chain_id, plan.chain_id);

    let seed_tunnel = establish_seed_tunnel(&mut creator, &mut seed_bridge, &plan, 2_010).unwrap();
    assert_eq!(seed_tunnel.probe.chain_id, plan.chain_id);
    assert_eq!(seed_tunnel.bridge_ack.chain_id, plan.chain_id);

    let bridge_set = fetch_bridge_set(&mut creator, &mut seed_bridge, &plan, 2_020).unwrap();
    assert_eq!(bridge_set.chain_id, plan.chain_id);

    let progress = seed_bridge.publisher_client().reported_progress();
    assert!(!progress.is_empty());
    assert!(progress.iter().all(|event| event.chain_id == plan.chain_id));

    let attempts = creator
        .begin_bootstrap_fanout(
            &plan.reply.response.bootstrap_session_id,
            &bridge_set,
            2_030,
        )
        .unwrap();
    assert!(!attempts.is_empty());
    assert!(attempts.iter().all(|attempt| attempt
        .bootstrap_session_id
        .starts_with(&plan.reply.response.bootstrap_session_id)));
}

#[test]
fn upload_chain_id_is_preserved_through_open_frames_and_acks() {
    let shared_client = InProcessPublisherClient::new(publisher());
    let mut bridge = startup_bridge("bridge-upload", 71, "198.51.100.70", &shared_client, 1_000);
    let mut creator = creator_runtime("creator-chain-01", 72, "203.0.113.70");
    prime_creator(&mut creator, &mut bridge, 1_100);

    let mut sender = ChunkSender::default();
    let session = sender
        .begin_session(&creator, b"phase-seven-chain-id-payload", 2_000)
        .unwrap();

    assert_eq!(session.chain_id(), "upload-creator-chain-01-upload-000001");
    assert!(session
        .frames()
        .iter()
        .all(|frame| frame.chain_id == session.chain_id()));

    let open = session.open_for_bridge("bridge-upload");
    assert_eq!(open.chain_id, session.chain_id());
    bridge
        .open_data_session_with_chain_id(session.chain_id(), open, 2_010)
        .unwrap();

    let ack = bridge
        .forward_session_frame_with_chain_id(session.chain_id(), session.frames()[0].clone(), 2_020)
        .unwrap();
    assert_eq!(ack.chain_id, session.chain_id());

    let close = session.close(BridgeCloseReason::Completed, 2_030);
    assert_eq!(close.chain_id, session.chain_id());
    bridge
        .close_data_session_with_chain_id(session.chain_id(), close, 2_030)
        .unwrap();

    let stored = bridge
        .publisher_client()
        .authority()
        .upload_session(session.session_id())
        .cloned()
        .unwrap();
    assert_eq!(stored.chain_id.as_deref(), Some(session.chain_id()));
    assert!(stored
        .frames_by_sequence
        .values()
        .all(|record| record.chain_id.as_deref() == Some(session.chain_id())));
}
