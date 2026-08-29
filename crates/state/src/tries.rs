//! Separate MPTs for accounts and contract storage (architecture.md §4.1).

use crate::account::{account_key, Account};
use crate::mpt::Trie;
use types::Address;

/// Accounts trie: `address → Account`. Contract: `state.account_trie`.
#[derive(Clone, Debug, Default)]
pub struct AccountTrie {
    trie: Trie,
}

impl AccountTrie {
    /// Empty accounts trie.
    pub fn new() -> Self {
        Self { trie: Trie::new() }
    }

    /// Insert or replace an account.
    pub fn put(&mut self, addr: &Address, account: &Account) {
        crate::mpt::put(&mut self.trie, &account_key(addr), account.encode());
    }

    /// Lookup an account.
    pub fn get(&self, addr: &Address) -> Option<Account> {
        crate::mpt::get(&self.trie, &account_key(addr)).and_then(|b| Account::decode(&b).ok())
    }

    /// Root of this trie only.
    pub fn root(&self) -> [u8; 32] {
        self.trie.root()
    }
}

/// Contract storage trie: `storage_key → value` (independent of accounts).
/// Contract: `state.contract_storage_trie`.
#[derive(Clone, Debug, Default)]
pub struct ContractStorageTrie {
    trie: Trie,
}

impl ContractStorageTrie {
    /// Empty contract-storage trie.
    pub fn new() -> Self {
        Self { trie: Trie::new() }
    }

    /// Write a storage slot.
    pub fn put(&mut self, key: &[u8], value: Vec<u8>) {
        crate::mpt::put(&mut self.trie, key, value);
    }

    /// Read a storage slot.
    pub fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        crate::mpt::get(&self.trie, key)
    }

    /// Root of this trie only.
    pub fn root(&self) -> [u8; 32] {
        self.trie.root()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Amount, Hash, Nonce};

    #[test]
    fn independent_roots() {
        let mut accounts = AccountTrie::new();
        let mut storage = ContractStorageTrie::new();
        let addr = Address::from_bytes([1u8; 32]);
        accounts.put(
            &addr,
            &Account {
                balance: Amount::new(1),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        storage.put(b"slot", b"v".to_vec());
        assert_ne!(accounts.root(), storage.root());
        let acc_root = accounts.root();
        storage.put(b"other", b"x".to_vec());
        assert_eq!(accounts.root(), acc_root);
        assert!(accounts.get(&Address::ZERO).is_none());
        assert!(storage.get(b"missing").is_none());
    }
}
