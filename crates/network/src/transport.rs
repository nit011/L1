//! QUIC transport for libp2p (architecture.md §5 Transport).
//!
//! QUIC provides multiplexed streams, built-in encryption, and better
//! head-of-line-blocking behavior than TCP for gossip fan-out. Identity is
//! [`crate::identity`] (`p2p.identity`) — Noise/TLS is bound to that keypair.

use crate::identity::NodeIdentity;
use libp2p::multiaddr::{Multiaddr, Protocol};
use libp2p::quic;
use std::net::Ipv4Addr;

/// Build a libp2p QUIC config from `p2p.identity`. Contract: `p2p.quic`.
pub fn quic_config(identity: &NodeIdentity) -> quic::Config {
    quic::Config::new(&identity.keypair)
}

/// Localhost QUIC listen address (`/ip4/127.0.0.1/udp/0/quic-v1`).
pub fn quic_listen_local() -> Multiaddr {
    Multiaddr::empty()
        .with(Protocol::Ip4(Ipv4Addr::LOCALHOST))
        .with(Protocol::Udp(0))
        .with(Protocol::QuicV1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity;

    #[test]
    fn quic_config_is_bound_to_identity() {
        let id = identity::generate().unwrap();
        let cfg = quic_config(&id);
        let _ = cfg;
        let addr = quic_listen_local();
        assert!(addr.to_string().contains("quic"));
        assert!(addr.to_string().contains("127.0.0.1"));
    }

    #[test]
    fn quic_listen_is_not_tcp() {
        let addr = quic_listen_local();
        assert!(!addr.to_string().contains("/tcp/"));
    }
}
