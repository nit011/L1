//! Post-speculation OCC validate (architecture.md §3.3, §4.3).
//!
//! A tx is stale if any slot it read was overwritten by an earlier (in block
//! order) writer — detected with [`VersionedSlots::validate`].

use crate::stm::rwset::{latest_version, SpecTx};
use crate::stm::schedule::Schedule;
use state::version::VersionedSlots;
use storage::memory::MemoryStore;
use types::collections::Set;

/// Tx indices that must be re-executed. Contract: `stm.validate`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StaleSet {
    /// Sorted indices.
    pub indices: Set<usize>,
}

/// Flag txs whose observed versions are no longer latest.
pub fn validate(
    specs: &[SpecTx],
    slots: &VersionedSlots<MemoryStore>,
    schedule: &Schedule,
) -> StaleSet {
    let _ = schedule.waves.len();
    let mut indices = Set::new();
    for s in specs {
        for key in &s.reads {
            let Some(obs) = s.observed.get(key) else {
                continue;
            };
            let ok = slots
                .validate(key, *obs)
                .expect("state.versioned_slot.validate");
            if !ok {
                indices.insert(s.index);
                break;
            }
            let _ = latest_version(slots, key);
        }
    }
    StaleSet { indices }
}

/// Mark `j` stale if an earlier `i` wrote a slot `j` read or wrote (§3.3).
pub fn stale_from_earlier_writes(specs: &[SpecTx]) -> StaleSet {
    let mut indices = Set::new();
    for j in specs {
        for i in specs {
            if i.index >= j.index {
                continue;
            }
            let rw_j: Set<_> = j.reads.union(&j.writes).cloned().collect();
            let hit: Set<_> = i.writes.intersection(&rw_j).cloned().collect();
            if !hit.is_empty() {
                indices.insert(j.index);
            }
        }
    }
    StaleSet { indices }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::World;
    use crate::stm::rwset::{seed_and_read, speculate};
    use crypto::address::from_ed25519;
    use crypto::sig::ed25519::SecretKey;
    use crypto::tx::sign;
    use state::version::VersionedSlots;
    use storage::memory::MemoryStore;
    use types::genesis::{Genesis, GenesisAccount};
    use types::{Address, Amount, ChainId, Hash, Nonce, GAS_TRANSFER};

    fn sk(b: u8) -> SecretKey {
        SecretKey::from_bytes(&[b; 32])
    }

    #[test]
    fn later_tx_flagged_when_same_account() {
        let ska = sk(4);
        let a = from_ed25519(&ska.verifying_key());
        let dest = Address::from_bytes([1u8; 32]);
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
        let tx0 = sign(
            &ska,
            types::tx::Tx::transfer(
                ChainId::new(1),
                Nonce::ZERO,
                GAS_TRANSFER,
                Amount::new(1),
                dest,
                Amount::new(5),
            ),
        );
        let tx1 = sign(
            &ska,
            types::tx::Tx::transfer(
                ChainId::new(1),
                Nonce(1),
                GAS_TRANSFER,
                Amount::new(1),
                dest,
                Amount::new(5),
            ),
        );
        let keys = crate::stm::rwset::inferred_keys(&tx0)
            .union(&crate::stm::rwset::inferred_keys(&tx1))
            .cloned()
            .collect();
        seed_and_read(&mut slots, &world, &keys);
        let specs = speculate(&world, &[tx0, tx1], &slots);
        let from_graph = stale_from_earlier_writes(&specs);
        assert!(from_graph.indices.contains(&1));
        // After committing tx0's writes into slots, OCC validate flags tx1.
        for k in &specs[0].writes {
            let _ = slots.write(k, b"committed".to_vec()).unwrap();
        }
        let sched = crate::stm::schedule::schedule(&crate::stm::graph::conflict_graph(&specs));
        let occ = validate(&specs, &slots, &sched);
        assert!(
            occ.indices.contains(&1),
            "later tx must be OCC-stale after earlier write"
        );
    }
}
