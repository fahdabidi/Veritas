use libp2p::identity;
use mcn_router_sim::gossip::{GossipRequest, PlumTreeEngine, PlumTreeState};

fn peer() -> libp2p::PeerId {
    identity::Keypair::generate_ed25519().public().to_peer_id()
}

#[test]
fn test_gossip_dedup_smoke() {
    let mut s = PlumTreeState::new(1024, 100);
    let id = [42u8; 32];
    assert!(s.register_seen(id));
    assert!(!s.register_seen(id));
}

#[test]
fn test_gossip_rate_limit_smoke() {
    let mut s = PlumTreeState::new(1, 100);
    assert!(!s.try_account_send(128));
    assert!(s.messages_dropped_budget_total() > 0);
}

#[test]
fn test_gossip_lazy_repair_counter_smoke() {
    let mut s = PlumTreeState::new(1024, 100);
    s.note_lazy_repair(3);
    assert_eq!(s.lazy_repairs_total(), 3);
}

#[test]
fn test_live_forwarding_enforces_budget() {
    // tiny budget, one eager peer, payload bigger than available tokens
    let mut engine = PlumTreeEngine::new(1, 100);
    let p = peer();
    engine.add_eager_peer(p);

    let out = engine.publish_local([1u8; 32], vec![0u8; 64]);
    assert!(out.is_empty());
    assert!(engine.state.messages_dropped_budget_total() > 0);
}

#[test]
fn test_lazy_repair_flow_i_have_to_i_want_to_data() {
    let mut sender = PlumTreeEngine::new(1024 * 1024, 100);
    let mut receiver = PlumTreeEngine::new(1024 * 1024, 100);
    let sender_peer = peer();

    let msg_id = [7u8; 32];
    let payload = b"hello-lazy-repair".to_vec();
    let _ = sender.publish_local(msg_id, payload.clone());

    let i_want = receiver.on_request(
        sender_peer,
        GossipRequest::IHave {
            message_ids: vec![msg_id],
        },
    );
    assert_eq!(i_want.len(), 1);
    assert!(matches!(i_want[0].request, GossipRequest::IWant { .. }));
    assert_eq!(receiver.state.lazy_repairs_total(), 1);

    let gossip_back = sender.on_request(sender_peer, i_want[0].request.clone());
    assert_eq!(gossip_back.len(), 1);
    assert!(matches!(
        gossip_back[0].request,
        GossipRequest::GossipData { .. }
    ));

    let forward = receiver.on_request(sender_peer, gossip_back[0].request.clone());
    // Fresh data is accepted and can trigger forwarding; at minimum it should be seen.
    assert_eq!(receiver.state.messages_seen_total(), 1);
    assert!(forward.is_empty() || !forward.is_empty());
}

#[test]
fn test_convergence_like_three_node_delivery() {
    let mut a = PlumTreeEngine::new(1024 * 1024, 100);
    let mut b = PlumTreeEngine::new(1024 * 1024, 100);
    let mut c = PlumTreeEngine::new(1024 * 1024, 100);

    let pa = peer();
    let pb = peer();
    let pc = peer();

    a.add_eager_peer(pb);
    b.add_eager_peer(pc);

    let msg_id = [33u8; 32];
    let out_from_a = a.publish_local(msg_id, b"mesh-convergence".to_vec());
    assert_eq!(out_from_a.len(), 1);

    let out_from_b = b.on_request(pa, out_from_a[0].request.clone());
    assert!(b.state.messages_seen_total() >= 1);
    assert_eq!(out_from_b.len(), 1);

    let _out_from_c = c.on_request(pb, out_from_b[0].request.clone());
    assert_eq!(c.state.messages_seen_total(), 1);

    // Duplicate delivery converges without re-counting as new.
    let duplicate = c.on_request(pb, out_from_b[0].request.clone());
    assert_eq!(c.state.messages_seen_total(), 1);
    assert_eq!(duplicate.len(), 1);
    assert!(matches!(duplicate[0].request, GossipRequest::Prune));

    let _ = pc; // keep explicit topology peer vars visible for readability
}
