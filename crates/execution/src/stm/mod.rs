//! Block-STM pipeline (architecture.md §3.2–§3.5).
//!
//! Speculate → conflict graph → schedule onto OS threads → OCC validate →
//! re-execute stale txs via [`crate::seq::apply_tx`]. The committed
//! `(world, receipts, app_hash)` must match [`crate::seq::apply_block`].
//!
//! Hot accounts (architecture.md §3.5 / §4.4) serialize: many txs touching one
//! account form a conflict chain. That is expected, not a bug.

pub mod graph;
pub mod reexec;
pub mod rwset;
pub mod schedule;
pub mod validate;

use crate::seq::{self, World};
use crate::stm::graph::conflict_graph;
use crate::stm::reexec::reexec_sequential;
use crate::stm::rwset::{inferred_keys, seed_and_read, speculate};
use crate::stm::schedule::{run_waves, schedule};
use crate::stm::validate::{stale_from_earlier_writes, validate};
use state::version::VersionedSlots;
use storage::memory::MemoryStore;
use types::block::Block;
use types::collections::Set;
use types::Hash;

/// Parallel apply. Contract: `stm.apply_block`.
///
/// Calls [`seq::apply_block`] for comparison. The value returned is the STM
/// commit; a mismatch panics (a silent sequential fallback would hide forks).
pub fn apply_block(pre: World, block: &Block) -> (World, Vec<crate::receipt::Receipt>, Hash) {
    let seq_out = seq::apply_block(pre.clone(), block);
    let stm_out = apply_block_engine(pre, block);
    assert_eq!(
        stm_out.1.iter().map(|r| r.encode()).collect::<Vec<_>>(),
        seq_out.1.iter().map(|r| r.encode()).collect::<Vec<_>>(),
        "stm.apply_block receipts diverged from exec.seq.apply_block"
    );
    assert_eq!(
        stm_out.2, seq_out.2,
        "stm.apply_block app_hash diverged from exec.seq.apply_block"
    );
    assert_eq!(stm_out.0.commit_state_root(), seq_out.0.commit_state_root());
    stm_out
}

/// STM commit without the sequential comparison (for benches).
pub fn apply_block_engine(
    pre: World,
    block: &Block,
) -> (World, Vec<crate::receipt::Receipt>, Hash) {
    let mut keys = Set::new();
    for tx in &block.txs {
        keys.extend(inferred_keys(tx));
    }
    let mut slots = VersionedSlots::new(MemoryStore::new());
    seed_and_read(&mut slots, &pre, &keys);
    let specs = speculate(&pre, &block.txs, &slots);
    let graph = conflict_graph(&specs);
    let sched = schedule(&graph);
    let _workers = run_waves(&sched, |_| {});
    let mut stale = stale_from_earlier_writes(&specs);
    stale
        .indices
        .extend(validate(&specs, &slots, &sched).indices);
    let (world, receipts) = reexec_sequential(pre, block, slots, &specs, stale);
    let encoded: Vec<Vec<u8>> = receipts.iter().map(|r| r.encode()).collect();
    let tx_r = types::block::tx_root_signed(&block.txs);
    let rec_root = types::block::receipts_root(&encoded);
    let st = world.commit_state_root();
    let app = seq::app_hash(&st, &tx_r, &rec_root);
    (world, receipts, app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::World;
    use crypto::address::from_ed25519;
    use crypto::sig::ed25519::SecretKey;
    use crypto::tx::sign;
    use types::genesis::{Genesis, GenesisAccount};
    use types::header::HeaderFields;
    use types::{
        Address, Amount, ChainId, Hash, Height, Nonce, Round, TestClock, ValidatorId, GAS_TRANSFER,
    };

    fn sk(b: u8) -> SecretKey {
        SecretKey::from_bytes(&[b; 32])
    }

    fn empty_fields() -> HeaderFields {
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

    #[test]
    fn empty_block_matches_seq() {
        let g = Genesis::new(ChainId::new(1));
        let pre = World::from_genesis(&g);
        let block = Block {
            header_fields: empty_fields(),
            txs: vec![],
        };
        let a = apply_block(pre.clone(), &block);
        let b = seq::apply_block(pre, &block);
        assert_eq!(a.2, b.2);
        assert!(a.1.is_empty());
    }

    #[test]
    fn hot_account_matches_seq() {
        let ska = sk(8);
        let a = from_ed25519(&ska.verifying_key());
        let dest = Address::from_bytes([3u8; 32]);
        let mut g = Genesis::new(ChainId::new(1));
        g.insert_alloc(
            a,
            GenesisAccount {
                balance: Amount::new(100_000),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        let pre = World::from_genesis(&g);
        let mut txs = Vec::new();
        for n in 0..8u64 {
            txs.push(sign(
                &ska,
                types::tx::Tx::transfer(
                    ChainId::new(1),
                    Nonce(n),
                    GAS_TRANSFER,
                    Amount::new(1),
                    dest,
                    Amount::new(1),
                ),
            ));
        }
        let block = Block {
            header_fields: empty_fields(),
            txs,
        };
        let stm = apply_block(pre.clone(), &block);
        let seq = seq::apply_block(pre, &block);
        assert_eq!(stm.2, seq.2);
        assert_eq!(stm.1, seq.1);
    }

    #[test]
    fn staking_bond_matches_seq() {
        let ska = sk(9);
        let from = from_ed25519(&ska.verifying_key());
        let vid = ValidatorId::from_bytes([4u8; 48]);
        let mut g = Genesis::new(ChainId::new(1));
        g.insert_alloc(
            from,
            GenesisAccount {
                balance: Amount::new(10_000),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        let pre = World::from_genesis(&g);
        let tx = types::tx::Tx::stake_bond(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            vid,
            Amount::new(200),
        );
        let block = Block {
            header_fields: empty_fields(),
            txs: vec![sign(&ska, tx)],
        };
        let stm = apply_block(pre.clone(), &block);
        let seq = seq::apply_block(pre, &block);
        assert_eq!(stm.2, seq.2);
        assert!(stm.1[0].success);
    }
}
