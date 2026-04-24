use crate::host_creator_client::HostCreatorClient;
use crate::network_transport::default_chain_id;
use crate::{CreatorRuntime, ExitBridgeRuntime, RuntimeResult};

#[derive(Debug)]
pub struct HostCreator {
    host_creator_id: String,
    client: Option<HostCreatorClient>,
}

impl HostCreator {
    pub fn new(host_creator_id: impl Into<String>) -> Self {
        Self {
            host_creator_id: host_creator_id.into(),
            client: None,
        }
    }

    pub fn host_creator_id(&self) -> &str {
        &self.host_creator_id
    }

    pub fn attach_client(&mut self, client: HostCreatorClient) -> RuntimeResult<()> {
        if client.host_creator_id() != self.host_creator_id {
            return Err(crate::RuntimeError::HostCreatorClientMismatch {
                expected_host_creator_id: self.host_creator_id.clone(),
                actual_actor_id: client.host_creator_id().to_string(),
            });
        }
        self.client = Some(client);
        Ok(())
    }

    pub fn forward_join_request(
        &mut self,
        creator: &CreatorRuntime,
        relay_bridge: &mut ExitBridgeRuntime,
        request_id: &str,
        now_ms: u64,
    ) -> RuntimeResult<gbn_bridge_protocol::BootstrapJoinReply> {
        let chain_id = default_chain_id("bootstrap", &self.host_creator_id, request_id);
        let relay_bridge_id = relay_bridge.config().bridge_id.clone();
        let reply = if let Some(client) = self.client.as_mut() {
            client.forward_join_request(creator, &relay_bridge_id, request_id, &chain_id, now_ms)?
        } else if relay_bridge.has_simulation_publisher_client() {
            relay_bridge.authority_client_mut().begin_bootstrap(
                &chain_id,
                gbn_bridge_protocol::CreatorJoinRequest {
                    chain_id: chain_id.clone(),
                    request_id: request_id.to_string(),
                    host_creator_id: self.host_creator_id.clone(),
                    relay_bridge_id,
                    creator: creator.pending_creator(),
                },
                now_ms,
            )?
        } else {
            return Err(crate::RuntimeError::MissingHostCreatorClient {
                host_creator_id: self.host_creator_id.clone(),
            });
        };
        relay_bridge.remember_bootstrap_chain_id(&reply.response.bootstrap_session_id, &chain_id);
        Ok(reply)
    }
}
