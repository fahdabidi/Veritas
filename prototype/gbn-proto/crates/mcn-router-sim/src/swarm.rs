use anyhow::Result;
use libp2p::{
    identity,
    kad::{store::MemoryStore, Kademlia, KademliaConfig},
    noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Swarm, SwarmBuilder,
};
use std::time::Duration;

#[derive(NetworkBehaviour)]
pub struct RouterBehaviour {
    pub kademlia: Kademlia<MemoryStore>,
}

pub fn build_swarm(local_key: identity::Keypair) -> Result<Swarm<RouterBehaviour>> {
    let local_peer_id = local_key.public().to_peer_id();

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
            let store = MemoryStore::new(peer_id);
            RouterBehaviour {
                kademlia: Kademlia::with_config(peer_id, store, kad_config),
            }
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    Ok(swarm)
}
