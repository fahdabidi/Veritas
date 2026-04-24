use crate::error::ProtocolError;

pub const CHAIN_ID_FIELD_NAME: &str = "chain_id";

pub type ChainId = String;

pub fn validate_chain_id(chain_id: &str) -> Result<(), ProtocolError> {
    if chain_id.trim().is_empty() {
        return Err(ProtocolError::Serialization(format!(
            "{CHAIN_ID_FIELD_NAME} must be non-empty"
        )));
    }

    Ok(())
}
