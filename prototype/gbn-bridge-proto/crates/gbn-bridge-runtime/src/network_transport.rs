use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde::{de::DeserializeOwned, Serialize};

use crate::{RuntimeError, RuntimeResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportMetadata {
    pub chain_id: String,
    pub request_id: String,
    pub actor_id: String,
    pub sent_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpTransportConfig {
    pub base_url: String,
    pub connect_timeout_ms: u64,
    pub read_timeout_ms: u64,
    pub write_timeout_ms: u64,
    pub max_response_bytes: usize,
}

impl HttpTransportConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            connect_timeout_ms: 5_000,
            read_timeout_ms: 5_000,
            write_timeout_ms: 5_000,
            max_response_bytes: 2 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedEndpoint {
    host: String,
    port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpJsonTransport {
    endpoint: ParsedEndpoint,
    connect_timeout_ms: u64,
    read_timeout_ms: u64,
    write_timeout_ms: u64,
    max_response_bytes: usize,
}

impl HttpJsonTransport {
    pub fn new(config: HttpTransportConfig) -> RuntimeResult<Self> {
        Ok(Self {
            endpoint: parse_base_url(&config.base_url)?,
            connect_timeout_ms: config.connect_timeout_ms,
            read_timeout_ms: config.read_timeout_ms,
            write_timeout_ms: config.write_timeout_ms,
            max_response_bytes: config.max_response_bytes,
        })
    }

    pub fn post_json<TRequest, TResponse>(
        &self,
        path: &str,
        payload: &TRequest,
    ) -> RuntimeResult<(u16, TResponse)>
    where
        TRequest: Serialize,
        TResponse: DeserializeOwned,
    {
        let body =
            serde_json::to_vec(payload).map_err(|error| RuntimeError::AuthorityProtocol {
                detail: format!("failed to serialize request body: {error}"),
            })?;

        let address = resolve_endpoint(&self.endpoint)?;
        let mut stream =
            TcpStream::connect_timeout(&address, Duration::from_millis(self.connect_timeout_ms))
                .map_err(|error| RuntimeError::AuthorityTransport {
                    operation: "connect",
                    detail: error.to_string(),
                })?;
        stream
            .set_read_timeout(Some(Duration::from_millis(self.read_timeout_ms)))
            .map_err(|error| RuntimeError::AuthorityTransport {
                operation: "set-read-timeout",
                detail: error.to_string(),
            })?;
        stream
            .set_write_timeout(Some(Duration::from_millis(self.write_timeout_ms)))
            .map_err(|error| RuntimeError::AuthorityTransport {
                operation: "set-write-timeout",
                detail: error.to_string(),
            })?;

        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.endpoint.host,
            self.endpoint.port,
            body.len()
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|error| RuntimeError::AuthorityTransport {
                operation: "write-headers",
                detail: error.to_string(),
            })?;
        stream
            .write_all(&body)
            .map_err(|error| RuntimeError::AuthorityTransport {
                operation: "write-body",
                detail: error.to_string(),
            })?;
        stream
            .shutdown(std::net::Shutdown::Write)
            .map_err(|error| RuntimeError::AuthorityTransport {
                operation: "shutdown-write",
                detail: error.to_string(),
            })?;

        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(|error| RuntimeError::AuthorityTransport {
                operation: "read-response",
                detail: error.to_string(),
            })?;
        if response.len() > self.max_response_bytes {
            return Err(RuntimeError::AuthorityProtocol {
                detail: format!(
                    "response exceeded configured max bytes ({} > {})",
                    response.len(),
                    self.max_response_bytes
                ),
            });
        }

        parse_http_response(&response)
    }
}

pub fn default_chain_id(prefix: &str, actor_id: &str, request_id: &str) -> String {
    crate::trace::default_chain_id(prefix, actor_id, request_id)
}

pub fn default_request_id(prefix: &str, actor_id: &str, sent_at_ms: u64) -> String {
    format!("{prefix}-{actor_id}-{sent_at_ms}")
}

fn parse_base_url(base_url: &str) -> RuntimeResult<ParsedEndpoint> {
    let trimmed = base_url.trim();
    let without_scheme =
        trimmed
            .strip_prefix("http://")
            .ok_or_else(|| RuntimeError::AuthorityProtocol {
                detail: format!(
                    "only plain http:// authority endpoints are supported, got `{trimmed}`"
                ),
            })?;
    let authority =
        without_scheme
            .split('/')
            .next()
            .ok_or_else(|| RuntimeError::AuthorityProtocol {
                detail: format!("invalid authority endpoint `{trimmed}`"),
            })?;
    let mut parts = authority.rsplitn(2, ':');
    let port = parts
        .next()
        .ok_or_else(|| RuntimeError::AuthorityProtocol {
            detail: format!("authority endpoint `{trimmed}` is missing a port"),
        })?
        .parse::<u16>()
        .map_err(|error| RuntimeError::AuthorityProtocol {
            detail: format!("invalid authority endpoint port in `{trimmed}`: {error}"),
        })?;
    let host = parts
        .next()
        .ok_or_else(|| RuntimeError::AuthorityProtocol {
            detail: format!("authority endpoint `{trimmed}` is missing a host"),
        })?
        .trim()
        .to_string();
    if host.is_empty() {
        return Err(RuntimeError::AuthorityProtocol {
            detail: format!("authority endpoint `{trimmed}` has an empty host"),
        });
    }

    Ok(ParsedEndpoint { host, port })
}

fn resolve_endpoint(endpoint: &ParsedEndpoint) -> RuntimeResult<std::net::SocketAddr> {
    let mut addresses = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .map_err(|error| RuntimeError::AuthorityTransport {
            operation: "resolve",
            detail: error.to_string(),
        })?;
    addresses
        .next()
        .ok_or_else(|| RuntimeError::AuthorityTransport {
            operation: "resolve",
            detail: format!("no socket addresses resolved for {}", endpoint.host),
        })
}

fn parse_http_response<TResponse>(response: &[u8]) -> RuntimeResult<(u16, TResponse)>
where
    TResponse: DeserializeOwned,
{
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| RuntimeError::AuthorityProtocol {
            detail: "response headers were not terminated".into(),
        })?;
    let header = std::str::from_utf8(&response[..header_end]).map_err(|error| {
        RuntimeError::AuthorityProtocol {
            detail: format!("response headers were not utf-8: {error}"),
        }
    })?;
    let status = header
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| RuntimeError::AuthorityProtocol {
            detail: "response status line was malformed".into(),
        })?
        .parse::<u16>()
        .map_err(|error| RuntimeError::AuthorityProtocol {
            detail: format!("response status code was invalid: {error}"),
        })?;
    let body = &response[header_end + 4..];
    let parsed = serde_json::from_slice(body).map_err(|error| RuntimeError::AuthorityProtocol {
        detail: format!("failed to parse authority response json: {error}"),
    })?;
    Ok((status, parsed))
}
