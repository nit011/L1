//! Block header timestamp bounds (development-plan.md §1; architecture.md §2.4).
//!
//! Implementation is [`types::header::timestamp_in_bounds`] so header construction
//! and consensus share one function.

use types::header::timestamp_in_bounds as bounds;
use types::{Clock, Height};

/// Whether `proposed_ms` is valid for `height` given the previous header time.
///
/// Contract: `header.timestamp.bounds`.
pub fn timestamp_in_bounds<C: Clock>(
    clock: &C,
    height: Height,
    prev_timestamp_ms: u64,
    proposed_ms: u64,
    max_drift_ms: u64,
) -> bool {
    bounds(clock, height, prev_timestamp_ms, proposed_ms, max_drift_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{TestClock, MAX_TIMESTAMP_DRIFT_MS};

    #[test]
    fn accept_within_bounds() {
        let clock = TestClock::new(1_000_000);
        assert!(timestamp_in_bounds(
            &clock,
            Height(1),
            900_000,
            1_000_100,
            MAX_TIMESTAMP_DRIFT_MS
        ));
    }

    #[test]
    fn reject_before_previous() {
        let clock = TestClock::new(1_000_000);
        assert!(!timestamp_in_bounds(
            &clock,
            Height(2),
            500_000,
            499_999,
            MAX_TIMESTAMP_DRIFT_MS
        ));
    }

    #[test]
    fn reject_too_far_in_future() {
        let clock = TestClock::new(1_000_000);
        let proposed = 1_000_000 + MAX_TIMESTAMP_DRIFT_MS + 1;
        assert!(!timestamp_in_bounds(
            &clock,
            Height(1),
            1,
            proposed,
            MAX_TIMESTAMP_DRIFT_MS
        ));
        let on_boundary = 1_000_000 + MAX_TIMESTAMP_DRIFT_MS;
        assert!(timestamp_in_bounds(
            &clock,
            Height(1),
            1,
            on_boundary,
            MAX_TIMESTAMP_DRIFT_MS
        ));
    }

    #[test]
    fn genesis_ignores_prev() {
        let clock = TestClock::new(50_000);
        assert!(timestamp_in_bounds(
            &clock,
            Height::GENESIS,
            99_999,
            40_000,
            MAX_TIMESTAMP_DRIFT_MS
        ));
    }
}
