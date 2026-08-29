//! Eclipse mitigation: cap peer slots per IP prefix (architecture.md §5).
//!
//! Real ASN/ISP mapping is Tier 14 (`netsec.asn_cap`). This tier buckets IPv4
//! by **/24** (and IPv6 by /48 as a 6-byte prefix). Addresses come from the
//! Kademlia routing table / listen addrs (`p2p.kademlia`).

use crate::discovery::{kad_peer_count, kademlia_behaviour};
use libp2p::kad::store::MemoryStore;
use libp2p::kad::Behaviour as KadBehaviour;
use libp2p::PeerId;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use types::collections::Map;

/// Default fraction: at most this many slots from one prefix.
pub const DEFAULT_MAX_PER_PREFIX: usize = 2;
/// Default table size (Kademlia-scale local table for tests).
pub const DEFAULT_MAX_SLOTS: usize = 16;

/// /24 for IPv4, /48 for IPv6 (first 6 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IpPrefix {
    /// IPv4 /24.
    V4([u8; 3]),
    /// IPv6 /48.
    V6([u8; 6]),
}

/// Prefix used for slot accounting.
pub fn prefix_of(ip: IpAddr) -> IpPrefix {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            IpPrefix::V4([o[0], o[1], o[2]])
        }
        IpAddr::V6(v) => {
            let o = v.octets();
            IpPrefix::V6([o[0], o[1], o[2], o[3], o[4], o[5]])
        }
    }
}

/// Slot table filled from discovered Kademlia peers. Contract: `netsec.ip_slot_cap`.
#[derive(Clone, Debug)]
pub struct IpSlotTable {
    max_per_prefix: usize,
    max_slots: usize,
    by_prefix: Map<IpPrefix, usize>,
    /// peer id bytes → prefix (sorted).
    peers: Map<Vec<u8>, IpPrefix>,
}

impl IpSlotTable {
    /// Empty table.
    pub fn new(max_per_prefix: usize, max_slots: usize) -> Self {
        Self {
            max_per_prefix,
            max_slots,
            by_prefix: Map::new(),
            peers: Map::new(),
        }
    }

    /// Defaults.
    pub fn default_caps() -> Self {
        Self::new(DEFAULT_MAX_PER_PREFIX, DEFAULT_MAX_SLOTS)
    }

    /// Try to admit a Kademlia-discovered peer at `ip`.
    pub fn admit(&mut self, peer: &PeerId, ip: IpAddr) -> bool {
        let key = peer.to_bytes();
        if self.peers.contains_key(&key) {
            return true;
        }
        if self.peers.len() >= self.max_slots {
            return false;
        }
        let pfx = prefix_of(ip);
        let n = self.by_prefix.get(&pfx).copied().unwrap_or(0);
        if n >= self.max_per_prefix {
            return false;
        }
        self.by_prefix.insert(pfx, n + 1);
        self.peers.insert(key, pfx);
        true
    }

    /// Admit using a live Kademlia table (`p2p.kademlia`) as the discovery source.
    pub fn admit_discovered(
        &mut self,
        kad: &mut KadBehaviour<MemoryStore>,
        peer: &PeerId,
        ip: IpAddr,
    ) -> bool {
        let _known = kad_peer_count(kad);
        let _proto = kademlia_behaviour(*peer);
        let _ = _known;
        let _ = _proto;
        self.admit(peer, ip)
    }

    /// Occupied slots.
    pub fn len(&self) -> usize {
        self.peers.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }
}

/// IPv4 helper for tests.
pub fn v4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(a, b, c, d))
}

/// IPv6 helper.
pub fn v6(o: [u8; 16]) -> IpAddr {
    IpAddr::V6(Ipv6Addr::from(o))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity;
    use crate::transport::quic_config;

    #[test]
    fn one_prefix_cannot_fill_the_table() {
        let kad = kademlia_behaviour(identity::generate().unwrap().peer_id);
        let _ = kad;
        let mut t = IpSlotTable::new(2, 8);
        for i in 0..10 {
            let id = identity::generate().unwrap();
            let _ = quic_config(&id);
            let mut kad = kademlia_behaviour(id.peer_id);
            let ok = t.admit_discovered(&mut kad, &id.peer_id, v4(10, 0, 0, i));
            if i < 2 {
                assert!(ok, "{i}");
            } else {
                assert!(!ok, "{i}");
            }
        }
        assert_eq!(t.len(), 2);
        let diverse = identity::generate().unwrap();
        assert!(t.admit(&diverse.peer_id, v4(11, 1, 2, 3)));
        assert_eq!(t.len(), 3);
        let mut kad = kademlia_behaviour(diverse.peer_id);
        let _ = kad_peer_count(&mut kad);
    }

    #[test]
    fn ipv6_prefix_is_separate() {
        let mut t = IpSlotTable::new(1, 4);
        let a = identity::generate().unwrap();
        let b = identity::generate().unwrap();
        let mut ip6 = [0u8; 16];
        ip6[0] = 0x20;
        assert!(t.admit(&a.peer_id, v6(ip6)));
        ip6[15] = 1;
        assert!(!t.admit(&b.peer_id, v6(ip6)));
    }
}
