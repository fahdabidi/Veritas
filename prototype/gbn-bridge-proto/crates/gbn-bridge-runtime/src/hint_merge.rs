use std::collections::BTreeSet;

use gbn_bridge_protocol::{PublicKeyBytes, ReachabilityClass};

use crate::catalog_cache::CatalogCache;
use crate::discovery::{DiscoveryHintSource, WeakDiscoveryState};
use crate::local_dht::{LocalDht, LocalHintSource};
use crate::{RuntimeError, RuntimeResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshCandidateAuthority {
    Signed,
    Weak,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshCandidateSource {
    Bootstrap,
    Catalog,
    WeakDiscovery(DiscoveryHintSource),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshCandidate {
    pub bridge_id: String,
    pub host: String,
    pub port: u16,
    pub authority: RefreshCandidateAuthority,
    pub source: RefreshCandidateSource,
    pub expires_at_ms: Option<u64>,
    pub observed_at_ms: u64,
}

impl RefreshCandidate {
    pub fn transport_eligible(&self) -> bool {
        matches!(self.authority, RefreshCandidateAuthority::Signed)
    }
}

pub fn merge_refresh_candidates(
    local_dht: &LocalDht,
    catalog_cache: &CatalogCache,
    discovery: &WeakDiscoveryState,
    publisher_key: &PublicKeyBytes,
    now_ms: u64,
    excluded_bridge_ids: &BTreeSet<String>,
) -> RuntimeResult<Vec<RefreshCandidate>> {
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();

    let mut bootstrap_candidates: Vec<_> = local_dht
        .snapshot()
        .into_iter()
        .filter(|node| matches!(node.source, LocalHintSource::Bootstrap))
        .filter(|node| node.expires_at_ms >= now_ms)
        .filter(|node| !excluded_bridge_ids.contains(&node.node_id))
        .collect();

    bootstrap_candidates.sort_by(|left, right| {
        right
            .active_tunnel_since_ms
            .cmp(&left.active_tunnel_since_ms)
            .then_with(|| right.last_updated_ms.cmp(&left.last_updated_ms))
            .then_with(|| left.node_id.cmp(&right.node_id))
    });

    for node in bootstrap_candidates {
        if seen.insert(node.node_id.clone()) {
            candidates.push(RefreshCandidate {
                bridge_id: node.node_id,
                host: node.ip_addr,
                port: node.udp_punch_port,
                authority: RefreshCandidateAuthority::Signed,
                source: RefreshCandidateSource::Bootstrap,
                expires_at_ms: Some(node.expires_at_ms),
                observed_at_ms: node.last_updated_ms,
            });
        }
    }

    if let Some(catalog) = catalog_cache.current() {
        if catalog.verify_authority(publisher_key, now_ms).is_ok() {
            let mut catalog_candidates: Vec<_> = catalog
                .bridges
                .iter()
                .filter(|bridge| matches!(bridge.reachability_class, ReachabilityClass::Direct))
                .filter(|bridge| !excluded_bridge_ids.contains(&bridge.bridge_id))
                .cloned()
                .collect();

            catalog_candidates.sort_by(|left, right| {
                right
                    .lease_expiry_ms
                    .cmp(&left.lease_expiry_ms)
                    .then_with(|| left.bridge_id.cmp(&right.bridge_id))
            });

            for bridge in catalog_candidates {
                if seen.insert(bridge.bridge_id.clone()) {
                    if let Some(endpoint) = bridge.ingress_endpoints.first() {
                        candidates.push(RefreshCandidate {
                            bridge_id: bridge.bridge_id,
                            host: endpoint.host.clone(),
                            port: bridge.udp_punch_port,
                            authority: RefreshCandidateAuthority::Signed,
                            source: RefreshCandidateSource::Catalog,
                            expires_at_ms: Some(bridge.lease_expiry_ms),
                            observed_at_ms: catalog.issued_at_ms,
                        });
                    }
                }
            }
        }
    }

    for hint in discovery.snapshot_fresh(now_ms) {
        if excluded_bridge_ids.contains(&hint.bridge_id) {
            continue;
        }

        if seen.insert(hint.bridge_id.clone()) {
            candidates.push(RefreshCandidate {
                bridge_id: hint.bridge_id,
                host: hint.host,
                port: hint.port,
                authority: RefreshCandidateAuthority::Weak,
                source: RefreshCandidateSource::WeakDiscovery(hint.source),
                expires_at_ms: None,
                observed_at_ms: hint.observed_at_ms,
            });
        }
    }

    if candidates.is_empty() {
        return Err(RuntimeError::NoUsableBridgeCandidate);
    }

    Ok(candidates)
}
