//! Block locator and header-then-body catch-up (architecture.md §5).
//!
//! Compact locator: most-recent heights dense, older exponential (Bitcoin-style).
//! Headers are stored with `store.header.put` (`gossip.headers_first`); complete
//! blocks with `store.block.put`. Snapshot sync is Tier 14.

use crate::blocks::{accept_body_after_header, accept_header};
use crate::topics::TopicError;
use storage::blocks::{get_header, put_block, tip};
use storage::kv::Store;
use types::block::Block;
use types::header::Header;
use types::{Hash, Height, TypesError};

/// Sparse list of known header hashes, tip-first. Contract: `sync.locator`.
pub fn locator<S: Store>(store: &S) -> Result<Vec<Hash>, TypesError> {
    let Some(tip_h) = tip(store)? else {
        return Ok(Vec::new());
    };
    let mut heights = Vec::new();
    let mut index = tip_h.0;
    let mut step = 1u64;
    loop {
        heights.push(Height(index));
        if index == 0 {
            break;
        }
        if heights.len() >= 10 {
            step = step.saturating_mul(2).max(1);
        }
        index = index.saturating_sub(step.min(index));
        if heights.len() > 32 {
            break;
        }
    }
    let mut out = Vec::new();
    for h in heights {
        if let Some(header) = get_header(store, h)? {
            out.push(header.hash());
        }
    }
    Ok(out)
}

/// A remote block offer after headers-first.
#[derive(Clone, Debug)]
pub struct BodyOffer {
    /// Header (must match the stored headers-first header).
    pub header: Header,
    /// Body.
    pub block: Block,
    /// `exec.receipt` encodings (may be empty).
    pub receipts: Vec<Vec<u8>>,
    /// Frozen `exec.app_hash`.
    pub app_hash: Hash,
}

/// Fetch missing headers then bodies. Contract: `sync.headers_then_bodies`.
pub fn headers_then_bodies<S: Store>(
    local: &mut S,
    remote_headers: &[Header],
    bodies: &[BodyOffer],
) -> Result<Option<Height>, TopicError> {
    let _ = locator(local).map_err(|_| TopicError::Codec)?;
    for h in remote_headers {
        if get_header(local, h.fields.height)
            .map_err(|_| TopicError::Codec)?
            .is_none()
        {
            accept_header(local, h).map_err(|_| TopicError::Codec)?;
        }
    }
    for offer in bodies {
        accept_body_after_header(&offer.header, &offer.block, &offer.receipts)?;
        put_block(
            local,
            &offer.header,
            &offer.block,
            &offer.receipts,
            &offer.app_hash,
        )
        .map_err(|_| TopicError::Codec)?;
    }
    tip(local).map_err(|_| TopicError::Codec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use state::merkle;
    use storage::blocks::{put_block as store_put_block, put_header};
    use storage::memory::MemoryStore;
    use types::header::{HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{Hash, Height, Round, TestClock, ValidatorId};

    fn header_at(height: u64) -> Header {
        let clock = TestClock::new(1_000 + height);
        let fields = HeaderFields::new(
            &clock,
            Height(height),
            Round::ZERO,
            ValidatorId::ZERO,
            0,
            1 + height,
        )
        .unwrap();
        let empty = Hash::from_bytes(merkle::compute_root(&[]));
        Header {
            fields,
            tx_root: empty,
            state_root: Hash::from_bytes([height as u8; 32]),
            receipts_root: empty,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        }
    }

    fn empty_block(header: &Header) -> Block {
        Block {
            header_fields: header.fields.clone(),
            txs: vec![],
        }
    }

    #[test]
    fn locator_is_tip_dense_then_sparse() {
        let mut store = MemoryStore::default();
        for h in 0..=20 {
            put_header(&mut store, &header_at(h)).unwrap();
        }
        // tip is only updated by put_block; put headers then set tip via last block
        let last = header_at(20);
        store_put_block(&mut store, &last, &empty_block(&last), &[], &Hash::ZERO).unwrap();
        let loc = locator(&store).unwrap();
        assert_eq!(loc.first().copied(), Some(last.hash()));
        assert!(loc.len() > 3);
        assert!(loc.len() <= 32);
        let mut store2 = MemoryStore::default();
        put_header(&mut store2, &header_at(0)).unwrap();
        store_put_block(
            &mut store2,
            &header_at(0),
            &empty_block(&header_at(0)),
            &[],
            &Hash::ZERO,
        )
        .unwrap();
        assert_eq!(locator(&store2).unwrap().len(), 1);
    }

    #[test]
    fn late_node_catches_up_from_genesis() {
        let mut source = MemoryStore::default();
        let mut late = MemoryStore::default();
        let mut headers = Vec::new();
        let mut bodies = Vec::new();
        for h in 0..=4 {
            let header = header_at(h);
            let block = empty_block(&header);
            store_put_block(&mut source, &header, &block, &[], &Hash::ZERO).unwrap();
            headers.push(header.clone());
            bodies.push(BodyOffer {
                header,
                block,
                receipts: vec![],
                app_hash: Hash::ZERO,
            });
        }
        // late node: genesis only
        let g = header_at(0);
        store_put_block(&mut late, &g, &empty_block(&g), &[], &Hash::ZERO).unwrap();
        assert_eq!(tip(&late).unwrap(), Some(Height(0)));
        let loc = locator(&late).unwrap();
        assert_eq!(loc, vec![g.hash()]);
        let tip_h = headers_then_bodies(&mut late, &headers, &bodies)
            .unwrap()
            .unwrap();
        assert_eq!(tip_h, Height(4));
        assert_eq!(tip(&late).unwrap(), tip(&source).unwrap());
        assert_eq!(
            get_header(&late, Height(4)).unwrap().unwrap().hash(),
            header_at(4).hash()
        );
    }
}
