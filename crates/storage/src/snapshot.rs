//! Point-in-time state snapshot as a faster alternative to full replay
//! (architecture.md §9.2 / §4.2).
//!
//! Snapshot sync is a **performance** path, not a second source of truth.
//! A node that loads this snapshot must arrive at a byte-identical
//! [`state.commit_root`] to one that ran [`crate::replay::replay_from_genesis`]
//! on the same chain. `storage` cannot import `state` (cycle); callers pass
//! `state::root::commit_root` (or `World::commit_state_root`).

use crate::kv::Store;
use crate::replay::replay_from_genesis;
use types::block::Block;
use types::genesis::Genesis;
use types::{Address, Hash};

/// Exported trie occupancy plus the committed combined root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateSnapshot {
    /// `state.commit_root` at the snapshot height.
    pub commit_root: [u8; 32],
    /// `(address, Account::encode)` pairs, sorted by address (determinism).
    pub accounts: Vec<(Address, Vec<u8>)>,
}

/// Bind account/storage trie roots through the real `state.commit_root`.
/// Contract: `sync.snapshot`.
pub fn snapshot_commit_root(
    account_root: &[u8; 32],
    contract_root: &[u8; 32],
    commit_root: impl Fn(&[u8; 32], &[u8; 32]) -> [u8; 32],
) -> [u8; 32] {
    commit_root(account_root, contract_root)
}

/// Build a snapshot. Account encodings are sorted by address bytes.
pub fn take_snapshot(
    commit_root: [u8; 32],
    mut accounts: Vec<(Address, Vec<u8>)>,
) -> StateSnapshot {
    accounts.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    StateSnapshot {
        commit_root,
        accounts,
    }
}

/// Restore encodings (sorted). Caller inserts into a live account trie.
pub fn restore_account_encodings(snap: &StateSnapshot) -> Vec<(Address, Vec<u8>)> {
    snap.accounts.clone()
}

/// Compare snapshot root to a full `store.replay_from_genesis` result.
pub fn snapshot_matches_replay(snap: &StateSnapshot, replayed_commit: &Hash) -> bool {
    snap.commit_root == *replayed_commit.as_bytes()
}

/// Invoke `store.replay_from_genesis` so snapshot tests share the same walk.
pub fn replay_for_snapshot_check<S, W, R, F>(
    store: &S,
    genesis: &Genesis,
    world: W,
    apply_block: F,
) -> Result<(W, Hash), crate::replay::ReplayError>
where
    S: Store,
    F: FnMut(W, &Block) -> (W, Vec<R>, Hash),
{
    replay_from_genesis(store, genesis, world, apply_block)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::{put_block, put_genesis_hash};
    use crate::memory::MemoryStore;
    use execution::seq::{apply_block, World};
    use state::account::Account;
    use state::root::commit_root;
    use types::header::{Header, HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{ChainId, Height, Round, TestClock, ValidatorId};

    #[test]
    fn snapshot_and_full_replay_same_commit_root() {
        let g = Genesis::new(ChainId::new(1));
        let mut store = MemoryStore::new();
        put_genesis_hash(&mut store, &g).unwrap();
        let mut live = World::from_genesis(&g);
        for i in 0..3u64 {
            let clock = TestClock::new(1_000_000);
            let fields =
                HeaderFields::new(&clock, Height(i), Round::ZERO, ValidatorId::ZERO, 0, 10 + i)
                    .unwrap();
            let block = types::block::Block {
                header_fields: fields.clone(),
                txs: vec![],
            };
            let (w, recs, app, st, tx_r, rec_r) =
                execution::seq::apply_block_with_roots(live, &block);
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
        }
        let live_root = live.commit_state_root();
        let via_commit =
            snapshot_commit_root(&live.accounts.root(), &live.storage.root(), commit_root);
        assert_eq!(via_commit, *live_root.as_bytes());

        let mut accounts = Vec::new();
        for (addr, ga) in &g.alloc {
            let acc = Account::from_genesis(ga);
            accounts.push((*addr, acc.encode()));
        }
        let snap = take_snapshot(via_commit, accounts);
        let restored = World::from_genesis(&g);
        for (addr, enc) in restore_account_encodings(&snap) {
            let acc = Account::decode(&enc).unwrap();
            assert_eq!(restored.accounts.get(&addr).unwrap(), acc);
        }

        let wiped = World::from_genesis(&g);
        let (replayed, _) = replay_for_snapshot_check(&store, &g, wiped, apply_block).unwrap();
        let replay_root = replayed.commit_state_root();
        assert!(
            snapshot_matches_replay(&snap, &replay_root),
            "snapshot {} replay {}",
            hex::encode(snap.commit_root),
            hex::encode(replay_root.as_bytes())
        );
        assert_eq!(replay_root, live_root);
        assert_eq!(
            hex::encode(replay_root.as_bytes()),
            "17412c6b501b28db07efb8ca00efd4927ce9aaf6941be49c4fc5963e3693a234"
        );
    }

    #[test]
    fn snapshot_root_mismatch_is_detected() {
        let snap = take_snapshot([1u8; 32], vec![]);
        assert!(!snapshot_matches_replay(
            &snap,
            &Hash::from_bytes([2u8; 32])
        ));
    }
}
