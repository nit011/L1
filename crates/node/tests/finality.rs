//! LAN finality vs architecture.md §10 (block time 1–2s, finality < 5s).
//!
//! Contract: `mvp.finality_lan`. Numbers are recorded, not rounded.

mod common;

use common::start_validators;
use std::time::{Duration, Instant};

fn tmp() -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "l1-finality-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

struct RunStats {
    time_to_first_commit_ms: u128,
    block_intervals_ms: Vec<u128>,
}

fn one_run(label: usize) -> RunStats {
    let root = tmp().join(format!("r{label}"));
    let t0 = Instant::now();
    let cluster = start_validators(&root, 4);
    let deadline = Instant::now() + Duration::from_secs(50);
    let mut last = None;
    let mut marks: Vec<(u64, u128)> = Vec::new();
    loop {
        if let Some(h) = common::read_tip(&cluster.dirs[0]) {
            if last != Some(h) {
                marks.push((h, t0.elapsed().as_millis()));
                last = Some(h);
            }
            if h >= 2 {
                break;
            }
        }
        if Instant::now() > deadline {
            panic!(
                "finality run {label} events:\n{}",
                common::events(&cluster.dirs[0])
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let time_to_first = marks.first().map(|m| m.1).unwrap_or(0);
    let mut intervals = Vec::new();
    for w in marks.windows(2) {
        intervals.push(w[1].1.saturating_sub(w[0].1));
    }
    eprintln!(
        "mvp.finality_lan run {label}: time_to_first_commit_ms={time_to_first} intervals_ms={intervals:?} marks={marks:?}"
    );
    RunStats {
        time_to_first_commit_ms: time_to_first,
        block_intervals_ms: intervals,
    }
}

#[test]
fn finality_lan_three_runs_against_architecture_targets() {
    let mut all = Vec::new();
    for i in 0..3 {
        all.push(one_run(i));
    }
    for (i, s) in all.iter().enumerate() {
        assert!(
            !s.block_intervals_ms.is_empty(),
            "run {i} produced no block intervals"
        );
        for (j, ms) in s.block_intervals_ms.iter().enumerate() {
            assert!(
                *ms >= 800 && *ms <= 2500,
                "run {i} interval {j} = {ms} ms (target block time 1–2s; allowing 0.8–2.5s LAN jitter)"
            );
            assert!(
                *ms < 5000,
                "run {i} interval {j} = {ms} ms exceeds 5s finality target"
            );
        }
        assert!(
            s.time_to_first_commit_ms < 45_000,
            "run {i} mesh warmup {} ms (not protocol TTF; first commit includes gossip join)",
            s.time_to_first_commit_ms
        );
    }
}
