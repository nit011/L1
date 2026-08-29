//! Per-round timeout schedule and clock binding (architecture.md §2.2).
//!
//! Timeouts grow additively per round so a partially synchronous network
//! eventually delivers messages (Tendermint-style). Values are taken from
//! [`types::spec`] / [`types::ParamsRegistry`], not magic numbers in this file.
//! Units are milliseconds as defined by [`types::Clock::now_millis`].

use types::{
    Clock, ParamId, ParamsRegistry, Round, TIMEOUT_DELTA_MS, TIMEOUT_PRECOMMIT_MS,
    TIMEOUT_PREVOTE_MS, TIMEOUT_PROPOSE_MS,
};

/// Consensus step whose timer is running (not the BFT state machine).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeoutStep {
    /// Propose timeout (architecture.md §2.2 step 1).
    Propose,
    /// Prevote timeout (architecture.md §2.2 step 2).
    Prevote,
    /// Precommit timeout (architecture.md §2.2 step 3).
    Precommit,
}

/// Per-round timeout schedule. Contract: `cons.timeout.config`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeoutConfig {
    propose_ms: u64,
    prevote_ms: u64,
    precommit_ms: u64,
    delta_ms: u64,
}

impl TimeoutConfig {
    /// Defaults from [`types::spec`] constants.
    pub fn from_spec() -> Self {
        Self {
            propose_ms: TIMEOUT_PROPOSE_MS,
            prevote_ms: TIMEOUT_PREVOTE_MS,
            precommit_ms: TIMEOUT_PRECOMMIT_MS,
            delta_ms: TIMEOUT_DELTA_MS,
        }
    }

    /// Load from a [`ParamsRegistry`] (falls back to spec if a key is missing).
    pub fn from_params(params: &ParamsRegistry) -> Self {
        Self {
            propose_ms: params
                .get(ParamId::TimeoutProposeMs)
                .unwrap_or(TIMEOUT_PROPOSE_MS),
            prevote_ms: params
                .get(ParamId::TimeoutPrevoteMs)
                .unwrap_or(TIMEOUT_PREVOTE_MS),
            precommit_ms: params
                .get(ParamId::TimeoutPrecommitMs)
                .unwrap_or(TIMEOUT_PRECOMMIT_MS),
            delta_ms: params
                .get(ParamId::TimeoutDeltaMs)
                .unwrap_or(TIMEOUT_DELTA_MS),
        }
    }

    /// Propose timeout at round 0 (ms).
    pub fn propose_ms(&self) -> u64 {
        self.propose_ms
    }

    /// Prevote timeout at round 0 (ms).
    pub fn prevote_ms(&self) -> u64 {
        self.prevote_ms
    }

    /// Precommit timeout at round 0 (ms).
    pub fn precommit_ms(&self) -> u64 {
        self.precommit_ms
    }

    /// Per-round additive delta (ms).
    pub fn delta_ms(&self) -> u64 {
        self.delta_ms
    }

    /// Duration for `step` at `round`: `base + round * delta`.
    pub fn duration_ms(&self, step: TimeoutStep, round: Round) -> u64 {
        let base = match step {
            TimeoutStep::Propose => self.propose_ms,
            TimeoutStep::Prevote => self.prevote_ms,
            TimeoutStep::Precommit => self.precommit_ms,
        };
        base.saturating_add(u64::from(round.0).saturating_mul(self.delta_ms))
    }

    /// Snapshot current time in the same millisecond units as this schedule.
    /// Calls [`Clock::now_millis`] (`clock.injected`).
    pub fn now_ms<C: Clock>(clock: &C) -> u64 {
        clock.now_millis()
    }
}

/// Clock bound to a timeout config so the BFT engine never reads `SystemTime`.
/// Contract: `cons.clock.bind`.
pub struct BoundClock<C: Clock> {
    clock: C,
    config: TimeoutConfig,
}

impl<C: Clock> BoundClock<C> {
    /// Bind `clock` to `config`.
    pub fn new(clock: C, config: TimeoutConfig) -> Self {
        Self { clock, config }
    }

    /// Current time from the injected clock.
    pub fn now_ms(&self) -> u64 {
        TimeoutConfig::now_ms(&self.clock)
    }

    /// Whether the timer started at `started_at_ms` has elapsed for this step/round.
    ///
    /// Fires at the first instant `now >= started_at + duration` (inclusive boundary).
    pub fn elapsed(&self, step: TimeoutStep, round: Round, started_at_ms: u64) -> bool {
        let due = started_at_ms.saturating_add(self.config.duration_ms(step, round));
        self.clock.now_millis() >= due
    }

    /// Borrow the schedule.
    pub fn config(&self) -> &TimeoutConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::TestClock;

    #[test]
    fn spec_and_params_match() {
        let a = TimeoutConfig::from_spec();
        let b = TimeoutConfig::from_params(&ParamsRegistry::new());
        assert_eq!(a, b);
        assert_eq!(
            a.duration_ms(TimeoutStep::Propose, Round::ZERO),
            TIMEOUT_PROPOSE_MS
        );
        let later = a.duration_ms(TimeoutStep::Propose, Round(3));
        assert_eq!(later, TIMEOUT_PROPOSE_MS + 3 * TIMEOUT_DELTA_MS);
        assert!(later > a.duration_ms(TimeoutStep::Propose, Round::ZERO));
        let clock = TestClock::new(42);
        assert_eq!(TimeoutConfig::now_ms(&clock), 42);
    }

    #[test]
    fn bound_clock_fires_exactly_at_boundary() {
        let clock = TestClock::new(10_000);
        let cfg = TimeoutConfig::from_spec();
        let dur = cfg.duration_ms(TimeoutStep::Prevote, Round::ZERO);
        let bound = BoundClock::new(clock, cfg);
        let start = bound.now_ms();
        assert!(!bound.elapsed(TimeoutStep::Prevote, Round::ZERO, start));
        bound.clock.advance(dur.saturating_sub(1));
        assert!(!bound.elapsed(TimeoutStep::Prevote, Round::ZERO, start));
        bound.clock.advance(1);
        assert!(bound.elapsed(TimeoutStep::Prevote, Round::ZERO, start));
        bound.clock.advance(1);
        assert!(bound.elapsed(TimeoutStep::Prevote, Round::ZERO, start));
    }
}
