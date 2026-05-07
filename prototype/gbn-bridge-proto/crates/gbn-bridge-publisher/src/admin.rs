//! Local admin HTTP surface for Conduit V2 service containers.
//!
//! This listener is intended to bind to 127.0.0.1:9090 inside each container.
//! Operators reach it through ECS exec; it is not exposed through public ingress.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ed25519_dalek::SigningKey;
use gbn_bridge_creator::{CreatorClient, CreatorError, SendDummyResult};
use gbn_bridge_protocol::{BridgeCommandPayload, PublicKeyBytes};
use serde::{Deserialize, Serialize};

use crate::api::AuthorityRoute;
use crate::control::BridgeAdminCommandReceipt;
use crate::metrics::{
    AuthorityMetricsSnapshot, BridgeMetrics, BridgeMetricsSnapshot, ReceiverMetrics,
    ReceiverMetricsSnapshot,
};
use crate::service::{AuthorityService, ServiceError};
use crate::storage::{BridgeRecord, IngestedFrameRecord};

pub const DEFAULT_ADMIN_BIND_ADDR: &str = "127.0.0.1:9090";
const DEFAULT_FRAME_LIMIT: usize = 1_000;
const DEFAULT_SEND_DUMMY_SIZE: usize = 512;
const MAX_SEND_DUMMY_SIZE: usize = 8 * 1024;

#[derive(Debug, Clone)]
pub struct AdminState {
    authority: Option<Arc<Mutex<AuthorityService>>>,
    metrics: AdminMetricsSource,
    creator: Option<AdminCreatorConfig>,
}

#[derive(Debug, Clone)]
enum AdminMetricsSource {
    Authority,
    Receiver(Arc<Mutex<ReceiverMetrics>>),
    Bridge(Arc<Mutex<BridgeMetrics>>),
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
pub struct AdminErrorResponse {
    pub error: AdminErrorBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminErrorBody {
    pub code: String,
    pub message: String,
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
        Self {
            authority: Some(service),
            metrics: AdminMetricsSource::Authority,
            creator: None,
        }
    }

    pub fn authority_with_creator(
        service: Arc<Mutex<AuthorityService>>,
        creator: AdminCreatorConfig,
    ) -> Self {
        Self {
            authority: Some(service),
            metrics: AdminMetricsSource::Authority,
            creator: Some(creator),
        }
    }

    pub fn stub() -> Self {
        Self {
            authority: None,
            metrics: AdminMetricsSource::Authority,
            creator: None,
        }
    }

    pub fn receiver(metrics: Arc<Mutex<ReceiverMetrics>>) -> Self {
        Self {
            authority: None,
            metrics: AdminMetricsSource::Receiver(metrics),
            creator: None,
        }
    }

    pub fn receiver_with_creator(
        metrics: Arc<Mutex<ReceiverMetrics>>,
        creator: AdminCreatorConfig,
    ) -> Self {
        Self {
            authority: None,
            metrics: AdminMetricsSource::Receiver(metrics),
            creator: Some(creator),
        }
    }

    pub fn bridge(metrics: Arc<Mutex<BridgeMetrics>>) -> Self {
        Self {
            authority: None,
            metrics: AdminMetricsSource::Bridge(metrics),
            creator: None,
        }
    }

    pub fn bridge_with_creator(
        metrics: Arc<Mutex<BridgeMetrics>>,
        creator: AdminCreatorConfig,
    ) -> Self {
        Self {
            authority: None,
            metrics: AdminMetricsSource::Bridge(metrics),
            creator: Some(creator),
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
        ("GET", _) => error_response(404, "not_found", "admin route not found"),
        ("POST", path) if path == AuthorityRoute::AdminSendDummy.path() => {
            inject_send_dummy(state, &request.body)
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
        Ok(result) => json_response::<SendDummyResult>(200, &result),
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

fn split_path_and_query(path: &str) -> (&str, Option<&str>) {
    match path.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (path, None),
    }
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
