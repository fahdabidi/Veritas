use std::env;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_creator::{CreatorBridgeRequest, CreatorBridgeResponse};
use gbn_bridge_protocol::{
    publisher_identity, BridgeCapability, BridgeCommandAckStatus, BridgeIngressEndpoint,
    BridgeLease, PublicKeyBytes, ReachabilityClass,
};
use gbn_bridge_publisher::{
    admin::{admin_bind_addr_from_env, AdminCreatorConfig, AdminHttpServer, AdminState},
    metrics_emitter::{cloudwatch_metrics_enabled, spawn_cloudwatch_emitter, MetricsEmitterConfig},
    metrics_http::MetricsHttpServer,
    metrics_otlp,
    metrics_prometheus::{bridge_metrics_text, stack_from_env},
    BridgeMetrics,
};
use gbn_bridge_runtime::{
    default_chain_id, default_request_id, BridgeControlClient, ExitBridgeConfig, ExitBridgeRuntime,
    ForwarderClient, HttpJsonTransport, HttpTransportConfig, PublisherApiClient, RuntimeError,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const DEFAULT_AUTHORITY_URL: &str = "http://127.0.0.1:8080";
const DEFAULT_RECEIVER_URL: &str = "http://127.0.0.1:8081";
const DEFAULT_CONTROL_URL: &str = "ws://127.0.0.1:8080/v1/bridge/control";
const DEFAULT_NODE_ID: &str = "exit-bridge";
const DEFAULT_INGRESS_HOST: &str = "127.0.0.1";
const STARTUP_RETRY_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_SIGNING_KEY_HEX: &str = "11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11\
11";
const DEFAULT_PUBLISHER_SIGNING_KEY_HEX: &str = "09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09\
09";

fn main() {
    if let Err(error) = run() {
        eprintln!("exit-bridge startup error: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let config = BridgeServiceConfig::from_env()?;
    let otlp_service_name =
        env::var("GBN_BRIDGE_OTLP_SERVICE_NAME").unwrap_or_else(|_| config.node_id.clone());
    let _otlp_guard = metrics_otlp::init_otlp_tracing_from_env(&otlp_service_name)?;
    let signing_key = config.load_signing_key()?;
    let publisher_public_key = config.load_publisher_public_key()?;
    let bridge_identity = PublicKeyBytes::from_verifying_key(&signing_key.verifying_key());
    let metrics = Arc::new(Mutex::new(BridgeMetrics::default()));
    let admin_addr = admin_bind_addr_from_env()?;
    let admin_creator = AdminCreatorConfig {
        actor_id: config.node_id.clone(),
        signing_key: signing_key.clone(),
        publisher_pub: publisher_public_key.clone(),
        authority_url: config.authority_url.clone(),
        creator_ip_addr: config.ingress_host.clone(),
        udp_punch_port: config.punch_port,
        timeout: Duration::from_secs(5),
    };
    let admin_server = AdminHttpServer::bind(
        admin_addr,
        AdminState::bridge_with_creator(metrics.clone(), admin_creator),
        1_048_576,
    )
    .map_err(|error| error.to_string())?;
    let _admin_handle = admin_server.spawn().map_err(|error| error.to_string())?;
    let prometheus_stack = stack_from_env();
    let metrics_for_prometheus = metrics.clone();
    let prometheus_addr: SocketAddr = env::var("GBN_BRIDGE_METRICS_BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:9100".to_string())
        .parse()
        .map_err(|_| "GBN_BRIDGE_METRICS_BIND_ADDR must be a valid socket address".to_string())?;
    let prometheus_server = MetricsHttpServer::bind(prometheus_addr, move || {
        let snapshot = metrics_for_prometheus
            .lock()
            .expect("bridge metrics mutex poisoned while rendering prometheus metrics")
            .snapshot();
        bridge_metrics_text(&snapshot, "bridge", &prometheus_stack)
    })
    .map_err(|error| error.to_string())?;
    let prometheus_local_addr = prometheus_server
        .local_addr()
        .map_err(|error| error.to_string())?;
    let _prometheus_handle = prometheus_server
        .spawn()
        .map_err(|error| error.to_string())?;
    let _metrics_handle = if cloudwatch_metrics_enabled() {
        let metrics_for_emitter = metrics.clone();
        Some(spawn_cloudwatch_emitter(
            MetricsEmitterConfig::from_env("bridge"),
            move |service, stack| {
                metrics_for_emitter
                    .lock()
                    .expect("bridge metrics mutex poisoned while emitting metrics")
                    .snapshot()
                    .cloudwatch_data(service, stack)
            },
        ))
    } else {
        None
    };

    let (build_version, build_source, build_created, image) = conduit_build_metadata();
    eprintln!(
        "exit-bridge build_version={} build_source={} build_created={} image={}",
        build_version, build_source, build_created, image
    );
    let authority_transport =
        HttpJsonTransport::new(HttpTransportConfig::new(config.authority_url.clone()))
            .map_err(|error| error.to_string())?;
    let receiver_transport =
        HttpJsonTransport::new(HttpTransportConfig::new(config.receiver_url.clone()))
            .map_err(|error| error.to_string())?;

    let publisher_client = PublisherApiClient::new(
        config.node_id.clone(),
        signing_key.clone(),
        publisher_public_key.clone(),
        authority_transport,
    );
    let mut runtime = ExitBridgeRuntime::new(
        ExitBridgeConfig {
            bridge_id: config.node_id.clone(),
            identity_pub: bridge_identity.clone(),
            ingress_endpoint: BridgeIngressEndpoint {
                host: config.ingress_host.clone(),
                port: config.punch_port,
            },
            requested_udp_punch_port: config.punch_port,
            capabilities: vec![
                BridgeCapability::BootstrapSeed,
                BridgeCapability::CatalogRefresh,
                BridgeCapability::SessionRelay,
                BridgeCapability::BatchAssignment,
                BridgeCapability::ProgressReporting,
            ],
        },
        publisher_client,
    );
    runtime.attach_forwarder_client(ForwarderClient::new(
        config.node_id.clone(),
        signing_key.clone(),
        publisher_public_key.clone(),
        receiver_transport,
    ));

    let lease = start_bridge_with_retry(&mut runtime, &config)?;
    eprintln!(
        "exit-bridge node_id={} ingress_host={} udp_punch_port={} lease_id={} authority_url={} receiver_url={}",
        config.node_id,
        config.ingress_host,
        config.punch_port,
        lease.lease_id,
        config.authority_url,
        config.receiver_url
    );
    eprintln!(
        "exit-bridge admin listening on {admin_addr}; prometheus metrics listening on {prometheus_local_addr}"
    );

    let control_client = connect_bridge_control_with_retry(
        &config,
        &lease,
        &bridge_identity,
        &signing_key,
        &publisher_public_key,
        None,
    )?;
    runtime.attach_control_client(control_client);
    metrics
        .lock()
        .expect("bridge metrics mutex poisoned")
        .record_control_reconnect();
    let (creator_upload_tx, creator_upload_rx) = mpsc::channel();
    let _creator_upload_handle =
        spawn_creator_upload_listener(config.punch_port, creator_upload_tx)
            .map_err(|error| error.to_string())?;

    let mut last_keepalive_ms = now_ms();
    loop {
        let current_ms = now_ms();
        handle_pending_creator_uploads(&mut runtime, &metrics, &creator_upload_rx);

        let control_ack = match runtime.receive_next_control_command(current_ms) {
            Ok(ack) => ack,
            Err(error) if is_control_transport_error(&error) => {
                try_reconnect_bridge_control(
                    &mut runtime,
                    &metrics,
                    &config,
                    &bridge_identity,
                    &signing_key,
                    &publisher_public_key,
                    &error,
                );
                last_keepalive_ms = now_ms();
                thread::sleep(Duration::from_millis(config.poll_interval_ms));
                continue;
            }
            Err(error) => return Err(error.to_string()),
        };

        if let Some(ack) = control_ack {
            let _chain_span =
                metrics_otlp::chain_span("bridge_control_command", &ack.chain_id).entered();
            metrics_otlp::record_chain_id(&ack.chain_id);
            metrics
                .lock()
                .expect("bridge metrics mutex poisoned")
                .record_command_ack(matches!(ack.status, BridgeCommandAckStatus::Rejected));
            eprintln!(
                "exit-bridge node_id={} applied command command_id={} seq_no={} chain_id={} status={:?}",
                config.node_id, ack.command_id, ack.seq_no, ack.chain_id, ack.status
            );
        }

        if let Some(lease) = runtime
            .heartbeat_tick(0, current_ms)
            .map_err(|error| error.to_string())?
        {
            eprintln!(
                "exit-bridge node_id={} renewed lease_id={} expires_at_ms={}",
                config.node_id, lease.lease_id, lease.lease_expiry_ms
            );
        }

        if current_ms.saturating_sub(last_keepalive_ms) >= config.keepalive_interval_ms {
            match runtime.send_control_keepalive(current_ms) {
                Ok(()) => {
                    last_keepalive_ms = current_ms;
                }
                Err(error) if is_control_transport_error(&error) => {
                    try_reconnect_bridge_control(
                        &mut runtime,
                        &metrics,
                        &config,
                        &bridge_identity,
                        &signing_key,
                        &publisher_public_key,
                        &error,
                    );
                    last_keepalive_ms = now_ms();
                }
                Err(error) => return Err(error.to_string()),
            }
        }

        thread::sleep(Duration::from_millis(config.poll_interval_ms));
    }
}

fn start_bridge_with_retry(
    runtime: &mut ExitBridgeRuntime,
    config: &BridgeServiceConfig,
) -> Result<BridgeLease, String> {
    let started_at_ms = now_ms();
    loop {
        match runtime.startup(config.reachability_class.clone(), now_ms()) {
            Ok(lease) => return Ok(lease),
            Err(error) if is_startup_retryable(&error) => {
                if now_ms().saturating_sub(started_at_ms) >= STARTUP_RETRY_TIMEOUT_MS {
                    return Err(error.to_string());
                }
                eprintln!(
                    "exit-bridge node_id={} waiting for authority during startup: {error}",
                    config.node_id
                );
                thread::sleep(Duration::from_millis(config.poll_interval_ms.max(250)));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn connect_bridge_control_with_retry(
    config: &BridgeServiceConfig,
    lease: &BridgeLease,
    bridge_identity: &PublicKeyBytes,
    signing_key: &SigningKey,
    publisher_public_key: &PublicKeyBytes,
    resume_acked_seq_no: Option<u64>,
) -> Result<BridgeControlClient, String> {
    let started_at_ms = now_ms();
    loop {
        match connect_bridge_control(
            config,
            lease,
            bridge_identity,
            signing_key,
            publisher_public_key,
            resume_acked_seq_no,
        ) {
            Ok(client) => return Ok(client),
            Err(error) => {
                if now_ms().saturating_sub(started_at_ms) >= STARTUP_RETRY_TIMEOUT_MS {
                    return Err(error);
                }
                eprintln!(
                    "exit-bridge node_id={} waiting for control endpoint during startup: {error}",
                    config.node_id
                );
                thread::sleep(Duration::from_millis(config.poll_interval_ms.max(250)));
            }
        }
    }
}

fn connect_bridge_control(
    config: &BridgeServiceConfig,
    lease: &BridgeLease,
    bridge_identity: &PublicKeyBytes,
    signing_key: &SigningKey,
    publisher_public_key: &PublicKeyBytes,
    resume_acked_seq_no: Option<u64>,
) -> Result<BridgeControlClient, String> {
    let connected_at_ms = now_ms();
    let control_chain_id =
        default_chain_id("bridge-control-connect", &config.node_id, &lease.lease_id);
    metrics_otlp::record_chain_id(&control_chain_id);
    let control_request_id = default_request_id("control-hello", &config.node_id, connected_at_ms);
    BridgeControlClient::connect(
        &config.control_url,
        &config.node_id,
        &lease.lease_id,
        bridge_identity,
        signing_key,
        publisher_public_key,
        &control_chain_id,
        &control_request_id,
        connected_at_ms,
        resume_acked_seq_no,
        config.control_max_skew_ms,
    )
    .map_err(|error| error.to_string())
}

fn try_reconnect_bridge_control(
    runtime: &mut ExitBridgeRuntime,
    metrics: &Arc<Mutex<BridgeMetrics>>,
    config: &BridgeServiceConfig,
    bridge_identity: &PublicKeyBytes,
    signing_key: &SigningKey,
    publisher_public_key: &PublicKeyBytes,
    cause: &RuntimeError,
) {
    eprintln!(
        "exit-bridge node_id={} reconnecting control session after {cause}",
        config.node_id
    );
    let Some(lease) = runtime.current_lease().cloned() else {
        eprintln!(
            "exit-bridge node_id={} cannot reconnect control session without an active lease",
            config.node_id
        );
        return;
    };
    let resume_acked_seq_no = runtime
        .control_client()
        .and_then(|client| client.last_acked_seq_no());
    match connect_bridge_control(
        config,
        &lease,
        bridge_identity,
        signing_key,
        publisher_public_key,
        resume_acked_seq_no,
    ) {
        Ok(control_client) => {
            runtime.attach_control_client(control_client);
            metrics
                .lock()
                .expect("bridge metrics mutex poisoned")
                .record_control_reconnect();
            eprintln!(
                "exit-bridge node_id={} reconnected control session",
                config.node_id
            );
        }
        Err(error) => {
            eprintln!(
                "exit-bridge node_id={} control reconnect attempt failed: {error}",
                config.node_id
            );
        }
    }
}

fn is_control_transport_error(error: &RuntimeError) -> bool {
    matches!(
        error,
        RuntimeError::ControlTransport { .. } | RuntimeError::MissingControlClient
    )
}

fn is_startup_retryable(error: &RuntimeError) -> bool {
    matches!(error, RuntimeError::AuthorityTransport { .. })
}

fn spawn_creator_upload_listener(
    punch_port: u16,
    work_tx: Sender<CreatorUploadWork>,
) -> io::Result<thread::JoinHandle<()>> {
    let socket = UdpSocket::bind(("0.0.0.0", punch_port))?;
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;
    Ok(thread::spawn(move || {
        let mut buffer = vec![0_u8; 60 * 1024];
        loop {
            let (read, peer) = match socket.recv_from(&mut buffer) {
                Ok(received) => received,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(error) => {
                    eprintln!("exit-bridge creator upload listener error: {error}");
                    continue;
                }
            };
            let request = match serde_json::from_slice::<CreatorBridgeRequest>(&buffer[..read]) {
                Ok(request) => request,
                Err(error) => {
                    let response = CreatorBridgeResponse::Error {
                        message: format!("invalid creator upload packet: {error}"),
                    };
                    if let Ok(payload) = serde_json::to_vec(&response) {
                        let _ = socket.send_to(&payload, peer);
                    }
                    continue;
                }
            };
            let (response_tx, response_rx) = mpsc::channel();
            if work_tx
                .send(CreatorUploadWork {
                    request,
                    response_tx,
                })
                .is_err()
            {
                let response = CreatorBridgeResponse::Error {
                    message: "bridge upload worker is unavailable".to_string(),
                };
                if let Ok(payload) = serde_json::to_vec(&response) {
                    let _ = socket.send_to(&payload, peer);
                }
                continue;
            }
            let response = match response_rx.recv_timeout(Duration::from_secs(5)) {
                Ok(response) => response,
                Err(error) => CreatorBridgeResponse::Error {
                    message: format!("bridge upload worker timed out: {error}"),
                },
            };
            let payload = match serde_json::to_vec(&response) {
                Ok(payload) => payload,
                Err(error) => {
                    eprintln!("exit-bridge creator upload response serialization error: {error}");
                    continue;
                }
            };
            let _ = socket.send_to(&payload, peer);
        }
    }))
}

struct CreatorUploadWork {
    request: CreatorBridgeRequest,
    response_tx: Sender<CreatorBridgeResponse>,
}

fn handle_pending_creator_uploads(
    runtime: &mut ExitBridgeRuntime,
    metrics: &Arc<Mutex<BridgeMetrics>>,
    work_rx: &Receiver<CreatorUploadWork>,
) {
    while let Ok(work) = work_rx.try_recv() {
        let response = handle_creator_upload_request(work.request, runtime, metrics);
        let _ = work.response_tx.send(response);
    }
}

fn handle_creator_upload_request(
    request: CreatorBridgeRequest,
    runtime: &mut ExitBridgeRuntime,
    metrics: &Arc<Mutex<BridgeMetrics>>,
) -> CreatorBridgeResponse {
    match request {
        CreatorBridgeRequest::Open(open) => {
            let chain_id = open.chain_id.clone();
            let _chain_span = metrics_otlp::chain_span("bridge_creator_open", &chain_id).entered();
            metrics_otlp::record_chain_id(&chain_id);
            let session_id = open.session_id.clone();
            let now = now_ms();
            match runtime.open_data_session_with_chain_id(&chain_id, open, now) {
                Ok(()) => {
                    eprintln!(
                        "exit-bridge creator upload opened session_id={} chain_id={}",
                        session_id, chain_id
                    );
                    CreatorBridgeResponse::Opened {
                        chain_id,
                        session_id,
                    }
                }
                Err(error) => CreatorBridgeResponse::Error {
                    message: error.to_string(),
                },
            }
        }
        CreatorBridgeRequest::Frame(frame) => {
            let chain_id = frame.chain_id.clone();
            let _chain_span = metrics_otlp::chain_span("bridge_creator_frame", &chain_id).entered();
            metrics_otlp::record_chain_id(&chain_id);
            let bytes = frame.ciphertext.len();
            let now = now_ms();
            match runtime.forward_session_frame_with_chain_id(&chain_id, frame, now) {
                Ok(ack) => {
                    metrics
                        .lock()
                        .expect("bridge metrics mutex poisoned")
                        .record_frame_forwarded(bytes);
                    eprintln!(
                        "exit-bridge creator upload forwarded session_id={} sequence={} chain_id={} status={:?}",
                        ack.session_id, ack.acked_sequence, ack.chain_id, ack.status
                    );
                    CreatorBridgeResponse::Ack(ack)
                }
                Err(error) => CreatorBridgeResponse::Error {
                    message: error.to_string(),
                },
            }
        }
        CreatorBridgeRequest::Close(close) => {
            let chain_id = close.chain_id.clone();
            let _chain_span = metrics_otlp::chain_span("bridge_creator_close", &chain_id).entered();
            metrics_otlp::record_chain_id(&chain_id);
            let session_id = close.session_id.clone();
            let now = now_ms();
            match runtime.close_data_session_with_chain_id(&chain_id, close, now) {
                Ok(()) => {
                    eprintln!(
                        "exit-bridge creator upload closed session_id={} chain_id={}",
                        session_id, chain_id
                    );
                    CreatorBridgeResponse::Closed {
                        chain_id,
                        session_id,
                    }
                }
                Err(error) => CreatorBridgeResponse::Error {
                    message: error.to_string(),
                },
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BridgeServiceConfig {
    node_id: String,
    ingress_host: String,
    authority_url: String,
    receiver_url: String,
    control_url: String,
    punch_port: u16,
    reachability_class: ReachabilityClass,
    control_max_skew_ms: u64,
    keepalive_interval_ms: u64,
    poll_interval_ms: u64,
    bridge_signing_key_hex: Option<String>,
    bridge_signing_seed_hex: Option<String>,
    publisher_public_key_hex: Option<String>,
    publisher_signing_key_hex: Option<String>,
}

impl BridgeServiceConfig {
    fn from_env() -> Result<Self, String> {
        let node_id_raw = env::var("GBN_BRIDGE_NODE_ID").unwrap_or_else(|_| DEFAULT_NODE_ID.into());
        let ingress_host_raw =
            env::var("GBN_BRIDGE_INGRESS_HOST").unwrap_or_else(|_| DEFAULT_INGRESS_HOST.into());
        let metadata = if node_id_raw == "auto" || ingress_host_raw == "auto" {
            Some(load_ecs_task_metadata().map_err(|error| error.to_string())?)
        } else {
            None
        };

        let node_id = if node_id_raw == "auto" {
            metadata
                .as_ref()
                .map(|metadata| metadata.default_node_id())
                .unwrap_or_else(|| DEFAULT_NODE_ID.to_string())
        } else {
            node_id_raw
        };
        let ingress_host = if ingress_host_raw == "auto" {
            metadata
                .as_ref()
                .and_then(|metadata| metadata.primary_ipv4())
                .unwrap_or_else(|| DEFAULT_INGRESS_HOST.to_string())
        } else {
            ingress_host_raw
        };

        Ok(Self {
            node_id,
            ingress_host,
            authority_url: env::var("GBN_BRIDGE_AUTHORITY_URL")
                .or_else(|_| env::var("GBN_BRIDGE_PUBLISHER_URL"))
                .unwrap_or_else(|_| DEFAULT_AUTHORITY_URL.to_string()),
            receiver_url: env::var("GBN_BRIDGE_RECEIVER_URL")
                .unwrap_or_else(|_| DEFAULT_RECEIVER_URL.to_string()),
            control_url: env::var("GBN_BRIDGE_CONTROL_URL")
                .unwrap_or_else(|_| DEFAULT_CONTROL_URL.to_string()),
            punch_port: parse_env_u16("GBN_BRIDGE_PUNCH_PORT", 443)?,
            reachability_class: parse_reachability_class(
                &env::var("GBN_BRIDGE_REACHABILITY_CLASS").unwrap_or_else(|_| "direct".to_string()),
            )?,
            control_max_skew_ms: parse_env_u64("GBN_BRIDGE_CONTROL_MAX_SKEW_MS", 30_000)?,
            keepalive_interval_ms: parse_env_u64(
                "GBN_BRIDGE_CONTROL_KEEPALIVE_INTERVAL_MS",
                5_000,
            )?,
            poll_interval_ms: parse_env_u64("GBN_BRIDGE_POLL_INTERVAL_MS", 250)?,
            bridge_signing_key_hex: env::var("GBN_BRIDGE_BRIDGE_SIGNING_KEY_HEX").ok(),
            bridge_signing_seed_hex: env::var("GBN_BRIDGE_BRIDGE_SIGNING_SEED_HEX").ok(),
            publisher_public_key_hex: env::var("GBN_BRIDGE_PUBLISHER_PUBLIC_KEY_HEX").ok(),
            publisher_signing_key_hex: env::var("GBN_BRIDGE_PUBLISHER_SIGNING_KEY_HEX").ok(),
        })
    }

    fn load_signing_key(&self) -> Result<SigningKey, String> {
        if let Some(value) = &self.bridge_signing_key_hex {
            return decode_hex_32(value).map(|bytes| SigningKey::from_bytes(&bytes));
        }
        if let Some(value) = &self.bridge_signing_seed_hex {
            return derive_signing_key(value, &self.node_id);
        }

        decode_hex_32(DEFAULT_SIGNING_KEY_HEX).map(|bytes| SigningKey::from_bytes(&bytes))
    }

    fn load_publisher_public_key(&self) -> Result<PublicKeyBytes, String> {
        if let Some(value) = &self.publisher_public_key_hex {
            let bytes = decode_hex_32(value)?;
            return Ok(PublicKeyBytes(bytes.to_vec()));
        }

        if let Some(value) = &self.publisher_signing_key_hex {
            let bytes = decode_hex_32(value)?;
            let signing_key = SigningKey::from_bytes(&bytes);
            return Ok(publisher_identity(&signing_key));
        }

        let bytes = decode_hex_32(DEFAULT_PUBLISHER_SIGNING_KEY_HEX)?;
        Ok(publisher_identity(&SigningKey::from_bytes(&bytes)))
    }
}

#[derive(Debug, Deserialize)]
struct EcsTaskMetadata {
    #[serde(rename = "TaskARN")]
    task_arn: Option<String>,
    #[serde(rename = "Containers", default)]
    containers: Vec<EcsContainerMetadata>,
}

#[derive(Debug, Deserialize)]
struct EcsContainerMetadata {
    #[serde(rename = "Networks", default)]
    networks: Vec<EcsNetworkMetadata>,
}

#[derive(Debug, Deserialize)]
struct EcsNetworkMetadata {
    #[serde(rename = "IPv4Addresses", default)]
    ipv4_addresses: Vec<String>,
}

impl EcsTaskMetadata {
    fn default_node_id(&self) -> String {
        self.task_arn
            .as_ref()
            .and_then(|arn| arn.rsplit('/').next())
            .map(|task_id| format!("exit-bridge-{task_id}"))
            .unwrap_or_else(|| DEFAULT_NODE_ID.to_string())
    }

    fn primary_ipv4(&self) -> Option<String> {
        self.containers
            .iter()
            .flat_map(|container| container.networks.iter())
            .flat_map(|network| network.ipv4_addresses.iter())
            .find(|address| !address.is_empty())
            .cloned()
    }
}

fn load_ecs_task_metadata() -> io::Result<EcsTaskMetadata> {
    let base_url = env::var("ECS_CONTAINER_METADATA_URI_V4").map_err(|_| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "ECS_CONTAINER_METADATA_URI_V4 is required for auto bridge metadata",
        )
    })?;
    let endpoint = parse_http_endpoint(&(base_url.trim_end_matches('/').to_string() + "/task"))?;
    let address = resolve_endpoint(&endpoint.host, endpoint.port)?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
        endpoint.path, endpoint.host, endpoint.port
    );
    stream.write_all(request.as_bytes())?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    let body = extract_http_body(&response)?;
    let body = decode_chunked_body(body).unwrap_or_else(|| body.to_vec());
    serde_json::from_slice(&body).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid ECS task metadata payload: {error}"),
        )
    })
}

#[derive(Debug)]
struct ParsedHttpEndpoint {
    host: String,
    port: u16,
    path: String,
}

fn parse_http_endpoint(url: &str) -> io::Result<ParsedHttpEndpoint> {
    let trimmed = url.trim();
    let without_scheme = trimmed
        .strip_prefix("http://")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "only http:// is supported"))?;
    let mut split = without_scheme.splitn(2, '/');
    let authority = split
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing host"))?;
    let path = format!("/{}", split.next().unwrap_or_default());
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => {
            let port = port
                .parse::<u16>()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid port"))?;
            (host, port)
        }
        _ if !authority.is_empty() => (authority, 80),
        _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, "missing host")),
    };

    Ok(ParsedHttpEndpoint {
        host: host.to_string(),
        port,
        path,
    })
}

fn extract_http_body(response: &[u8]) -> io::Result<&[u8]> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "missing http header terminator")
        })?;
    Ok(&response[header_end + 4..])
}

fn decode_chunked_body(body: &[u8]) -> Option<Vec<u8>> {
    let mut cursor = 0;
    let mut decoded = Vec::new();

    loop {
        let line_end = body[cursor..]
            .windows(2)
            .position(|window| window == b"\r\n")?
            + cursor;
        let size_line = std::str::from_utf8(&body[cursor..line_end]).ok()?;
        let size_hex = size_line.split(';').next()?.trim();
        let size = usize::from_str_radix(size_hex, 16).ok()?;
        cursor = line_end + 2;

        if size == 0 {
            return Some(decoded);
        }
        if body.len() < cursor + size + 2 || &body[cursor + size..cursor + size + 2] != b"\r\n" {
            return None;
        }

        decoded.extend_from_slice(&body[cursor..cursor + size]);
        cursor += size + 2;
    }
}

fn resolve_endpoint(host: &str, port: u16) -> io::Result<SocketAddr> {
    let mut addresses = (host, port).to_socket_addrs()?;
    addresses.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("no addresses resolved for {host}"),
        )
    })
}

fn parse_reachability_class(value: &str) -> Result<ReachabilityClass, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "direct" => Ok(ReachabilityClass::Direct),
        "brokered" => Ok(ReachabilityClass::Brokered),
        "relay_only" | "relay-only" => Ok(ReachabilityClass::RelayOnly),
        other => Err(format!(
            "GBN_BRIDGE_REACHABILITY_CLASS must be direct, brokered, or relay_only, got {other:?}"
        )),
    }
}

fn decode_hex_32(value: &str) -> Result<[u8; 32], String> {
    let trimmed = value.trim();
    if trimmed.len() != 64 {
        return Err(format!(
            "hex value must contain exactly 64 characters, got {}",
            trimmed.len()
        ));
    }

    let mut bytes = [0_u8; 32];
    for (index, chunk) in trimmed.as_bytes().chunks(2).enumerate() {
        let pair =
            std::str::from_utf8(chunk).map_err(|_| "hex value must be valid utf-8".to_string())?;
        bytes[index] =
            u8::from_str_radix(pair, 16).map_err(|_| format!("invalid hex byte {pair:?}"))?;
    }
    Ok(bytes)
}

fn derive_signing_key(seed_hex: &str, node_id: &str) -> Result<SigningKey, String> {
    let seed = decode_hex_32(seed_hex)?;
    let mut hasher = Sha256::new();
    hasher.update(seed);
    hasher.update(node_id.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 32];
    bytes.copy_from_slice(&digest[..32]);
    Ok(SigningKey::from_bytes(&bytes))
}

fn parse_env_u16(key: &str, default: u16) -> Result<u16, String> {
    match env::var(key) {
        Ok(value) => value
            .parse::<u16>()
            .map_err(|_| format!("{key} must be a valid u16, got {value:?}")),
        Err(_) => Ok(default),
    }
}

fn parse_env_u64(key: &str, default: u64) -> Result<u64, String> {
    match env::var(key) {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|_| format!("{key} must be a valid u64, got {value:?}")),
        Err(_) => Ok(default),
    }
}

fn conduit_build_metadata() -> (String, String, String, String) {
    (
        env::var("VERITAS_CONDUIT_BUILD_VERSION").unwrap_or_else(|_| "unknown".to_string()),
        env::var("VERITAS_CONDUIT_BUILD_SOURCE").unwrap_or_else(|_| "unknown".to_string()),
        env::var("VERITAS_CONDUIT_BUILD_CREATED").unwrap_or_else(|_| "unknown".to_string()),
        env::var("VERITAS_CONDUIT_IMAGE").unwrap_or_else(|_| "unknown".to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::decode_chunked_body;
    use super::parse_http_endpoint;

    #[test]
    fn ecs_metadata_endpoint_without_port_uses_http_default_port() {
        let endpoint = parse_http_endpoint("http://169.254.170.2/v4/task").unwrap();

        assert_eq!(endpoint.host, "169.254.170.2");
        assert_eq!(endpoint.port, 80);
        assert_eq!(endpoint.path, "/v4/task");
    }

    #[test]
    fn endpoint_with_explicit_port_keeps_that_port() {
        let endpoint = parse_http_endpoint("http://publisher-authority:8080/health").unwrap();

        assert_eq!(endpoint.host, "publisher-authority");
        assert_eq!(endpoint.port, 8080);
        assert_eq!(endpoint.path, "/health");
    }

    #[test]
    fn chunked_metadata_body_is_decoded_before_json_parsing() {
        let body = b"9\r\n{\"ok\":tru\r\n2\r\ne}\r\n0\r\n\r\n";

        assert_eq!(decode_chunked_body(body).unwrap(), br#"{"ok":true}"#);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis() as u64
}
