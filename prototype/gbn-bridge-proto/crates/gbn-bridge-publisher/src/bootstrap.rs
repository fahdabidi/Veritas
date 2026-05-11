pub mod distribution;
pub mod fanout;
pub mod session;

use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    encrypt_bootstrap_payload, encryption_private_from_signing_key, publisher_encryption_identity,
    BootstrapDhtEntry, BootstrapDhtEntryUnsigned, BootstrapJoinReply, BootstrapPayloadKind,
    BridgeCapability, BridgeDhtEntry, BridgeDhtEntryUnsigned, BridgeIngressEndpointKind,
    BridgeSeedAssign, BridgeSetResponse, BridgeSetResponseUnsigned, CreatorBootstrapPayload,
    CreatorBootstrapResponse, CreatorBootstrapResponseUnsigned, CreatorDhtEntry,
    CreatorDhtEntryUnsigned, CreatorJoinRequest, DhtBridgeIngressEndpoint, PublicKeyBytes,
    ReachabilityClass, SeedBridgeCatalogPayload,
};
use serde::{Deserialize, Serialize};

use crate::policy;
use crate::punch;
use crate::storage::{BridgeRecord, InMemoryAuthorityStorage};
use crate::{AuthorityConfig, AuthorityError, AuthorityPolicy, AuthorityResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityBootstrapPlan {
    pub creator_entry: BootstrapDhtEntry,
    pub creator_dht_entry: CreatorDhtEntry,
    pub response: CreatorBootstrapResponse,
    pub bridge_set: BridgeSetResponse,
    pub seed_punch: gbn_bridge_protocol::BridgePunchStart,
    pub seed_assignment: BridgeSeedAssign,
    pub encrypted_bootstrap_payload: Option<gbn_bridge_protocol::EncryptedBootstrapPayload>,
    pub encrypted_seed_bridge_catalog_payload:
        Option<gbn_bridge_protocol::EncryptedBootstrapPayload>,
}

impl AuthorityBootstrapPlan {
    pub fn join_reply(&self) -> BootstrapJoinReply {
        distribution::join_reply(
            &self.response.chain_id,
            self.creator_entry.clone(),
            self.creator_dht_entry.clone(),
            self.response.clone(),
            self.encrypted_bootstrap_payload.clone(),
            self.encrypted_seed_bridge_catalog_payload.clone(),
            None,
        )
    }

    pub fn compatibility_join_reply(&self) -> BootstrapJoinReply {
        distribution::join_reply(
            &self.response.chain_id,
            self.creator_entry.clone(),
            self.creator_dht_entry.clone(),
            self.response.clone(),
            self.encrypted_bootstrap_payload.clone(),
            self.encrypted_seed_bridge_catalog_payload.clone(),
            Some(self.bridge_set.clone()),
        )
    }
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

pub fn creator_dht_entry(
    request: &CreatorJoinRequest,
    signing_key: &SigningKey,
    config: &AuthorityConfig,
    now_ms: u64,
    active: bool,
) -> AuthorityResult<CreatorDhtEntry> {
    request.creator.pub_key.to_verifying_key()?;

    if request.creator.udp_punch_port == 0 {
        return Err(AuthorityError::InvalidCreatorJoin {
            reason: "creator udp punch port must be non-zero",
        });
    }

    CreatorDhtEntry::sign(
        CreatorDhtEntryUnsigned {
            node_id: request.creator.node_id.clone(),
            ip_addr: request.creator.ip_addr.clone(),
            pub_key: request.creator.pub_key.clone(),
            udp_punch_port: request.creator.udp_punch_port,
            entry_expiry_ms: now_ms + config.bootstrap_entry_ttl_ms,
        },
        signing_key,
        active,
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

fn bridge_capability_label(capability: &BridgeCapability) -> &'static str {
    match capability {
        BridgeCapability::BootstrapSeed => "bootstrap_seed",
        BridgeCapability::CatalogRefresh => "catalog_refresh",
        BridgeCapability::SessionRelay => "session_relay",
        BridgeCapability::BatchAssignment => "batch_assignment",
        BridgeCapability::ProgressReporting => "progress_reporting",
    }
}

pub fn bridge_dht_entry(
    record: &BridgeRecord,
    signing_key: &SigningKey,
    config: &AuthorityConfig,
    now_ms: u64,
    active: bool,
) -> AuthorityResult<BridgeDhtEntry> {
    if !record.is_active(now_ms) {
        return Err(AuthorityError::LeaseExpired {
            bridge_id: record.bridge_id.clone(),
            lease_id: record.current_lease.lease_id.clone(),
            lease_expiry_ms: record.current_lease.lease_expiry_ms,
            heartbeat_at_ms: now_ms,
        });
    }

    let endpoint_kind = match record.reachability_class {
        ReachabilityClass::Direct => BridgeIngressEndpointKind::Direct,
        ReachabilityClass::Brokered => BridgeIngressEndpointKind::Brokered,
        ReachabilityClass::RelayOnly => BridgeIngressEndpointKind::RelayOnly,
    };
    let ingress_endpoints = record
        .ingress_endpoints
        .iter()
        .map(|endpoint| DhtBridgeIngressEndpoint {
            kind: endpoint_kind.clone(),
            ip_addr: endpoint.host.clone(),
            port: endpoint.port,
            ttl_ms: None,
        })
        .collect::<Vec<_>>();

    BridgeDhtEntry::sign(
        BridgeDhtEntryUnsigned {
            bridge_id: record.bridge_id.clone(),
            identity_pub: record.identity_pub.clone(),
            ingress_endpoints,
            udp_punch_port: record.assigned_udp_punch_port,
            reachability_class: record.reachability_class.clone(),
            lease_expiry_ms: record.current_lease.lease_expiry_ms,
            entry_expiry_ms: record
                .current_lease
                .lease_expiry_ms
                .min(now_ms + config.bootstrap_entry_ttl_ms),
            capabilities: record
                .capabilities
                .iter()
                .map(bridge_capability_label)
                .map(ToOwned::to_owned)
                .collect(),
        },
        signing_key,
        active,
    )
    .map_err(Into::into)
}

pub fn begin_bootstrap(
    storage: &mut InMemoryAuthorityStorage,
    signing_key: &SigningKey,
    publisher_pub: &PublicKeyBytes,
    config: &AuthorityConfig,
    policy: &AuthorityPolicy,
    chain_id: &str,
    request: CreatorJoinRequest,
    now_ms: u64,
) -> AuthorityResult<AuthorityBootstrapPlan> {
    let creator_entry = creator_bootstrap_entry(&request, signing_key, config, now_ms)?;
    let creator_dht_entry = creator_dht_entry(&request, signing_key, config, now_ms, true)?;
    let eligible = policy::bootstrap_candidates(storage, now_ms, policy);
    if eligible.is_empty() {
        return Err(AuthorityError::NoEligibleBootstrapBridge);
    }

    let seed_record = eligible
        .iter()
        .find(|record| record.bridge_id != request.relay_bridge_id)
        .cloned()
        .ok_or_else(|| AuthorityError::InsufficientBootstrapBridges {
            active_bridge_count: eligible.len(),
            relay_bridge_id: request.relay_bridge_id.clone(),
        })?;

    let selected_bridge_records = eligible
        .into_iter()
        .take(config.bootstrap_bridge_count)
        .collect::<Vec<_>>();

    let bridge_entries = selected_bridge_records
        .iter()
        .map(|record| bridge_bootstrap_entry(record, signing_key, config, now_ms))
        .collect::<AuthorityResult<Vec<_>>>()?;
    let bridge_dht_entries = selected_bridge_records
        .iter()
        .map(|record| {
            let mut entry = storage
                .publisher_bridge_dht_entries
                .get(&record.bridge_id)
                .cloned()
                .ok_or_else(|| AuthorityError::PublisherBridgeDhtEntryMissing {
                    bridge_id: record.bridge_id.clone(),
                })?;
            entry.verify_authority(publisher_pub, now_ms)?;
            entry.active = false;
            Ok(entry)
        })
        .collect::<AuthorityResult<Vec<_>>>()?;

    let bootstrap_session_id = storage.next_bootstrap_id();
    let response = CreatorBootstrapResponse::sign(
        CreatorBootstrapResponseUnsigned {
            chain_id: chain_id.to_string(),
            bootstrap_session_id: bootstrap_session_id.clone(),
            seed_bridge: bridge_bootstrap_entry(&seed_record, signing_key, config, now_ms)?,
            publisher_pub: publisher_pub.clone(),
            publisher_encryption_pub: Some(publisher_encryption_identity(signing_key)),
            response_expiry_ms: now_ms + config.bootstrap_response_ttl_ms,
            assigned_bridge_count: bridge_entries.len() as u16,
        },
        signing_key,
    )?;

    let bridge_set = BridgeSetResponse::sign(
        BridgeSetResponseUnsigned {
            chain_id: chain_id.to_string(),
            bootstrap_session_id: bootstrap_session_id.clone(),
            bridge_entries: bridge_entries.clone(),
            bridge_dht_entries: bridge_dht_entries.clone(),
            response_expiry_ms: now_ms + config.bootstrap_response_ttl_ms,
        },
        signing_key,
    )?;

    let seed_punch = punch::issue_seed_punch_instruction(
        signing_key,
        chain_id,
        &bootstrap_session_id,
        &seed_record.bridge_id,
        creator_entry.clone(),
        config,
        now_ms,
    )?;
    let seed_assignment = distribution::issue_seed_assignment(
        signing_key,
        &seed_record.bridge_id,
        creator_entry.clone(),
        bridge_set.clone(),
        seed_punch.clone(),
        now_ms + config.bootstrap_response_ttl_ms,
    )?;
    let (encrypted_bootstrap_payload, encrypted_seed_bridge_catalog_payload) =
        if let Some(creator_encryption_pub) = &request.creator.encryption_pub_key {
            let sender_private = encryption_private_from_signing_key(signing_key);
            let recipient_key_id = request.creator.node_id.clone();
            let bootstrap_payload = CreatorBootstrapPayload {
                chain_id: chain_id.to_string(),
                bootstrap_session_id: bootstrap_session_id.clone(),
                creator_entry: creator_entry.clone(),
                creator_dht_entry: creator_dht_entry.clone(),
                response: response.clone(),
                publisher_entry: None,
            };
            let encrypted_bootstrap = encrypt_bootstrap_payload(
                BootstrapPayloadKind::CreatorBootstrap,
                chain_id,
                &bootstrap_session_id,
                &bootstrap_payload,
                creator_encryption_pub,
                &recipient_key_id,
                sender_private,
            )?;
            let catalog_payload = SeedBridgeCatalogPayload {
                chain_id: chain_id.to_string(),
                bootstrap_session_id: bootstrap_session_id.clone(),
                seed_bridge_id: seed_record.bridge_id.clone(),
                bridge_set: bridge_set.clone(),
            };
            let encrypted_catalog = encrypt_bootstrap_payload(
                BootstrapPayloadKind::SeedBridgeCatalog,
                chain_id,
                &bootstrap_session_id,
                &catalog_payload,
                creator_encryption_pub,
                recipient_key_id,
                sender_private,
            )?;
            (Some(encrypted_bootstrap), Some(encrypted_catalog))
        } else {
            (None, None)
        };

    storage.bootstrap_sessions.insert(
        bootstrap_session_id.clone(),
        session::build_session_record(
            config,
            chain_id,
            &request.request_id,
            creator_entry.clone(),
            response.clone(),
            bridge_set.clone(),
            request.host_creator_id,
            request.relay_bridge_id,
            seed_record.bridge_id.clone(),
            bridge_entries
                .iter()
                .map(|entry| entry.node_id.clone())
                .collect(),
            now_ms,
        ),
    );

    Ok(AuthorityBootstrapPlan {
        creator_entry,
        creator_dht_entry,
        response,
        bridge_set,
        seed_punch,
        seed_assignment,
        encrypted_bootstrap_payload,
        encrypted_seed_bridge_catalog_payload,
    })
}
