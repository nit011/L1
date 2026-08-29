//! Replay stored blocks from genesis through `exec.seq.apply_block`.
//!
//! Implements the Tier 3/4 exit bar (development-plan.md): store blocks, wipe
//! live state, walk the chain, re-apply, match stored roots.
//!
//! `storage` cannot depend on `execution` as a library (cycle: `state` →
//! `storage` → `execution` → `state`). Callers pass the frozen
//! `execution::seq::apply_block` function; this module **invokes** it.

use crate::blocks::{get_app_hash, get_block, get_genesis_hash, tip};
use crate::kv::Store;
use types::block::Block;
use types::genesis::Genesis;
use types::Hash;
use types::TypesError;

/// Replay failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayError {
    /// KV / codec.
    Store(TypesError),
    /// Missing or mismatched `genesis.hash`.
    GenesisMismatch,
    /// Missing block for a committed height.
    MissingBlock,
    /// Recomputed `app_hash` ≠ stored value.
    AppHashMismatch {
        height: u64,
        expected: Hash,
        got: Hash,
    },
}

impl From<TypesError> for ReplayError {
    fn from(e: TypesError) -> Self {
        Self::Store(e)
    }
}

/// Walk committed blocks in height order and re-run `apply_block`.
///
/// Contract: `store.replay_from_genesis`.
/// `apply_block` must be `execution::seq::apply_block`
/// (`apply_block(pre_state, block) -> (post_state, receipts, app_hash)`).
pub fn replay_from_genesis<S, W, R, F>(
    store: &S,
    genesis: &Genesis,
    mut world: W,
    mut apply_block: F,
) -> Result<(W, Hash), ReplayError>
where
    S: Store,
    F: FnMut(W, &Block) -> (W, Vec<R>, Hash),
{
    let stored_g = get_genesis_hash(store)?.ok_or(ReplayError::GenesisMismatch)?;
    let gh = genesis.hash();
    if stored_g != gh {
        return Err(ReplayError::GenesisMismatch);
    }
    let Some(end) = tip(store)? else {
        return Ok((world, gh));
    };
    let mut last_app = gh;
    let mut h = 0u64;
    while h <= end.0 {
        let height = types::Height(h);
        let block = get_block(store, height)?.ok_or(ReplayError::MissingBlock)?;
        let stored_app = get_app_hash(store, height)?.ok_or(ReplayError::MissingBlock)?;
        let (next, _receipts, app) = apply_block(world, &block);
        if app != stored_app {
            return Err(ReplayError::AppHashMismatch {
                height: h,
                expected: stored_app,
                got: app,
            });
        }
        world = next;
        last_app = app;
        h += 1;
    }
    Ok((world, last_app))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::{put_block, put_genesis_hash};
    use crate::memory::MemoryStore;
    use execution::receipt::Receipt;
    use execution::seq::{apply_block, World};
    use types::header::{Header, HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{ChainId, Height, Round, TestClock, ValidatorId};

    fn empty_block(height: Height, ts: u64) -> (Header, types::block::Block, Hash, Vec<Receipt>) {
        let clock = TestClock::new(1_000_000);
        let fields =
            HeaderFields::new(&clock, height, Round::ZERO, ValidatorId::ZERO, 0, ts).unwrap();
        let block = types::block::Block {
            header_fields: fields.clone(),
            txs: vec![],
        };
        let world = World::from_genesis(&Genesis::new(ChainId::new(1)));
        let (w2, recs, app, st, tx_r, rec_r) =
            execution::seq::apply_block_with_roots(world, &block);
        let _ = w2;
        let header = Header {
            fields,
            tx_root: tx_r,
            state_root: st,
            receipts_root: rec_r,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        };
        (header, block, app, recs)
    }

    #[test]
    fn replay_matches_live_empty_blocks() {
        let g = Genesis::new(ChainId::new(1));
        let mut store = MemoryStore::new();
        put_genesis_hash(&mut store, &g).unwrap();
        let live = World::from_genesis(&g);
        let mut live_world = live.clone();
        let mut last = Hash::ZERO;
        for i in 0..3u64 {
            let (header, block, app, recs) = empty_block(Height(i), 10 + i);
            let enc: Vec<Vec<u8>> = recs.iter().map(|r| r.encode()).collect();
            let (w, _, app2) = apply_block(live_world, &block);
            assert_eq!(app, app2);
            live_world = w;
            last = app;
            put_block(&mut store, &header, &block, &enc, &app).unwrap();
        }
        let wiped = World::from_genesis(&g);
        let (replayed, app) = replay_from_genesis(&store, &g, wiped, apply_block).unwrap();
        assert_eq!(app, last);
        assert_eq!(replayed.commit_state_root(), live_world.commit_state_root());
    }

    #[test]
    fn replay_rejects_wrong_genesis() {
        let g = Genesis::new(ChainId::new(1));
        let g2 = Genesis::new(ChainId::new(2));
        let mut store = MemoryStore::new();
        put_genesis_hash(&mut store, &g).unwrap();
        let err =
            replay_from_genesis(&store, &g2, World::from_genesis(&g2), apply_block).unwrap_err();
        assert_eq!(err, ReplayError::GenesisMismatch);
    }
}
