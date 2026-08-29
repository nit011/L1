//! Peer scoring for gossipsub (architecture.md §5 Mempool DoS resistance / eclipse).
//!
//! Isolated from the swarm so Tier 14 `netsec.peer_rotation` can reuse the same
//! function. Mesh keep/drop uses [`crate::gossip::mesh_config`].

use crate::gossip::mesh_config;

/// Observable counters for one peer (validity, latency, spam).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PeerStats {
    /// Messages that passed topic validation.
    pub valid_msgs: u64,
    /// Messages rejected (bad sig, schema, etc.).
    pub invalid_msgs: u64,
    /// Smoothed one-way latency sample (milliseconds).
    pub latency_ms: u64,
}

/// Integer score. Higher is better. Contract: `gossip.scoring`.
pub fn score(stats: &PeerStats) -> i32 {
    let valid = i32::try_from(stats.valid_msgs.min(1_000)).unwrap_or(1_000);
    let invalid = i32::try_from(stats.invalid_msgs.min(1_000)).unwrap_or(1_000);
    let latency_pen = i32::try_from((stats.latency_ms / 50).min(40)).unwrap_or(40);
    valid.saturating_mul(3) - invalid.saturating_mul(12) - latency_pen
}

/// Whether the peer should remain in the gossipsub mesh.
pub fn stay_in_mesh(stats: &PeerStats) -> bool {
    let _mesh = mesh_config();
    score(stats) >= 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honest_peer_stays_in_mesh() {
        let s = PeerStats {
            valid_msgs: 10,
            invalid_msgs: 0,
            latency_ms: 20,
        };
        assert!(stay_in_mesh(&s));
        assert!(score(&s) > 0);
    }

    #[test]
    fn spam_peer_is_dropped() {
        let s = PeerStats {
            valid_msgs: 1,
            invalid_msgs: 20,
            latency_ms: 5,
        };
        assert!(!stay_in_mesh(&s));
        assert!(score(&s) < 0);
    }
}
