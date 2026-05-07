use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::Value;

use crate::metrics::ReceiverMetrics;
use crate::metrics_otlp;
use crate::metrics_prometheus::{receiver_metrics_text, stack_from_env, PROMETHEUS_CONTENT_TYPE};

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8081";
const DEFAULT_AUTHORITY_URL: &str = "http://127.0.0.1:8080";
const DEFAULT_REQUEST_MAX_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverProxyConfig {
    pub bind_addr: String,
    pub authority_url: String,
    pub request_max_bytes: usize,
}

impl Default for ReceiverProxyConfig {
    fn default() -> Self {
        Self {
            bind_addr: DEFAULT_BIND_ADDR.to_string(),
            authority_url: DEFAULT_AUTHORITY_URL.to_string(),
            request_max_bytes: DEFAULT_REQUEST_MAX_BYTES,
        }
    }
}

impl ReceiverProxyConfig {
    pub fn from_env() -> Result<Self, String> {
        Ok(Self {
            bind_addr: std::env::var("GBN_BRIDGE_RECEIVER_BIND_ADDR")
                .unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string()),
            authority_url: std::env::var("GBN_BRIDGE_AUTHORITY_URL")
                .or_else(|_| std::env::var("GBN_BRIDGE_PUBLISHER_URL"))
                .unwrap_or_else(|_| DEFAULT_AUTHORITY_URL.to_string()),
            request_max_bytes: std::env::var("GBN_BRIDGE_REQUEST_MAX_BYTES")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(DEFAULT_REQUEST_MAX_BYTES),
        })
    }

    pub fn socket_addr(&self) -> io::Result<SocketAddr> {
        self.bind_addr.parse::<SocketAddr>().map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "invalid receiver bind address")
        })
    }
}

#[derive(Debug)]
pub struct ReceiverProxyServer {
    listener: TcpListener,
    config: ReceiverProxyConfig,
    metrics: Arc<Mutex<ReceiverMetrics>>,
}

pub struct ReceiverProxyHandle {
    local_addr: SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<io::Result<()>>>,
}

impl ReceiverProxyServer {
    pub fn bind(config: ReceiverProxyConfig) -> io::Result<Self> {
        Self::bind_with_metrics(config, Arc::new(Mutex::new(ReceiverMetrics::default())))
    }

    pub fn bind_with_metrics(
        config: ReceiverProxyConfig,
        metrics: Arc<Mutex<ReceiverMetrics>>,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(config.socket_addr()?)?;
        Ok(Self {
            listener,
            config,
            metrics,
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn spawn(self) -> io::Result<ReceiverProxyHandle> {
        let stop = Arc::new(AtomicBool::new(false));
        let local_addr = self.local_addr()?;
        let stop_for_thread = Arc::clone(&stop);
        let join = thread::spawn(move || self.run_loop(stop_for_thread));
        Ok(ReceiverProxyHandle {
            local_addr,
            stop,
            join: Some(join),
        })
    }

    pub fn serve_forever(self) -> io::Result<()> {
        self.listener.set_nonblocking(false)?;
        loop {
            let (stream, _) = self.listener.accept()?;
            let config = self.config.clone();
            let metrics = Arc::clone(&self.metrics);
            thread::spawn(move || {
                if let Err(error) = handle_connection(stream, &config, &metrics) {
                    eprintln!("publisher-receiver proxy error: {error}");
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
                    let config = self.config.clone();
                    let metrics = Arc::clone(&self.metrics);
                    thread::spawn(move || {
                        if let Err(error) = handle_connection(stream, &config, &metrics) {
                            eprintln!("publisher-receiver proxy error: {error}");
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

impl ReceiverProxyHandle {
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
                .map_err(|_| io::Error::other("publisher receiver thread panicked"))?,
            None => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedEndpoint {
    host: String,
    port: u16,
}

fn handle_connection(
    mut stream: TcpStream,
    config: &ReceiverProxyConfig,
    metrics: &Arc<Mutex<ReceiverMetrics>>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let request = read_http_request(&mut stream, config.request_max_bytes)?;
    if request.method == "GET" && request.path == "/metrics" {
        let snapshot = metrics
            .lock()
            .expect("receiver metrics mutex poisoned while rendering prometheus metrics")
            .snapshot();
        let body = receiver_metrics_text(&snapshot, "receiver", &stack_from_env());
        stream.write_all(&text_response(200, "OK", PROMETHEUS_CONTENT_TYPE, body))?;
        return Ok(());
    }

    if !allowed_path(&request.path) {
        stream.write_all(&json_response(404, "not_found", "receiver route not found"))?;
        return Ok(());
    }

    let chain_id = extract_chain_id(&request.body);
    let _chain_span = chain_id
        .as_deref()
        .map(|chain_id| metrics_otlp::chain_span("publisher_receiver_proxy", chain_id).entered());
    if let Some(chain_id) = &chain_id {
        metrics_otlp::record_chain_id(chain_id);
        eprintln!(
            "publisher-receiver proxy method={} path={} chain_id={}",
            request.method, request.path, chain_id
        );
    } else {
        eprintln!(
            "publisher-receiver proxy method={} path={}",
            request.method, request.path
        );
    }

    match forward_request(config, &request) {
        Ok(response) => {
            record_receiver_metrics(metrics, &request, response_status(&response));
            stream.write_all(&response)?
        }
        Err(error) => {
            eprintln!(
                "publisher-receiver upstream failure path={} detail={error}",
                request.path
            );
            record_receiver_metrics(metrics, &request, Some(502));
            stream.write_all(&json_response(
                502,
                "upstream_unavailable",
                "publisher authority is unavailable",
            ))?;
        }
    }

    Ok(())
}

fn record_receiver_metrics(
    metrics: &Arc<Mutex<ReceiverMetrics>>,
    request: &HttpRequest,
    status: Option<u16>,
) {
    let ok = status
        .map(|status| (200..300).contains(&status))
        .unwrap_or(false);
    let mut metrics = metrics.lock().expect("receiver metrics mutex poisoned");
    match request.path.as_str() {
        "/v1/receiver/open" if ok => metrics.record_session_opened(),
        "/v1/receiver/close" if ok => metrics.record_session_closed(),
        "/v1/receiver/frame" if ok => metrics.record_frame_accepted(request.body.len()),
        "/v1/receiver/frame" => metrics.record_frame_rejected(),
        _ => {}
    }
}

fn allowed_path(path: &str) -> bool {
    matches!(
        path,
        "/healthz" | "/readyz" | "/v1/receiver/open" | "/v1/receiver/frame" | "/v1/receiver/close"
    )
}

fn forward_request(config: &ReceiverProxyConfig, request: &HttpRequest) -> io::Result<Vec<u8>> {
    let endpoint = parse_base_url(&config.authority_url)?;
    let address = resolve_endpoint(&endpoint)?;
    let mut upstream = TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
    upstream.set_read_timeout(Some(Duration::from_secs(5)))?;
    upstream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let request_bytes = match request.method.as_str() {
        "GET" => format!(
            "GET {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
            request.path, endpoint.host, endpoint.port
        )
        .into_bytes(),
        "POST" => {
            let headers = format!(
                "POST {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                request.path,
                endpoint.host,
                endpoint.port,
                request.body.len()
            );
            let mut bytes = headers.into_bytes();
            bytes.extend_from_slice(&request.body);
            bytes
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported method",
            ))
        }
    };

    upstream.write_all(&request_bytes)?;
    upstream.shutdown(std::net::Shutdown::Write)?;

    let mut response = Vec::new();
    upstream.read_to_end(&mut response)?;
    Ok(response)
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

fn parse_base_url(base_url: &str) -> io::Result<ParsedEndpoint> {
    let trimmed = base_url.trim();
    let without_scheme = trimmed
        .strip_prefix("http://")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "only http:// is supported"))?;
    let authority = without_scheme
        .split('/')
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing authority host"))?;
    let mut parts = authority.rsplitn(2, ':');
    let port = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing authority port"))?
        .parse::<u16>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid authority port"))?;
    let host = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing authority host"))?
        .trim();

    Ok(ParsedEndpoint {
        host: host.to_string(),
        port,
    })
}

fn resolve_endpoint(endpoint: &ParsedEndpoint) -> io::Result<SocketAddr> {
    let mut addresses = (endpoint.host.as_str(), endpoint.port).to_socket_addrs()?;
    addresses.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("no addresses resolved for {}", endpoint.host),
        )
    })
}

fn extract_chain_id(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    value
        .get("chain_id")?
        .as_str()
        .map(|value| value.to_string())
}

fn response_status(response: &[u8]) -> Option<u16> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")?;
    let header = std::str::from_utf8(&response[..header_end]).ok()?;
    header
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse::<u16>()
        .ok()
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn json_response(status_code: u16, code: &str, message: &str) -> Vec<u8> {
    let status_text = match status_code {
        404 => "Not Found",
        502 => "Bad Gateway",
        _ => "OK",
    };
    let body = serde_json::to_vec(&serde_json::json!({
        "ok": false,
        "error": {
            "code": code,
            "message": message,
        }
    }))
    .expect("receiver proxy error response should serialize");
    let headers = format!(
        "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut response = headers.into_bytes();
    response.extend_from_slice(&body);
    response
}

fn text_response(
    status_code: u16,
    status_text: &str,
    content_type: &str,
    body: impl AsRef<str>,
) -> Vec<u8> {
    let body = body.as_ref().as_bytes();
    let headers = format!(
        "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut response = headers.into_bytes();
    response.extend_from_slice(body);
    response
}
