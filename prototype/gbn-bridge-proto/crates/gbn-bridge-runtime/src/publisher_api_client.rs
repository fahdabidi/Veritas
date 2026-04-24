use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BootstrapJoinReply, BootstrapProgress, BridgeCatalogRequest,
    BridgeCatalogResponse, BridgeHeartbeat, BridgeLease, BridgeRegister, CreatorJoinRequest,
    PublicKeyBytes, ReachabilityClass,
};
use gbn_bridge_publisher::{
    api::{AuthorityApiRequest, AuthorityApiRequestUnsigned, AuthorityApiResponse, AuthorityRoute},
    BootstrapJoinBody, BootstrapProgressBody, BootstrapProgressReceipt, BridgeHeartbeatBody,
    BridgeRegisterBody, CreatorCatalogBody,
};

use crate::network_transport::{
    default_chain_id, default_request_id, HttpJsonTransport, TransportMetadata,
};
use crate::{RuntimeError, RuntimeResult};

#[derive(Debug, Clone)]
pub struct PublisherApiClient {
    actor_id: String,
    actor_pub: PublicKeyBytes,
    signing_key: SigningKey,
    publisher_pub: PublicKeyBytes,
    transport: HttpJsonTransport,
}

impl PublisherApiClient {
    pub fn new(
        actor_id: impl Into<String>,
        signing_key: SigningKey,
        publisher_pub: PublicKeyBytes,
        transport: HttpJsonTransport,
    ) -> Self {
        let actor_pub = publisher_identity(&signing_key);
        Self {
            actor_id: actor_id.into(),
            actor_pub,
            signing_key,
            publisher_pub,
            transport,
        }
    }

    pub fn actor_id(&self) -> &str {
        &self.actor_id
    }

    pub fn actor_pub(&self) -> &PublicKeyBytes {
        &self.actor_pub
    }

    pub fn publisher_public_key(&self) -> &PublicKeyBytes {
        &self.publisher_pub
    }

    pub fn register_bridge(
        &mut self,
        register: BridgeRegister,
        reachability_class: ReachabilityClass,
        now_ms: u64,
    ) -> RuntimeResult<BridgeLease> {
        let request_id = default_request_id("bridge-register", &self.actor_id, now_ms);
        let chain_id = default_chain_id("bridge-register", &self.actor_id, &request_id);
        self.post(
            AuthorityRoute::BridgeRegister,
            TransportMetadata {
                chain_id,
                request_id,
                actor_id: self.actor_id.clone(),
                sent_at_ms: now_ms,
            },
            BridgeRegisterBody {
                register,
                reachability_class,
                now_ms,
            },
        )
    }

    pub fn renew_lease(&mut self, heartbeat: BridgeHeartbeat) -> RuntimeResult<BridgeLease> {
        let request_id = default_request_id(
            "bridge-heartbeat",
            &self.actor_id,
            heartbeat.heartbeat_at_ms,
        );
        let chain_id = default_chain_id("bridge-heartbeat", &self.actor_id, &request_id);
        self.post(
            AuthorityRoute::BridgeHeartbeat,
            TransportMetadata {
                chain_id,
                request_id,
                actor_id: self.actor_id.clone(),
                sent_at_ms: heartbeat.heartbeat_at_ms,
            },
            BridgeHeartbeatBody { heartbeat },
        )
    }

    pub fn issue_catalog(
        &mut self,
        chain_id: &str,
        request: &BridgeCatalogRequest,
        now_ms: u64,
    ) -> RuntimeResult<BridgeCatalogResponse> {
        let request_id = default_request_id("creator-catalog", &self.actor_id, now_ms);
        self.post(
            AuthorityRoute::CreatorCatalog,
            TransportMetadata {
                chain_id: chain_id.to_string(),
                request_id,
                actor_id: self.actor_id.clone(),
                sent_at_ms: now_ms,
            },
            CreatorCatalogBody {
                request: request.clone(),
                now_ms,
            },
        )
    }

    pub fn begin_bootstrap(
        &mut self,
        chain_id: &str,
        request: CreatorJoinRequest,
        now_ms: u64,
    ) -> RuntimeResult<BootstrapJoinReply> {
        let request_id = request.request_id.clone();
        self.post(
            AuthorityRoute::BootstrapJoin,
            TransportMetadata {
                chain_id: chain_id.to_string(),
                request_id,
                actor_id: self.actor_id.clone(),
                sent_at_ms: now_ms,
            },
            BootstrapJoinBody { request, now_ms },
        )
    }

    pub fn report_progress(
        &mut self,
        chain_id: &str,
        progress: BootstrapProgress,
    ) -> RuntimeResult<BootstrapProgressReceipt> {
        let request_id =
            default_request_id("bridge-progress", &self.actor_id, progress.reported_at_ms);
        self.post(
            AuthorityRoute::BridgeProgress,
            TransportMetadata {
                chain_id: chain_id.to_string(),
                request_id,
                actor_id: self.actor_id.clone(),
                sent_at_ms: progress.reported_at_ms,
            },
            BootstrapProgressBody { progress },
        )
    }

    fn post<TBody, TResponse>(
        &mut self,
        route: AuthorityRoute,
        metadata: TransportMetadata,
        body: TBody,
    ) -> RuntimeResult<TResponse>
    where
        TBody: serde::Serialize + Clone,
        TResponse: serde::de::DeserializeOwned + serde::Serialize + Clone,
    {
        let request = AuthorityApiRequest::sign(
            AuthorityApiRequestUnsigned {
                chain_id: metadata.chain_id.clone(),
                request_id: metadata.request_id.clone(),
                sent_at_ms: metadata.sent_at_ms,
                actor_id: metadata.actor_id.clone(),
                body,
            },
            &self.signing_key,
        )?;
        let (status, response): (u16, AuthorityApiResponse<TResponse>) =
            self.transport.post_json(route.path(), &request)?;
        self.verify_response(status, response, &metadata)
    }

    fn verify_response<TResponse>(
        &self,
        status: u16,
        response: AuthorityApiResponse<TResponse>,
        metadata: &TransportMetadata,
    ) -> RuntimeResult<TResponse>
    where
        TResponse: serde::Serialize + Clone,
    {
        response
            .verify_authority(&self.publisher_pub)
            .map_err(RuntimeError::from)?;
        if response.chain_id != metadata.chain_id {
            return Err(RuntimeError::AuthorityProtocol {
                detail: format!(
                    "authority response chain_id mismatch: expected `{}`, got `{}`",
                    metadata.chain_id, response.chain_id
                ),
            });
        }
        if response.request_id != metadata.request_id {
            return Err(RuntimeError::AuthorityProtocol {
                detail: format!(
                    "authority response request_id mismatch: expected `{}`, got `{}`",
                    metadata.request_id, response.request_id
                ),
            });
        }
        if !response.ok {
            let detail = response
                .error
                .map(|error| format!("{}: {}", error.code, error.message))
                .unwrap_or_else(|| format!("authority route returned HTTP {status} without body"));
            return Err(RuntimeError::AuthorityProtocol { detail });
        }
        response
            .body
            .ok_or_else(|| RuntimeError::AuthorityProtocol {
                detail: "authority response had no success body".into(),
            })
    }
}
