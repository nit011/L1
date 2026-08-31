//! Erasure-coded DA chunks (architecture.md §6).
//!
//! Pipeline: `block.body` → Reed-Solomon `k` data + `m` parity shards →
//! reconstruct from any `k` of `k+m`. Sampling lives in [`crate::das`].
//!
//! # Parameters (`k=4`, `m=2`)
//!
//! - **Fault tolerance:** any 2 of 6 shards may be missing (parity-only or mixed)
//!   and [`reconstruct`] still recovers the body (development-plan.md:
//!   "reconstruct from any k chunks").
//! - **Overhead:** 50% extra bytes versus the padded data shards. A 1:1 (`m=k`)
//!   code would double gossip load; `m=2` is enough to survive two missing
//!   chunks without paying a full extra copy of the block.
//! - **Bandwidth scaling:** shard *count* is fixed (`k+m = 6`). Shard *length*
//!   grows as `O(body_len / k)` via Tier 0 padding, so a later
//!   `limits.max_block_bytes` (Tier 14) constrains total DA traffic linearly.

use crate::rs::{self, RsError};
use storage::codec::{decode_block_body, encode_block_body};
use thiserror::Error;
use types::block::Block;

/// Data shards (`k`). Contract: `da.chunk.split`.
pub const DATA_SHARDS: usize = 4;
/// Parity shards (`m`). Contract: `da.chunk.split`.
pub const PARITY_SHARDS: usize = 2;

/// A single RS shard with its position in `0..k+m`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaShard {
    /// Index in the RS codeword (`0..k` data, `k..k+m` parity).
    pub index: u16,
    /// Equal-length shard bytes from [`rs::encode`].
    pub payload: Vec<u8>,
}

/// Chunking / reconstruction errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ChunkError {
    /// Reed-Solomon failed.
    #[error("da chunk rs: {0}")]
    Rs(#[from] RsError),
    /// Recovered bytes were not a canonical `block.body`.
    #[error("da chunk: reconstructed body is not a valid block")]
    BadBody,
    /// Wrong number of slots for `k+m`.
    #[error("da chunk: expected {expected} shard slots, got {got}")]
    Width { expected: usize, got: usize },
}

/// Split a [`Block`] (`block.body`) into `k` data + `m` parity shards.
///
/// Calls Tier 0 [`rs::encode`]. Contract: `da.chunk.split`.
pub fn split(block: &Block) -> Result<Vec<DaShard>, ChunkError> {
    let body = encode_block_body(block);
    let shards = rs::encode(&body, DATA_SHARDS, PARITY_SHARDS)?;
    Ok(shards
        .into_iter()
        .enumerate()
        .map(|(i, payload)| DaShard {
            index: i as u16,
            payload,
        })
        .collect())
}

/// Reconstruct [`Block`] from any `k` of `k+m` shards (`None` = missing).
///
/// `slots` must have length `k+m`. Calls Tier 0 [`rs::decode`].
/// Contract: `da.chunk.reconstruct`.
pub fn reconstruct(slots: &[Option<Vec<u8>>]) -> Result<Block, ChunkError> {
    let n = DATA_SHARDS + PARITY_SHARDS;
    if slots.len() != n {
        return Err(ChunkError::Width {
            expected: n,
            got: slots.len(),
        });
    }
    let bytes = rs::decode(slots, DATA_SHARDS, PARITY_SHARDS)?;
    decode_block_body(&bytes).map_err(|_| ChunkError::BadBody)
}

/// Merkle / KZG leaf bytes: `index_be16 || payload` (deterministic order).
pub fn leaf_bytes(shard: &DaShard) -> Vec<u8> {
    let mut v = Vec::with_capacity(2 + shard.payload.len());
    v.extend_from_slice(&shard.index.to_be_bytes());
    v.extend_from_slice(&shard.payload);
    v
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use types::header::HeaderFields;
    use types::tx::{SignedTx, Tx};
    use types::{
        Address, Amount, ChainId, Height, Nonce, Round, TestClock, ValidatorId, GAS_TRANSFER,
    };

    pub(crate) fn test_block(tx_count: usize) -> Block {
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
        let txs = (0..tx_count)
            .map(|i| {
                let tx = Tx::transfer(
                    ChainId::new(1),
                    Nonce(i as u64),
                    GAS_TRANSFER,
                    Amount::new(1),
                    Address::ZERO,
                    Amount::new(i as u128 + 1),
                );
                SignedTx {
                    tx,
                    signature: [1u8; 64],
                    public_key: [2u8; 32],
                }
            })
            .collect();
        Block {
            header_fields: fields,
            txs,
        }
    }

    fn slots_from(shards: &[DaShard], drop: &[usize]) -> Vec<Option<Vec<u8>>> {
        shards
            .iter()
            .enumerate()
            .map(|(i, s)| {
                if drop.contains(&i) {
                    None
                } else {
                    Some(s.payload.clone())
                }
            })
            .collect()
    }

    #[test]
    fn split_happy_path_width_and_round_trip() {
        let block = test_block(3);
        let shards = split(&block).unwrap();
        assert_eq!(shards.len(), DATA_SHARDS + PARITY_SHARDS);
        for (i, s) in shards.iter().enumerate() {
            assert_eq!(s.index as usize, i);
        }
        let all: Vec<_> = shards.into_iter().map(|s| Some(s.payload)).collect();
        assert_eq!(reconstruct(&all).unwrap(), block);
    }

    #[test]
    fn reconstruct_parity_only_loss() {
        let block = test_block(2);
        let shards = split(&block).unwrap();
        // Drop both parity shards (indices 4, 5); all data remain.
        let slots = slots_from(&shards, &[4, 5]);
        assert_eq!(reconstruct(&slots).unwrap(), block);
    }

    #[test]
    fn reconstruct_mixed_data_and_parity_loss() {
        let block = test_block(4);
        let shards = split(&block).unwrap();
        // Drop data shard 0 and parity shard 5 — mixed loss, still k present.
        let slots = slots_from(&shards, &[0, 5]);
        assert_eq!(reconstruct(&slots).unwrap(), block);
    }

    #[test]
    fn reconstruct_too_few_shards_fails_not_silent() {
        let block = test_block(1);
        let shards = split(&block).unwrap();
        let slots = slots_from(&shards, &[0, 1, 2]);
        assert!(reconstruct(&slots).is_err());
    }

    #[test]
    fn shard_len_scales_with_block_size() {
        let small = split(&test_block(1)).unwrap();
        let large = split(&test_block(40)).unwrap();
        assert_eq!(small.len(), large.len());
        let sl = small[0].payload.len();
        let ll = large[0].payload.len();
        assert!(ll > sl, "large shard {ll} should exceed small {sl}");
        assert!(
            ll < sl.saturating_mul(80),
            "shard length should stay O(body/k), got small={sl} large={ll}"
        );
        for s in &large {
            assert_eq!(s.payload.len(), ll);
        }
    }

    #[test]
    fn randomized_loss_patterns() {
        let block = test_block(5);
        let shards = split(&block).unwrap();
        let n = shards.len();
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..48 {
            seed = seed.wrapping_mul(0x5851_f42d_4c95_7f2d).wrapping_add(1);
            let mut drop_mask = seed;
            let mut drop = Vec::new();
            for i in 0..n {
                if drop_mask & 1 == 1 {
                    drop.push(i);
                }
                drop_mask >>= 1;
            }
            let present = n - drop.len();
            let slots = slots_from(&shards, &drop);
            if present >= DATA_SHARDS {
                assert_eq!(reconstruct(&slots).expect("k shards should decode"), block);
            } else {
                assert!(
                    reconstruct(&slots).is_err(),
                    "fewer than k shards must fail (dropped {drop:?})"
                );
            }
        }
    }
}
