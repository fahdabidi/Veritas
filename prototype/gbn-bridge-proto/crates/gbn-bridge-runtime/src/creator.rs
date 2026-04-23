use std::collections::BTreeSet;

use gbn_bridge_protocol::{
    BootstrapDhtEntry, BridgeCatalogRequest, BridgeCatalogResponse, BridgeDescriptor,
    BridgeRefreshHint, BridgeSetResponse, PendingCreator, PublicKeyBytes, RefreshHintReason,
};

use crate::bridge::ExitBridgeRuntime;
use crate::catalog_cache::CatalogCache;
use crate::discovery::{DiscoveryHint, WeakDiscoveryState};
use crate::hint_merge::{merge_refresh_candidates, RefreshCandidate};
use crate::local_dht::LocalDht;
use crate::punch_fanout::{CreatorPunchAck, CreatorPunchAttempt, PunchFanout};
use crate::seed_catalog::SeedCatalog;
use crate::selector;
use crate::{RuntimeError, RuntimeResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatorConfig {
    pub creator_id: String,
    pub ip_addr: String,
    pub pub_key: PublicKeyBytes,
    pub udp_punch_port: u16,
}

#[derive(Debug)]
pub struct CreatorRuntime {
    config: CreatorConfig,
    publisher_trust_root: Option<PublicKeyBytes>,
    catalog_cache: CatalogCache,
    local_dht: LocalDht,
    weak_discovery: WeakDiscoveryState,
    failed_bridges: BTreeSet<String>,
    punch_fanout: PunchFanout,
    self_entry: Option<BootstrapDhtEntry>,
}

impl CreatorRuntime {
    pub fn new(config: CreatorConfig) -> Self {
        Self {
            config,
            publisher_trust_root: None,
            catalog_cache: CatalogCache::default(),
            local_dht: LocalDht::default(),
            weak_discovery: WeakDiscoveryState::default(),
            failed_bridges: BTreeSet::default(),
            punch_fanout: PunchFanout::default(),
            self_entry: None,
        }
    }

    pub fn config(&self) -> &CreatorConfig {
        &self.config
    }

    pub fn publisher_trust_root(&self) -> Option<&PublicKeyBytes> {
        self.publisher_trust_root.as_ref()
    }

    pub fn catalog_cache(&self) -> &CatalogCache {
        &self.catalog_cache
    }

    pub fn local_dht(&self) -> &LocalDht {
        &self.local_dht
    }

    pub fn weak_discovery(&self) -> &WeakDiscoveryState {
        &self.weak_discovery
    }

    pub fn punch_fanout(&self) -> &PunchFanout {
        &self.punch_fanout
    }

    pub fn self_entry(&self) -> Option<&BootstrapDhtEntry> {
        self.self_entry.as_ref()
    }

    pub fn pending_creator(&self) -> PendingCreator {
        PendingCreator {
            node_id: self.config.creator_id.clone(),
            ip_addr: self.config.ip_addr.clone(),
            pub_key: self.config.pub_key.clone(),
            udp_punch_port: self.config.udp_punch_port,
        }
    }

    pub fn load_publisher_trust_root(
        &mut self,
        publisher_key: PublicKeyBytes,
    ) -> RuntimeResult<()> {
        if let Some(existing) = &self.publisher_trust_root {
            if existing != &publisher_key {
                return Err(RuntimeError::PublisherTrustRootMismatch {
                    expected: existing.clone(),
                    actual: publisher_key,
                });
            }
        }

        self.publisher_trust_root = Some(publisher_key);
        Ok(())
    }

    pub fn set_discovery_enabled(&mut self, enabled: bool) {
        self.weak_discovery.set_enabled(enabled);
    }

    pub fn seed_discovery(&mut self, seed_catalog: &SeedCatalog) -> usize {
        self.weak_discovery
            .ingest(seed_catalog.hints().iter().cloned())
    }

    pub fn ingest_weak_discovery_hints(&mut self, hints: &[DiscoveryHint]) -> usize {
        self.weak_discovery.ingest(hints.iter().cloned())
    }

    pub fn apply_bootstrap_response(
        &mut self,
        response: &gbn_bridge_protocol::CreatorBootstrapResponse,
        now_ms: u64,
    ) -> RuntimeResult<()> {
        self.load_publisher_trust_root(response.publisher_pub.clone())?;
        let publisher_key = self.publisher_trust_root_required()?.clone();
        response.verify_authority(&publisher_key, now_ms)?;
        self.local_dht.upsert_bootstrap_entries(
            std::slice::from_ref(&response.seed_bridge),
            &publisher_key,
            now_ms,
        )?;
        Ok(())
    }

    pub fn remember_self_entry(
        &mut self,
        self_entry: BootstrapDhtEntry,
        now_ms: u64,
    ) -> RuntimeResult<()> {
        if self_entry.node_id != self.config.creator_id {
            return Err(RuntimeError::CreatorIdentityMismatch {
                expected_creator_id: self.config.creator_id.clone(),
                actual_creator_id: self_entry.node_id,
            });
        }

        let publisher_key = self.publisher_trust_root_required()?;
        self_entry.verify_authority(publisher_key, now_ms)?;
        self.self_entry = Some(self_entry);
        Ok(())
    }

    pub fn ingest_catalog(
        &mut self,
        response: BridgeCatalogResponse,
        now_ms: u64,
    ) -> RuntimeResult<()> {
        let publisher_key = self.publisher_trust_root_required()?.clone();
        self.catalog_cache
            .replace_verified(response.clone(), &publisher_key, now_ms)?;
        self.local_dht
            .upsert_catalog_bridges(&response.bridges, &publisher_key, now_ms)?;
        self.failed_bridges.clear();
        Ok(())
    }

    pub fn ordered_refresh_bridges(&self, now_ms: u64) -> RuntimeResult<Vec<BridgeDescriptor>> {
        let publisher_key = self.publisher_trust_root_required()?;
        let catalog = self.catalog_cache.load_valid(publisher_key, now_ms)?;
        selector::ordered_direct_bridges(catalog, publisher_key, now_ms, &self.failed_bridges)
    }

    pub fn select_refresh_bridge(&self, now_ms: u64) -> RuntimeResult<BridgeDescriptor> {
        let publisher_key = self.publisher_trust_root_required()?;
        let catalog = self.catalog_cache.load_valid(publisher_key, now_ms)?;
        selector::select_next_direct_bridge(catalog, publisher_key, now_ms, &self.failed_bridges)
    }

    pub fn record_refresh_failure(&mut self, bridge_id: &str) {
        self.failed_bridges.insert(bridge_id.to_string());
    }

    pub fn refresh_catalog_via_bridge(
        &mut self,
        bridge: &mut ExitBridgeRuntime,
        reason: RefreshHintReason,
        now_ms: u64,
    ) -> RuntimeResult<BridgeCatalogResponse> {
        let request = BridgeCatalogRequest {
            creator_id: self.config.creator_id.clone(),
            known_catalog_id: self
                .catalog_cache
                .current()
                .map(|catalog| catalog.catalog_id.clone()),
            direct_only: true,
            refresh_hint: Some(BridgeRefreshHint {
                bridge_id: Some(bridge.config().bridge_id.clone()),
                reason,
                last_success_ms: None,
                stale_after_ms: self
                    .catalog_cache
                    .current()
                    .map(|catalog| catalog.expires_at_ms),
            }),
        };

        let response = bridge
            .publisher_client_mut()
            .issue_catalog(&request, now_ms)?;
        self.ingest_catalog(response.clone(), now_ms)?;
        Ok(response)
    }

    pub fn store_bridge_set(
        &mut self,
        response: &BridgeSetResponse,
        now_ms: u64,
    ) -> RuntimeResult<()> {
        let publisher_key = self.publisher_trust_root_required()?.clone();
        response.verify_authority(&publisher_key, now_ms)?;
        self.local_dht.upsert_bootstrap_entries(
            &response.bridge_entries,
            &publisher_key,
            now_ms,
        )?;
        Ok(())
    }

    pub fn ordered_refresh_candidates(&self, now_ms: u64) -> RuntimeResult<Vec<RefreshCandidate>> {
        let publisher_key = self.publisher_trust_root_required()?;
        merge_refresh_candidates(
            &self.local_dht,
            &self.catalog_cache,
            &self.weak_discovery,
            publisher_key,
            now_ms,
            &self.failed_bridges,
        )
    }

    pub fn select_refresh_candidate(&self, now_ms: u64) -> RuntimeResult<RefreshCandidate> {
        self.ordered_refresh_candidates(now_ms)?
            .into_iter()
            .next()
            .ok_or(RuntimeError::NoUsableBridgeCandidate)
    }

    pub fn refresh_catalog_via_candidate(
        &mut self,
        candidate: &RefreshCandidate,
        bridge: &mut ExitBridgeRuntime,
        reason: RefreshHintReason,
        now_ms: u64,
    ) -> RuntimeResult<BridgeCatalogResponse> {
        if candidate.bridge_id != bridge.config().bridge_id {
            return Err(RuntimeError::UnexpectedBridgeRuntime {
                expected_bridge_id: candidate.bridge_id.clone(),
                actual_bridge_id: bridge.config().bridge_id.clone(),
            });
        }

        self.refresh_catalog_via_bridge(bridge, reason, now_ms)
    }

    pub fn begin_refresh_fanout(&mut self, now_ms: u64) -> RuntimeResult<Vec<CreatorPunchAttempt>> {
        let publisher_key = self.publisher_trust_root_required()?.clone();
        let catalog = self
            .catalog_cache
            .load_valid(&publisher_key, now_ms)?
            .clone();
        self.punch_fanout
            .begin_for_catalog(&catalog, &publisher_key, now_ms)
    }

    pub fn begin_bootstrap_fanout(
        &mut self,
        bootstrap_session_id: &str,
        response: &BridgeSetResponse,
        now_ms: u64,
    ) -> RuntimeResult<Vec<CreatorPunchAttempt>> {
        let publisher_key = self.publisher_trust_root_required()?.clone();
        response.verify_authority(&publisher_key, now_ms)?;
        self.local_dht.upsert_bootstrap_entries(
            &response.bridge_entries,
            &publisher_key,
            now_ms,
        )?;
        self.punch_fanout.begin_for_bootstrap_entries(
            bootstrap_session_id,
            &response.bridge_entries,
            &publisher_key,
            now_ms,
        )
    }

    pub fn acknowledge_tunnel(
        &mut self,
        attempt: &CreatorPunchAttempt,
        established_at_ms: u64,
    ) -> RuntimeResult<CreatorPunchAck> {
        let ack = self.punch_fanout.acknowledge(
            &attempt.bootstrap_session_id,
            &attempt.target_node_id,
            attempt.probe_nonce,
            established_at_ms,
        )?;
        self.local_dht
            .mark_tunnel_active(&attempt.target_node_id, established_at_ms);
        Ok(ack)
    }

    pub fn mark_bridge_active(&mut self, bridge_id: &str, established_at_ms: u64) {
        self.local_dht
            .mark_tunnel_active(bridge_id, established_at_ms);
    }

    fn publisher_trust_root_required(&self) -> RuntimeResult<&PublicKeyBytes> {
        self.publisher_trust_root
            .as_ref()
            .ok_or(RuntimeError::MissingPublisherTrustRoot)
    }
}
