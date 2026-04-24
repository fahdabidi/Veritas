use gbn_bridge_protocol::validate_chain_id;

use crate::RuntimeResult;

pub fn default_chain_id(prefix: &str, actor_id: &str, request_id: &str) -> String {
    format!("{prefix}-{actor_id}-{request_id}")
}

pub fn import_chain_id(chain_id: &str) -> RuntimeResult<String> {
    validate_chain_id(chain_id)?;
    Ok(chain_id.trim().to_string())
}
