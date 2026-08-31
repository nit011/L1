//! Snapshot vs headers-then-bodies catch-up (`stress.snapshot_sync_test`).
//!
//! Under a live compose chain, the `joiner` service uses `node.catchup`
//! (`headers_then_bodies` + replay). In-process we:
//! 1. Apply blocks into a live [`MemoryStore`] (`store.block.put`).
//! 2. Copy the same headers/bodies into an empty store via
//!    [`network::sync::headers_then_bodies`].
//! 3. Require [`storage::snapshot::snapshot_matches_replay`] against the live
//!    [`World::commit_state_root`].

use execution::seq::{apply_block, World};
use network::sync::{headers_then_bodies, BodyOffer};
use state::account::Account;
use state::root::commit_root;
use std::time::Instant;
use storage::blocks::{put_block, put_genesis_hash};
use storage::memory::MemoryStore;
use storage::snapshot::{
    replay_for_snapshot_check, snapshot_commit_root, snapshot_matches_replay, take_snapshot,
};
use types::block::Block;
use types::genesis::Genesis;
use types::header::{Header, HeaderFields, DA_ROOT_PLACEHOLDER};
use types::{ChainId, Hash, Height, Round, TestClock, ValidatorId};

/// Catch-up comparison.
#[derive(Clone, Debug)]
pub struct SyncReport {
    /// Full replay wall ms.
    pub full_replay_ms: u128,
    /// Snapshot helper wall ms.
    pub snapshot_ms: u128,
    /// Roots matched (snapshot vs replay vs live).
    pub roots_equal: bool,
    /// Headers-then-bodies landed the same tip.
    pub headers_then_bodies_ok: bool,
    /// Joiner compose catch-up ms (if run).
    pub joiner_ms: Option<u128>,
}

fn build_chain(n_blocks: u64) -> (Genesis, MemoryStore, World, Vec<Header>, Vec<BodyOffer>) {
    let g = Genesis::new(ChainId::new(18));
    let mut store = MemoryStore::new();
    put_genesis_hash(&mut store, &g).unwrap();
    let mut live = World::from_genesis(&g);
    let clock = TestClock::new(1_000_000);
    let mut headers = Vec::new();
    let mut bodies = Vec::new();
    for i in 0..n_blocks {
        let fields =
            HeaderFields::new(&clock, Height(i), Round::ZERO, ValidatorId::ZERO, 0, 10 + i)
                .unwrap();
        let block = Block {
            header_fields: fields.clone(),
            txs: vec![],
        };
        let (w, recs, app, st, tx_r, rec_r) = execution::seq::apply_block_with_roots(live, &block);
        live = w;
        let header = Header {
            fields,
            tx_root: tx_r,
            state_root: st,
            receipts_root: rec_r,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        };
        let enc: Vec<Vec<u8>> = recs.iter().map(|r| r.encode()).collect();
        put_block(&mut store, &header, &block, &enc, &app).unwrap();
        bodies.push(BodyOffer {
            header: header.clone(),
            block,
            receipts: enc,
            app_hash: app,
        });
        headers.push(header);
    }
    (g, store, live, headers, bodies)
}

/// Apply empty blocks, snapshot vs `replay_from_genesis`, plus headers-then-bodies.
pub fn compare_paths(n_blocks: u64) -> SyncReport {
    let (g, store, live, headers, bodies) = build_chain(n_blocks);
    let live_root = live.commit_state_root();
    let t_snap = Instant::now();
    let via = snapshot_commit_root(&live.accounts.root(), &live.storage.root(), commit_root);
    assert_eq!(via, *live_root.as_bytes());
    let mut accounts = Vec::new();
    for (addr, ga) in &g.alloc {
        accounts.push((*addr, Account::from_genesis(ga).encode()));
    }
    let snap = take_snapshot(via, accounts);
    let snapshot_ms = t_snap.elapsed().as_millis();

    let t_full = Instant::now();
    let wiped = World::from_genesis(&g);
    let (replayed, _) = replay_for_snapshot_check(&store, &g, wiped, apply_block).unwrap();
    let full_replay_ms = t_full.elapsed().as_millis();

    let mut via_htb = MemoryStore::new();
    put_genesis_hash(&mut via_htb, &g).unwrap();
    let tip = headers_then_bodies(&mut via_htb, &headers, &bodies).unwrap();
    let headers_then_bodies_ok = tip == Some(Height(n_blocks.saturating_sub(1)));
    let (replay_htb, _) =
        replay_for_snapshot_check(&via_htb, &g, World::from_genesis(&g), apply_block).unwrap();

    let roots_equal = snapshot_matches_replay(&snap, &replayed.commit_state_root())
        && replayed.commit_state_root() == live_root
        && replay_htb.commit_state_root() == live_root;
    SyncReport {
        full_replay_ms,
        snapshot_ms,
        roots_equal,
        headers_then_bodies_ok,
        joiner_ms: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_and_full_replay_same_root() {
        let r = compare_paths(3);
        eprintln!(
            "stress.snapshot_sync_test full_replay_ms={} snapshot_ms={} equal={} htb={}",
            r.full_replay_ms, r.snapshot_ms, r.roots_equal, r.headers_then_bodies_ok
        );
        assert!(r.roots_equal);
        assert!(r.headers_then_bodies_ok);
    }

    #[test]
    #[ignore]
    fn docker_joiner_catchup() {
        use crate::harness::{bring_up, docker_ok, tear_down, wait_tip, LoadConfig};
        use std::time::Duration;
        assert!(docker_ok());
        let cfg = LoadConfig {
            duration: Duration::from_secs(2),
            ..LoadConfig::default()
        };
        let _ = bring_up(&cfg);
        wait_tip(0, 1, Duration::from_secs(40));
        let behind = crate::harness::read_tip(0).unwrap_or(0);
        let t0 = Instant::now();
        let st = crate::harness::compose_cmd()
            .args(["--profile", "join", "up", "-d", "--no-build", "joiner"])
            .status()
            .unwrap();
        assert!(st.success());
        let mut joiner_h = 0u64;
        let deadline = Instant::now() + Duration::from_secs(40);
        while Instant::now() < deadline {
            if let Ok(s) =
                std::fs::read_to_string(crate::harness::repo_root().join("infra/data/joiner/tip"))
            {
                if let Some(h) = s.lines().next().and_then(|l| l.parse().ok()) {
                    joiner_h = h;
                    if h >= behind {
                        break;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let joiner_ms = t0.elapsed().as_millis();
        eprintln!(
            "joiner catchup behind={behind} got={joiner_h} ms={joiner_ms} (node.catchup / headers_then_bodies)"
        );
        let r = compare_paths(3);
        tear_down();
        assert!(r.roots_equal && r.headers_then_bodies_ok);
        assert!(joiner_h > 0 || behind == 0);
        let _ = joiner_ms;
    }
}
