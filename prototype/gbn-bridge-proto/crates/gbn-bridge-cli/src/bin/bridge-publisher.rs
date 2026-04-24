use ed25519_dalek::SigningKey;
use gbn_bridge_publisher::{AuthorityServer, PublisherAuthority, PublisherServiceConfig};

fn main() {
    if let Err(error) = run() {
        eprintln!("bridge-publisher startup error: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let config = PublisherServiceConfig::from_env()?;
    let signing_key = publisher_signing_key_from_env()?;
    let authority = PublisherAuthority::new(signing_key);
    let server = AuthorityServer::new(authority, config);
    let bound = server.bind().map_err(|error| error.to_string())?;
    println!(
        "bridge-publisher authority API listening on {}",
        bound.local_addr()
    );
    bound.serve_forever().map_err(|error| error.to_string())
}

fn publisher_signing_key_from_env() -> Result<SigningKey, String> {
    match std::env::var("GBN_BRIDGE_PUBLISHER_SIGNING_KEY_HEX") {
        Ok(value) => {
            let bytes = decode_hex_32(&value)?;
            Ok(SigningKey::from_bytes(&bytes))
        }
        Err(_) => {
            eprintln!(
                "GBN_BRIDGE_PUBLISHER_SIGNING_KEY_HEX is not set; using the default development publisher key"
            );
            Ok(SigningKey::from_bytes(&[9_u8; 32]))
        }
    }
}

fn decode_hex_32(value: &str) -> Result<[u8; 32], String> {
    let trimmed = value.trim();
    if trimmed.len() != 64 {
        return Err(format!(
            "GBN_BRIDGE_PUBLISHER_SIGNING_KEY_HEX must contain 64 hex characters, got {}",
            trimmed.len()
        ));
    }

    let mut bytes = [0_u8; 32];
    for (index, chunk) in trimmed.as_bytes().chunks(2).enumerate() {
        let pair = std::str::from_utf8(chunk)
            .map_err(|_| "publisher signing key hex must be valid utf-8".to_string())?;
        bytes[index] = u8::from_str_radix(pair, 16)
            .map_err(|_| format!("invalid publisher signing key hex byte {pair:?}"))?;
    }

    Ok(bytes)
}
