use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    encrypt_for_publisher, sign_payload, verify_payload, BootstrapJoinReply, BridgeAckStatus,
    BridgeCatalogRequest, BridgeCatalogResponse, BridgeClose, BridgeCloseReason, BridgeData,
    BridgeDhtEntry, BridgeIngressEndpointKind, BridgeOpen, CreatorJoinRequest,
    DhtBridgeIngressEndpoint, LocalDiscoveryTable, PendingCreator, PublicKeyBytes,
    ReachabilityClass, SelfOnboardingState, SignatureBytes, DEFAULT_UDP_PUNCH_PORT,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::CreatorError;
use crate::local_dht::LocalDhtStore;
use crate::pipeline::{
    dispatch_upload_session, load_upload_session, plan_lanes, save_upload_session,
    DispatchUploadOptions, LanePlanError, SendUploadSessionResult, UploadSessionStatus,
};
use crate::session::CreatorSession;
use crate::upload::{CreatorBridgeFrameFragment, CreatorBridgeRequest, CreatorBridgeResponse};

const BOOTSTRAP_JOIN_PATH: &str = "/v1/bootstrap/join";
const CREATOR_CATALOG_PATH: &str = "/v1/creator/catalog";
const HTTP_TIMESTAMP_GUARD_MS: u64 = 25;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_UDP_DATAGRAM_BYTES: usize = 60 * 1024;
const MAX_REASSEMBLED_UPLOAD_FRAME_BYTES: usize = 512 * 1024;
const MAX_SAFE_UPLOAD_DATAGRAM_BYTES: usize = 1_200;
const DEFAULT_FRAME_FRAGMENT_BYTES: usize = 700;
const MIN_FRAME_FRAGMENT_BYTES: usize = 256;
const MAX_FRAME_FRAGMENT_BYTES: usize = 48 * 1024;
const DEFAULT_UPLOAD_CLOSE_TIMEOUT_MS: u64 = 2_000;
const DEFAULT_SUSPECT_TTL_MS: u64 = 300_000;
const ENCRYPTION_ENVELOPE_NAME: &str = "publisher_x25519_hkdf_aes256gcm_v1";

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
                encryption_pub_key: None,
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
        chain_id: Option<String>,
    ) -> Result<DiscoveryProbeResult, CreatorError> {
        let started = Instant::now();
        let now_ms = now_ms();
        let request_id = format!("discovery-probe-{}-{now_ms}", self.actor_id);
        let chain_id = chain_id
            .unwrap_or_else(|| default_chain_id("discovery-probe", &self.actor_id, &request_id));
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
            actor_id: self.actor_id.clone(),
            route_source: "authority_bootstrap".to_string(),
            candidate_bridge_ids: vec![session.bridge_id.clone()],
            selected_bridge_ids: vec![session.bridge_id.clone()],
            assigned_bridge_id: session.bridge_id,
            encryption_envelope: "legacy_plaintext".to_string(),
            ciphertext_only_at_bridge: false,
            frames: 1,
            elapsed_ms: started.elapsed().as_millis() as u64,
            force_bridge_failure_used: false,
        })
    }

    pub fn send_dummy_from_local_dht(
        &self,
        store: &LocalDhtStore,
        size: usize,
        force_bridge_failure: bool,
        chain_id: Option<String>,
        plaintext_marker: Option<&[u8]>,
    ) -> Result<SendDummyResult, CreatorError> {
        let started = Instant::now();
        let now_ms = now_ms();
        let chain_id = chain_id.unwrap_or_else(|| {
            default_chain_id("send-dummy", &self.actor_id, &format!("local-dht-{now_ms}"))
        });
        let mut snapshot = store.snapshot();
        ensure_onboarded(&snapshot)?;
        let publisher_entry = snapshot
            .publisher_entry
            .clone()
            .ok_or(CreatorError::MissingPublisherEntry)?;
        publisher_entry.verify_trust_root(&self.publisher_pub, now_ms)?;

        let (mut candidates, drops) =
            select_route_candidates(&snapshot, &self.publisher_pub, now_ms);
        if candidates.is_empty() {
            return Err(CreatorError::NoEligibleBridge {
                filter_drops: drops,
            });
        }
        let candidate_bridge_ids = candidates
            .iter()
            .map(|candidate| candidate.entry.bridge_id.clone())
            .collect::<Vec<_>>();

        if force_bridge_failure {
            let failed_bridge_id = candidates[0].entry.bridge_id.clone();
            let suspect_until_ms = now_ms.saturating_add(DEFAULT_SUSPECT_TTL_MS);
            if let Some(entry) = snapshot
                .bridge_entries
                .iter_mut()
                .find(|entry| entry.bridge_id == failed_bridge_id)
            {
                entry.suspect_until_ms = Some(suspect_until_ms);
            }
            snapshot = store
                .replace(snapshot)
                .map_err(|error| CreatorError::LocalDht(error.to_string()))?;
            let (reranked, rerank_drops) =
                select_route_candidates(&snapshot, &self.publisher_pub, now_ms);
            candidates = reranked;
            if candidates.is_empty() {
                return Err(CreatorError::NoEligibleBridge {
                    filter_drops: rerank_drops,
                });
            }
        }

        let selected = candidates
            .first()
            .expect("candidate set checked above")
            .entry
            .clone();
        let bridge_address = bridge_upload_address(&selected)?;
        let session_id_bytes = session_id_bytes(&chain_id);
        let session_id = hex_bytes(&session_id_bytes);
        let frame = synthesize_frame_with_marker(size, plaintext_marker);
        let ephemeral_private = ephemeral_private_bytes(&self.actor_id, &chain_id, now_ms);
        let publisher_encryption_pub = publisher_entry
            .encryption_pub_key
            .as_ref()
            .unwrap_or(&publisher_entry.pub_key);
        let encrypted = encrypt_for_publisher(
            &frame,
            publisher_encryption_pub,
            publisher_entry.node_id.clone(),
            session_id_bytes,
            0,
            1,
            ephemeral_private,
        )?;
        let encrypted_frame = serde_json::to_vec(&encrypted).map_err(|error| {
            CreatorError::Protocol(format!(
                "failed to serialize encrypted dummy frame: {error}"
            ))
        })?;
        let session = CreatorSession {
            session_id: format!("upload-{session_id}"),
            bridge_id: selected.bridge_id.clone(),
            bridge_address,
            bootstrap_chain_id: chain_id.clone(),
            started_at: Instant::now(),
        };
        let ack_chain_id = self.upload_frame(&session, encrypted_frame)?;

        Ok(SendDummyResult {
            chain_id: ack_chain_id,
            actor_id: self.actor_id.clone(),
            route_source: "local_dht".to_string(),
            candidate_bridge_ids,
            selected_bridge_ids: vec![selected.bridge_id.clone()],
            assigned_bridge_id: selected.bridge_id,
            encryption_envelope: ENCRYPTION_ENVELOPE_NAME.to_string(),
            ciphertext_only_at_bridge: true,
            frames: 1,
            elapsed_ms: started.elapsed().as_millis() as u64,
            force_bridge_failure_used: force_bridge_failure,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_upload_session_from_local_dht(
        &self,
        store: &LocalDhtStore,
        base_state_dir: &Path,
        session_id: &str,
        target_lane_count: u32,
        lane_open_timeout_ms: u64,
        chunk_ack_timeout_ms: u64,
        suspect_ttl_ms: u64,
        force_lane_failure: Vec<String>,
        chain_id: Option<String>,
    ) -> Result<SendUploadSessionResult, CreatorError> {
        let now_ms = now_ms();
        let current_dht = store.snapshot();
        ensure_onboarded(&current_dht)?;
        let chain_id = chain_id.unwrap_or_else(|| {
            default_chain_id(
                "send-upload",
                &self.actor_id,
                &format!("{session_id}-{now_ms}"),
            )
        });
        let mut session = load_upload_session(base_state_dir, session_id)
            .map_err(|error| CreatorError::FrameUploadFailed(error.to_string()))?;
        if !matches!(
            session.status,
            UploadSessionStatus::Built | UploadSessionStatus::Partial
        ) {
            return Err(CreatorError::FrameUploadFailed(format!(
                "upload session `{session_id}` is not dispatchable from status {:?}",
                session.status
            )));
        }
        let publisher_entry = session
            .local_dht_snapshot
            .publisher_entry
            .clone()
            .ok_or(CreatorError::MissingPublisherEntry)?;
        publisher_entry.verify_trust_root(&self.publisher_pub, now_ms)?;
        let lane_plan = plan_lanes(
            &session.local_dht_snapshot,
            &self.publisher_pub,
            target_lane_count,
            now_ms,
        )
        .map_err(|error| match error {
            LanePlanError::NoEligibleBridges { filter_drops } => {
                CreatorError::NoEligibleBridge { filter_drops }
            }
            LanePlanError::InvalidTargetLaneCount => CreatorError::Protocol(error.to_string()),
        })?;
        session.status = UploadSessionStatus::Dispatching;
        save_upload_session(base_state_dir, &session)
            .map_err(|error| CreatorError::FrameUploadFailed(error.to_string()))?;

        let dispatch_result = dispatch_upload_session(
            &mut session,
            lane_plan,
            DispatchUploadOptions {
                chain_id,
                actor_id: self.actor_id.clone(),
                actor_pub: self.actor_pub.clone(),
                lane_open_timeout_ms,
                chunk_ack_timeout_ms,
                suspect_ttl_ms,
                force_lane_failure,
                now_ms,
            },
            |bridge_address, request| {
                let timeout = match &request {
                    CreatorBridgeRequest::Open(_) => timeout_from_ms(lane_open_timeout_ms),
                    CreatorBridgeRequest::Frame(_) | CreatorBridgeRequest::FrameFragment(_) => {
                        timeout_from_ms(chunk_ack_timeout_ms)
                    }
                    CreatorBridgeRequest::Close(_) => upload_close_timeout(),
                };
                self.bridge_round_trip_with_timeout(bridge_address, request, timeout)
            },
        );
        if dispatch_result.is_err() && session.status == UploadSessionStatus::Dispatching {
            session.status = UploadSessionStatus::Failed;
            session.plan.session_status = UploadSessionStatus::Failed;
        }
        save_upload_session(base_state_dir, &session)
            .map_err(|error| CreatorError::FrameUploadFailed(error.to_string()))?;
        let result = dispatch_result?;
        let failed_bridge_ids = session
            .plan
            .lanes
            .iter()
            .filter(|lane| lane.is_failed())
            .map(|lane| lane.bridge_id.clone())
            .collect::<Vec<_>>();
        if !failed_bridge_ids.is_empty() {
            let mut next = current_dht;
            let suspect_until_ms = now_ms.saturating_add(suspect_ttl_ms);
            for entry in &mut next.bridge_entries {
                if failed_bridge_ids.contains(&entry.bridge_id) {
                    entry.suspect_until_ms = Some(suspect_until_ms);
                }
            }
            store
                .replace(next)
                .map_err(|error| CreatorError::LocalDht(error.to_string()))?;
        }
        Ok(result)
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
        self.bridge_round_trip_with_timeout(bridge_address, request, self.timeout)
    }

    fn bridge_round_trip_with_timeout(
        &self,
        bridge_address: &str,
        request: CreatorBridgeRequest,
        timeout: Duration,
    ) -> Result<CreatorBridgeResponse, CreatorError> {
        if let CreatorBridgeRequest::Frame(frame) = &request {
            let payload = serde_json::to_vec(&request).map_err(|error| {
                CreatorError::Protocol(format!(
                    "failed to serialize bridge upload request: {error}"
                ))
            })?;
            if payload.len() > MAX_SAFE_UPLOAD_DATAGRAM_BYTES {
                return self.fragmented_bridge_frame_round_trip(
                    bridge_address,
                    frame.clone(),
                    timeout,
                );
            }
            return self.bridge_payload_round_trip(bridge_address, &payload, timeout);
        }

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

        self.bridge_payload_round_trip(bridge_address, &payload, timeout)
    }

    fn fragmented_bridge_frame_round_trip(
        &self,
        bridge_address: &str,
        frame: BridgeData,
        timeout: Duration,
    ) -> Result<CreatorBridgeResponse, CreatorError> {
        let frame_bytes = serde_json::to_vec(&frame).map_err(|error| {
            CreatorError::Protocol(format!("failed to serialize bridge upload frame: {error}"))
        })?;
        if frame_bytes.len() > MAX_REASSEMBLED_UPLOAD_FRAME_BYTES {
            return Err(CreatorError::FrameUploadFailed(format!(
                "bridge upload frame is too large to fragment ({} > {})",
                frame_bytes.len(),
                MAX_REASSEMBLED_UPLOAD_FRAME_BYTES
            )));
        }
        let frame_fragment_bytes = frame_fragment_bytes();
        let total_fragments = frame_bytes.len().div_ceil(frame_fragment_bytes);
        if total_fragments == 0 || total_fragments > u16::MAX as usize {
            return Err(CreatorError::FrameUploadFailed(format!(
                "bridge upload frame fragment count is invalid: {total_fragments}"
            )));
        }
        let total_fragments = total_fragments as u16;
        let bridge_address = resolve_bridge_address(bridge_address)?;
        let socket = self.creator_udp_socket(timeout)?;
        for (index, chunk) in frame_bytes.chunks(frame_fragment_bytes).enumerate() {
            let fragment_index = index as u16;
            let fragment =
                CreatorBridgeFrameFragment::new(&frame, fragment_index, total_fragments, chunk);
            let request = CreatorBridgeRequest::FrameFragment(fragment);
            let payload = serde_json::to_vec(&request).map_err(|error| {
                CreatorError::Protocol(format!(
                    "failed to serialize bridge upload frame fragment: {error}"
                ))
            })?;
            if payload.len() > MAX_UDP_DATAGRAM_BYTES {
                return Err(CreatorError::FrameUploadFailed(format!(
                    "bridge upload fragment datagram is too large ({} > {})",
                    payload.len(),
                    MAX_UDP_DATAGRAM_BYTES
                )));
            }
            let response = self.bridge_socket_round_trip(&socket, bridge_address, &payload)?;
            if fragment_index + 1 == total_fragments {
                return match response {
                    CreatorBridgeResponse::Ack(_) | CreatorBridgeResponse::Error { .. } => {
                        Ok(response)
                    }
                    other => Err(CreatorError::FrameUploadFailed(format!(
                        "unexpected final bridge fragment response: {other:?}"
                    ))),
                };
            }
            match response {
                CreatorBridgeResponse::FrameFragmentAccepted {
                    frame_id,
                    fragment_index: accepted_index,
                    total_fragments: accepted_total,
                    ..
                } if frame_id == frame.frame_id
                    && accepted_index == fragment_index
                    && accepted_total == total_fragments => {}
                CreatorBridgeResponse::Error { message } => {
                    return Err(CreatorError::FrameUploadFailed(message));
                }
                other => {
                    return Err(CreatorError::FrameUploadFailed(format!(
                        "unexpected bridge fragment response: {other:?}"
                    )));
                }
            }
        }

        Err(CreatorError::FrameUploadFailed(
            "bridge upload frame fragmentation produced no fragments".to_string(),
        ))
    }

    fn bridge_payload_round_trip(
        &self,
        bridge_address: &str,
        payload: &[u8],
        timeout: Duration,
    ) -> Result<CreatorBridgeResponse, CreatorError> {
        let bridge_address = resolve_bridge_address(bridge_address)?;
        let socket = self.creator_udp_socket(timeout)?;
        self.bridge_socket_round_trip(&socket, bridge_address, payload)
    }

    fn creator_udp_socket(&self, timeout: Duration) -> Result<UdpSocket, CreatorError> {
        let socket = UdpSocket::bind("0.0.0.0:0").map_err(|error| CreatorError::Transport {
            operation: "bind-creator-udp",
            detail: error.to_string(),
        })?;
        socket
            .set_read_timeout(Some(timeout))
            .map_err(|error| CreatorError::Transport {
                operation: "set-creator-udp-read-timeout",
                detail: error.to_string(),
            })?;
        socket
            .set_write_timeout(Some(timeout))
            .map_err(|error| CreatorError::Transport {
                operation: "set-creator-udp-write-timeout",
                detail: error.to_string(),
            })?;
        Ok(socket)
    }

    fn bridge_socket_round_trip(
        &self,
        socket: &UdpSocket,
        bridge_address: SocketAddr,
        payload: &[u8],
    ) -> Result<CreatorBridgeResponse, CreatorError> {
        if payload.len() > MAX_UDP_DATAGRAM_BYTES {
            return Err(CreatorError::FrameUploadFailed(format!(
                "bridge upload datagram is too large ({} > {})",
                payload.len(),
                MAX_UDP_DATAGRAM_BYTES
            )));
        }
        socket
            .send_to(payload, bridge_address)
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

fn resolve_bridge_address(bridge_address: &str) -> Result<SocketAddr, CreatorError> {
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
    Ok(bridge_address)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeFilterDrops {
    pub expired_lease: usize,
    pub expired_entry: usize,
    pub bad_signature: usize,
    pub relay_only: usize,
    pub suspect: usize,
    pub inactive: usize,
    pub no_ingress: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RouteCandidate {
    entry: BridgeDhtEntry,
    last_seen_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendDummyResult {
    pub chain_id: String,
    pub actor_id: String,
    pub route_source: String,
    pub candidate_bridge_ids: Vec<String>,
    pub selected_bridge_ids: Vec<String>,
    pub assigned_bridge_id: String,
    pub encryption_envelope: String,
    pub ciphertext_only_at_bridge: bool,
    pub frames: u32,
    pub elapsed_ms: u64,
    pub force_bridge_failure_used: bool,
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
    synthesize_frame_with_marker(size, None)
}

fn synthesize_frame_with_marker(size: usize, marker: Option<&[u8]>) -> Vec<u8> {
    let mut buf = Vec::with_capacity(size);
    if let Some(marker) = marker.filter(|marker| !marker.is_empty()) {
        let marker_len = marker.len().min(size);
        buf.extend_from_slice(&marker[..marker_len]);
    }
    for i in 0..size {
        if buf.len() >= size {
            break;
        }
        buf.push((i % 251) as u8);
    }
    buf
}

fn frame_fragment_bytes() -> usize {
    std::env::var("GBN_BRIDGE_UPLOAD_FRAGMENT_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map(|value| value.clamp(MIN_FRAME_FRAGMENT_BYTES, MAX_FRAME_FRAGMENT_BYTES))
        .unwrap_or(DEFAULT_FRAME_FRAGMENT_BYTES)
}

fn upload_close_timeout() -> Duration {
    std::env::var("GBN_BRIDGE_CREATOR_UPLOAD_CLOSE_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(timeout_from_ms)
        .unwrap_or_else(|| timeout_from_ms(DEFAULT_UPLOAD_CLOSE_TIMEOUT_MS))
}

fn ensure_onboarded(table: &LocalDiscoveryTable) -> Result<(), CreatorError> {
    if matches!(
        table.self_onboarding_state,
        SelfOnboardingState::Onboarded | SelfOnboardingState::FanoutPartial
    ) {
        return Ok(());
    }
    Err(CreatorError::CreatorNotOnboarded {
        current_state: serde_json::to_value(table.self_onboarding_state)
            .ok()
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| format!("{:?}", table.self_onboarding_state)),
    })
}

fn select_route_candidates(
    table: &LocalDiscoveryTable,
    publisher_pub: &PublicKeyBytes,
    now_ms: u64,
) -> (Vec<RouteCandidate>, BridgeFilterDrops) {
    let mut drops = BridgeFilterDrops::default();
    let mut candidates = Vec::new();

    for entry in &table.bridge_entries {
        if now_ms > entry.lease_expiry_ms {
            drops.expired_lease += 1;
            continue;
        }
        if now_ms > entry.entry_expiry_ms {
            drops.expired_entry += 1;
            continue;
        }
        if entry.verify_authority(publisher_pub, now_ms).is_err() {
            drops.bad_signature += 1;
            continue;
        }
        if matches!(entry.reachability_class, ReachabilityClass::RelayOnly) {
            drops.relay_only += 1;
            continue;
        }
        if entry
            .suspect_until_ms
            .is_some_and(|suspect| suspect > now_ms)
        {
            drops.suspect += 1;
            continue;
        }
        if !entry.active {
            drops.inactive += 1;
            continue;
        }
        if bridge_upload_address(entry).is_err() {
            drops.no_ingress += 1;
            continue;
        }
        let last_seen_ms = table
            .active_tunnels
            .iter()
            .filter(|tunnel| tunnel.peer_id == entry.bridge_id)
            .map(|tunnel| tunnel.last_seen_ms)
            .max()
            .unwrap_or(0);
        candidates.push(RouteCandidate {
            entry: entry.clone(),
            last_seen_ms,
        });
    }

    candidates.sort_by(|left, right| {
        right
            .last_seen_ms
            .cmp(&left.last_seen_ms)
            .then_with(|| right.entry.lease_expiry_ms.cmp(&left.entry.lease_expiry_ms))
            .then_with(|| left.entry.bridge_id.cmp(&right.entry.bridge_id))
    });
    (candidates, drops)
}

fn bridge_upload_address(entry: &BridgeDhtEntry) -> Result<String, CreatorError> {
    let endpoint = entry
        .ingress_endpoints
        .iter()
        .find(|endpoint| {
            matches!(
                endpoint.kind,
                BridgeIngressEndpointKind::Direct | BridgeIngressEndpointKind::Brokered
            )
        })
        .or_else(|| entry.ingress_endpoints.first())
        .ok_or_else(|| {
            CreatorError::Protocol(format!(
                "bridge `{}` has no ingress endpoint in local DHT",
                entry.bridge_id
            ))
        })?;
    endpoint_upload_address(endpoint)
}

fn endpoint_upload_address(endpoint: &DhtBridgeIngressEndpoint) -> Result<String, CreatorError> {
    if endpoint.ip_addr.trim().is_empty() || endpoint.port == 0 {
        return Err(CreatorError::Protocol(
            "bridge ingress endpoint has empty host or zero port".to_string(),
        ));
    }
    Ok(format!("{}:{}", endpoint.ip_addr, endpoint.port))
}

fn session_id_bytes(chain_id: &str) -> [u8; 16] {
    let digest = Sha256::digest(chain_id.as_bytes());
    let mut out = [0_u8; 16];
    out.copy_from_slice(&digest[..16]);
    out
}

fn ephemeral_private_bytes(actor_id: &str, chain_id: &str, now_ms: u64) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"veritas/conduit/v2/send-dummy-ephemeral");
    hasher.update(actor_id.as_bytes());
    hasher.update(chain_id.as_bytes());
    hasher.update(now_ms.to_le_bytes());
    hasher.finalize().into()
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn default_chain_id(prefix: &str, actor_id: &str, request_id: &str) -> String {
    format!("{prefix}-{actor_id}-{request_id}")
}

fn timeout_from_ms(timeout_ms: u64) -> Duration {
    Duration::from_millis(timeout_ms.max(1))
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
