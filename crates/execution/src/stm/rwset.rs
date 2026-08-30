//! Speculative read/write sets (architecture.md §3.3, §4.2).
//!
//! RW sets are **inferred** by running [`crate::seq::apply_tx`] against a snapshot
//! and recording which versioned slots were observed — not declared by the tx
//! (architecture.md §3.3 rejects explicit RW declarations as the default).

use crate::receipt::Receipt;
use crate::seq::{apply_tx, World};
use crypto::address::from_ed25519;
use crypto::sig::ed25519::public_key_from_bytes;
use state::account::account_key;
use state::version::{SlotVersion, VersionedSlots};
use storage::memory::MemoryStore;
use types::collections::{Map, Set};
use types::tx::SignedTx;
use types::Address;

/// Synthetic slot for the staking ledger (architecture.md §3.3 write-set).
pub const STAKING_SLOT: &[u8] = b"stm.staking";

/// Speculative result for one transaction. Contract: `stm.rwset.speculate`.
#[derive(Clone, Debug)]
pub struct SpecTx {
    /// Index in the block (canonical order).
    pub index: usize,
    /// Slots read (versioned-slot keys).
    pub reads: Set<Vec<u8>>,
    /// Slots written on success.
    pub writes: Set<Vec<u8>>,
    /// Version observed at each read (`state.versioned_slot.read`).
    pub observed: Map<Vec<u8>, SlotVersion>,
    /// Snapshot after this tx applied to the **pre-block** world.
    pub spec_world: World,
    /// Receipt from [`apply_tx`].
    pub receipt: Receipt,
}

/// Sender address (same crypto path as `exec.seq.apply_tx`).
pub fn sender_of(signed: &SignedTx) -> Option<Address> {
    let pk = public_key_from_bytes(&signed.public_key).ok()?;
    Some(from_ed25519(&pk))
}

/// Account and staking keys this envelope may touch.
pub fn inferred_keys(signed: &SignedTx) -> Set<Vec<u8>> {
    let mut keys = Set::new();
    if let Some(from) = sender_of(signed) {
        keys.insert(account_key(&from));
    }
    if let Some(t) = signed.tx.as_transfer() {
        keys.insert(account_key(&t.to));
    }
    if signed.tx.as_stake().is_some() {
        keys.insert(STAKING_SLOT.to_vec());
    }
    keys
}

/// Latest version via [`VersionedSlots::validate`] (OCC; `latest` is crate-private).
pub fn latest_version(slots: &VersionedSlots<MemoryStore>, key: &[u8]) -> SlotVersion {
    let mut v: SlotVersion = 0;
    loop {
        if slots.validate(key, v).expect("versioned_slot.validate") {
            return v;
        }
        v = v.saturating_add(1);
        if v == 0 {
            return 0;
        }
    }
}

/// Seed slots from the pre-block world, then [`read`] each key.
pub fn seed_and_read(
    slots: &mut VersionedSlots<MemoryStore>,
    world: &World,
    keys: &Set<Vec<u8>>,
) -> Map<Vec<u8>, SlotVersion> {
    let mut observed = Map::new();
    for key in keys {
        if key.as_slice() == STAKING_SLOT {
            if slots.validate(key, 0).unwrap() {
                let _ = slots.read(key, 0).expect("versioned_slot.read");
                observed.insert(key.clone(), 0);
            }
            continue;
        }
        if key.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(key);
            let addr = Address::from_bytes(arr);
            if let Some(acc) = world.accounts.get(&addr) {
                if slots.validate(key, 0).unwrap() {
                    let _ = slots.write(key, acc.encode()).expect("seed write");
                }
            }
        }
        let latest = latest_version(slots, key);
        let _ = slots.read(key, latest).expect("versioned_slot.read");
        observed.insert(key.clone(), latest);
    }
    observed
}

fn writes_from(_signed: &SignedTx, receipt: &Receipt, reads: &Set<Vec<u8>>) -> Set<Vec<u8>> {
    if !receipt.success {
        return Set::new();
    }
    reads.clone()
}

/// Speculate one tx on a clone of `pre` (architecture.md §4.2 transparent RW).
pub fn speculate_one(
    index: usize,
    pre: &World,
    signed: &SignedTx,
    slots: &VersionedSlots<MemoryStore>,
) -> SpecTx {
    let keys = inferred_keys(signed);
    let mut observed = Map::new();
    for key in &keys {
        let latest = latest_version(slots, key);
        let _ = slots.read(key, latest).expect("versioned_slot.read");
        observed.insert(key.clone(), latest);
    }
    let mut spec_world = pre.clone();
    let receipt = apply_tx(&mut spec_world, signed);
    let writes = writes_from(signed, &receipt, &keys);
    SpecTx {
        index,
        reads: keys,
        writes,
        observed,
        spec_world,
        receipt,
    }
}

/// Parallel speculate for every tx in block order of results.
/// Contract: `stm.rwset.speculate`.
pub fn speculate(
    pre: &World,
    txs: &[SignedTx],
    slots: &VersionedSlots<MemoryStore>,
) -> Vec<SpecTx> {
    if txs.is_empty() {
        return Vec::new();
    }
    std::thread::scope(|scope| {
        let mut joins = Vec::with_capacity(txs.len());
        for (i, tx) in txs.iter().enumerate() {
            joins.push(scope.spawn(move || speculate_one(i, pre, tx, slots)));
        }
        let mut out: Vec<SpecTx> = joins.into_iter().map(|j| j.join().unwrap()).collect();
        out.sort_by_key(|s| s.index);
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::World;
    use crypto::address::from_ed25519;
    use crypto::sig::ed25519::SecretKey;
    use crypto::tx::sign;
    use types::genesis::{Genesis, GenesisAccount};
    use types::{Amount, ChainId, Hash, Nonce, GAS_TRANSFER};

    fn sk(b: u8) -> SecretKey {
        SecretKey::from_bytes(&[b; 32])
    }

    #[test]
    fn speculate_independent_transfers_disjoint_writes() {
        let ska = sk(1);
        let skb = sk(2);
        let a = from_ed25519(&ska.verifying_key());
        let b = from_ed25519(&skb.verifying_key());
        let mut g = Genesis::new(ChainId::new(1));
        for addr in [a, b] {
            g.insert_alloc(
                addr,
                GenesisAccount {
                    balance: Amount::new(10_000),
                    nonce: Nonce::ZERO,
                    code_hash: Hash::ZERO,
                },
            );
        }
        let world = World::from_genesis(&g);
        let mut slots = VersionedSlots::new(MemoryStore::new());
        let c = Address::from_bytes([3u8; 32]);
        let d = Address::from_bytes([4u8; 32]);
        let tx1 = sign(
            &ska,
            types::tx::Tx::transfer(
                ChainId::new(1),
                Nonce::ZERO,
                GAS_TRANSFER,
                Amount::new(1),
                c,
                Amount::new(10),
            ),
        );
        let tx2 = sign(
            &skb,
            types::tx::Tx::transfer(
                ChainId::new(1),
                Nonce::ZERO,
                GAS_TRANSFER,
                Amount::new(1),
                d,
                Amount::new(7),
            ),
        );
        let keys: Set<Vec<u8>> = inferred_keys(&tx1)
            .union(&inferred_keys(&tx2))
            .cloned()
            .collect();
        seed_and_read(&mut slots, &world, &keys);
        let spec = speculate(&world, &[tx1, tx2], &slots);
        assert_eq!(spec.len(), 2);
        assert!(spec[0].receipt.success && spec[1].receipt.success);
        let inter: Set<_> = spec[0]
            .writes
            .intersection(&spec[1].writes)
            .cloned()
            .collect();
        assert!(
            inter.is_empty(),
            "independent transfers must not share writes"
        );
        assert!(!spec[0].observed.is_empty());
    }

    #[test]
    fn speculate_same_sender_overlapping_writes() {
        let ska = sk(3);
        let a = from_ed25519(&ska.verifying_key());
        let dest = Address::from_bytes([9u8; 32]);
        let mut g = Genesis::new(ChainId::new(1));
        g.insert_alloc(
            a,
            GenesisAccount {
                balance: Amount::new(10_000),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        let world = World::from_genesis(&g);
        let mut slots = VersionedSlots::new(MemoryStore::new());
        let tx1 = sign(
            &ska,
            types::tx::Tx::transfer(
                ChainId::new(1),
                Nonce::ZERO,
                GAS_TRANSFER,
                Amount::new(1),
                dest,
                Amount::new(1),
            ),
        );
        let tx2 = sign(
            &ska,
            types::tx::Tx::transfer(
                ChainId::new(1),
                Nonce(1),
                GAS_TRANSFER,
                Amount::new(1),
                dest,
                Amount::new(1),
            ),
        );
        let keys: Set<Vec<u8>> = inferred_keys(&tx1)
            .union(&inferred_keys(&tx2))
            .cloned()
            .collect();
        seed_and_read(&mut slots, &world, &keys);
        let spec = speculate(&world, &[tx1, tx2], &slots);
        let inter: Set<_> = spec[0]
            .writes
            .intersection(&spec[1].reads)
            .cloned()
            .collect();
        assert!(!inter.is_empty());
        assert_eq!(spec.len(), 2);
    }
}
