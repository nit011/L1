//! Per-peer message rate limits (architecture.md §5 Mempool DoS resistance).
//!
//! Bounds are derived from Tier 0 `spec.constants` (`MAX_BLOCK_BYTES` /
//! `MAX_TX_BYTES`, `MEMPOOL_MAX_TXS`). A peer that exceeds the cap is dropped
//! immediately in the same window — not "eventually".

use crate::gossip::mesh_config;
use libp2p::PeerId;
use types::collections::Map;
use types::{MAX_BLOCK_BYTES, MAX_TX_BYTES, MEMPOOL_MAX_TXS};

/// Messages allowed from one peer per window.
///
/// Derived from `spec.constants`: a peer cannot push more than one block's
/// worth of max-size txs, and never more than `MEMPOOL_MAX_TXS`.
pub fn peer_msg_limit() -> u32 {
    let from_size = (MAX_BLOCK_BYTES / MAX_TX_BYTES).max(1);
    from_size.min(MEMPOOL_MAX_TXS)
}

/// Per-peer counters keyed by sorted peer-id bytes. Contract: `netsec.peer_rate_limit`.
#[derive(Clone, Debug, Default)]
pub struct PeerRateLimiter {
    /// `PeerId` bytes → count in the current window.
    counts: Map<Vec<u8>, u32>,
    /// Cap from [`peer_msg_limit`].
    limit: u32,
}

impl PeerRateLimiter {
    /// New limiter using `spec.constants`.
    pub fn new() -> Self {
        let _ = mesh_config();
        Self {
            counts: Map::new(),
            limit: peer_msg_limit(),
        }
    }

    /// Record one inbound gossip message. Returns `false` if the peer is over cap.
    pub fn allow(&mut self, peer: &PeerId) -> bool {
        let key = peer.to_bytes();
        let n = self.counts.entry(key).or_insert(0);
        if *n >= self.limit {
            return false;
        }
        *n = n.saturating_add(1);
        true
    }

    /// Messages already counted for `peer` in this window.
    pub fn count(&self, peer: &PeerId) -> u32 {
        self.counts.get(&peer.to_bytes()).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity;

    #[test]
    fn limit_comes_from_spec_constants() {
        assert_eq!(
            peer_msg_limit(),
            (MAX_BLOCK_BYTES / MAX_TX_BYTES).min(MEMPOOL_MAX_TXS)
        );
        assert!(peer_msg_limit() > 0);
    }

    #[test]
    fn flood_is_dropped_in_same_run() {
        let mut lim = PeerRateLimiter::new();
        lim.limit = 3;
        let peer = identity::generate().unwrap().peer_id;
        assert!(lim.allow(&peer));
        assert!(lim.allow(&peer));
        assert!(lim.allow(&peer));
        assert!(!lim.allow(&peer));
        assert_eq!(lim.count(&peer), 3);
        let other = identity::generate().unwrap().peer_id;
        assert!(lim.allow(&other));
    }
}
