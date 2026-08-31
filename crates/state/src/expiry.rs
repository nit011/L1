//! State expiry and Merkle-proof reactivation (architecture.md §4.2).
//!
//! Live accounts stay in the hashed [`crate::account::Account`] trie with the
//! **Tier 1 encoding**. When rent is unpaid, the account is deleted from the
//! live trie and an inclusion proof of the last value is retained. Reactivation
//! requires that proof plus `mpt.prove_exclusion` on the live trie.
//!
//! `state.rent` (`execution::rent::rent_gas`) cannot be imported here (`execution`
//! already depends on `state`). Callers pass `rent_due` / `prepaid` from that
//! function; `crates/execution/src/rent.rs` tests call both.

use crate::account::{account_key, Account};
use crate::mpt::proof::{prove, prove_exclusion, verify, MptProof};
use crate::tries::AccountTrie;
use types::Address;

/// Last known account at expiry, plus an inclusion proof against that root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpiryRecord {
    /// Expired account.
    pub address: Address,
    /// Canonical `Account::encode` at expiry.
    pub encoded: Vec<u8>,
    /// Inclusion proof (`mpt.prove`).
    pub inclusion: MptProof,
    /// Account-trie root at expiry.
    pub root: [u8; 32],
}

/// Expiry / reactivation errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExpiryError {
    /// Rent still covers the account.
    StillPaid,
    /// Account missing or proof failed.
    Proof,
    /// Live trie still holds the key (not genuinely expired).
    StillPresent,
}

/// Drop an unpaid account from the live trie. Contract: `state.expiry`.
pub fn expire(
    trie: &mut AccountTrie,
    address: &Address,
    rent_due: u64,
    prepaid: u64,
) -> Result<ExpiryRecord, ExpiryError> {
    if rent_due <= prepaid {
        return Err(ExpiryError::StillPaid);
    }
    let account = trie.get(address).ok_or(ExpiryError::Proof)?;
    let encoded = account.encode();
    let key = account_key(address);
    let inclusion = prove(trie.as_trie(), &key).ok_or(ExpiryError::Proof)?;
    let root = trie.root();
    if !verify(&key, &inclusion, &root) {
        return Err(ExpiryError::Proof);
    }
    trie.delete(address);
    Ok(ExpiryRecord {
        address: *address,
        encoded,
        inclusion,
        root,
    })
}

/// Restore an expired account. Contract: `state.reactivate`.
pub fn reactivate(
    trie: &mut AccountTrie,
    record: &ExpiryRecord,
    exclusion: &MptProof,
) -> Result<Account, ExpiryError> {
    let key = account_key(&record.address);
    if trie.get(&record.address).is_some() {
        return Err(ExpiryError::StillPresent);
    }
    if record.inclusion.value.as_deref() != Some(record.encoded.as_slice()) {
        return Err(ExpiryError::Proof);
    }
    if !verify(&key, &record.inclusion, &record.root) {
        return Err(ExpiryError::Proof);
    }
    let live_root = trie.root();
    let ex = prove_exclusion(trie.as_trie(), &key).ok_or(ExpiryError::Proof)?;
    if !verify(&key, exclusion, &live_root) || exclusion != &ex {
        return Err(ExpiryError::Proof);
    }
    let account = Account::decode(&record.encoded).map_err(|_| ExpiryError::Proof)?;
    trie.put(&record.address, &account);
    Ok(account)
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Amount, Hash, Nonce};

    fn acc() -> Account {
        Account {
            balance: Amount::new(7),
            nonce: Nonce(1),
            code_hash: Hash::ZERO,
        }
    }

    #[test]
    fn expire_removes_live_account_keeps_inclusion_proof() {
        let addr = Address::from_bytes([1u8; 32]);
        let mut trie = AccountTrie::new();
        trie.put(&addr, &acc());
        let rec = expire(&mut trie, &addr, 10, 0).unwrap();
        assert!(trie.get(&addr).is_none());
        assert!(verify(&account_key(&addr), &rec.inclusion, &rec.root));
        assert_eq!(Account::decode(&rec.encoded).unwrap(), acc());
        assert!(expire(&mut trie, &addr, 1, 5).is_err());
        let mut paid = AccountTrie::new();
        paid.put(&addr, &acc());
        assert_eq!(expire(&mut paid, &addr, 3, 3), Err(ExpiryError::StillPaid));
    }

    #[test]
    fn reactivate_happy_and_tampered_proof() {
        let addr = Address::from_bytes([2u8; 32]);
        let mut trie = AccountTrie::new();
        trie.put(&addr, &acc());
        let rec = expire(&mut trie, &addr, 9, 0).unwrap();
        let excl = prove_exclusion(trie.as_trie(), &account_key(&addr)).unwrap();
        reactivate(&mut trie, &rec, &excl).unwrap();
        assert_eq!(trie.get(&addr).unwrap(), acc());

        trie.delete(&addr);
        let excl2 = prove_exclusion(trie.as_trie(), &account_key(&addr)).unwrap();
        let mut bad = rec.clone();
        bad.encoded[8] ^= 1;
        assert_eq!(reactivate(&mut trie, &bad, &excl2), Err(ExpiryError::Proof));
    }
}
