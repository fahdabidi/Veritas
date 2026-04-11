use anyhow::Result;
use aws_sdk_servicediscovery::types::HealthStatusFilter;
use crate::observability::MetricsReporter;
use crate::gossip::{
    new_plumtree_behaviour, GossipRequest, GossipResponse, OutboundGossip, PlumTreeBehaviour,
    PlumTreeEngine,
};
use libp2p::futures::StreamExt;
use libp2p::{
    identity,
    kad::{store::MemoryStore, Behaviour as Kademlia, Config as KademliaConfig},
    multiaddr::Protocol,
    noise,
    request_response,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Swarm, SwarmBuilder,
};
use std::{collections::HashMap, env, net::IpAddr};
use std::time::Duration;
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
}

pub fn gossip_config_from_env() -> (usize, usize) {
    let gossip_bps = env::var("GBN_GOSSIP_BPS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(15 * 1024 * 1024 / 8);
    let max_tracked_messages = env::var("GBN_MAX_TRACKED_MESSAGES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .or_else(|| env::var("GBN_MAX_TRACKED_PEERS").ok().and_then(|v| v.parse::<usize>().ok()))
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
        Self {
            engine: PlumTreeEngine::new(gossip_bps, max_tracked_messages),
            metrics,
            last_gossip_bytes_published: 0,
            last_gossip_publish: Instant::now(),
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

    let added = bootstrap_from_cloudmap(&mut swarm).await?;
    tokio::spawn(async move {
        if let Ok(reporter) = MetricsReporter::from_env().await {
            let _ = reporter.publish_bootstrap_result(added > 0).await;
        }
    });
    let _ = register_with_cloudmap(&swarm).await;

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
                request,
                channel,
                ..
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
    if let Some(event) = swarm.next().await {
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
        if let Some(metrics) = &runtime.metrics {
            let _ = metrics.publish_gossip_bandwidth_bytes(delta).await;
        }
        runtime.last_gossip_bytes_published = total;
        runtime.last_gossip_publish = Instant::now();
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
    let service_name = env::var("GBN_CLOUDMAP_SERVICE_NAME").unwrap_or_else(|_| "relay".to_string());
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
        .health_status(HealthStatusFilter::Healthy)
        .send()
        .await?;

    let mut added = 0usize;
    for instance in instances.instances() {
        let Some(attrs) = instance.attributes() else { continue };
        let ip: Option<String> = attrs.get("AWS_INSTANCE_IPV4").cloned();
        let peer_id_str: Option<String> = attrs.get("GBN_PEER_ID").cloned();

        let Some(ip) = ip else { continue };
        let Ok(ip_addr) = ip.parse::<IpAddr>() else { continue };

        let mut addr = libp2p::Multiaddr::empty();
        addr.push(Protocol::from(ip_addr));
        addr.push(Protocol::Tcp(p2p_port));

        if let Some(peer_id_str) = peer_id_str {
            if let Ok(peer_id) = peer_id_str.parse::<libp2p::PeerId>() {
                swarm.behaviour_mut().kademlia.add_address(&peer_id, addr.clone());
                added += 1;
            }
        }

        let _ = swarm.dial(addr);
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
