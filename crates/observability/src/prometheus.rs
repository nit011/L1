//! Prometheus-format gauges/counters (architecture.md §10).
//!
//! Contract: `obs.prometheus_exporter`.
//!
//! Passive observation of:
//! - [`consensus::steps::commit`] (`cons.commit`) — success/failure, block
//!   interval, finality sample
//! - [`network::gossip::mesh_config`] (`gossip.mesh`) — mesh degree, topic count
//! - [`mempool::limits::check_tx_bytes`] / [`mempool::limits::ensure_room`]
//!   (`mempool.size_limits`) — occupancy proxy and full/evict outcomes
//!
//! Recording uses only atomics + `try_lock` and **never** returns an error to
//! the caller. A down exporter (`scrape` fails) does not change commit, gossip,
//! or mempool results.

use consensus::propose::Proposal;
use consensus::safety::CommitLog;
use consensus::steps::{commit, CommitError, Finalized};
use consensus::vote::Vote;
use mempool::limits::{check_tx_bytes, ensure_room};
use mempool::verify::VerifyError;
use mempool::Mempool;
use network::gossip::{all_topics, gossipsub_behaviour, mesh_config};
use network::identity::NodeIdentity;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use types::collections::Map;
use types::tx::SignedTx;
use types::{ValidatorId, VotingPower};

/// In-process metric set. Contract: `obs.prometheus_exporter`.
#[derive(Debug)]
pub struct Metrics {
    exporter_up: AtomicBool,
    commits_ok: AtomicU64,
    commits_none: AtomicU64,
    commits_err: AtomicU64,
    block_interval_ms: AtomicU64,
    finality_ms: AtomicU64,
    last_commit_ms: Mutex<Option<Instant>>,
    mesh_n: AtomicU64,
    gossip_topics: AtomicU64,
    gossip_msgs: AtomicU64,
    mempool_occupancy: AtomicU64,
    mempool_cap: AtomicU64,
    mempool_full: AtomicU64,
    mempool_evict: AtomicU64,
    rpc_submit_ok: AtomicU64,
    rpc_submit_err: AtomicU64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    /// Exporter starts **up**; tests may flip it down.
    pub fn new() -> Self {
        Self {
            exporter_up: AtomicBool::new(true),
            commits_ok: AtomicU64::new(0),
            commits_none: AtomicU64::new(0),
            commits_err: AtomicU64::new(0),
            block_interval_ms: AtomicU64::new(0),
            finality_ms: AtomicU64::new(0),
            last_commit_ms: Mutex::new(None),
            mesh_n: AtomicU64::new(0),
            gossip_topics: AtomicU64::new(0),
            gossip_msgs: AtomicU64::new(0),
            mempool_occupancy: AtomicU64::new(0),
            mempool_cap: AtomicU64::new(u64::from(types::MEMPOOL_MAX_TXS)),
            mempool_full: AtomicU64::new(0),
            mempool_evict: AtomicU64::new(0),
            rpc_submit_ok: AtomicU64::new(0),
            rpc_submit_err: AtomicU64::new(0),
        }
    }

    /// Simulate an unreachable Prometheus backend.
    pub fn set_exporter_up(&self, up: bool) {
        self.exporter_up.store(up, Ordering::Relaxed);
    }

    /// Whether scrapes should succeed.
    pub fn exporter_up(&self) -> bool {
        self.exporter_up.load(Ordering::Relaxed)
    }

    fn bump(a: &AtomicU64) {
        a.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a `cons.commit` outcome. Never panics the caller on lock poison:
    /// poisoned `last_commit_ms` is skipped.
    pub fn record_commit_result(&self, result: &Result<Option<Finalized>, CommitError>) {
        match result {
            Ok(Some(_)) => Self::bump(&self.commits_ok),
            Ok(None) => Self::bump(&self.commits_none),
            Err(_) => Self::bump(&self.commits_err),
        }
        let now = Instant::now();
        if let Ok(mut g) = self.last_commit_ms.try_lock() {
            if let Some(prev) = *g {
                let ms = now.saturating_duration_since(prev).as_millis() as u64;
                self.block_interval_ms.store(ms, Ordering::Relaxed);
                self.finality_ms.store(ms, Ordering::Relaxed);
            }
            *g = Some(now);
        }
    }

    /// Inject a measured interval (simnet / `mvp.finality_lan` numbers).
    pub fn record_timings_ms(&self, block_interval_ms: u64, finality_ms: u64) {
        self.block_interval_ms
            .store(block_interval_ms, Ordering::Relaxed);
        self.finality_ms.store(finality_ms, Ordering::Relaxed);
        Self::bump(&self.commits_ok);
    }

    /// Mesh snapshot from `gossip.mesh`.
    pub fn record_mesh(&self, mesh_n: u64, topics: u64) {
        self.mesh_n.store(mesh_n, Ordering::Relaxed);
        self.gossip_topics.store(topics, Ordering::Relaxed);
        Self::bump(&self.gossip_msgs);
    }

    /// Mempool occupancy vs `MEMPOOL_MAX_TXS`.
    pub fn record_mempool(&self, occupancy: u64, evicted: bool, full: bool) {
        self.mempool_occupancy.store(occupancy, Ordering::Relaxed);
        if evicted {
            Self::bump(&self.mempool_evict);
        }
        if full {
            Self::bump(&self.mempool_full);
        }
    }

    /// RPC submit outcome (used by `obs.otel_tracing`).
    pub fn record_rpc_submit(&self, ok: bool) {
        if ok {
            Self::bump(&self.rpc_submit_ok);
        } else {
            Self::bump(&self.rpc_submit_err);
        }
    }

    /// Gauges for SLO evaluation.
    pub fn block_interval_ms(&self) -> u64 {
        self.block_interval_ms.load(Ordering::Relaxed)
    }

    /// Last observed finality sample (ms).
    pub fn finality_ms(&self) -> u64 {
        self.finality_ms.load(Ordering::Relaxed)
    }

    /// Successful commits.
    pub fn commits_ok(&self) -> u64 {
        self.commits_ok.load(Ordering::Relaxed)
    }

    /// Mesh target degree.
    pub fn mesh_n(&self) -> u64 {
        self.mesh_n.load(Ordering::Relaxed)
    }

    /// Observed mempool occupancy.
    pub fn mempool_occupancy(&self) -> u64 {
        self.mempool_occupancy.load(Ordering::Relaxed)
    }

    /// Prometheus text exposition. Local snapshot even if the exporter is down.
    pub fn render(&self) -> String {
        let up = if self.exporter_up() { 1 } else { 0 };
        format!(
            "# HELP l1_exporter_up 1 if the scrape endpoint is considered available\n\
             # TYPE l1_exporter_up gauge\n\
             l1_exporter_up {up}\n\
             # HELP l1_commit_success_total cons.commit Ok(Some)\n\
             # TYPE l1_commit_success_total counter\n\
             l1_commit_success_total {}\n\
             # HELP l1_commit_empty_total cons.commit Ok(None) (no QC)\n\
             # TYPE l1_commit_empty_total counter\n\
             l1_commit_empty_total {}\n\
             # HELP l1_commit_failure_total cons.commit Err\n\
             # TYPE l1_commit_failure_total counter\n\
             l1_commit_failure_total {}\n\
             # HELP l1_block_interval_ms last observed commit-to-commit interval (architecture.md §10 1–2s)\n\
             # TYPE l1_block_interval_ms gauge\n\
             l1_block_interval_ms {}\n\
             # HELP l1_finality_ms last observed finality sample (architecture.md §10 <5s)\n\
             # TYPE l1_finality_ms gauge\n\
             l1_finality_ms {}\n\
             # HELP l1_gossip_mesh_n gossip.mesh target degree\n\
             # TYPE l1_gossip_mesh_n gauge\n\
             l1_gossip_mesh_n {}\n\
             # HELP l1_gossip_topics subscribed gossip topics\n\
             # TYPE l1_gossip_topics gauge\n\
             l1_gossip_topics {}\n\
             # HELP l1_gossip_msgs_total observed mesh snapshots\n\
             # TYPE l1_gossip_msgs_total counter\n\
             l1_gossip_msgs_total {}\n\
             # HELP l1_mempool_occupancy txs observed under mempool.size_limits\n\
             # TYPE l1_mempool_occupancy gauge\n\
             l1_mempool_occupancy {}\n\
             # HELP l1_mempool_capacity MEMPOOL_MAX_TXS\n\
             # TYPE l1_mempool_capacity gauge\n\
             l1_mempool_capacity {}\n\
             # HELP l1_mempool_full_total ensure_room MempoolFull\n\
             # TYPE l1_mempool_full_total counter\n\
             l1_mempool_full_total {}\n\
             # HELP l1_mempool_evicted_total successful room-making evictions\n\
             # TYPE l1_mempool_evicted_total counter\n\
             l1_mempool_evicted_total {}\n\
             # HELP l1_rpc_submit_ok_total l1_submitTx success\n\
             # TYPE l1_rpc_submit_ok_total counter\n\
             l1_rpc_submit_ok_total {}\n\
             # HELP l1_rpc_submit_err_total l1_submitTx error\n\
             # TYPE l1_rpc_submit_err_total counter\n\
             l1_rpc_submit_err_total {}\n",
            self.commits_ok.load(Ordering::Relaxed),
            self.commits_none.load(Ordering::Relaxed),
            self.commits_err.load(Ordering::Relaxed),
            self.block_interval_ms(),
            self.finality_ms(),
            self.mesh_n.load(Ordering::Relaxed),
            self.gossip_topics.load(Ordering::Relaxed),
            self.gossip_msgs.load(Ordering::Relaxed),
            self.mempool_occupancy.load(Ordering::Relaxed),
            self.mempool_cap.load(Ordering::Relaxed),
            self.mempool_full.load(Ordering::Relaxed),
            self.mempool_evict.load(Ordering::Relaxed),
            self.rpc_submit_ok.load(Ordering::Relaxed),
            self.rpc_submit_err.load(Ordering::Relaxed),
        )
    }

    /// Scrape endpoint. Fails when the backend is marked down — callers of
    /// commit/gossip/mempool must **not** use this `Result`.
    pub fn scrape(&self) -> Result<String, ExporterDown> {
        if !self.exporter_up() {
            return Err(ExporterDown);
        }
        Ok(self.render())
    }
}

/// Prometheus scrape failed (backend unreachable).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExporterDown;

/// Call the real `cons.commit`, then record. The `Result` is **exactly**
/// `commit`'s — metrics cannot change it.
pub fn observe_commit(
    metrics: &Metrics,
    precommits: &[Vote],
    validators: &Map<ValidatorId, VotingPower>,
    reachable: VotingPower,
    proposal: &Proposal,
    log: &mut CommitLog,
) -> Result<Option<Finalized>, CommitError> {
    let out = commit(precommits, validators, reachable, proposal, log);
    metrics.record_commit_result(&out);
    let _ = metrics.scrape();
    out
}

/// Snapshot `gossip.mesh` (config + behaviour construction + topic list).
pub fn observe_mesh(metrics: &Metrics, identity: &NodeIdentity) {
    let cfg = mesh_config();
    let topics = all_topics();
    let _ = gossipsub_behaviour(&identity.keypair);
    metrics.record_mesh(cfg.mesh_n as u64, topics.len() as u64);
    let _ = metrics.scrape();
}

/// Run `mempool.size_limits` then record occupancy/full. Return value unchanged.
pub fn observe_size_limits(
    metrics: &Metrics,
    pool: &mut Mempool,
    incoming: &SignedTx,
    occupancy_after: u64,
) -> Result<(), VerifyError> {
    check_tx_bytes(incoming)?;
    let before_full = matches!(ensure_room(pool, incoming), Err(VerifyError::MempoolFull));
    if before_full {
        metrics.record_mempool(occupancy_after, false, true);
        let _ = metrics.scrape();
        return Err(VerifyError::MempoolFull);
    }
    metrics.record_mempool(occupancy_after, false, false);
    let _ = metrics.scrape();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use consensus::propose::propose;
    use consensus::steps::{precommit_step, prevote_step};
    use consensus::vote::VoteReplayLog;
    use consensus::vrf;
    use crypto::from_bls;
    use crypto::sig::bls as bls_mod;
    use crypto::sig::ed25519::SecretKey as EdSk;
    use crypto::tx::sign;
    use crypto::vrf::public_key_from_seed;
    use network::identity;
    use state::account::Account;
    use std::sync::Arc;
    use std::thread;
    use types::header::{Header, HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::tx::Tx;
    use types::{Amount, ChainId, Epoch, Hash, Height, Nonce, Round, TestClock, GAS_TRANSFER};

    fn header_for(id: ValidatorId) -> Header {
        let clock = TestClock::new(1_000);
        let fields = HeaderFields::new(&clock, Height::GENESIS, Round::ZERO, id, 0, 1).unwrap();
        Header {
            fields,
            tx_root: Hash::ZERO,
            state_root: Hash::ZERO,
            receipts_root: Hash::ZERO,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        }
    }

    fn commit_ok(metrics: &Metrics) -> Result<Option<Finalized>, CommitError> {
        let sk = bls_mod::keygen().unwrap();
        let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(1));
        let mut validators = Map::new();
        validators.insert(id, VotingPower(1));
        let vrf_sk = [4u8; 32];
        let vrf_pk = public_key_from_seed(&vrf_sk);
        let seed = vrf::derive_seed(&[9u8; 32], Epoch::ZERO);
        let header = header_for(id);
        let app = Hash::from_bytes([7u8; 32]);
        let (_, proof) = vrf::leader_prove(&vrf_sk, &seed, &id).unwrap();
        let p = propose(
            &sk,
            id,
            id,
            &vrf_pk,
            &proof,
            &validators,
            &seed,
            Height::GENESIS,
            Round::ZERO,
            || (header.clone(), app),
        )
        .unwrap();
        let mut log = VoteReplayLog::new();
        let mut vrf_pks = Map::new();
        vrf_pks.insert(id, vrf_pk);
        let pv = prevote_step(
            &sk,
            id,
            &p,
            &vrf_pks,
            &seed,
            &validators,
            Height::GENESIS,
            Round::ZERO,
            None,
            None,
            &mut log,
        )
        .unwrap();
        let (pc, _) = precommit_step(
            &sk,
            id,
            Height::GENESIS,
            Round::ZERO,
            std::slice::from_ref(&pv),
            &validators,
            &p.header,
            &mut log,
        );
        let mut clog = CommitLog::new();
        observe_commit(
            metrics,
            std::slice::from_ref(&pc),
            &validators,
            VotingPower(1),
            &p,
            &mut clog,
        )
    }

    #[test]
    fn commit_observed_without_changing_outcome() {
        let m = Metrics::new();
        let f = commit_ok(&m).unwrap().unwrap();
        assert_eq!(f.app_hash, Hash::from_bytes([7u8; 32]));
        assert_eq!(m.commits_ok(), 1);
        assert!(m.scrape().unwrap().contains("l1_commit_success_total 1"));
    }

    #[test]
    fn exporter_down_does_not_fail_commit() {
        let m = Metrics::new();
        m.set_exporter_up(false);
        assert_eq!(m.scrape(), Err(ExporterDown));
        let f = commit_ok(&m).unwrap().unwrap();
        assert_eq!(f.height, Height::GENESIS);
        assert_eq!(m.commits_ok(), 1);
        let id = identity::generate().unwrap();
        observe_mesh(&m, &id);
        assert_eq!(m.mesh_n(), mesh_config().mesh_n as u64);
        assert_eq!(m.scrape(), Err(ExporterDown));
    }

    #[test]
    fn concurrent_record_never_panics() {
        let m = Arc::new(Metrics::new());
        let mut joins = Vec::new();
        for t in 0..8 {
            let m = Arc::clone(&m);
            joins.push(thread::spawn(move || {
                for i in 0..2_000u64 {
                    m.record_timings_ms(1_000 + (i % 50), 1_200);
                    m.record_mesh(4, 7);
                    m.record_mempool(i % 10, t == 0 && i == 0, false);
                    let _ = m.scrape();
                }
            }));
        }
        for j in joins {
            j.join().expect("metric thread panicked");
        }
        assert!(m.commits_ok() > 0);
    }

    #[test]
    fn mempool_full_is_recorded_not_swallowed() {
        let m = Metrics::new();
        let mut pool = Mempool::with_limits(1, 1, usize::MAX);
        let sk = EdSk::from_bytes(&[8u8; 32]);
        let tx = sign(
            &sk,
            Tx::transfer(
                ChainId::new(1),
                Nonce::ZERO,
                GAS_TRANSFER,
                Amount::new(1),
                types::Address::ZERO,
                Amount::new(1),
            ),
        );
        let acct = Account {
            balance: Amount::new(10_000),
            nonce: Nonce::ZERO,
            code_hash: types::Hash::ZERO,
        };
        pool.insert(tx.clone(), &acct).unwrap();
        let other = sign(
            &EdSk::from_bytes(&[9u8; 32]),
            Tx::transfer(
                ChainId::new(1),
                Nonce::ZERO,
                GAS_TRANSFER,
                Amount::new(1),
                types::Address::ZERO,
                Amount::new(1),
            ),
        );
        let err = observe_size_limits(&m, &mut pool, &other, 1).unwrap_err();
        assert_eq!(err, VerifyError::MempoolFull);
        assert!(m.render().contains("l1_mempool_full_total 1"));
    }
}
