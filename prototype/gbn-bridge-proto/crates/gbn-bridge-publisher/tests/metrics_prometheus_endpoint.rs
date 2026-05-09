use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ed25519_dalek::SigningKey;
use gbn_bridge_publisher::metrics_http::MetricsHttpServer;
use gbn_bridge_publisher::metrics_prometheus::{
    bridge_metrics_text, receiver_metrics_text, PROMETHEUS_CONTENT_TYPE,
};
use gbn_bridge_publisher::{
    AuthorityServer, BridgeMetrics, BridgeMetricsSnapshot, PublisherAuthority,
    PublisherServiceConfig, ReceiverMetrics, ReceiverProxyConfig, ReceiverProxyServer,
};

#[test]
fn authority_public_metrics_endpoint_returns_prometheus_exposition() {
    let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
    let config = PublisherServiceConfig {
        bind_addr: "127.0.0.1:0".to_string(),
        ..PublisherServiceConfig::default()
    };
    let server = AuthorityServer::new(PublisherAuthority::new(signing_key), config);
    let bound = server.bind().unwrap();
    let addr = bound.local_addr();
    let handle = bound.spawn().unwrap();

    let response = http_get(addr, "/metrics");
    handle.join().unwrap();

    assert!(response.starts_with("HTTP/1.1 200 OK"));
    assert!(response.contains(PROMETHEUS_CONTENT_TYPE));
    assert!(response.contains("# TYPE conduit_authority_successful_registrations_total counter"));
    assert!(response.contains(
        "conduit_authority_successful_registrations_total{service=\"authority\",stack=\""
    ));
}

#[test]
fn receiver_public_metrics_endpoint_returns_current_snapshot() {
    let metrics = Arc::new(Mutex::new(ReceiverMetrics::default()));
    metrics.lock().unwrap().record_frame_accepted(128);
    let config = ReceiverProxyConfig {
        bind_addr: "127.0.0.1:0".to_string(),
        authority_url: "http://127.0.0.1:1".to_string(),
        request_max_bytes: 1_048_576,
        ..ReceiverProxyConfig::default()
    };
    let server = ReceiverProxyServer::bind_with_metrics(config, metrics).unwrap();
    let addr = server.local_addr().unwrap();
    let handle = server.spawn().unwrap();

    let response = http_get(addr, "/metrics");
    handle.join().unwrap();

    assert!(response.starts_with("HTTP/1.1 200 OK"));
    assert!(response.contains(PROMETHEUS_CONTENT_TYPE));
    assert!(
        response.contains("conduit_receiver_frames_accepted_total{service=\"receiver\",stack=\"")
    );
    assert!(response.contains("} 1"));
    assert!(response.contains("conduit_receiver_bytes_ingested_total"));
    assert!(response.contains("} 128"));
}

#[test]
fn bridge_metrics_listener_returns_prometheus_exposition() {
    let snapshot = BridgeMetricsSnapshot {
        frames_forwarded: 7,
        bytes_forwarded: 4096,
        ..BridgeMetricsSnapshot::default()
    };
    let server = MetricsHttpServer::bind("127.0.0.1:0".parse().unwrap(), move || {
        bridge_metrics_text(&snapshot, "bridge", "dev-local")
    })
    .unwrap();
    let addr = server.local_addr().unwrap();
    let handle = server.spawn().unwrap();

    let response = http_get(addr, "/metrics");
    handle.join().unwrap();

    assert!(response.starts_with("HTTP/1.1 200 OK"));
    assert!(response.contains(PROMETHEUS_CONTENT_TYPE));
    assert!(response.contains(
        "conduit_bridge_frames_forwarded_total{service=\"bridge\",stack=\"dev-local\"} 7"
    ));
    assert!(response.contains(
        "conduit_bridge_bytes_forwarded_total{service=\"bridge\",stack=\"dev-local\"} 4096"
    ));
}

#[test]
fn prometheus_renderers_include_service_and_stack_labels() {
    let mut metrics = BridgeMetrics::default();
    metrics.record_control_reconnect();
    let bridge = bridge_metrics_text(&metrics.snapshot(), "bridge", "dev-local");
    let receiver = receiver_metrics_text(
        &gbn_bridge_publisher::ReceiverMetricsSnapshot {
            sessions_opened: 2,
            ..gbn_bridge_publisher::ReceiverMetricsSnapshot::default()
        },
        "receiver",
        "dev-local",
    );

    assert!(bridge.contains("{service=\"bridge\",stack=\"dev-local\"} 1"));
    assert!(receiver.contains("{service=\"receiver\",stack=\"dev-local\"} 2"));
}

fn http_get(addr: SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}
