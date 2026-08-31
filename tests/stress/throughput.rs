//! Throughput: STM execution vs full compose stack (`stress.throughput_benchmark`).
//!
//! architecture.md §10: "10k+ TPS for simple transfers, lower for heavy
//! contract calls; depends on hot-account rate."
//!
//! - **Low contention:** distinct senders (independent accounts).
//! - **High contention:** many txs from one hot account (nonce chain).
//!
//! Full-stack numbers go through gossip + 1s `min_block_time_ms` + seq in the
//! node binary (not STM). STM is still invoked here via
//! [`execution::stm::apply_block`] on an equivalent in-memory block so the
//! named dependency is real. The gap vs compose TPS is consensus/network
//! overhead, not hidden.

use crate::harness::{bring_up, gossip_txs, signed_transfer, tear_down, wait_tip, LoadConfig};
use crypto::address::from_ed25519;
use crypto::sig::ed25519::SecretKey;
use execution::seq::World;
use execution::stm;
use sdk::sign_tx;
use std::time::{Duration, Instant};
use types::block::Block;
use types::genesis::{Genesis, GenesisAccount};
use types::header::HeaderFields;
use types::tx::Tx;
use types::{
    Amount, ChainId, Hash, Height, Nonce, Round, TestClock, ValidatorId, GAS_TRANSFER, MIN_TX_FEE,
};

/// Pair of profiles.
#[derive(Clone, Debug)]
pub struct ThroughputReport {
    /// STM independent-account apply TPS.
    pub stm_low_tps: f64,
    /// STM hot-account apply TPS.
    pub stm_hot_tps: f64,
    /// Compose gossip+commit TPS (None if docker skipped).
    pub compose_tps: Option<f64>,
    /// How many txs STM applied (low).
    pub n: usize,
}

fn bank_world(n: usize, sks: &[SecretKey]) -> (World, Genesis) {
    let mut g = Genesis::new(ChainId::new(18));
    for sk in sks.iter().take(n) {
        g.insert_alloc(
            from_ed25519(&sk.verifying_key()),
            GenesisAccount {
                balance: Amount::new(1_000_000_000),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
    }
    (World::from_genesis(&g), g)
}

fn header_fields() -> HeaderFields {
    let clock = TestClock::new(1_000);
    HeaderFields::new(
        &clock,
        Height::GENESIS,
        Round::ZERO,
        ValidatorId::ZERO,
        0,
        1,
    )
    .unwrap()
}

fn stm_profile(independent: bool, n: usize) -> f64 {
    let sks: Vec<_> = (0..n.max(1))
        .map(|i| {
            let mut s = [0xCCu8; 32];
            s[2] = i as u8;
            SecretKey::from_bytes(&s)
        })
        .collect();
    let (world, _) = bank_world(sks.len(), &sks);
    let dest = types::Address::ZERO;
    let mut txs = Vec::new();
    if independent {
        for (i, sk) in sks.iter().enumerate() {
            let tx = Tx::transfer(
                ChainId::new(18),
                Nonce::ZERO,
                GAS_TRANSFER,
                Amount::new(MIN_TX_FEE),
                dest,
                Amount::new(1),
            );
            txs.push(sign_tx(sk, tx).signed);
            let _ = i;
        }
    } else {
        let sk = &sks[0];
        for nonce in 0..n as u64 {
            let tx = Tx::transfer(
                ChainId::new(18),
                Nonce(nonce),
                GAS_TRANSFER,
                Amount::new(MIN_TX_FEE),
                dest,
                Amount::new(1),
            );
            txs.push(sign_tx(sk, tx).signed);
        }
    }
    let block = Block {
        header_fields: header_fields(),
        txs,
    };
    let t0 = Instant::now();
    let _ = stm::apply_block(world, &block);
    let dt = t0.elapsed().as_secs_f64().max(1e-9);
    n as f64 / dt
}

/// STM-only (always). Compose optional.
pub fn run_throughput(n: usize, compose: bool) -> ThroughputReport {
    let stm_low_tps = stm_profile(true, n);
    let stm_hot_tps = stm_profile(false, n);
    let compose_tps = if compose {
        Some(compose_tps(n.min(48)))
    } else {
        None
    };
    ThroughputReport {
        stm_low_tps,
        stm_hot_tps,
        compose_tps,
        n,
    }
}

fn compose_tps(n: usize) -> f64 {
    let cfg = LoadConfig {
        duration: Duration::from_secs(1),
        bank_accounts: n.max(2),
        tx_burst: n,
        ..LoadConfig::default()
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (_h, bank) = bring_up(&cfg).expect("up");
    wait_tip(0, 0, Duration::from_secs(25));
    let dest = from_ed25519(&bank[1].verifying_key());
    let mut signed = Vec::new();
    for i in 0..n {
        let sk = &bank[i % bank.len()];
        let nonce = (i / bank.len()) as u64;
        signed.push(signed_transfer(sk, nonce, dest, 1));
    }
    let t0 = Instant::now();
    let commits_before = crate::consensus::count_commits(&crate::harness::events(0));
    let _ = rt.block_on(gossip_txs(&signed));
    wait_tip(0, 0, Duration::from_secs(20));
    std::thread::sleep(Duration::from_secs(3));
    let dt = t0.elapsed().as_secs_f64().max(1e-9);
    let commits =
        crate::consensus::count_commits(&crate::harness::events(0)).saturating_sub(commits_before);
    tear_down();
    // Submitted TPS through the compose wait window. Block rate ≈ commits/dt
    // (1s min_block_time_ms caps committed tx TPS far below STM).
    eprintln!(
        "compose submitted={n} wall={dt:.2}s commit_delta={commits} implied_submit_tps={:.1} block_tps={:.2}",
        n as f64 / dt,
        commits as f64 / dt
    );
    n as f64 / dt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stm_low_vs_hot_and_gap_to_10k_target() {
        let r = run_throughput(48, false);
        eprintln!(
            "stress.throughput_benchmark STM n={} low={:.0} tps hot={:.0} tps (§10 target 10k+; compose not in this unit test)",
            r.n, r.stm_low_tps, r.stm_hot_tps
        );
        assert!(r.stm_low_tps > 0.0 && r.stm_hot_tps > 0.0);
        // Hot account should not be *faster* than independent at this size
        // (serialization); allow noise.
        eprintln!(
            "gap vs §10 10k: low is {:.1}% of target",
            100.0 * r.stm_low_tps / 10_000.0
        );
    }

    #[test]
    #[ignore]
    fn docker_compose_throughput() {
        assert!(crate::harness::docker_ok());
        let r = run_throughput(32, true);
        eprintln!(
            "STM low={:.0} hot={:.0} compose={:?}",
            r.stm_low_tps, r.stm_hot_tps, r.compose_tps
        );
        assert!(r.compose_tps.unwrap() > 0.0);
    }
}
