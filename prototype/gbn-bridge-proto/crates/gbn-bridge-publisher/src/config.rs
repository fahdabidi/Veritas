use std::{env, net::SocketAddr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublisherServiceConfig {
    pub bind_addr: String,
    pub auth_max_skew_ms: u64,
    pub replay_ttl_ms: u64,
    pub request_max_bytes: usize,
}

impl Default for PublisherServiceConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:8080".into(),
            auth_max_skew_ms: 30_000,
            replay_ttl_ms: 300_000,
            request_max_bytes: 1_048_576,
        }
    }
}

impl PublisherServiceConfig {
    pub fn from_env() -> Result<Self, String> {
        Ok(Self {
            bind_addr: env_or("GBN_BRIDGE_PUBLISHER_BIND_ADDR", "127.0.0.1:8080"),
            auth_max_skew_ms: parse_env_u64("GBN_BRIDGE_AUTH_MAX_SKEW_MS", 30_000)?,
            replay_ttl_ms: parse_env_u64("GBN_BRIDGE_REPLAY_TTL_MS", 300_000)?,
            request_max_bytes: parse_env_usize("GBN_BRIDGE_REQUEST_MAX_BYTES", 1_048_576)?,
        })
    }

    pub fn socket_addr(&self) -> Result<SocketAddr, String> {
        self.bind_addr.parse::<SocketAddr>().map_err(|_| {
            format!(
                "GBN_BRIDGE_PUBLISHER_BIND_ADDR must be a valid socket address, got {:?}",
                self.bind_addr
            )
        })
    }
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn parse_env_u64(key: &str, default: u64) -> Result<u64, String> {
    match env::var(key) {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|_| format!("{key} must be a valid u64, got {value:?}")),
        Err(_) => Ok(default),
    }
}

fn parse_env_usize(key: &str, default: usize) -> Result<usize, String> {
    match env::var(key) {
        Ok(value) => value
            .parse::<usize>()
            .map_err(|_| format!("{key} must be a valid usize, got {value:?}")),
        Err(_) => Ok(default),
    }
}
