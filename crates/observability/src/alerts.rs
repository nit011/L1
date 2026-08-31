//! Alertmanager-style rules over SLO evaluations (architecture.md §10).
//!
//! Contract: `obs.alert_rules`. Observes [`crate::slo::evaluate`]; does **not**
//! pause the chain or flip config (Tier 17). A rule stays silent unless the
//! SLO stays breached for `for_samples` consecutive evaluations.

use crate::prometheus::Metrics;
use crate::slo::{evaluate, SloReport};

/// One alerting rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertRule {
    /// Consecutive SLO-breach samples required before firing.
    pub for_samples: usize,
}

impl Default for AlertRule {
    fn default() -> Self {
        Self { for_samples: 3 }
    }
}

/// Current alert state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertState {
    /// Not firing.
    Silent,
    /// Sustained SLO breach.
    Firing,
}

/// Walk a history of SLO reports (oldest first).
pub fn evaluate_history(history: &[SloReport], rule: &AlertRule) -> AlertState {
    let need = rule.for_samples.max(1);
    if history.len() < need {
        return AlertState::Silent;
    }
    let tail = &history[history.len() - need..];
    if tail.iter().all(|r| r.any_breach()) {
        AlertState::Firing
    } else {
        AlertState::Silent
    }
}

/// Evaluate the latest scrape, append-style: pass prior reports plus a new
/// sample from `metrics`.
pub fn evaluate_with_metrics(
    prior: &[SloReport],
    metrics: &Metrics,
    rule: &AlertRule,
) -> (SloReport, AlertState) {
    let mut hist = prior.to_vec();
    let next = evaluate(metrics);
    hist.push(next.clone());
    let state = evaluate_history(&hist, rule);
    (next, state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prometheus::Metrics;
    use crate::slo::evaluate;

    #[test]
    fn blip_stays_silent_sustained_fires() {
        let rule = AlertRule { for_samples: 3 };
        let ok = {
            let m = Metrics::new();
            m.record_timings_ms(1_200, 1_200);
            evaluate(&m)
        };
        let bad = {
            let m = Metrics::new();
            m.record_timings_ms(1_500, 8_000);
            evaluate(&m)
        };
        assert_eq!(
            evaluate_history(&[ok.clone(), bad.clone(), ok.clone()], &rule),
            AlertState::Silent
        );
        assert_eq!(
            evaluate_history(&[bad.clone(), bad.clone(), bad.clone()], &rule),
            AlertState::Firing
        );
        assert_eq!(
            evaluate_history(&[bad.clone(), bad.clone()], &rule),
            AlertState::Silent
        );
    }

    #[test]
    fn rule_uses_slo_definitions() {
        let m = Metrics::new();
        m.record_timings_ms(9_000, 9_000);
        let rule = AlertRule { for_samples: 1 };
        let (r, st) = evaluate_with_metrics(&[], &m, &rule);
        assert!(r.any_breach());
        assert_eq!(st, AlertState::Firing);
    }
}
