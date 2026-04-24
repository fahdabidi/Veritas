use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    BootstrapDhtEntry, BootstrapDhtEntryUnsigned, BridgeSetResponse, BridgeSetResponseUnsigned,
    CreatorBootstrapResponse, CreatorBootstrapResponseUnsigned, CreatorJoinRequest, PublicKeyBytes,
};
use serde::{Deserialize, Serialize};

use crate::policy;
use crate::punch;
use crate::storage::{BootstrapSessionRecord, BridgeRecord, InMemoryAuthorityStorage};
use crate::{AuthorityConfig, AuthorityError, AuthorityPolicy, AuthorityResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityBootstrapPlan {
    pub creator_entry: BootstrapDhtEntry,
    pub response: CreatorBootstrapResponse,
    pub bridge_set: BridgeSetResponse,
    pub seed_punch: gbn_bridge_protocol::BridgePunchStart,
}

pub fn creator_bootstrap_entry(
    request: &CreatorJoinRequest,
    signing_key: &SigningKey,
    config: &AuthorityConfig,
    now_ms: u64,
) -> AuthorityResult<BootstrapDhtEntry> {
    request.creator.pub_key.to_verifying_key()?;

    if request.creator.udp_punch_port == 0 {
        return Err(AuthorityError::InvalidCreatorJoin {
            reason: "creator udp punch port must be non-zero",
        });
    }

    BootstrapDhtEntry::sign(
        BootstrapDhtEntryUnsigned {
            node_id: request.creator.node_id.clone(),
            ip_addr: request.creator.ip_addr.clone(),
            pub_key: request.creator.pub_key.clone(),
            udp_punch_port: request.creator.udp_punch_port,
            entry_expiry_ms: now_ms + config.bootstrap_entry_ttl_ms,
        },
        signing_key,
    )
    .map_err(Into::into)
}

pub fn bridge_bootstrap_entry(
    record: &BridgeRecord,
    signing_key: &SigningKey,
    config: &AuthorityConfig,
    now_ms: u64,
) -> AuthorityResult<BootstrapDhtEntry> {
    let ip_addr = record
        .ingress_endpoints
        .first()
        .ok_or(AuthorityError::InvalidBridgeRegistration {
            reason: "bridge ingress endpoints are required",
        })?
        .host
        .clone();

    BootstrapDhtEntry::sign(
        BootstrapDhtEntryUnsigned {
            node_id: record.bridge_id.clone(),
            ip_addr,
            pub_key: record.identity_pub.clone(),
            udp_punch_port: record.assigned_udp_punch_port,
            entry_expiry_ms: record
                .current_lease
                .lease_expiry_ms
                .min(now_ms + config.bootstrap_entry_ttl_ms),
        },
        signing_key,
    )
    .map_err(Into::into)
}

pub fn begin_bootstrap(
    storage: &mut InMemoryAuthorityStorage,
    signing_key: &SigningKey,
    publisher_pub: &PublicKeyBytes,
    config: &AuthorityConfig,
    policy: &AuthorityPolicy,
    request: CreatorJoinRequest,
    now_ms: u64,
) -> AuthorityResult<AuthorityBootstrapPlan> {
    let creator_entry = creator_bootstrap_entry(&request, signing_key, config, now_ms)?;
    let mut eligible = policy::bootstrap_candidates(storage, now_ms, policy);
    if eligible.is_empty() {
        return Err(AuthorityError::NoEligibleBootstrapBridge);
    }

    let seed_record = eligible
        .iter()
        .find(|record| record.bridge_id != request.relay_bridge_id)
        .cloned()
        .unwrap_or_else(|| eligible[0].clone());

    let bridge_entries = eligible
        .drain(..)
        .take(config.bootstrap_bridge_count)
        .map(|record| bridge_bootstrap_entry(&record, signing_key, config, now_ms))
        .collect::<AuthorityResult<Vec<_>>>()?;

    let bootstrap_session_id = storage.next_bootstrap_id();
    let response = CreatorBootstrapResponse::sign(
        CreatorBootstrapResponseUnsigned {
            bootstrap_session_id: bootstrap_session_id.clone(),
            seed_bridge: bridge_bootstrap_entry(&seed_record, signing_key, config, now_ms)?,
            publisher_pub: publisher_pub.clone(),
            response_expiry_ms: now_ms + config.bootstrap_response_ttl_ms,
            assigned_bridge_count: bridge_entries.len() as u16,
        },
        signing_key,
    )?;

    let bridge_set = BridgeSetResponse::sign(
        BridgeSetResponseUnsigned {
            bootstrap_session_id: bootstrap_session_id.clone(),
            bridge_entries: bridge_entries.clone(),
            response_expiry_ms: now_ms + config.bootstrap_response_ttl_ms,
        },
        signing_key,
    )?;

    let seed_punch = punch::issue_seed_punch_instruction(
        signing_key,
        &bootstrap_session_id,
        &seed_record.bridge_id,
        creator_entry.clone(),
        config,
        now_ms,
    )?;

    storage.bootstrap_sessions.insert(
        bootstrap_session_id.clone(),
        BootstrapSessionRecord {
            bootstrap_session_id: bootstrap_session_id.clone(),
            creator_entry: creator_entry.clone(),
            host_creator_id: request.host_creator_id,
            relay_bridge_id: request.relay_bridge_id,
            seed_bridge_id: seed_record.bridge_id.clone(),
            bridge_ids: bridge_entries
                .iter()
                .map(|entry| entry.node_id.clone())
                .collect(),
            created_at_ms: now_ms,
            response_expiry_ms: now_ms + config.bootstrap_response_ttl_ms,
            progress_events: Vec::new(),
        },
    );

    Ok(AuthorityBootstrapPlan {
        creator_entry,
        response,
        bridge_set,
        seed_punch,
    })
}
