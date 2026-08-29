//! Safety predicates (architecture.md §2.1, §2.4).

use crate::qc::has_quorum;
use crate::steps::Finalized;
use types::collections::Map;
use types::{Hash, Height, VotingPower};

/// True when reachable power cannot form a precommit QC. Contract: `cons.halt_no_quorum`.
///
/// Safety over liveness: do **not** commit on a minority partition.
pub fn halt_no_quorum(reachable: VotingPower, total: VotingPower) -> bool {
    !has_quorum(reachable, total)
}

/// At most one distinct committed block per height. Contract: `cons.safety.no_two_commits`.
#[derive(Clone, Debug, Default)]
pub struct CommitLog {
    by_height: Map<Height, Hash>,
}

impl CommitLog {
    /// Empty log.
    pub fn new() -> Self {
        Self {
            by_height: Map::new(),
        }
    }

    /// Record a [`Finalized`] from `cons.commit`. Errors on a conflicting hash.
    pub fn record(&mut self, f: &Finalized) -> Result<(), SafetyError> {
        if let Some(prev) = self.by_height.get(&f.height) {
            if *prev != f.block_hash {
                return Err(SafetyError::TwoCommits { height: f.height });
            }
            return Ok(());
        }
        self.by_height.insert(f.height, f.block_hash);
        Ok(())
    }

    /// Committed header hash at `height`, if any.
    pub fn get(&self, height: Height) -> Option<Hash> {
        self.by_height.get(&height).copied()
    }
}

/// Safety violation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SafetyError {
    /// Two different blocks at one height.
    TwoCommits { height: Height },
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::Round;

    #[test]
    fn halt_when_less_than_quorum_reachable() {
        let total = VotingPower(4);
        assert!(halt_no_quorum(VotingPower(2), total));
        assert!(!halt_no_quorum(VotingPower(3), total));
    }

    #[test]
    fn no_two_commits_detects_conflict() {
        let mut log = CommitLog::new();
        let a = Finalized {
            height: Height::GENESIS,
            round: Round::ZERO,
            block_hash: Hash::from_bytes([1u8; 32]),
            app_hash: Hash::from_bytes([9u8; 32]),
        };
        log.record(&a).unwrap();
        let mut b = a.clone();
        b.block_hash = Hash::from_bytes([2u8; 32]);
        assert!(matches!(
            log.record(&b),
            Err(SafetyError::TwoCommits { height }) if height == Height::GENESIS
        ));
        log.record(&a).unwrap();
    }
}
