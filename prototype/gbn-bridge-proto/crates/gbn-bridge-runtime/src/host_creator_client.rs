use gbn_bridge_protocol::BootstrapJoinReply;

use crate::publisher_api_client::PublisherApiClient;
use crate::CreatorRuntime;
use crate::{RuntimeError, RuntimeResult};

#[derive(Debug, Clone)]
pub struct HostCreatorClient {
    host_creator_id: String,
    publisher_client: PublisherApiClient,
}

impl HostCreatorClient {
    pub fn new(
        host_creator_id: impl Into<String>,
        publisher_client: PublisherApiClient,
    ) -> RuntimeResult<Self> {
        let host_creator_id = host_creator_id.into();
        if publisher_client.actor_id() != host_creator_id {
            return Err(RuntimeError::HostCreatorClientMismatch {
                expected_host_creator_id: host_creator_id,
                actual_actor_id: publisher_client.actor_id().to_string(),
            });
        }
        Ok(Self {
            host_creator_id,
            publisher_client,
        })
    }

    pub fn host_creator_id(&self) -> &str {
        &self.host_creator_id
    }

    pub fn forward_join_request(
        &mut self,
        creator: &CreatorRuntime,
        relay_bridge_id: &str,
        request_id: &str,
        chain_id: &str,
        now_ms: u64,
    ) -> RuntimeResult<BootstrapJoinReply> {
        self.publisher_client.begin_bootstrap(
            chain_id,
            gbn_bridge_protocol::CreatorJoinRequest {
                chain_id: chain_id.to_string(),
                request_id: request_id.to_string(),
                host_creator_id: self.host_creator_id.clone(),
                relay_bridge_id: relay_bridge_id.to_string(),
                creator: creator.pending_creator(),
            },
            now_ms,
        )
    }
}
