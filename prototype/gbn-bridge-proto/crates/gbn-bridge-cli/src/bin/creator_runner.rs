use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use ed25519_dalek::SigningKey;
use gbn_bridge_creator::LocalDhtStore;
use gbn_bridge_protocol::{publisher_identity, PublicKeyBytes};
use gbn_bridge_publisher::{
    admin::{admin_bind_addr_from_env, AdminHttpServer, AdminNodeMetadata, AdminState},
    metrics_otlp,
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
    let _otlp_guard = metrics_otlp::init_otlp_tracing_from_env("creator-runner")?;
    let admin_addr = admin_bind_addr_from_env()?;
    let node_id = env::var("GBN_BRIDGE_NODE_ID").unwrap_or_else(|_| "creator".to_string());
    let conduit_actor = env::var("GBN_CONDUIT_ACTOR")
        .ok()
        .or_else(|| env::var("GBN_NODE_ACTOR").ok())
        .filter(|value| !value.trim().is_empty());
    let actor_id = conduit_actor.clone().unwrap_or_else(|| node_id.clone());
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
    let admin_server = AdminHttpServer::bind(
        admin_addr,
        AdminState::creator(metadata, local_dht.clone()),
        1_048_576,
    )
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
        "creator-runner admin listening on {}; state_path={}",
        socket_addr_display(admin_addr),
        state_path.display()
    );

    admin_server
        .serve_forever()
        .map_err(|error| error.to_string())
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
