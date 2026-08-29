//! Tendermint lock and round change (architecture.md §2.2, §2.4).

use crate::replay::VoteKind;
use crate::timeout::{BoundClock, TimeoutStep};
use crate::vote::{nil, Vote, VoteBlock};
use blst::min_pk::SecretKey;
use types::{Clock, Hash, Height, Round, ValidatorId};

/// Locked value after a precommit (architecture.md §2.1 safety).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lock {
    /// Height of the lock.
    pub height: Height,
    /// Round in which we precommitted.
    pub round: Round,
    /// Locked block (`header.hash`), never nil.
    pub block_hash: Hash,
}

/// Whether a prevote for `candidate` is allowed. Contract: `cons.lock`.
///
/// After a precommit at `(height, round, H)`, we must not prevote a different
/// block at a later round unless we have a **higher-round polka** (2/3+
/// prevotes) for that other block.
pub fn prevote_allowed(
    lock: Option<Lock>,
    height: Height,
    candidate: VoteBlock,
    polka_round: Option<(Round, Hash)>,
) -> bool {
    let Some(lock) = lock else {
        return true;
    };
    if lock.height != height {
        return true;
    }
    match candidate {
        VoteBlock::Nil => true,
        VoteBlock::Block(h) => {
            if h == lock.block_hash {
                return true;
            }
            matches!(polka_round, Some((r, ph)) if r > lock.round && ph == h)
        }
    }
}

/// Advance round after a bound timeout or a nil-vote quorum. Contract: `cons.round_change`.
pub fn round_change_on_timeout<C: Clock>(
    clock: &BoundClock<C>,
    step: TimeoutStep,
    round: Round,
    started_at_ms: u64,
) -> Option<Round> {
    if clock.elapsed(step, round, started_at_ms) {
        Some(round.saturating_next())
    } else {
        None
    }
}

/// Nil vote used when a round times out without a decision. Contract: `vote.nil`.
pub fn timeout_nil(
    sk: &SecretKey,
    signer: ValidatorId,
    height: Height,
    round: Round,
    kind: VoteKind,
) -> Vote {
    nil(sk, signer, height, round, kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeout::TimeoutConfig;
    use types::TestClock;

    #[test]
    fn lock_blocks_conflicting_prevote_without_polka() {
        let lock = Some(Lock {
            height: Height(1),
            round: Round::ZERO,
            block_hash: Hash::from_bytes([1u8; 32]),
        });
        let other = VoteBlock::Block(Hash::from_bytes([2u8; 32]));
        assert!(!prevote_allowed(lock, Height(1), other, None));
        assert!(prevote_allowed(
            lock,
            Height(1),
            other,
            Some((Round(1), Hash::from_bytes([2u8; 32]))),
        ));
        assert!(prevote_allowed(
            lock,
            Height(1),
            VoteBlock::Block(Hash::from_bytes([1u8; 32])),
            None
        ));
    }

    #[test]
    fn propose_timeout_advances_round() {
        let clock = TestClock::new(0);
        let cfg = TimeoutConfig::from_spec();
        let dur = cfg.duration_ms(TimeoutStep::Propose, Round::ZERO);
        let bound = BoundClock::new(&clock, cfg);
        assert!(round_change_on_timeout(&bound, TimeoutStep::Propose, Round::ZERO, 0).is_none());
        clock.advance(dur);
        assert_eq!(
            round_change_on_timeout(&bound, TimeoutStep::Propose, Round::ZERO, 0),
            Some(Round(1))
        );
    }
}
