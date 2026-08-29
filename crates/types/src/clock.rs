//! Injected clocks so consensus timeouts are deterministic in tests.
//!
//! See architecture.md §2 (round timeouts) and development-plan.md Tier 0.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Millisecond timestamps used by consensus timeouts and block `timestamp`.
pub trait Clock: Send + Sync {
    /// Current unix time in milliseconds.
    fn now_millis(&self) -> u64;
}

/// Wall-clock clock for production nodes.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_millis() as u64
    }
}

/// Manually advanceable clock for tests (development-plan.md Tier 0).
#[derive(Debug)]
pub struct TestClock {
    millis: AtomicU64,
}

impl TestClock {
    /// Start at `millis`.
    pub fn new(millis: u64) -> Self {
        Self {
            millis: AtomicU64::new(millis),
        }
    }

    /// Advance the clock by `delta` milliseconds.
    pub fn advance(&self, delta: u64) {
        self.millis.fetch_add(delta, Ordering::SeqCst);
    }

    /// Set the clock to an absolute millisecond value.
    pub fn set(&self, millis: u64) {
        self.millis.store(millis, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now_millis(&self) -> u64 {
        self.millis.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clock_advances() {
        let clock = TestClock::new(1_000);
        assert_eq!(clock.now_millis(), 1_000);
        clock.advance(50);
        assert_eq!(clock.now_millis(), 1_050);
        clock.set(9);
        assert_eq!(clock.now_millis(), 9);
    }

    #[test]
    fn system_clock_is_nonzero() {
        let t = SystemClock.now_millis();
        assert!(t > 1_600_000_000_000, "got {t}");
    }
}
