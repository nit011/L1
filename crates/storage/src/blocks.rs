//! Persist headers and full blocks (architecture.md §4).
//!
//! `store.block.put` uses the same header ops as `store.header.put` and a
//! single `kv.batch` so header, body, and indexes commit together.

use crate::codec::{
    app_hash_key, block_key, encode_block_body, header_from_preimage, header_hash_key,
    header_height_key, KEY_GENESIS, KEY_TIP,
};
use crate::index::{receipt_put_ops, tx_index_ops};
use crate::kv::{BatchOp, Store};
use types::block::Block;
use types::genesis::Genesis;
use types::header::Header;
use types::{Hash, Height, TypesError};

/// Header write ops. Contract: `store.header.put`.
///
/// Calls `Header::hash` (`header.hash`) for the hash→height index.
pub fn put_header_ops(header: &Header) -> Vec<BatchOp> {
    let hash = header.hash();
    let height = header.fields.height;
    vec![
        BatchOp::Put {
            key: header_height_key(height),
            value: header.hash_preimage(),
        },
        BatchOp::Put {
            key: header_hash_key(&hash),
            value: height.0.to_be_bytes().to_vec(),
        },
    ]
}

/// Persist a header via `kv.batch`. Contract: `store.header.put`.
pub fn put_header<S: Store>(store: &mut S, header: &Header) -> Result<(), TypesError> {
    store.apply_batch(&put_header_ops(header))
}

/// Load a header by height.
pub fn get_header<S: Store>(store: &S, height: Height) -> Result<Option<Header>, TypesError> {
    match store.get(&header_height_key(height))? {
        None => Ok(None),
        Some(bytes) => Ok(Some(header_from_preimage(&bytes)?)),
    }
}

/// Height recorded under `header.hash`.
pub fn height_by_header_hash<S: Store>(
    store: &S,
    hash: &Hash,
) -> Result<Option<Height>, TypesError> {
    match store.get(&header_hash_key(hash))? {
        None => Ok(None),
        Some(b) if b.len() == 8 => Ok(Some(Height(u64::from_be_bytes(
            b.as_slice().try_into().unwrap(),
        )))),
        Some(_) => Err(TypesError::Kv("corrupt header-hash index")),
    }
}

/// Block + header + index ops in one batch. Contract: `store.block.put`.
///
/// Invokes [`put_header_ops`] (the `store.header.put` write path). Receipt
/// bytes must be `exec.receipt` encodings (`Receipt::encode`).
pub fn put_block_ops(
    header: &Header,
    block: &Block,
    receipt_encodings: &[Vec<u8>],
    app_hash: &Hash,
) -> Vec<BatchOp> {
    debug_assert_eq!(block.txs.len(), receipt_encodings.len());
    let mut ops = put_header_ops(header);
    ops.push(BatchOp::Put {
        key: block_key(header.fields.height),
        value: encode_block_body(block),
    });
    ops.extend(tx_index_ops(header.fields.height, block));
    ops.extend(receipt_put_ops(block, receipt_encodings));
    ops.push(BatchOp::Put {
        key: app_hash_key(header.fields.height),
        value: app_hash.as_bytes().to_vec(),
    });
    ops.push(BatchOp::Put {
        key: KEY_TIP.to_vec(),
        value: header.fields.height.0.to_be_bytes().to_vec(),
    });
    ops
}

/// Persist a full block atomically. Contract: `store.block.put`.
pub fn put_block<S: Store>(
    store: &mut S,
    header: &Header,
    block: &Block,
    receipt_encodings: &[Vec<u8>],
    app_hash: &Hash,
) -> Result<(), TypesError> {
    store.apply_batch(&put_block_ops(header, block, receipt_encodings, app_hash))
}

/// Load a block body by height.
pub fn get_block<S: Store>(store: &S, height: Height) -> Result<Option<Block>, TypesError> {
    match store.get(&block_key(height))? {
        None => Ok(None),
        Some(bytes) => Ok(Some(crate::codec::decode_block_body(&bytes)?)),
    }
}

/// Stored `exec.app_hash` for a height.
pub fn get_app_hash<S: Store>(store: &S, height: Height) -> Result<Option<Hash>, TypesError> {
    match store.get(&app_hash_key(height))? {
        None => Ok(None),
        Some(b) if b.len() == 32 => {
            let mut a = [0u8; 32];
            a.copy_from_slice(&b);
            Ok(Some(Hash::from_bytes(a)))
        }
        Some(_) => Err(TypesError::Kv("corrupt app_hash")),
    }
}

/// Highest committed height, if any.
pub fn tip<S: Store>(store: &S) -> Result<Option<Height>, TypesError> {
    match store.get(KEY_TIP)? {
        None => Ok(None),
        Some(b) if b.len() == 8 => Ok(Some(Height(u64::from_be_bytes(
            b.as_slice().try_into().unwrap(),
        )))),
        Some(_) => Err(TypesError::Kv("corrupt tip")),
    }
}

/// Record `genesis.hash` (replay anchor).
pub fn put_genesis_hash<S: Store>(store: &mut S, genesis: &Genesis) -> Result<(), TypesError> {
    store.apply_batch(&[BatchOp::Put {
        key: KEY_GENESIS.to_vec(),
        value: genesis.hash().as_bytes().to_vec(),
    }])
}

/// Stored genesis digest.
pub fn get_genesis_hash<S: Store>(store: &S) -> Result<Option<Hash>, TypesError> {
    match store.get(KEY_GENESIS)? {
        None => Ok(None),
        Some(b) if b.len() == 32 => {
            let mut a = [0u8; 32];
            a.copy_from_slice(&b);
            Ok(Some(Hash::from_bytes(a)))
        }
        Some(_) => Err(TypesError::Kv("corrupt genesis hash")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryStore;
    use types::header::{HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{ChainId, Round, TestClock, ValidatorId};

    fn sample_header() -> Header {
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
        Header {
            fields,
            tx_root: Hash::ZERO,
            state_root: Hash::from_bytes([9u8; 32]),
            receipts_root: Hash::ZERO,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        }
    }

    #[test]
    fn put_header_round_trip_and_hash_index() {
        let mut s = MemoryStore::new();
        let h = sample_header();
        put_header(&mut s, &h).unwrap();
        let loaded = get_header(&s, Height::GENESIS).unwrap().unwrap();
        assert_eq!(loaded, h);
        assert_eq!(
            height_by_header_hash(&s, &h.hash()).unwrap(),
            Some(Height::GENESIS)
        );
    }

    #[test]
    fn get_header_missing() {
        let s = MemoryStore::new();
        assert!(get_header(&s, Height::GENESIS).unwrap().is_none());
    }

    #[test]
    fn put_block_uses_header_ops_and_stores_body() {
        let mut s = MemoryStore::new();
        let header = sample_header();
        let block = Block {
            header_fields: header.fields.clone(),
            txs: vec![],
        };
        let app = Hash::from_bytes([3u8; 32]);
        put_block(&mut s, &header, &block, &[], &app).unwrap();
        assert_eq!(get_header(&s, Height::GENESIS).unwrap().unwrap(), header);
        assert_eq!(get_block(&s, Height::GENESIS).unwrap().unwrap(), block);
        assert_eq!(get_app_hash(&s, Height::GENESIS).unwrap(), Some(app));
        assert_eq!(tip(&s).unwrap(), Some(Height::GENESIS));
    }

    #[test]
    fn genesis_hash_matches_types() {
        let mut s = MemoryStore::new();
        let g = Genesis::new(ChainId::new(1));
        put_genesis_hash(&mut s, &g).unwrap();
        assert_eq!(get_genesis_hash(&s).unwrap(), Some(g.hash()));
    }
}
