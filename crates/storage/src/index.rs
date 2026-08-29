//! Secondary indexes for txs and receipts (architecture.md §4).
//!
//! Built in the same `kv.batch` as `store.block.put` so a block never exists
//! without its indexes (and vice versa).

use crate::codec::{decode_tx_locator, encode_tx_locator, receipt_key, tx_hash, tx_index_key};
use crate::kv::{BatchOp, Store};
use types::block::Block;
use types::tx::SignedTx;
use types::{Hash, Height, TypesError};

use crate::blocks;

/// Tx-hash index ops. Contract: `store.tx.by_hash`.
///
/// Keys use `tx.envelope` canonical bytes via [`tx_hash`].
pub fn tx_index_ops(height: Height, block: &Block) -> Vec<BatchOp> {
    block
        .txs
        .iter()
        .enumerate()
        .map(|(i, signed)| BatchOp::Put {
            key: tx_index_key(&tx_hash(&signed.tx)),
            value: encode_tx_locator(height, i as u32),
        })
        .collect()
}

/// Receipt index ops. Contract: `store.receipt.put`.
///
/// `encoded[i]` must be `execution::receipt::Receipt::encode()` (`exec.receipt`).
pub fn receipt_put_ops(block: &Block, encoded: &[Vec<u8>]) -> Vec<BatchOp> {
    block
        .txs
        .iter()
        .zip(encoded.iter())
        .map(|(signed, rec)| BatchOp::Put {
            key: receipt_key(&tx_hash(&signed.tx)),
            value: rec.clone(),
        })
        .collect()
}

/// Look up a tx by envelope hash. Contract: `store.tx.by_hash`.
pub fn get_tx_by_hash<S: Store>(
    store: &S,
    hash: &Hash,
) -> Result<Option<(Height, u32, SignedTx)>, TypesError> {
    let Some(loc) = store.get(&tx_index_key(hash))? else {
        return Ok(None);
    };
    let (height, index) = decode_tx_locator(&loc)?;
    let Some(block) = blocks::get_block(store, height)? else {
        return Err(TypesError::Kv("tx index without block"));
    };
    let signed = block
        .txs
        .get(index as usize)
        .cloned()
        .ok_or(TypesError::Kv("tx index out of range"))?;
    if tx_hash(&signed.tx) != *hash {
        return Err(TypesError::Kv("tx hash mismatch"));
    }
    Ok(Some((height, index, signed)))
}

/// Look up receipt bytes by tx hash. Contract: `store.receipt.put`.
pub fn get_receipt<S: Store>(store: &S, tx_hash_key: &Hash) -> Result<Option<Vec<u8>>, TypesError> {
    store.get(&receipt_key(tx_hash_key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::{put_block, put_header_ops};
    use crate::kv::apply_batch;
    use crate::memory::MemoryStore;
    use execution::receipt::{Receipt, RejectReason};
    use types::header::{Header, HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::tx::Tx;
    use types::{
        Address, Amount, ChainId, Hash, Nonce, Round, TestClock, ValidatorId, GAS_TRANSFER,
    };

    fn header_at(height: Height) -> Header {
        let clock = TestClock::new(1_000 + height.0);
        let fields = HeaderFields::new(
            &clock,
            height,
            Round::ZERO,
            ValidatorId::ZERO,
            0,
            1 + height.0,
        )
        .unwrap();
        Header {
            fields,
            tx_root: Hash::ZERO,
            state_root: Hash::ZERO,
            receipts_root: Hash::ZERO,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        }
    }

    struct FailOnTxIndex {
        inner: MemoryStore,
    }

    impl Store for FailOnTxIndex {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, TypesError> {
            self.inner.get(key)
        }
        fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), TypesError> {
            if key.starts_with(crate::codec::PREFIX_TX) {
                return Err(TypesError::Kv("simulated mid-batch failure"));
            }
            self.inner.put(key, value)
        }
        fn delete(&mut self, key: &[u8]) -> Result<(), TypesError> {
            self.inner.delete(key)
        }
        fn prefix(&self, prefix: &[u8]) -> Result<Vec<crate::kv::KvEntry>, TypesError> {
            self.inner.prefix(prefix)
        }
    }

    fn dummy_signed() -> SignedTx {
        SignedTx {
            tx: Tx::transfer(
                ChainId::new(1),
                Nonce::ZERO,
                GAS_TRANSFER,
                Amount::new(1),
                Address::ZERO,
                Amount::new(1),
            ),
            signature: [1u8; 64],
            public_key: [2u8; 32],
        }
    }

    #[test]
    fn tx_and_receipt_indexed_with_block() {
        let mut s = MemoryStore::new();
        let header = header_at(Height::GENESIS);
        let signed = dummy_signed();
        let h = tx_hash(&signed.tx);
        let block = Block {
            header_fields: header.fields.clone(),
            txs: vec![signed.clone()],
        };
        let rec = Receipt {
            success: true,
            gas_used: GAS_TRANSFER,
            events: vec![],
            reason: None,
        };
        let enc = rec.encode();
        put_block(
            &mut s,
            &header,
            &block,
            std::slice::from_ref(&enc),
            &Hash::ZERO,
        )
        .unwrap();
        let got = get_tx_by_hash(&s, &h).unwrap().unwrap();
        assert_eq!(got.0, Height::GENESIS);
        assert_eq!(got.2, signed);
        assert_eq!(
            get_receipt(&s, &h).unwrap().as_deref(),
            Some(enc.as_slice())
        );
    }

    #[test]
    fn missing_tx_hash() {
        let s = MemoryStore::new();
        assert!(get_tx_by_hash(&s, &Hash::ZERO).unwrap().is_none());
    }

    #[test]
    fn index_and_block_roll_back_together() {
        let header = header_at(Height::GENESIS);
        let signed = dummy_signed();
        let h = tx_hash(&signed.tx);
        let block = Block {
            header_fields: header.fields.clone(),
            txs: vec![signed],
        };
        let rec = Receipt {
            success: false,
            gas_used: 0,
            events: vec![],
            reason: Some(RejectReason::WrongNonce),
        };
        let ops = crate::blocks::put_block_ops(&header, &block, &[rec.encode()], &Hash::ZERO);
        assert!(ops.len() > put_header_ops(&header).len());
        let mut s = FailOnTxIndex {
            inner: MemoryStore::new(),
        };
        let err = apply_batch(&mut s, &ops).unwrap_err();
        assert!(matches!(err, TypesError::Kv(_)));
        assert!(blocks::get_block(&s, Height::GENESIS).unwrap().is_none());
        assert!(get_tx_by_hash(&s, &h).unwrap().is_none());
        assert!(get_receipt(&s, &h).unwrap().is_none());
    }

    #[test]
    fn codec_tx_hash_uses_envelope() {
        let t = dummy_signed().tx;
        let a = tx_hash(&t);
        let b = Hash::from_bytes(types::hashing::blake3_array(&t.encode()));
        assert_eq!(a, b);
    }
}
