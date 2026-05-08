use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::descriptor::ReachabilityClass;
use crate::error::ProtocolError;
use crate::signing::{
    ensure_not_expired, sign_payload, verify_payload, PublicKeyBytes, SignatureBytes,
};

pub const LOCAL_DISCOVERY_TABLE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeIngressEndpointKind {
    Direct,
    Brokered,
    RelayOnly,
}

impl Default for BridgeIngressEndpointKind {
    fn default() -> Self {
        Self::Direct
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeIngressEndpoint {
    #[serde(default)]
    pub kind: BridgeIngressEndpointKind,
    pub ip_addr: String,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u64>,
}

impl BridgeIngressEndpoint {
    pub fn direct(ip_addr: impl Into<String>, port: u16) -> Self {
        Self {
            kind: BridgeIngressEndpointKind::Direct,
            ip_addr: ip_addr.into(),
            port,
            ttl_ms: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublisherDhtEntry {
    pub node_id: String,
    pub authority_url: String,
    pub receiver_url: String,
    pub pub_key: PublicKeyBytes,
    pub entry_expiry_ms: u64,
}

impl PublisherDhtEntry {
    pub fn verify_trust_root(
        &self,
        trusted_publisher_key: &PublicKeyBytes,
        now_ms: u64,
    ) -> Result<(), ProtocolError> {
        if &self.pub_key != trusted_publisher_key {
            return Err(ProtocolError::InvalidSignature);
        }
        ensure_not_expired("publisher dht entry", self.entry_expiry_ms, now_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeDhtEntryUnsigned {
    pub bridge_id: String,
    pub identity_pub: PublicKeyBytes,
    pub ingress_endpoints: Vec<BridgeIngressEndpoint>,
    pub udp_punch_port: u16,
    pub reachability_class: ReachabilityClass,
    pub lease_expiry_ms: u64,
    pub entry_expiry_ms: u64,
    pub capabilities: Vec<String>,
}

impl BridgeDhtEntryUnsigned {
    pub fn validate_shape(&self) -> Result<(), ProtocolError> {
        if self.ingress_endpoints.is_empty() {
            return Err(ProtocolError::EmptyIngressEndpoints);
        }
        if self.udp_punch_port == 0 {
            return Err(ProtocolError::InvalidUdpPunchPort);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeDhtEntry {
    pub bridge_id: String,
    pub identity_pub: PublicKeyBytes,
    pub ingress_endpoints: Vec<BridgeIngressEndpoint>,
    pub udp_punch_port: u16,
    pub reachability_class: ReachabilityClass,
    pub lease_expiry_ms: u64,
    pub entry_expiry_ms: u64,
    pub capabilities: Vec<String>,
    pub publisher_sig: SignatureBytes,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspect_until_ms: Option<u64>,
}

impl BridgeDhtEntry {
    pub fn sign(
        unsigned: BridgeDhtEntryUnsigned,
        signing_key: &SigningKey,
        active: bool,
    ) -> Result<Self, ProtocolError> {
        unsigned.validate_shape()?;
        let publisher_sig = sign_payload(&unsigned, signing_key)?;
        Ok(Self {
            bridge_id: unsigned.bridge_id,
            identity_pub: unsigned.identity_pub,
            ingress_endpoints: unsigned.ingress_endpoints,
            udp_punch_port: unsigned.udp_punch_port,
            reachability_class: unsigned.reachability_class,
            lease_expiry_ms: unsigned.lease_expiry_ms,
            entry_expiry_ms: unsigned.entry_expiry_ms,
            capabilities: unsigned.capabilities,
            publisher_sig,
            active,
            suspect_until_ms: None,
        })
    }

    pub fn unsigned_payload(&self) -> BridgeDhtEntryUnsigned {
        BridgeDhtEntryUnsigned {
            bridge_id: self.bridge_id.clone(),
            identity_pub: self.identity_pub.clone(),
            ingress_endpoints: self.ingress_endpoints.clone(),
            udp_punch_port: self.udp_punch_port,
            reachability_class: self.reachability_class.clone(),
            lease_expiry_ms: self.lease_expiry_ms,
            entry_expiry_ms: self.entry_expiry_ms,
            capabilities: self.capabilities.clone(),
        }
    }

    pub fn verify_authority(
        &self,
        publisher_key: &PublicKeyBytes,
        now_ms: u64,
    ) -> Result<(), ProtocolError> {
        let unsigned = self.unsigned_payload();
        unsigned.validate_shape()?;
        verify_payload(&unsigned, publisher_key, &self.publisher_sig)?;
        ensure_not_expired("bridge dht lease", self.lease_expiry_ms, now_ms)?;
        ensure_not_expired("bridge dht entry", self.entry_expiry_ms, now_ms)
    }

    pub fn is_route_eligible(&self, now_ms: u64) -> bool {
        self.active
            && self
                .suspect_until_ms
                .is_none_or(|suspect| now_ms >= suspect)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatorDhtEntryUnsigned {
    pub node_id: String,
    pub ip_addr: String,
    pub pub_key: PublicKeyBytes,
    pub udp_punch_port: u16,
    pub entry_expiry_ms: u64,
}

impl CreatorDhtEntryUnsigned {
    pub fn validate_shape(&self) -> Result<(), ProtocolError> {
        if self.udp_punch_port == 0 {
            return Err(ProtocolError::InvalidUdpPunchPort);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatorDhtEntry {
    pub node_id: String,
    pub ip_addr: String,
    pub pub_key: PublicKeyBytes,
    pub udp_punch_port: u16,
    pub entry_expiry_ms: u64,
    pub publisher_sig: SignatureBytes,
    pub active: bool,
}

impl CreatorDhtEntry {
    pub fn sign(
        unsigned: CreatorDhtEntryUnsigned,
        signing_key: &SigningKey,
        active: bool,
    ) -> Result<Self, ProtocolError> {
        unsigned.validate_shape()?;
        let publisher_sig = sign_payload(&unsigned, signing_key)?;
        Ok(Self {
            node_id: unsigned.node_id,
            ip_addr: unsigned.ip_addr,
            pub_key: unsigned.pub_key,
            udp_punch_port: unsigned.udp_punch_port,
            entry_expiry_ms: unsigned.entry_expiry_ms,
            publisher_sig,
            active,
        })
    }

    pub fn unsigned_payload(&self) -> CreatorDhtEntryUnsigned {
        CreatorDhtEntryUnsigned {
            node_id: self.node_id.clone(),
            ip_addr: self.ip_addr.clone(),
            pub_key: self.pub_key.clone(),
            udp_punch_port: self.udp_punch_port,
            entry_expiry_ms: self.entry_expiry_ms,
        }
    }

    pub fn verify_authority(
        &self,
        publisher_key: &PublicKeyBytes,
        now_ms: u64,
    ) -> Result<(), ProtocolError> {
        let unsigned = self.unsigned_payload();
        unsigned.validate_shape()?;
        verify_payload(&unsigned, publisher_key, &self.publisher_sig)?;
        ensure_not_expired("creator dht entry", self.entry_expiry_ms, now_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCreatorSeedState {
    pub host_creator_actor_id: String,
    #[serde(default)]
    pub chain_id: String,
    pub publisher_entry: PublisherDhtEntry,
    pub exit_bridge_a_entry: BridgeDhtEntry,
    pub seeded_at_ms: u64,
    #[serde(default)]
    pub bootstrap_genesis: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewCreatorSeedState {
    pub new_creator_actor_id: String,
    pub host_creator_entry: CreatorDhtEntry,
    pub seeded_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapSession {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
    pub started_at_ms: u64,
    pub last_event_ms: u64,
    pub last_state: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelfOnboardingState {
    None,
    NewCreatorSeeded,
    Bootstrapping,
    SeedBridgeAssigned,
    SeedTunnelActive,
    BridgeSetReceived,
    FanoutInProgress,
    FanoutPartial,
    Onboarded,
    SeedTunnelFailed,
    FanoutFailed,
}

impl Default for SelfOnboardingState {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostRoleState {
    NotHost,
    HostSeeded,
}

impl Default for HostRoleState {
    fn default() -> Self {
        Self::NotHost
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelPeerRole {
    Creator,
    ExitBridge,
    Publisher,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelState {
    pub peer_id: String,
    pub peer_role: TunnelPeerRole,
    pub established_at_ms: u64,
    pub last_seen_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalDiscoveryTable {
    pub schema_version: u32,
    pub actor_id: String,
    pub role: String,
    pub self_onboarding_state: SelfOnboardingState,
    pub host_role_state: HostRoleState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_seed_state: Option<HostCreatorSeedState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_creator_seed_state: Option<NewCreatorSeedState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_entry: Option<PublisherDhtEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_creator_entry: Option<CreatorDhtEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_entry: Option<CreatorDhtEntry>,
    pub bridge_entries: Vec<BridgeDhtEntry>,
    pub active_tunnels: Vec<TunnelState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_bootstrap_session: Option<BootstrapSession>,
    pub last_update_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl LocalDiscoveryTable {
    pub fn empty(actor_id: impl Into<String>, now_ms: u64) -> Self {
        Self {
            schema_version: LOCAL_DISCOVERY_TABLE_SCHEMA_VERSION,
            actor_id: actor_id.into(),
            role: "creator".to_string(),
            self_onboarding_state: SelfOnboardingState::None,
            host_role_state: HostRoleState::NotHost,
            host_seed_state: None,
            new_creator_seed_state: None,
            publisher_entry: None,
            host_creator_entry: None,
            creator_entry: None,
            bridge_entries: Vec::new(),
            active_tunnels: Vec::new(),
            current_bootstrap_session: None,
            last_update_ms: now_ms,
            last_error: None,
        }
    }

    pub fn reset(&mut self, now_ms: u64) {
        let actor_id = self.actor_id.clone();
        *self = Self::empty(actor_id, now_ms);
    }

    pub fn validate_and_prune(
        &mut self,
        trusted_publisher_key: &PublicKeyBytes,
        now_ms: u64,
    ) -> usize {
        let mut dropped = 0;

        if self.publisher_entry.as_ref().is_some_and(|entry| {
            entry
                .verify_trust_root(trusted_publisher_key, now_ms)
                .is_err()
        }) {
            self.publisher_entry = None;
            dropped += 1;
        }

        if self.host_creator_entry.as_ref().is_some_and(|entry| {
            entry
                .verify_authority(trusted_publisher_key, now_ms)
                .is_err()
        }) {
            self.host_creator_entry = None;
            dropped += 1;
        }

        if self.creator_entry.as_ref().is_some_and(|entry| {
            entry
                .verify_authority(trusted_publisher_key, now_ms)
                .is_err()
        }) {
            self.creator_entry = None;
            dropped += 1;
        }

        let before = self.bridge_entries.len();
        self.bridge_entries.retain(|entry| {
            entry
                .verify_authority(trusted_publisher_key, now_ms)
                .is_ok()
        });
        dropped += before.saturating_sub(self.bridge_entries.len());

        dropped
    }
}
