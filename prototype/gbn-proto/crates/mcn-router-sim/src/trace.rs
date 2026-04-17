use std::sync::OnceLock;

#[cfg(feature = "distributed-trace")]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "distributed-trace")]
static TRACE_COUNTER: AtomicU64 = AtomicU64::new(1);

static NODE_ID: OnceLock<String> = OnceLock::new();

pub fn node_id() -> String {
    NODE_ID
        .get_or_init(|| {
            let inst = std::env::var("GBN_INSTANCE_IPV4")
                .or_else(|_| std::env::var("HOSTNAME"))
                .unwrap_or_else(|_| "unknown-node".to_string());
            format!("node@{}", inst)
        })
        .clone()
}

#[cfg(feature = "distributed-trace")]
pub fn next_hop_id() -> String {
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let ctr = TRACE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}:{}:{}", node_id(), ts_ms, ctr)
}

#[cfg(not(feature = "distributed-trace"))]
pub fn next_hop_id() -> String {
    String::new()
}

pub fn chain_to_string(chain: &[String]) -> String {
    chain.join(" -> ")
}
