//! P2P networking: libp2p QUIC, Kademlia, gossipsub (architecture.md §5).
//!
//! This crate may import `consensus` (message types and `qc.verify` /
//! `CommitLog`). `consensus` must never import this crate or libp2p.

pub mod blocks;
pub mod codec;
pub mod discovery;
pub mod eclipse;
pub mod gossip;
pub mod identity;
pub mod rate_limit;
pub mod scoring;
pub mod sync;
pub mod topics;
pub mod transport;
pub mod validation;
pub mod validator_mesh;
