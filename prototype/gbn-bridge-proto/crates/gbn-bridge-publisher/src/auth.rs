use std::collections::BTreeMap;

use gbn_bridge_protocol::ProtocolError;
use serde::Serialize;
use thiserror::Error;

use crate::api::AuthorityApiRequest;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AuthError {
    #[error("chain_id must be non-empty")]
    EmptyChainId,

    #[error("request_id must be non-empty")]
    EmptyRequestId,

    #[error("actor_id must be non-empty")]
    EmptyActorId,

    #[error("replay detected for request `{request_id}`")]
    ReplayDetected { request_id: String },

    #[error("actor mismatch: expected `{expected}`, got `{actual}`")]
    ActorIdMismatch { expected: String, actual: String },

    #[error("actor key mismatch for `{actor_id}`")]
    ActorKeyMismatch { actor_id: String },

    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}

#[derive(Debug, Clone)]
pub struct RequestAuthenticator {
    max_skew_ms: u64,
    replay_ttl_ms: u64,
    seen_requests: BTreeMap<String, u64>,
}

impl RequestAuthenticator {
    pub fn new(max_skew_ms: u64, replay_ttl_ms: u64) -> Self {
        Self {
            max_skew_ms,
            replay_ttl_ms,
            seen_requests: BTreeMap::new(),
        }
    }

    pub fn verify_signed_request<T>(
        &mut self,
        request: &AuthorityApiRequest<T>,
        now_ms: u64,
    ) -> Result<(), AuthError>
    where
        T: Serialize + Clone,
    {
        if request.chain_id.trim().is_empty() {
            return Err(AuthError::EmptyChainId);
        }
        if request.request_id.trim().is_empty() {
            return Err(AuthError::EmptyRequestId);
        }
        if request.actor_id.trim().is_empty() {
            return Err(AuthError::EmptyActorId);
        }

        if request.sent_at_ms > now_ms {
            return Err(AuthError::Protocol(
                ProtocolError::ReplayTimestampInFuture {
                    sent_at_ms: request.sent_at_ms,
                    now_ms,
                },
            ));
        }

        if now_ms.saturating_sub(request.sent_at_ms) > self.max_skew_ms {
            return Err(AuthError::Protocol(ProtocolError::ReplayWindowExpired {
                sent_at_ms: request.sent_at_ms,
                now_ms,
                max_age_ms: self.max_skew_ms,
            }));
        }

        self.reap(now_ms);

        if self.seen_requests.contains_key(&request.request_id) {
            return Err(AuthError::ReplayDetected {
                request_id: request.request_id.clone(),
            });
        }

        request.verify_signature()?;
        self.seen_requests
            .insert(request.request_id.clone(), request.sent_at_ms);
        Ok(())
    }

    pub fn ensure_actor_id_matches(&self, expected: &str, actual: &str) -> Result<(), AuthError> {
        if expected != actual {
            return Err(AuthError::ActorIdMismatch {
                expected: expected.to_string(),
                actual: actual.to_string(),
            });
        }

        Ok(())
    }

    pub fn ensure_actor_key_matches(
        &self,
        actor_id: &str,
        expected: &gbn_bridge_protocol::PublicKeyBytes,
        actual: &gbn_bridge_protocol::PublicKeyBytes,
    ) -> Result<(), AuthError> {
        if expected != actual {
            return Err(AuthError::ActorKeyMismatch {
                actor_id: actor_id.to_string(),
            });
        }

        Ok(())
    }

    fn reap(&mut self, now_ms: u64) {
        let cutoff = now_ms.saturating_sub(self.replay_ttl_ms);
        self.seen_requests
            .retain(|_, seen_at_ms| *seen_at_ms >= cutoff);
    }
}
