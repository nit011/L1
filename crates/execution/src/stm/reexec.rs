//! Sequential re-execution of stale txs (architecture.md §3.3).
//!
//! Flagged txs are applied with [`crate::seq::apply_tx`] on the committed
//! world (not a second interpreter). Then OCC is re-checked.

use crate::receipt::Receipt;
use crate::seq::{apply_tx, World};
use crate::stm::rwset::{inferred_keys, seed_and_read, speculate, SpecTx, STAKING_SLOT};
use crate::stm::validate::{stale_from_earlier_writes, validate, StaleSet};
use state::account::Account;
use state::version::VersionedSlots;
use storage::memory::MemoryStore;
use types::block::Block;
use types::collections::Set;
use types::Address;

fn merge_spec(dst: &mut World, spec: &SpecTx) {
    for key in &spec.writes {
        if key.as_slice() == STAKING_SLOT {
            dst.staking = spec.spec_world.staking.clone();
            continue;
        }
        if key.len() != 32 {
            continue;
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(key);
        let addr = Address::from_bytes(arr);
        if let Some(acc) = spec.spec_world.accounts.get(&addr) {
            dst.accounts.put(&addr, &acc);
        }
    }
}

fn bump_writes(slots: &mut VersionedSlots<MemoryStore>, world: &World, writes: &Set<Vec<u8>>) {
    for key in writes {
        if key.as_slice() == STAKING_SLOT {
            let _ = slots.write(key, b"s".to_vec());
            continue;
        }
        if key.len() != 32 {
            continue;
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(key);
        let addr = Address::from_bytes(arr);
        let acc = world.accounts.get(&addr).unwrap_or_else(Account::empty);
        let _ = slots.write(key, acc.encode());
    }
}

/// Commit in block order: reuse speculation when OCC says the snapshot is
/// still valid, otherwise [`apply_tx`]. Contract: `stm.reexec_sequential`.
pub fn reexec_sequential(
    pre: World,
    block: &Block,
    mut slots: VersionedSlots<MemoryStore>,
    specs: &[SpecTx],
    mut stale: StaleSet,
) -> (World, Vec<Receipt>) {
    let mut world = pre;
    let mut receipts = Vec::with_capacity(block.txs.len());
    for (i, signed) in block.txs.iter().enumerate() {
        let spec = &specs[i];
        let need = stale.indices.contains(&i);
        let rec = if !need {
            merge_spec(&mut world, spec);
            bump_writes(&mut slots, &world, &spec.writes);
            spec.receipt.clone()
        } else {
            let r = apply_tx(&mut world, signed);
            let keys = inferred_keys(signed);
            let writes = if r.success { keys } else { Set::new() };
            bump_writes(&mut slots, &world, &writes);
            r
        };
        receipts.push(rec);
        // Re-validate remaining txs against updated slots.
        let rest: Vec<SpecTx> = specs[i + 1..].to_vec();
        if !rest.is_empty() {
            let sched = crate::stm::schedule::schedule(&crate::stm::graph::conflict_graph(&rest));
            let more = validate(&rest, &slots, &sched);
            for idx in more.indices {
                stale.indices.insert(idx);
            }
        }
    }
    let _ = stale_from_earlier_writes(specs);
    (world, receipts)
}

/// Full OCC loop helper used by `stm.apply_block`.
pub fn commit_block(
    pre: World,
    block: &Block,
) -> (World, Vec<Receipt>, VersionedSlots<MemoryStore>) {
    let mut keys = Set::new();
    for tx in &block.txs {
        keys.extend(inferred_keys(tx));
    }
    let mut slots = VersionedSlots::new(MemoryStore::new());
    seed_and_read(&mut slots, &pre, &keys);
    let specs = speculate(&pre, &block.txs, &slots);
    let mut stale = stale_from_earlier_writes(&specs);
    let sched = crate::stm::schedule::schedule(&crate::stm::graph::conflict_graph(&specs));
    stale
        .indices
        .extend(validate(&specs, &slots, &sched).indices);
    let (world, receipts) = reexec_sequential(pre, block, slots, &specs, stale);
    (world, receipts, VersionedSlots::new(MemoryStore::new()))
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
        Amount, ChainId, Hash, Height, Nonce, Round, TestClock, ValidatorId, GAS_TRANSFER,
    };

    fn sk(b: u8) -> SecretKey {
        SecretKey::from_bytes(&[b; 32])
    }

    fn block_of(txs: Vec<types::tx::SignedTx>) -> Block {
        let clock = TestClock::new(1_000);
        let fields = HeaderFields::new(
            &clock,
            Height::GENESIS,
            Round::ZERO,
            ValidatorId::ZERO,
            0,
            1,
        )
        .unwrap();
        Block {
            header_fields: fields,
            txs,
        }
    }

    #[test]
    fn chained_same_account_resolves() {
        let ska = sk(5);
        let a = from_ed25519(&ska.verifying_key());
        let dest = Address::from_bytes([2u8; 32]);
        let mut g = Genesis::new(ChainId::new(1));
        g.insert_alloc(
            a,
            GenesisAccount {
                balance: Amount::new(10_000),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        let pre = World::from_genesis(&g);
        let mut txs = Vec::new();
        for n in 0..3u64 {
            txs.push(sign(
                &ska,
                types::tx::Tx::transfer(
                    ChainId::new(1),
                    Nonce(n),
                    GAS_TRANSFER,
                    Amount::new(1),
                    dest,
                    Amount::new(10),
                ),
            ));
        }
        let block = block_of(txs);
        let (world, recs, _) = commit_block(pre.clone(), &block);
        assert!(recs.iter().all(|r| r.success));
        let seq = crate::seq::apply_block(pre, &block);
        assert_eq!(recs, seq.1);
        assert_eq!(world.commit_state_root(), seq.0.commit_state_root());
        assert_eq!(world.account(&a).nonce.0, 3, "three chained txs bump nonce");
    }
}
