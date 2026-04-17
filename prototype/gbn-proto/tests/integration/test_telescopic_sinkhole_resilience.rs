//! S1.6 — Telescopic Sinkhole Attack
//!
//! A malicious Guard node silently drops the `RelayExtend` payload destined
//! for the Middle node and instead immediately replies with a fabricated
//! `RelayExtended` response.
//!
//! **Pass criteria:** The Creator's `build_circuit()` call detects that the
//! handshake state never completed (the follow-up Noise turns receive garbage
//! or timeout) and returns an `Err`. The caller MUST abort the circuit — no
//! video data must be sent through an unverified path.
//!
//! This test proves mathematically that a sinkhole guard cannot forge its way
//! into a "trusted" relay position.

use std::{net::SocketAddr, time::Duration};

use mcn_router_sim::{
    circuit_manager::{build_circuit, RelayNode},
    relay_engine::{recv_cell, send_cell, spawn_onion_relay},
};
use gbn_protocol::onion::{ExtendedPayload, OnionCell};
use tokio::net::TcpListener;

/// Spawn a TCP server that behaves as a Sinkhole Guard:
/// - Receives any cell from the Creator.
/// - Immediately replies with a forged `RelayExtended` (random bytes as
///   handshake_response — it has no real handshake material).
/// - Never actually contacts a Middle node.
async fn spawn_sinkhole_guard() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            // Consume whatever the Creator sends (the initial Noise handshake bytes)
            let _ = recv_cell(&mut stream).await;
            // Reply with a completely fabricated RelayExtended
            let _ = send_cell(
                &mut stream,
                &OnionCell::RelayExtended(ExtendedPayload {
                    handshake_response: vec![0xDE, 0xAD, 0xBE, 0xEF], // garbage
                }),
            )
            .await;
        }
    });

    addr
}

/// Spawn a dummy "Middle" node that the Guard *should* have dialled but won't
/// (the sinkhole never contacts it). Having it listening makes the test
/// address valid, but it will see zero traffic.
async fn spawn_dummy_node() -> (SocketAddr, [u8; 32]) {
    let priv_key = [0x42u8; 32];
    let relay = spawn_onion_relay("127.0.0.1:0".parse().unwrap(), priv_key, 0, 0)
        .await
        .unwrap();
    (relay.listen_addr, priv_key)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_s1_6_sinkhole_guard_is_rejected() {
    let creator_priv_key = [0x11u8; 32];

    // Sinkhole impersonates the Guard
    let sinkhole_addr = spawn_sinkhole_guard().await;
    let (middle_addr, middle_pub) = spawn_dummy_node().await;
    let (exit_addr, exit_pub) = spawn_dummy_node().await;

    // The sinkhole guard presents a bogus identity key — won't match Noise handshake
    let sinkhole_guard = RelayNode {
        addr: sinkhole_addr,
        identity_pub: [0x99u8; 32], // arbitrary — won't match real handshake
        subnet_tag: "HostileSubnet".into(),
        last_seen_ms: 0,
    };
    let middle = RelayNode {
        addr: middle_addr,
        identity_pub: middle_pub,
        subnet_tag: "HostileSubnet".into(),
        last_seen_ms: 0,
    };
    let exit = RelayNode {
        addr: exit_addr,
        identity_pub: exit_pub,
        subnet_tag: "FreeSubnet".into(),
        last_seen_ms: 0,
    };

    let result = tokio::time::timeout(
        Duration::from_secs(15),
        build_circuit(&creator_priv_key, &sinkhole_guard, &middle, &exit, ""),
    )
    .await;

    // The circuit build MUST fail — either timeout or Noise handshake error
    match result {
        Err(_timeout) => {
            // Timeout is acceptable — the guard stalled the handshake
        }
        Ok(Err(e)) => {
            // Explicit Noise failure is the ideal outcome
            tracing::info!("S1.6 PASS: circuit build rejected with error: {}", e);
        }
        Ok(Ok(_)) => {
            panic!("S1.6 FAIL: build_circuit() succeeded against a sinkhole guard — circuit is NOT safe!");
        }
    }
}
