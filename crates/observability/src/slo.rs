//! SLO thresholds from architecture.md §10.
//!
//! Contract: `obs.slo_definitions`.
//!
//! - Block time **1–2 s** (`BLOCK_TIME_MIN_MS`..=`BLOCK_TIME_MAX_MS`)
//! - Time to finality **< 5 s** (`FINALITY_MAX_MS`)
//!
//! Evaluated against [`crate::prometheus::Metrics`] produced by
//! `obs.prometheus_exporter`. Numbers are cross-checked against
//! `mvp.finality_lan` (`crates/node/tests/finality.rs`), which asserts LAN
//! intervals in 800–2500 ms and always `< 5000` ms.

use crate::prometheus::Metrics;

/// Architecture.md §10 block-time lower bound (ms).
pub const BLOCK_TIME_MIN_MS: u64 = 1_000;
/// Architecture.md §10 block-time upper bound (ms).
pub const BLOCK_TIME_MAX_MS: u64 = 2_000;
/// Architecture.md §10 finality target (ms, exclusive).
pub const FINALITY_MAX_MS: u64 = 5_000;

/// One SLO evaluation against exporter gauges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SloReport {
    /// Last `l1_block_interval_ms`.
    pub block_interval_ms: u64,
    /// Last `l1_finality_ms`.
    pub finality_ms: u64,
    /// True if interval is outside 1–2s (zero = “no sample yet”, not a breach).
    pub block_time_breached: bool,
    /// True if finality ≥ 5s.
    pub finality_breached: bool,
}

impl SloReport {
    /// Either SLO is failing.
    pub fn any_breach(&self) -> bool {
        self.block_time_breached || self.finality_breached
    }
}

/// Evaluate current exporter gauges. Contract: `obs.slo_definitions`.
pub fn evaluate(metrics: &Metrics) -> SloReport {
    let block_interval_ms = metrics.block_interval_ms();
    let finality_ms = metrics.finality_ms();
    let block_time_breached = block_interval_ms != 0
        && !(BLOCK_TIME_MIN_MS..=BLOCK_TIME_MAX_MS).contains(&block_interval_ms);
    let finality_breached = finality_ms >= FINALITY_MAX_MS;
    SloReport {
        block_interval_ms,
        finality_ms,
        block_time_breached,
        finality_breached,
    }
}

/// LAN jitter band used by `mvp.finality_lan` (not the spec target itself).
pub const LAN_INTERVAL_MIN_MS: u64 = 800;
/// LAN jitter upper bound in `mvp.finality_lan`.
pub const LAN_INTERVAL_MAX_MS: u64 = 2_500;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prometheus::Metrics;

    #[test]
    fn eight_second_finality_is_a_breach() {
        let m = Metrics::new();
        m.record_timings_ms(1_500, 8_000);
        let r = evaluate(&m);
        assert!(r.finality_breached);
        assert!(!r.block_time_breached);
        assert!(r.any_breach());
        assert!(m.render().contains("l1_finality_ms 8000"));
    }

    #[test]
    fn in_target_sample_is_clean() {
        let m = Metrics::new();
        m.record_timings_ms(1_200, 1_200);
        let r = evaluate(&m);
        assert!(!r.any_breach());
    }

    #[test]
    fn finality_lan_source_shares_the_five_second_cap() {
        let src = include_str!("../../node/tests/finality.rs");
        assert!(src.contains("mvp.finality_lan"));
        assert!(src.contains("5000"));
        assert!(src.contains("800") && src.contains("2500"));
        assert_eq!(FINALITY_MAX_MS, 5_000);
        assert_eq!(LAN_INTERVAL_MIN_MS, 800);
        assert_eq!(LAN_INTERVAL_MAX_MS, 2_500);
    }
}
