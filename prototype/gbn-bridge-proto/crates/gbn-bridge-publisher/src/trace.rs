use gbn_bridge_protocol::validate_chain_id;

use crate::{AuthorityError, AuthorityResult};

pub fn inherited_chain_id(chain_id: &str) -> AuthorityResult<String> {
    validate_chain_id(chain_id)?;
    Ok(chain_id.trim().to_string())
}

pub fn optional_chain_id(chain_id: Option<&str>) -> AuthorityResult<Option<String>> {
    chain_id.map(inherited_chain_id).transpose()
}

pub fn ensure_matching_chain_id(
    context: &'static str,
    expected: &str,
    actual: &str,
) -> AuthorityResult<()> {
    if expected != actual {
        return Err(AuthorityError::ChainIdMismatch {
            context,
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(())
}
