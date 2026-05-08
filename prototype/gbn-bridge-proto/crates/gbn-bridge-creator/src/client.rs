use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    sign_payload, verify_payload, BootstrapJoinReply, BridgeAckStatus, BridgeCatalogRequest,
    BridgeCatalogResponse, BridgeClose, BridgeCloseReason, BridgeData, BridgeOpen,
    CreatorJoinRequest, PendingCreator, PublicKeyBytes, SignatureBytes, DEFAULT_UDP_PUNCH_PORT,
};
use serde::{Deserialize, Serialize};

use crate::error::CreatorError;
use crate::session::CreatorSession;
use crate::upload::{CreatorBridgeRequest, CreatorBridgeResponse};

const BOOTSTRAP_JOIN_PATH: &str = "/v1/bootstrap/join";
const CREATOR_CATALOG_PATH: &str = "/v1/creator/catalog";
const HTTP_TIMESTAMP_GUARD_MS: u64 = 25;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_UDP_DATAGRAM_BYTES: usize = 60 * 1024;

#[derive(Debug, Clone)]
pub struct CreatorClient {
    actor_id: String,
    creator_ip_addr: String,
    udp_punch_port: u16,
    signing_key: SigningKey,
    actor_pub: PublicKeyBytes,
    publisher_pub: PublicKeyBytes,
    timeout: Duration,
}

impl CreatorClient {
    pub fn new(
        actor_id: impl Into<String>,
        signing_key: SigningKey,
        publisher_pub: PublicKeyBytes,
    ) -> Self {
        let actor_pub = PublicKeyBytes::from_verifying_key(&signing_key.verifying_key());
        Self {
            actor_id: actor_id.into(),
            creator_ip_addr: "127.0.0.1".into(),
            udp_punch_port: DEFAULT_UDP_PUNCH_PORT,
            signing_key,
            actor_pub,
            publisher_pub,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_creator_endpoint(mut self, ip_addr: impl Into<String>, udp_port: u16) -> Self {
        self.creator_ip_addr = ip_addr.into();
        self.udp_punch_port = udp_port;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn bootstrap_join(&self, authority_url: &str) -> Result<CreatorSession, CreatorError> {
        let now_ms = now_ms();
        let request_id = format!("send-dummy-{}-{now_ms}", self.actor_id);
        let chain_id = default_chain_id("send-dummy", &self.actor_id, &request_id);
        self.bootstrap_join_with_ids(authority_url, chain_id, request_id, now_ms)
    }

    fn bootstrap_join_with_ids(
        &self,
        authority_url: &str,
        chain_id: String,
        request_id: String,
        now_ms: u64,
    ) -> Result<CreatorSession, CreatorError> {
        let join = CreatorJoinRequest {
            chain_id: chain_id.clone(),
            request_id: request_id.clone(),
            host_creator_id: self.actor_id.clone(),
            relay_bridge_id: self.actor_id.clone(),
            creator: PendingCreator {
                node_id: self.actor_id.clone(),
                ip_addr: self.creator_ip_addr.clone(),
                pub_key: self.actor_pub.clone(),
                udp_punch_port: self.udp_punch_port,
            },
        };
        let response: AuthorityApiResponse<BootstrapJoinReply> = self.post_authority_json(
            authority_url,
            BOOTSTRAP_JOIN_PATH,
            AuthorityApiRequestUnsigned {
                chain_id: chain_id.clone(),
                request_id: request_id.clone(),
                sent_at_ms: now_ms.saturating_sub(HTTP_TIMESTAMP_GUARD_MS),
                actor_id: self.actor_id.clone(),
                body: BootstrapJoinBody {
                    request: join,
                    now_ms,
                },
            },
        )?;
        self.verify_authority_response(&response, &chain_id, &request_id)?;
        if !response.ok {
            return Err(CreatorError::BootstrapFailed(
                response
                    .error
                    .map(|error| format!("{}: {}", error.code, error.message))
                    .unwrap_or_else(|| "authority rejected bootstrap without an error body".into()),
            ));
        }
        let reply = response.body.ok_or_else(|| {
            CreatorError::BootstrapFailed("authority bootstrap response had no body".into())
        })?;
        reply.verify_authority(&self.publisher_pub, now_ms)?;
        let seed_bridge = reply.response.seed_bridge.clone();
        if seed_bridge.node_id.trim().is_empty() {
            return Err(CreatorError::NoBridgeAssigned);
        }

        Ok(CreatorSession {
            session_id: format!("upload-{}", reply.response.bootstrap_session_id),
            bridge_id: seed_bridge.node_id,
            bridge_address: format!("{}:{}", seed_bridge.ip_addr, seed_bridge.udp_punch_port),
            bootstrap_chain_id: reply.chain_id,
            started_at: Instant::now(),
        })
    }

    pub fn discovery_probe(
        &self,
        authority_url: &str,
    ) -> Result<DiscoveryProbeResult, CreatorError> {
        let started = Instant::now();
        let now_ms = now_ms();
        let request_id = format!("discovery-probe-{}-{now_ms}", self.actor_id);
        let chain_id = default_chain_id("discovery-probe", &self.actor_id, &request_id);
        let catalog_request_id = format!("{request_id}-catalog");
        let bootstrap_request_id = format!("{request_id}-bootstrap");
        let catalog_request = BridgeCatalogRequest {
            creator_id: self.actor_id.clone(),
            known_catalog_id: None,
            direct_only: false,
            refresh_hint: None,
        };
        let catalog_response: AuthorityApiResponse<BridgeCatalogResponse> = self
            .post_authority_json(
                authority_url,
                CREATOR_CATALOG_PATH,
                AuthorityApiRequestUnsigned {
                    chain_id: chain_id.clone(),
                    request_id: catalog_request_id.clone(),
                    sent_at_ms: now_ms.saturating_sub(HTTP_TIMESTAMP_GUARD_MS),
                    actor_id: self.actor_id.clone(),
                    body: CatalogRequestBody {
                        request: catalog_request,
                        now_ms,
                    },
                },
            )?;
        self.verify_authority_response(&catalog_response, &chain_id, &catalog_request_id)?;
        if !catalog_response.ok {
            return Err(CreatorError::BootstrapFailed(
                catalog_response
                    .error
                    .map(|error| format!("{}: {}", error.code, error.message))
                    .unwrap_or_else(|| "authority rejected catalog without an error body".into()),
            ));
        }
        let catalog = catalog_response.body.ok_or_else(|| {
            CreatorError::BootstrapFailed("authority catalog response had no body".into())
        })?;
        catalog.verify_authority(&self.publisher_pub, now_ms)?;

        let known_bridge_ids = catalog
            .bridges
            .iter()
            .map(|bridge| bridge.bridge_id.clone())
            .collect::<Vec<_>>();
        let session = self.bootstrap_join_with_ids(
            authority_url,
            chain_id.clone(),
            bootstrap_request_id,
            now_ms,
        )?;
        Ok(DiscoveryProbeResult {
            chain_id,
            actor_id: self.actor_id.clone(),
            assigned_bridge_id: session.bridge_id,
            bridge_address: session.bridge_address,
            known_bridge_count: known_bridge_ids.len(),
            known_bridge_ids,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    pub fn upload_frame(
        &self,
        session: &CreatorSession,
        frame_bytes: Vec<u8>,
    ) -> Result<String, CreatorError> {
        let opened_at_ms = now_ms();
        let open = BridgeOpen {
            chain_id: session.bootstrap_chain_id.clone(),
            session_id: session.session_id.clone(),
            creator_id: self.actor_id.clone(),
            bridge_id: session.bridge_id.clone(),
            creator_session_pub: self.actor_pub.clone(),
            opened_at_ms,
            expected_chunks: Some(1),
        };
        match self.bridge_round_trip(&session.bridge_address, CreatorBridgeRequest::Open(open))? {
            CreatorBridgeResponse::Opened { .. } => {}
            CreatorBridgeResponse::Error { message } => {
                return Err(CreatorError::FrameUploadFailed(message));
            }
            other => {
                return Err(CreatorError::FrameUploadFailed(format!(
                    "unexpected bridge open response: {other:?}"
                )));
            }
        }

        let frame = BridgeData {
            chain_id: session.bootstrap_chain_id.clone(),
            session_id: session.session_id.clone(),
            frame_id: format!("{}-frame-000000", session.session_id),
            sequence: 0,
            sent_at_ms: opened_at_ms.saturating_add(1),
            ciphertext: frame_bytes,
            final_frame: true,
        };
        let ack = match self
            .bridge_round_trip(&session.bridge_address, CreatorBridgeRequest::Frame(frame))?
        {
            CreatorBridgeResponse::Ack(ack) => ack,
            CreatorBridgeResponse::Error { message } => {
                return Err(CreatorError::FrameUploadFailed(message));
            }
            other => {
                return Err(CreatorError::FrameUploadFailed(format!(
                    "unexpected bridge frame response: {other:?}"
                )));
            }
        };
        if matches!(ack.status, BridgeAckStatus::Rejected) {
            return Err(CreatorError::FrameUploadFailed(format!(
                "bridge rejected sequence {} for session {}",
                ack.acked_sequence, ack.session_id
            )));
        }

        let close = BridgeClose {
            chain_id: session.bootstrap_chain_id.clone(),
            session_id: session.session_id.clone(),
            closed_at_ms: opened_at_ms.saturating_add(2),
            reason: BridgeCloseReason::Completed,
        };
        match self.bridge_round_trip(&session.bridge_address, CreatorBridgeRequest::Close(close))? {
            CreatorBridgeResponse::Closed { .. } => {}
            CreatorBridgeResponse::Error { message } => {
                return Err(CreatorError::FrameUploadFailed(message));
            }
            other => {
                return Err(CreatorError::FrameUploadFailed(format!(
                    "unexpected bridge close response: {other:?}"
                )));
            }
        }

        Ok(ack.chain_id)
    }

    pub fn send_dummy(
        &self,
        authority_url: &str,
        size: usize,
    ) -> Result<SendDummyResult, CreatorError> {
        let started = Instant::now();
        let session = self.bootstrap_join(authority_url)?;
        let frame = synthesize_frame(size);
        let chain_id = self.upload_frame(&session, frame)?;
        Ok(SendDummyResult {
            chain_id,
            assigned_bridge_id: session.bridge_id,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    fn post_authority_json<TBody, TResponse>(
        &self,
        base_url: &str,
        path: &str,
        unsigned: AuthorityApiRequestUnsigned<TBody>,
    ) -> Result<TResponse, CreatorError>
    where
        TBody: Serialize + Clone,
        TResponse: for<'de> Deserialize<'de>,
    {
        let request = AuthorityApiRequest::sign(unsigned, &self.signing_key)?;
        let body = serde_json::to_vec(&request).map_err(|error| {
            CreatorError::Protocol(format!("failed to serialize authority request: {error}"))
        })?;
        let endpoint = parse_base_url(base_url)?;
        let address = resolve_endpoint(&endpoint)?;
        let mut stream = TcpStream::connect_timeout(&address, self.timeout).map_err(|error| {
            CreatorError::Transport {
                operation: "connect-authority",
                detail: error.to_string(),
            }
        })?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|error| CreatorError::Transport {
                operation: "set-authority-read-timeout",
                detail: error.to_string(),
            })?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|error| CreatorError::Transport {
                operation: "set-authority-write-timeout",
                detail: error.to_string(),
            })?;
        let request_head = format!(
            "POST {path} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            endpoint.host,
            endpoint.port,
            body.len()
        );
        stream
            .write_all(request_head.as_bytes())
            .map_err(|error| CreatorError::Transport {
                operation: "write-authority-headers",
                detail: error.to_string(),
            })?;
        stream
            .write_all(&body)
            .map_err(|error| CreatorError::Transport {
                operation: "write-authority-body",
                detail: error.to_string(),
            })?;
        stream
            .shutdown(std::net::Shutdown::Write)
            .map_err(|error| CreatorError::Transport {
                operation: "shutdown-authority-write",
                detail: error.to_string(),
            })?;

        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(|error| CreatorError::Transport {
                operation: "read-authority-response",
                detail: error.to_string(),
            })?;
        parse_http_response(&response)
    }

    fn verify_authority_response<T>(
        &self,
        response: &AuthorityApiResponse<T>,
        chain_id: &str,
        request_id: &str,
    ) -> Result<(), CreatorError>
    where
        T: Serialize + Clone,
    {
        response.verify_authority(&self.publisher_pub)?;
        if response.chain_id != chain_id {
            return Err(CreatorError::BootstrapFailed(format!(
                "authority response chain_id mismatch: expected `{chain_id}`, got `{}`",
                response.chain_id
            )));
        }
        if response.request_id != request_id {
            return Err(CreatorError::BootstrapFailed(format!(
                "authority response request_id mismatch: expected `{request_id}`, got `{}`",
                response.request_id
            )));
        }
        Ok(())
    }

    fn bridge_round_trip(
        &self,
        bridge_address: &str,
        request: CreatorBridgeRequest,
    ) -> Result<CreatorBridgeResponse, CreatorError> {
        let payload = serde_json::to_vec(&request).map_err(|error| {
            CreatorError::Protocol(format!(
                "failed to serialize bridge upload request: {error}"
            ))
        })?;
        if payload.len() > MAX_UDP_DATAGRAM_BYTES {
            return Err(CreatorError::FrameUploadFailed(format!(
                "bridge upload datagram is too large ({} > {})",
                payload.len(),
                MAX_UDP_DATAGRAM_BYTES
            )));
        }

        let mut addresses =
            bridge_address
                .to_socket_addrs()
                .map_err(|error| CreatorError::Transport {
                    operation: "resolve-bridge",
                    detail: error.to_string(),
                })?;
        let bridge_address = addresses.next().ok_or_else(|| CreatorError::Transport {
            operation: "resolve-bridge",
            detail: format!("no socket address resolved for `{bridge_address}`"),
        })?;
        let socket = UdpSocket::bind("0.0.0.0:0").map_err(|error| CreatorError::Transport {
            operation: "bind-creator-udp",
            detail: error.to_string(),
        })?;
        socket
            .set_read_timeout(Some(self.timeout))
            .map_err(|error| CreatorError::Transport {
                operation: "set-creator-udp-read-timeout",
                detail: error.to_string(),
            })?;
        socket
            .set_write_timeout(Some(self.timeout))
            .map_err(|error| CreatorError::Transport {
                operation: "set-creator-udp-write-timeout",
                detail: error.to_string(),
            })?;
        socket
            .send_to(&payload, bridge_address)
            .map_err(|error| CreatorError::Transport {
                operation: "send-bridge-upload",
                detail: error.to_string(),
            })?;

        let mut buffer = vec![0_u8; MAX_UDP_DATAGRAM_BYTES];
        let (read, _) = socket
            .recv_from(&mut buffer)
            .map_err(|error| CreatorError::Transport {
                operation: "recv-bridge-upload-ack",
                detail: error.to_string(),
            })?;
        serde_json::from_slice(&buffer[..read]).map_err(|error| {
            CreatorError::Protocol(format!("failed to parse bridge upload response: {error}"))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendDummyResult {
    pub chain_id: String,
    pub assigned_bridge_id: String,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryProbeResult {
    pub chain_id: String,
    pub actor_id: String,
    pub assigned_bridge_id: String,
    pub bridge_address: String,
    pub known_bridge_count: usize,
    pub known_bridge_ids: Vec<String>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BootstrapJoinBody {
    request: CreatorJoinRequest,
    now_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CatalogRequestBody {
    request: BridgeCatalogRequest,
    now_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AuthorityApiAuth {
    actor_pub: PublicKeyBytes,
    signature: SignatureBytes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AuthorityApiRequestUnsigned<T> {
    chain_id: String,
    request_id: String,
    sent_at_ms: u64,
    actor_id: String,
    body: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AuthorityApiRequest<T> {
    chain_id: String,
    request_id: String,
    sent_at_ms: u64,
    actor_id: String,
    body: T,
    auth: AuthorityApiAuth,
}

impl<T> AuthorityApiRequest<T>
where
    T: Serialize + Clone,
{
    fn sign(
        unsigned: AuthorityApiRequestUnsigned<T>,
        signing_key: &SigningKey,
    ) -> Result<Self, CreatorError> {
        let signature = sign_payload(&unsigned, signing_key)?;
        Ok(Self {
            chain_id: unsigned.chain_id,
            request_id: unsigned.request_id,
            sent_at_ms: unsigned.sent_at_ms,
            actor_id: unsigned.actor_id,
            body: unsigned.body,
            auth: AuthorityApiAuth {
                actor_pub: PublicKeyBytes::from_verifying_key(&signing_key.verifying_key()),
                signature,
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AuthorityApiErrorBody {
    code: String,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AuthorityApiResponseUnsigned<T> {
    chain_id: String,
    request_id: String,
    served_at_ms: u64,
    ok: bool,
    body: Option<T>,
    error: Option<AuthorityApiErrorBody>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AuthorityApiResponse<T> {
    chain_id: String,
    request_id: String,
    served_at_ms: u64,
    ok: bool,
    body: Option<T>,
    error: Option<AuthorityApiErrorBody>,
    publisher_sig: SignatureBytes,
}

impl<T> AuthorityApiResponse<T>
where
    T: Serialize + Clone,
{
    fn unsigned_payload(&self) -> AuthorityApiResponseUnsigned<T> {
        AuthorityApiResponseUnsigned {
            chain_id: self.chain_id.clone(),
            request_id: self.request_id.clone(),
            served_at_ms: self.served_at_ms,
            ok: self.ok,
            body: self.body.clone(),
            error: self.error.clone(),
        }
    }

    fn verify_authority(&self, publisher_key: &PublicKeyBytes) -> Result<(), CreatorError> {
        verify_payload(&self.unsigned_payload(), publisher_key, &self.publisher_sig)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedEndpoint {
    host: String,
    port: u16,
}

fn synthesize_frame(size: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(size);
    for i in 0..size {
        buf.push((i % 251) as u8);
    }
    buf
}

fn default_chain_id(prefix: &str, actor_id: &str, request_id: &str) -> String {
    format!("{prefix}-{actor_id}-{request_id}")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis() as u64
}

fn parse_base_url(base_url: &str) -> Result<ParsedEndpoint, CreatorError> {
    let trimmed = base_url.trim();
    let without_scheme = trimmed.strip_prefix("http://").ok_or_else(|| {
        CreatorError::Protocol(format!(
            "only plain http:// authority endpoints are supported, got `{trimmed}`"
        ))
    })?;
    let authority = without_scheme
        .split('/')
        .next()
        .ok_or_else(|| CreatorError::Protocol(format!("invalid authority endpoint `{trimmed}`")))?;
    let mut parts = authority.rsplitn(2, ':');
    let port = parts
        .next()
        .ok_or_else(|| {
            CreatorError::Protocol(format!("authority endpoint `{trimmed}` is missing a port"))
        })?
        .parse::<u16>()
        .map_err(|error| {
            CreatorError::Protocol(format!(
                "invalid authority endpoint port in `{trimmed}`: {error}"
            ))
        })?;
    let host = parts
        .next()
        .ok_or_else(|| {
            CreatorError::Protocol(format!("authority endpoint `{trimmed}` is missing a host"))
        })?
        .trim()
        .to_string();
    if host.is_empty() {
        return Err(CreatorError::Protocol(format!(
            "authority endpoint `{trimmed}` has an empty host"
        )));
    }
    Ok(ParsedEndpoint { host, port })
}

fn resolve_endpoint(endpoint: &ParsedEndpoint) -> Result<SocketAddr, CreatorError> {
    let mut addresses = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .map_err(|error| CreatorError::Transport {
            operation: "resolve-authority",
            detail: error.to_string(),
        })?;
    addresses.next().ok_or_else(|| CreatorError::Transport {
        operation: "resolve-authority",
        detail: format!("no socket addresses resolved for {}", endpoint.host),
    })
}

fn parse_http_response<TResponse>(response: &[u8]) -> Result<TResponse, CreatorError>
where
    TResponse: for<'de> Deserialize<'de>,
{
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| CreatorError::Protocol("response headers were not terminated".into()))?;
    let header = std::str::from_utf8(&response[..header_end]).map_err(|error| {
        CreatorError::Protocol(format!("response headers were not utf-8: {error}"))
    })?;
    let _status = header
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| CreatorError::Protocol("response status line was malformed".into()))?
        .parse::<u16>()
        .map_err(|error| {
            CreatorError::Protocol(format!("response status code was invalid: {error}"))
        })?;
    serde_json::from_slice(&response[header_end + 4..]).map_err(|error| {
        CreatorError::Protocol(format!("failed to parse authority response json: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::synthesize_frame;

    #[test]
    fn synthesized_dummy_frame_is_deterministic() {
        assert_eq!(synthesize_frame(6), vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(synthesize_frame(253)[251], 0);
        assert_eq!(synthesize_frame(253)[252], 1);
    }
}
