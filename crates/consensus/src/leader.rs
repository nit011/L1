//! TEST-ONLY round-robin leader (development-plan.md Tier 4).
//!
//! **Do not wire this into production.** Production leader selection is
//! [`crate::vrf::weighted_leader`] (architecture.md §2.3). Round-robin leaks
//! the next N proposers and is kept only so unit tests can drive a deterministic
//! proposer without VRF keys.

use types::{Round, ValidatorId};

/// TEST-ONLY: `validators[round % n]` after sorting by [`ValidatorId`].
///
/// Contract: `cons.round_robin.testdouble`.
pub fn round_robin_testdouble(validators: &[ValidatorId], round: Round) -> Option<ValidatorId> {
    if validators.is_empty() {
        return None;
    }
    let mut ordered = validators.to_vec();
    ordered.sort();
    let n = ordered.len();
    let idx = (round.0 as usize) % n;
    Some(ordered[idx])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(b: u8) -> ValidatorId {
        ValidatorId::from_bytes([b; 48])
    }

    #[test]
    fn cycles_sorted_ids() {
        let set = [v(3), v(1), v(2)];
        assert_eq!(round_robin_testdouble(&set, Round::ZERO), Some(v(1)));
        assert_eq!(round_robin_testdouble(&set, Round(1)), Some(v(2)));
        assert_eq!(round_robin_testdouble(&set, Round(2)), Some(v(3)));
        assert_eq!(round_robin_testdouble(&set, Round(3)), Some(v(1)));
    }

    #[test]
    fn empty_set_is_none() {
        assert!(round_robin_testdouble(&[], Round::ZERO).is_none());
    }
}
