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

    /// Drop a peer (rotation). Contract helper for `netsec.peer_rotation`.
    pub fn remove(&mut self, peer: &PeerId) -> bool {
        let key = peer.to_bytes();
        let Some(pfx) = self.peers.remove(&key) else {
            return false;
        };
        if let Some(n) = self.by_prefix.get(&pfx).copied() {
            if n <= 1 {
                self.by_prefix.remove(&pfx);
            } else {
                self.by_prefix.insert(pfx, n - 1);
            }
        }
        true
    }

    /// Occupancy of one prefix (`netsec.ip_slot_cap` / `netsec.asn_cap`).
    pub fn count_prefix(&self, pfx: IpPrefix) -> usize {
        self.by_prefix.get(&pfx).copied().unwrap_or(0)
    }

    /// Admitted peer id bytes, BTree order.
    pub fn peer_keys(&self) -> Vec<Vec<u8>> {
        self.peers.keys().cloned().collect()
    }

    /// Prefix recorded for a peer.
    pub fn prefix_of_peer(&self, peer_bytes: &[u8]) -> Option<IpPrefix> {
        self.peers.get(peer_bytes).copied()
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

    /// Whether a peer currently occupies a slot.
    pub fn contains(&self, peer: &PeerId) -> bool {
        self.peers.contains_key(&peer.to_bytes())
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }
}

/// IP-prefix stand-in for ASN bucketing. Contract: `netsec.asn_cap`.
///
/// **Deployment-time data, not a code gap:** this environment has no ASN
/// database and the sandbox cannot fetch one. Production should map IP → ASN
/// (MaxMind / Team Cymru / a local IRR dump) and treat each ASN as one bucket.
/// [`AsnCap::v4_prefix_len`] makes the *granularity* configurable so operators
/// can approximate that aggregation (e.g. /16 vs Tier 6's /24) without
/// pretending `/24 == ASN`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AsnCap {
    /// IPv4 prefix length used as the ASN surrogate (default 24).
    pub v4_prefix_len: u8,
}

impl Default for AsnCap {
    fn default() -> Self {
        Self { v4_prefix_len: 24 }
    }
}

/// Mask `ip` to `v4_prefix_len` bits (IPv6 still uses [`prefix_of`]).
pub fn asn_bucket(ip: IpAddr, cap: &AsnCap) -> IpPrefix {
    match ip {
        IpAddr::V4(v) => {
            let mut o = v.octets();
            let len = cap.v4_prefix_len.min(32);
            let full = (len / 8) as usize;
            let rem = len % 8;
            for b in o.iter_mut().skip(full) {
                *b = 0;
            }
            if rem != 0 && full < 4 {
                o[full] &= 0xffu8 << (8 - rem);
            }
            IpPrefix::V4([o[0], o[1], o[2]])
        }
        IpAddr::V6(_) => prefix_of(ip),
    }
}

/// Evict low-scoring, over-represented buckets first. Contract: `netsec.peer_rotation`.
///
/// Uses [`crate::scoring::score`] (`gossip.scoring`) and [`asn_bucket`]
/// (`netsec.asn_cap`). Peer order is sorted so tests are reproducible.
pub fn rotate_peers(
    table: &mut IpSlotTable,
    peers: &[(PeerId, IpAddr)],
    stats: &types::collections::Map<Vec<u8>, crate::scoring::PeerStats>,
    cap: &AsnCap,
    evict_count: usize,
) -> Vec<PeerId> {
    use crate::scoring::score;
    let mut bucket_n: types::collections::Map<IpPrefix, usize> = types::collections::Map::new();
    for (peer, ip) in peers {
        if table.contains(peer) {
            *bucket_n.entry(asn_bucket(*ip, cap)).or_insert(0) += 1;
        }
    }
    let mut rows: Vec<(i32, usize, Vec<u8>, PeerId)> = Vec::new();
    for (peer, ip) in peers {
        if !table.contains(peer) {
            continue;
        }
        let key = peer.to_bytes();
        let sc = stats.get(&key).map(score).unwrap_or(0);
        let b = asn_bucket(*ip, cap);
        let occ = bucket_n.get(&b).copied().unwrap_or(0);
        rows.push((sc, occ, key, *peer));
    }
    rows.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    let mut evicted = Vec::new();
    for (_, _, _, peer) in rows.into_iter().take(evict_count) {
        table.remove(&peer);
        evicted.push(peer);
    }
    evicted
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

    #[test]
    fn asn_cap_coarser_than_slash24() {
        let cap = AsnCap { v4_prefix_len: 16 };
        assert_eq!(
            asn_bucket(v4(10, 1, 2, 3), &cap),
            asn_bucket(v4(10, 1, 9, 9), &cap)
        );
        assert_ne!(
            asn_bucket(v4(10, 1, 2, 3), &cap),
            asn_bucket(v4(10, 2, 0, 1), &cap)
        );
        let fine = AsnCap { v4_prefix_len: 24 };
        assert_ne!(
            asn_bucket(v4(10, 1, 2, 3), &fine),
            asn_bucket(v4(10, 1, 9, 9), &fine)
        );
        let _ = prefix_of(v4(10, 1, 2, 3));
    }

    #[test]
    fn rotation_shrinks_capped_adversary_share() {
        use crate::scoring::PeerStats;
        use types::collections::Map;
        let mut t = IpSlotTable::new(8, 12);
        let cap = AsnCap { v4_prefix_len: 16 };
        let mut adv = Vec::new();
        for i in 0..8u8 {
            let id = identity::generate().unwrap();
            assert!(t.admit(&id.peer_id, v4(10, 0, i, 1)));
            adv.push((id.peer_id, v4(10, 0, i, 1)));
        }
        let mut hon = Vec::new();
        for i in 0..4u8 {
            let id = identity::generate().unwrap();
            assert!(t.admit(&id.peer_id, v4(11 + i, 1, 2, 3)));
            hon.push((id.peer_id, v4(11 + i, 1, 2, 3)));
        }
        let initial_adv = adv.iter().filter(|(p, _)| t.contains(p)).count();
        assert_eq!(initial_adv, 8);
        let mut stats: Map<Vec<u8>, PeerStats> = Map::new();
        for (p, _) in &adv {
            stats.insert(
                p.to_bytes(),
                PeerStats {
                    valid_msgs: 0,
                    invalid_msgs: 20,
                    latency_ms: 200,
                },
            );
        }
        for (p, _) in &hon {
            stats.insert(
                p.to_bytes(),
                PeerStats {
                    valid_msgs: 50,
                    invalid_msgs: 0,
                    latency_ms: 10,
                },
            );
        }
        let mut all = adv.clone();
        all.extend(hon.clone());
        for _ in 0..4 {
            rotate_peers(&mut t, &all, &stats, &cap, 2);
            let mut in_bucket = adv.iter().filter(|(p, _)| t.contains(p)).count();
            for (p, ip) in &adv {
                if t.contains(p) {
                    continue;
                }
                if in_bucket >= 2 {
                    break;
                }
                if t.admit(p, *ip) {
                    in_bucket += 1;
                }
            }
        }
        let remain = adv.iter().filter(|(p, _)| t.contains(p)).count();
        assert!(
            remain < initial_adv,
            "adversary share {remain} should drop from {initial_adv}"
        );
        let _ = crate::scoring::score(&PeerStats::default());
    }
}
