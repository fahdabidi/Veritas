use libp2p::{
    request_response::{self, ProtocolSupport},
    PeerId, StreamProtocol,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};

pub type MessageId = [u8; 32];
pub type PlumTreeBehaviour = request_response::cbor::Behaviour<GossipRequest, GossipResponse>;

use crate::circuit_manager::RelayNode;
#[cfg(feature = "distributed-trace")]
use crate::trace;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GbnGossipMsg {
    TestString(String),
    DirectorySync(Vec<RelayNode>),
    NodeAnnounce(RelayNode),
}

#[cfg(feature = "distributed-trace")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEnvelope {
    pub chain: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipRequest {
    GossipData {
        message_id: MessageId,
        payload: Vec<u8>,
        #[cfg(feature = "distributed-trace")]
        trace: TraceEnvelope,
    },
    IHave {
        message_ids: Vec<MessageId>,
    },
    IWant {
        message_ids: Vec<MessageId>,
    },
    Prune,
    Graft,
    DirectNodeAnnounce {
        node: RelayNode,
    },
    DirectNodePropagate {
        snapshot_ts_ms: u64,
        nodes: Vec<RelayNode>,
    },
    DirectNodeProbe,
    DirectNodeProbeResponse {
        node: RelayNode,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipResponse {
    Ack,
}

pub fn new_plumtree_behaviour() -> PlumTreeBehaviour {
    let protocol = StreamProtocol::new("/gbn/plumtree/1.0.0");
    let cfg = request_response::Config::default();
    request_response::cbor::Behaviour::new([(protocol, ProtocolSupport::Full)], cfg)
}

#[derive(Debug, Clone)]
pub struct TokenBucket {
    capacity: usize,
    tokens: f64,
    refill_per_sec: f64,
    last_refill: Instant,
}

impl TokenBucket {
    pub fn new(bytes_per_sec: usize) -> Self {
        Self {
            capacity: bytes_per_sec,
            tokens: bytes_per_sec as f64,
            refill_per_sec: bytes_per_sec as f64,
            last_refill: Instant::now(),
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.last_refill = now;
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity as f64);
    }

    pub fn try_consume(&mut self, bytes: usize) -> bool {
        self.refill();
        if self.tokens >= bytes as f64 {
            self.tokens -= bytes as f64;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlumTreeState {
    pub eager_peers: HashSet<PeerId>,
    pub lazy_peers: HashSet<PeerId>,
    pub missing_messages: HashMap<MessageId, Instant>,
    seen: VecDeque<MessageId>,
    seen_set: HashSet<MessageId>,
    max_tracked_messages: usize,
    budget: TokenBucket,
    bytes_sent_total: u64,
    messages_seen_total: u64,
    messages_dropped_budget_total: u64,
    lazy_repairs_total: u64,
}

#[derive(Debug, Clone)]
pub struct OutboundGossip {
    pub peer: PeerId,
    pub request: GossipRequest,
}

#[derive(Debug, Clone)]
pub struct PlumTreeEngine {
    pub state: PlumTreeState,
    payloads: HashMap<MessageId, Vec<u8>>,
    #[cfg(feature = "distributed-trace")]
    trace_chains: HashMap<MessageId, Vec<String>>,
}

impl PlumTreeState {
    pub fn new(gossip_bps: usize, max_tracked_messages: usize) -> Self {
        Self {
            eager_peers: HashSet::new(),
            lazy_peers: HashSet::new(),
            missing_messages: HashMap::new(),
            seen: VecDeque::new(),
            seen_set: HashSet::new(),
            max_tracked_messages,
            budget: TokenBucket::new(gossip_bps),
            bytes_sent_total: 0,
            messages_seen_total: 0,
            messages_dropped_budget_total: 0,
            lazy_repairs_total: 0,
        }
    }

    pub fn register_seen(&mut self, message_id: MessageId) -> bool {
        if self.seen_set.contains(&message_id) {
            return false;
        }
        self.seen.push_back(message_id);
        self.seen_set.insert(message_id);
        self.messages_seen_total += 1;
        while self.seen.len() > self.max_tracked_messages {
            if let Some(old) = self.seen.pop_front() {
                self.seen_set.remove(&old);
            }
        }
        true
    }

    pub fn try_account_send(&mut self, bytes: usize) -> bool {
        if self.budget.try_consume(bytes) {
            self.bytes_sent_total += bytes as u64;
            true
        } else {
            self.messages_dropped_budget_total += 1;
            false
        }
    }

    pub fn note_lazy_repair(&mut self, count: usize) {
        self.lazy_repairs_total += count as u64;
    }

    pub fn expire_missing_older_than(&mut self, ttl: Duration) {
        let now = Instant::now();
        self.missing_messages
            .retain(|_, t| now.duration_since(*t) <= ttl);
    }

    pub fn bytes_sent_total(&self) -> u64 {
        self.bytes_sent_total
    }
    pub fn messages_seen_total(&self) -> u64 {
        self.messages_seen_total
    }
    pub fn messages_dropped_budget_total(&self) -> u64 {
        self.messages_dropped_budget_total
    }
    pub fn lazy_repairs_total(&self) -> u64 {
        self.lazy_repairs_total
    }
}

impl PlumTreeEngine {
    pub fn new(gossip_bps: usize, max_tracked_messages: usize) -> Self {
        Self {
            state: PlumTreeState::new(gossip_bps, max_tracked_messages),
            payloads: HashMap::new(),
            #[cfg(feature = "distributed-trace")]
            trace_chains: HashMap::new(),
        }
    }

    pub fn add_eager_peer(&mut self, peer: PeerId) {
        self.state.lazy_peers.remove(&peer);
        self.state.eager_peers.insert(peer);
    }

    pub fn add_lazy_peer(&mut self, peer: PeerId) {
        self.state.eager_peers.remove(&peer);
        self.state.lazy_peers.insert(peer);
    }

    pub fn publish_local(
        &mut self,
        message_id: MessageId,
        payload: Vec<u8>,
    ) -> Vec<OutboundGossip> {
        self.payloads.insert(message_id, payload.clone());
        #[cfg(feature = "distributed-trace")]
        {
            self.trace_chains
                .insert(message_id, vec![trace::next_hop_id()]);
        }
        if !self.state.register_seen(message_id) {
            return Vec::new();
        }
        self.build_forwarding(None, message_id, &payload)
    }

    pub fn on_request(&mut self, from: PeerId, request: GossipRequest) -> Vec<OutboundGossip> {
        match request {
            #[cfg(feature = "distributed-trace")]
            GossipRequest::GossipData {
                message_id,
                payload,
                trace,
            } => {
                let mut chain = trace.chain;
                chain.push(trace::next_hop_id());
                self.trace_chains.insert(message_id, chain);
                if !self.state.register_seen(message_id) {
                    return vec![OutboundGossip {
                        peer: from,
                        request: GossipRequest::Prune,
                    }];
                }

                self.payloads.insert(message_id, payload.clone());
                self.state.missing_messages.remove(&message_id);
                self.add_eager_peer(from);
                self.build_forwarding(Some(from), message_id, &payload)
            }
            #[cfg(not(feature = "distributed-trace"))]
            GossipRequest::GossipData {
                message_id,
                payload,
            } => {
                if !self.state.register_seen(message_id) {
                    // Duplicate on eager edge -> ask sender to PRUNE us.
                    return vec![OutboundGossip {
                        peer: from,
                        request: GossipRequest::Prune,
                    }];
                }

                self.payloads.insert(message_id, payload.clone());
                self.state.missing_messages.remove(&message_id);
                self.add_eager_peer(from);
                self.build_forwarding(Some(from), message_id, &payload)
            }
            GossipRequest::IHave { message_ids } => {
                let mut wanted = Vec::new();
                for id in message_ids {
                    if !self.payloads.contains_key(&id) {
                        self.state.missing_messages.insert(id, Instant::now());
                        wanted.push(id);
                    }
                }
                if wanted.is_empty() {
                    Vec::new()
                } else {
                    self.state.note_lazy_repair(wanted.len());
                    vec![OutboundGossip {
                        peer: from,
                        request: GossipRequest::IWant {
                            message_ids: wanted,
                        },
                    }]
                }
            }
            GossipRequest::IWant { message_ids } => {
                let mut out = Vec::new();
                for id in message_ids {
                    if let Some(payload) = self.payloads.get(&id).cloned() {
                        if self.state.try_account_send(payload.len() + 32) {
                            #[cfg(feature = "distributed-trace")]
                            let trace_chain = self
                                .trace_chains
                                .get(&id)
                                .cloned()
                                .unwrap_or_else(|| vec![trace::next_hop_id()]);
                            out.push(OutboundGossip {
                                peer: from,
                                #[cfg(feature = "distributed-trace")]
                                request: GossipRequest::GossipData {
                                    message_id: id,
                                    payload,
                                    trace: TraceEnvelope { chain: trace_chain },
                                },
                                #[cfg(not(feature = "distributed-trace"))]
                                request: GossipRequest::GossipData {
                                    message_id: id,
                                    payload,
                                },
                            });
                        }
                    }
                }
                out
            }
            GossipRequest::Prune => {
                self.add_lazy_peer(from);
                Vec::new()
            }
            GossipRequest::Graft => {
                self.add_eager_peer(from);
                Vec::new()
            }
            GossipRequest::DirectNodeAnnounce { .. }
            | GossipRequest::DirectNodePropagate { .. }
            | GossipRequest::DirectNodeProbe
            | GossipRequest::DirectNodeProbeResponse { .. } => Vec::new(),
        }
    }

    fn build_forwarding(
        &mut self,
        from: Option<PeerId>,
        message_id: MessageId,
        payload: &[u8],
    ) -> Vec<OutboundGossip> {
        let mut out = Vec::new();
        #[cfg(feature = "distributed-trace")]
        let trace_chain = self
            .trace_chains
            .get(&message_id)
            .cloned()
            .unwrap_or_else(|| vec![trace::next_hop_id()]);
        let eager_targets: Vec<_> = self
            .state
            .eager_peers
            .iter()
            .copied()
            .filter(|p| Some(*p) != from)
            .collect();

        for peer in eager_targets {
            if self.state.try_account_send(payload.len() + 32) {
                out.push(OutboundGossip {
                    peer,
                    #[cfg(feature = "distributed-trace")]
                    request: GossipRequest::GossipData {
                        message_id,
                        payload: payload.to_vec(),
                        trace: TraceEnvelope {
                            chain: trace_chain.clone(),
                        },
                    },
                    #[cfg(not(feature = "distributed-trace"))]
                    request: GossipRequest::GossipData {
                        message_id,
                        payload: payload.to_vec(),
                    },
                });
            }
        }

        let lazy_targets: Vec<_> = self
            .state
            .lazy_peers
            .iter()
            .copied()
            .filter(|p| Some(*p) != from)
            .collect();
        for peer in lazy_targets {
            if self.state.try_account_send(32) {
                out.push(OutboundGossip {
                    peer,
                    request: GossipRequest::IHave {
                        message_ids: vec![message_id],
                    },
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::identity;

    #[test]
    fn dedup_register_seen_works() {
        let mut s = PlumTreeState::new(1024, 2);
        let id = [1u8; 32];
        assert!(s.register_seen(id));
        assert!(!s.register_seen(id));
    }

    #[test]
    fn rate_limit_tracks_drops() {
        let mut s = PlumTreeState::new(1, 10);
        assert!(!s.try_account_send(64));
        assert!(s.messages_dropped_budget_total() >= 1);
    }

    #[test]
    fn missing_expiry_works() {
        let mut s = PlumTreeState::new(1024, 10);
        s.missing_messages
            .insert([3u8; 32], Instant::now() - Duration::from_secs(120));
        s.expire_missing_older_than(Duration::from_secs(1));
        assert!(s.missing_messages.is_empty());
    }

    #[test]
    fn i_have_triggers_lazy_repair() {
        let mut e = PlumTreeEngine::new(1024, 10);
        let from = identity::Keypair::generate_ed25519().public().to_peer_id();
        let id = [9u8; 32];
        let out = e.on_request(
            from,
            GossipRequest::IHave {
                message_ids: vec![id],
            },
        );
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0].request, GossipRequest::IWant { .. }));
        assert_eq!(e.state.lazy_repairs_total(), 1);
    }

    #[test]
    fn publish_local_sends_full_payload_only_to_eager_peers() {
        let mut e = PlumTreeEngine::new(1024 * 1024, 10);
        let eager_a = identity::Keypair::generate_ed25519().public().to_peer_id();
        let eager_b = identity::Keypair::generate_ed25519().public().to_peer_id();
        let lazy = identity::Keypair::generate_ed25519().public().to_peer_id();
        e.add_eager_peer(eager_a);
        e.add_eager_peer(eager_b);
        e.add_lazy_peer(lazy);

        let out = e.publish_local([7u8; 32], b"hello".to_vec());
        assert_eq!(out.len(), 3);

        let mut full_payload_targets = Vec::new();
        let mut lazy_targets = Vec::new();
        for item in out {
            match item.request {
                GossipRequest::GossipData { .. } => full_payload_targets.push(item.peer),
                GossipRequest::IHave { .. } => lazy_targets.push(item.peer),
                other => panic!("unexpected forwarding request: {other:?}"),
            }
        }

        assert_eq!(full_payload_targets.len(), 2);
        assert!(full_payload_targets.contains(&eager_a));
        assert!(full_payload_targets.contains(&eager_b));
        assert_eq!(lazy_targets, vec![lazy]);
    }
}
