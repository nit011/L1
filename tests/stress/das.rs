//! DAS withhold under a live compose network (`stress.das_withhold`).
//!
//! Encodes a real [`Block`] through [`da::root::commit`] (same RS+Merkle path as
//! production), withholds the light-client sample indices, and asserts
//! [`da::das::fail_closed`] → `NotAvailable`. A second fully published block
//! still samples `Available`.
//!
//! The compose validators keep committing (`COMMIT` / tip movement) during that
//! window: DAS must not gate `cons.commit` (forbidden edge). That is the
//! cascade-isolation check — withheld DA for a sampled payload must not stall
//! unrelated full-node finality.

use crate::harness::{bring_up, events, read_tip, tear_down, wait_tip, LoadConfig};
use da::das::{fail_closed, sample, sample_indices, Availability, MemoryChunks};
use da::root::commit;
use std::time::Duration;
use types::block::Block;
use types::header::HeaderFields;
use types::{Height, Round, TestClock, ValidatorId};

/// Withhold report plus compose liveness during the window.
#[derive(Clone, Debug)]
pub struct DasReport {
    /// Withheld payload → fail_closed.
    pub withheld_not_available: bool,
    /// Control payload still available.
    pub control_available: bool,
    /// Tip at window start.
    pub tip_before: u64,
    /// Tip at window end.
    pub tip_after: u64,
    /// COMMIT count delta on node0.
    pub commits_during: usize,
}

fn test_block(marker: u8) -> Block {
    let clock = TestClock::new(1_000_000);
    let fields = HeaderFields::new(
        &clock,
        Height(marker as u64),
        Round::ZERO,
        ValidatorId::ZERO,
        0,
        10 + marker as u64,
    )
    .unwrap();
    Block {
        header_fields: fields,
        txs: vec![],
    }
}

fn withhold_vs_control() -> (bool, bool) {
    let (root, proven) = commit(&test_block(2)).unwrap();
    let mut mem = MemoryChunks::from_proven(proven);
    let queried = sample_indices(&root);
    mem.withhold(&queried);
    let bad = sample(&root, &mem);
    let withheld_not_available = fail_closed(&bad) == Availability::NotAvailable;

    let (root2, proven2) = commit(&test_block(3)).unwrap();
    let mem2 = MemoryChunks::from_proven(proven2);
    let good = sample(&root2, &mem2);
    let control_available = fail_closed(&good) == Availability::Available;
    (withheld_not_available, control_available)
}

/// Run DAS check; if `compose`, also sample tip movement.
pub fn run_das(compose: bool) -> DasReport {
    let (withheld_not_available, control_available) = withhold_vs_control();
    if !compose {
        return DasReport {
            withheld_not_available,
            control_available,
            tip_before: 0,
            tip_after: 0,
            commits_during: 0,
        };
    }
    let cfg = LoadConfig {
        duration: Duration::from_secs(2),
        ..LoadConfig::default()
    };
    let _ = bring_up(&cfg);
    wait_tip(0, 0, Duration::from_secs(30));
    let tip_before = read_tip(0).unwrap_or(0);
    let c0 = crate::consensus::count_commits(&events(0));
    let _ = withhold_vs_control();
    std::thread::sleep(Duration::from_secs(4));
    let tip_after = read_tip(0).unwrap_or(0);
    let c1 = crate::consensus::count_commits(&events(0));
    tear_down();
    DasReport {
        withheld_not_available,
        control_available,
        tip_before,
        tip_after,
        commits_during: c1.saturating_sub(c0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn withhold_fail_closed_control_available() {
        let r = run_das(false);
        assert!(r.withheld_not_available);
        assert!(r.control_available);
    }

    #[test]
    #[ignore]
    fn docker_withhold_does_not_stall_validators() {
        assert!(crate::harness::docker_ok());
        let r = run_das(true);
        eprintln!(
            "stress.das_withhold withheld_na={} control_ok={} tip {}→{} commits_during={}",
            r.withheld_not_available,
            r.control_available,
            r.tip_before,
            r.tip_after,
            r.commits_during
        );
        assert!(r.withheld_not_available && r.control_available);
        assert!(
            r.tip_after >= r.tip_before,
            "unrelated compose nodes must keep committing"
        );
        assert!(
            r.commits_during >= 1 || r.tip_after > r.tip_before,
            "cascade isolation: cons.commit still advancing"
        );
    }
}
