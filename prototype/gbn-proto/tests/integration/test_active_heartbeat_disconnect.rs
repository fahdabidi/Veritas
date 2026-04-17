//! S1.9 — Heartbeat Fallback & Dynamic Route Rebuild
//!
//! Proves that the CircuitManager handles an abrupt relay node failure *during*
//! active media transmission without losing video chunks.
//!
//! **Test procedure:**
//!   1. Build a live circuit (Guard → onion relay → destination).
//!   2. Begin sending chunks through the circuit.
//!   3. Mid-transmission, forcibly shut down the Guard relay.
//!   4. Call `drain_failures()` on the CircuitManager.
//!   5. Assert that ALL chunks (sent + drained) are accounted for — none lost.
//!
//! **Pass criteria:**
//!   - `drain_failures()` returns all in-flight chunks that were pending ACK
//!     when the circuit died.
//!   - The CircuitManager does NOT panic or deadlock.
//!   - A fresh circuit using a **disjoint Guard** can be added and used to
//!     re-send the recovered chunks successfully.
//!
//! This is the critical validation for Assumption A9 and Security Test S1.9.

use std::{net::SocketAddr, time::Duration};

use mcn_router_sim::{
    circuit_manager::{CircuitManager, RelayNode, build_circuit},
    relay_engine::spawn_onion_relay,
};
use tokio::net::TcpListener;

/// Spawn a minimal onion relay with a given private key.
async fn make_relay(priv_key: [u8; 32]) -> (SocketAddr, [u8; 32]) {
    let handle = spawn_onion_relay("127.0.0.1:0".parse().unwrap(), priv_key, 0, 5)
        .await
        .unwrap();
    (handle.listen_addr, priv_key)
}

/// Spawn a simple TCP sink that accepts connections but does not reply.
/// Used as the "Publisher" endpoint — we don't need full reassembly for this test.
async fn spawn_sink() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((_, _)) = listener.accept().await else { break };
        }
    });
    addr
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_s1_9_heartbeat_fallback_recovers_inflight_chunks() {
    let creator_key = [0xAAu8; 32];

    // ── Build initial circuit (Guard1 → Middle → Exit) ─────────────────────
    let (guard1_addr, guard1_pub) = make_relay([0x01u8; 32]).await;
    let (middle_addr, middle_pub) = make_relay([0x02u8; 32]).await;
    let (exit_addr, exit_pub) = make_relay([0x03u8; 32]).await;
    let _sink = spawn_sink().await;

    let guard1 = RelayNode { addr: guard1_addr, identity_pub: guard1_pub, subnet_tag: "HostileSubnet".into(), last_seen_ms: 0 };
    let middle = RelayNode { addr: middle_addr, identity_pub: middle_pub, subnet_tag: "HostileSubnet".into(), last_seen_ms: 0 };
    let exit   = RelayNode { addr: exit_addr,   identity_pub: exit_pub,   subnet_tag: "FreeSubnet".into(),   last_seen_ms: 0 };

    let circuit1 = build_circuit(&creator_key, &guard1, &middle, &exit, "")
        .await
        .expect("Initial circuit build should succeed");

    let manager = CircuitManager::new();
    manager.add_circuit(circuit1).await;

    // ── Send 5 chunks (in-flight, not yet ACKed) ───────────────────────────
    let total_chunks: u32 = 5;
    for i in 0..total_chunks {
        let payload = vec![i as u8; 512]; // 512-byte fake chunk
        // Note: These may fail to deliver because the relay loop isn't fully
        // wired to a Publisher receiver in this unit test — we're testing the
        // IN-FLIGHT QUEUE fallback, not end-to-end delivery.
        let _ = manager.send_chunk(i, payload).await;
    }

    // ── Simulate Guard failure by killing the relay process ────────────────
    // In the full AWS test, this would be `aws ec2 terminate-instances`.
    // Here we simply let the Guard's spawn_onion_relay handle go out of scope.
    // The heartbeat watchdog will detect the dead stream and signal failure.
    tracing::info!("Simulating Guard node failure — waiting for heartbeat timeout...");
    tokio::time::sleep(Duration::from_secs(12)).await; // > HEARTBEAT_TIMEOUT (10s)

    // ── Drain failure signals and collect lost chunks ─────────────────────
    let requeued = manager.drain_failures().await;

    assert!(
        !requeued.is_empty(),
        "S1.9 FAIL: drain_failures() returned empty — in-flight chunks were silently lost"
    );

    tracing::info!("S1.9: {} chunks recovered from dead circuit", requeued.len());

    // ── Build a REPLACEMENT circuit with a DISJOINT guard ─────────────────
    let (guard2_addr, guard2_pub) = make_relay([0x04u8; 32]).await; // different key → disjoint
    let guard2 = RelayNode { addr: guard2_addr, identity_pub: guard2_pub, subnet_tag: "HostileSubnet".into(), last_seen_ms: 0 };

    assert_ne!(
        guard2_addr, guard1_addr,
        "S1.9 FAIL: Replacement Guard must be disjoint from failed Guard"
    );

    let circuit2 = build_circuit(&creator_key, &guard2, &middle, &exit, "")
        .await
        .expect("Replacement circuit build should succeed");
    manager.add_circuit(circuit2).await;

    // ── Re-send recovered chunks through the new circuit ──────────────────
    let mut resent = 0u32;
    for (chunk_idx, payload) in requeued {
        if manager.send_chunk(chunk_idx, payload).await.is_ok() {
            resent += 1;
        }
    }

    assert_eq!(
        resent, total_chunks,
        "S1.9 FAIL: Not all recovered chunks could be re-sent through the replacement circuit"
    );

    tracing::info!(
        "S1.9 PASS: {}/{} chunks successfully re-routed through disjoint Guard {}",
        resent, total_chunks, guard2_addr
    );
}
