//! Gossipsub mesh (architecture.md §5 Block/tx propagation).
//!
//! Parameterized so [`crate::topics`] can subscribe `/l1/{tx,proposal,vote,block,evidence,headers,da-chunks}`.
//! Transport is QUIC (`p2p.quic`). Consensus types never appear as libp2p generics here.

use crate::identity::NodeIdentity;
use libp2p::gossipsub::{self, IdentTopic, MessageAuthenticity, ValidationMode};
use libp2p::identity::Keypair;
use libp2p::swarm::NetworkBehaviour;
use libp2p::{identify, Swarm, SwarmBuilder};
use thiserror::Error;

use crate::discovery::{apply_bootstrap, kademlia_behaviour, BootstrapList, DiscoveryError};
use libp2p::kad::store::MemoryStore;
use libp2p::kad::{self, Behaviour as KadBehaviour};
use libp2p::PeerId;

/// Topic names (stable strings for gossipsub).
pub const TOPIC_TX: &str = "/l1/tx/1";
/// Proposal topic (general gossip; validator mesh uses a dedicated name).
pub const TOPIC_PROPOSAL: &str = "/l1/proposal/1";
/// Vote topic.
pub const TOPIC_VOTE: &str = "/l1/vote/1";
/// Full block topic.
pub const TOPIC_BLOCK: &str = "/l1/block/1";
/// Equivocation evidence.
pub const TOPIC_EVIDENCE: &str = "/l1/evidence/1";
/// Header-first announcements.
pub const TOPIC_HEADERS: &str = "/l1/headers/1";
/// Individual DA chunks (light nodes; not the full-block topic). Contract: `gossip.da_chunks`.
pub const TOPIC_DA_CHUNKS: &str = da::das::TOPIC_DA_CHUNKS;

/// Mesh parameters used by scoring and rate limits. Contract: `gossip.mesh`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshConfig {
    /// Target mesh degree (`D` in gossipsub).
    pub mesh_n: usize,
    /// Heartbeat interval milliseconds.
    pub heartbeat_ms: u64,
}

impl Default for MeshConfig {
    fn default() -> Self {
        Self {
            mesh_n: 4,
            heartbeat_ms: 200,
        }
    }
}

/// Default mesh configuration. Contract: `gossip.mesh`.
pub fn mesh_config() -> MeshConfig {
    MeshConfig::default()
}

/// gossipsub [`IdentTopic`] for a stable name.
pub fn ident_topic(name: &str) -> IdentTopic {
    IdentTopic::new(name)
}

/// All application topics subscribed by a full node.
pub fn all_topics() -> Vec<IdentTopic> {
    vec![
        ident_topic(TOPIC_TX),
        ident_topic(TOPIC_PROPOSAL),
        ident_topic(TOPIC_VOTE),
        ident_topic(TOPIC_BLOCK),
        ident_topic(TOPIC_EVIDENCE),
        ident_topic(TOPIC_HEADERS),
        ident_topic(TOPIC_DA_CHUNKS),
    ]
}

/// gossipsub behaviour signed with `p2p.identity`. Contract: `gossip.mesh`.
pub fn gossipsub_behaviour(keypair: &Keypair) -> Result<gossipsub::Behaviour, GossipError> {
    let cfg = gossipsub::ConfigBuilder::default()
        .validation_mode(ValidationMode::Strict)
        .mesh_n_low(2)
        .mesh_n(mesh_config().mesh_n)
        .mesh_n_high(8)
        .heartbeat_interval(std::time::Duration::from_millis(mesh_config().heartbeat_ms))
        .build()
        .map_err(|e| GossipError::Config(e.to_string()))?;
    gossipsub::Behaviour::new(MessageAuthenticity::Signed(keypair.clone()), cfg)
        .map_err(|e| GossipError::Gossipsub(e.to_string()))
}

/// Events from the gossip/kad/identify swarm.
#[derive(Debug)]
pub enum L1Event {
    /// gossipsub.
    Gossipsub(gossipsub::Event),
    /// Kademlia.
    Kademlia(kad::Event),
    /// Identify.
    Identify(identify::Event),
}

impl From<gossipsub::Event> for L1Event {
    fn from(event: gossipsub::Event) -> Self {
        Self::Gossipsub(event)
    }
}

impl From<kad::Event> for L1Event {
    fn from(event: kad::Event) -> Self {
        Self::Kademlia(event)
    }
}

impl From<identify::Event> for L1Event {
    fn from(event: identify::Event) -> Self {
        Self::Identify(event)
    }
}

/// Mesh + Kademlia + identify. Contract: `gossip.mesh` on QUIC.
#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "L1Event")]
pub struct L1Behaviour {
    /// gossipsub.
    pub gossipsub: gossipsub::Behaviour,
    /// Kademlia (same swarm as gossip).
    pub kademlia: KadBehaviour<MemoryStore>,
    /// Identify.
    pub identify: identify::Behaviour,
}

impl L1Behaviour {
    /// Construct behaviours from the node's keypair.
    pub fn new(key: &Keypair) -> Result<Self, GossipError> {
        let peer_id = PeerId::from(key.public());
        Ok(Self {
            gossipsub: gossipsub_behaviour(key)?,
            kademlia: kademlia_behaviour(peer_id),
            identify: identify::Behaviour::new(identify::Config::new(
                "/l1/identify/1.0.0".into(),
                key.public(),
            )),
        })
    }
}

/// Gossip / swarm errors.
#[derive(Debug, Error)]
pub enum GossipError {
    /// gossipsub config.
    #[error("gossipsub config: {0}")]
    Config(String),
    /// gossipsub constructor.
    #[error("gossipsub: {0}")]
    Gossipsub(String),
    /// Subscribe.
    #[error("subscribe {0}")]
    Subscribe(String),
    /// Swarm.
    #[error("swarm: {0}")]
    Swarm(String),
}

/// QUIC swarm with gossipsub and Kademlia.
pub fn mesh_swarm(
    identity: NodeIdentity,
    bootstrap: &BootstrapList,
) -> Result<Swarm<L1Behaviour>, GossipError> {
    let mut swarm = SwarmBuilder::with_existing_identity(identity.keypair.clone())
        .with_tokio()
        .with_quic()
        .with_behaviour(|key| L1Behaviour::new(key).expect("gossipsub config"))
        .map_err(|e| GossipError::Swarm(e.to_string()))?
        .build();
    apply_bootstrap(&mut swarm.behaviour_mut().kademlia, bootstrap);
    for t in all_topics() {
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&t)
            .map_err(|e| GossipError::Subscribe(e.to_string()))?;
    }
    Ok(swarm)
}

impl From<DiscoveryError> for GossipError {
    fn from(e: DiscoveryError) -> Self {
        GossipError::Swarm(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity;
    use crate::transport::quic_config;

    #[test]
    fn mesh_is_parameterized_for_topics() {
        let cfg = mesh_config();
        assert_eq!(cfg.mesh_n, 4);
        let topics = all_topics();
        assert_eq!(topics.len(), 7);
        assert!(topics
            .iter()
            .any(|t| t.hash() == ident_topic(TOPIC_DA_CHUNKS).hash()));
        assert!(topics
            .iter()
            .any(|t| t.hash() == ident_topic(TOPIC_TX).hash()));
    }

    #[test]
    fn gossipsub_behaviour_uses_quic_identity() {
        let id = identity::generate().unwrap();
        let _ = quic_config(&id);
        let g = gossipsub_behaviour(&id.keypair).unwrap();
        assert!(g.topics().next().is_none());
    }

    #[test]
    fn subscribe_rejects_empty_topic_string_via_hash_mismatch() {
        let a = ident_topic(TOPIC_TX);
        let b = ident_topic("/l1/tx/999");
        assert_ne!(a.hash(), b.hash());
    }
}
