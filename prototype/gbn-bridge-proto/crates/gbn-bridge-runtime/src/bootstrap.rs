use gbn_bridge_protocol::{
    BootstrapJoinReply, BridgeCommandAckStatus, BridgePunchAck, BridgePunchProbe, BridgeSetRequest,
    BridgeSetResponse,
};

use crate::{CreatorRuntime, ExitBridgeRuntime, HostCreator, RuntimeError, RuntimeResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapJoinPlan {
    pub chain_id: String,
    pub reply: BootstrapJoinReply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedTunnelOutcome {
    pub probe: BridgePunchProbe,
    pub bridge_ack: BridgePunchAck,
}

pub fn request_first_contact(
    creator: &mut CreatorRuntime,
    host_creator: &mut HostCreator,
    relay_bridge: &mut ExitBridgeRuntime,
    request_id: &str,
    now_ms: u64,
) -> RuntimeResult<BootstrapJoinPlan> {
    let reply = host_creator.forward_join_request(creator, relay_bridge, request_id, now_ms)?;
    creator.apply_bootstrap_response(&reply.response, now_ms)?;
    creator.remember_self_entry(reply.creator_entry.clone(), now_ms)?;
    Ok(BootstrapJoinPlan {
        chain_id: crate::network_transport::default_chain_id(
            "bootstrap",
            host_creator.host_creator_id(),
            request_id,
        ),
        reply,
    })
}

pub fn establish_seed_tunnel(
    creator: &mut CreatorRuntime,
    seed_bridge: &mut ExitBridgeRuntime,
    plan: &BootstrapJoinPlan,
    now_ms: u64,
) -> RuntimeResult<SeedTunnelOutcome> {
    if seed_bridge.config().bridge_id != plan.reply.response.seed_bridge.node_id {
        return Err(RuntimeError::UnexpectedBridgeRuntime {
            expected_bridge_id: plan.reply.response.seed_bridge.node_id.clone(),
            actual_bridge_id: seed_bridge.config().bridge_id.clone(),
        });
    }

    seed_bridge
        .remember_bootstrap_chain_id(&plan.reply.response.bootstrap_session_id, &plan.chain_id);
    let ack =
        seed_bridge
            .receive_next_control_command(now_ms)?
            .ok_or(RuntimeError::ControlProtocol {
                detail: "seed bridge did not receive bootstrap seed assignment".into(),
            })?;
    if ack.status != BridgeCommandAckStatus::Applied {
        return Err(RuntimeError::ControlProtocol {
            detail: format!(
                "seed bridge rejected bootstrap seed assignment with status `{}`",
                match ack.status {
                    BridgeCommandAckStatus::Applied => "applied",
                    BridgeCommandAckStatus::Duplicate => "duplicate",
                    BridgeCommandAckStatus::Rejected => "rejected",
                }
            ),
        });
    }
    let active_attempt = seed_bridge
        .active_punch_attempt(&plan.reply.response.bootstrap_session_id)
        .cloned()
        .ok_or_else(|| RuntimeError::BootstrapSessionNotTracked {
            bootstrap_session_id: plan.reply.response.bootstrap_session_id.clone(),
        })?;
    let source_udp_punch_port = seed_bridge
        .current_lease()
        .map(|lease| lease.udp_punch_port)
        .unwrap_or(seed_bridge.config().requested_udp_punch_port);
    let probe = BridgePunchProbe {
        chain_id: plan.chain_id.clone(),
        bootstrap_session_id: active_attempt.bootstrap_session_id.clone(),
        source_node_id: seed_bridge.config().bridge_id.clone(),
        source_pub_key: seed_bridge.config().identity_pub.clone(),
        source_ip_addr: seed_bridge.config().ingress_endpoint.host.clone(),
        source_udp_punch_port,
        probe_nonce: active_attempt.probe_nonce,
    };
    let bridge_ack = seed_bridge.acknowledge_tunnel(
        &probe.bootstrap_session_id,
        &creator.config().creator_id,
        creator.config().udp_punch_port,
        probe.probe_nonce,
        now_ms,
    )?;
    creator.mark_bridge_active(&plan.reply.response.seed_bridge.node_id, now_ms);

    Ok(SeedTunnelOutcome { probe, bridge_ack })
}

pub fn fetch_bridge_set(
    creator: &mut CreatorRuntime,
    seed_bridge: &mut ExitBridgeRuntime,
    plan: &BootstrapJoinPlan,
    now_ms: u64,
) -> RuntimeResult<BridgeSetResponse> {
    if seed_bridge.config().bridge_id != plan.reply.response.seed_bridge.node_id {
        return Err(RuntimeError::UnexpectedBridgeRuntime {
            expected_bridge_id: plan.reply.response.seed_bridge.node_id.clone(),
            actual_bridge_id: seed_bridge.config().bridge_id.clone(),
        });
    }

    let response = seed_bridge.serve_bridge_set(
        &BridgeSetRequest {
            chain_id: plan.chain_id.clone(),
            bootstrap_session_id: plan.reply.response.bootstrap_session_id.clone(),
            creator_id: creator.config().creator_id.clone(),
            requested_bridge_count: plan.reply.response.assigned_bridge_count,
        },
        now_ms,
    )?;

    if response.bootstrap_session_id != plan.reply.response.bootstrap_session_id {
        return Err(RuntimeError::BridgeSetSessionMismatch {
            expected_bootstrap_session_id: plan.reply.response.bootstrap_session_id.clone(),
            actual_bootstrap_session_id: response.bootstrap_session_id,
        });
    }

    creator.store_bridge_set(&response, now_ms)?;
    Ok(response)
}
