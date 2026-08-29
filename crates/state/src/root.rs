//! Combined state root published in the block header (architecture.md §4.1).
//!
//! # Consensus-critical combination
//!
//! `state_root = merkle.compute_root([account_trie_root, contract_storage_trie_root])`
//! where each leaf is the 32-byte trie root (not re-hashed before the leaf
//! tag inside the generic Merkle tree).
//!
//! This is a 2-leaf generic Merkle tree using domain tag `merkle` (see
//! `crates/state/src/merkle.rs`). Changing this formula forks the chain.
//! Do not silently switch to concatenation, XOR, or an untagged BLAKE3.

use crate::merkle::compute_root;
use crate::tries::{AccountTrie, ContractStorageTrie};

/// Combine the two independent trie roots. Contract: `state.commit_root`.
pub fn commit_root(account_root: &[u8; 32], contract_root: &[u8; 32]) -> [u8; 32] {
    compute_root(&[account_root.to_vec(), contract_root.to_vec()])
}

/// Convenience: roots from live tries.
pub fn commit_tries(accounts: &AccountTrie, storage: &ContractStorageTrie) -> [u8; 32] {
    commit_root(&accounts.root(), &storage.root())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::Account;
    use types::{Address, Amount, Hash, Nonce};

    #[test]
    fn combination_is_order_sensitive_and_deterministic() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        assert_eq!(commit_root(&a, &b), commit_root(&a, &b));
        assert_ne!(commit_root(&a, &b), commit_root(&b, &a));
        assert_ne!(commit_root(&a, &b), commit_root(&a, &a));
    }

    #[test]
    fn live_tries() {
        let mut accounts = AccountTrie::new();
        let mut storage = ContractStorageTrie::new();
        let empty = commit_tries(&accounts, &storage);
        accounts.put(
            &Address::from_bytes([9u8; 32]),
            &Account {
                balance: Amount::new(1),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        assert_ne!(commit_tries(&accounts, &storage), empty);
        storage.put(b"k", b"v".to_vec());
        assert_ne!(commit_tries(&accounts, &storage), empty);
    }
}
