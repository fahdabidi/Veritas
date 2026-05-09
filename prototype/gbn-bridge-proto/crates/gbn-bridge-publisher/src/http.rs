use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, TryLockError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::Serialize;

use crate::api::{
    AuthorityApiRequest, AuthorityRoute, BootstrapJoinBody, BootstrapProgressBody,
    BridgeHeartbeatBody, BridgeRegisterBody, CreatorCatalogBody, ReceiverCloseBody,
    ReceiverFrameBody, ReceiverOpenBody,
};
use crate::control::{handle_control_connection, looks_like_control_upgrade};
use crate::metrics_otlp;
use crate::metrics_prometheus::{authority_metrics_text, stack_from_env, PROMETHEUS_CONTENT_TYPE};
use crate::service::{AuthorityService, ServiceError};

pub struct AuthorityHttpServer {
    listener: TcpListener,
    service: Arc<Mutex<AuthorityService>>,
    request_max_bytes: usize,
}

pub struct AuthorityHttpServerHandle {
    local_addr: SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<io::Result<()>>>,
}

impl AuthorityHttpServer {
    pub fn bind(
        bind_addr: SocketAddr,
        service: Arc<Mutex<AuthorityService>>,
        request_max_bytes: usize,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(bind_addr)?;
        Ok(Self {
            listener,
            service,
            request_max_bytes,
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn spawn(self) -> io::Result<AuthorityHttpServerHandle> {
        let stop = Arc::new(AtomicBool::new(false));
        let local_addr = self.local_addr()?;
        let stop_for_thread = Arc::clone(&stop);
        let join = thread::spawn(move || self.run_loop(stop_for_thread));
        Ok(AuthorityHttpServerHandle {
            local_addr,
            stop,
            join: Some(join),
        })
    }

    pub fn serve_forever(self) -> io::Result<()> {
        self.listener.set_nonblocking(false)?;
        loop {
            let (stream, _) = self.listener.accept()?;
            let service = Arc::clone(&self.service);
            let request_max_bytes = self.request_max_bytes;
            thread::spawn(move || {
                if let Err(error) = handle_connection(stream, &service, request_max_bytes) {
                    eprintln!("authority connection error: {error}");
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
                    let service = Arc::clone(&self.service);
                    let request_max_bytes = self.request_max_bytes;
                    thread::spawn(move || {
                        if let Err(error) = handle_connection(stream, &service, request_max_bytes) {
                            eprintln!("authority connection error: {error}");
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

impl AuthorityHttpServerHandle {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn join(mut self) -> io::Result<()> {
        self.shutdown();
        match self.join.take() {
            Some(join) => join.join().map_err(|_| {
                io::Error::new(io::ErrorKind::Other, "authority server thread panicked")
            })?,
            None => Ok(()),
        }
    }
}

fn handle_connection(
    mut stream: TcpStream,
    service: &Arc<Mutex<AuthorityService>>,
    request_max_bytes: usize,
) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    if looks_like_control_upgrade(&stream)? {
        return handle_control_connection(stream, service);
    }

    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let request = read_http_request(&mut stream, request_max_bytes)?;
    let response = route_request(service, request);
    stream.write_all(&response)?;
    Ok(())
}

fn route_request(service: &Arc<Mutex<AuthorityService>>, request: HttpRequest) -> Vec<u8> {
    if request.method == "GET" && request.path == "/healthz" {
        return match service.try_lock() {
            Ok(service) => match service.healthz() {
                Ok(response) => ok_json_response(&response),
                Err(error) => error_json_response(&service, "system-healthz", "healthz", error),
            },
            Err(TryLockError::WouldBlock) => health_probe_busy_response(),
            Err(TryLockError::Poisoned(_)) => raw_http_response(
                500,
                br#"{"status":"error","code":"service_mutex_poisoned"}"#.to_vec(),
            ),
        };
    }

    if request.method == "GET" && request.path == "/readyz" {
        return match service.try_lock() {
            Ok(mut service) => match service.readyz() {
                Ok(response) => ok_json_response(&response),
                Err(error) => error_json_response(&service, "system-readyz", "readyz", error),
            },
            Err(TryLockError::WouldBlock) => readiness_probe_busy_response(),
            Err(TryLockError::Poisoned(_)) => raw_http_response(
                500,
                br#"{"status":"error","code":"service_mutex_poisoned"}"#.to_vec(),
            ),
        };
    }

    let mut service = service.lock().expect("authority service mutex poisoned");

    let route_result = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/metrics") => {
            let snapshot = service.publisher_authority().metrics_snapshot();
            let body = authority_metrics_text(&snapshot, "authority", &stack_from_env());
            raw_http_response_with_content_type(200, PROMETHEUS_CONTENT_TYPE, body.into_bytes())
        }
        ("POST", path) if path == AuthorityRoute::BridgeRegister.path() => {
            match deserialize_and_handle::<BridgeRegisterBody, _, _>(&request.body, |payload| {
                service.handle_bridge_register(payload)
            }) {
                Ok(response) => ok_json_response(&response),
                Err((chain_id, request_id, error)) => {
                    error_json_response(&service, &chain_id, &request_id, error)
                }
            }
        }
        ("POST", path) if path == AuthorityRoute::BridgeHeartbeat.path() => {
            match deserialize_and_handle_with_trace::<BridgeHeartbeatBody, _, _>(
                &request.body,
                heartbeat_trace_operation(),
                |payload| service.handle_bridge_heartbeat(payload),
            ) {
                Ok(response) => ok_json_response(&response),
                Err((chain_id, request_id, error)) => {
                    error_json_response(&service, &chain_id, &request_id, error)
                }
            }
        }
        ("POST", path) if path == AuthorityRoute::CreatorCatalog.path() => {
            match deserialize_and_handle::<CreatorCatalogBody, _, _>(&request.body, |payload| {
                service.handle_creator_catalog(payload)
            }) {
                Ok(response) => ok_json_response(&response),
                Err((chain_id, request_id, error)) => {
                    error_json_response(&service, &chain_id, &request_id, error)
                }
            }
        }
        ("POST", path) if path == AuthorityRoute::BootstrapJoin.path() => {
            match deserialize_and_handle::<BootstrapJoinBody, _, _>(&request.body, |payload| {
                service.handle_bootstrap_join(payload)
            }) {
                Ok(response) => ok_json_response(&response),
                Err((chain_id, request_id, error)) => {
                    error_json_response(&service, &chain_id, &request_id, error)
                }
            }
        }
        ("POST", path) if path == AuthorityRoute::BridgeProgress.path() => {
            match deserialize_and_handle::<BootstrapProgressBody, _, _>(&request.body, |payload| {
                service.handle_progress_report(payload)
            }) {
                Ok(response) => ok_json_response(&response),
                Err((chain_id, request_id, error)) => {
                    error_json_response(&service, &chain_id, &request_id, error)
                }
            }
        }
        ("POST", path) if path == AuthorityRoute::ReceiverOpen.path() => {
            match deserialize_and_handle::<ReceiverOpenBody, _, _>(&request.body, |payload| {
                service.handle_receiver_open(payload)
            }) {
                Ok(response) => ok_json_response(&response),
                Err((chain_id, request_id, error)) => {
                    error_json_response(&service, &chain_id, &request_id, error)
                }
            }
        }
        ("POST", path) if path == AuthorityRoute::ReceiverFrame.path() => {
            match deserialize_and_handle::<ReceiverFrameBody, _, _>(&request.body, |payload| {
                service.handle_receiver_frame(payload)
            }) {
                Ok(response) => ok_json_response(&response),
                Err((chain_id, request_id, error)) => {
                    error_json_response(&service, &chain_id, &request_id, error)
                }
            }
        }
        ("POST", path) if path == AuthorityRoute::ReceiverClose.path() => {
            match deserialize_and_handle::<ReceiverCloseBody, _, _>(&request.body, |payload| {
                service.handle_receiver_close(payload)
            }) {
                Ok(response) => ok_json_response(&response),
                Err((chain_id, request_id, error)) => {
                    error_json_response(&service, &chain_id, &request_id, error)
                }
            }
        }
        ("POST", _) => error_json_response(
            &service,
            "system-route",
            "route-not-found",
            ServiceError::NotFound(format!("unknown route {}", request.path)),
        ),
        _ => error_json_response(
            &service,
            "system-method",
            "method-not-allowed",
            ServiceError::BadRequest(format!(
                "unsupported method/path combination {} {}",
                request.method, request.path
            )),
        ),
    };

    route_result
}

fn deserialize_and_handle<T, R, F>(
    body: &[u8],
    handler: F,
) -> Result<R, (String, String, ServiceError)>
where
    F: FnOnce(AuthorityApiRequest<T>) -> Result<R, ServiceError>,
    T: for<'de> serde::Deserialize<'de>,
{
    deserialize_and_handle_with_trace(body, Some("authority_http_request"), handler)
}

fn deserialize_and_handle_with_trace<T, R, F>(
    body: &[u8],
    trace_operation: Option<&'static str>,
    handler: F,
) -> Result<R, (String, String, ServiceError)>
where
    F: FnOnce(AuthorityApiRequest<T>) -> Result<R, ServiceError>,
    T: for<'de> serde::Deserialize<'de>,
{
    let parsed: Result<AuthorityApiRequest<T>, _> = serde_json::from_slice(body);
    match parsed {
        Ok(request) => {
            let chain_id = request.chain_id.clone();
            let request_id = request.request_id.clone();
            let _chain_span = trace_operation
                .map(|operation| metrics_otlp::chain_span(operation, &chain_id).entered());
            if trace_operation.is_some() {
                metrics_otlp::record_chain_id(&chain_id);
            }
            handler(request).map_err(|error| (chain_id, request_id, error))
        }
        Err(error) => Err((
            "system-parse".into(),
            "malformed-request".into(),
            ServiceError::BadRequest(format!("invalid request json: {error}")),
        )),
    }
}

fn heartbeat_trace_operation() -> Option<&'static str> {
    matches_env_true("GBN_BRIDGE_TRACE_HEARTBEATS").then_some("authority_heartbeat")
}

fn matches_env_true(key: &str) -> bool {
    std::env::var(key)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "on" | "yes"
            )
        })
        .unwrap_or(false)
}

fn ok_json_response<T>(response: &T) -> Vec<u8>
where
    T: Serialize,
{
    json_http_response(200, response)
}

fn error_json_response(
    service: &AuthorityService,
    chain_id: &str,
    request_id: &str,
    error: ServiceError,
) -> Vec<u8> {
    let status_code = error.http_status();
    match service.error_response(chain_id, request_id, error.clone()) {
        Ok(response) => json_http_response(status_code, &response),
        Err(_) => {
            let fallback = serde_json::to_vec(&serde_json::json!({
                "chain_id": chain_id,
                "request_id": request_id,
                "served_at_ms": 0,
                "ok": false,
                "body": serde_json::Value::Null,
                "error": {
                    "code": error.code(),
                    "message": error.message(),
                },
                "publisher_sig": [],
            }))
            .expect("fallback error response should serialize");
            raw_http_response(status_code, fallback)
        }
    }
}

fn json_http_response<T>(status_code: u16, payload: &T) -> Vec<u8>
where
    T: Serialize,
{
    let body = serde_json::to_vec(payload).expect("authority response should serialize");
    raw_http_response(status_code, body)
}

fn health_probe_busy_response() -> Vec<u8> {
    raw_http_response(
        200,
        br#"{"status":"ok","degraded":"authority_service_busy"}"#.to_vec(),
    )
}

fn readiness_probe_busy_response() -> Vec<u8> {
    raw_http_response(
        200,
        br#"{"status":"ready","degraded":"authority_service_busy"}"#.to_vec(),
    )
}

fn raw_http_response(status_code: u16, body: Vec<u8>) -> Vec<u8> {
    raw_http_response_with_content_type(status_code, "application/json", body)
}

fn raw_http_response_with_content_type(
    status_code: u16,
    content_type: &str,
    body: Vec<u8>,
) -> Vec<u8> {
    let status_text = match status_code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        410 => "Gone",
        503 => "Service Unavailable",
        500 => "Internal Server Error",
        _ => "OK",
    };

    let headers = format!(
        "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut response = headers.into_bytes();
    response.extend_from_slice(&body);
    response
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
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

#[cfg(test)]
mod tests {
    use super::*;

    use ed25519_dalek::SigningKey;

    use crate::{PublisherAuthority, PublisherServiceConfig};

    fn authority_service() -> Arc<Mutex<AuthorityService>> {
        let authority = PublisherAuthority::new(SigningKey::from_bytes(&[77_u8; 32]));
        Arc::new(Mutex::new(AuthorityService::new(
            authority,
            &PublisherServiceConfig::default(),
        )))
    }

    fn request(method: &str, path: &str) -> HttpRequest {
        HttpRequest {
            method: method.to_string(),
            path: path.to_string(),
            body: Vec::new(),
        }
    }

    fn response_status(response: &[u8]) -> u16 {
        std::str::from_utf8(response)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap()
    }

    #[test]
    fn healthz_returns_ok_while_authority_service_is_busy() {
        let service = authority_service();
        let _guard = service.lock().unwrap();

        let response = route_request(&service, request("GET", "/healthz"));

        assert_eq!(response_status(&response), 200);
        assert!(String::from_utf8_lossy(&response).contains("authority_service_busy"));
    }

    #[test]
    fn readyz_returns_degraded_ready_while_authority_service_is_busy() {
        let service = authority_service();
        let _guard = service.lock().unwrap();

        let response = route_request(&service, request("GET", "/readyz"));

        assert_eq!(response_status(&response), 200);
        assert!(String::from_utf8_lossy(&response).contains("authority_service_busy"));
    }
}
