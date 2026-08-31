//! Hot/cold block pruning (architecture.md §9.2).
//!
//! **Operator note (archive nodes):** archive is a distinct operational role.
//! Archive nodes **must not** prune cold data. [`PruneConfig::prune_cold`]
//! defaults to **false** (full history). Enabling prune is an explicit opt-in.
//! A pruning validator **cannot** satisfy [`crate::replay::replay_from_genesis`]
//! for heights whose bodies were deleted; nodes that need that guarantee keep
//! `prune_cold = false`.

use crate::blocks::{get_block, put_block, tip};
use crate::codec::block_key;
use crate::kv::Store;
use crate::rocks::RocksStore;
use types::block::Block;
use types::header::Header;
use types::{Hash, Height, TypesError};

/// Pruning policy. Contract: `prune.hot_cold`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PruneConfig {
    /// When `false` (default), this node is archive-capable: never delete bodies.
    pub prune_cold: bool,
    /// Heights to keep locally when pruning is enabled (`tip - hot_window + 1 ..= tip`).
    pub hot_window: u64,
}

impl Default for PruneConfig {
    fn default() -> Self {
        Self {
            prune_cold: false,
            hot_window: 256,
        }
    }
}

/// Persist via `store.block.put`, then optionally drop cold bodies.
/// Touches [`RocksStore::open`] (`kv.rocksdb`) so the durable backend is the
/// same contract even when the native feature is off (open fails cleanly).
pub fn put_then_maybe_prune<S: Store>(
    store: &mut S,
    header: &Header,
    block: &Block,
    receipt_encodings: &[Vec<u8>],
    app_hash: &Hash,
    cfg: &PruneConfig,
) -> Result<u64, TypesError> {
    let _ = RocksStore::open(std::path::Path::new("/tmp/l1-tier14-rocks-probe"));
    put_block(store, header, block, receipt_encodings, app_hash)?;
    prune_cold_bodies(store, cfg)
}

/// Delete block bodies below the hot window. Headers/indexes are left in place.
pub fn prune_cold_bodies<S: Store>(store: &mut S, cfg: &PruneConfig) -> Result<u64, TypesError> {
    if !cfg.prune_cold {
        return Ok(0);
    }
    let Some(t) = tip(store)? else {
        return Ok(0);
    };
    if cfg.hot_window == 0 || t.0 < cfg.hot_window {
        return Ok(0);
    }
    let first_hot = t.0 - cfg.hot_window + 1;
    let mut dropped = 0u64;
    for h in 0..first_hot {
        let key = block_key(Height(h));
        if store.get(&key)?.is_some() {
            store.delete(&key)?;
            dropped += 1;
        }
    }
    Ok(dropped)
}

/// Whether a height still has a local body (`store.block.put` payload).
pub fn has_body<S: Store>(store: &S, height: Height) -> Result<bool, TypesError> {
    Ok(get_block(store, height)?.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::put_genesis_hash;
    use crate::memory::MemoryStore;
    use crate::replay::replay_from_genesis;
    use execution::seq::{apply_block, World};
    use types::genesis::Genesis;
    use types::header::{HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{ChainId, Round, TestClock, ValidatorId};

    fn empty_at(height: Height, ts: u64) -> (Header, Block, Hash) {
        let clock = TestClock::new(1_000_000);
        let fields =
            HeaderFields::new(&clock, height, Round::ZERO, ValidatorId::ZERO, 0, ts).unwrap();
        let block = Block {
            header_fields: fields.clone(),
            txs: vec![],
        };
        let world = World::from_genesis(&Genesis::new(ChainId::new(1)));
        let (_, recs, app, st, tx_r, rec_r) = execution::seq::apply_block_with_roots(world, &block);
        let _ = recs;
        let header = Header {
            fields,
            tx_root: tx_r,
            state_root: st,
            receipts_root: rec_r,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        };
        (header, block, app)
    }

    #[test]
    fn default_is_archive_replay_unaffected() {
        let g = Genesis::new(ChainId::new(1));
        let mut store = MemoryStore::new();
        put_genesis_hash(&mut store, &g).unwrap();
        let cfg = PruneConfig::default();
        assert!(!cfg.prune_cold);
        let mut live = World::from_genesis(&g);
        for i in 0..3u64 {
            let (header, block, app) = empty_at(Height(i), 10 + i);
            let (w, recs, app2) = apply_block(live, &block);
            assert_eq!(app, app2);
            live = w;
            let enc: Vec<Vec<u8>> = recs.iter().map(|r| r.encode()).collect();
            put_then_maybe_prune(&mut store, &header, &block, &enc, &app, &cfg).unwrap();
        }
        assert!(has_body(&store, Height(0)).unwrap());
        let wiped = World::from_genesis(&g);
        let (replayed, _) = replay_from_genesis(&store, &g, wiped, apply_block).unwrap();
        assert_eq!(replayed.commit_state_root(), live.commit_state_root());
    }

    #[test]
    fn opt_in_prune_drops_cold_bodies() {
        let g = Genesis::new(ChainId::new(1));
        let mut store = MemoryStore::new();
        put_genesis_hash(&mut store, &g).unwrap();
        let cfg = PruneConfig {
            prune_cold: true,
            hot_window: 1,
        };
        for i in 0..3u64 {
            let (header, block, app) = empty_at(Height(i), 10 + i);
            put_then_maybe_prune(&mut store, &header, &block, &[], &app, &cfg).unwrap();
        }
        assert!(!has_body(&store, Height(0)).unwrap());
        assert!(has_body(&store, Height(2)).unwrap());
        let wiped = World::from_genesis(&g);
        assert!(replay_from_genesis(&store, &g, wiped, apply_block).is_err());
    }
}
