//! Storage rent charged against `state.account` occupancy (architecture.md §4.2, §9.2).
//!
//! **Frozen-spec decision:** [`Account`] canonical encoding is **unchanged**.
//! Occupied bytes = `Account::encode().len()` plus contract-storage bytes tracked
//! in this auxiliary book — not new hashed account fields. Expiry/reactivation
//! still uses the real account trie (`mpt.prove` of `Account::encode()`).

use crate::gas::gas_meter;
use state::account::Account;
use types::tx::Tx;
use types::{Address, Amount, ChainId, Nonce, GAS_TRANSFER};

/// Rent gas for `epochs` given account encoding size plus extra storage.
/// Contract: `state.rent`. Calls `tx.gas_meter` for the gas unit.
pub fn rent_gas(
    account: &Account,
    extra_storage_bytes: u64,
    epochs: u64,
) -> Result<u64, crate::gas::GasError> {
    let probe = Tx::transfer(
        ChainId::new(1),
        Nonce::ZERO,
        GAS_TRANSFER,
        Amount::ZERO,
        Address::ZERO,
        Amount::ZERO,
    );
    let unit = gas_meter(&probe)?;
    let bytes = account.encode().len() as u64 + extra_storage_bytes;
    Ok(bytes
        .saturating_mul(epochs)
        .saturating_mul(unit / GAS_TRANSFER.max(1)))
}

/// Apply [`rent_gas`] then [`state::expiry::expire`] (expiry cannot import this crate).
pub fn expire_if_unpaid(
    trie: &mut state::tries::AccountTrie,
    address: &types::Address,
    account: &Account,
    extra_storage_bytes: u64,
    epochs: u64,
    prepaid: u64,
) -> Result<state::expiry::ExpiryRecord, state::expiry::ExpiryError> {
    let due = rent_gas(account, extra_storage_bytes, epochs)
        .map_err(|_| state::expiry::ExpiryError::Proof)?;
    state::expiry::expire(trie, address, due, prepaid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use state::expiry::reactivate;
    use state::tries::AccountTrie;
    use types::{Hash, Nonce};

    fn acc(bal: u128) -> Account {
        Account {
            balance: Amount::new(bal),
            nonce: Nonce::ZERO,
            code_hash: Hash::ZERO,
        }
    }

    #[test]
    fn more_storage_accrues_more_rent() {
        let a = acc(1);
        let small = rent_gas(&a, 10, 3).unwrap();
        let large = rent_gas(&a, 10_000, 3).unwrap();
        assert!(large > small, "{large} vs {small}");
        assert_eq!(rent_gas(&a, 10, 3).unwrap(), small);
    }

    #[test]
    fn rent_feeds_expiry_without_changing_account_encode() {
        let addr = Address::from_bytes([3u8; 32]);
        let account = acc(9);
        let before = account.encode();
        let mut trie = AccountTrie::new();
        trie.put(&addr, &account);
        let rec = expire_if_unpaid(&mut trie, &addr, &account, 100, 2, 0).unwrap();
        assert!(trie.get(&addr).is_none());
        let excl =
            state::mpt::proof::prove_exclusion(trie.as_trie(), &state::account::account_key(&addr))
                .unwrap();
        let back = reactivate(&mut trie, &rec, &excl).unwrap();
        assert_eq!(back.encode(), before);
    }
}
