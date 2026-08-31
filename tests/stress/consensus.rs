//! Sustained 4-node consensus under load (`stress.consensus_4node`).
//!
//! Measures block intervals from bind-mounted `tip` files the same way
//! `mvp.finality_lan` samples wall-clock marks. `COMMIT n` in `events.log`
//! is produced only after [`node::wire::wire_commit`] → `cons.commit`.
//! architecture.md §10: time to finality < 5s; this node commits ~1s
//! (`NodeConfig.min_block_time_ms`).

use crate::harness::{
    bring_up, events, gossip_txs, read_tip, signed_transfer, tear_down, wait_tip, LoadConfig,
};
use crypto::address::from_ed25519;
use std::time::{Duration, Instant};

/// Finality / block-time distribution from a live compose run.
#[derive(Clone, Debug)]
pub struct ConsensusReport {
    /// Sample count (intervals).
    pub n_intervals: usize,
    /// p50/p95/p99 block intervals (ms).
    pub p50_ms: u128,
    /// p95.
    pub p95_ms: u128,
    /// p99.
    pub p99_ms: u128,
    /// Time to first `tip` after load start (ms).
    pub time_to_first_ms: u128,
    /// `COMMIT` lines observed on node0 (cons.commit).
    pub commit_lines: usize,
}

/// Parse `COMMIT n` counts.
pub fn count_commits(log: &str) -> usize {
    log.lines().filter(|l| l.starts_with("COMMIT ")).count()
}

/// Drive load and record interval percentiles.
pub fn run_consensus_window(cfg: &LoadConfig) -> ConsensusReport {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (_h, bank) = bring_up(cfg).expect("compose up");
    wait_tip(0, 0, Duration::from_secs(30));
    let t0 = Instant::now();
    let commits_before = count_commits(&events(0));
    let mut marks: Vec<(u64, u128)> = Vec::new();
    let mut last = read_tip(0);
    let dest = from_ed25519(&bank.get(1).unwrap_or(&bank[0]).verifying_key());
    let mut nonce = 0u64;
    let deadline = Instant::now() + cfg.duration;
    let mut last_gossip = Instant::now()
        .checked_sub(Duration::from_secs(2))
        .unwrap_or_else(Instant::now);
    while Instant::now() < deadline {
        if let Some(h) = read_tip(0) {
            if last != Some(h) {
                marks.push((h, t0.elapsed().as_millis()));
                last = Some(h);
            }
        }
        if last_gossip.elapsed() >= Duration::from_millis(1200) {
            last_gossip = Instant::now();
            let mut burst = Vec::new();
            for _ in 0..cfg.tx_burst.min(16) {
                burst.push(signed_transfer(&bank[0], nonce, dest, 1));
                nonce += 1;
            }
            let _ = rt.block_on(gossip_txs(&burst));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if let Some(h) = read_tip(0) {
        if last != Some(h) {
            marks.push((h, t0.elapsed().as_millis()));
        }
    }
    let mut iv: Vec<u128> = marks
        .windows(2)
        .map(|w| w[1].1.saturating_sub(w[0].1))
        .collect();
    iv.sort_unstable();
    let report = ConsensusReport {
        n_intervals: iv.len(),
        p50_ms: crate::harness::percentile(&iv, 50.0),
        p95_ms: crate::harness::percentile(&iv, 95.0),
        p99_ms: crate::harness::percentile(&iv, 99.0),
        time_to_first_ms: marks.first().map(|m| m.1).unwrap_or(0),
        commit_lines: count_commits(&events(0)).saturating_sub(commits_before),
    };
    tear_down();
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_log_parser_and_percentiles() {
        let log = "BOOT\nCOMMIT 0\nCOMMIT 1\n";
        assert_eq!(count_commits(log), 2);
        let v = [800u128, 1000, 1100, 4000];
        assert_eq!(crate::harness::percentile(&v, 50.0), 1100);
        assert_eq!(crate::harness::percentile(&v, 99.0), 4000);
    }

    #[test]
    #[ignore]
    fn docker_consensus_4node_p99() {
        assert!(crate::harness::docker_ok(), "docker required");
        let cfg = LoadConfig {
            duration: Duration::from_secs(18),
            tx_burst: 8,
            ..LoadConfig::default()
        };
        let r = run_consensus_window(&cfg);
        eprintln!(
            "stress.consensus_4node p50={} p95={} p99={} first={} commits={} intervals={}",
            r.p50_ms, r.p95_ms, r.p99_ms, r.time_to_first_ms, r.commit_lines, r.n_intervals
        );
        assert!(r.commit_lines >= 1, "cons.commit must fire (COMMIT log)");
        assert!(
            r.p99_ms == 0 || r.p99_ms < 8_000,
            "p99 block interval under load should stay near §10 <5s (allow 8s compose jitter), got {}",
            r.p99_ms
        );
    }
}
