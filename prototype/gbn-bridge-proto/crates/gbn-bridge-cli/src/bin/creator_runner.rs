use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ed25519_dalek::SigningKey;
use gbn_bridge_creator::LocalDhtStore;
use gbn_bridge_protocol::{publisher_identity, PublicKeyBytes};
use gbn_bridge_publisher::{
    admin::{
        admin_bind_addr_from_env, creator_trace_service_name, AdminCreatorConfig, AdminHttpServer,
        AdminNodeMetadata, AdminState,
    },
    metrics_http::MetricsHttpServer,
    metrics_otlp,
    metrics_prometheus::{creator_metrics_text, stack_from_env},
};
use sha2::{Digest, Sha256};

const DEFAULT_STATE_DIR: &str = "/var/lib/gbn-conduit";
const LOCAL_DHT_FILE: &str = "local_dht.json";
const CREATOR_IDENTITY_KEY_FILE: &str = "creator_identity_key.hex";
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
        eprintln!("creator-runner startup error: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let admin_addr = admin_bind_addr_from_env()?;
    let node_id = env::var("GBN_BRIDGE_NODE_ID").unwrap_or_else(|_| "creator".to_string());
    let conduit_actor = env::var("GBN_CONDUIT_ACTOR")
        .ok()
        .or_else(|| env::var("GBN_NODE_ACTOR").ok())
        .filter(|value| !value.trim().is_empty());
    let actor_id = conduit_actor.clone().unwrap_or_else(|| node_id.clone());
    let otlp_service_name = env::var("GBN_BRIDGE_OTLP_SERVICE_NAME")
        .unwrap_or_else(|_| creator_trace_service_name(&actor_id, &node_id));
    let _otlp_guard = metrics_otlp::init_otlp_tracing_from_env(&otlp_service_name)?;
    let state_dir = PathBuf::from(
        env::var("GBN_BRIDGE_STATE_DIR").unwrap_or_else(|_| DEFAULT_STATE_DIR.to_string()),
    );
    let state_path = state_dir.join(LOCAL_DHT_FILE);
    let publisher_public_key = load_publisher_public_key()?;
    let creator_signing_key = load_or_create_creator_signing_key(&state_dir, &actor_id)?;
    let creator_public_key =
        PublicKeyBytes::from_verifying_key(&creator_signing_key.verifying_key());
    let now_ms = now_ms();
    let local_dht = LocalDhtStore::load_or_create(
        actor_id.clone(),
        state_path.clone(),
        Some(&publisher_public_key),
        now_ms,
    )
    .map_err(|error| format!("failed to initialize local DHT state: {error}"))?;
    let metadata = AdminNodeMetadata::from_env(node_id, "creator")
        .with_public_key(&creator_public_key)
        .with_publisher_public_key(&publisher_public_key)
        .with_creator_transport(
            env::var("GBN_BRIDGE_INGRESS_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            env::var("GBN_BRIDGE_PUNCH_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(443),
        );
    let creator_config = AdminCreatorConfig {
        actor_id: actor_id.clone(),
        signing_key: creator_signing_key,
        publisher_pub: publisher_public_key,
        authority_url: env::var("GBN_BRIDGE_AUTHORITY_URL")
            .or_else(|_| env::var("GBN_BRIDGE_PUBLISHER_URL"))
            .unwrap_or_else(|_| "http://publisher-authority:8080".to_string()),
        creator_ip_addr: metadata
            .ip_addr
            .clone()
            .unwrap_or_else(|| "127.0.0.1".to_string()),
        udp_punch_port: metadata.creator_udp_punch_port.unwrap_or(443),
        timeout: Duration::from_secs(
            env::var("GBN_BRIDGE_ADMIN_BOOTSTRAP_TIMEOUT_SECS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(5),
        ),
    };
    let _bootstrap_hint_handle = maybe_spawn_bootstrap_hint_server(
        &actor_id,
        &creator_public_key,
        metadata
            .ip_addr
            .clone()
            .unwrap_or_else(|| "127.0.0.1".to_string()),
        metadata.creator_udp_punch_port.unwrap_or(443),
    )?;
    let admin_server = AdminHttpServer::bind(
        admin_addr,
        AdminState::creator_with_config(metadata, local_dht.clone(), creator_config),
        1_048_576,
    )
    .map_err(|error| error.to_string())?;
    let prometheus_stack = stack_from_env();
    let prometheus_actor_id = actor_id.clone();
    let prometheus_service_name = otlp_service_name.clone();
    let prometheus_addr: SocketAddr = env::var("GBN_BRIDGE_METRICS_BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:9100".to_string())
        .parse()
        .map_err(|_| "GBN_BRIDGE_METRICS_BIND_ADDR must be a valid socket address".to_string())?;
    let prometheus_server = MetricsHttpServer::bind(prometheus_addr, move || {
        creator_metrics_text(
            &prometheus_actor_id,
            &prometheus_service_name,
            &prometheus_stack,
        )
    })
    .map_err(|error| error.to_string())?;
    let prometheus_local_addr = prometheus_server
        .local_addr()
        .map_err(|error| error.to_string())?;
    let _prometheus_handle = prometheus_server
        .spawn()
        .map_err(|error| error.to_string())?;

    let (build_version, build_source, build_created, image) = conduit_build_metadata();
    println!(
        "creator-runner node_id={} conduit_actor={} onboarding_state={} build_version={} build_source={} build_created={} image={}",
        local_dht.snapshot().actor_id,
        conduit_actor.as_deref().unwrap_or("none"),
        serde_json::to_string(&local_dht.snapshot().self_onboarding_state)
            .unwrap_or_else(|_| "\"unknown\"".to_string()),
        build_version,
        build_source,
        build_created,
        image
    );
    println!(
        "creator-runner admin listening on {}; metrics listening on {}; state_path={}",
        socket_addr_display(admin_addr),
        socket_addr_display(prometheus_local_addr),
        state_path.display()
    );

    admin_server
        .serve_forever()
        .map_err(|error| error.to_string())
}

fn maybe_spawn_bootstrap_hint_server(
    actor_id: &str,
    public_key: &PublicKeyBytes,
    fallback_host: String,
    udp_punch_port: u16,
) -> Result<Option<JoinHandle<()>>, String> {
    let bind_addr = match env::var("GBN_BRIDGE_BOOTSTRAP_HINT_BIND_ADDR") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => return Ok(None),
    };
    let bind_addr: SocketAddr = bind_addr.parse().map_err(|_| {
        "GBN_BRIDGE_BOOTSTRAP_HINT_BIND_ADDR must be a valid socket address".to_string()
    })?;
    let public_host = env::var("GBN_BRIDGE_BOOTSTRAP_HINT_PUBLIC_HOST")
        .or_else(|_| env::var("GBN_BRIDGE_INGRESS_HOST"))
        .unwrap_or(fallback_host);
    let public_port = env::var("GBN_BRIDGE_BOOTSTRAP_HINT_PUBLIC_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(bind_addr.port());
    let run_id = env::var("GBN_PASS4_RUN_ID").unwrap_or_else(|_| "pass4-aws-public".to_string());
    let actor_id = actor_id.to_string();
    let public_key_bytes = public_key.0.clone();

    let listener = TcpListener::bind(bind_addr)
        .map_err(|error| format!("failed to bind bootstrap hint server: {error}"))?;
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("failed to read bootstrap hint bind addr: {error}"))?;
    println!("creator-runner bootstrap hint listening on {local_addr}");

    Ok(Some(thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let actor_id = actor_id.clone();
                    let public_key_bytes = public_key_bytes.clone();
                    let public_host = public_host.clone();
                    let run_id = run_id.clone();
                    thread::spawn(move || {
                        if let Err(error) = handle_bootstrap_hint_connection(
                            stream,
                            &actor_id,
                            &public_key_bytes,
                            &public_host,
                            public_port,
                            udp_punch_port,
                            &run_id,
                        ) {
                            eprintln!("bootstrap hint connection error: {error}");
                        }
                    });
                }
                Err(error) => eprintln!("bootstrap hint listener error: {error}"),
            }
        }
    })))
}

fn handle_bootstrap_hint_connection(
    mut stream: TcpStream,
    actor_id: &str,
    public_key: &[u8],
    public_host: &str,
    public_port: u16,
    udp_punch_port: u16,
    run_id: &str,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let request_line = read_http_request_line(&mut stream)?;
    let response = if request_line.starts_with("GET /healthz ") {
        http_response(200, "OK", "text/plain; charset=utf-8", "ok\n")
    } else if request_line.starts_with("GET /v1/mobile/bootstrap-dht-qr ") {
        let now = now_ms();
        let expires = now.saturating_add(3_600_000);
        let publisher_sig = vec![1_u8; 64];
        let body = serde_json::json!({
            "schema_version": 1,
            "chain_id": format!("{run_id}-host-seed"),
            "run_id": run_id,
            "host_creator_id": actor_id,
            "host_creator_public_key_hex": bytes_to_hex(public_key),
            "host_creator_entry": {
                "node_id": actor_id,
                "ip_addr": public_host,
                "pub_key": public_key,
                "udp_punch_port": udp_punch_port,
                "entry_expiry_ms": expires,
                "publisher_sig": publisher_sig,
                "active": true
            },
            "host_creator_reachability": {
                "reachability_class": "direct",
                "capabilities": ["bootstrap_seed"]
            },
            "host_creator_bootstrap_endpoints": [{
                "protocol": "http",
                "host": public_host,
                "port": public_port
            }],
            "issued_at_ms": now,
            "expires_at_ms": expires,
            "payload_hash": format!("sha256:{}", bytes_to_hex(&Sha256::digest(format!("{run_id}:{actor_id}:{now}").as_bytes()))),
            "signature": "bootstrap-hint-endpoint"
        })
        .to_string();
        http_response(200, "OK", "application/json", body)
    } else {
        http_response(404, "Not Found", "text/plain; charset=utf-8", "not found\n")
    };
    stream.write_all(&response)
}

fn read_http_request_line(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 256];
    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(2).any(|window| window == b"\n") || buffer.len() > 8192 {
            break;
        }
    }
    let request = std::str::from_utf8(&buffer).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "request must be utf-8")
    })?;
    Ok(request.lines().next().unwrap_or_default().to_string())
}

fn http_response(
    status_code: u16,
    status_text: &str,
    content_type: &str,
    body: impl AsRef<str>,
) -> Vec<u8> {
    let body = body.as_ref().as_bytes();
    let headers = format!(
        "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut response = headers.into_bytes();
    response.extend_from_slice(body);
    response
}

fn socket_addr_display(addr: SocketAddr) -> String {
    addr.to_string()
}

fn load_publisher_public_key() -> Result<PublicKeyBytes, String> {
    if let Ok(path) = env::var("GBN_PUBLISHER_PUB_KEY_PATH") {
        let raw = fs::read_to_string(path)
            .map_err(|error| format!("failed to read GBN_PUBLISHER_PUB_KEY_PATH: {error}"))?;
        return decode_hex_32(&raw).map(|bytes| PublicKeyBytes(bytes.to_vec()));
    }
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

fn load_or_create_creator_signing_key(
    state_dir: &Path,
    actor_id: &str,
) -> Result<SigningKey, String> {
    if let Ok(value) = env::var("GBN_BRIDGE_CREATOR_SIGNING_KEY_HEX") {
        return decode_hex_32(&value).map(|bytes| SigningKey::from_bytes(&bytes));
    }

    let key_path = state_dir.join(CREATOR_IDENTITY_KEY_FILE);
    if key_path.exists() {
        let raw = fs::read_to_string(&key_path)
            .map_err(|error| format!("failed to read creator identity key: {error}"))?;
        return decode_hex_32(&raw).map(|bytes| SigningKey::from_bytes(&bytes));
    }

    fs::create_dir_all(state_dir)
        .map_err(|error| format!("failed to create creator state dir: {error}"))?;
    let mut hasher = Sha256::new();
    hasher.update(actor_id.as_bytes());
    hasher.update(now_ms().to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 32];
    bytes.copy_from_slice(&digest[..32]);
    fs::write(&key_path, bytes_to_hex(&bytes))
        .map_err(|error| format!("failed to persist creator identity key: {error}"))?;
    Ok(SigningKey::from_bytes(&bytes))
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

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_millis() as u64
}

fn conduit_build_metadata() -> (String, String, String, String) {
    (
        env::var("VERITAS_CONDUIT_BUILD_VERSION").unwrap_or_else(|_| "unknown".to_string()),
        env::var("VERITAS_CONDUIT_BUILD_SOURCE").unwrap_or_else(|_| "unknown".to_string()),
        env::var("VERITAS_CONDUIT_BUILD_CREATED").unwrap_or_else(|_| "unknown".to_string()),
        env::var("VERITAS_CONDUIT_IMAGE").unwrap_or_else(|_| "unknown".to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn creates_empty_local_dht_file_when_missing() {
        let dir = unique_test_dir("creator-missing");
        let path = dir.join(LOCAL_DHT_FILE);
        let publisher = load_publisher_public_key().expect("default publisher key should load");

        let store = LocalDhtStore::load_or_create("host-creator", &path, Some(&publisher), 1_000)
            .expect("missing state file should be created");
        let table = store.snapshot();

        assert_eq!(table.actor_id, "host-creator");
        assert_eq!(table.role, "creator");
        assert_eq!(
            table.self_onboarding_state,
            gbn_bridge_protocol::SelfOnboardingState::None
        );
        assert!(path.exists());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn reloads_existing_local_dht_file() {
        let dir = unique_test_dir("creator-existing");
        let path = dir.join(LOCAL_DHT_FILE);
        let publisher = load_publisher_public_key().expect("default publisher key should load");
        let original = gbn_bridge_protocol::LocalDiscoveryTable::empty("new-creator", 2_000);
        gbn_bridge_creator::local_dht::persist_table(&path, &original)
            .expect("state file should be written");

        let store = LocalDhtStore::load_or_create("new-creator", &path, Some(&publisher), 3_000)
            .expect("existing state file should load");
        let table = store.snapshot();

        assert_eq!(table, original);

        let _ = fs::remove_dir_all(dir);
    }

    fn unique_test_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        env::temp_dir().join(format!("veritas-{name}-{}-{nanos}", std::process::id()))
    }
}
