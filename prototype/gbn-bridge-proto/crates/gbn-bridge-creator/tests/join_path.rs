use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{publisher_identity, CreatorJoinRequest, PendingCreator};

fn public_key(seed: u8) -> gbn_bridge_protocol::PublicKeyBytes {
    publisher_identity(&SigningKey::from_bytes(&[seed; 32]))
}

#[test]
fn creator_join_request_preserves_distinct_new_host_and_relay_actors() {
    let request = CreatorJoinRequest {
        chain_id: "seed-new-creator-creator-new-1000".to_string(),
        request_id: "seed-new-creator-creator-new-1000-join".to_string(),
        host_creator_id: "creator-host".to_string(),
        relay_bridge_id: "exit-bridge-a".to_string(),
        creator: PendingCreator {
            node_id: "creator-new".to_string(),
            ip_addr: "127.0.0.1".to_string(),
            pub_key: public_key(20),
            encryption_pub_key: None,
            udp_punch_port: 4443,
        },
    };

    assert_ne!(request.creator.node_id, request.host_creator_id);
    assert_ne!(request.creator.node_id, request.relay_bridge_id);
    assert_ne!(request.host_creator_id, request.relay_bridge_id);
}
