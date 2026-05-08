//! Local admin HTTP surface for Conduit V2 service containers.
//!
//! This listener binds to 127.0.0.1:9090 by default inside each container.
//! Operators reach it through ECS exec; it is not exposed through public ingress.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ed25519_dalek::SigningKey;
use gbn_bridge_creator::{
    CreatorClient, CreatorError, DiscoveryProbeResult, LocalDhtStore, SendDummyResult,
};
use gbn_bridge_protocol::{
    BootstrapJoinReply, BootstrapSession, BridgeCommandPayload, BridgeDhtEntry, CreatorDhtEntry,
    CreatorDhtEntryUnsigned, CreatorJoinRequest, DhtBridgeIngressEndpoint, HostCreatorSeedState,
    HostRoleState, LocalDiscoveryTable, NewCreatorSeedState, PendingCreator, ProtocolError,
    PublicKeyBytes, PublisherDhtEntry, ReachabilityClass, SelfOnboardingState, TunnelPeerRole,
    TunnelState,
};
use serde::{Deserialize, Serialize};

use crate::api::{
    AuthorityApiRequest, AuthorityApiRequestUnsigned, AuthorityApiResponse, AuthorityRoute,
    BootstrapJoinBody,
};
use crate::control::BridgeAdminCommandReceipt;
use crate::metrics::{
    AuthorityMetricsSnapshot, BridgeMetrics, BridgeMetricsSnapshot, ReceiverMetrics,
    ReceiverMetricsSnapshot,
};
use crate::metrics_otlp;
use crate::service::{AuthorityService, ServiceError};
use crate::storage::{BridgeRecord, IngestedFrameRecord};
use crate::AuthorityError;

pub const ADMIN_BIND_ADDR_ENV: &str = "GBN_BRIDGE_ADMIN_BIND_ADDR";
pub const DEFAULT_ADMIN_BIND_ADDR: &str = "127.0.0.1:9090";
const DEFAULT_FRAME_LIMIT: usize = 1_000;
const DEFAULT_SEND_DUMMY_SIZE: usize = 512;
const MAX_SEND_DUMMY_SIZE: usize = 8 * 1024;

pub fn admin_bind_addr_from_env() -> Result<SocketAddr, String> {
    std::env::var(ADMIN_BIND_ADDR_ENV)
        .unwrap_or_else(|_| DEFAULT_ADMIN_BIND_ADDR.to_string())
        .parse()
        .map_err(|_| format!("{ADMIN_BIND_ADDR_ENV} must be a valid socket address"))
}

#[derive(Debug, Clone)]
pub struct AdminState {
    authority: Option<Arc<Mutex<AuthorityService>>>,
    metrics: AdminMetricsSource,
    creator: Option<AdminCreatorConfig>,
    node_metadata: AdminNodeMetadata,
    local_dht: AdminLocalDhtSource,
}

#[derive(Debug, Clone)]
enum AdminMetricsSource {
    Authority,
    Receiver(Arc<Mutex<ReceiverMetrics>>),
    Bridge(Arc<Mutex<BridgeMetrics>>),
}

#[derive(Debug, Clone)]
enum AdminLocalDhtSource {
    NotApplicable(AdminLocalDhtResponse),
    Creator(LocalDhtStore),
}

#[derive(Debug, Clone)]
pub struct AdminCreatorConfig {
    pub actor_id: String,
    pub signing_key: SigningKey,
    pub publisher_pub: PublicKeyBytes,
    pub authority_url: String,
    pub creator_ip_addr: String,
    pub udp_punch_port: u16,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminNodeMetadata {
    pub node_id: String,
    pub role: String,
    pub conduit_actor: Option<String>,
    pub admin_addr: String,
    pub admin_bind_addr: String,
    pub state_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_addr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_udp_punch_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_public_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_surface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingress_endpoints: Option<Vec<DhtBridgeIngressEndpoint>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udp_punch_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reachability_class: Option<ReachabilityClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expiry_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_signature: Option<String>,
    pub build_version: String,
    pub build_source: String,
    pub image: String,
}

impl AdminNodeMetadata {
    pub fn from_env(node_id: impl Into<String>, role: impl Into<String>) -> Self {
        let role = role.into();
        let conduit_actor = std::env::var("GBN_CONDUIT_ACTOR")
            .ok()
            .or_else(|| std::env::var("GBN_NODE_ACTOR").ok())
            .filter(|value| !value.trim().is_empty());
        let admin_addr = std::env::var(ADMIN_BIND_ADDR_ENV)
            .unwrap_or_else(|_| DEFAULT_ADMIN_BIND_ADDR.to_string());
        let authority_url = std::env::var("GBN_BRIDGE_AUTHORITY_URL")
            .ok()
            .or_else(|| std::env::var("GBN_BRIDGE_PUBLISHER_URL").ok());
        let receiver_url = std::env::var("GBN_BRIDGE_RECEIVER_URL").ok();
        let publisher_public_key = env_public_key_hex();
        Self {
            node_id: node_id.into(),
            role,
            conduit_actor,
            admin_addr: admin_addr.clone(),
            admin_bind_addr: admin_addr,
            state_dir: std::env::var("GBN_BRIDGE_STATE_DIR").ok(),
            ip_addr: std::env::var("GBN_BRIDGE_INGRESS_HOST").ok(),
            creator_udp_punch_port: env_u16("GBN_BRIDGE_PUNCH_PORT"),
            public_key: None,
            publisher_public_key,
            publisher_surface: None,
            authority_url,
            receiver_url,
            ingress_endpoints: None,
            udp_punch_port: None,
            reachability_class: None,
            capabilities: None,
            lease_expiry_ms: None,
            publisher_signature: None,
            build_version: std::env::var("VERITAS_CONDUIT_BUILD_VERSION")
                .unwrap_or_else(|_| "unknown".to_string()),
            build_source: std::env::var("VERITAS_CONDUIT_BUILD_SOURCE")
                .unwrap_or_else(|_| "unknown".to_string()),
            image: std::env::var("VERITAS_CONDUIT_IMAGE").unwrap_or_else(|_| "unknown".to_string()),
        }
    }

    pub fn with_public_key(mut self, key: &PublicKeyBytes) -> Self {
        self.public_key = Some(bytes_to_hex(&key.0));
        self
    }

    pub fn with_publisher_public_key(mut self, key: &PublicKeyBytes) -> Self {
        self.publisher_public_key = Some(bytes_to_hex(&key.0));
        self
    }

    pub fn with_publisher_surface(mut self, surface: impl Into<String>) -> Self {
        self.publisher_surface = Some(surface.into());
        self
    }

    pub fn with_authority_url(mut self, url: impl Into<String>) -> Self {
        self.authority_url = Some(url.into());
        self
    }

    pub fn with_receiver_url(mut self, url: impl Into<String>) -> Self {
        self.receiver_url = Some(url.into());
        self
    }

    pub fn with_creator_transport(
        mut self,
        ip_addr: impl Into<String>,
        udp_punch_port: u16,
    ) -> Self {
        self.ip_addr = Some(ip_addr.into());
        self.creator_udp_punch_port = Some(udp_punch_port);
        self
    }

    pub fn with_bridge_transport(
        mut self,
        ingress_host: impl Into<String>,
        udp_punch_port: u16,
        reachability_class: ReachabilityClass,
    ) -> Self {
        let ingress_host = ingress_host.into();
        self.ingress_endpoints = Some(vec![DhtBridgeIngressEndpoint::direct(
            ingress_host,
            udp_punch_port,
        )]);
        self.udp_punch_port = Some(udp_punch_port);
        self.reachability_class = Some(reachability_class);
        self.capabilities = Some(vec![
            "bootstrap_seed".to_string(),
            "catalog_refresh".to_string(),
            "session_relay".to_string(),
            "batch_assignment".to_string(),
            "progress_reporting".to_string(),
        ]);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminLocalDhtResponse {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_surface: Option<String>,
    pub state: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_path: Option<String>,
}

impl AdminLocalDhtResponse {
    pub fn not_applicable(role: impl Into<String>) -> Self {
        let role = role.into();
        Self {
            reason: format!("{role} does not maintain creator local-DHT state"),
            role,
            publisher_surface: None,
            state: "not_applicable".to_string(),
            state_path: None,
        }
    }

    pub fn with_publisher_surface(mut self, surface: impl Into<String>) -> Self {
        self.publisher_surface = Some(surface.into());
        self.reason = format!(
            "publisher {} surface does not maintain creator local-DHT state",
            self.publisher_surface.as_deref().unwrap_or("unknown")
        );
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgesResponse {
    pub bridges: Vec<BridgeRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FramesResponse {
    pub frames: Vec<IngestedFrameRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "service", content = "snapshot")]
pub enum MetricsResponse {
    Authority(AuthorityMetricsSnapshot),
    Receiver(ReceiverMetricsSnapshot),
    Bridge(BridgeMetricsSnapshot),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InjectCommandRequest {
    pub payload: BridgeCommandPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendDummyRequest {
    pub size: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryProbeRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedHostCreatorRequest {
    pub host_creator_id: String,
    pub publisher_entry: PublisherDhtEntry,
    pub exit_bridge_a_entry: BridgeDhtEntry,
    #[serde(default)]
    pub bootstrap_genesis: bool,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedHostCreatorResponse {
    pub host_creator_id: String,
    pub self_onboarding_state: SelfOnboardingState,
    pub host_role_state: HostRoleState,
    pub seeded_bridge_id: String,
    pub publisher_node_id: String,
    pub chain_id: String,
    pub genesis: bool,
    pub forced: bool,
    pub idempotent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeDhtEntryResponse {
    pub bridge: BridgeDhtEntry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatorDhtEntrySignRequest {
    pub creator: CreatorDhtEntryUnsigned,
    #[serde(default = "default_true")]
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatorDhtEntryResponse {
    pub creator: CreatorDhtEntry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitializePublisherDhtRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitializePublisherDhtResponse {
    pub active_bridge_count: usize,
    pub initialized_bridge_count: usize,
    pub publisher_dht_entry_count: usize,
    pub stale_entry_count: usize,
    pub bridge_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedNewCreatorRequest {
    pub new_creator_id: String,
    pub host_creator_entry: CreatorDhtEntry,
    #[serde(default = "default_true")]
    pub start_bootstrap: bool,
    #[serde(default)]
    pub force: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_admin_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedNewCreatorResponse {
    pub new_creator_id: String,
    pub host_creator_id: String,
    pub self_onboarding_state: SelfOnboardingState,
    pub chain_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_session_id: Option<String>,
    pub started_bootstrap: bool,
    pub forced: bool,
    pub idempotent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartBootstrapRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostJoinRelayBody {
    pub host_creator_id: String,
    pub creator: PendingCreator,
    pub now_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostJoinRelayResponse {
    pub chain_id: String,
    pub new_creator_id: String,
    pub host_creator_id: String,
    pub relay_bridge_id: String,
    pub bootstrap_session_id: String,
    pub bootstrap_reply: BootstrapJoinReply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminErrorResponse {
    pub error: AdminErrorBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminErrorBody {
    pub code: String,
    pub message: String,
}

fn default_true() -> bool {
    true
}

pub struct AdminHttpServer {
    listener: TcpListener,
    state: AdminState,
    request_max_bytes: usize,
}

pub struct AdminHttpServerHandle {
    local_addr: SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<io::Result<()>>>,
}

impl AdminState {
    pub fn authority(service: Arc<Mutex<AuthorityService>>) -> Self {
        let publisher_pub = service
            .lock()
            .expect("authority service mutex poisoned while reading publisher key")
            .publisher_public_key()
            .clone();
        Self {
            authority: Some(service),
            metrics: AdminMetricsSource::Authority,
            creator: None,
            node_metadata: AdminNodeMetadata::from_env("publisher-authority", "publisher")
                .with_publisher_surface("authority")
                .with_public_key(&publisher_pub)
                .with_publisher_public_key(&publisher_pub),
            local_dht: AdminLocalDhtSource::NotApplicable(
                AdminLocalDhtResponse::not_applicable("publisher")
                    .with_publisher_surface("authority"),
            ),
        }
    }

    pub fn authority_with_creator(
        service: Arc<Mutex<AuthorityService>>,
        creator: AdminCreatorConfig,
    ) -> Self {
        Self {
            authority: Some(service),
            metrics: AdminMetricsSource::Authority,
            node_metadata: AdminNodeMetadata::from_env("publisher-authority", "publisher")
                .with_publisher_surface("authority")
                .with_authority_url(creator.authority_url.clone())
                .with_public_key(&creator.publisher_pub)
                .with_publisher_public_key(&creator.publisher_pub),
            local_dht: AdminLocalDhtSource::NotApplicable(
                AdminLocalDhtResponse::not_applicable("publisher")
                    .with_publisher_surface("authority"),
            ),
            creator: Some(creator),
        }
    }

    pub fn stub() -> Self {
        Self {
            authority: None,
            metrics: AdminMetricsSource::Authority,
            creator: None,
            node_metadata: AdminNodeMetadata::from_env("publisher-authority", "publisher"),
            local_dht: AdminLocalDhtSource::NotApplicable(AdminLocalDhtResponse::not_applicable(
                "publisher",
            )),
        }
    }

    pub fn receiver(metrics: Arc<Mutex<ReceiverMetrics>>) -> Self {
        Self {
            authority: None,
            metrics: AdminMetricsSource::Receiver(metrics),
            creator: None,
            node_metadata: AdminNodeMetadata::from_env("publisher-receiver", "publisher")
                .with_publisher_surface("receiver"),
            local_dht: AdminLocalDhtSource::NotApplicable(
                AdminLocalDhtResponse::not_applicable("publisher")
                    .with_publisher_surface("receiver"),
            ),
        }
    }

    pub fn receiver_with_creator(
        metrics: Arc<Mutex<ReceiverMetrics>>,
        creator: AdminCreatorConfig,
    ) -> Self {
        Self {
            authority: None,
            metrics: AdminMetricsSource::Receiver(metrics),
            node_metadata: AdminNodeMetadata::from_env("publisher-receiver", "publisher")
                .with_publisher_surface("receiver")
                .with_public_key(&creator.publisher_pub)
                .with_publisher_public_key(&creator.publisher_pub),
            local_dht: AdminLocalDhtSource::NotApplicable(
                AdminLocalDhtResponse::not_applicable("publisher")
                    .with_publisher_surface("receiver"),
            ),
            creator: Some(creator),
        }
    }

    pub fn bridge(metrics: Arc<Mutex<BridgeMetrics>>) -> Self {
        Self {
            authority: None,
            metrics: AdminMetricsSource::Bridge(metrics),
            creator: None,
            node_metadata: AdminNodeMetadata::from_env("exit-bridge", "exit_bridge"),
            local_dht: AdminLocalDhtSource::NotApplicable(AdminLocalDhtResponse::not_applicable(
                "exit_bridge",
            )),
        }
    }

    pub fn bridge_with_creator(
        metrics: Arc<Mutex<BridgeMetrics>>,
        creator: AdminCreatorConfig,
    ) -> Self {
        Self {
            authority: None,
            metrics: AdminMetricsSource::Bridge(metrics),
            node_metadata: AdminNodeMetadata::from_env(creator.actor_id.clone(), "exit_bridge")
                .with_public_key(&PublicKeyBytes::from_verifying_key(
                    &creator.signing_key.verifying_key(),
                ))
                .with_publisher_public_key(&creator.publisher_pub)
                .with_bridge_transport(
                    creator.creator_ip_addr.clone(),
                    creator.udp_punch_port,
                    ReachabilityClass::Direct,
                ),
            local_dht: AdminLocalDhtSource::NotApplicable(AdminLocalDhtResponse::not_applicable(
                "exit_bridge",
            )),
            creator: Some(creator),
        }
    }

    pub fn creator(metadata: AdminNodeMetadata, local_dht: LocalDhtStore) -> Self {
        Self {
            authority: None,
            metrics: AdminMetricsSource::Authority,
            creator: None,
            node_metadata: metadata,
            local_dht: AdminLocalDhtSource::Creator(local_dht),
        }
    }

    pub fn creator_with_config(
        metadata: AdminNodeMetadata,
        local_dht: LocalDhtStore,
        creator: AdminCreatorConfig,
    ) -> Self {
        Self {
            authority: None,
            metrics: AdminMetricsSource::Authority,
            creator: Some(creator),
            node_metadata: metadata,
            local_dht: AdminLocalDhtSource::Creator(local_dht),
        }
    }
}

impl AdminHttpServer {
    pub fn bind(
        bind_addr: SocketAddr,
        state: AdminState,
        request_max_bytes: usize,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(bind_addr)?;
        Ok(Self {
            listener,
            state,
            request_max_bytes,
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn spawn(self) -> io::Result<AdminHttpServerHandle> {
        let stop = Arc::new(AtomicBool::new(false));
        let local_addr = self.local_addr()?;
        let stop_for_thread = Arc::clone(&stop);
        let join = thread::spawn(move || self.run_loop(stop_for_thread));
        Ok(AdminHttpServerHandle {
            local_addr,
            stop,
            join: Some(join),
        })
    }

    pub fn serve_forever(self) -> io::Result<()> {
        self.listener.set_nonblocking(false)?;
        loop {
            let (stream, _) = self.listener.accept()?;
            let state = self.state.clone();
            let request_max_bytes = self.request_max_bytes;
            thread::spawn(move || {
                if let Err(error) = handle_connection(stream, &state, request_max_bytes) {
                    eprintln!("admin connection error: {error}");
                }
            });
        }
    }

    fn run_loop(self, stop: Arc<AtomicBool>) -> io::Result<()> {
        self.listener.set_nonblocking(true)?;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }

            match self.listener.accept() {
                Ok((stream, _)) => {
                    let state = self.state.clone();
                    let request_max_bytes = self.request_max_bytes;
                    thread::spawn(move || {
                        if let Err(error) = handle_connection(stream, &state, request_max_bytes) {
                            eprintln!("admin connection error: {error}");
                        }
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error),
            }
        }
    }
}

impl AdminHttpServerHandle {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn join(mut self) -> io::Result<()> {
        self.shutdown();
        match self.join.take() {
            Some(join) => join
                .join()
                .map_err(|_| io::Error::other("admin server thread panicked"))?,
            None => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FramesQuery {
    chain_id: Option<String>,
    limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn handle_connection(
    mut stream: TcpStream,
    state: &AdminState,
    request_max_bytes: usize,
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let request = read_http_request(&mut stream, request_max_bytes)?;
    let response = route_request(state, request);
    stream.write_all(&response)?;
    Ok(())
}

fn route_request(state: &AdminState, request: HttpRequest) -> Vec<u8> {
    let (path, query) = split_path_and_query(&request.path);
    match (request.method.as_str(), path) {
        ("GET", path) if path == AuthorityRoute::AdminBridges.path() => list_bridges(state),
        ("GET", path) if path == AuthorityRoute::AdminFrames.path() => {
            match parse_frames_query(query) {
                Ok(query) => list_frames(state, query),
                Err(message) => error_response(400, "bad_query", &message),
            }
        }
        ("GET", path) if path == AuthorityRoute::AdminMetrics.path() => metrics_snapshot(state),
        ("GET", path) if path == AuthorityRoute::AdminNodeMetadata.path() => node_metadata(state),
        ("GET", path) if path == AuthorityRoute::AdminLocalDht.path() => local_dht(state),
        ("GET", path) => match admin_bridge_dht_entry_target(path) {
            Some(bridge_id) => bridge_dht_entry(state, bridge_id),
            None => error_response(404, "not_found", "admin route not found"),
        },
        ("POST", path) if path == AuthorityRoute::AdminInitializePublisherDht.path() => {
            initialize_publisher_dht(state, &request.body)
        }
        ("POST", path) if path == AuthorityRoute::AdminCreatorDhtEntry.path() => {
            creator_dht_entry(state, &request.body)
        }
        ("POST", path) if path == AuthorityRoute::AdminResetCreatorState.path() => {
            reset_creator_state(state, &request.body)
        }
        ("POST", path) if path == AuthorityRoute::AdminSeedHostCreator.path() => {
            seed_host_creator(state, &request.body)
        }
        ("POST", path) if path == AuthorityRoute::AdminSeedNewCreator.path() => {
            seed_new_creator(state, &request.body)
        }
        ("POST", path) if path == AuthorityRoute::AdminStartBootstrap.path() => {
            start_bootstrap(state, &request.body)
        }
        ("POST", path) if path == AuthorityRoute::AdminHostJoinRelay.path() => {
            host_join_relay(state, &request.body)
        }
        ("POST", path) if path == AuthorityRoute::AdminSendDummy.path() => {
            inject_send_dummy(state, &request.body)
        }
        ("POST", path) if path == AuthorityRoute::AdminDiscoveryProbe.path() => {
            inject_discovery_probe(state, &request.body)
        }
        ("POST", path) => match admin_bridge_command_target(path) {
            Some(bridge_id) => inject_bridge_command(state, bridge_id, &request.body),
            None => error_response(404, "not_found", "admin route not found"),
        },
        _ => error_response(
            405,
            "method_not_allowed",
            "unsupported admin method/path combination",
        ),
    }
}

fn node_metadata(state: &AdminState) -> Vec<u8> {
    json_response(200, &state.node_metadata)
}

fn local_dht(state: &AdminState) -> Vec<u8> {
    match &state.local_dht {
        AdminLocalDhtSource::NotApplicable(response) => json_response(200, response),
        AdminLocalDhtSource::Creator(store) => json_response(200, &store.snapshot()),
    }
}

fn reset_creator_state(state: &AdminState, body: &[u8]) -> Vec<u8> {
    if !body.is_empty() {
        if let Err(error) = serde_json::from_slice::<serde_json::Value>(body) {
            return error_response(
                400,
                "bad_request",
                &format!("invalid reset-creator-state json: {error}"),
            );
        }
    }

    let AdminLocalDhtSource::Creator(store) = &state.local_dht else {
        return error_response(
            405,
            "method_not_allowed",
            "reset-creator-state is only available on creator admin listeners",
        );
    };

    let now_ms = now_ms();
    let chain_id = format!("reset-creator-state-{}-{now_ms}", store.actor_id());
    match store.reset(chain_id, now_ms) {
        Ok(response) => json_response(200, &response),
        Err(error) => error_response(500, "local_dht_reset_failed", &error.to_string()),
    }
}

fn seed_host_creator(state: &AdminState, body: &[u8]) -> Vec<u8> {
    let AdminLocalDhtSource::Creator(store) = &state.local_dht else {
        return error_response(
            405,
            "method_not_allowed",
            "seed-host-creator is only available on creator admin listeners",
        );
    };

    let request = match serde_json::from_slice::<SeedHostCreatorRequest>(body) {
        Ok(request) => request,
        Err(error) => {
            return error_response(
                400,
                "bad_request",
                &format!("invalid seed-host-creator json: {error}"),
            )
        }
    };
    let now_ms = now_ms();
    let proposed_chain_id = format!("seed-host-creator-{}-{now_ms}", request.host_creator_id);
    emit_host_seed_event(
        "host_creator_seed_requested",
        &proposed_chain_id,
        &request.host_creator_id,
        &request.exit_bridge_a_entry.bridge_id,
        &request.publisher_entry.node_id,
        request.bootstrap_genesis,
        request.force,
    );

    if request.host_creator_id != store.actor_id() {
        return error_response(
            409,
            "host_creator_id_mismatch",
            &format!(
                "seed request targets host_creator_id `{}`, but this creator is `{}`",
                request.host_creator_id,
                store.actor_id()
            ),
        );
    }

    let trusted_publisher_key = match trusted_publisher_key(state) {
        Ok(key) => key,
        Err(message) => return error_response(409, "publisher_trust_mismatch", &message),
    };
    if request
        .publisher_entry
        .verify_trust_root(&trusted_publisher_key, now_ms)
        .is_err()
    {
        return error_response(
            409,
            "publisher_trust_mismatch",
            "publisher DHT entry does not match the configured publisher trust root or has expired",
        );
    }
    if let Err(error) = request
        .exit_bridge_a_entry
        .verify_authority(&trusted_publisher_key, now_ms)
    {
        return bridge_seed_validation_error(error);
    }
    if request.exit_bridge_a_entry.reachability_class == ReachabilityClass::RelayOnly {
        return error_response(
            409,
            "bridge_relay_only_ineligible",
            "SeedHostCreator requires a non-relay-only ExitBridgeA entry",
        );
    }

    let snapshot = store.snapshot();
    if let Some(existing) = &snapshot.host_seed_state {
        if host_seed_matches(existing, &request) {
            let chain_id = existing_or_proposed_chain_id(existing, &proposed_chain_id);
            emit_host_seed_event(
                "host_creator_seed_idempotent_replay",
                &chain_id,
                &request.host_creator_id,
                &request.exit_bridge_a_entry.bridge_id,
                &request.publisher_entry.node_id,
                request.bootstrap_genesis,
                false,
            );
            return json_response(
                200,
                &seed_host_response(
                    &snapshot,
                    &request,
                    chain_id,
                    existing.bootstrap_genesis,
                    false,
                    true,
                ),
            );
        }

        if !request.force {
            return error_response(
                409,
                "seed_already_present",
                "HostCreator seed state is already present; use force=true to replace it",
            );
        }
        emit_host_seed_event(
            "host_creator_seed_force_replaced",
            &proposed_chain_id,
            &request.host_creator_id,
            &request.exit_bridge_a_entry.bridge_id,
            &request.publisher_entry.node_id,
            request.bootstrap_genesis,
            true,
        );
    }

    if !request.bootstrap_genesis
        && snapshot.self_onboarding_state != SelfOnboardingState::Onboarded
    {
        return error_response(
            409,
            "host_creator_not_onboarded",
            "SeedHostCreator requires an already-onboarded creator unless bootstrap_genesis=true",
        );
    }

    if request.bootstrap_genesis {
        emit_host_seed_warn_event(
            "host_creator_genesis_seed_used",
            &proposed_chain_id,
            &request.host_creator_id,
            &request.exit_bridge_a_entry.bridge_id,
            &request.publisher_entry.node_id,
            true,
            request.force,
        );
    }
    emit_host_seed_event(
        "host_creator_seed_validated",
        &proposed_chain_id,
        &request.host_creator_id,
        &request.exit_bridge_a_entry.bridge_id,
        &request.publisher_entry.node_id,
        request.bootstrap_genesis,
        request.force,
    );

    let mut next = if request.force {
        LocalDiscoveryTable::empty(store.actor_id().to_string(), now_ms)
    } else {
        snapshot.clone()
    };
    next.self_onboarding_state = SelfOnboardingState::Onboarded;
    next.host_role_state = HostRoleState::HostSeeded;
    next.publisher_entry = Some(request.publisher_entry.clone());
    next.bridge_entries = vec![request.exit_bridge_a_entry.clone()];
    next.host_seed_state = Some(HostCreatorSeedState {
        host_creator_actor_id: request.host_creator_id.clone(),
        chain_id: proposed_chain_id.clone(),
        publisher_entry: request.publisher_entry.clone(),
        exit_bridge_a_entry: request.exit_bridge_a_entry.clone(),
        seeded_at_ms: now_ms,
        bootstrap_genesis: request.bootstrap_genesis,
    });
    next.last_update_ms = now_ms;
    next.last_error = None;

    match store.replace(next) {
        Ok(committed) => {
            emit_host_seed_event(
                "host_creator_seed_stored",
                &proposed_chain_id,
                &request.host_creator_id,
                &request.exit_bridge_a_entry.bridge_id,
                &request.publisher_entry.node_id,
                request.bootstrap_genesis,
                request.force,
            );
            json_response(
                200,
                &seed_host_response(
                    &committed,
                    &request,
                    proposed_chain_id,
                    request.bootstrap_genesis,
                    request.force,
                    false,
                ),
            )
        }
        Err(error) => error_response(500, "local_dht_seed_failed", &error.to_string()),
    }
}

fn seed_new_creator(state: &AdminState, body: &[u8]) -> Vec<u8> {
    let AdminLocalDhtSource::Creator(store) = &state.local_dht else {
        return error_response(
            405,
            "method_not_allowed",
            "seed-new-creator is only available on creator admin listeners",
        );
    };
    let request = match serde_json::from_slice::<SeedNewCreatorRequest>(body) {
        Ok(request) => request,
        Err(error) => {
            return error_response(
                400,
                "bad_request",
                &format!("invalid seed-new-creator json: {error}"),
            )
        }
    };
    let now_ms = now_ms();
    let proposed_chain_id = format!("seed-new-creator-{}-{now_ms}", request.new_creator_id);
    emit_new_seed_event(
        "new_creator_seed_requested",
        &proposed_chain_id,
        &request.new_creator_id,
        &request.host_creator_entry.node_id,
        None,
        None,
        request.force,
    );

    if request.new_creator_id != store.actor_id() {
        return error_response(
            409,
            "new_creator_id_mismatch",
            &format!(
                "seed request targets new_creator_id `{}`, but this creator is `{}`",
                request.new_creator_id,
                store.actor_id()
            ),
        );
    }
    let trusted_publisher_key = match trusted_publisher_key(state) {
        Ok(key) => key,
        Err(message) => return error_response(409, "publisher_trust_mismatch", &message),
    };
    if let Err(error) = request
        .host_creator_entry
        .verify_authority(&trusted_publisher_key, now_ms)
    {
        return host_creator_entry_validation_error(error);
    }

    let snapshot = store.snapshot();
    if let Some(existing) = &snapshot.new_creator_seed_state {
        if new_seed_matches(existing, &request)
            && matches!(
                snapshot.self_onboarding_state,
                SelfOnboardingState::NewCreatorSeeded | SelfOnboardingState::Bootstrapping
            )
        {
            let chain_id = existing_new_seed_chain_id(existing, &proposed_chain_id);
            emit_new_seed_event(
                "new_creator_seed_idempotent_replay",
                &chain_id,
                &request.new_creator_id,
                &request.host_creator_entry.node_id,
                None,
                snapshot
                    .current_bootstrap_session
                    .as_ref()
                    .map(|session| session.session_id.as_str()),
                false,
            );
            return json_response(
                200,
                &seed_new_response(
                    &snapshot,
                    &request,
                    chain_id,
                    snapshot
                        .current_bootstrap_session
                        .as_ref()
                        .map(|session| session.session_id.clone()),
                    false,
                    true,
                ),
            );
        }

        if !request.force {
            return error_response(
                409,
                "seed_already_present",
                "NewCreator seed state is already present; use force=true to replace it",
            );
        }
        emit_new_seed_event(
            "new_creator_seed_force_replaced",
            &proposed_chain_id,
            &request.new_creator_id,
            &request.host_creator_entry.node_id,
            None,
            None,
            true,
        );
    } else if !request.force && !new_creator_seed_state_allows_seed(snapshot.self_onboarding_state)
    {
        return error_response(
            409,
            "new_creator_already_seeded",
            "selected creator already has an in-flight or completed onboarding state",
        );
    }

    let mut next = if request.force {
        LocalDiscoveryTable::empty(store.actor_id().to_string(), now_ms)
    } else {
        snapshot.clone()
    };
    next.self_onboarding_state = if request.start_bootstrap {
        SelfOnboardingState::Bootstrapping
    } else {
        SelfOnboardingState::NewCreatorSeeded
    };
    next.host_creator_entry = Some(request.host_creator_entry.clone());
    next.new_creator_seed_state = Some(NewCreatorSeedState {
        new_creator_actor_id: request.new_creator_id.clone(),
        chain_id: proposed_chain_id.clone(),
        host_creator_entry: request.host_creator_entry.clone(),
        seeded_at_ms: now_ms,
        start_bootstrap: request.start_bootstrap,
    });
    next.current_bootstrap_session = None;
    next.last_update_ms = now_ms;
    next.last_error = None;
    let committed = match store.replace(next) {
        Ok(table) => table,
        Err(error) => return error_response(500, "local_dht_seed_failed", &error.to_string()),
    };
    emit_new_seed_event(
        "new_creator_seed_stored",
        &proposed_chain_id,
        &request.new_creator_id,
        &request.host_creator_entry.node_id,
        None,
        None,
        request.force,
    );

    if !request.start_bootstrap {
        return json_response(
            200,
            &seed_new_response(&committed, &request, proposed_chain_id, None, false, false),
        );
    }

    match begin_new_creator_bootstrap(state, store, &request, &proposed_chain_id, now_ms) {
        Ok((table, bootstrap_session_id)) => json_response(
            200,
            &seed_new_response(
                &table,
                &request,
                proposed_chain_id,
                Some(bootstrap_session_id),
                true,
                false,
            ),
        ),
        Err((status, code, message)) => error_response(status, code, &message),
    }
}

fn start_bootstrap(state: &AdminState, body: &[u8]) -> Vec<u8> {
    let AdminLocalDhtSource::Creator(store) = &state.local_dht else {
        return error_response(
            405,
            "method_not_allowed",
            "start-bootstrap is only available on creator admin listeners",
        );
    };
    if !body.is_empty() {
        if let Err(error) = serde_json::from_slice::<StartBootstrapRequest>(body) {
            return error_response(
                400,
                "bad_request",
                &format!("invalid start-bootstrap json: {error}"),
            );
        }
    }
    let snapshot = store.snapshot();
    let Some(seed) = snapshot.new_creator_seed_state.clone() else {
        return error_response(
            409,
            "new_creator_not_seeded",
            "start-bootstrap requires a prior SeedNewCreator payload",
        );
    };
    let chain_id = existing_new_seed_chain_id(
        &seed,
        &format!(
            "seed-new-creator-{}-{}",
            seed.new_creator_actor_id,
            now_ms()
        ),
    );
    let request = SeedNewCreatorRequest {
        new_creator_id: seed.new_creator_actor_id,
        host_creator_entry: seed.host_creator_entry,
        start_bootstrap: true,
        force: false,
        host_admin_url: None,
    };
    match begin_new_creator_bootstrap(state, store, &request, &chain_id, now_ms()) {
        Ok((table, bootstrap_session_id)) => json_response(
            200,
            &seed_new_response(
                &table,
                &request,
                chain_id,
                Some(bootstrap_session_id),
                true,
                false,
            ),
        ),
        Err((status, code, message)) => error_response(status, code, &message),
    }
}

fn host_join_relay(state: &AdminState, body: &[u8]) -> Vec<u8> {
    let AdminLocalDhtSource::Creator(store) = &state.local_dht else {
        return error_response(
            405,
            "method_not_allowed",
            "host join relay is only available on creator admin listeners",
        );
    };
    let Some(config) = &state.creator else {
        return error_response(
            501,
            "not_supported",
            "host join relay requires a configured creator identity",
        );
    };
    let request = match serde_json::from_slice::<AuthorityApiRequest<HostJoinRelayBody>>(body) {
        Ok(request) => request,
        Err(error) => {
            return error_response(
                400,
                "bad_request",
                &format!("invalid host join relay json: {error}"),
            )
        }
    };
    if let Err(error) = request.verify_signature() {
        return error_response(
            401,
            "new_creator_signature_invalid",
            &format!("NewCreator relay signature failed validation: {error}"),
        );
    }
    if request.actor_id != request.body.creator.node_id {
        return error_response(
            409,
            "new_creator_id_mismatch",
            "host join relay actor_id must match the pending creator id",
        );
    }
    if request.auth.actor_pub != request.body.creator.pub_key {
        return error_response(
            401,
            "new_creator_signature_invalid",
            "host join relay signature key must match the pending creator public key",
        );
    }
    if request.body.host_creator_id != store.actor_id() {
        return error_response(
            409,
            "host_creator_id_mismatch",
            "host join relay request targeted a different HostCreator",
        );
    }

    let snapshot = store.snapshot();
    let Some(host_seed) = snapshot.host_seed_state.clone() else {
        return error_response(
            409,
            "host_creator_not_seeded",
            "HostCreator has no seeded Publisher or ExitBridgeA state",
        );
    };
    if snapshot.host_role_state != HostRoleState::HostSeeded {
        return error_response(
            409,
            "host_creator_not_seeded",
            "HostCreator local state is not host_seeded",
        );
    }
    let relay_bridge_id = host_seed.exit_bridge_a_entry.bridge_id.clone();
    emit_join_path_event(
        "host_creator_join_received",
        &request.chain_id,
        &request.body.creator.node_id,
        store.actor_id(),
        Some(&relay_bridge_id),
        None,
    );

    let join = CreatorJoinRequest {
        chain_id: request.chain_id.clone(),
        request_id: request.request_id.clone(),
        host_creator_id: store.actor_id().to_string(),
        relay_bridge_id: relay_bridge_id.clone(),
        creator: request.body.creator.clone(),
    };
    let authority_request = match AuthorityApiRequest::sign(
        AuthorityApiRequestUnsigned {
            chain_id: request.chain_id.clone(),
            request_id: request.request_id.clone(),
            sent_at_ms: request.sent_at_ms,
            actor_id: store.actor_id().to_string(),
            body: BootstrapJoinBody {
                request: join,
                now_ms: request.body.now_ms,
            },
        },
        &config.signing_key,
    ) {
        Ok(request) => request,
        Err(error) => {
            return error_response(
                500,
                "host_join_sign_failed",
                &format!("failed to sign HostCreator authority join envelope: {error}"),
            )
        }
    };
    let response = match post_json_raw(
        &host_seed.publisher_entry.authority_url,
        AuthorityRoute::BootstrapJoin.path(),
        &authority_request,
        config.timeout,
    ) {
        Ok(response) => response,
        Err(error) => {
            return error_response(
                502,
                "publisher_join_unreachable",
                &format!("failed to relay join to Publisher authority: {error}"),
            )
        }
    };
    let (status, response_body) = match parse_http_status_body(&response) {
        Ok(parsed) => parsed,
        Err(error) => return error_response(502, "publisher_join_bad_response", &error),
    };
    let authority_response: AuthorityApiResponse<BootstrapJoinReply> =
        match serde_json::from_slice(response_body) {
            Ok(response) => response,
            Err(error) => {
                return error_response(
                    502,
                    "publisher_join_bad_response",
                    &format!("failed to decode Publisher join response: {error}"),
                )
            }
        };
    if let Err(error) = authority_response.verify_authority(&config.publisher_pub) {
        return error_response(
            502,
            "publisher_join_signature_invalid",
            &format!("Publisher join response signature failed validation: {error}"),
        );
    }
    if status != 200 || !authority_response.ok {
        return error_response(
            502,
            "publisher_join_rejected",
            &authority_response
                .error
                .map(|error| format!("{}: {}", error.code, error.message))
                .unwrap_or_else(|| format!("Publisher returned HTTP {status}")),
        );
    }
    let Some(reply) = authority_response.body else {
        return error_response(
            502,
            "publisher_join_bad_response",
            "Publisher join response had no body",
        );
    };
    if let Err(error) = reply.verify_authority(&config.publisher_pub, request.body.now_ms) {
        return error_response(
            502,
            "publisher_bootstrap_payload_invalid",
            &format!("Publisher bootstrap payload failed validation: {error}"),
        );
    }
    let bootstrap_session_id = reply.response.bootstrap_session_id.clone();
    emit_join_path_event(
        "publisher_response_to_host_via_bridge",
        &request.chain_id,
        &request.body.creator.node_id,
        store.actor_id(),
        Some(&relay_bridge_id),
        Some(&bootstrap_session_id),
    );
    emit_join_path_event(
        "host_response_received_from_bridge",
        &request.chain_id,
        &request.body.creator.node_id,
        store.actor_id(),
        Some(&relay_bridge_id),
        Some(&bootstrap_session_id),
    );
    emit_join_path_event(
        "host_creator_join_relayed_via_bridge",
        &request.chain_id,
        &request.body.creator.node_id,
        store.actor_id(),
        Some(&relay_bridge_id),
        Some(&bootstrap_session_id),
    );
    emit_join_path_event(
        "host_relayed_response_to_new_creator",
        &request.chain_id,
        &request.body.creator.node_id,
        store.actor_id(),
        Some(&relay_bridge_id),
        Some(&bootstrap_session_id),
    );
    json_response(
        200,
        &HostJoinRelayResponse {
            chain_id: request.chain_id,
            new_creator_id: request.body.creator.node_id,
            host_creator_id: store.actor_id().to_string(),
            relay_bridge_id,
            bootstrap_session_id,
            bootstrap_reply: reply,
        },
    )
}

fn inject_send_dummy(state: &AdminState, body: &[u8]) -> Vec<u8> {
    let Some(config) = &state.creator else {
        return error_response(
            501,
            "not_supported",
            "send-dummy is not configured on this admin listener",
        );
    };
    let request = if body.is_empty() {
        SendDummyRequest { size: None }
    } else {
        match serde_json::from_slice::<SendDummyRequest>(body) {
            Ok(request) => request,
            Err(error) => {
                return error_response(
                    400,
                    "bad_request",
                    &format!("invalid send-dummy json: {error}"),
                )
            }
        }
    };
    let size = request.size.unwrap_or(DEFAULT_SEND_DUMMY_SIZE);
    if size > MAX_SEND_DUMMY_SIZE {
        return error_response(
            400,
            "bad_request",
            &format!("send-dummy size must be <= {MAX_SEND_DUMMY_SIZE} bytes"),
        );
    }

    let client = CreatorClient::new(
        config.actor_id.clone(),
        config.signing_key.clone(),
        config.publisher_pub.clone(),
    )
    .with_creator_endpoint(config.creator_ip_addr.clone(), config.udp_punch_port)
    .with_timeout(config.timeout);
    match client.send_dummy(&config.authority_url, size) {
        Ok(result) => {
            let _chain_span =
                metrics_otlp::chain_span("admin_send_dummy", &result.chain_id).entered();
            metrics_otlp::record_chain_id(&result.chain_id);
            json_response::<SendDummyResult>(200, &result)
        }
        Err(error) => creator_error_response(error),
    }
}

fn inject_discovery_probe(state: &AdminState, body: &[u8]) -> Vec<u8> {
    let Some(config) = &state.creator else {
        return error_response(
            501,
            "not_supported",
            "discovery-probe is not configured on this admin listener",
        );
    };
    if !body.is_empty() {
        match serde_json::from_slice::<DiscoveryProbeRequest>(body) {
            Ok(_) => {}
            Err(error) => {
                return error_response(
                    400,
                    "bad_request",
                    &format!("invalid discovery-probe json: {error}"),
                )
            }
        }
    }

    let client = CreatorClient::new(
        config.actor_id.clone(),
        config.signing_key.clone(),
        config.publisher_pub.clone(),
    )
    .with_creator_endpoint(config.creator_ip_addr.clone(), config.udp_punch_port)
    .with_timeout(config.timeout);
    match client.discovery_probe(&config.authority_url) {
        Ok(result) => {
            let _chain_span =
                metrics_otlp::chain_span("admin_discovery_probe", &result.chain_id).entered();
            metrics_otlp::record_chain_id(&result.chain_id);
            json_response::<DiscoveryProbeResult>(200, &result)
        }
        Err(error) => creator_error_response(error),
    }
}

fn list_bridges(state: &AdminState) -> Vec<u8> {
    let Some(authority) = &state.authority else {
        return error_response(
            501,
            "not_supported",
            "bridge registry is only available on the publisher authority",
        );
    };
    let service = authority
        .lock()
        .expect("authority service mutex poisoned while listing bridges");
    let response = BridgesResponse {
        bridges: service.publisher_authority().list_bridges(),
    };
    json_response(200, &response)
}

fn bridge_dht_entry(state: &AdminState, bridge_id: &str) -> Vec<u8> {
    let Some(authority) = &state.authority else {
        return error_response(
            501,
            "not_supported",
            "bridge DHT entries are only available on the publisher authority",
        );
    };
    let service = authority
        .lock()
        .expect("authority service mutex poisoned while signing bridge DHT entry");
    match service
        .publisher_authority()
        .bridge_dht_entry(bridge_id, now_ms())
    {
        Ok(bridge) => json_response(200, &BridgeDhtEntryResponse { bridge }),
        Err(error) => authority_error_response(error),
    }
}

fn initialize_publisher_dht(state: &AdminState, body: &[u8]) -> Vec<u8> {
    let Some(authority) = &state.authority else {
        return error_response(
            501,
            "not_supported",
            "publisher DHT initialization is only available on the publisher authority",
        );
    };
    if !body.is_empty() {
        if let Err(error) = serde_json::from_slice::<InitializePublisherDhtRequest>(body) {
            return error_response(
                400,
                "bad_request",
                &format!("invalid publisher-dht initialization json: {error}"),
            );
        }
    }
    let mut service = authority
        .lock()
        .expect("authority service mutex poisoned while initializing publisher DHT");
    match service
        .publisher_authority_mut()
        .initialize_publisher_bridge_dht(now_ms())
    {
        Ok(summary) => json_response(
            200,
            &InitializePublisherDhtResponse {
                active_bridge_count: summary.active_bridge_count,
                initialized_bridge_count: summary.initialized_bridge_count,
                publisher_dht_entry_count: service
                    .publisher_authority()
                    .publisher_bridge_dht_entry_count(),
                stale_entry_count: summary.stale_entry_count,
                bridge_ids: summary.bridge_ids,
            },
        ),
        Err(error) => authority_error_response(error),
    }
}

fn creator_dht_entry(state: &AdminState, body: &[u8]) -> Vec<u8> {
    let Some(authority) = &state.authority else {
        return error_response(
            501,
            "not_supported",
            "creator DHT entry signing is only available on the publisher authority",
        );
    };
    let request = match serde_json::from_slice::<CreatorDhtEntrySignRequest>(body) {
        Ok(request) => request,
        Err(error) => {
            return error_response(
                400,
                "bad_request",
                &format!("invalid creator-dht-entry json: {error}"),
            )
        }
    };
    let service = authority
        .lock()
        .expect("authority service mutex poisoned while signing creator DHT entry");
    match service
        .publisher_authority()
        .creator_dht_entry(request.creator, request.active)
    {
        Ok(creator) => json_response(200, &CreatorDhtEntryResponse { creator }),
        Err(error) => authority_error_response(error),
    }
}

fn list_frames(state: &AdminState, query: FramesQuery) -> Vec<u8> {
    let Some(authority) = &state.authority else {
        return error_response(
            501,
            "not_supported",
            "ingested frames are only available on the publisher authority",
        );
    };
    let service = authority
        .lock()
        .expect("authority service mutex poisoned while listing frames");
    let response = FramesResponse {
        frames: service
            .publisher_authority()
            .list_frames(query.chain_id.as_deref(), query.limit),
    };
    json_response(200, &response)
}

fn inject_bridge_command(state: &AdminState, bridge_id: &str, body: &[u8]) -> Vec<u8> {
    let Some(authority) = &state.authority else {
        return error_response(
            501,
            "not_supported",
            "bridge command injection is only available on the publisher authority",
        );
    };
    let request = match serde_json::from_slice::<InjectCommandRequest>(body) {
        Ok(request) => request,
        Err(error) => {
            return error_response(
                400,
                "bad_request",
                &format!("invalid admin command json: {error}"),
            )
        }
    };
    let mut service = authority
        .lock()
        .expect("authority service mutex poisoned while injecting command");
    match service.push_admin_command(bridge_id, request.payload) {
        Ok(receipt) => json_response::<BridgeAdminCommandReceipt>(200, &receipt),
        Err(error) => service_error_response(error),
    }
}

fn metrics_snapshot(state: &AdminState) -> Vec<u8> {
    let response = match &state.metrics {
        AdminMetricsSource::Authority => {
            let snapshot = match &state.authority {
                Some(authority) => authority
                    .lock()
                    .expect("authority service mutex poisoned while reading metrics")
                    .publisher_authority()
                    .metrics_snapshot(),
                None => AuthorityMetricsSnapshot::default(),
            };
            MetricsResponse::Authority(snapshot)
        }
        AdminMetricsSource::Receiver(metrics) => MetricsResponse::Receiver(
            metrics
                .lock()
                .expect("receiver metrics mutex poisoned")
                .snapshot(),
        ),
        AdminMetricsSource::Bridge(metrics) => MetricsResponse::Bridge(
            metrics
                .lock()
                .expect("bridge metrics mutex poisoned")
                .snapshot(),
        ),
    };
    json_response(200, &response)
}

fn trusted_publisher_key(state: &AdminState) -> Result<PublicKeyBytes, String> {
    let value = state
        .node_metadata
        .publisher_public_key
        .as_deref()
        .ok_or_else(|| {
            "creator admin metadata does not include publisher_public_key".to_string()
        })?;
    let bytes = decode_hex_bytes(value)?;
    if bytes.len() != 32 {
        return Err(format!(
            "publisher_public_key must decode to 32 bytes, got {}",
            bytes.len()
        ));
    }
    Ok(PublicKeyBytes(bytes))
}

fn decode_hex_bytes(value: &str) -> Result<Vec<u8>, String> {
    let value = value.trim().strip_prefix("0x").unwrap_or(value.trim());
    if value.len() % 2 != 0 {
        return Err("hex value has an odd number of characters".to_string());
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    for idx in (0..value.len()).step_by(2) {
        let byte = u8::from_str_radix(&value[idx..idx + 2], 16)
            .map_err(|_| format!("invalid hex byte at offset {idx}"))?;
        out.push(byte);
    }
    Ok(out)
}

fn bridge_seed_validation_error(error: ProtocolError) -> Vec<u8> {
    match error {
        ProtocolError::Expired { .. } => error_response(
            409,
            "bridge_expired",
            "ExitBridgeA DHT entry or lease has expired",
        ),
        _ => error_response(
            409,
            "bridge_signature_invalid",
            "ExitBridgeA DHT entry is not signed by the configured Publisher trust root",
        ),
    }
}

fn authority_error_response(error: AuthorityError) -> Vec<u8> {
    match &error {
        AuthorityError::BridgeNotFound { .. }
        | AuthorityError::PublisherBridgeDhtEntryMissing { .. } => {
            error_response(404, "not_found", &error.to_string())
        }
        AuthorityError::LeaseExpired { .. } => {
            error_response(409, "bridge_expired", &error.to_string())
        }
        _ => error_response(500, "authority_error", &error.to_string()),
    }
}

fn host_seed_matches(existing: &HostCreatorSeedState, request: &SeedHostCreatorRequest) -> bool {
    existing.host_creator_actor_id == request.host_creator_id
        && existing.publisher_entry == request.publisher_entry
        && existing.exit_bridge_a_entry == request.exit_bridge_a_entry
        && existing.bootstrap_genesis == request.bootstrap_genesis
}

fn existing_or_proposed_chain_id(
    existing: &HostCreatorSeedState,
    proposed_chain_id: &str,
) -> String {
    if existing.chain_id.is_empty() {
        proposed_chain_id.to_string()
    } else {
        existing.chain_id.clone()
    }
}

fn seed_host_response(
    table: &LocalDiscoveryTable,
    request: &SeedHostCreatorRequest,
    chain_id: String,
    genesis: bool,
    forced: bool,
    idempotent: bool,
) -> SeedHostCreatorResponse {
    SeedHostCreatorResponse {
        host_creator_id: request.host_creator_id.clone(),
        self_onboarding_state: table.self_onboarding_state,
        host_role_state: table.host_role_state,
        seeded_bridge_id: request.exit_bridge_a_entry.bridge_id.clone(),
        publisher_node_id: request.publisher_entry.node_id.clone(),
        chain_id,
        genesis,
        forced,
        idempotent,
    }
}

fn seed_new_response(
    table: &LocalDiscoveryTable,
    request: &SeedNewCreatorRequest,
    chain_id: String,
    bootstrap_session_id: Option<String>,
    started_bootstrap: bool,
    idempotent: bool,
) -> SeedNewCreatorResponse {
    SeedNewCreatorResponse {
        new_creator_id: request.new_creator_id.clone(),
        host_creator_id: request.host_creator_entry.node_id.clone(),
        self_onboarding_state: table.self_onboarding_state,
        chain_id,
        bootstrap_session_id,
        started_bootstrap,
        forced: request.force,
        idempotent,
    }
}

fn begin_new_creator_bootstrap(
    state: &AdminState,
    store: &LocalDhtStore,
    request: &SeedNewCreatorRequest,
    chain_id: &str,
    now_ms: u64,
) -> Result<(LocalDiscoveryTable, String), (u16, &'static str, String)> {
    let config = state.creator.as_ref().ok_or_else(|| {
        (
            501,
            "not_supported",
            "seed-new-creator start_bootstrap requires a configured creator identity".to_string(),
        )
    })?;
    let host_admin_url = request
        .host_admin_url
        .clone()
        .unwrap_or_else(|| format!("http://{}:9090", request.host_creator_entry.ip_addr));
    let request_id = format!("{chain_id}-join");
    emit_join_path_event(
        "new_creator_join_started",
        chain_id,
        &request.new_creator_id,
        &request.host_creator_entry.node_id,
        None,
        None,
    );
    let relay_request = AuthorityApiRequest::sign(
        AuthorityApiRequestUnsigned {
            chain_id: chain_id.to_string(),
            request_id: request_id.clone(),
            sent_at_ms: now_ms.saturating_sub(25),
            actor_id: request.new_creator_id.clone(),
            body: HostJoinRelayBody {
                host_creator_id: request.host_creator_entry.node_id.clone(),
                creator: PendingCreator {
                    node_id: request.new_creator_id.clone(),
                    ip_addr: state
                        .node_metadata
                        .ip_addr
                        .clone()
                        .unwrap_or_else(|| config.creator_ip_addr.clone()),
                    pub_key: PublicKeyBytes::from_verifying_key(
                        &config.signing_key.verifying_key(),
                    ),
                    udp_punch_port: state
                        .node_metadata
                        .creator_udp_punch_port
                        .unwrap_or(config.udp_punch_port),
                },
                now_ms,
            },
        },
        &config.signing_key,
    )
    .map_err(|error| {
        (
            500,
            "new_creator_join_sign_failed",
            format!("failed to sign NewCreator host-join envelope: {error}"),
        )
    })?;
    let response = post_json_raw(
        &host_admin_url,
        AuthorityRoute::AdminHostJoinRelay.path(),
        &relay_request,
        config.timeout,
    )
    .map_err(|error| {
        mark_new_creator_failed(store, chain_id, &error);
        (
            502,
            "host_creator_unreachable",
            format!("failed to send join request to HostCreator: {error}"),
        )
    })?;
    let (status, response_body) = parse_http_status_body(&response).map_err(|error| {
        mark_new_creator_failed(store, chain_id, &error);
        (502, "host_creator_bad_response", error)
    })?;
    if status != 200 {
        let message = serde_json::from_slice::<AdminErrorResponse>(response_body)
            .map(|error| format!("{}: {}", error.error.code, error.error.message))
            .unwrap_or_else(|_| format!("HostCreator returned HTTP {status}"));
        if message.contains("bootstrap payload insufficient bridges") {
            mark_new_creator_failed_with_state(
                store,
                chain_id,
                SelfOnboardingState::FanoutFailed,
                "fanout_failed",
                &message,
            );
        } else {
            mark_new_creator_failed(store, chain_id, &message);
        }
        return Err((502, "host_creator_join_failed", message));
    }
    let relay =
        serde_json::from_slice::<HostJoinRelayResponse>(response_body).map_err(|error| {
            let message = format!("failed to decode HostCreator relay response: {error}");
            mark_new_creator_failed(store, chain_id, &message);
            (502, "host_creator_bad_response", message)
        })?;
    if relay.chain_id != chain_id {
        let message = format!(
            "HostCreator relay response chain_id mismatch: expected `{chain_id}`, got `{}`",
            relay.chain_id
        );
        mark_new_creator_failed(store, chain_id, &message);
        return Err((502, "host_creator_bad_response", message));
    }
    if relay.new_creator_id != request.new_creator_id {
        let message = format!(
            "HostCreator relay response creator mismatch: expected `{}`, got `{}`",
            request.new_creator_id, relay.new_creator_id
        );
        mark_new_creator_failed(store, chain_id, &message);
        return Err((502, "host_creator_bad_response", message));
    }
    if let Err(error) = relay
        .bootstrap_reply
        .verify_authority(&config.publisher_pub, now_ms)
    {
        let message = format!("Publisher bootstrap payload failed validation: {error}");
        mark_new_creator_failed(store, chain_id, &message);
        return Err((502, "publisher_bootstrap_payload_invalid", message));
    }
    let creator_entry = relay
        .bootstrap_reply
        .creator_dht_entry
        .clone()
        .ok_or_else(|| {
            let message = "Publisher bootstrap payload did not include a signed Creator DHT entry"
                .to_string();
            mark_new_creator_failed(store, chain_id, &message);
            (502, "bootstrap_payload_missing_creator_entry", message)
        })?;
    let bridge_set = relay.bootstrap_reply.bridge_set.clone().ok_or_else(|| {
        let message =
            "Publisher bootstrap payload did not include the signed bridge set".to_string();
        mark_new_creator_failed(store, chain_id, &message);
        (502, "bootstrap_payload_missing_bridge_set", message)
    })?;
    if bridge_set.bridge_dht_entries.is_empty() {
        let message =
            "Publisher bootstrap payload did not include signed bridge DHT entries".to_string();
        mark_new_creator_failed(store, chain_id, &message);
        return Err((502, "bootstrap_payload_missing_bridge_dht_entries", message));
    }
    let seed_bridge_id = relay.bootstrap_reply.response.seed_bridge.node_id.clone();
    if !bridge_set
        .bridge_dht_entries
        .iter()
        .any(|entry| entry.bridge_id == seed_bridge_id)
    {
        let message =
            "Publisher bootstrap payload bridge set did not include the selected seed bridge"
                .to_string();
        mark_new_creator_failed(store, chain_id, &message);
        return Err((502, "bootstrap_payload_missing_seed_bridge", message));
    }
    emit_join_path_event(
        "new_creator_bootstrap_response_received",
        chain_id,
        &request.new_creator_id,
        &request.host_creator_entry.node_id,
        Some(&relay.relay_bridge_id),
        Some(&relay.bootstrap_session_id),
    );
    emit_join_path_event(
        "seed_bridge_payload_received",
        chain_id,
        &request.new_creator_id,
        &request.host_creator_entry.node_id,
        Some(&seed_bridge_id),
        Some(&relay.bootstrap_session_id),
    );
    emit_join_path_event(
        "seed_bridge_punch_started",
        chain_id,
        &request.new_creator_id,
        &request.host_creator_entry.node_id,
        Some(&seed_bridge_id),
        Some(&relay.bootstrap_session_id),
    );
    emit_join_path_event(
        "seed_bridge_punch_progress_publisher",
        chain_id,
        &request.new_creator_id,
        &request.host_creator_entry.node_id,
        Some(&seed_bridge_id),
        Some(&relay.bootstrap_session_id),
    );
    emit_join_path_event(
        "new_creator_seed_tunnel_ack",
        chain_id,
        &request.new_creator_id,
        &request.host_creator_entry.node_id,
        Some(&seed_bridge_id),
        Some(&relay.bootstrap_session_id),
    );
    emit_join_path_event(
        "new_creator_punch_progress_publisher",
        chain_id,
        &request.new_creator_id,
        &request.host_creator_entry.node_id,
        Some(&seed_bridge_id),
        Some(&relay.bootstrap_session_id),
    );
    emit_join_path_event(
        "new_creator_bridge_set_requested",
        chain_id,
        &request.new_creator_id,
        &request.host_creator_entry.node_id,
        Some(&seed_bridge_id),
        Some(&relay.bootstrap_session_id),
    );
    emit_join_path_event(
        "seed_bridge_bridge_set_returned",
        chain_id,
        &request.new_creator_id,
        &request.host_creator_entry.node_id,
        Some(&seed_bridge_id),
        Some(&relay.bootstrap_session_id),
    );
    let mut table = store.snapshot();
    let mut bridge_entries = bridge_set.bridge_dht_entries.clone();
    for entry in &mut bridge_entries {
        entry.active = true;
    }
    table.self_onboarding_state = SelfOnboardingState::Onboarded;
    table.current_bootstrap_session = Some(BootstrapSession {
        session_id: relay.bootstrap_session_id.clone(),
        chain_id: Some(chain_id.to_string()),
        started_at_ms: now_ms,
        last_event_ms: now_ms,
        last_state: "onboarded".to_string(),
    });
    table.creator_entry = Some(creator_entry);
    table.bridge_entries = bridge_entries.clone();
    table.active_tunnels.retain(|tunnel| {
        tunnel.bootstrap_session_id.as_deref() != Some(relay.bootstrap_session_id.as_str())
    });
    for entry in &bridge_entries {
        table.active_tunnels.push(TunnelState {
            peer_id: entry.bridge_id.clone(),
            peer_role: TunnelPeerRole::ExitBridge,
            established_at_ms: now_ms,
            last_seen_ms: now_ms,
            bootstrap_session_id: Some(relay.bootstrap_session_id.clone()),
        });
        emit_join_path_event(
            "new_creator_bridge_entry_active",
            chain_id,
            &request.new_creator_id,
            &request.host_creator_entry.node_id,
            Some(&entry.bridge_id),
            Some(&relay.bootstrap_session_id),
        );
    }
    table.last_update_ms = now_ms;
    table.last_error = None;
    emit_join_path_event(
        "new_creator_local_dht_updated",
        chain_id,
        &request.new_creator_id,
        &request.host_creator_entry.node_id,
        Some(&seed_bridge_id),
        Some(&relay.bootstrap_session_id),
    );
    emit_join_path_event(
        "publisher_remaining_bridges_triggered",
        chain_id,
        &request.new_creator_id,
        &request.host_creator_entry.node_id,
        Some(&seed_bridge_id),
        Some(&relay.bootstrap_session_id),
    );
    store
        .replace(table)
        .map(|table| (table, relay.bootstrap_session_id))
        .map_err(|error| (500, "local_dht_bootstrap_update_failed", error.to_string()))
}

fn mark_new_creator_failed(store: &LocalDhtStore, chain_id: &str, error: &str) {
    mark_new_creator_failed_with_state(
        store,
        chain_id,
        SelfOnboardingState::SeedTunnelFailed,
        "failed",
        error,
    );
}

fn mark_new_creator_failed_with_state(
    store: &LocalDhtStore,
    chain_id: &str,
    state: SelfOnboardingState,
    last_state: &str,
    error: &str,
) {
    let now_ms = now_ms();
    let mut table = store.snapshot();
    table.self_onboarding_state = state;
    table.current_bootstrap_session = Some(BootstrapSession {
        session_id: format!("{chain_id}-failed"),
        chain_id: Some(chain_id.to_string()),
        started_at_ms: now_ms,
        last_event_ms: now_ms,
        last_state: last_state.to_string(),
    });
    table.last_update_ms = now_ms;
    table.last_error = Some(error.to_string());
    let _ = store.replace(table);
}

fn new_creator_seed_state_allows_seed(state: SelfOnboardingState) -> bool {
    matches!(
        state,
        SelfOnboardingState::None
            | SelfOnboardingState::SeedTunnelFailed
            | SelfOnboardingState::FanoutFailed
    )
}

fn new_seed_matches(existing: &NewCreatorSeedState, request: &SeedNewCreatorRequest) -> bool {
    existing.new_creator_actor_id == request.new_creator_id
        && existing.host_creator_entry == request.host_creator_entry
        && existing.start_bootstrap == request.start_bootstrap
}

fn existing_new_seed_chain_id(existing: &NewCreatorSeedState, proposed_chain_id: &str) -> String {
    if existing.chain_id.is_empty() {
        proposed_chain_id.to_string()
    } else {
        existing.chain_id.clone()
    }
}

fn host_creator_entry_validation_error(error: ProtocolError) -> Vec<u8> {
    match error {
        ProtocolError::Expired { .. } => error_response(
            409,
            "host_creator_expired",
            "HostCreator DHT entry has expired",
        ),
        _ => error_response(
            409,
            "host_creator_signature_invalid",
            "HostCreator DHT entry is not signed by the configured Publisher trust root",
        ),
    }
}

fn emit_host_seed_event(
    event: &'static str,
    chain_id: &str,
    host_creator_id: &str,
    bridge_id: &str,
    publisher_node_id: &str,
    bootstrap_genesis: bool,
    forced: bool,
) {
    let _chain_span = metrics_otlp::chain_span("admin_seed_host_creator", chain_id).entered();
    metrics_otlp::record_chain_id(chain_id);
    tracing::info!(
        event,
        chain_id,
        host_creator_id,
        seeded_bridge_id = bridge_id,
        publisher_node_id,
        genesis = bootstrap_genesis,
        forced
    );
}

fn emit_host_seed_warn_event(
    event: &'static str,
    chain_id: &str,
    host_creator_id: &str,
    bridge_id: &str,
    publisher_node_id: &str,
    bootstrap_genesis: bool,
    forced: bool,
) {
    let _chain_span = metrics_otlp::chain_span("admin_seed_host_creator", chain_id).entered();
    metrics_otlp::record_chain_id(chain_id);
    tracing::warn!(
        event,
        chain_id,
        host_creator_id,
        seeded_bridge_id = bridge_id,
        publisher_node_id,
        genesis = bootstrap_genesis,
        forced
    );
}

fn emit_new_seed_event(
    event: &'static str,
    chain_id: &str,
    new_creator_id: &str,
    host_creator_id: &str,
    relay_bridge_id: Option<&str>,
    bootstrap_session_id: Option<&str>,
    forced: bool,
) {
    let _chain_span = metrics_otlp::chain_span("admin_seed_new_creator", chain_id).entered();
    metrics_otlp::record_chain_id(chain_id);
    tracing::info!(
        event,
        chain_id,
        new_creator_id,
        host_creator_id,
        relay_bridge_id = relay_bridge_id.unwrap_or("unknown"),
        bootstrap_session_id = bootstrap_session_id.unwrap_or("none"),
        forced
    );
}

fn emit_join_path_event(
    event: &'static str,
    chain_id: &str,
    new_creator_id: &str,
    host_creator_id: &str,
    relay_bridge_id: Option<&str>,
    bootstrap_session_id: Option<&str>,
) {
    let _chain_span = metrics_otlp::chain_span("creator_bootstrap_join", chain_id).entered();
    metrics_otlp::record_chain_id(chain_id);
    tracing::info!(
        event,
        chain_id,
        new_creator_id,
        host_creator_id,
        relay_bridge_id = relay_bridge_id.unwrap_or("unknown"),
        bootstrap_session_id = bootstrap_session_id.unwrap_or("none")
    );
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedHttpUrl {
    host: String,
    port: u16,
}

fn post_json_raw<T>(
    base_url: &str,
    path: &str,
    payload: &T,
    timeout: Duration,
) -> Result<Vec<u8>, String>
where
    T: Serialize,
{
    let endpoint = parse_http_url(base_url)?;
    let address = resolve_http_url(&endpoint)?;
    let body = serde_json::to_vec(payload)
        .map_err(|error| format!("failed to serialize json request: {error}"))?;
    let mut stream = TcpStream::connect_timeout(&address, timeout)
        .map_err(|error| format!("failed to connect to {}: {error}", base_url))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| format!("failed to set read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| format!("failed to set write timeout: {error}"))?;
    let request_head = format!(
        "POST {path} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        endpoint.host,
        endpoint.port,
        body.len()
    );
    stream
        .write_all(request_head.as_bytes())
        .map_err(|error| format!("failed to write request headers: {error}"))?;
    stream
        .write_all(&body)
        .map_err(|error| format!("failed to write request body: {error}"))?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|error| format!("failed to shutdown request write side: {error}"))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|error| format!("failed to read response: {error}"))?;
    Ok(response)
}

fn parse_http_url(base_url: &str) -> Result<ParsedHttpUrl, String> {
    let trimmed = base_url.trim();
    let without_scheme = trimmed
        .strip_prefix("http://")
        .ok_or_else(|| format!("only plain http:// URLs are supported, got `{trimmed}`"))?;
    let authority = without_scheme
        .split('/')
        .next()
        .ok_or_else(|| format!("invalid URL `{trimmed}`"))?;
    let mut parts = authority.rsplitn(2, ':');
    let port = parts
        .next()
        .ok_or_else(|| format!("URL `{trimmed}` is missing a port"))?
        .parse::<u16>()
        .map_err(|error| format!("invalid URL port in `{trimmed}`: {error}"))?;
    let host = parts
        .next()
        .ok_or_else(|| format!("URL `{trimmed}` is missing a host"))?
        .trim()
        .to_string();
    if host.is_empty() {
        return Err(format!("URL `{trimmed}` has an empty host"));
    }
    Ok(ParsedHttpUrl { host, port })
}

fn resolve_http_url(endpoint: &ParsedHttpUrl) -> Result<SocketAddr, String> {
    let mut addresses = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .map_err(|error| format!("failed to resolve {}: {error}", endpoint.host))?;
    addresses
        .next()
        .ok_or_else(|| format!("no socket addresses resolved for {}", endpoint.host))
}

fn parse_http_status_body(response: &[u8]) -> Result<(u16, &[u8]), String> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "response headers were not terminated".to_string())?;
    let header = std::str::from_utf8(&response[..header_end])
        .map_err(|error| format!("response headers were not utf-8: {error}"))?;
    let status = header
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| "response status line was malformed".to_string())?
        .parse::<u16>()
        .map_err(|error| format!("response status code was invalid: {error}"))?;
    Ok((status, &response[header_end + 4..]))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis() as u64
}

fn split_path_and_query(path: &str) -> (&str, Option<&str>) {
    match path.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (path, None),
    }
}

fn env_u16(name: &str) -> Option<u16> {
    std::env::var(name).ok()?.parse().ok()
}

fn env_public_key_hex() -> Option<String> {
    std::env::var("GBN_BRIDGE_PUBLISHER_PUBLIC_KEY_HEX")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn parse_frames_query(query: Option<&str>) -> Result<FramesQuery, String> {
    let mut chain_id = None;
    let mut limit = DEFAULT_FRAME_LIMIT;

    let Some(query) = query else {
        return Ok(FramesQuery { chain_id, limit });
    };

    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "chain_id" if !value.is_empty() => chain_id = Some(value.to_string()),
            "limit" if !value.is_empty() => {
                limit = value
                    .parse::<usize>()
                    .map_err(|_| format!("limit must be a positive integer, got {value:?}"))?;
                if limit == 0 {
                    return Err("limit must be greater than zero".to_string());
                }
            }
            _ => {}
        }
    }

    Ok(FramesQuery { chain_id, limit })
}

fn admin_bridge_command_target(path: &str) -> Option<&str> {
    let bridge_id = path
        .strip_prefix("/v1/admin/bridges/")?
        .strip_suffix("/command")?;
    if bridge_id.is_empty() || bridge_id.contains('/') {
        None
    } else {
        Some(bridge_id)
    }
}

fn admin_bridge_dht_entry_target(path: &str) -> Option<&str> {
    let bridge_id = path
        .strip_prefix("/v1/admin/bridges/")?
        .strip_suffix("/dht-entry")?;
    if bridge_id.is_empty() || bridge_id.contains('/') {
        None
    } else {
        Some(bridge_id)
    }
}

fn read_http_request(stream: &mut TcpStream, request_max_bytes: usize) -> io::Result<HttpRequest> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];

    let header_end = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before request completed",
            ));
        }

        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > request_max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request exceeds configured max bytes",
            ));
        }

        if let Some(index) = find_header_end(&buffer) {
            break index;
        }
    };

    let headers = std::str::from_utf8(&buffer[..header_end])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "request headers must be utf-8"))?;
    let mut lines = headers.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request line"))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request method"))?
        .to_string();
    let path = request_parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request path"))?
        .to_string();

    let content_length = lines
        .find_map(|line| {
            let mut parts = line.splitn(2, ':');
            let key = parts.next()?.trim();
            let value = parts.next()?.trim();
            if key.eq_ignore_ascii_case("content-length") {
                value.parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);

    let body_start = header_end + 4;
    while buffer.len() < body_start + content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before request body completed",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > request_max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request exceeds configured max bytes",
            ));
        }
    }

    Ok(HttpRequest {
        method,
        path,
        body: buffer[body_start..body_start + content_length].to_vec(),
    })
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn json_response<T>(status_code: u16, payload: &T) -> Vec<u8>
where
    T: Serialize,
{
    let body = serde_json::to_vec(payload).expect("admin response should serialize");
    raw_response(status_code, body)
}

fn error_response(status_code: u16, code: &str, message: &str) -> Vec<u8> {
    json_response(
        status_code,
        &AdminErrorResponse {
            error: AdminErrorBody {
                code: code.to_string(),
                message: message.to_string(),
            },
        },
    )
}

fn service_error_response(error: ServiceError) -> Vec<u8> {
    error_response(error.http_status(), error.code(), error.message())
}

fn creator_error_response(error: CreatorError) -> Vec<u8> {
    let status = match &error {
        CreatorError::NoBridgeAssigned => 409,
        CreatorError::BootstrapFailed(_) | CreatorError::FrameUploadFailed(_) => 502,
        CreatorError::Transport { .. } => 502,
        CreatorError::Protocol(_) => 500,
    };
    error_response(status, "send_dummy_failed", &error.to_string())
}

fn raw_response(status_code: u16, body: Vec<u8>) -> Vec<u8> {
    let status_text = match status_code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        409 => "Conflict",
        502 => "Bad Gateway",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        _ => "OK",
    };
    let headers = format!(
        "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut response = headers.into_bytes();
    response.extend_from_slice(&body);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_states_expose_phase0_node_roles() {
        assert_eq!(AdminState::stub().node_metadata.role, "publisher");

        let receiver = AdminState::receiver(Arc::new(Mutex::new(ReceiverMetrics::default())));
        assert_eq!(receiver.node_metadata.role, "publisher");

        let bridge = AdminState::bridge(Arc::new(Mutex::new(BridgeMetrics::default())));
        assert_eq!(bridge.node_metadata.role, "exit_bridge");

        let creator = AdminState::creator(
            AdminNodeMetadata::from_env("creator-host", "creator"),
            LocalDhtStore::start(
                "creator-host",
                "/tmp/local_dht.json",
                gbn_bridge_protocol::LocalDiscoveryTable::empty("creator-host", 1_000),
            ),
        );
        assert_eq!(creator.node_metadata.role, "creator");
        let AdminLocalDhtSource::Creator(store) = creator.local_dht else {
            panic!("expected creator local DHT source");
        };
        assert_eq!(
            store.snapshot().self_onboarding_state,
            gbn_bridge_protocol::SelfOnboardingState::None
        );
    }
}
