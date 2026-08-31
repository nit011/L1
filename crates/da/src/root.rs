//! DA commitment: Merkle root over chunks plus a KZG polynomial commitment
//! (architecture.md §6 pipeline and §7 KZG scalability path).
//!
//! # Which commitment is authoritative
//!
//! - **Merkle root (`DaRoot::merkle`)** — authoritative for `header.da_root`
//!   and for per-chunk inclusion proofs used by DAS. Light nodes check a
//!   sample with [`state::merkle::verify`].
//! - **KZG (`DaRoot::kzg`)** — additional polynomial commitment over the same
//!   ordered chunk hashes (Tier 1 [`crypto::kzg::commit`] on the toy SRS from
//!   [`crypto::kzg::setup`]). Intended for more compact sampling proofs later;
//!   this tier does not replace Merkle checks with KZG openings.

use crate::chunk::{leaf_bytes, split, ChunkError, DaShard, DATA_SHARDS, PARITY_SHARDS};
use ark_bls12_381::Fr;
use ark_ff::PrimeField;
use crypto::hash::blake3::hash_to_array;
use crypto::kzg::{commit as kzg_commit, setup as kzg_setup, KzgCommitment, KzgError};
use state::merkle::{self, MerkleProof};
use thiserror::Error;
use types::block::Block;
use types::Hash;

/// Toy SRS degree (≥ `k+m` coefficients). Consumes Tier 0 `kzg.setup`.
pub const KZG_SRS_DEGREE: usize = 16;
/// Deterministic toy setup seed (not a production ceremony).
pub const KZG_SRS_SEED: &[u8] = b"l1-da-kzg-toy-srs";

/// Dual commitment to the erasure-coded chunk set. Contract: `da.root`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaRoot {
    /// Merkle root over ordered chunk leaves — goes in `header.da_root`.
    pub merkle: Hash,
    /// KZG commitment to chunk-hash coefficients.
    pub kzg: KzgCommitment,
    /// `k` used when splitting.
    pub data_shards: usize,
    /// `m` used when splitting.
    pub parity_shards: usize,
}

/// Chunk plus Merkle inclusion proof against [`DaRoot::merkle`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvenChunk {
    /// RS shard.
    pub shard: DaShard,
    /// Proof from [`merkle::prove`].
    pub proof: MerkleProof,
}

/// `da.root` errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RootError {
    /// Chunking failed.
    #[error("{0}")]
    Chunk(#[from] ChunkError),
    /// KZG commit/setup failed.
    #[error("da root kzg: {0}")]
    Kzg(#[from] KzgError),
    /// Merkle prove failed (index out of range).
    #[error("da root: merkle proof missing")]
    Proof,
}

fn leaves(shards: &[DaShard]) -> Vec<Vec<u8>> {
    shards.iter().map(leaf_bytes).collect()
}

fn chunk_hash_fr(leaf: &[u8]) -> Fr {
    Fr::from_le_bytes_mod_order(&hash_to_array(leaf))
}

/// Commit to `block.body` chunks. Calls [`split`], [`merkle::compute_root`],
/// and [`kzg_commit`]. Contract: `da.root`.
pub fn commit(block: &Block) -> Result<(DaRoot, Vec<ProvenChunk>), RootError> {
    let shards = split(block)?;
    let leaf_set = leaves(&shards);
    let merkle = Hash::from_bytes(merkle::compute_root(&leaf_set));
    let coeffs: Vec<Fr> = leaf_set.iter().map(|l| chunk_hash_fr(l)).collect();
    let srs = kzg_setup(KZG_SRS_DEGREE, KZG_SRS_SEED)?;
    let kzg = kzg_commit(&srs, &coeffs)?;
    let mut proven = Vec::with_capacity(shards.len());
    for (i, shard) in shards.into_iter().enumerate() {
        let proof = merkle::prove(&leaf_set, i).ok_or(RootError::Proof)?;
        proven.push(ProvenChunk { shard, proof });
    }
    Ok((
        DaRoot {
            merkle,
            kzg,
            data_shards: DATA_SHARDS,
            parity_shards: PARITY_SHARDS,
        },
        proven,
    ))
}

/// Verify one gossiped chunk against [`DaRoot::merkle`] via `merkle.verify`.
pub fn verify_chunk(root: &DaRoot, proven: &ProvenChunk) -> bool {
    let leaf = leaf_bytes(&proven.shard);
    merkle::verify(&leaf, &proven.proof, root.merkle.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::tests::test_block;

    #[test]
    fn root_happy_path_and_proofs() {
        let block = test_block(2);
        let (root, proven) = commit(&block).unwrap();
        assert_eq!(proven.len(), DATA_SHARDS + PARITY_SHARDS);
        assert_ne!(root.merkle, Hash::ZERO);
        assert!(!root.kzg.bytes.is_empty());
        for p in &proven {
            assert!(verify_chunk(&root, p));
        }
    }

    #[test]
    fn tampered_chunk_fails_merkle() {
        let block = test_block(1);
        let (root, mut proven) = commit(&block).unwrap();
        proven[0].shard.payload[0] ^= 0xff;
        assert!(!verify_chunk(&root, &proven[0]));
    }

    #[test]
    fn independent_nodes_agree_on_root() {
        let block = test_block(6);
        let (a, pa) = commit(&block).unwrap();
        let (b, pb) = commit(&block).unwrap();
        assert_eq!(a.merkle, b.merkle);
        assert_eq!(a.kzg.bytes, b.kzg.bytes);
        assert_eq!(
            pa.iter().map(|p| p.shard.index).collect::<Vec<_>>(),
            pb.iter().map(|p| p.shard.index).collect::<Vec<_>>()
        );
        let leaves_a: Vec<_> = pa.iter().map(|p| leaf_bytes(&p.shard)).collect();
        let leaves_b: Vec<_> = pb.iter().map(|p| leaf_bytes(&p.shard)).collect();
        assert_eq!(leaves_a, leaves_b);
        assert_eq!(merkle::compute_root(&leaves_a), *a.merkle.as_bytes());
    }

    #[test]
    fn header_da_root_consumes_merkle_commitment() {
        use types::header::{apply_da_root, Header, HeaderFields, DA_ROOT_PLACEHOLDER};
        use types::{Height, Round, TestClock, ValidatorId};
        let block = test_block(1);
        let (root, _) = commit(&block).unwrap();
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
        let mut h = Header {
            fields,
            tx_root: Hash::ZERO,
            state_root: Hash::ZERO,
            receipts_root: Hash::ZERO,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        };
        let placeholder_hash = h.hash();
        apply_da_root(&mut h, root.merkle);
        assert_eq!(h.da_root, root.merkle);
        assert_ne!(h.hash(), placeholder_hash);
    }
}
