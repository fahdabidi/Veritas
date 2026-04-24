use ed25519_dalek::SigningKey;
use gbn_bridge_protocol::{
    publisher_identity, BridgeAck, BridgeClose, BridgeData, BridgeOpen, PublicKeyBytes,
};
use gbn_bridge_publisher::api::{
    AuthorityApiRequest, AuthorityApiRequestUnsigned, AuthorityApiResponse, AuthorityRoute,
    EmptyResponse, ReceiverCloseBody, ReceiverFrameBody, ReceiverOpenBody,
};

use crate::network_transport::{default_request_id, HttpJsonTransport, TransportMetadata};
use crate::{RuntimeError, RuntimeResult};

const HTTP_TIMESTAMP_GUARD_MS: u64 = 25;

#[derive(Debug, Clone)]
pub struct ForwarderClient {
    actor_id: String,
    actor_pub: PublicKeyBytes,
    signing_key: SigningKey,
    publisher_pub: PublicKeyBytes,
    transport: HttpJsonTransport,
}

impl ForwarderClient {
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

    pub fn open_session(
        &mut self,
        chain_id: &str,
        open: BridgeOpen,
        now_ms: u64,
    ) -> RuntimeResult<()> {
        let request_id = default_request_id("receiver-open", &self.actor_id, now_ms);
        let _: AuthorityApiResponse<EmptyResponse> = self.post(
            AuthorityRoute::ReceiverOpen,
            TransportMetadata {
                chain_id: chain_id.to_string(),
                request_id,
                actor_id: self.actor_id.clone(),
                sent_at_ms: guarded_sent_at(now_ms),
            },
            ReceiverOpenBody { open },
        )?;
        Ok(())
    }

    pub fn forward_frame(
        &mut self,
        chain_id: &str,
        via_bridge_id: &str,
        frame: BridgeData,
        received_at_ms: u64,
    ) -> RuntimeResult<BridgeAck> {
        let request_id = default_request_id("receiver-frame", &self.actor_id, received_at_ms);
        let response: AuthorityApiResponse<BridgeAck> = self.post(
            AuthorityRoute::ReceiverFrame,
            TransportMetadata {
                chain_id: chain_id.to_string(),
                request_id,
                actor_id: self.actor_id.clone(),
                sent_at_ms: guarded_sent_at(received_at_ms),
            },
            ReceiverFrameBody {
                via_bridge_id: via_bridge_id.to_string(),
                frame,
                received_at_ms,
            },
        )?;
        response
            .body
            .ok_or_else(|| RuntimeError::AuthorityProtocol {
                detail: "receiver frame response had no body".into(),
            })
    }

    pub fn close_session(
        &mut self,
        chain_id: &str,
        bridge_id: &str,
        close: BridgeClose,
        now_ms: u64,
    ) -> RuntimeResult<()> {
        let request_id = default_request_id("receiver-close", &self.actor_id, now_ms);
        let _: AuthorityApiResponse<EmptyResponse> = self.post(
            AuthorityRoute::ReceiverClose,
            TransportMetadata {
                chain_id: chain_id.to_string(),
                request_id,
                actor_id: self.actor_id.clone(),
                sent_at_ms: guarded_sent_at(now_ms),
            },
            ReceiverCloseBody {
                bridge_id: bridge_id.to_string(),
                close,
            },
        )?;
        Ok(())
    }

    fn post<TBody, TResponse>(
        &mut self,
        route: AuthorityRoute,
        metadata: TransportMetadata,
        body: TBody,
    ) -> RuntimeResult<AuthorityApiResponse<TResponse>>
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
    ) -> RuntimeResult<AuthorityApiResponse<TResponse>>
    where
        TResponse: serde::Serialize + Clone,
    {
        response
            .verify_authority(&self.publisher_pub)
            .map_err(RuntimeError::from)?;
        if response.chain_id != metadata.chain_id {
            return Err(RuntimeError::AuthorityProtocol {
                detail: format!(
                    "receiver response chain_id mismatch: expected `{}`, got `{}`",
                    metadata.chain_id, response.chain_id
                ),
            });
        }
        if response.request_id != metadata.request_id {
            return Err(RuntimeError::AuthorityProtocol {
                detail: format!(
                    "receiver response request_id mismatch: expected `{}`, got `{}`",
                    metadata.request_id, response.request_id
                ),
            });
        }
        if !response.ok {
            let detail = response
                .error
                .map(|error| format!("{}: {}", error.code, error.message))
                .unwrap_or_else(|| format!("receiver route returned HTTP {status} without body"));
            return Err(RuntimeError::AuthorityProtocol { detail });
        }
        Ok(response)
    }
}

fn guarded_sent_at(now_ms: u64) -> u64 {
    now_ms.saturating_sub(HTTP_TIMESTAMP_GUARD_MS)
}
