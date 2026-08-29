//! Block body and Merkle roots (architecture.md §4).
//!
//! # Frozen leaf encodings
//!
//! - `block.tx_root`: Merkle over **canonical `Tx::encode()` bytes** (not a pre-hash).
//! - `block.receipts_root`: Merkle over **canonical receipt encodings** from `exec.receipt`.
//! - `block.state_root`: two-leaf Merkle of `(account_trie_root, contract_storage_root)`
//!   — same as `state.commit_root`.

use crate::hashing::merkle_root;
use crate::header::HeaderFields;
use crate::tx::{SignedTx, Tx};
use crate::Hash;

/// Tx Merkle root. Contract: `block.tx_root`.
pub fn tx_root(txs: &[Tx]) -> Hash {
    let leaves: Vec<Vec<u8>> = txs.iter().map(|t| t.encode()).collect();
    Hash::from_bytes(merkle_root(&leaves))
}

/// Tx root from signed bodies (envelope only).
pub fn tx_root_signed(txs: &[SignedTx]) -> Hash {
    tx_root(&txs.iter().map(|s| s.tx.clone()).collect::<Vec<_>>())
}

/// Receipts Merkle root. Contract: `block.receipts_root`.
pub fn receipts_root(encoded_receipts: &[Vec<u8>]) -> Hash {
    Hash::from_bytes(merkle_root(encoded_receipts))
}

/// State root from the two trie roots. Contract: `block.state_root`.
///
/// Callers must pass the roots from `AccountTrie::root` / `ContractStorageTrie::root`
/// (or `state::root::commit_root` of those bytes).
pub fn state_root(account_root: &[u8; 32], contract_root: &[u8; 32]) -> Hash {
    Hash::from_bytes(merkle_root(&[
        account_root.to_vec(),
        contract_root.to_vec(),
    ]))
}

/// Block as gossiped/stored. Contract: `block.body`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// Header fields (roots filled after execution or by the proposer).
    pub header_fields: HeaderFields,
    /// Ordered transactions.
    pub txs: Vec<SignedTx>,
}

impl Block {
    /// Envelopes in order.
    pub fn envelopes(&self) -> Vec<Tx> {
        self.txs.iter().map(|s| s.tx.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::HeaderFields;
    use crate::{Address, Amount, ChainId, Height, Nonce, Round, TestClock, ValidatorId};

    #[test]
    fn tx_root_empty_and_order() {
        let empty = tx_root(&[]);
        let tx = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            21_000,
            Amount::ZERO,
            Address::ZERO,
            Amount::ZERO,
        );
        let one = tx_root(std::slice::from_ref(&tx));
        assert_ne!(empty, one);
        let two = tx_root(&[tx.clone(), tx.clone()]);
        assert_ne!(one, two);
    }

    #[test]
    fn body_holds_header_and_txs() {
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
        let b = Block {
            header_fields: fields,
            txs: vec![],
        };
        assert!(b.envelopes().is_empty());
    }
}
