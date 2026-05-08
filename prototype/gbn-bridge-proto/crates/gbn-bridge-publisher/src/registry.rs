use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    BridgeHeartbeat, BridgeLease, BridgeLeaseUnsigned, BridgeRegister, BridgeRevoke,
    BridgeRevokeUnsigned, ReachabilityClass, RevocationReason,
};

use crate::storage::{BridgeRecord, InMemoryAuthorityStorage};
use crate::{AuthorityConfig, AuthorityError, AuthorityResult};

pub fn register_bridge(
    storage: &mut InMemoryAuthorityStorage,
    signing_key: &SigningKey,
    config: &AuthorityConfig,
    request: BridgeRegister,
    reachability_class: ReachabilityClass,
    now_ms: u64,
) -> AuthorityResult<BridgeLease> {
    request.identity_pub.to_verifying_key()?;

    if request.ingress_endpoints.is_empty() {
        return Err(AuthorityError::InvalidBridgeRegistration {
            reason: "bridge ingress endpoints are required",
        });
    }

    if let Some(record) = storage
        .bridges
        .get(&request.bridge_id)
        .filter(|record| record.is_active(now_ms))
    {
        if record.identity_pub != request.identity_pub {
            return Err(AuthorityError::BridgeAlreadyRegistered {
                bridge_id: request.bridge_id,
            });
        }
    }

    let bridge_id = request.bridge_id.clone();
    let assigned_udp_punch_port = if request.requested_udp_punch_port == 0 {
        config.default_udp_punch_port
    } else {
        request.requested_udp_punch_port
    };

    let lease_unsigned = BridgeLeaseUnsigned {
        lease_id: storage.next_lease_id(),
        bridge_id: bridge_id.clone(),
        udp_punch_port: assigned_udp_punch_port,
        reachability_class: reachability_class.clone(),
        lease_expiry_ms: now_ms + config.lease_ttl_ms,
        issued_at_ms: now_ms,
        heartbeat_interval_ms: config.heartbeat_interval_ms,
        capabilities: request.capabilities.clone(),
    };
    let lease = BridgeLease::sign(lease_unsigned, signing_key)?;

    let heartbeat = BridgeHeartbeat {
        lease_id: lease.lease_id.clone(),
        bridge_id: bridge_id.clone(),
        heartbeat_at_ms: now_ms,
        active_sessions: 0,
        observed_ingress: None,
    };

    storage.bridges.insert(
        bridge_id.clone(),
        BridgeRecord {
            bridge_id,
            identity_pub: request.identity_pub,
            ingress_endpoints: request.ingress_endpoints,
            assigned_udp_punch_port,
            reachability_class,
            capabilities: request.capabilities,
            current_lease: lease.clone(),
            last_heartbeat: heartbeat,
            revoked_reason: None,
            revoked_at_ms: None,
        },
    );

    Ok(lease)
}

pub fn renew_lease_from_heartbeat(
    storage: &mut InMemoryAuthorityStorage,
    signing_key: &SigningKey,
    config: &AuthorityConfig,
    heartbeat: BridgeHeartbeat,
) -> AuthorityResult<BridgeLease> {
    let record = storage
        .bridges
        .get_mut(&heartbeat.bridge_id)
        .ok_or_else(|| AuthorityError::BridgeNotFound {
            bridge_id: heartbeat.bridge_id.clone(),
        })?;

    if record.revoked_reason.is_some() {
        return Err(AuthorityError::BridgeRevoked {
            bridge_id: heartbeat.bridge_id,
        });
    }

    if heartbeat.lease_id != record.current_lease.lease_id {
        return Err(AuthorityError::LeaseMismatch {
            bridge_id: heartbeat.bridge_id,
            expected: record.current_lease.lease_id.clone(),
            actual: heartbeat.lease_id,
        });
    }

    if heartbeat.heartbeat_at_ms > record.current_lease.lease_expiry_ms {
        return Err(AuthorityError::LeaseExpired {
            bridge_id: record.bridge_id.clone(),
            lease_id: record.current_lease.lease_id.clone(),
            lease_expiry_ms: record.current_lease.lease_expiry_ms,
            heartbeat_at_ms: heartbeat.heartbeat_at_ms,
        });
    }

    record.last_heartbeat = heartbeat.clone();
    let lease_unsigned = BridgeLeaseUnsigned {
        lease_id: record.current_lease.lease_id.clone(),
        bridge_id: record.bridge_id.clone(),
        udp_punch_port: record.assigned_udp_punch_port,
        reachability_class: record.reachability_class.clone(),
        lease_expiry_ms: heartbeat.heartbeat_at_ms + config.lease_ttl_ms,
        issued_at_ms: heartbeat.heartbeat_at_ms,
        heartbeat_interval_ms: config.heartbeat_interval_ms,
        capabilities: record.capabilities.clone(),
    };
    let renewed = BridgeLease::sign(lease_unsigned, signing_key)?;
    record.current_lease = renewed.clone();

    Ok(renewed)
}

pub fn reclassify_bridge(
    storage: &mut InMemoryAuthorityStorage,
    signing_key: &SigningKey,
    config: &AuthorityConfig,
    bridge_id: &str,
    reachability_class: ReachabilityClass,
    udp_punch_port: Option<u16>,
    now_ms: u64,
) -> AuthorityResult<BridgeLease> {
    let record =
        storage
            .bridges
            .get_mut(bridge_id)
            .ok_or_else(|| AuthorityError::BridgeNotFound {
                bridge_id: bridge_id.to_string(),
            })?;

    if record.revoked_reason.is_some() {
        return Err(AuthorityError::BridgeRevoked {
            bridge_id: bridge_id.to_string(),
        });
    }

    let assigned_udp_punch_port = udp_punch_port.unwrap_or(record.assigned_udp_punch_port);
    if assigned_udp_punch_port == 0 {
        return Err(AuthorityError::InvalidBridgeRegistration {
            reason: "bridge udp punch port must be non-zero",
        });
    }

    record.assigned_udp_punch_port = assigned_udp_punch_port;
    record.reachability_class = reachability_class.clone();
    let lease_unsigned = BridgeLeaseUnsigned {
        lease_id: record.current_lease.lease_id.clone(),
        bridge_id: record.bridge_id.clone(),
        udp_punch_port: assigned_udp_punch_port,
        reachability_class,
        lease_expiry_ms: now_ms + config.lease_ttl_ms,
        issued_at_ms: now_ms,
        heartbeat_interval_ms: config.heartbeat_interval_ms,
        capabilities: record.capabilities.clone(),
    };
    let lease = BridgeLease::sign(lease_unsigned, signing_key)?;
    record.current_lease = lease.clone();
    record.last_heartbeat.heartbeat_at_ms = now_ms;

    Ok(lease)
}

pub fn revoke_bridge(
    storage: &mut InMemoryAuthorityStorage,
    signing_key: &SigningKey,
    bridge_id: &str,
    reason: RevocationReason,
    now_ms: u64,
) -> AuthorityResult<BridgeRevoke> {
    let record =
        storage
            .bridges
            .get_mut(bridge_id)
            .ok_or_else(|| AuthorityError::BridgeNotFound {
                bridge_id: bridge_id.to_string(),
            })?;

    record.revoked_reason = Some(reason.clone());
    record.revoked_at_ms = Some(now_ms);
    let revoke = BridgeRevoke::sign(
        BridgeRevokeUnsigned {
            lease_id: record.current_lease.lease_id.clone(),
            bridge_id: bridge_id.to_string(),
            revoked_at_ms: now_ms,
            reason,
        },
        signing_key,
    )?;

    Ok(revoke)
}

pub fn active_bridge_records(
    storage: &InMemoryAuthorityStorage,
    now_ms: u64,
    direct_only: bool,
) -> Vec<BridgeRecord> {
    storage
        .bridges
        .values()
        .filter(|record| record.is_active(now_ms))
        .filter(|record| !direct_only || record.is_direct())
        .cloned()
        .collect()
}
