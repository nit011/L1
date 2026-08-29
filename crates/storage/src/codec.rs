//! Canonical encodings for stored headers, block bodies, and signed txs.
//!
//! Uses Tier 0 `encoding.canonical.encode` and `Tx::encode` (`tx.envelope`).

use types::encoding::{decode, encode};
use types::header::{Header, HeaderFields};
use types::tx::{SignedTx, Tx};
use types::{Hash, Height, Round, TypesError, ValidatorId};

/// Prefix: header by height. Contract `store.header.put`.
pub const PREFIX_HEADER_HEIGHT: &[u8] = b"h/";
/// Prefix: height by `header.hash`.
pub const PREFIX_HEADER_HASH: &[u8] = b"H/";
/// Prefix: block body by height. Contract `store.block.put`.
pub const PREFIX_BLOCK: &[u8] = b"b/";
/// Prefix: tx hash → (height, index). Contract `store.tx.by_hash`.
pub const PREFIX_TX: &[u8] = b"t/";
/// Prefix: tx hash → receipt bytes. Contract `store.receipt.put`.
pub const PREFIX_RECEIPT: &[u8] = b"r/";
/// Prefix: app_hash by height.
pub const PREFIX_APP: &[u8] = b"a/";
/// Latest committed height.
pub const KEY_TIP: &[u8] = b"m/tip";
/// `genesis.hash` bytes.
pub const KEY_GENESIS: &[u8] = b"m/genesis";
/// Execution WAL record. Contract `wal.execution`.
pub const KEY_WAL: &[u8] = b"w/exec";

/// Key `h/{height}`.
pub fn header_height_key(height: Height) -> Vec<u8> {
    let mut k = PREFIX_HEADER_HEIGHT.to_vec();
    k.extend_from_slice(&height.0.to_be_bytes());
    k
}

/// Key `H/{header.hash}`.
pub fn header_hash_key(hash: &Hash) -> Vec<u8> {
    let mut k = PREFIX_HEADER_HASH.to_vec();
    k.extend_from_slice(hash.as_bytes());
    k
}

/// Key `b/{height}`.
pub fn block_key(height: Height) -> Vec<u8> {
    let mut k = PREFIX_BLOCK.to_vec();
    k.extend_from_slice(&height.0.to_be_bytes());
    k
}

/// Key `t/{tx_hash}`.
pub fn tx_index_key(tx_hash: &Hash) -> Vec<u8> {
    let mut k = PREFIX_TX.to_vec();
    k.extend_from_slice(tx_hash.as_bytes());
    k
}

/// Key `r/{tx_hash}`.
pub fn receipt_key(tx_hash: &Hash) -> Vec<u8> {
    let mut k = PREFIX_RECEIPT.to_vec();
    k.extend_from_slice(tx_hash.as_bytes());
    k
}

/// Key `a/{height}`.
pub fn app_hash_key(height: Height) -> Vec<u8> {
    let mut k = PREFIX_APP.to_vec();
    k.extend_from_slice(&height.0.to_be_bytes());
    k
}

/// BLAKE3 of canonical `Tx::encode()` (same digest width as `types.hash`).
pub fn tx_hash(tx: &Tx) -> Hash {
    Hash::from_bytes(types::hashing::blake3_array(&tx.encode()))
}

const HEADER_PREIMAGE_LEN: usize = 8 + 4 + 48 + 8 + 32 * 5;

/// Reconstruct a header from [`Header::hash_preimage`].
pub fn header_from_preimage(p: &[u8]) -> Result<Header, TypesError> {
    if p.len() != HEADER_PREIMAGE_LEN {
        return Err(TypesError::BadLength {
            expected: HEADER_PREIMAGE_LEN,
            actual: p.len(),
        });
    }
    let height = Height(u64::from_be_bytes(p[0..8].try_into().unwrap()));
    let round = Round(u32::from_be_bytes(p[8..12].try_into().unwrap()));
    let mut proposer = [0u8; 48];
    proposer.copy_from_slice(&p[12..60]);
    let timestamp_ms = u64::from_be_bytes(p[60..68].try_into().unwrap());
    let mut tx_root = [0u8; 32];
    tx_root.copy_from_slice(&p[68..100]);
    let mut state_root = [0u8; 32];
    state_root.copy_from_slice(&p[100..132]);
    let mut receipts_root = [0u8; 32];
    receipts_root.copy_from_slice(&p[132..164]);
    let mut validators_hash = [0u8; 32];
    validators_hash.copy_from_slice(&p[164..196]);
    let mut da_root = [0u8; 32];
    da_root.copy_from_slice(&p[196..228]);
    Ok(Header {
        fields: HeaderFields {
            height,
            round,
            proposer: ValidatorId::from_bytes(proposer),
            timestamp_ms,
        },
        tx_root: Hash::from_bytes(tx_root),
        state_root: Hash::from_bytes(state_root),
        receipts_root: Hash::from_bytes(receipts_root),
        validators_hash: Hash::from_bytes(validators_hash),
        da_root: Hash::from_bytes(da_root),
    })
}

/// Encode a signed envelope for the block body.
pub fn encode_signed_tx(s: &SignedTx) -> Vec<u8> {
    let t = s.tx.encode();
    let mut p = Vec::with_capacity(4 + t.len() + 64 + 32);
    p.extend_from_slice(&(t.len() as u32).to_be_bytes());
    p.extend_from_slice(&t);
    p.extend_from_slice(&s.signature);
    p.extend_from_slice(&s.public_key);
    encode(&p)
}

/// Inverse of [`encode_signed_tx`].
pub fn decode_signed_tx(buf: &[u8]) -> Result<SignedTx, TypesError> {
    let p = decode(buf)?;
    if p.len() < 4 + 64 + 32 {
        return Err(TypesError::BadLength {
            expected: 100,
            actual: p.len(),
        });
    }
    let tlen = u32::from_be_bytes(p[0..4].try_into().unwrap()) as usize;
    let need = 4 + tlen + 64 + 32;
    if p.len() != need {
        return Err(TypesError::BadLength {
            expected: need,
            actual: p.len(),
        });
    }
    let tx = Tx::decode(&p[4..4 + tlen])?;
    let mut signature = [0u8; 64];
    signature.copy_from_slice(&p[4 + tlen..4 + tlen + 64]);
    let mut public_key = [0u8; 32];
    public_key.copy_from_slice(&p[4 + tlen + 64..need]);
    Ok(SignedTx {
        tx,
        signature,
        public_key,
    })
}

/// Encode ordered txs plus header fields (contract `block.body`).
pub fn encode_block_body(block: &types::block::Block) -> Vec<u8> {
    let f = &block.header_fields;
    let mut p = Vec::new();
    p.extend_from_slice(&f.height.0.to_be_bytes());
    p.extend_from_slice(&f.round.0.to_be_bytes());
    p.extend_from_slice(f.proposer.as_bytes());
    p.extend_from_slice(&f.timestamp_ms.to_be_bytes());
    p.extend_from_slice(&(block.txs.len() as u32).to_be_bytes());
    for tx in &block.txs {
        let e = encode_signed_tx(tx);
        p.extend_from_slice(&(e.len() as u32).to_be_bytes());
        p.extend_from_slice(&e);
    }
    encode(&p)
}

/// Inverse of [`encode_block_body`].
pub fn decode_block_body(buf: &[u8]) -> Result<types::block::Block, TypesError> {
    let p = decode(buf)?;
    if p.len() < 8 + 4 + 48 + 8 + 4 {
        return Err(TypesError::BadLength {
            expected: 72,
            actual: p.len(),
        });
    }
    let height = Height(u64::from_be_bytes(p[0..8].try_into().unwrap()));
    let round = Round(u32::from_be_bytes(p[8..12].try_into().unwrap()));
    let mut proposer = [0u8; 48];
    proposer.copy_from_slice(&p[12..60]);
    let timestamp_ms = u64::from_be_bytes(p[60..68].try_into().unwrap());
    let n = u32::from_be_bytes(p[68..72].try_into().unwrap()) as usize;
    let mut i = 72;
    let mut txs = Vec::with_capacity(n);
    for _ in 0..n {
        if i + 4 > p.len() {
            return Err(TypesError::CodecTruncated);
        }
        let elen = u32::from_be_bytes(p[i..i + 4].try_into().unwrap()) as usize;
        i += 4;
        if i + elen > p.len() {
            return Err(TypesError::CodecTruncated);
        }
        txs.push(decode_signed_tx(&p[i..i + elen])?);
        i += elen;
    }
    if i != p.len() {
        return Err(TypesError::CodecLength {
            expected: i,
            actual: p.len(),
        });
    }
    Ok(types::block::Block {
        header_fields: HeaderFields {
            height,
            round,
            proposer: ValidatorId::from_bytes(proposer),
            timestamp_ms,
        },
        txs,
    })
}

/// `(height BE || index BE)` for the tx secondary index.
pub fn encode_tx_locator(height: Height, index: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(12);
    v.extend_from_slice(&height.0.to_be_bytes());
    v.extend_from_slice(&index.to_be_bytes());
    v
}

/// Inverse of [`encode_tx_locator`].
pub fn decode_tx_locator(buf: &[u8]) -> Result<(Height, u32), TypesError> {
    if buf.len() != 12 {
        return Err(TypesError::BadLength {
            expected: 12,
            actual: buf.len(),
        });
    }
    Ok((
        Height(u64::from_be_bytes(buf[0..8].try_into().unwrap())),
        u32::from_be_bytes(buf[8..12].try_into().unwrap()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::block::Block;
    use types::{Address, Amount, ChainId, Nonce, TestClock, GAS_TRANSFER};

    #[test]
    fn header_preimage_round_trip() {
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
        let h = Header {
            fields,
            tx_root: Hash::from_bytes([1u8; 32]),
            state_root: Hash::from_bytes([2u8; 32]),
            receipts_root: Hash::from_bytes([3u8; 32]),
            validators_hash: Hash::from_bytes([4u8; 32]),
            da_root: types::header::DA_ROOT_PLACEHOLDER,
        };
        let back = header_from_preimage(&h.hash_preimage()).unwrap();
        assert_eq!(back, h);
        assert_eq!(back.hash(), h.hash());
    }

    #[test]
    fn block_body_round_trip_empty() {
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
        assert_eq!(decode_block_body(&encode_block_body(&b)).unwrap(), b);
    }

    #[test]
    fn signed_tx_round_trip() {
        let tx = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            Address::ZERO,
            Amount::new(2),
        );
        let s = SignedTx {
            tx,
            signature: [7u8; 64],
            public_key: [8u8; 32],
        };
        assert_eq!(decode_signed_tx(&encode_signed_tx(&s)).unwrap(), s);
    }
}
