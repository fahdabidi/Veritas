use gbn_bridge_protocol::{
    BridgeDhtEntry, BridgeIngressEndpointKind, LocalDiscoveryTable, PublicKeyBytes,
    ReachabilityClass,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::BridgeFilterDrops;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanePlan {
    pub target_lane_count: u32,
    pub selected_bridges: Vec<BridgeDhtEntry>,
    pub overflow_pool: Vec<BridgeDhtEntry>,
    pub filter_drops: BridgeFilterDrops,
}

#[derive(Debug, Error)]
pub enum LanePlanError {
    #[error("target_lane_count must be greater than zero")]
    InvalidTargetLaneCount,

    #[error("no active publisher-signed direct/brokered bridge available in local DHT")]
    NoEligibleBridges { filter_drops: BridgeFilterDrops },
}

pub fn plan_lanes(
    local_dht: &LocalDiscoveryTable,
    publisher_pub: &PublicKeyBytes,
    target_lane_count: u32,
    now_ms: u64,
) -> Result<LanePlan, LanePlanError> {
    if target_lane_count == 0 {
        return Err(LanePlanError::InvalidTargetLaneCount);
    }
    let mut drops = BridgeFilterDrops::default();
    let mut candidates = Vec::new();

    for entry in &local_dht.bridge_entries {
        if !entry.active {
            drops.inactive += 1;
            continue;
        }
        if now_ms > entry.lease_expiry_ms {
            drops.expired_lease += 1;
            continue;
        }
        if now_ms > entry.entry_expiry_ms {
            drops.expired_entry += 1;
            continue;
        }
        if entry.verify_authority(publisher_pub, now_ms).is_err() {
            drops.bad_signature += 1;
            continue;
        }
        if matches!(entry.reachability_class, ReachabilityClass::RelayOnly) {
            drops.relay_only += 1;
            continue;
        }
        if entry
            .suspect_until_ms
            .is_some_and(|suspect| suspect > now_ms)
        {
            drops.suspect += 1;
            continue;
        }
        if !entry.ingress_endpoints.iter().any(|endpoint| {
            endpoint.port != 0
                && !endpoint.ip_addr.trim().is_empty()
                && matches!(
                    endpoint.kind,
                    BridgeIngressEndpointKind::Direct | BridgeIngressEndpointKind::Brokered
                )
        }) {
            drops.no_ingress += 1;
            continue;
        }
        let last_seen_ms = local_dht
            .active_tunnels
            .iter()
            .filter(|tunnel| tunnel.peer_id == entry.bridge_id)
            .map(|tunnel| tunnel.last_seen_ms)
            .max()
            .unwrap_or(0);
        candidates.push((entry.clone(), last_seen_ms));
    }

    candidates.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| right.0.lease_expiry_ms.cmp(&left.0.lease_expiry_ms))
            .then_with(|| left.0.bridge_id.cmp(&right.0.bridge_id))
    });

    if candidates.is_empty() {
        return Err(LanePlanError::NoEligibleBridges {
            filter_drops: drops,
        });
    }

    let target = target_lane_count as usize;
    let selected_bridges = candidates
        .iter()
        .take(target)
        .map(|(entry, _)| entry.clone())
        .collect::<Vec<_>>();
    let overflow_pool = candidates
        .iter()
        .skip(target)
        .map(|(entry, _)| entry.clone())
        .collect::<Vec<_>>();

    Ok(LanePlan {
        target_lane_count,
        selected_bridges,
        overflow_pool,
        filter_drops: drops,
    })
}
