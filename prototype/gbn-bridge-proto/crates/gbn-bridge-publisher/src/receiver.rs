use gbn_bridge_protocol::{BridgeClose, BridgeOpen};

use crate::{AuthorityResult, PublisherAuthority};

pub fn open_session(
    authority: &mut PublisherAuthority,
    chain_id: &str,
    open: BridgeOpen,
) -> AuthorityResult<()> {
    authority.open_bridge_session_with_chain_id(Some(chain_id), open)
}

pub fn close_session(
    authority: &mut PublisherAuthority,
    chain_id: &str,
    close: BridgeClose,
) -> AuthorityResult<()> {
    authority.close_bridge_session_with_chain_id(Some(chain_id), close)
}
