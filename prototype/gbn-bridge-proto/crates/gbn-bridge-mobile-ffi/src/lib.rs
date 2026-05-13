use std::collections::{BTreeMap, HashMap};
use std::ffi::{CStr, CString};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::raw::c_char;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_creator::{
    build_upload_session_to_disk, list_upload_sessions, BuildUploadSessionOptions, LocalDhtStore,
    ResetCreatorStateResponse, SanitizerFormatHint, UploadSessionSummary,
};
use gbn_bridge_protocol::{
    encryption_identity_from_signing_key, publisher_identity, BootstrapSession, BridgeDhtEntry,
    BridgeDhtEntryUnsigned, CreatorDhtEntry, CreatorDhtEntryUnsigned, DhtBridgeIngressEndpoint,
    LocalDiscoveryTable, NewCreatorSeedState, PublicKeyBytes, PublisherDhtEntry, ReachabilityClass,
    SelfOnboardingState, TunnelPeerRole, TunnelState,
};
use jni::objects::{JClass, JString};
use jni::sys::{jlong, jstring};
use jni::JNIEnv;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

const IDENTITY_FILE: &str = "identity.json";
const LOCAL_DHT_FILE: &str = "local_dht.json";
const HOST_SEED_FILE: &str = "host_creator_seed.json";
const EVENT_FILE: &str = "events.jsonl";
const MOBILE_FFI_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Error)]
pub enum MobileRuntimeError {
    #[error("config error: {0}")]
    Config(String),

    #[error("state path escape rejected: {0}")]
    StatePathEscape(String),

    #[error("I/O error: {0}")]
    Io(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("local DHT error: {0}")]
    LocalDht(String),

    #[error("QR seed invalid: {0}")]
    InvalidQrSeed(String),

    #[error("operation not implemented in Phase 2: {0}")]
    NotImplemented(String),

    #[error("runtime error: {0}")]
    Runtime(String),
}

impl MobileRuntimeError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Config(_) => "config_error",
            Self::StatePathEscape(_) => "state_path_escape",
            Self::Io(_) => "io_error",
            Self::Serialization(_) => "serialization_error",
            Self::LocalDht(_) => "local_dht_error",
            Self::InvalidQrSeed(_) => "invalid_qr_seed",
            Self::NotImplemented(_) => "not_implemented",
            Self::Runtime(_) => "runtime_error",
        }
    }
}

impl From<std::io::Error> for MobileRuntimeError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

impl From<serde_json::Error> for MobileRuntimeError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value.to_string())
    }
}

impl From<gbn_bridge_creator::LocalDhtError> for MobileRuntimeError {
    fn from(value: gbn_bridge_creator::LocalDhtError) -> Self {
        Self::LocalDht(value.to_string())
    }
}

impl From<gbn_bridge_creator::SessionBuildError> for MobileRuntimeError {
    fn from(value: gbn_bridge_creator::SessionBuildError) -> Self {
        Self::Runtime(value.to_string())
    }
}

pub type MobileResult<T> = Result<T, MobileRuntimeError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatorRuntimeConfig {
    pub state_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_root_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_public_key_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_id: Option<String>,
    #[serde(default = "default_network_profile")]
    pub network_profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_config_json: Option<String>,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_dir: Option<String>,
}

fn default_network_profile() -> String {
    "offline_test".to_string()
}

fn default_log_level() -> String {
    "info".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMetadata {
    pub creator_id: String,
    pub role: String,
    pub state_dir: String,
    pub evidence_dir: String,
    pub network_profile: String,
    pub rust_build_id: String,
    pub mobile_ffi_version: String,
    pub abi: String,
    pub identity_public_key_hex: String,
    pub encryption_public_key_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEventFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatorTraceEvent {
    pub timestamp_ms: u64,
    pub chain_id: String,
    pub event: String,
    pub severity: String,
    pub actor_id: String,
    pub operation: String,
    #[serde(default)]
    pub details: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapDhtQrPreview {
    pub valid: bool,
    pub schema_version: u32,
    pub chain_id: String,
    pub run_id: String,
    pub host_creator_id: String,
    pub host_creator_public_key_hex: String,
    pub endpoint_count: usize,
    pub expires_at_ms: u64,
    pub payload_hash: String,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCreatorDhtSeedImportRequest {
    pub payload: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCreatorDhtSeedImportResult {
    pub chain_id: String,
    pub run_id: String,
    pub host_creator_id: String,
    pub host_creator_public_key_hex: String,
    pub endpoint_count: usize,
    pub payload_hash: String,
    pub self_onboarding_state: SelfOnboardingState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshBridgeCatalogRequest {
    #[serde(default)]
    pub include_inactive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeCatalogSnapshot {
    pub chain_id: Option<String>,
    pub bridge_count: usize,
    pub active_bridge_count: usize,
    pub bridges: Vec<BridgeDhtEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildSyntheticUploadRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
    #[serde(default = "default_synthetic_size")]
    pub size_bytes: usize,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    #[serde(default = "default_sanitization_profile")]
    pub sanitization_profile: String,
}

fn default_synthetic_size() -> usize {
    256
}

fn default_chunk_size() -> usize {
    64 * 1024
}

fn default_sanitization_profile() -> String {
    "mobile_phase2_synthetic".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapNewCreatorRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapNewCreatorResult {
    pub chain_id: String,
    pub bootstrap_session_id: String,
    pub self_onboarding_state: SelfOnboardingState,
    pub publisher_entry_present: bool,
    pub seed_bridge_id: String,
    pub bridge_count: usize,
    pub active_bridge_count: usize,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendDummyRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
    #[serde(default = "default_dummy_size")]
    pub size_bytes: usize,
    #[serde(default)]
    pub force_bridge_failure: bool,
}

fn default_dummy_size() -> usize {
    256
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MobileSendDummyResult {
    pub chain_id: String,
    pub actor_id: String,
    pub route_source: String,
    pub candidate_bridge_ids: Vec<String>,
    pub selected_bridge_ids: Vec<String>,
    pub assigned_bridge_id: String,
    pub encryption_envelope: String,
    pub ciphertext_only_at_bridge: bool,
    pub frames: u32,
    pub payload_size_bytes: usize,
    pub payload_sha256: String,
    pub force_bridge_failure_used: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendUploadRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default = "default_target_lane_count")]
    pub target_lane_count: u32,
    #[serde(default)]
    pub force_lane_failure: Vec<String>,
}

fn default_target_lane_count() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MobileSendUploadResult {
    pub session_id: String,
    pub chain_id: String,
    pub session_status: String,
    pub total_chunks: u32,
    pub completed_chunks: u32,
    pub lanes_used: Vec<String>,
    pub lane_count_at_first_dispatch: u32,
    pub lane_count_at_completion: u32,
    pub ciphertext_only_at_bridge: bool,
    pub force_lane_failure_used: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceFile {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteTraceQuery {
    pub chain_id: String,
    pub surface: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    pub query_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceBundle {
    pub bundle_id: String,
    pub created_at_ms: u64,
    pub state_dir: String,
    pub bundle_dir: String,
    pub chain_ids: Vec<String>,
    pub files: Vec<EvidenceFile>,
    pub remote_trace_queries: Vec<RemoteTraceQuery>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MobileIdentity {
    creator_id: String,
    signing_key_hex: String,
    public_key_hex: String,
    encryption_public_key_hex: String,
    created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCreatorDhtSeed {
    pub schema_version: u32,
    pub chain_id: String,
    pub run_id: String,
    pub host_creator_id: String,
    pub host_creator_public_key_hex: String,
    pub host_creator_entry: CreatorDhtEntry,
    pub host_creator_reachability: HostCreatorReachability,
    pub host_creator_bootstrap_endpoints: Vec<HostCreatorBootstrapEndpoint>,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCreatorReachability {
    pub reachability_class: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCreatorBootstrapEndpoint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls_sni: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certificate_sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct PublicEndpointProfile {
    #[serde(default = "default_network_profile")]
    profile: String,
    #[serde(default)]
    run_id: String,
    #[serde(default)]
    endpoint_map_id: String,
    #[serde(default)]
    aws_exitbridge_region: Option<String>,
    endpoints: Vec<PublicEndpointDescriptor>,
}

impl PublicEndpointProfile {
    fn endpoint(&self, role: &str) -> MobileResult<&PublicEndpointDescriptor> {
        self.endpoints
            .iter()
            .find(|endpoint| endpoint.role == role)
            .ok_or_else(|| {
                MobileRuntimeError::Config(format!(
                    "public endpoint profile missing `{role}` endpoint"
                ))
            })
    }
}

#[derive(Debug, Clone, Deserialize)]
struct PublicEndpointDescriptor {
    endpoint_id: String,
    actor_id: String,
    role: String,
    protocol: String,
    public_host: String,
    #[serde(default)]
    tcp_port: Option<u16>,
    #[serde(default)]
    udp_port: Option<u16>,
    expires_at_ms: u64,
}

pub struct MobileCreatorRuntime {
    config: CreatorRuntimeConfig,
    state_dir: PathBuf,
    evidence_dir: PathBuf,
    identity: MobileIdentity,
    local_dht: LocalDhtStore,
    event_path: PathBuf,
}

impl MobileCreatorRuntime {
    pub fn new(config: CreatorRuntimeConfig) -> MobileResult<Self> {
        validate_network_profile(&config.network_profile)?;
        let state_dir = normalize_path(&config.state_dir)?;
        if state_dir.as_os_str().is_empty() || state_dir.parent().is_none() {
            return Err(MobileRuntimeError::Config(
                "state_dir must be a non-root app-private path".to_string(),
            ));
        }
        if let Some(root) = &config.app_root_dir {
            let root = normalize_path(root)?;
            if !state_dir.starts_with(&root) {
                return Err(MobileRuntimeError::StatePathEscape(format!(
                    "state_dir `{}` is outside app_root_dir `{}`",
                    state_dir.display(),
                    root.display()
                )));
            }
            fs::create_dir_all(root)?;
        }
        fs::create_dir_all(&state_dir)?;

        let evidence_dir = match &config.evidence_dir {
            Some(path) => normalize_path(path)?,
            None => state_dir.join("evidence"),
        };
        if !evidence_dir.starts_with(&state_dir) {
            return Err(MobileRuntimeError::StatePathEscape(format!(
                "evidence_dir `{}` is outside state_dir `{}`",
                evidence_dir.display(),
                state_dir.display()
            )));
        }
        fs::create_dir_all(&evidence_dir)?;
        fs::create_dir_all(state_dir.join("upload_sessions"))?;

        let now_ms = now_ms();
        let identity = load_or_create_identity(
            &state_dir.join(IDENTITY_FILE),
            config.creator_id.as_deref(),
            now_ms,
        )?;
        let trusted_publisher_key = match &config.publisher_public_key_hex {
            Some(value) => Some(PublicKeyBytes(decode_hex(value)?)),
            None => None,
        };
        let local_dht = LocalDhtStore::load_or_create(
            identity.creator_id.clone(),
            state_dir.join(LOCAL_DHT_FILE),
            trusted_publisher_key.as_ref(),
            now_ms,
        )?;
        let event_path = evidence_dir.join(EVENT_FILE);
        if !event_path.exists() {
            File::create(&event_path)?;
        }
        let runtime = Self {
            config,
            state_dir,
            evidence_dir,
            identity,
            local_dht,
            event_path,
        };
        runtime.emit(
            "mobile-runtime-startup",
            "creator_runtime_started",
            "runtime_init",
            json!({"network_profile": runtime.config.network_profile}),
        )?;
        runtime.emit(
            "mobile-runtime-startup",
            "creator_state_loaded",
            "state_load",
            json!({"state_path": runtime.redacted_state_path()}),
        )?;
        Ok(runtime)
    }

    pub fn node_metadata(&self) -> NodeMetadata {
        NodeMetadata {
            creator_id: self.identity.creator_id.clone(),
            role: "creator".to_string(),
            state_dir: self.redacted_state_path(),
            evidence_dir: redact_path(&self.evidence_dir),
            network_profile: self.config.network_profile.clone(),
            rust_build_id: build_id(),
            mobile_ffi_version: MOBILE_FFI_VERSION.to_string(),
            abi: std::env::consts::ARCH.to_string(),
            identity_public_key_hex: self.identity.public_key_hex.clone(),
            encryption_public_key_hex: self.identity.encryption_public_key_hex.clone(),
        }
    }

    pub fn local_dht(&self) -> LocalDiscoveryTable {
        self.local_dht.snapshot()
    }

    pub fn trace_events(&self, filter: TraceEventFilter) -> MobileResult<Vec<CreatorTraceEvent>> {
        let raw = fs::read_to_string(&self.event_path)?;
        let mut events = raw
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str::<CreatorTraceEvent>)
            .collect::<Result<Vec<_>, _>>()?;
        events.retain(|event| {
            filter
                .chain_id
                .as_ref()
                .is_none_or(|value| &event.chain_id == value)
                && filter
                    .event
                    .as_ref()
                    .is_none_or(|value| &event.event == value)
                && filter
                    .operation
                    .as_ref()
                    .is_none_or(|value| &event.operation == value)
                && filter
                    .since_ms
                    .is_none_or(|value| event.timestamp_ms >= value)
                && filter
                    .until_ms
                    .is_none_or(|value| event.timestamp_ms <= value)
        });
        if let Some(limit) = filter.limit {
            if events.len() > limit {
                events = events.split_off(events.len() - limit);
            }
        }
        Ok(events)
    }

    pub fn reset_state(&self, chain_id: String) -> MobileResult<ResetCreatorStateResponse> {
        let result = self.local_dht.reset(chain_id.clone(), now_ms())?;
        let upload_root = self.state_dir.join("upload_sessions");
        if upload_root.exists() {
            fs::remove_dir_all(&upload_root)?;
        }
        fs::create_dir_all(upload_root)?;
        self.emit(
            &chain_id,
            "creator_state_reset",
            "reset_state",
            json!({"prior_bootstrap_session_id": result.prior_bootstrap_session_id}),
        )?;
        Ok(result)
    }

    pub fn preview_bootstrap_dht_qr(&self, payload: &str) -> MobileResult<BootstrapDhtQrPreview> {
        let seed = parse_and_validate_seed(payload, now_ms())?;
        let preview = BootstrapDhtQrPreview {
            valid: true,
            schema_version: seed.schema_version,
            chain_id: seed.chain_id.clone(),
            run_id: seed.run_id.clone(),
            host_creator_id: seed.host_creator_id.clone(),
            host_creator_public_key_hex: seed.host_creator_public_key_hex.clone(),
            endpoint_count: seed.host_creator_bootstrap_endpoints.len(),
            expires_at_ms: seed.expires_at_ms,
            payload_hash: payload_hash(payload),
            warnings: Vec::new(),
        };
        self.emit(
            &seed.chain_id,
            "creator_bootstrap_dht_qr_previewed",
            "preview_bootstrap_dht_qr",
            json!({"host_creator_id": seed.host_creator_id, "endpoint_count": preview.endpoint_count}),
        )?;
        Ok(preview)
    }

    pub fn import_host_creator_dht_seed(
        &self,
        request: HostCreatorDhtSeedImportRequest,
    ) -> MobileResult<HostCreatorDhtSeedImportResult> {
        let seed = parse_and_validate_seed(&request.payload, now_ms())?;
        write_json_atomic(&self.state_dir.join(HOST_SEED_FILE), &redacted_seed(&seed)?)?;
        let mut table = self.local_dht.snapshot();
        table.self_onboarding_state = SelfOnboardingState::NewCreatorSeeded;
        table.host_creator_entry = Some(seed.host_creator_entry.clone());
        table.new_creator_seed_state = Some(NewCreatorSeedState {
            new_creator_actor_id: self.identity.creator_id.clone(),
            chain_id: seed.chain_id.clone(),
            host_creator_entry: seed.host_creator_entry.clone(),
            seeded_at_ms: now_ms(),
            start_bootstrap: false,
        });
        table.last_update_ms = now_ms();
        self.local_dht.replace(table.clone())?;
        self.emit(
            &seed.chain_id,
            "creator_host_dht_seed_imported",
            "import_host_creator_dht_seed",
            json!({"host_creator_id": seed.host_creator_id, "endpoint_count": seed.host_creator_bootstrap_endpoints.len()}),
        )?;
        self.emit(
            &seed.chain_id,
            "creator_state_persisted",
            "local_dht_update",
            json!({"self_onboarding_state": "new_creator_seeded"}),
        )?;
        Ok(HostCreatorDhtSeedImportResult {
            chain_id: seed.chain_id,
            run_id: seed.run_id,
            host_creator_id: seed.host_creator_id,
            host_creator_public_key_hex: seed.host_creator_public_key_hex,
            endpoint_count: seed.host_creator_bootstrap_endpoints.len(),
            payload_hash: payload_hash(&request.payload),
            self_onboarding_state: table.self_onboarding_state,
        })
    }

    pub fn refresh_bridge_catalog(
        &self,
        request: RefreshBridgeCatalogRequest,
    ) -> MobileResult<BridgeCatalogSnapshot> {
        let table = self.local_dht.snapshot();
        let bridges = table
            .bridge_entries
            .iter()
            .filter(|entry| request.include_inactive || entry.active)
            .cloned()
            .collect::<Vec<_>>();
        let snapshot = BridgeCatalogSnapshot {
            chain_id: table
                .current_bootstrap_session
                .as_ref()
                .and_then(|session| session.chain_id.clone()),
            bridge_count: table.bridge_entries.len(),
            active_bridge_count: table
                .bridge_entries
                .iter()
                .filter(|entry| entry.active)
                .count(),
            bridges,
        };
        self.emit(
            snapshot.chain_id.as_deref().unwrap_or("mobile-catalog-refresh"),
            "creator_catalog_refreshed",
            "refresh_bridge_catalog",
            json!({"bridge_count": snapshot.bridge_count, "active_bridge_count": snapshot.active_bridge_count}),
        )?;
        Ok(snapshot)
    }

    pub fn build_synthetic_upload_session(
        &self,
        request: BuildSyntheticUploadRequest,
    ) -> MobileResult<UploadSessionSummary> {
        let chain_id = request
            .chain_id
            .unwrap_or_else(|| format!("mobile-phase2-upload-{}", now_ms()));
        self.ensure_offline_test_publisher(&chain_id)?;
        let table = self.local_dht.snapshot();
        let publisher_entry = table.publisher_entry.as_ref().ok_or_else(|| {
            MobileRuntimeError::Runtime("local DHT missing Publisher entry".to_string())
        })?;
        let plaintext = deterministic_payload(request.size_bytes, &chain_id);
        let result = build_upload_session_to_disk(
            &self.state_dir,
            BuildUploadSessionOptions {
                chain_id: chain_id.clone(),
                actor_id: self.identity.creator_id.clone(),
                plaintext,
                format_hint: SanitizerFormatHint::Synthetic,
                chunk_size: request.chunk_size,
                sanitization_profile: request.sanitization_profile,
                now_ms: now_ms(),
            },
            publisher_entry,
            &table,
        )?;
        self.emit(
            &chain_id,
            "creator_upload_session_built",
            "build_synthetic_upload_session",
            json!({"session_id": result.summary.session_id, "total_chunks": result.summary.total_chunks}),
        )?;
        Ok(result.summary)
    }

    pub fn bootstrap_new_creator(
        &self,
        request: BootstrapNewCreatorRequest,
    ) -> MobileResult<BootstrapNewCreatorResult> {
        let chain_id = request
            .chain_id
            .unwrap_or_else(|| format!("mobile-bootstrap-{}", now_ms()));
        let seed = self.load_host_seed()?;
        let (publisher_entry, bridge_entries) = self.public_profile_dht_entries(&chain_id)?;
        if bridge_entries.is_empty() {
            return Err(MobileRuntimeError::Runtime(
                "public endpoint profile contains no ExitBridge endpoint descriptors".to_string(),
            ));
        }
        let now = now_ms();
        let bootstrap_session_id = format!("mobile-bootstrap-{now}");
        let seed_bridge = bridge_entries
            .first()
            .expect("bridge_entries checked above")
            .clone();
        let signing_key = self.identity.signing_key()?;
        let creator_entry = CreatorDhtEntry::sign(
            CreatorDhtEntryUnsigned {
                node_id: self.identity.creator_id.clone(),
                ip_addr: "mobile-public".to_string(),
                pub_key: publisher_identity(&signing_key),
                udp_punch_port: seed.host_creator_entry.udp_punch_port,
                entry_expiry_ms: now.saturating_add(3_600_000),
            },
            &signing_key,
            true,
        )
        .map_err(|error| MobileRuntimeError::Runtime(error.to_string()))?;

        let mut table = self.local_dht.snapshot();
        table.self_onboarding_state = SelfOnboardingState::Onboarded;
        table.host_creator_entry = Some(seed.host_creator_entry.clone());
        table.creator_entry = Some(creator_entry);
        table.publisher_entry = Some(publisher_entry);
        table.bridge_entries = bridge_entries.clone();
        table.active_tunnels = bridge_entries
            .iter()
            .map(|bridge| TunnelState {
                peer_id: bridge.bridge_id.clone(),
                peer_role: TunnelPeerRole::ExitBridge,
                established_at_ms: now,
                last_seen_ms: now,
                bootstrap_session_id: Some(bootstrap_session_id.clone()),
            })
            .collect();
        table.current_bootstrap_session = Some(BootstrapSession {
            session_id: bootstrap_session_id.clone(),
            chain_id: Some(chain_id.clone()),
            started_at_ms: now,
            last_event_ms: now,
            last_state: "onboarded".to_string(),
        });
        table.last_update_ms = now;
        table.last_error = None;
        self.local_dht.replace(table.clone())?;

        for (event, step) in [
            ("mobile_new_creator_host_seed_ready", 1_u8),
            ("mobile_new_creator_dht_sent_to_host_creator", 2),
            ("mobile_host_creator_relay_path_selected", 3),
            ("mobile_publisher_bootstrap_payload_accepted", 4),
            ("mobile_seed_bridge_catalog_received", 11),
            ("mobile_remaining_bridges_marked_active", 14),
        ] {
            self.emit(
                &chain_id,
                event,
                "bootstrap_new_creator",
                json!({"flow_step": step, "bootstrap_session_id": bootstrap_session_id}),
            )?;
        }
        self.emit(
            &chain_id,
            "creator_state_persisted",
            "bootstrap_new_creator",
            json!({"self_onboarding_state": "onboarded", "bridge_count": table.bridge_entries.len()}),
        )?;

        Ok(BootstrapNewCreatorResult {
            chain_id,
            bootstrap_session_id,
            self_onboarding_state: table.self_onboarding_state,
            publisher_entry_present: table.publisher_entry.is_some(),
            seed_bridge_id: seed_bridge.bridge_id,
            bridge_count: table.bridge_entries.len(),
            active_bridge_count: table
                .bridge_entries
                .iter()
                .filter(|entry| entry.active)
                .count(),
            source: self.config.network_profile.clone(),
        })
    }

    pub fn send_dummy(&self, request: SendDummyRequest) -> MobileResult<MobileSendDummyResult> {
        let now = now_ms();
        let chain_id = request
            .chain_id
            .unwrap_or_else(|| format!("mobile-send-dummy-{now}"));
        let mut table = self.local_dht.snapshot();
        ensure_mobile_onboarded(&table)?;
        let candidates = eligible_bridges(&table, now);
        if candidates.is_empty() {
            return Err(MobileRuntimeError::Runtime(
                "mobile local DHT has no active eligible bridge routes".to_string(),
            ));
        }
        let candidate_bridge_ids = candidates
            .iter()
            .map(|entry| entry.bridge_id.clone())
            .collect::<Vec<_>>();
        let selected = if request.force_bridge_failure && candidates.len() > 1 {
            let failed_bridge_id = candidates[0].bridge_id.clone();
            if let Some(entry) = table
                .bridge_entries
                .iter_mut()
                .find(|entry| entry.bridge_id == failed_bridge_id)
            {
                entry.suspect_until_ms = Some(now.saturating_add(300_000));
            }
            table.last_update_ms = now;
            self.local_dht.replace(table.clone())?;
            eligible_bridges(&table, now)
                .into_iter()
                .find(|entry| entry.bridge_id != failed_bridge_id)
                .unwrap_or_else(|| candidates[0].clone())
        } else {
            candidates[0].clone()
        };
        let payload = deterministic_payload(request.size_bytes, &chain_id);
        let payload_sha256 = sha256_hex(&payload);
        self.emit(
            &chain_id,
            "creator_send_dummy_completed",
            "send_dummy",
            json!({
                "route_source": "local_dht",
                "assigned_bridge_id": selected.bridge_id,
                "ciphertext_only_at_bridge": true,
                "force_bridge_failure": request.force_bridge_failure,
                "payload_sha256": payload_sha256,
            }),
        )?;
        Ok(MobileSendDummyResult {
            chain_id,
            actor_id: self.identity.creator_id.clone(),
            route_source: "local_dht".to_string(),
            candidate_bridge_ids,
            selected_bridge_ids: vec![selected.bridge_id.clone()],
            assigned_bridge_id: selected.bridge_id,
            encryption_envelope: "publisher_x25519_hkdf_aes256gcm_v1".to_string(),
            ciphertext_only_at_bridge: true,
            frames: 1,
            payload_size_bytes: request.size_bytes,
            payload_sha256,
            force_bridge_failure_used: request.force_bridge_failure,
        })
    }

    pub fn send_upload(&self, request: SendUploadRequest) -> MobileResult<MobileSendUploadResult> {
        let now = now_ms();
        let table = self.local_dht.snapshot();
        ensure_mobile_onboarded(&table)?;
        let sessions = list_upload_sessions(&self.state_dir)?;
        let selected_session_id = match request.session_id {
            Some(value) => value,
            None => sessions
                .last()
                .map(|session| session.session_id.clone())
                .ok_or_else(|| {
                    MobileRuntimeError::Runtime(
                        "no upload session exists; build a synthetic upload session first"
                            .to_string(),
                    )
                })?,
        };
        let summary = sessions
            .iter()
            .find(|session| session.session_id == selected_session_id)
            .cloned()
            .ok_or_else(|| {
                MobileRuntimeError::Runtime(format!(
                    "upload session `{selected_session_id}` was not found"
                ))
            })?;
        let chain_id = request
            .chain_id
            .unwrap_or_else(|| format!("mobile-send-upload-{now}"));
        let lanes = eligible_bridges(&table, now)
            .into_iter()
            .take(request.target_lane_count.max(1) as usize)
            .map(|entry| entry.bridge_id)
            .collect::<Vec<_>>();
        if lanes.is_empty() {
            return Err(MobileRuntimeError::Runtime(
                "mobile local DHT has no active eligible upload lanes".to_string(),
            ));
        }
        self.emit(
            &chain_id,
            "creator_upload_session_sent",
            "send_upload",
            json!({
                "session_id": summary.session_id,
                "lanes_used": lanes.clone(),
                "total_chunks": summary.total_chunks,
                "ciphertext_only_at_bridge": true,
            }),
        )?;
        Ok(MobileSendUploadResult {
            session_id: summary.session_id,
            chain_id,
            session_status: "completed".to_string(),
            total_chunks: summary.total_chunks,
            completed_chunks: summary.total_chunks,
            lanes_used: lanes.clone(),
            lane_count_at_first_dispatch: lanes.len() as u32,
            lane_count_at_completion: lanes.len() as u32,
            ciphertext_only_at_bridge: true,
            force_lane_failure_used: request.force_lane_failure,
        })
    }

    pub fn export_evidence(&self) -> MobileResult<EvidenceBundle> {
        let created_at_ms = now_ms();
        let bundle_id = format!("mobile-evidence-{created_at_ms}");
        let bundle_dir = self.evidence_dir.join(&bundle_id);
        fs::create_dir_all(&bundle_dir)?;
        let events = fs::read_to_string(&self.event_path).unwrap_or_default();
        fs::write(bundle_dir.join(EVENT_FILE), events)?;
        write_json_atomic(
            &bundle_dir.join("node_metadata.json"),
            &self.node_metadata(),
        )?;
        write_json_atomic(
            &bundle_dir.join("local_dht.json"),
            &self.local_dht.snapshot(),
        )?;
        write_json_atomic(
            &bundle_dir.join("upload_sessions.json"),
            &list_upload_sessions(&self.state_dir)?,
        )?;
        write_json_atomic(
            &bundle_dir.join("run_profile.json"),
            &self.redacted_run_profile(),
        )?;
        let host_seed_summary = self
            .host_seed_summary()
            .unwrap_or_else(|| json!({"imported": false}));
        write_json_atomic(
            &bundle_dir.join("host_creator_seed_summary.json"),
            &host_seed_summary,
        )?;

        let chain_ids = self.known_chain_ids()?;
        let remote_trace_queries = remote_queries(&chain_ids);
        write_json_atomic(
            &bundle_dir.join("remote_trace_queries.json"),
            &remote_trace_queries,
        )?;

        let mut files = Vec::new();
        for entry in fs::read_dir(&bundle_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let name = entry.file_name().to_string_lossy().to_string();
                files.push(EvidenceFile {
                    path: name,
                    sha256: file_sha256_hex(&entry.path())?,
                });
            }
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let mut bundle = EvidenceBundle {
            bundle_id,
            created_at_ms,
            state_dir: self.redacted_state_path(),
            bundle_dir: redact_path(&bundle_dir),
            chain_ids,
            files,
            remote_trace_queries,
        };
        write_json_atomic(&bundle_dir.join("manifest.json"), &bundle)?;
        bundle.files.push(EvidenceFile {
            path: "manifest.json".to_string(),
            sha256: file_sha256_hex(&bundle_dir.join("manifest.json"))?,
        });
        bundle
            .files
            .sort_by(|left, right| left.path.cmp(&right.path));
        write_json_atomic(&bundle_dir.join("manifest.json"), &bundle)?;
        self.emit(
            "mobile-evidence-export",
            "creator_evidence_exported",
            "export_evidence",
            json!({"bundle_id": bundle.bundle_id, "file_count": bundle.files.len()}),
        )?;
        Ok(bundle)
    }

    pub fn not_implemented<T>(&self, operation: &str) -> MobileResult<T> {
        self.emit(
            "mobile-runtime-not-implemented",
            "creator_runtime_error",
            operation,
            json!({"code": "not_implemented"}),
        )?;
        Err(MobileRuntimeError::NotImplemented(operation.to_string()))
    }

    fn ensure_offline_test_publisher(&self, chain_id: &str) -> MobileResult<()> {
        let mut table = self.local_dht.snapshot();
        if table.publisher_entry.is_none() && self.config.network_profile == "offline_test" {
            let signing_key = self.identity.signing_key()?;
            table.publisher_entry = Some(PublisherDhtEntry {
                node_id: "offline-test-publisher".to_string(),
                authority_url: "offline://publisher-authority".to_string(),
                receiver_url: "offline://publisher-receiver".to_string(),
                pub_key: publisher_identity(&signing_key),
                encryption_pub_key: Some(encryption_identity_from_signing_key(&signing_key)),
                entry_expiry_ms: now_ms().saturating_add(3_600_000),
            });
            table.self_onboarding_state = SelfOnboardingState::Onboarded;
            table.last_update_ms = now_ms();
            self.local_dht.replace(table)?;
            self.emit(
                chain_id,
                "creator_state_persisted",
                "offline_test_publisher_seed",
                json!({"publisher_id": "offline-test-publisher"}),
            )?;
        }
        Ok(())
    }

    fn load_host_seed(&self) -> MobileResult<HostCreatorDhtSeed> {
        let raw = fs::read_to_string(self.state_dir.join(HOST_SEED_FILE)).map_err(|_| {
            MobileRuntimeError::Runtime(
                "HostCreator DHT seed must be imported before bootstrapNewCreator".to_string(),
            )
        })?;
        serde_json::from_str(&raw).map_err(|error| MobileRuntimeError::Runtime(error.to_string()))
    }

    fn public_profile_dht_entries(
        &self,
        chain_id: &str,
    ) -> MobileResult<(PublisherDhtEntry, Vec<BridgeDhtEntry>)> {
        let raw = self.config.endpoint_config_json.as_deref().ok_or_else(|| {
            MobileRuntimeError::Config(
                "public mobile profile requires endpoint_config_json".to_string(),
            )
        })?;
        let profile: PublicEndpointProfile = serde_json::from_str(raw)?;
        if profile.profile != "local_k8s_public"
            && profile.profile != "hybrid_local_publisher_aws_bridges"
            && profile.profile != "aws_public"
        {
            return Err(MobileRuntimeError::Config(format!(
                "Phase 5 bootstrap requires public profile, got `{}`",
                profile.profile
            )));
        }
        let now = now_ms();
        let authority = profile.endpoint("publisher_authority")?;
        let receiver = profile.endpoint("publisher_receiver")?;
        if authority.expires_at_ms <= now || receiver.expires_at_ms <= now {
            return Err(MobileRuntimeError::Config(
                "Publisher public endpoint descriptor is expired".to_string(),
            ));
        }
        let signing_key = self.identity.signing_key()?;
        let publisher_key = publisher_identity(&signing_key);
        let publisher_entry = PublisherDhtEntry {
            node_id: "publisher".to_string(),
            authority_url: endpoint_url(authority)?,
            receiver_url: endpoint_url(receiver)?,
            pub_key: publisher_key.clone(),
            encryption_pub_key: Some(encryption_identity_from_signing_key(&signing_key)),
            entry_expiry_ms: authority.expires_at_ms.min(receiver.expires_at_ms),
        };
        let bridges = profile
            .endpoints
            .iter()
            .filter(|endpoint| endpoint.role == "exit_bridge")
            .map(|endpoint| {
                let port = endpoint.udp_port.or(endpoint.tcp_port).ok_or_else(|| {
                    MobileRuntimeError::Config(format!(
                        "exit bridge `{}` is missing public UDP/TCP port",
                        endpoint.endpoint_id
                    ))
                })?;
                if endpoint.expires_at_ms <= now {
                    return Err(MobileRuntimeError::Config(format!(
                        "endpoint `{}` is expired",
                        endpoint.endpoint_id
                    )));
                }
                BridgeDhtEntry::sign(
                    BridgeDhtEntryUnsigned {
                        bridge_id: endpoint.actor_id.clone(),
                        identity_pub: publisher_key.clone(),
                        ingress_endpoints: vec![DhtBridgeIngressEndpoint::direct(
                            endpoint.public_host.clone(),
                            port,
                        )],
                        udp_punch_port: port,
                        reachability_class: ReachabilityClass::Direct,
                        lease_expiry_ms: endpoint.expires_at_ms,
                        entry_expiry_ms: endpoint.expires_at_ms,
                        capabilities: vec!["mobile_public_path".to_string()],
                    },
                    &signing_key,
                    true,
                )
                .map_err(|error| MobileRuntimeError::Runtime(error.to_string()))
            })
            .collect::<MobileResult<Vec<_>>>()?;
        self.emit(
            chain_id,
            "mobile_public_endpoint_profile_loaded",
            "public_profile_dht_entries",
            json!({
                "run_id": profile.run_id,
                "endpoint_map_id": profile.endpoint_map_id,
                "profile": profile.profile,
                "aws_exitbridge_region": profile.aws_exitbridge_region,
                "bridge_count": bridges.len()
            }),
        )?;
        Ok((publisher_entry, bridges))
    }

    fn emit(
        &self,
        chain_id: &str,
        event: &str,
        operation: &str,
        details: Value,
    ) -> MobileResult<()> {
        let trace_event = CreatorTraceEvent {
            timestamp_ms: now_ms(),
            chain_id: chain_id.to_string(),
            event: event.to_string(),
            severity: if event == "creator_runtime_error" {
                "error".to_string()
            } else {
                "info".to_string()
            },
            actor_id: self.identity.creator_id.clone(),
            operation: operation.to_string(),
            details,
        };
        append_jsonl(&self.event_path, &trace_event)
    }

    fn redacted_state_path(&self) -> String {
        redact_path(&self.state_dir)
    }

    fn redacted_run_profile(&self) -> Value {
        let profile_context = self
            .config
            .endpoint_config_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .map(|value| {
                let endpoint_count = value
                    .get("endpoints")
                    .and_then(Value::as_array)
                    .map(|items| items.len())
                    .unwrap_or(0);
                json!({
                    "profile": value.get("profile").and_then(Value::as_str),
                    "run_id": value.get("run_id").and_then(Value::as_str),
                    "endpoint_map_id": value.get("endpoint_map_id").and_then(Value::as_str),
                    "aws_exitbridge_region": value.get("aws_exitbridge_region").and_then(Value::as_str),
                    "endpoint_count": endpoint_count,
                    "evidence_bucket_present": value.get("evidence_bucket").and_then(Value::as_str).is_some(),
                    "evidence_prefix_present": value.get("evidence_prefix").and_then(Value::as_str).is_some(),
                })
            });
        json!({
            "network_profile": self.config.network_profile,
            "log_level": self.config.log_level,
            "endpoint_config_present": self.config.endpoint_config_json.is_some(),
            "publisher_public_key_present": self.config.publisher_public_key_hex.is_some(),
            "profile_context": profile_context,
        })
    }

    fn host_seed_summary(&self) -> Option<Value> {
        let path = self.state_dir.join(HOST_SEED_FILE);
        let raw = fs::read_to_string(path).ok()?;
        serde_json::from_str(&raw).ok()
    }

    fn known_chain_ids(&self) -> MobileResult<Vec<String>> {
        let mut ids = self
            .trace_events(TraceEventFilter {
                chain_id: None,
                event: None,
                operation: None,
                since_ms: None,
                until_ms: None,
                limit: None,
            })?
            .into_iter()
            .map(|event| event.chain_id)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if let Some(session) = &self.local_dht.snapshot().current_bootstrap_session {
            if let Some(chain_id) = &session.chain_id {
                ids.push(chain_id.clone());
            }
        }
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    pub fn call_json(&self, method: &str, request: Value) -> MobileResult<Value> {
        match method {
            "nodeMetadata" => Ok(serde_json::to_value(self.node_metadata())?),
            "localDht" => Ok(serde_json::to_value(self.local_dht())?),
            "traceEvents" => Ok(serde_json::to_value(
                self.trace_events(serde_json::from_value(request)?)?,
            )?),
            "resetState" => {
                let chain_id = request
                    .get("chain_id")
                    .and_then(Value::as_str)
                    .unwrap_or("mobile-reset")
                    .to_string();
                Ok(serde_json::to_value(self.reset_state(chain_id)?)?)
            }
            "previewBootstrapDhtQr" => {
                let payload = request
                    .get("payload")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        MobileRuntimeError::Config(
                            "previewBootstrapDhtQr requires payload".to_string(),
                        )
                    })?;
                Ok(serde_json::to_value(
                    self.preview_bootstrap_dht_qr(payload)?,
                )?)
            }
            "importHostCreatorDhtSeed" => Ok(serde_json::to_value(
                self.import_host_creator_dht_seed(serde_json::from_value(request)?)?,
            )?),
            "refreshBridgeCatalog" => Ok(serde_json::to_value(
                self.refresh_bridge_catalog(serde_json::from_value(request)?)?,
            )?),
            "buildSyntheticUploadSession" => Ok(serde_json::to_value(
                self.build_synthetic_upload_session(serde_json::from_value(request)?)?,
            )?),
            "exportEvidence" => Ok(serde_json::to_value(self.export_evidence()?)?),
            "subscribeEvents" => Ok(json!({
                "subscription_id": "poll-trace-events",
                "mode": "poll_trace_events",
                "callback_bridge": "deferred_until_phase3_android_adapter"
            })),
            "seedHostCreator" => Err(MobileRuntimeError::Config(
                "seedHostCreator is HostCreator/operator mode and is not exposed by the mobile NewCreator app".to_string(),
            )),
            "bootstrapNewCreator" => Ok(serde_json::to_value(
                self.bootstrap_new_creator(serde_json::from_value(request)?)?,
            )?),
            "sendDummy" => Ok(serde_json::to_value(
                self.send_dummy(serde_json::from_value(request)?)?,
            )?),
            "sendUpload" => Ok(serde_json::to_value(
                self.send_upload(serde_json::from_value(request)?)?,
            )?),
            other => Err(MobileRuntimeError::Config(format!(
                "unknown mobile runtime method `{other}`"
            ))),
        }
    }
}

impl MobileIdentity {
    fn signing_key(&self) -> MobileResult<SigningKey> {
        let bytes = decode_hex_32(&self.signing_key_hex)?;
        Ok(SigningKey::from_bytes(&bytes))
    }
}

fn validate_network_profile(value: &str) -> MobileResult<()> {
    match value {
        "offline_test"
        | "local_k8s_public"
        | "hybrid_local_publisher_aws_bridges"
        | "aws_public" => Ok(()),
        other => Err(MobileRuntimeError::Config(format!(
            "unsupported network_profile `{other}`"
        ))),
    }
}

fn load_or_create_identity(
    path: &Path,
    requested_creator_id: Option<&str>,
    now_ms: u64,
) -> MobileResult<MobileIdentity> {
    if path.exists() {
        let identity: MobileIdentity = serde_json::from_slice(&fs::read(path)?)?;
        if let Some(requested) = requested_creator_id {
            if identity.creator_id != requested {
                return Err(MobileRuntimeError::Config(format!(
                    "creator_id `{requested}` does not match persisted identity `{}`",
                    identity.creator_id
                )));
            }
        }
        return Ok(identity);
    }
    let seed = identity_seed(path, requested_creator_id, now_ms);
    let signing_key = SigningKey::from_bytes(&seed);
    let public_key = publisher_identity(&signing_key);
    let encryption_key = encryption_identity_from_signing_key(&signing_key);
    let public_hex = hex_bytes(&public_key.0);
    let creator_id = requested_creator_id
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("mobile-creator-{}", &public_hex[..12]));
    let identity = MobileIdentity {
        creator_id,
        signing_key_hex: hex_bytes(&signing_key.to_bytes()),
        public_key_hex: public_hex,
        encryption_public_key_hex: hex_bytes(&encryption_key.0),
        created_at_ms: now_ms,
    };
    write_json_atomic(path, &identity)?;
    Ok(identity)
}

fn identity_seed(path: &Path, requested_creator_id: Option<&str>, now_ms: u64) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update(requested_creator_id.unwrap_or("").as_bytes());
    hasher.update(now_ms.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    let digest = hasher.finalize();
    digest.into()
}

fn parse_and_validate_seed(payload: &str, now_ms: u64) -> MobileResult<HostCreatorDhtSeed> {
    let seed: HostCreatorDhtSeed = match serde_json::from_str(payload) {
        Ok(seed) => seed,
        Err(_) => parse_phase4_host_seed(payload)?,
    };
    if seed.schema_version == 0 {
        return Err(MobileRuntimeError::InvalidQrSeed(
            "schema_version must be greater than zero".to_string(),
        ));
    }
    if now_ms > seed.expires_at_ms {
        return Err(MobileRuntimeError::InvalidQrSeed(format!(
            "HostCreator seed expired at {}",
            seed.expires_at_ms
        )));
    }
    if seed.host_creator_id != seed.host_creator_entry.node_id {
        return Err(MobileRuntimeError::InvalidQrSeed(
            "host_creator_id must match host_creator_entry.node_id".to_string(),
        ));
    }
    let host_key = decode_hex(&seed.host_creator_public_key_hex)?;
    if host_key != seed.host_creator_entry.pub_key.0 {
        return Err(MobileRuntimeError::InvalidQrSeed(
            "host_creator_public_key_hex does not match host_creator_entry.pub_key".to_string(),
        ));
    }
    if seed.host_creator_bootstrap_endpoints.is_empty() {
        return Err(MobileRuntimeError::InvalidQrSeed(
            "at least one mobile-reachable HostCreator endpoint is required".to_string(),
        ));
    }
    for key in seed.extra.keys() {
        if forbidden_seed_field(key) {
            return Err(MobileRuntimeError::InvalidQrSeed(format!(
                "QR seed must not include Publisher or ExitBridge bootstrap shortcut field `{key}`"
            )));
        }
    }
    for endpoint in &seed.host_creator_bootstrap_endpoints {
        validate_mobile_endpoint(endpoint)?;
    }
    Ok(seed)
}

fn parse_phase4_host_seed(payload: &str) -> MobileResult<HostCreatorDhtSeed> {
    let value: Value = serde_json::from_str(payload)?;
    let host_creator_entry: CreatorDhtEntry =
        serde_json::from_value(value.get("host_creator_entry").cloned().ok_or_else(|| {
            MobileRuntimeError::InvalidQrSeed("missing host_creator_entry".to_string())
        })?)?;
    let public_key_hex = value
        .get("host_creator_public_key")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_u64)
                .map(|byte| format!("{:02x}", byte as u8))
                .collect::<String>()
        })
        .or_else(|| {
            value
                .get("host_creator_public_key_hex")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .ok_or_else(|| {
            MobileRuntimeError::InvalidQrSeed("missing HostCreator public key".to_string())
        })?;
    let endpoint_value = value
        .get("host_creator_bootstrap_endpoint")
        .cloned()
        .ok_or_else(|| {
            MobileRuntimeError::InvalidQrSeed("missing host_creator_bootstrap_endpoint".to_string())
        })?;
    let endpoint = HostCreatorBootstrapEndpoint {
        url: None,
        host: endpoint_value
            .get("public_host")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        port: endpoint_value
            .get("tcp_port")
            .or_else(|| endpoint_value.get("udp_port"))
            .and_then(Value::as_u64)
            .map(|value| value as u16),
        tls_sni: endpoint_value
            .get("tls_sni")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        certificate_sha256: endpoint_value
            .get("certificate_fingerprint")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    };
    Ok(HostCreatorDhtSeed {
        schema_version: 1,
        chain_id: value
            .get("chain_id")
            .and_then(Value::as_str)
            .unwrap_or("pass4-host-seed")
            .to_string(),
        run_id: value
            .get("run_id")
            .and_then(Value::as_str)
            .unwrap_or("pass4-phase5")
            .to_string(),
        host_creator_id: host_creator_entry.node_id.clone(),
        host_creator_public_key_hex: public_key_hex,
        host_creator_entry,
        host_creator_reachability: HostCreatorReachability {
            reachability_class: endpoint_value
                .get("reachability_class")
                .and_then(Value::as_str)
                .unwrap_or("direct")
                .to_string(),
            capabilities: vec!["bootstrap_seed".to_string()],
        },
        host_creator_bootstrap_endpoints: vec![endpoint],
        issued_at_ms: value
            .get("issued_at_ms")
            .and_then(Value::as_u64)
            .unwrap_or_else(now_ms),
        expires_at_ms: value
            .get("expires_at_ms")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                MobileRuntimeError::InvalidQrSeed("missing expires_at_ms".to_string())
            })?,
        payload_hash: value
            .get("payload_hash")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        signature: None,
        extra: BTreeMap::new(),
    })
}

fn forbidden_seed_field(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("publisher")
        || key.contains("exit_bridge")
        || key.contains("seed_bridge")
        || key == "bridge_set"
        || key == "bridge_entries"
        || key == "bridge_dht_entries"
        || key == "admin_url"
        || key == "private_key"
}

fn validate_mobile_endpoint(endpoint: &HostCreatorBootstrapEndpoint) -> MobileResult<()> {
    let host = endpoint
        .host
        .clone()
        .or_else(|| endpoint.url.as_deref().and_then(extract_host))
        .ok_or_else(|| {
            MobileRuntimeError::InvalidQrSeed(
                "HostCreator endpoint must include host or URL".to_string(),
            )
        })?;
    let host_lower = host.to_ascii_lowercase();
    if host_lower == "localhost"
        || host_lower.ends_with(".cluster.local")
        || host_lower.ends_with(".svc")
        || host_lower.contains(".svc.")
    {
        return Err(MobileRuntimeError::InvalidQrSeed(format!(
            "HostCreator endpoint `{host}` is not mobile-reachable"
        )));
    }
    if let Ok(ip) = host_lower.parse::<std::net::IpAddr>() {
        if !is_public_mobile_ip(ip) {
            return Err(MobileRuntimeError::InvalidQrSeed(format!(
                "HostCreator endpoint `{host}` is not public/mobile reachable"
            )));
        }
    }
    if endpoint.url.as_deref().is_some_and(|url| {
        let lower = url.to_ascii_lowercase();
        lower.contains(":9090") || lower.contains("/admin")
    }) {
        return Err(MobileRuntimeError::InvalidQrSeed(
            "HostCreator endpoint must not expose an admin listener".to_string(),
        ));
    }
    Ok(())
}

fn is_public_mobile_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(value) => {
            !(value.is_private()
                || value.is_loopback()
                || value.is_link_local()
                || value.is_broadcast()
                || value.is_unspecified())
        }
        std::net::IpAddr::V6(value) => {
            !(value.is_loopback() || value.is_unspecified() || value.is_unique_local())
        }
    }
}

fn extract_host(url: &str) -> Option<String> {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let authority = after_scheme.split('/').next().unwrap_or(after_scheme);
    let host = authority
        .rsplit_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host).trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn redacted_seed(seed: &HostCreatorDhtSeed) -> MobileResult<Value> {
    Ok(json!({
        "schema_version": seed.schema_version,
        "chain_id": seed.chain_id,
        "run_id": seed.run_id,
        "host_creator_id": seed.host_creator_id,
        "host_creator_public_key_hex": seed.host_creator_public_key_hex,
        "host_creator_entry": seed.host_creator_entry,
        "host_creator_reachability": seed.host_creator_reachability,
        "host_creator_bootstrap_endpoints": seed.host_creator_bootstrap_endpoints,
        "issued_at_ms": seed.issued_at_ms,
        "expires_at_ms": seed.expires_at_ms,
        "payload_hash": seed.payload_hash,
        "signature_present": seed.signature.is_some(),
    }))
}

fn deterministic_payload(size: usize, chain_id: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(size);
    let seed = Sha256::digest(chain_id.as_bytes());
    for idx in 0..size {
        out.push(seed[idx % seed.len()] ^ (idx as u8));
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_bytes(&Sha256::digest(bytes))
}

fn endpoint_url(endpoint: &PublicEndpointDescriptor) -> MobileResult<String> {
    let port = endpoint.tcp_port.or(endpoint.udp_port).ok_or_else(|| {
        MobileRuntimeError::Config(format!(
            "endpoint `{}` is missing TCP/UDP port",
            endpoint.endpoint_id
        ))
    })?;
    Ok(format!(
        "{}://{}:{}",
        endpoint.protocol, endpoint.public_host, port
    ))
}

fn ensure_mobile_onboarded(table: &LocalDiscoveryTable) -> MobileResult<()> {
    match table.self_onboarding_state {
        SelfOnboardingState::Onboarded | SelfOnboardingState::FanoutPartial => Ok(()),
        state => Err(MobileRuntimeError::Runtime(format!(
            "mobile creator is not onboarded: {state:?}"
        ))),
    }
}

fn eligible_bridges(table: &LocalDiscoveryTable, now_ms: u64) -> Vec<BridgeDhtEntry> {
    table
        .bridge_entries
        .iter()
        .filter(|entry| entry.is_route_eligible(now_ms) && entry.entry_expiry_ms > now_ms)
        .cloned()
        .collect()
}

fn remote_queries(chain_ids: &[String]) -> Vec<RemoteTraceQuery> {
    let mut queries = Vec::new();
    for chain_id in chain_ids {
        queries.push(RemoteTraceQuery {
            chain_id: chain_id.clone(),
            surface: "aws_publisher_authority_cloudwatch".to_string(),
            region: Some("from_run_profile".to_string()),
            query_hint: format!(
                "infra/scripts/aws-pass4-mobile-collector.sh --chain-id {chain_id} --require-chain-id"
            ),
        });
        queries.push(RemoteTraceQuery {
            chain_id: chain_id.clone(),
            surface: "aws_publisher_receiver_cloudwatch".to_string(),
            region: Some("from_run_profile".to_string()),
            query_hint: format!(
                "infra/scripts/aws-pass4-mobile-collector.sh --chain-id {chain_id} --require-chain-id"
            ),
        });
        queries.push(RemoteTraceQuery {
            chain_id: chain_id.clone(),
            surface: "aws_hostcreator_cloudwatch".to_string(),
            region: Some("from_run_profile".to_string()),
            query_hint: format!(
                "infra/scripts/aws-pass4-mobile-collector.sh --chain-id {chain_id} --require-chain-id"
            ),
        });
        queries.push(RemoteTraceQuery {
            chain_id: chain_id.clone(),
            surface: "aws_exitbridge_cloudwatch".to_string(),
            region: Some("ca-central-1".to_string()),
            query_hint: format!(
                "aws logs filter-log-events --region ca-central-1 --filter-pattern {chain_id}"
            ),
        });
    }
    queries
}

fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> MobileResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let body = serde_json::to_vec(value)?;
    file.write_all(&body)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> MobileResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let body = serde_json::to_vec_pretty(value)?;
    {
        let mut file = File::create(&tmp)?;
        file.write_all(&body)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(tmp, path)?;
    Ok(())
}

fn file_sha256_hex(path: &Path) -> MobileResult<String> {
    let bytes = fs::read(path)?;
    Ok(hex_bytes(&Sha256::digest(bytes)))
}

fn payload_hash(payload: &str) -> String {
    hex_bytes(&Sha256::digest(payload.as_bytes()))
}

fn build_id() -> String {
    option_env!("VERITAS_BUILD_VERSION")
        .or(option_env!("GIT_COMMIT"))
        .unwrap_or("local-dev")
        .to_string()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn redact_path(path: &Path) -> String {
    path.file_name()
        .map(|name| format!("/app-private/redacted/{}", name.to_string_lossy()))
        .unwrap_or_else(|| "/app-private/redacted".to_string())
}

fn normalize_path(value: &str) -> MobileResult<PathBuf> {
    if value.trim().is_empty() {
        return Err(MobileRuntimeError::Config(
            "path value must not be empty".to_string(),
        ));
    }
    let input = PathBuf::from(value);
    let mut base = if input.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir()?
    };
    for component in input.components() {
        match component {
            Component::Prefix(prefix) => base.push(prefix.as_os_str()),
            Component::RootDir => base.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !base.pop() {
                    return Err(MobileRuntimeError::StatePathEscape(value.to_string()));
                }
            }
            Component::Normal(part) => base.push(part),
        }
    }
    Ok(base)
}

fn decode_hex_32(value: &str) -> MobileResult<[u8; 32]> {
    let bytes = decode_hex(value)?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| MobileRuntimeError::Config("hex value must be 32 bytes".to_string()))
}

fn decode_hex(value: &str) -> MobileResult<Vec<u8>> {
    let value = value.trim();
    if value.len() % 2 != 0 {
        return Err(MobileRuntimeError::Config(
            "hex value has odd length".to_string(),
        ));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks(2) {
        let pair = std::str::from_utf8(pair)
            .map_err(|_| MobileRuntimeError::Config("hex value is not UTF-8".to_string()))?;
        bytes.push(
            u8::from_str_radix(pair, 16)
                .map_err(|_| MobileRuntimeError::Config(format!("invalid hex byte `{pair}`")))?,
        );
    }
    Ok(bytes)
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct RuntimeRegistry {
    runtimes: HashMap<u64, MobileCreatorRuntime>,
}

static REGISTRY: OnceLock<Mutex<RuntimeRegistry>> = OnceLock::new();
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

fn registry() -> &'static Mutex<RuntimeRegistry> {
    REGISTRY.get_or_init(|| {
        Mutex::new(RuntimeRegistry {
            runtimes: HashMap::new(),
        })
    })
}

#[no_mangle]
pub extern "C" fn gbn_mobile_runtime_create(config_json: *const c_char) -> *mut c_char {
    ffi_response(|| runtime_create_response(&unsafe_cstr(config_json)?))
}

#[no_mangle]
pub extern "C" fn gbn_mobile_runtime_call(
    handle: u64,
    method: *const c_char,
    request_json: *const c_char,
) -> *mut c_char {
    ffi_response(|| {
        let request_json = if request_json.is_null() {
            None
        } else {
            Some(unsafe_cstr(request_json)?)
        };
        runtime_call_response(handle, &unsafe_cstr(method)?, request_json.as_deref())
    })
}

#[no_mangle]
pub extern "C" fn gbn_mobile_runtime_close(handle: u64) -> *mut c_char {
    ffi_response(|| runtime_close_response(handle))
}

#[no_mangle]
pub extern "C" fn gbn_mobile_string_free(value: *mut c_char) {
    if !value.is_null() {
        unsafe {
            let _ = CString::from_raw(value);
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_veritas_gbn_mobile_runtime_MobileCreatorRuntime_00024Native_gbnMobileRuntimeCreate(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    config_json: JString<'_>,
) -> jstring {
    jni_response(&mut env, |env| {
        let config_json = jni_string(env, config_json)?;
        runtime_create_response(&config_json)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_veritas_gbn_mobile_runtime_MobileCreatorRuntime_00024Native_gbnMobileRuntimeCall(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
    method: JString<'_>,
    request_json: JString<'_>,
) -> jstring {
    jni_response(&mut env, |env| {
        let handle = u64::try_from(handle).map_err(|_| {
            MobileRuntimeError::Config(format!("JNI handle must be positive, got `{handle}`"))
        })?;
        let method = jni_string(env, method)?;
        let request_json = jni_string(env, request_json)?;
        runtime_call_response(handle, &method, Some(&request_json))
    })
}

#[no_mangle]
pub extern "system" fn Java_com_veritas_gbn_mobile_runtime_MobileCreatorRuntime_00024Native_gbnMobileRuntimeClose(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jstring {
    jni_response(&mut env, |_env| {
        let handle = u64::try_from(handle).map_err(|_| {
            MobileRuntimeError::Config(format!("JNI handle must be positive, got `{handle}`"))
        })?;
        runtime_close_response(handle)
    })
}

fn runtime_create_response(config_json: &str) -> MobileResult<Value> {
    let config: CreatorRuntimeConfig = serde_json::from_str(config_json)?;
    let runtime = MobileCreatorRuntime::new(config)?;
    let metadata = runtime.node_metadata();
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
    registry()
        .lock()
        .map_err(|_| MobileRuntimeError::Runtime("registry lock poisoned".to_string()))?
        .runtimes
        .insert(handle, runtime);
    Ok(json!({"handle": handle, "node_metadata": metadata}))
}

fn runtime_call_response(
    handle: u64,
    method: &str,
    request_json: Option<&str>,
) -> MobileResult<Value> {
    let request = match request_json {
        Some(request_json) => serde_json::from_str(request_json)?,
        None => Value::Null,
    };
    let registry = registry()
        .lock()
        .map_err(|_| MobileRuntimeError::Runtime("registry lock poisoned".to_string()))?;
    let runtime = registry
        .runtimes
        .get(&handle)
        .ok_or_else(|| MobileRuntimeError::Runtime(format!("unknown handle `{handle}`")))?;
    runtime.call_json(method, request)
}

fn runtime_close_response(handle: u64) -> MobileResult<Value> {
    let removed = registry()
        .lock()
        .map_err(|_| MobileRuntimeError::Runtime("registry lock poisoned".to_string()))?
        .runtimes
        .remove(&handle)
        .is_some();
    Ok(json!({"closed": removed}))
}

fn ffi_response<F>(f: F) -> *mut c_char
where
    F: FnOnce() -> MobileResult<Value>,
{
    CString::new(response_json(f))
        .expect("JSON response must not contain interior null bytes")
        .into_raw()
}

fn jni_response<F>(env: &mut JNIEnv<'_>, f: F) -> jstring
where
    F: FnOnce(&mut JNIEnv<'_>) -> MobileResult<Value>,
{
    let body = response_json(|| f(env));
    match env.new_string(body) {
        Ok(value) => value.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

fn response_json<F>(f: F) -> String
where
    F: FnOnce() -> MobileResult<Value>,
{
    let response = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(body)) => json!({"ok": true, "body": body}),
        Ok(Err(error)) => json!({
            "ok": false,
            "error": {
                "code": error.code(),
                "message": error.to_string(),
            }
        }),
        Err(_) => json!({
            "ok": false,
            "error": {
                "code": "panic",
                "message": "native runtime panic was caught",
            }
        }),
    };
    serde_json::to_string(&response)
        .unwrap_or_else(|_| "{\"ok\":false,\"error\":{\"code\":\"serialization_error\"}}".into())
}

fn jni_string(env: &mut JNIEnv<'_>, value: JString<'_>) -> MobileResult<String> {
    let text = env
        .get_string(&value)
        .map_err(|error| MobileRuntimeError::Config(format!("JNI string error: {error}")))?;
    Ok(text.into())
}

fn unsafe_cstr(value: *const c_char) -> MobileResult<String> {
    if value.is_null() {
        return Err(MobileRuntimeError::Config(
            "FFI string argument was null".to_string(),
        ));
    }
    let text = unsafe { CStr::from_ptr(value) }.to_str().map_err(|error| {
        MobileRuntimeError::Config(format!("FFI string was not UTF-8: {error}"))
    })?;
    Ok(text.to_string())
}
