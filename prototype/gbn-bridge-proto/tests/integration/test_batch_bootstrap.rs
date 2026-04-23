use gbn_bridge_publisher::PublisherAuthority;

use crate::common::*;

#[test]
fn eleventh_join_request_rolls_into_the_next_batch_window() {
    let mut authority = PublisherAuthority::new(publisher_signing_key());
    authority
        .register_bridge(
            bridge_register("bridge-a", 71, "198.51.100.60", 443),
            gbn_bridge_protocol::ReachabilityClass::Direct,
            1_000,
        )
        .unwrap();
    authority
        .register_bridge(
            bridge_register("bridge-b", 72, "198.51.100.61", 443),
            gbn_bridge_protocol::ReachabilityClass::Direct,
            1_000,
        )
        .unwrap();

    for index in 0..10 {
        let result = authority
            .enqueue_join_request_for_batch(
                creator_join_request(&format!("join-{index:03}"), "bridge-a", 80 + index as u8),
                5_000,
            )
            .unwrap();
        assert!(result.is_none());
    }

    let finalized = authority
        .enqueue_join_request_for_batch(creator_join_request("join-010", "bridge-a", 99), 5_000)
        .unwrap()
        .expect("11th request should flush the first batch");
    assert_eq!(finalized.assignments.len(), 10);
    assert_eq!(finalized.bridge_assignments.len(), 2);

    let rollover = authority
        .flush_ready_batch(5_500)
        .unwrap()
        .expect("rollover batch should flush after the batch window");
    assert_eq!(rollover.assignments.len(), 1);
    assert_eq!(rollover.bridge_assignments.len(), 2);
}
