mod common;

#[path = "integration/test_batch_bootstrap.rs"]
mod test_batch_bootstrap;
#[path = "integration/test_bridge_registration.rs"]
mod test_bridge_registration;
#[path = "integration/test_bridge_reuse_timeout.rs"]
mod test_bridge_reuse_timeout;
#[path = "integration/test_catalog_refresh.rs"]
mod test_catalog_refresh;
#[path = "integration/test_chain_id.rs"]
mod test_chain_id;
#[path = "integration/test_creator_failover.rs"]
mod test_creator_failover;
#[path = "integration/test_first_creator_bootstrap.rs"]
mod test_first_creator_bootstrap;
#[path = "integration/test_payload_confidentiality.rs"]
mod test_payload_confidentiality;
#[path = "integration/test_reachability_filtering.rs"]
mod test_reachability_filtering;
#[path = "integration/test_udp_punch_ack.rs"]
mod test_udp_punch_ack;
