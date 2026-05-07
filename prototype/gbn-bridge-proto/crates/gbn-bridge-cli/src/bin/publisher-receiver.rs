use std::env;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{publisher_identity, PublicKeyBytes, DEFAULT_UDP_PUNCH_PORT};
use gbn_bridge_publisher::{
    admin::{AdminCreatorConfig, AdminHttpServer, AdminState, DEFAULT_ADMIN_BIND_ADDR},
    metrics_emitter::{cloudwatch_metrics_enabled, spawn_cloudwatch_emitter, MetricsEmitterConfig},
    ReceiverMetrics, ReceiverProxyConfig, ReceiverProxyServer,
};
use sha2::{Digest, Sha256};

const DEFAULT_PUBLISHER_SIGNING_KEY_HEX: &str = "09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09";

fn main() {
    if let Err(error) = run() {
        eprintln!("publisher-receiver startup error: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let config = ReceiverProxyConfig::from_env()?;
    let request_max_bytes = config.request_max_bytes;
    let metrics = Arc::new(Mutex::new(ReceiverMetrics::default()));
    let admin_creator = AdminCreatorConfig {
        actor_id: env::var("GBN_BRIDGE_ADMIN_CREATOR_ID")
            .unwrap_or_else(|_| "publisher-receiver".to_string()),
        signing_key: load_creator_signing_key("publisher-receiver")?,
        publisher_pub: load_publisher_public_key()?,
        authority_url: config.authority_url.clone(),
        creator_ip_addr: "127.0.0.1".to_string(),
        udp_punch_port: DEFAULT_UDP_PUNCH_PORT,
        timeout: Duration::from_secs(5),
    };
    let server = ReceiverProxyServer::bind_with_metrics(config.clone(), metrics.clone())
        .map_err(|error| error.to_string())?;
    let admin_addr: SocketAddr = DEFAULT_ADMIN_BIND_ADDR
        .parse()
        .expect("default admin bind address should be valid");
    let admin_server = AdminHttpServer::bind(
        admin_addr,
        AdminState::receiver_with_creator(metrics.clone(), admin_creator),
        request_max_bytes,
    )
    .map_err(|error| error.to_string())?;
    let _admin_handle = admin_server.spawn().map_err(|error| error.to_string())?;
    let _metrics_handle = if cloudwatch_metrics_enabled() {
        let metrics_for_emitter = metrics.clone();
        Some(spawn_cloudwatch_emitter(
            MetricsEmitterConfig::from_env("receiver"),
            move |service, stack| {
                metrics_for_emitter
                    .lock()
                    .expect("receiver metrics mutex poisoned while emitting metrics")
                    .snapshot()
                    .cloudwatch_data(service, stack)
            },
        ))
    } else {
        None
    };
    println!(
        "publisher-receiver proxy listening on {} and forwarding to {}; admin listening on {}",
        server.local_addr().map_err(|error| error.to_string())?,
        config.authority_url,
        DEFAULT_ADMIN_BIND_ADDR
    );
    server.serve_forever().map_err(|error| error.to_string())
}

fn load_creator_signing_key(default_actor_id: &str) -> Result<SigningKey, String> {
    if let Ok(value) = env::var("GBN_BRIDGE_CREATOR_SIGNING_KEY_HEX") {
        return decode_hex_32(&value).map(|bytes| SigningKey::from_bytes(&bytes));
    }

    let actor_id =
        env::var("GBN_BRIDGE_ADMIN_CREATOR_ID").unwrap_or_else(|_| default_actor_id.to_string());
    let mut hasher = Sha256::new();
    hasher.update(actor_id.as_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(now_ms().to_le_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 32];
    bytes.copy_from_slice(&digest[..32]);
    Ok(SigningKey::from_bytes(&bytes))
}

fn load_publisher_public_key() -> Result<PublicKeyBytes, String> {
    if let Ok(value) = env::var("GBN_BRIDGE_PUBLISHER_PUBLIC_KEY_HEX") {
        return decode_hex_32(&value).map(|bytes| PublicKeyBytes(bytes.to_vec()));
    }
    if let Ok(value) = env::var("GBN_BRIDGE_PUBLISHER_SIGNING_KEY_HEX") {
        let bytes = decode_hex_32(&value)?;
        return Ok(publisher_identity(&SigningKey::from_bytes(&bytes)));
    }
    let bytes = decode_hex_32(DEFAULT_PUBLISHER_SIGNING_KEY_HEX)?;
    Ok(publisher_identity(&SigningKey::from_bytes(&bytes)))
}

fn decode_hex_32(value: &str) -> Result<[u8; 32], String> {
    let trimmed = value.trim();
    if trimmed.len() != 64 {
        return Err(format!(
            "hex value must contain exactly 64 characters, got {}",
            trimmed.len()
        ));
    }

    let mut bytes = [0_u8; 32];
    for (index, chunk) in trimmed.as_bytes().chunks(2).enumerate() {
        let pair =
            std::str::from_utf8(chunk).map_err(|_| "hex value must be valid utf-8".to_string())?;
        bytes[index] =
            u8::from_str_radix(pair, 16).map_err(|_| format!("invalid hex byte {pair:?}"))?;
    }
    Ok(bytes)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis() as u64
}
