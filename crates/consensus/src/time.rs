//! Block header timestamp bounds (development-plan.md §1; architecture.md §2.4).
//!
//! A manipulable timestamp is a grinding surface if it ever seeds leader
//! election. VRF seeds must not use this value; this check still rejects
//! timestamps that go backwards or lie too far in the future of the injected clock.

use types::{Clock, Height};

/// Whether `proposed_ms` is valid for `height` given the previous header time.
///
/// Rules (unix milliseconds, same unit as [`Clock::now_millis`]):
/// - `proposed_ms` must not be strictly before `prev_timestamp_ms` (except genesis,
///   where there is no previous block and `prev_timestamp_ms` is ignored).
/// - `proposed_ms` must not exceed `clock.now_millis() + max_drift_ms`.
///
/// Contract: `header.timestamp.bounds`.
pub fn timestamp_in_bounds<C: Clock>(
    clock: &C,
    height: Height,
    prev_timestamp_ms: u64,
    proposed_ms: u64,
    max_drift_ms: u64,
) -> bool {
    if height != Height::GENESIS && proposed_ms < prev_timestamp_ms {
        return false;
    }
    let now = clock.now_millis();
    let max_future = now.saturating_add(max_drift_ms);
    proposed_ms <= max_future
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
