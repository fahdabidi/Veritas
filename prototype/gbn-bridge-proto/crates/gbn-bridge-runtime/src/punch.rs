use std::collections::BTreeMap;

use gbn_bridge_protocol::{
    BootstrapDhtEntry, BridgePunchAck, BridgePunchProbe, BridgePunchStart, PublicKeyBytes,
};

use crate::{RuntimeError, RuntimeResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PunchAuthorization {
    PublisherInstruction,
    CreatorRefresh,
}

#[derive(Debug, Clone)]
pub struct ActivePunchAttempt {
    pub chain_id: String,
    pub bootstrap_session_id: String,
    pub authorization: PunchAuthorization,
    pub target: BootstrapDhtEntry,
    pub attempt_expiry_ms: u64,
    pub probe_nonce: u64,
}

#[derive(Debug, Clone, Default)]
pub struct PunchManager {
    attempts: BTreeMap<String, ActivePunchAttempt>,
    next_probe_nonce: u64,
    next_refresh_session_seq: u64,
}

#[derive(Debug, Clone)]
pub struct PunchSource {
    pub bridge_id: String,
    pub bridge_identity_pub: PublicKeyBytes,
    pub source_ip_addr: String,
    pub source_udp_punch_port: u16,
}

impl PunchManager {
    pub fn active_attempt_count(&self) -> usize {
        self.attempts.len()
    }

    pub fn active_attempt(&self, bootstrap_session_id: &str) -> Option<&ActivePunchAttempt> {
        self.attempts.get(bootstrap_session_id)
    }

    pub fn begin_from_instruction(
        &mut self,
        source: &PunchSource,
        publisher_key: &PublicKeyBytes,
        instruction: BridgePunchStart,
        now_ms: u64,
    ) -> RuntimeResult<BridgePunchProbe> {
        instruction.verify_authority(publisher_key, now_ms)?;
        if instruction.initiator_id != source.bridge_id {
            return Err(RuntimeError::PunchUnauthorized {
                reason: "instruction initiator does not match bridge id",
            });
        }

        self.insert_attempt(
            instruction.chain_id,
            instruction.bootstrap_session_id,
            PunchAuthorization::PublisherInstruction,
            instruction.target,
            instruction.attempt_expiry_ms,
            source,
        )
    }

    pub fn begin_from_refresh_entry(
        &mut self,
        source: &PunchSource,
        publisher_key: &PublicKeyBytes,
        target: BootstrapDhtEntry,
        now_ms: u64,
    ) -> RuntimeResult<BridgePunchProbe> {
        target.verify_authority(publisher_key, now_ms)?;
        self.next_refresh_session_seq += 1;
        let bootstrap_session_id = format!("refresh-{:06}", self.next_refresh_session_seq);

        self.insert_attempt(
            bootstrap_session_id.clone(),
            bootstrap_session_id,
            PunchAuthorization::CreatorRefresh,
            target.clone(),
            target.entry_expiry_ms,
            source,
        )
    }

    pub fn acknowledge(
        &mut self,
        bootstrap_session_id: &str,
        source_node_id: &str,
        responder_node_id: &str,
        observed_udp_punch_port: u16,
        acked_probe_nonce: u64,
        established_at_ms: u64,
    ) -> RuntimeResult<BridgePunchAck> {
        let attempt = self.attempts.get(bootstrap_session_id).ok_or_else(|| {
            RuntimeError::BootstrapSessionNotTracked {
                bootstrap_session_id: bootstrap_session_id.to_string(),
            }
        })?;

        if established_at_ms > attempt.attempt_expiry_ms {
            return Err(RuntimeError::PunchAttemptExpired {
                bootstrap_session_id: bootstrap_session_id.to_string(),
                attempt_expiry_ms: attempt.attempt_expiry_ms,
                now_ms: established_at_ms,
            });
        }

        if acked_probe_nonce != attempt.probe_nonce {
            return Err(RuntimeError::ProbeNonceMismatch {
                bootstrap_session_id: bootstrap_session_id.to_string(),
                expected: attempt.probe_nonce,
                actual: acked_probe_nonce,
            });
        }

        Ok(BridgePunchAck {
            chain_id: attempt.chain_id.clone(),
            bootstrap_session_id: bootstrap_session_id.to_string(),
            source_node_id: source_node_id.to_string(),
            responder_node_id: responder_node_id.to_string(),
            observed_udp_punch_port,
            acked_probe_nonce,
            established_at_ms,
        })
    }

    fn insert_attempt(
        &mut self,
        chain_id: String,
        bootstrap_session_id: String,
        authorization: PunchAuthorization,
        target: BootstrapDhtEntry,
        attempt_expiry_ms: u64,
        source: &PunchSource,
    ) -> RuntimeResult<BridgePunchProbe> {
        self.next_probe_nonce += 1;
        let probe_nonce = self.next_probe_nonce;
        self.attempts.insert(
            bootstrap_session_id.clone(),
            ActivePunchAttempt {
                chain_id: chain_id.clone(),
                bootstrap_session_id: bootstrap_session_id.clone(),
                authorization,
                target: target.clone(),
                attempt_expiry_ms,
                probe_nonce,
            },
        );

        Ok(BridgePunchProbe {
            chain_id,
            bootstrap_session_id,
            source_node_id: source.bridge_id.clone(),
            source_pub_key: source.bridge_identity_pub.clone(),
            source_ip_addr: source.source_ip_addr.clone(),
            source_udp_punch_port: source.source_udp_punch_port,
            probe_nonce,
        })
    }
}
