//! CI regression guardrail (`stress.perf_regression_ci`).
//!
//! Baselines from this machine, 2026-08-31 (debug `cargo test`, n=48 STM):
//! - STM low-contention ≈ 230–430 TPS (varies with load; dual seq+STM apply).
//! - Compose p99 block interval ≈ **1243 ms** (18s window, 16 intervals).
//!
//! Fail if STM low TPS drops more than **30%** below 200 TPS (floor **140**),
//! or compose p99 exceeds **8000 ms** (§10 finality <5s; 8s slack for Colima
//! jitter). The synthetic-fail unit test injects TPS=1 and p99=50s so the
//! guardrail is proven to trip.

use crate::consensus::ConsensusReport;
use crate::throughput::ThroughputReport;

/// Recorded 2026-08-31 (this machine). STM 48 independent transfers.
pub const BASELINE_STM_LOW_TPS: f64 = 200.0;
/// 30% drop allowed (harness/noise); below this is a regression.
pub const STM_LOW_TPS_FLOOR: f64 = BASELINE_STM_LOW_TPS * 0.70;
/// Compose p99 block interval ceiling (ms).
pub const P99_BLOCK_MS_MAX: u128 = 8_000;

/// Guardrail outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuardResult {
    /// Within tolerance.
    Pass,
    /// Throughput or latency regression.
    Fail(&'static str),
}

/// Check STM + optional consensus report.
pub fn evaluate(tp: &ThroughputReport, cons: Option<&ConsensusReport>) -> GuardResult {
    if tp.stm_low_tps < STM_LOW_TPS_FLOOR {
        return GuardResult::Fail("stm_low_tps below floor");
    }
    if let Some(c) = cons {
        if c.n_intervals > 0 && c.p99_ms > P99_BLOCK_MS_MAX {
            return GuardResult::Fail("p99 block interval above ceiling");
        }
    }
    GuardResult::Pass
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::throughput::run_throughput;

    #[test]
    fn current_stm_meets_baseline_floor() {
        let tp = run_throughput(48, false);
        eprintln!(
            "stress.perf_regression_ci STM low={:.0} floor={:.0} hot={:.0}",
            tp.stm_low_tps, STM_LOW_TPS_FLOOR, tp.stm_hot_tps
        );
        assert_eq!(evaluate(&tp, None), GuardResult::Pass);
    }

    #[test]
    fn synthetic_throttle_fails_guardrail() {
        let tp = ThroughputReport {
            stm_low_tps: 1.0,
            stm_hot_tps: 1.0,
            compose_tps: Some(0.1),
            n: 48,
        };
        assert_eq!(
            evaluate(&tp, None),
            GuardResult::Fail("stm_low_tps below floor")
        );
        let cons = ConsensusReport {
            n_intervals: 4,
            p50_ms: 1_000,
            p95_ms: 2_000,
            p99_ms: 50_000,
            time_to_first_ms: 2_000,
            commit_lines: 10,
        };
        let tp_ok = run_throughput(16, false);
        assert_eq!(
            evaluate(&tp_ok, Some(&cons)),
            GuardResult::Fail("p99 block interval above ceiling")
        );
    }
}
