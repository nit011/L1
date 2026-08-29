//! Header-first block propagation (architecture.md §5 Block/tx propagation).
//!
//! Headers are stored via Tier 4 `store.header.put` as they arrive. Bodies are
//! fetched and checked separately via [`crate::topics::ingest_block`].

use crate::codec::decode_header;
use crate::topics::{ingest_block, TopicError};
use storage::blocks::put_header;
use storage::kv::Store;
use types::block::Block;
use types::header::Header;
use types::TypesError;

/// Accept a header and persist it. Contract: `gossip.headers_first`.
pub fn accept_header<S: Store>(store: &mut S, header: &Header) -> Result<(), TypesError> {
    let _ = header.hash();
    put_header(store, header)
}

/// Decode gossip bytes, then [`accept_header`].
pub fn accept_header_bytes<S: Store>(store: &mut S, inner: &[u8]) -> Result<Header, TopicError> {
    let header = decode_header(inner).map_err(|_| TopicError::Codec)?;
    accept_header(store, &header).map_err(|_| TopicError::Codec)?;
    Ok(header)
}

/// After the header is stored, accept a body that matches it (`gossip.block`).
pub fn accept_body_after_header(
    header: &Header,
    block: &Block,
    receipt_leaves: &[Vec<u8>],
) -> Result<(), TopicError> {
    ingest_block(header, block, receipt_leaves)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::encode_header;
    use state::merkle;
    use storage::blocks::get_header;
    use storage::memory::MemoryStore;
    use types::header::{HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{Hash, Height, Round, TestClock, ValidatorId};

    fn empty_header() -> Header {
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
        let empty = Hash::from_bytes(merkle::compute_root(&[]));
        Header {
            fields,
            tx_root: empty,
            state_root: Hash::ZERO,
            receipts_root: empty,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        }
    }

    #[test]
    fn headers_first_stores_without_body() {
        let h = empty_header();
        let mut store = MemoryStore::default();
        accept_header(&mut store, &h).unwrap();
        let got = get_header(&store, Height::GENESIS).unwrap().unwrap();
        assert_eq!(got.hash(), h.hash());
        assert!(storage::blocks::get_block(&store, Height::GENESIS)
            .unwrap()
            .is_none());
    }

    #[test]
    fn malformed_header_bytes_rejected() {
        let mut store = MemoryStore::default();
        assert!(accept_header_bytes(&mut store, &[1, 2, 3]).is_err());
        let h = empty_header();
        accept_header_bytes(&mut store, &encode_header(&h)).unwrap();
        let block = Block {
            header_fields: h.fields.clone(),
            txs: vec![],
        };
        accept_body_after_header(&h, &block, &[]).unwrap();
    }
}
