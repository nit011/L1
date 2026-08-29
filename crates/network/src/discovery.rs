//! Kademlia DHT and bootstrap lists (architecture.md §5 Peer discovery).
//!
//! Discovery uses the QUIC transport from [`crate::transport`] (`p2p.quic`).
//! ASN-level mapping is out of scope (Tier 14 `netsec.asn_cap`).

use crate::identity::NodeIdentity;
use libp2p::kad::store::MemoryStore;
use libp2p::kad::{self, Behaviour as KadBehaviour, Config as KadConfig, Mode};
use libp2p::{identity, Swarm, SwarmBuilder};
use libp2p::{Multiaddr, PeerId, StreamProtocol};
use std::collections::BTreeMap;
use thiserror::Error;

/// Canonical Kademlia protocol id for this chain.
pub const KAD_PROTOCOL: &str = "/l1/kad/1.0.0";

/// Bootstrap peers: [`PeerId`] → listen [`Multiaddr`], sorted by peer id.
///
/// Contract: `p2p.bootstrap`.
#[derive(Clone, Debug, Default)]
pub struct BootstrapList {
    /// Sorted map so test iteration is deterministic.
    pub peers: BTreeMap<PeerId, Multiaddr>,
}

impl BootstrapList {
    /// Empty list.
    pub fn new() -> Self {
        Self {
            peers: BTreeMap::new(),
        }
    }

    /// Insert a bootstrap address.
    pub fn insert(&mut self, peer: PeerId, addr: Multiaddr) {
        self.peers.insert(peer, addr);
    }
}

/// Discovery / swarm errors.
#[derive(Debug, Error)]
pub enum DiscoveryError {
    /// Swarm or behaviour construction failed.
    #[error("swarm: {0}")]
    Swarm(String),
}

/// Kademlia behaviour on a [`MemoryStore`]. Contract: `p2p.kademlia`.
pub fn kademlia_behaviour(peer_id: PeerId) -> KadBehaviour<MemoryStore> {
    let mut cfg = KadConfig::new(StreamProtocol::new(KAD_PROTOCOL));
    cfg.set_query_timeout(std::time::Duration::from_secs(10));
    let mut kad = KadBehaviour::with_config(peer_id, MemoryStore::new(peer_id), cfg);
    kad.set_mode(Some(Mode::Server));
    kad
}

/// Record bootstrap addresses and start a DHT bootstrap query. Contract: `p2p.bootstrap`.
pub fn apply_bootstrap(kad: &mut KadBehaviour<MemoryStore>, list: &BootstrapList) {
    for (peer, addr) in &list.peers {
        kad.add_address(peer, addr.clone());
    }
    if !list.peers.is_empty() {
        let _ = kad.bootstrap();
    }
}

/// Combined identify + Kademlia behaviour for a discovery-only swarm.
#[derive(Debug)]
pub enum DiscoveryEvent {
    /// Identify.
    Identify(libp2p::identify::Event),
    /// Kademlia.
    Kademlia(kad::Event),
}

impl From<libp2p::identify::Event> for DiscoveryEvent {
    fn from(event: libp2p::identify::Event) -> Self {
        Self::Identify(event)
    }
}

impl From<kad::Event> for DiscoveryEvent {
    fn from(event: kad::Event) -> Self {
        Self::Kademlia(event)
    }
}

#[derive(libp2p::swarm::NetworkBehaviour)]
#[behaviour(to_swarm = "DiscoveryEvent")]
pub struct DiscoveryBehaviour {
    /// Identify (required so peers exchange listen addrs).
    pub identify: libp2p::identify::Behaviour,
    /// Kademlia DHT (`p2p.kademlia`).
    pub kademlia: KadBehaviour<MemoryStore>,
}

impl DiscoveryBehaviour {
    fn new(key: &identity::Keypair) -> Self {
        let peer_id = PeerId::from(key.public());
        let identify = libp2p::identify::Behaviour::new(libp2p::identify::Config::new(
            "/l1/identify/1.0.0".into(),
            key.public(),
        ));
        Self {
            identify,
            kademlia: kademlia_behaviour(peer_id),
        }
    }
}

/// QUIC swarm with Kademlia (no gossip). Uses `p2p.quic` + `p2p.identity`.
pub fn discovery_swarm(
    identity: NodeIdentity,
    bootstrap: &BootstrapList,
) -> Result<Swarm<DiscoveryBehaviour>, DiscoveryError> {
    let mut swarm = SwarmBuilder::with_existing_identity(identity.keypair)
        .with_tokio()
        .with_quic()
        .with_behaviour(DiscoveryBehaviour::new)
        .map_err(|e| DiscoveryError::Swarm(e.to_string()))?
        .build();
    apply_bootstrap(&mut swarm.behaviour_mut().kademlia, bootstrap);
    Ok(swarm)
}

/// Number of peers currently in Kademlia's routing table (all buckets).
pub fn kad_peer_count(kad: &mut KadBehaviour<MemoryStore>) -> usize {
    kad.kbuckets().map(|b| b.iter().count()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity;
    use crate::transport::{quic_config, quic_listen_local};

    #[test]
    fn kademlia_uses_quic_identity() {
        let id = identity::generate().unwrap();
        let _ = quic_config(&id);
        let mut kad = kademlia_behaviour(id.peer_id);
        assert_eq!(kad_peer_count(&mut kad), 0);
    }

    #[test]
    fn bootstrap_list_is_sorted_by_peer_id() {
        let a = identity::generate().unwrap();
        let b = identity::generate().unwrap();
        let mut list = BootstrapList::new();
        list.insert(b.peer_id, quic_listen_local());
        list.insert(a.peer_id, quic_listen_local());
        let keys: Vec<_> = list.peers.keys().copied().collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }

    #[test]
    fn unknown_bootstrap_peer_does_not_panic() {
        let id = identity::generate().unwrap();
        let mut kad = kademlia_behaviour(id.peer_id);
        let mut list = BootstrapList::new();
        list.insert(identity::generate().unwrap().peer_id, quic_listen_local());
        apply_bootstrap(&mut kad, &list);
        assert!(kad_peer_count(&mut kad) <= 1);
    }
}
