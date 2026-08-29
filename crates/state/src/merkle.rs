//! Generic Merkle tree over arbitrary leaves (architecture.md §4).
//!
//! Used for tx/receipt roots later and as a helper for MPT proof hashing.
//! Internal nodes are `blake3(domain.tag.apply(Merkle, left || right))`.

use crypto::hash::blake3::hash_to_array;
use crypto::{apply_domain, DomainTag};

/// Hash a payload under the generic Merkle domain (not `MptNode`).
fn dhash(payload: &[u8]) -> [u8; 32] {
    hash_to_array(&apply_domain(DomainTag::Merkle, payload))
}

fn hash_leaf(leaf: &[u8]) -> [u8; 32] {
    let mut p = Vec::with_capacity(1 + leaf.len());
    p.push(0x00);
    p.extend_from_slice(leaf);
    dhash(&p)
}

fn hash_branch(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut p = Vec::with_capacity(65);
    p.push(0x01);
    p.extend_from_slice(left);
    p.extend_from_slice(right);
    dhash(&p)
}

/// Compute the Merkle root of `leaves`. Empty tree is the domain-hash of `[]`.
///
/// Contract: `merkle.compute_root`.
pub fn compute_root(leaves: &[Vec<u8>]) -> [u8; 32] {
    if leaves.is_empty() {
        return dhash(b"");
    }
    let mut layer: Vec<[u8; 32]> = leaves.iter().map(|l| hash_leaf(l)).collect();
    while layer.len() > 1 {
        if layer.len() % 2 == 1 {
            layer.push(*layer.last().expect("nonempty"));
        }
        let mut next = Vec::with_capacity(layer.len() / 2);
        for chunk in layer.chunks(2) {
            next.push(hash_branch(&chunk[0], &chunk[1]));
        }
        layer = next;
    }
    layer[0]
}

/// Inclusion proof: siblings from leaf toward root, plus the leaf index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MerkleProof {
    /// Index of the leaf in the original list.
    pub leaf_index: usize,
    /// Sibling hashes, leaf-level first.
    pub siblings: Vec<[u8; 32]>,
}

/// Prove inclusion of `leaves[index]`.
pub fn prove(leaves: &[Vec<u8>], index: usize) -> Option<MerkleProof> {
    if index >= leaves.len() {
        return None;
    }
    let mut layer: Vec<[u8; 32]> = leaves.iter().map(|l| hash_leaf(l)).collect();
    let mut idx = index;
    let mut siblings = Vec::new();
    while layer.len() > 1 {
        if layer.len() % 2 == 1 {
            layer.push(*layer.last().expect("nonempty"));
        }
        let sib = if idx.is_multiple_of(2) {
            idx + 1
        } else {
            idx - 1
        };
        siblings.push(layer[sib]);
        idx /= 2;
        let mut next = Vec::with_capacity(layer.len() / 2);
        for chunk in layer.chunks(2) {
            next.push(hash_branch(&chunk[0], &chunk[1]));
        }
        layer = next;
    }
    Some(MerkleProof {
        leaf_index: index,
        siblings,
    })
}

/// Verify `leaf` against `root` using only the proof (no full tree).
///
/// Contract: `merkle.verify`.
pub fn verify(leaf: &[u8], proof: &MerkleProof, root: &[u8; 32]) -> bool {
    let mut acc = hash_leaf(leaf);
    let mut idx = proof.leaf_index;
    for sib in &proof.siblings {
        acc = if idx.is_multiple_of(2) {
            hash_branch(&acc, sib)
        } else {
            hash_branch(sib, &acc)
        };
        idx /= 2;
    }
    acc == *root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_single() {
        let empty = compute_root(&[]);
        assert_eq!(empty, dhash(b""));
        let one = vec![b"only".to_vec()];
        let r = compute_root(&one);
        let p = prove(&one, 0).unwrap();
        assert!(verify(b"only", &p, &r));
        assert!(!verify(b"other", &p, &r));
    }

    #[test]
    fn multi_leaf_round_trip_and_tamper() {
        let leaves: Vec<Vec<u8>> = (0..5).map(|i| vec![i]).collect();
        let root = compute_root(&leaves);
        let p = prove(&leaves, 3).unwrap();
        assert!(verify(&[3], &p, &root));
        let mut bad = p.clone();
        bad.siblings[0][0] ^= 1;
        assert!(!verify(&[3], &bad, &root));
        assert!(prove(&leaves, 99).is_none());
    }

    #[test]
    fn same_insert_order_same_root() {
        let a = vec![b"x".to_vec(), b"y".to_vec(), b"z".to_vec()];
        assert_eq!(compute_root(&a), compute_root(&a));
    }
}
