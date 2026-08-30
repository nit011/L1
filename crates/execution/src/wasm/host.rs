//! WASM host functions for contract storage (architecture.md §4.1).
//!
//! Guests import `host.sload` / `host.sstore` only. They never receive a
//! trie handle. Canonical values live in [`ContractStorageTrie`]; versions
//! are recorded with [`VersionedSlots::read`] / [`write`].
//!
//! Slot keys are `contract:32 || 28×0x00 || slot:u32 BE` so iteration on the
//! trie remains byte-sorted (Tier 0 determinism).

use crate::seq::World;
use state::tries::ContractStorageTrie;
use state::version::VersionedSlots;
use storage::memory::MemoryStore;
use types::Address;

/// Host data for wasmtime (`Send` pointer into the live [`World`]).
pub struct Host {
    /// Mutable world for this transaction.
    pub world: *mut World,
    /// Contract whose storage is being accessed.
    pub contract: Address,
}

unsafe impl Send for Host {}

/// 64-byte storage key for `(contract, slot)`.
pub fn storage_key(contract: &Address, slot: u32) -> Vec<u8> {
    let mut k = Vec::with_capacity(64);
    k.extend_from_slice(contract.as_bytes());
    k.extend_from_slice(&[0u8; 28]);
    k.extend_from_slice(&slot.to_be_bytes());
    k
}

fn i64_from_value(raw: Option<&[u8]>) -> i64 {
    match raw {
        Some(b) if b.len() == 8 => i64::from_be_bytes(b.try_into().unwrap()),
        _ => 0,
    }
}

/// Load a slot. Contract: `wasm.host.sload`.
///
/// Calls [`ContractStorageTrie::get`] and [`VersionedSlots::read`].
pub fn sload(world: &mut World, contract: Address, slot: u32) -> i64 {
    let key = storage_key(&contract, slot);
    world.storage_reads.insert(key.clone());
    let trie_val = ContractStorageTrie::get(&world.storage, &key);
    let latest = if world.versioned.validate(&key, 0).expect("validate") {
        0
    } else {
        let mut v = 1u64;
        loop {
            if world.versioned.validate(&key, v).expect("validate") {
                break v;
            }
            v = v.saturating_add(1);
            if v == 0 {
                break 0;
            }
        }
    };
    let _ = VersionedSlots::<MemoryStore>::read(&world.versioned, &key, latest)
        .expect("versioned_slot.read");
    i64_from_value(trie_val.as_deref())
}

/// Store a slot. Contract: `wasm.host.sstore`.
///
/// Calls [`sload`] then [`ContractStorageTrie::put`] and [`VersionedSlots::write`].
pub fn sstore(world: &mut World, contract: Address, slot: u32, value: i64) {
    let _ = sload(world, contract, slot);
    let key = storage_key(&contract, slot);
    world.storage_writes.insert(key.clone());
    let bytes = value.to_be_bytes().to_vec();
    world.storage.put(&key, bytes.clone());
    let _ = world
        .versioned
        .write(&key, bytes)
        .expect("versioned_slot.write");
}

/// Host `sload` for wasmtime.
pub fn host_sload(caller: wasmtime::Caller<'_, Host>, slot: i32) -> i64 {
    let (ptr, contract) = {
        let h = caller.data();
        (h.world, h.contract)
    };
    let world = unsafe { &mut *ptr };
    sload(world, contract, slot as u32)
}

/// Host `sstore` for wasmtime.
pub fn host_sstore(caller: wasmtime::Caller<'_, Host>, slot: i32, value: i64) {
    let (ptr, contract) = {
        let h = caller.data();
        (h.world, h.contract)
    };
    let world = unsafe { &mut *ptr };
    sstore(world, contract, slot as u32, value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::genesis::Genesis;
    use types::ChainId;

    #[test]
    fn sload_missing_is_zero() {
        let mut world = World::from_genesis(&Genesis::new(ChainId::new(1)));
        let c = Address::from_bytes([1u8; 32]);
        assert_eq!(sload(&mut world, c, 0), 0);
        assert!(!world.storage_reads.is_empty());
    }

    #[test]
    fn sstore_then_sload() {
        let mut world = World::from_genesis(&Genesis::new(ChainId::new(1)));
        let c = Address::from_bytes([2u8; 32]);
        sstore(&mut world, c, 7, 99);
        assert_eq!(sload(&mut world, c, 7), 99);
        assert!(world.storage_writes.contains(&storage_key(&c, 7)));
    }

    #[test]
    fn keys_are_sorted_64_bytes() {
        let a = Address::from_bytes([0u8; 32]);
        let k0 = storage_key(&a, 0);
        let k1 = storage_key(&a, 1);
        assert_eq!(k0.len(), 64);
        assert!(k0 < k1);
    }
}
