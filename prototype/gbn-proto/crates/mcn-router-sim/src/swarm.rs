use crate::circuit_manager::{RelayNode, ValidationState};
use crate::control::{bootstrap_validate_unvalidated_node, minimal_bootstrap_snapshot};
use crate::gossip::{
    new_plumtree_behaviour, GbnGossipMsg, GossipRequest, GossipResponse, MessageId, OutboundGossip,
    PlumTreeBehaviour, PlumTreeEngine,
};
use crate::observability::MetricsReporter;
use anyhow::{Context, Result};
use libp2p::futures::StreamExt;
use libp2p::{
    core::ConnectedPoint,
    identity,
    kad::{store::MemoryStore, Behaviour as Kademlia, Config as KademliaConfig},
    multiaddr::Protocol,
    noise, request_response,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, PeerId, Swarm, SwarmBuilder,
};
use mcn_crypto::x25519_pubkey_from_privkey;
use rand::seq::SliceRandom;
use rand::Rng;
use std::time::Duration;
use std::{
    collections::HashMap,
    env,
    net::{IpAddr, SocketAddr},
    sync::{Arc, RwLock},
};
use tokio::time::Instant;

#[derive(NetworkBehaviour)]
pub struct RouterBehaviour {
    pub kademlia: Kademlia<MemoryStore>,
    pub gossip: PlumTreeBehaviour,
}

#[derive(Debug, Clone)]
pub struct GossipRuntime {
    pub engine: PlumTreeEngine,
    pub metrics: Option<MetricsReporter>,
    pub last_gossip_bytes_published: u64,
    pub last_gossip_publish: Instant,
    pub last_gossip_expiry: Instant,
    pub role: String,
    pub creator_publish_interval: Duration,
    pub last_creator_publish: Option<Instant>, // None = never published; triggers immediately once peers connect
    pub creator_seq: u64,
    pub last_rebootstrap: Instant,
    pub rebootstrap_interval: Duration,
    pub last_announce: Instant,
    pub announce_interval: Duration,
    pub last_propagate: Instant,
    pub propagate_interval: Duration,
    pub propagate_peer_percent: usize,
    pub propagate_entry_percent: usize,
    pub propagate_max_entries: usize,
    pub seed_store: Arc<RwLock<HashMap<SocketAddr, RelayNode>>>,
    pub peer_ip_map: HashMap<IpAddr, PeerId>,
    pub peer_ip_reverse: HashMap<PeerId, IpAddr>,
    pub pending_direct_validation: HashMap<IpAddr, String>,
    pub pending_bootstrap_validation: HashMap<SocketAddr, String>,
    pub noise_priv_key: [u8; 32],
}

pub enum SwarmControlCmd {
    DumpDht(tokio::sync::mpsc::Sender<Vec<String>>),
    BroadcastSeed,
    UnicastDHT { target_addr: String },
}

pub fn gossip_config_from_env() -> (usize, usize) {
    let gossip_bps = env::var("GBN_GOSSIP_BPS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(15 * 1024 * 1024 / 8);
    let max_tracked_messages = env::var("GBN_MAX_TRACKED_MESSAGES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .or_else(|| {
            env::var("GBN_MAX_TRACKED_PEERS")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
        })
        .unwrap_or(10_000);
    (gossip_bps, max_tracked_messages)
}

pub fn max_tracked_peers_from_env() -> usize {
    env::var("GBN_MAX_TRACKED_PEERS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(10_000)
}

fn role_participates_in_node_announce(role: &str) -> bool {
    matches!(role, "relay" | "creator" | "publisher")
}

impl GossipRuntime {
    pub async fn from_env(
        seed_store: Arc<RwLock<HashMap<SocketAddr, RelayNode>>>,
        noise_priv_key: [u8; 32],
    ) -> Self {
        let (gossip_bps, max_tracked_messages) = gossip_config_from_env();
        let metrics = MetricsReporter::from_env().await.ok();
        let role = env::var("GBN_ROLE").unwrap_or_else(|_| "relay".to_string());
        let creator_publish_interval = Duration::from_secs(
            env::var("GBN_CREATOR_PUBLISH_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(30),
        );
        let announce_interval = Duration::from_secs(
            env::var("GBN_NODE_ANNOUNCE_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(10),
        );
        let propagate_interval = Duration::from_secs(
            env::var("GBN_NODE_PROPAGATE_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(10),
        );
        Self {
            engine: PlumTreeEngine::new(gossip_bps, max_tracked_messages),
            metrics,
            last_gossip_bytes_published: 0,
            last_gossip_publish: Instant::now(),
            last_gossip_expiry: Instant::now(),
            role,
            creator_publish_interval,
            last_creator_publish: None, // None = never published; fires as soon as first peer connects
            creator_seq: 0,
            last_rebootstrap: Instant::now(),
            rebootstrap_interval: Duration::from_secs(
                env::var("GBN_REBOOTSTRAP_INTERVAL_SECS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(60),
            ),
            last_announce: Instant::now(),
            announce_interval,
            last_propagate: Instant::now(),
            propagate_interval,
            propagate_peer_percent: env::var("GBN_PROPAGATE_PEER_PERCENT")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .map(|v| v.clamp(1, 100))
                .unwrap_or(40),
            propagate_entry_percent: env::var("GBN_PROPAGATE_ENTRY_PERCENT")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .map(|v| v.clamp(1, 100))
                .unwrap_or(40),
            propagate_max_entries: env::var("GBN_PROPAGATE_MAX_ENTRIES")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(200),
            seed_store,
            peer_ip_map: HashMap::new(),
            peer_ip_reverse: HashMap::new(),
            pending_direct_validation: HashMap::new(),
            pending_bootstrap_validation: HashMap::new(),
            noise_priv_key,
        }
    }
}

pub async fn build_swarm(local_key: identity::Keypair) -> Result<Swarm<RouterBehaviour>> {
    let mut swarm = SwarmBuilder::with_existing_identity(local_key)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_behaviour(|key| {
            let peer_id = key.public().to_peer_id();
            let mut kad_config = KademliaConfig::default();
            // Faster queries for the simulated environment
            kad_config.set_query_timeout(Duration::from_secs(5));
            let mut store_config = libp2p::kad::store::MemoryStoreConfig::default();
            let max_tracked_peers = max_tracked_peers_from_env();
            store_config.max_records = max_tracked_peers;
            store_config.max_provided_keys = max_tracked_peers;
            let store = MemoryStore::with_config(peer_id, store_config);

            RouterBehaviour {
                kademlia: Kademlia::with_config(peer_id, store, kad_config),
                gossip: new_plumtree_behaviour(),
            }
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    // Start listening for inbound connections BEFORE registering with Cloud Map.
    // Without listen_on, nodes accept no inbound dials — ConnectionEstablished never
    // fires, lazy_peers stays empty, and gossip never flows even if dials are initiated.
    let p2p_port = env::var("GBN_P2P_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(4001);
    let listen_addr: libp2p::Multiaddr = format!("/ip4/0.0.0.0/tcp/{}", p2p_port)
        .parse()
        .expect("static multiaddr template");
    swarm.listen_on(listen_addr)?;

    let added = bootstrap_from_static_seeds(&mut swarm).await?;
    tokio::spawn(async move {
        match MetricsReporter::from_env().await {
            Ok(reporter) => {
                if let Err(e) = reporter.publish_bootstrap_result(added > 0).await {
                    tracing::warn!("CloudWatch publish BootstrapResult failed: {e}");
                }
            }
            Err(e) => tracing::warn!("CloudWatch MetricsReporter init failed: {e}"),
        }
    });

    Ok(swarm)
}

pub async fn run_swarm_until_ctrl_c(
    swarm: &mut Swarm<RouterBehaviour>,
    runtime: &mut GossipRuntime,
    mut control_rx: tokio::sync::mpsc::Receiver<SwarmControlCmd>,
) -> Result<()> {
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                break;
            }
            Some(cmd) = control_rx.recv() => {
                match cmd {
                    SwarmControlCmd::DumpDht(reply_tx) => {
                        let mut peers = Vec::new();
                        for bucket in swarm.behaviour_mut().kademlia.kbuckets() {
                            for entry in bucket.iter() {
                                peers.push(entry.node.key.preimage().to_string());
                            }
                        }
                        let _ = reply_tx.send(peers).await;
                    }
                    SwarmControlCmd::BroadcastSeed => {
                        tracing::info!("Swarm Control: Executing manual BroadcastSeed from local seed store...");
                        let nodes: Vec<RelayNode> = runtime.seed_store.read().unwrap().values().cloned().collect();
                        tracing::info!("Found {} nodes in local seed store for manual BroadcastSeed", nodes.len());
                        let msg = GbnGossipMsg::DirectorySync(nodes);
                        if let Ok(payload) = serde_json::to_vec(&msg) {
                            let mut msg_id = [0u8; 32];
                            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
                            msg_id[0..8].copy_from_slice(&ts.to_le_bytes());
                            msg_id[8..16].copy_from_slice(&0x5EED_5EED_u64.to_le_bytes());
                            let outbound = runtime.engine.publish_local(msg_id, payload);
                            send_outbound(swarm, outbound);
                        }
                    }
                    SwarmControlCmd::UnicastDHT { target_addr } => {
                        let target_sa: SocketAddr = match target_addr.parse() {
                            Ok(sa) => sa,
                            Err(e) => {
                                tracing::warn!("UnicastDHT: invalid target addr '{}': {}", target_addr, e);
                                continue;
                            }
                        };
                        let target_ip = target_sa.ip();
                        match runtime.peer_ip_map.get(&target_ip).copied() {
                            None => {
                                tracing::warn!("UnicastDHT: no connected libp2p peer found for IP {}", target_ip);
                                crate::control::push_packet_meta_trace(
                                    "ComponentError", 0,
                                    &format!("swarm.UnicastDHT ERROR no_peer_for_ip={}", target_ip),
                                    "", "component.error",
                                );
                            }
                            Some(peer_id) => {
                                if let Some(local_node) = get_local_relay_node() {
                                    let msg = GbnGossipMsg::NodeAnnounce(local_node);
                                    if let Ok(payload) = serde_json::to_vec(&msg) {
                                        let ts = std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap()
                                            .as_millis() as u64;
                                        let mut msg_id: MessageId = [0u8; 32];
                                        msg_id[0..8].copy_from_slice(&ts.to_le_bytes());
                                        // 0x554E4943 = "UNIC"
                                        msg_id[16..24].copy_from_slice(&0x554E_4943_u64.to_le_bytes());
                                        let request = GossipRequest::GossipData {
                                            message_id: msg_id,
                                            payload,
                                            #[cfg(feature = "distributed-trace")]
                                            trace: crate::gossip::TraceEnvelope {
                                                chain: vec![crate::trace::next_hop_id()],
                                            },
                                        };
                                        swarm.behaviour_mut().gossip.send_request(&peer_id, request);
                                        crate::control::push_packet_meta_trace(
                                            "ComponentOutput", 0,
                                            &format!("swarm.UnicastDHT OUTPUT sent NodeAnnounce to peer={:?} target={}", peer_id, target_addr),
                                            "", "component.output",
                                        );
                                        tracing::info!("UnicastDHT: sent NodeAnnounce to peer {:?} ({})", peer_id, target_addr);
                                    }
                                } else {
                                    tracing::warn!("UnicastDHT: no local relay node available (not a relay role?)");
                                }
                            }
                        }
                    }
                }
            }
            res = drive_swarm_once(swarm, runtime) => {
                res?;
            }
        }
    }
    Ok(())
}

fn ip_from_multiaddr(addr: &libp2p::Multiaddr) -> Option<IpAddr> {
    for proto in addr.iter() {
        match proto {
            Protocol::Ip4(ip) => return Some(IpAddr::V4(ip)),
            Protocol::Ip6(ip) => return Some(IpAddr::V6(ip)),
            _ => {}
        }
    }
    None
}

fn send_outbound(
    swarm: &mut Swarm<RouterBehaviour>,
    outbound: impl IntoIterator<Item = OutboundGossip>,
) {
    for msg in outbound {
        let (kind, bytes) = match &msg.request {
            GossipRequest::GossipData { payload, .. } => ("GossipData", payload.len()),
            GossipRequest::IHave { message_ids } => ("IHave", message_ids.len() * 32),
            GossipRequest::IWant { message_ids } => ("IWant", message_ids.len() * 32),
            GossipRequest::Prune => ("Prune", 0),
            GossipRequest::Graft => ("Graft", 0),
            GossipRequest::DirectNodeAnnounce { .. } => ("DirectNodeAnnounce", 0),
            GossipRequest::DirectNodePropagate { nodes, .. } => {
                ("DirectNodePropagate", nodes.len() * 64)
            }
            GossipRequest::DirectNodeProbe => ("DirectNodeProbe", 0),
            GossipRequest::DirectNodeProbeResponse { .. } => ("DirectNodeProbeResponse", 0),
        };
        #[cfg(feature = "distributed-trace")]
        let id_chain = match &msg.request {
            GossipRequest::GossipData { trace, .. } => crate::trace::chain_to_string(&trace.chain),
            _ => String::new(),
        };
        #[cfg(not(feature = "distributed-trace"))]
        let id_chain = String::new();
        crate::control::push_packet_meta_trace(
            "GossipSend",
            bytes,
            &format!("{kind} outbound to {}", msg.peer),
            &id_chain,
            "outgoing",
        );
        swarm
            .behaviour_mut()
            .gossip
            .send_request(&msg.peer, msg.request);
    }
}

fn apply_seed_update_from_gossip_msg(
    runtime: &mut GossipRuntime,
    gbn_msg: GbnGossipMsg,
    id_chain: &str,
) {
    match gbn_msg {
        GbnGossipMsg::DirectorySync(nodes) => {
            tracing::info!("Received DirectorySync with {} nodes", nodes.len());
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let mut store = runtime.seed_store.write().unwrap();
            let mut accepted = 0usize;
            let mut continuity_events = 0usize;
            for node in &nodes {
                let (was_accepted, events) = upsert_seed_store_node(
                    &mut store,
                    node.clone(),
                    now_ms,
                    SeedUpdateSource::Propagated,
                );
                if was_accepted {
                    accepted += 1;
                }
                continuity_events += events.len();
                for event in events {
                    tracing::warn!("Seed-store continuity: {}", event);
                }
            }
            crate::control::push_packet_meta_trace(
                "InternalAction",
                accepted * 32,
                &format!(
                    "DHT updated: DirectorySync accepted={} continuity_events={}",
                    accepted, continuity_events
                ),
                id_chain,
                "internal",
            );
        }
        GbnGossipMsg::NodeAnnounce(node) => {
            tracing::debug!("Received NodeAnnounce from {}", node.addr);
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let mut store = runtime.seed_store.write().unwrap();
            let (was_accepted, events) = upsert_seed_store_node(
                &mut store,
                node.clone(),
                now_ms,
                SeedUpdateSource::Propagated,
            );
            for event in events {
                tracing::warn!("Seed-store continuity: {}", event);
            }
            if was_accepted {
                runtime
                    .pending_direct_validation
                    .insert(node.addr.ip(), id_chain.to_string());
            }
            crate::control::push_packet_meta_trace(
                "InternalAction",
                if was_accepted { 32 } else { 0 },
                &format!(
                    "DHT updated: NodeAnnounce {} accepted={}",
                    node.addr, was_accepted
                ),
                id_chain,
                "internal",
            );
        }
        _ => {}
    }
}

pub fn handle_gossip_event(
    swarm: &mut Swarm<RouterBehaviour>,
    runtime: &mut GossipRuntime,
    event: request_response::Event<GossipRequest, GossipResponse>,
) {
    if let request_response::Event::Message { peer, message } = event {
        #[cfg(feature = "dht-validation-policy")]
        let source_validation_state = runtime.peer_ip_reverse.get(&peer).and_then(|ip| {
            runtime
                .seed_store
                .read()
                .unwrap()
                .values()
                .find(|node| node.addr.ip() == *ip)
                .map(|node| node.validation_state.clone())
        });
        match message {
            request_response::Message::Request {
                request, channel, ..
            } => {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                #[cfg(feature = "dht-validation-policy")]
                match &request {
                    GossipRequest::DirectNodeAnnounce { node } => {
                        let mut store = runtime.seed_store.write().unwrap();
                        let (accepted, events) = upsert_seed_store_node(
                            &mut store,
                            node.clone(),
                            now_ms,
                            SeedUpdateSource::Direct,
                        );
                        for event in events {
                            tracing::warn!("Seed-store continuity: {}", event);
                        }
                        if accepted {
                            runtime
                                .pending_bootstrap_validation
                                .insert(node.addr, String::new());
                        }
                        crate::control::push_packet_meta_trace(
                            "InternalAction",
                            if accepted { 32 } else { 0 },
                            &format!(
                                "DHT updated: DirectNodeAnnounce {} accepted={}",
                                node.addr, accepted
                            ),
                            "",
                            "internal",
                        );
                    }
                    GossipRequest::DirectNodePropagate {
                        snapshot_ts_ms: _,
                        nodes,
                    } => {
                        let source_allowed = matches!(
                            source_validation_state,
                            Some(ValidationState::Complete) | Some(ValidationState::Direct)
                        );
                        if !source_allowed {
                            crate::control::push_packet_meta_trace(
                                "InternalAction",
                                0,
                                "DHT ignored: DirectNodePropagate from non-direct source",
                                "",
                                "internal",
                            );
                            let _ = swarm
                                .behaviour_mut()
                                .gossip
                                .send_response(channel, GossipResponse::Ack);
                            return;
                        }
                        let mut store = runtime.seed_store.write().unwrap();
                        let mut accepted = 0usize;
                        for node in nodes {
                            let (ok, events) = upsert_seed_store_node(
                                &mut store,
                                node.clone(),
                                now_ms,
                                SeedUpdateSource::Propagated,
                            );
                            if ok {
                                accepted += 1;
                                runtime
                                    .pending_direct_validation
                                    .insert(node.addr.ip(), String::new());
                            }
                            for event in events {
                                tracing::warn!("Seed-store continuity: {}", event);
                            }
                        }
                        crate::control::push_packet_meta_trace(
                            "InternalAction",
                            accepted * 32,
                            &format!("DHT updated: DirectNodePropagate accepted={accepted}"),
                            "",
                            "internal",
                        );
                    }
                    GossipRequest::DirectNodeProbe => {
                        if let Some(local_node) = get_local_relay_node() {
                            swarm.behaviour_mut().gossip.send_request(
                                &peer,
                                GossipRequest::DirectNodeProbeResponse { node: local_node },
                            );
                        }
                    }
                    GossipRequest::DirectNodeProbeResponse { node } => {
                        let mut store = runtime.seed_store.write().unwrap();
                        let (accepted, events) = upsert_seed_store_node(
                            &mut store,
                            node.clone(),
                            now_ms,
                            SeedUpdateSource::Direct,
                        );
                        let parent_chain = runtime
                            .pending_direct_validation
                            .remove(&node.addr.ip())
                            .unwrap_or_default();
                        if accepted {
                            runtime
                                .pending_bootstrap_validation
                                .insert(node.addr, parent_chain);
                        }
                        for event in events {
                            tracing::warn!("Seed-store continuity: {}", event);
                        }
                        crate::control::push_packet_meta_trace(
                            "InternalAction",
                            if accepted { 32 } else { 0 },
                            &format!(
                                "DHT updated: DirectNodeProbeResponse {} accepted={}",
                                node.addr, accepted
                            ),
                            "",
                            "internal",
                        );
                    }
                    _ => {}
                }
                #[cfg(not(feature = "dht-validation-policy"))]
                let _ = now_ms;

                // Peek at GossipData before engine takes ownership
                let mut id_chain = String::new();
                if let GossipRequest::GossipData {
                    payload,
                    message_id,
                    ..
                } = &request
                {
                    #[cfg(feature = "distributed-trace")]
                    if let GossipRequest::GossipData { trace, .. } = &request {
                        id_chain = crate::trace::chain_to_string(&trace.chain);
                    }
                    crate::control::push_packet_meta_trace(
                        "GossipRecv",
                        payload.len(),
                        &format!("GossipData received message_id={}", hex::encode(message_id)),
                        &id_chain,
                        "incoming",
                    );
                    if let Ok(gbn_msg) = serde_json::from_slice::<GbnGossipMsg>(payload) {
                        apply_seed_update_from_gossip_msg(runtime, gbn_msg, &id_chain);
                    }
                }

                let outbound = runtime.engine.on_request(peer, request);
                send_outbound(swarm, outbound);
                let _ = swarm
                    .behaviour_mut()
                    .gossip
                    .send_response(channel, GossipResponse::Ack);
            }
            request_response::Message::Response { .. } => {}
        }
    }
}

fn short_key_fingerprint(key: &[u8; 32]) -> String {
    let hex_key = hex::encode(key);
    hex_key.chars().take(12).collect()
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum SeedUpdateSource {
    Direct,
    Propagated,
}

// Keep the seed-store coherent across relay restarts and IP churn:
// - one identity key should map to one latest address,
// - one address should map to the latest announced key.
// Returns (accepted, continuity_events).
fn upsert_seed_store_node(
    store: &mut HashMap<SocketAddr, RelayNode>,
    mut incoming: RelayNode,
    now_ms: u64,
    source: SeedUpdateSource,
) -> (bool, Vec<String>) {
    let mut events = Vec::new();
    #[cfg(feature = "dht-validation-policy")]
    {
        let existing_score = store
            .get(&incoming.addr)
            .map(|node| node.validation_score)
            .unwrap_or(0);
        incoming.last_observed_ms = now_ms;
        incoming.last_seen_ms = now_ms;
        if incoming.announce_ts_ms == 0 {
            incoming.announce_ts_ms = now_ms;
        }
        match source {
            SeedUpdateSource::Direct => {
                incoming.last_direct_seen_ms = Some(now_ms);
                incoming.validation_score = existing_score.max(10);
                incoming.validation_state = crate::circuit_manager::ValidationState::Unvalidated;
            }
            SeedUpdateSource::Propagated => {
                incoming.last_propagated_seen_ms = Some(now_ms);
                incoming.validation_score = existing_score;
                incoming.validation_state = crate::circuit_manager::ValidationState::PropagatedOnly;
            }
        }
    }
    #[cfg(not(feature = "dht-validation-policy"))]
    {
        let _ = source;
        incoming.last_seen_ms = now_ms;
    }

    if incoming.identity_pub.iter().all(|b| *b == 0) {
        events.push(format!(
            "reject_zero_identity addr={} subnet={}",
            incoming.addr, incoming.subnet_tag
        ));
        return (false, events);
    }

    if let Some(existing) = store.get(&incoming.addr) {
        #[cfg(feature = "dht-validation-policy")]
        if existing.announce_ts_ms > incoming.announce_ts_ms {
            events.push(format!(
                "reject_stale_announce addr={} incoming_ts={} existing_ts={}",
                incoming.addr, incoming.announce_ts_ms, existing.announce_ts_ms
            ));
            return (false, events);
        }
        if existing.identity_pub != incoming.identity_pub {
            events.push(format!(
                "addr_rekey addr={} old_fp={} new_fp={}",
                incoming.addr,
                short_key_fingerprint(&existing.identity_pub),
                short_key_fingerprint(&incoming.identity_pub),
            ));
        }
    }

    let previous_addr_for_identity = store.iter().find_map(|(addr, node)| {
        if *addr != incoming.addr && node.identity_pub == incoming.identity_pub {
            Some(*addr)
        } else {
            None
        }
    });
    if let Some(old_addr) = previous_addr_for_identity {
        store.remove(&old_addr);
        events.push(format!(
            "identity_move fp={} old_addr={} new_addr={}",
            short_key_fingerprint(&incoming.identity_pub),
            old_addr,
            incoming.addr
        ));
    }

    #[cfg(feature = "dht-validation-policy")]
    if let Some(existing) = store.get(&incoming.addr) {
        let merged_direct = existing
            .last_direct_seen_ms
            .max(incoming.last_direct_seen_ms);
        let merged_prop = existing
            .last_propagated_seen_ms
            .max(incoming.last_propagated_seen_ms);
        incoming.last_direct_seen_ms = merged_direct;
        incoming.last_propagated_seen_ms = merged_prop;
        incoming.last_observed_ms = existing.last_observed_ms.max(incoming.last_observed_ms);
        incoming.last_seen_ms = incoming.last_observed_ms;
        incoming.validation_score = existing.validation_score.max(incoming.validation_score);
        incoming.validation_state = if existing.validation_state
            == crate::circuit_manager::ValidationState::Isolated
        {
            crate::circuit_manager::ValidationState::Isolated
        } else if existing.validation_state == crate::circuit_manager::ValidationState::Complete {
            crate::circuit_manager::ValidationState::Complete
        } else if existing.validation_state == crate::circuit_manager::ValidationState::Direct {
            crate::circuit_manager::ValidationState::Direct
        } else if incoming.last_direct_seen_ms.is_some() {
            crate::circuit_manager::ValidationState::Unvalidated
        } else {
            match source {
                SeedUpdateSource::Propagated => {
                    crate::circuit_manager::ValidationState::PropagatedOnly
                }
                SeedUpdateSource::Direct => crate::circuit_manager::ValidationState::Unvalidated,
            }
        };
    }

    store.insert(incoming.addr, incoming);
    (true, events)
}

pub async fn drive_swarm_once(
    swarm: &mut Swarm<RouterBehaviour>,
    runtime: &mut GossipRuntime,
) -> Result<()> {
    // Poll swarm with a 200ms timeout so periodic tasks (gossip publish, re-bootstrap,
    // creator inject) always run on schedule even when no swarm events are arriving.
    // Without this, swarm.next().await blocks indefinitely on an idle network and all
    // the timers below never fire — causing GossipBandwidthBytes / ChunksDelivered to
    // be zero forever.
    // We extract the event BEFORE the match so swarm is free to re-borrow inside handle_gossip_event.
    let swarm_event = tokio::select! {
        event = swarm.next() => event,
        _ = tokio::time::sleep(Duration::from_millis(200)) => None,
    };

    if let Some(event) = swarm_event {
        match event {
            SwarmEvent::ConnectionEstablished {
                peer_id, endpoint, ..
            } => {
                runtime.engine.add_lazy_peer(peer_id);
                let remote_addr = match &endpoint {
                    ConnectedPoint::Dialer { address, .. } => address,
                    ConnectedPoint::Listener { send_back_addr, .. } => send_back_addr,
                };
                if let Some(ip) = ip_from_multiaddr(remote_addr) {
                    runtime.peer_ip_map.insert(ip, peer_id);
                    #[cfg(feature = "dht-validation-policy")]
                    runtime.peer_ip_reverse.insert(peer_id, ip);
                }
                #[cfg(feature = "dht-validation-policy")]
                if let Some(local_node) = get_local_relay_node() {
                    swarm.behaviour_mut().gossip.send_request(
                        &peer_id,
                        GossipRequest::DirectNodeAnnounce {
                            node: local_node.clone(),
                        },
                    );
                    if let Some(ip) = runtime.peer_ip_reverse.get(&peer_id).copied() {
                        if runtime.pending_direct_validation.contains_key(&ip) {
                            swarm
                                .behaviour_mut()
                                .gossip
                                .send_request(&peer_id, GossipRequest::DirectNodeProbe);
                        }
                    }
                }
            }
            SwarmEvent::Behaviour(RouterBehaviourEvent::Gossip(event)) => {
                handle_gossip_event(swarm, runtime, event);
            }
            _ => {}
        }
    }

    if runtime.last_gossip_publish.elapsed() >= Duration::from_secs(10) {
        let total = runtime.engine.state.bytes_sent_total();
        let delta = total.saturating_sub(runtime.last_gossip_bytes_published);
        // Prefer the cached reporter; fall back to spawning a fresh one so that a None
        // runtime.metrics (e.g. from a transient init-time credential error) never
        // silently drops metrics for the entire lifetime of the process.
        if let Some(metrics) = &runtime.metrics {
            let _ = metrics.publish_gossip_bandwidth_bytes(delta).await;
        } else {
            tokio::spawn(async move {
                if let Ok(reporter) = MetricsReporter::from_env().await {
                    let _ = reporter.publish_gossip_bandwidth_bytes(delta).await;
                }
            });
        }
        runtime.last_gossip_bytes_published = total;
        runtime.last_gossip_publish = Instant::now();
    }

    // Periodically expire stale IWant requests to prevent unbounded growth.
    if runtime.last_gossip_expiry.elapsed() >= Duration::from_secs(60) {
        runtime
            .engine
            .state
            .expire_missing_older_than(Duration::from_secs(300));
        runtime.last_gossip_expiry = Instant::now();
    }

    // Periodic re-bootstrap: if we have no peers (e.g. we lost all connections or bootstrap
    // fired before anyone registered), re-discover from Cloud Map every rebootstrap_interval.
    let total_known_peers =
        runtime.engine.state.eager_peers.len() + runtime.engine.state.lazy_peers.len();
    if total_known_peers == 0 && runtime.last_rebootstrap.elapsed() >= runtime.rebootstrap_interval
    {
        let added = bootstrap_from_static_seeds(swarm).await.unwrap_or(0);
        tracing::info!(
            "Re-bootstrap attempt: discovered {} new peers via Static Seeds",
            added
        );
        runtime.last_rebootstrap = Instant::now();
    }

    // Publish local node presence to the Gossip mesh for service roles that
    // should appear in runtime discovery (relay/creator/publisher).
    if role_participates_in_node_announce(&runtime.role) {
        #[cfg(feature = "dht-validation-policy")]
        let announce_due = runtime.last_announce.elapsed() >= runtime.announce_interval;
        #[cfg(not(feature = "dht-validation-policy"))]
        let announce_due = runtime.last_announce.elapsed() >= Duration::from_secs(10);
        if announce_due {
            runtime.last_announce = Instant::now();
            if let Some(local_node) = get_local_relay_node() {
                let msg = GbnGossipMsg::NodeAnnounce(local_node);
                if let Ok(payload) = serde_json::to_vec(&msg) {
                    let mut msg_id = [0u8; 32];
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64;
                    msg_id[0..8].copy_from_slice(&ts.to_le_bytes());
                    // 0x414E_4E4F is "ANNO"
                    msg_id[16..24].copy_from_slice(&0x414E_4E4F_u64.to_le_bytes());
                    let outbound = runtime.engine.publish_local(msg_id, payload);
                    send_outbound(swarm, outbound);
                }
            }
        }
    }

    #[cfg(feature = "dht-validation-policy")]
    if runtime.last_propagate.elapsed() >= runtime.propagate_interval {
        runtime.last_propagate = Instant::now();
        let peers: Vec<PeerId> = runtime
            .engine
            .state
            .eager_peers
            .iter()
            .copied()
            .chain(runtime.engine.state.lazy_peers.iter().copied())
            .collect();
        if !peers.is_empty() {
            let mut peer_targets = peers;
            peer_targets.shuffle(&mut rand::thread_rng());
            let mut peer_count = (peer_targets.len() * runtime.propagate_peer_percent) / 100;
            peer_count = peer_count.clamp(1, peer_targets.len());
            peer_targets.truncate(peer_count);
            let snapshot_ts_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            for peer in peer_targets {
                let (target_addr, target_state) = runtime
                    .peer_ip_reverse
                    .get(&peer)
                    .and_then(|ip| {
                        runtime
                            .seed_store
                            .read()
                            .unwrap()
                            .values()
                            .find(|node| node.addr.ip() == *ip)
                            .map(|node| (Some(node.addr), Some(node.validation_state.clone())))
                    })
                    .unwrap_or((None, None));
                let nodes = minimal_bootstrap_snapshot(
                    &runtime.seed_store,
                    target_addr,
                    target_state.as_ref(),
                    runtime.propagate_max_entries,
                );
                if !nodes.is_empty() {
                    swarm.behaviour_mut().gossip.send_request(
                        &peer,
                        GossipRequest::DirectNodePropagate {
                            snapshot_ts_ms,
                            nodes,
                        },
                    );
                }
            }
        }
    }

    #[cfg(feature = "dht-validation-policy")]
    if !runtime.pending_direct_validation.is_empty() {
        let pending_probes: Vec<IpAddr> =
            runtime.pending_direct_validation.keys().copied().collect();
        for ip in pending_probes {
            if let Some(peer) = runtime.peer_ip_map.get(&ip).copied() {
                swarm
                    .behaviour_mut()
                    .gossip
                    .send_request(&peer, GossipRequest::DirectNodeProbe);
            } else {
                let p2p_port = env::var("GBN_P2P_PORT")
                    .ok()
                    .and_then(|p| p.parse::<u16>().ok())
                    .unwrap_or(4001);
                let mut addr = libp2p::Multiaddr::empty();
                addr.push(Protocol::from(ip));
                addr.push(Protocol::Tcp(p2p_port));
                let _ = swarm.dial(addr);
            }
        }
    }

    #[cfg(feature = "dht-validation-policy")]
    if !runtime.pending_bootstrap_validation.is_empty() {
        let pending_targets: Vec<(SocketAddr, String)> = runtime
            .pending_bootstrap_validation
            .iter()
            .map(|(guard_addr, parent_chain)| (*guard_addr, parent_chain.clone()))
            .collect();
        for (guard_addr, parent_chain) in pending_targets {
            match bootstrap_validate_unvalidated_node(
                &runtime.seed_store,
                runtime.noise_priv_key,
                guard_addr,
                &parent_chain,
            )
            .await
            {
                Ok(promoted) => {
                    if promoted {
                        runtime.pending_bootstrap_validation.remove(&guard_addr);
                    }
                }
                Err(error) => {
                    tracing::debug!(
                        "Bootstrap validation attempt failed for {}: {error:#}",
                        guard_addr
                    );
                }
            }
        }
    }

    // Creator role: periodically inject a test gossip message to exercise the PlumTree network.
    // Without at least one publish_local() call, GossipBandwidthBytes stays zero forever.
    // last_creator_publish is None on first start (fire immediately once peers connect),
    // then Some(last_time) (fire again after creator_publish_interval elapses).
    let creator_due = runtime.role == "creator"
        && match runtime.last_creator_publish {
            None => true,
            Some(t) => t.elapsed() >= runtime.creator_publish_interval,
        };
    if creator_due {
        let total_peers =
            runtime.engine.state.eager_peers.len() + runtime.engine.state.lazy_peers.len();
        if total_peers > 0 {
            let mut msg_id: MessageId = [0u8; 32];
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            msg_id[0..8].copy_from_slice(&ts.to_le_bytes());
            msg_id[8..16].copy_from_slice(&runtime.creator_seq.to_le_bytes());

            let payload = format!("gbn-test-chunk-seq-{}", runtime.creator_seq).into_bytes();
            runtime.creator_seq += 1;

            let outbound = runtime.engine.publish_local(msg_id, payload);
            let n_targets = outbound.len();
            send_outbound(swarm, outbound);

            tracing::info!(
                seq = runtime.creator_seq,
                peers = total_peers,
                targets = n_targets,
                "Creator: injected gossip message"
            );

            if let Some(metrics) = &runtime.metrics {
                let _ = metrics.publish_chunks_delivered(1).await;
            } else {
                tokio::spawn(async move {
                    if let Ok(reporter) = MetricsReporter::from_env().await {
                        let _ = reporter.publish_chunks_delivered(1).await;
                    }
                });
            }
            // Only reset the timer after a successful publish so we retry quickly if no peers yet
            runtime.last_creator_publish = Some(Instant::now());
        } else {
            tracing::debug!("Creator: no peers yet, deferring gossip publish");
        }
    }

    Ok(())
}

pub async fn bootstrap_from_static_seeds(swarm: &mut Swarm<RouterBehaviour>) -> Result<usize> {
    // Check for Docker DNS fallback mode
    if let Ok(mode) = env::var("GBN_DISCOVERY_MODE") {
        if mode == "docker-dns" {
            return bootstrap_from_docker_dns(swarm).await;
        }
    }

    let seed_ips_str = match env::var("GBN_SEED_IPS") {
        Ok(v) if !v.is_empty() => v,
        _ => return Ok(0),
    };

    let p2p_port = env::var("GBN_P2P_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(4001);

    let mut added = 0usize;
    for ip_str in seed_ips_str.split(',') {
        let ip_str = ip_str.trim();
        if ip_str.is_empty() {
            continue;
        }

        let ip_addr: IpAddr = match ip_str.parse() {
            Ok(a) => a,
            // fallback, if it includes port
            Err(_) => {
                if let Ok(socket_addr) = ip_str.parse::<std::net::SocketAddr>() {
                    socket_addr.ip()
                } else {
                    continue;
                }
            }
        };

        let mut addr = libp2p::Multiaddr::empty();
        addr.push(Protocol::from(ip_addr));
        addr.push(Protocol::Tcp(p2p_port));

        if swarm.dial(addr.clone()).is_ok() {
            added += 1;
            tracing::info!("Dialed static seed node: {}", addr);
        } else {
            tracing::warn!("Failed dialing static seed node: {}", addr);
        }
    }

    Ok(added)
}

/// Docker DNS fallback discovery for local testing
async fn bootstrap_from_docker_dns(swarm: &mut Swarm<RouterBehaviour>) -> Result<usize> {
    use std::net::ToSocketAddrs;

    let p2p_port = env::var("GBN_P2P_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(4001);

    // Docker Compose service names that run relay nodes
    let service_names = ["relay-hostile", "relay-free", "creator", "publisher"];
    let mut added = 0usize;

    for name in service_names.iter() {
        // Resolve service name to IP addresses via Docker's internal DNS
        let host_port = format!("{}:{}", name, p2p_port);
        if let Ok(addrs) = host_port.to_socket_addrs() {
            for addr in addrs {
                // Docker resolves to container IPs (e.g., 172.30.x.x)
                let ip_addr = addr.ip();
                let mut multiaddr = libp2p::Multiaddr::empty();
                multiaddr.push(libp2p::multiaddr::Protocol::from(ip_addr));
                multiaddr.push(libp2p::multiaddr::Protocol::Tcp(p2p_port));

                // In Docker DNS mode we don't have peer IDs, so we just dial
                // The swarm will exchange peer IDs upon connection
                let _ = swarm.dial(multiaddr.clone());
                added += 1;

                tracing::debug!("Docker DNS discovered {} -> {}", name, multiaddr);
            }
        }
    }

    tracing::info!("Docker DNS bootstrap discovered {} addresses", added);
    Ok(added)
}

// ────────────────────────── Phase 2: Onion Discovery Helpers ─────────────────────────

/// Constructs a RelayNode for this process by reading GBN_INSTANCE_IPV4,
/// GBN_ONION_PORT, GBN_SUBNET_TAG, and deriving the Noise pubkey.
pub fn get_local_relay_node() -> Option<RelayNode> {
    let ipv4 = env::var("GBN_INSTANCE_IPV4").ok()?;
    let onion_port: u16 = env::var("GBN_ONION_PORT")
        .unwrap_or_else(|_| "9001".to_string())
        .parse()
        .ok()?;
    let ip_addr: IpAddr = ipv4.parse().ok()?;

    let noise_pub_hex = if let Ok(pub_hex) = env::var("GBN_NOISE_PUBKEY_HEX") {
        pub_hex
    } else if let Ok(priv_hex) = env::var("GBN_NOISE_PRIVKEY_HEX") {
        match hex::decode(&priv_hex) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                hex::encode(x25519_pubkey_from_privkey(&arr))
            }
            _ => return None,
        }
    } else {
        return None;
    };

    let noise_bytes = hex::decode(&noise_pub_hex).ok()?;
    if noise_bytes.len() != 32 {
        return None;
    }
    let mut identity_pub = [0u8; 32];
    identity_pub.copy_from_slice(&noise_bytes);

    let subnet_tag = env::var("GBN_SUBNET_TAG").unwrap_or_else(|_| "Unknown".to_string());

    Some(RelayNode {
        addr: SocketAddr::new(ip_addr, onion_port),
        identity_pub,
        subnet_tag,
        announce_ts_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        last_direct_seen_ms: None,
        last_propagated_seen_ms: None,
        last_observed_ms: 0,
        validation_state: crate::circuit_manager::ValidationState::Direct,
        validation_score: 100,
        last_seen_ms: 0,
    })
}

/// Statically resolve the Publisher's mpub-receiver address and X25519 public key.
/// Fetches `GBN_PUBLISHER_IP` and `GBN_PUBLISHER_PUBKEY_HEX` injected into environment.
pub async fn discover_publisher_static() -> Result<(SocketAddr, [u8; 32])> {
    let fallback_port = env::var("GBN_MPUB_PORT").unwrap_or_else(|_| "7001".to_string());

    let ip_str = env::var("GBN_PUBLISHER_IP").context("GBN_PUBLISHER_IP not set")?;
    let pub_key_hex =
        env::var("GBN_PUBLISHER_PUBKEY_HEX").context("GBN_PUBLISHER_PUBKEY_HEX not set")?;

    let addr = if let Ok(socket_addr) = ip_str.parse::<SocketAddr>() {
        socket_addr
    } else {
        let ip_addr: std::net::IpAddr = ip_str.parse()?;
        let port: u16 = fallback_port.parse().unwrap_or(7001);
        SocketAddr::new(ip_addr, port)
    };

    let key_bytes = hex::decode(&pub_key_hex)?;
    anyhow::ensure!(key_bytes.len() == 32, "Publisher pubkey must be 32 bytes");
    let mut pubkey = [0u8; 32];
    pubkey.copy_from_slice(&key_bytes);

    tracing::info!("Found Publisher static env at {}", addr);
    Ok((addr, pubkey))
}

/// Cached Publisher SocketAddr for exit relays.
/// Re-queries Cloud Map at most once every 60 seconds to handle Publisher restarts.
static PUBLISHER_ADDR_CACHE: tokio::sync::OnceCell<
    tokio::sync::Mutex<(Option<SocketAddr>, std::time::Instant)>,
> = tokio::sync::OnceCell::const_new();

/// Resolve the Publisher's mpub-receiver address for exit relay forwarding.
///
/// Reads `GBN_PUBLISHER_ADDR` env var first (fast path for static environments).
/// Falls back to Cloud Map discovery with a 60-second TTL cache.
pub async fn discover_publisher_addr_for_exit_relay() -> Result<SocketAddr> {
    // Fast path: static env var (for local testing or when address is known)
    if let Ok(addr_str) = env::var("GBN_PUBLISHER_ADDR") {
        if !addr_str.is_empty() {
            return Ok(addr_str.parse()?);
        }
    }

    // Initialize cache
    let cache = PUBLISHER_ADDR_CACHE
        .get_or_init(|| async {
            tokio::sync::Mutex::new((
                None,
                std::time::Instant::now() - std::time::Duration::from_secs(120),
            ))
        })
        .await;

    let mut guard = cache.lock().await;
    let (cached_addr, last_refresh) = &mut *guard;

    // Return cached value if fresh (< 60 seconds old)
    if let Some(addr) = cached_addr {
        if last_refresh.elapsed() < std::time::Duration::from_secs(60) {
            return Ok(*addr);
        }
    }

    // Refresh from static env
    let (addr, _) = discover_publisher_static().await?;
    *cached_addr = Some(addr);
    *last_refresh = std::time::Instant::now();
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit_manager::ValidationState;

    fn test_node(addr: &str, key: u8) -> RelayNode {
        RelayNode {
            addr: addr.parse().unwrap(),
            identity_pub: [key; 32],
            subnet_tag: "HostileSubnet".to_string(),
            announce_ts_ms: 1,
            last_direct_seen_ms: None,
            last_propagated_seen_ms: None,
            last_observed_ms: 0,
            validation_state: ValidationState::PropagatedOnly,
            validation_score: 0,
            last_seen_ms: 0,
        }
    }

    #[test]
    fn relay_creator_and_publisher_participate_in_node_announce() {
        assert!(role_participates_in_node_announce("relay"));
        assert!(role_participates_in_node_announce("creator"));
        assert!(role_participates_in_node_announce("publisher"));
        assert!(!role_participates_in_node_announce("seed"));
    }

    #[test]
    fn propagated_updates_start_as_propagated_only() {
        let mut store = HashMap::new();
        let node = test_node("10.0.0.10:9001", 1);

        let (accepted, events) =
            upsert_seed_store_node(&mut store, node.clone(), 100, SeedUpdateSource::Propagated);

        assert!(accepted);
        assert!(events.is_empty());
        let saved = store.get(&node.addr).unwrap();
        assert_eq!(saved.validation_state, ValidationState::PropagatedOnly);
        assert_eq!(saved.validation_score, 0);
        assert_eq!(saved.last_propagated_seen_ms, Some(100));
        assert_eq!(saved.last_direct_seen_ms, None);
    }

    #[test]
    fn direct_updates_start_as_unvalidated_with_minimum_score() {
        let mut store = HashMap::new();
        let node = test_node("10.0.0.11:9001", 2);

        let (accepted, events) =
            upsert_seed_store_node(&mut store, node.clone(), 200, SeedUpdateSource::Direct);

        assert!(accepted);
        assert!(events.is_empty());
        let saved = store.get(&node.addr).unwrap();
        assert_eq!(saved.validation_state, ValidationState::Unvalidated);
        assert_eq!(saved.validation_score, 10);
        assert_eq!(saved.last_direct_seen_ms, Some(200));
        assert_eq!(saved.last_propagated_seen_ms, None);
    }

    #[test]
    fn propagated_updates_do_not_demote_direct_nodes() {
        let mut store = HashMap::new();
        let node = test_node("10.0.0.12:9001", 3);

        let (accepted, _) =
            upsert_seed_store_node(&mut store, node.clone(), 100, SeedUpdateSource::Direct);
        assert!(accepted);

        let (accepted, _) =
            upsert_seed_store_node(&mut store, node.clone(), 200, SeedUpdateSource::Propagated);
        assert!(accepted);

        let saved = store.get(&node.addr).unwrap();
        assert_eq!(saved.validation_state, ValidationState::Unvalidated);
        assert_eq!(saved.validation_score, 10);
        assert_eq!(saved.last_direct_seen_ms, Some(100));
        assert_eq!(saved.last_propagated_seen_ms, Some(200));
    }

    #[test]
    fn direct_updates_of_existing_direct_nodes_preserve_direct_state() {
        let mut store = HashMap::new();
        let node = RelayNode {
            validation_state: ValidationState::Direct,
            validation_score: 42,
            ..test_node("10.0.0.13:9001", 4)
        };
        store.insert(node.addr, node.clone());

        let (accepted, events) =
            upsert_seed_store_node(&mut store, node.clone(), 300, SeedUpdateSource::Direct);

        assert!(accepted);
        assert!(events.is_empty());
        let saved = store.get(&node.addr).unwrap();
        assert_eq!(saved.validation_state, ValidationState::Direct);
        assert_eq!(saved.validation_score, 42);
        assert_eq!(saved.last_direct_seen_ms, Some(300));
    }

    #[test]
    fn identity_move_replaces_old_address_for_same_key() {
        let mut store = HashMap::new();
        let old_node = test_node("10.0.0.14:9001", 5);
        let new_node = test_node("10.0.0.15:9001", 5);
        store.insert(old_node.addr, old_node.clone());

        let (accepted, events) =
            upsert_seed_store_node(&mut store, new_node.clone(), 400, SeedUpdateSource::Direct);

        assert!(accepted);
        assert!(store.contains_key(&new_node.addr));
        assert!(!store.contains_key(&old_node.addr));
        assert!(events.iter().any(|event| event.contains("identity_move")));
    }
}
