use crate::circuit_manager::RelayNode;
use crate::gossip::{
    new_plumtree_behaviour, GossipRequest, GossipResponse, MessageId, OutboundGossip,
    PlumTreeBehaviour, PlumTreeEngine,
};
use crate::observability::MetricsReporter;
use anyhow::Result;
use aws_sdk_servicediscovery::types::HealthStatusFilter;
use libp2p::futures::StreamExt;
use libp2p::{
    identity,
    kad::{store::MemoryStore, Behaviour as Kademlia, Config as KademliaConfig},
    multiaddr::Protocol,
    noise, request_response,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Swarm, SwarmBuilder,
};
use mcn_crypto::x25519_pubkey_from_privkey;
use rand::Rng;
use std::time::Duration;
use std::{
    collections::HashMap,
    env,
    net::{IpAddr, SocketAddr},
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

impl GossipRuntime {
    pub async fn from_env() -> Self {
        let (gossip_bps, max_tracked_messages) = gossip_config_from_env();
        let metrics = MetricsReporter::from_env().await.ok();
        let role = env::var("GBN_ROLE").unwrap_or_else(|_| "relay".to_string());
        let creator_publish_interval = Duration::from_secs(
            env::var("GBN_CREATOR_PUBLISH_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(30),
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

    // Stagger startup with a random jitter FIRST to spread both Cloud Map
    // RegisterInstance API calls AND peer-discovery bootstrapping across a
    // 0–GBN_BOOTSTRAP_JITTER_SECS window.  Without jitter, all 100 nodes call
    // RegisterInstance simultaneously, exceeding the 100-TPS Cloud Map limit
    // and causing silent throttling failures for most nodes.
    let jitter_max = env::var("GBN_BOOTSTRAP_JITTER_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(20);
    if jitter_max > 0 {
        let jitter = rand::thread_rng().gen_range(0..=jitter_max);
        tracing::info!(
            "Bootstrap jitter: sleeping {}s before registration and peer discovery",
            jitter
        );
        tokio::time::sleep(Duration::from_secs(jitter)).await;
    }

    // Register AFTER jitter so API calls are spread over the jitter window
    // rather than all hitting Cloud Map at T+0.
    if let Err(e) = register_with_cloudmap(&swarm).await {
        tracing::warn!("Cloud Map registration failed (will run without relay discovery): {e}");
    }

    let added = bootstrap_from_cloudmap(&mut swarm).await?;
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
) -> Result<()> {
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                let _ = deregister_from_cloudmap().await;
                break;
            }
            res = drive_swarm_once(swarm, runtime) => {
                res?;
            }
        }
    }
    Ok(())
}

fn send_outbound(
    swarm: &mut Swarm<RouterBehaviour>,
    outbound: impl IntoIterator<Item = OutboundGossip>,
) {
    for msg in outbound {
        swarm
            .behaviour_mut()
            .gossip
            .send_request(&msg.peer, msg.request);
    }
}

pub fn handle_gossip_event(
    swarm: &mut Swarm<RouterBehaviour>,
    runtime: &mut GossipRuntime,
    event: request_response::Event<GossipRequest, GossipResponse>,
) {
    if let request_response::Event::Message { peer, message } = event {
        match message {
            request_response::Message::Request {
                request, channel, ..
            } => {
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
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                runtime.engine.add_lazy_peer(peer_id);
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
        let added = bootstrap_from_cloudmap(swarm).await.unwrap_or(0);
        tracing::info!(
            "Re-bootstrap attempt: discovered {} new peers via Cloud Map",
            added
        );
        runtime.last_rebootstrap = Instant::now();
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

pub async fn bootstrap_from_cloudmap(swarm: &mut Swarm<RouterBehaviour>) -> Result<usize> {
    // Check for Docker DNS fallback mode
    if let Ok(mode) = env::var("GBN_DISCOVERY_MODE") {
        if mode == "docker-dns" {
            return bootstrap_from_docker_dns(swarm).await;
        }
    }

    let namespace = match env::var("GBN_CLOUDMAP_NAMESPACE") {
        Ok(v) if !v.is_empty() => v,
        _ => return Ok(0),
    };
    let service_name = env::var("GBN_CLOUDMAP_SERVICE_NAME")
        .or_else(|_| env::var("GBN_CLOUDMAP_SERVICE"))
        .unwrap_or_else(|_| "relay".to_string());
    let p2p_port = env::var("GBN_P2P_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(4001);

    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let client = aws_sdk_servicediscovery::Client::new(&config);

    let instances = client
        .discover_instances()
        .namespace_name(&namespace)
        .service_name(&service_name)
        // Use All rather than Healthy: instances registered via RegisterInstance (without
        // a Cloud Map health check config) have UNKNOWN health status. The Healthy filter
        // returns zero results for UNKNOWN-status instances, breaking peer discovery.
        .health_status(HealthStatusFilter::All)
        .send()
        .await?;

    let mut added = 0usize;
    for instance in instances.instances() {
        let Some(attrs) = instance.attributes() else {
            continue;
        };
        let ip: Option<String> = attrs.get("AWS_INSTANCE_IPV4").cloned();
        let peer_id_str: Option<String> = attrs.get("GBN_PEER_ID").cloned();

        let Some(ip) = ip else { continue };
        let Ok(ip_addr) = ip.parse::<IpAddr>() else {
            continue;
        };

        let mut addr = libp2p::Multiaddr::empty();
        addr.push(Protocol::from(ip_addr));
        addr.push(Protocol::Tcp(p2p_port));

        if let Some(peer_id_str) = peer_id_str {
            if let Ok(peer_id) = peer_id_str.parse::<libp2p::PeerId>() {
                swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(&peer_id, addr.clone());
            }
        }

        if swarm.dial(addr).is_ok() {
            added += 1;
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

pub async fn register_with_cloudmap(swarm: &Swarm<RouterBehaviour>) -> Result<()> {
    let service_id = match env::var("GBN_CLOUDMAP_SERVICE_ID") {
        Ok(v) if !v.is_empty() => v,
        _ => return Ok(()),
    };

    let instance_id = env::var("GBN_CLOUDMAP_INSTANCE_ID")
        .or_else(|_| env::var("HOSTNAME"))
        .unwrap_or_else(|_| swarm.local_peer_id().to_string());

    let ipv4 = match env::var("GBN_INSTANCE_IPV4") {
        Ok(v) if !v.is_empty() => v,
        _ => return Ok(()),
    };

    let mut attrs = HashMap::new();
    attrs.insert("AWS_INSTANCE_IPV4".to_string(), ipv4);
    attrs.insert("GBN_PEER_ID".to_string(), swarm.local_peer_id().to_string());

    if let Ok(port) = env::var("GBN_P2P_PORT") {
        attrs.insert("AWS_INSTANCE_PORT".to_string(), port);
    }

    // Phase 2: onion relay attributes
    let onion_port = env::var("GBN_ONION_PORT").unwrap_or_else(|_| "9001".to_string());
    attrs.insert("GBN_ONION_PORT".to_string(), onion_port);

    // Derive and register Noise_XX public key.
    // Prefer a pre-set GBN_NOISE_PUBKEY_HEX; fall back to deriving from the
    // private key generated by entrypoint.sh (GBN_NOISE_PRIVKEY_HEX).
    let noise_pub_hex = if let Ok(pub_hex) = env::var("GBN_NOISE_PUBKEY_HEX") {
        Some(pub_hex)
    } else if let Ok(priv_hex) = env::var("GBN_NOISE_PRIVKEY_HEX") {
        match hex::decode(&priv_hex) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                Some(hex::encode(x25519_pubkey_from_privkey(&arr)))
            }
            _ => {
                tracing::warn!("GBN_NOISE_PRIVKEY_HEX is not valid 32-byte hex; skipping noise pubkey registration");
                None
            }
        }
    } else {
        tracing::warn!("Neither GBN_NOISE_PUBKEY_HEX nor GBN_NOISE_PRIVKEY_HEX set; relay will not be discoverable for circuit building");
        None
    };
    if let Some(pub_hex) = noise_pub_hex {
        attrs.insert("GBN_NOISE_PUBKEY_HEX".to_string(), pub_hex);
    }

    let role = env::var("GBN_ROLE").unwrap_or_else(|_| "relay".to_string());
    let subnet_tag = env::var("GBN_SUBNET_TAG").unwrap_or_else(|_| "Unknown".to_string());
    attrs.insert("role".to_string(), role);
    attrs.insert("GBN_SUBNET_TAG".to_string(), subnet_tag);

    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let client = aws_sdk_servicediscovery::Client::new(&config);

    client
        .register_instance()
        .service_id(service_id)
        .instance_id(instance_id)
        .set_attributes(Some(attrs))
        .send()
        .await?;

    Ok(())
}

pub async fn deregister_from_cloudmap() -> Result<()> {
    let service_id = match env::var("GBN_CLOUDMAP_SERVICE_ID") {
        Ok(v) if !v.is_empty() => v,
        _ => return Ok(()),
    };

    let instance_id = env::var("GBN_CLOUDMAP_INSTANCE_ID")
        .or_else(|_| env::var("HOSTNAME"))
        .unwrap_or_default();
    if instance_id.is_empty() {
        return Ok(());
    }

    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let client = aws_sdk_servicediscovery::Client::new(&config);

    client
        .deregister_instance()
        .service_id(service_id)
        .instance_id(instance_id)
        .send()
        .await?;

    Ok(())
}

// ────────────────────────── Phase 2: Onion Discovery Helpers ─────────────────────────

/// Discover all live relay nodes from Cloud Map, returning onion-capable `RelayNode` structs.
///
/// Each `RelayNode` contains:
/// - `addr`: `<IP>:<GBN_ONION_PORT>` (the onion relay TCP listener, port 9001 by default)
/// - `identity_pub`: 32-byte Noise_XX Curve25519 public key
/// - `subnet_tag`: `"HostileSubnet"` or `"FreeSubnet"`
///
/// Skips instances that are missing any required attribute.
pub async fn discover_relay_nodes_from_cloudmap() -> Result<Vec<RelayNode>> {
    let namespace = match env::var("GBN_CLOUDMAP_NAMESPACE") {
        Ok(v) if !v.is_empty() => v,
        _ => return Ok(Vec::new()),
    };
    let service_name = env::var("GBN_CLOUDMAP_SERVICE_NAME")
        .or_else(|_| env::var("GBN_CLOUDMAP_SERVICE"))
        .unwrap_or_else(|_| "relay".to_string());

    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let client = aws_sdk_servicediscovery::Client::new(&config);

    let instances = client
        .discover_instances()
        .namespace_name(&namespace)
        .service_name(&service_name)
        .health_status(HealthStatusFilter::All)
        .send()
        .await?;

    let mut nodes = Vec::new();
    for instance in instances.instances() {
        let Some(attrs) = instance.attributes() else {
            continue;
        };

        let ip = match attrs.get("AWS_INSTANCE_IPV4") {
            Some(v) => v.clone(),
            None => continue,
        };
        let onion_port_str = match attrs.get("GBN_ONION_PORT") {
            Some(v) => v.clone(),
            None => continue,
        };
        let noise_pub_hex = match attrs.get("GBN_NOISE_PUBKEY_HEX") {
            Some(v) => v.clone(),
            None => continue,
        };
        let subnet_tag = match attrs.get("GBN_SUBNET_TAG") {
            Some(v) => v.clone(),
            None => continue,
        };
        if attrs.contains_key("GBN_PUB_KEY_HEX") && attrs.contains_key("GBN_MPUB_PORT") {
            continue;
        }
        // We only build circuits through dedicated relay tasks. Skip everything
        // that explicitly registers as non-relay (publisher/creator/local helper).
        if let Some(role) = attrs.get("role") {
            if role != "relay" {
                continue;
            }
        }

        let onion_port: u16 = match onion_port_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let ip_addr: std::net::IpAddr = match ip.parse() {
            Ok(a) => a,
            Err(_) => continue,
        };
        let addr = SocketAddr::new(ip_addr, onion_port);

        let noise_bytes = match hex::decode(&noise_pub_hex) {
            Ok(b) if b.len() == 32 => b,
            _ => continue,
        };
        let mut identity_pub = [0u8; 32];
        identity_pub.copy_from_slice(&noise_bytes);

        nodes.push(RelayNode {
            addr,
            identity_pub,
            subnet_tag,
        });
    }

    tracing::info!("Cloud Map: discovered {} relay nodes", nodes.len());
    Ok(nodes)
}

/// Discover the Publisher's mpub-receiver address and X25519 public key from Cloud Map.
///
/// Filters Cloud Map instances by `role=publisher` attribute.
/// Returns `(mpub_receiver_addr, x25519_pubkey_bytes)`.
pub async fn discover_publisher_from_cloudmap() -> Result<(SocketAddr, [u8; 32])> {
    let namespace = match env::var("GBN_CLOUDMAP_NAMESPACE") {
        Ok(v) if !v.is_empty() => v,
        _ => anyhow::bail!("GBN_CLOUDMAP_NAMESPACE not set"),
    };
    let service_name = env::var("GBN_CLOUDMAP_SERVICE_NAME")
        .or_else(|_| env::var("GBN_CLOUDMAP_SERVICE"))
        .unwrap_or_else(|_| "relay".to_string());

    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let client = aws_sdk_servicediscovery::Client::new(&config);

    let instances = client
        .discover_instances()
        .namespace_name(&namespace)
        .service_name(&service_name)
        .health_status(HealthStatusFilter::All)
        .send()
        .await?;

    for instance in instances.instances() {
        let Some(attrs) = instance.attributes() else {
            continue;
        };
        if attrs.get("role").map(|r| r == "publisher").unwrap_or(false) {
            let ip = match attrs.get("AWS_INSTANCE_IPV4") {
                Some(v) => v.clone(),
                None => continue,
            };
            let port_str = attrs
                .get("GBN_MPUB_PORT")
                .cloned()
                .unwrap_or_else(|| "7001".to_string());
            let pub_key_hex = match attrs.get("GBN_PUB_KEY_HEX") {
                Some(v) => v.clone(),
                None => continue,
            };

            let port: u16 = port_str.parse().unwrap_or(7001);
            let ip_addr: std::net::IpAddr = ip.parse()?;
            let addr = SocketAddr::new(ip_addr, port);

            let key_bytes = hex::decode(&pub_key_hex)?;
            anyhow::ensure!(key_bytes.len() == 32, "Publisher pubkey must be 32 bytes");
            let mut pubkey = [0u8; 32];
            pubkey.copy_from_slice(&key_bytes);

            tracing::info!("Cloud Map: found Publisher at {}", addr);
            return Ok((addr, pubkey));
        }
    }
    anyhow::bail!("Publisher not found in Cloud Map — has it registered yet?")
}

/// Register Publisher-specific attributes in Cloud Map.
///
/// Called by the Publisher node on startup after deriving its X25519 keypair.
/// Sets `GBN_PUB_KEY_HEX`, `role=publisher`, and `GBN_MPUB_PORT` attributes
/// on the service instance (identified by `GBN_CLOUDMAP_SERVICE_ID` + hostname).
pub async fn register_publisher_pubkey_in_cloudmap(pub_key_hex: &str) -> Result<()> {
    let service_id = match env::var("GBN_CLOUDMAP_SERVICE_ID") {
        Ok(v) if !v.is_empty() => v,
        _ => return Ok(()),
    };
    let instance_id = env::var("GBN_CLOUDMAP_INSTANCE_ID")
        .or_else(|_| env::var("HOSTNAME"))
        .unwrap_or_else(|_| "publisher".to_string());
    let ipv4 = match env::var("GBN_INSTANCE_IPV4") {
        Ok(v) if !v.is_empty() => v,
        _ => return Ok(()),
    };
    let mpub_port = env::var("GBN_MPUB_PORT").unwrap_or_else(|_| "7001".to_string());

    let mut attrs = HashMap::new();
    attrs.insert("AWS_INSTANCE_IPV4".to_string(), ipv4);
    attrs.insert("GBN_PUB_KEY_HEX".to_string(), pub_key_hex.to_string());
    attrs.insert("GBN_MPUB_PORT".to_string(), mpub_port);
    attrs.insert("role".to_string(), "publisher".to_string());
    attrs.insert("GBN_SUBNET_TAG".to_string(), "FreeSubnet".to_string());

    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let client = aws_sdk_servicediscovery::Client::new(&config);
    client
        .register_instance()
        .service_id(service_id)
        .instance_id(instance_id)
        .set_attributes(Some(attrs))
        .send()
        .await?;
    tracing::info!("Publisher: registered pubkey in Cloud Map");
    Ok(())
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

    // Refresh from Cloud Map
    let (addr, _) = discover_publisher_from_cloudmap().await?;
    *cached_addr = Some(addr);
    *last_refresh = std::time::Instant::now();
    Ok(addr)
}
