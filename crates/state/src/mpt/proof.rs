//! Inclusion and exclusion proofs for the hexary MPT (architecture.md §4.1).
//!
//! A verifier needs only the proof object and the claimed root — never the full trie.
//! Node hashes are checked with `hash.blake3` after `domain.tag.apply(MptNode, …)`.
//! The node chain is additionally bound with [`crate::merkle::verify`] so the
//! generic Merkle contract is used inside MPT proof machinery (not MPT-specific).

use super::node::{deserialize_node, hash_encoded, Node};
use super::path::bytes_to_nibbles;
use super::Trie;
use crate::merkle::{self, MerkleProof};

/// Serialized nodes from root toward the terminus, plus the claimed value if present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MptProof {
    /// Serialized node payloads (not domain-wrapped), root first.
    pub nodes: Vec<Vec<u8>>,
    /// `Some` for inclusion; `None` for exclusion.
    pub value: Option<Vec<u8>>,
    /// Generic Merkle proof of `nodes[0]` in the node-payload list.
    pub chain_merkle: MerkleProof,
    /// `merkle.compute_root` of `nodes`.
    pub chain_merkle_root: [u8; 32],
}

fn bind_chain(nodes: &[Vec<u8>]) -> Option<(MerkleProof, [u8; 32])> {
    let chain_merkle_root = merkle::compute_root(nodes);
    let chain_merkle = merkle::prove(nodes, 0)?;
    Some((chain_merkle, chain_merkle_root))
}

/// Collect the node chain for `key`. Contract: `mpt.prove`.
pub fn prove(trie: &Trie, key: &[u8]) -> Option<MptProof> {
    let nibbles = bytes_to_nibbles(key);
    let mut nodes = Vec::new();
    let mut h = trie.root();
    let mut remaining = nibbles.as_slice();
    loop {
        let bytes = trie.node_bytes(&h)?.to_vec();
        nodes.push(bytes.clone());
        match deserialize_node(&bytes)? {
            Node::Empty => break,
            Node::Leaf { path, value } => {
                let value = if path.as_slice() == remaining {
                    Some(value)
                } else {
                    None
                };
                let (chain_merkle, chain_merkle_root) = bind_chain(&nodes)?;
                return Some(MptProof {
                    nodes,
                    value,
                    chain_merkle,
                    chain_merkle_root,
                });
            }
            Node::Extension { path, next } => {
                if !remaining.starts_with(&path) {
                    break;
                }
                remaining = &remaining[path.len()..];
                h = next;
            }
            Node::Branch { children, value } => {
                if remaining.is_empty() {
                    let (chain_merkle, chain_merkle_root) = bind_chain(&nodes)?;
                    return Some(MptProof {
                        nodes,
                        value,
                        chain_merkle,
                        chain_merkle_root,
                    });
                }
                let n = remaining[0] as usize;
                remaining = &remaining[1..];
                match children[n] {
                    Some(next) => h = next,
                    None => break,
                }
            }
        }
    }
    let (chain_merkle, chain_merkle_root) = bind_chain(&nodes)?;
    Some(MptProof {
        nodes,
        value: None,
        chain_merkle,
        chain_merkle_root,
    })
}

/// Exclusion proof (same walk; absent when the key is present).
/// Contract: `mpt.prove_exclusion`.
pub fn prove_exclusion(trie: &Trie, key: &[u8]) -> Option<MptProof> {
    let p = prove(trie, key)?;
    if p.value.is_some() {
        None
    } else {
        Some(p)
    }
}

fn walk_verify(nodes: &[Vec<u8>], key: &[u8], expected_value: Option<&[u8]>) -> bool {
    if nodes.is_empty() {
        return false;
    }
    let key_n = bytes_to_nibbles(key);
    let mut pos = 0usize;
    for (i, raw) in nodes.iter().enumerate() {
        let node = match deserialize_node(raw) {
            Some(n) => n,
            None => return false,
        };
        let is_last = i + 1 == nodes.len();
        match node {
            Node::Empty => return is_last && expected_value.is_none(),
            Node::Leaf { path, value } => {
                if !is_last {
                    return false;
                }
                let rest = &key_n[pos..];
                if path.as_slice() == rest {
                    return expected_value == Some(value.as_slice());
                }
                return expected_value.is_none();
            }
            Node::Extension { path, next } => {
                if pos > key_n.len() || !key_n[pos..].starts_with(&path) {
                    return is_last && expected_value.is_none();
                }
                if is_last {
                    return false;
                }
                pos += path.len();
                if next != hash_encoded(&nodes[i + 1]) {
                    return false;
                }
            }
            Node::Branch { children, value } => {
                if pos == key_n.len() {
                    return is_last && expected_value == value.as_deref();
                }
                let n = key_n[pos] as usize;
                if children[n].is_none() {
                    return is_last && expected_value.is_none();
                }
                if is_last {
                    return false;
                }
                pos += 1;
                if children[n] != Some(hash_encoded(&nodes[i + 1])) {
                    return false;
                }
            }
        }
    }
    false
}

/// Verify an inclusion or exclusion proof against `root` only.
/// Contract: `mpt.verify`.
pub fn verify(key: &[u8], proof: &MptProof, root: &[u8; 32]) -> bool {
    if proof.nodes.is_empty() {
        return false;
    }
    if hash_encoded(&proof.nodes[0]) != *root {
        return false;
    }
    if merkle::compute_root(&proof.nodes) != proof.chain_merkle_root {
        return false;
    }
    if !merkle::verify(
        &proof.nodes[0],
        &proof.chain_merkle,
        &proof.chain_merkle_root,
    ) {
        return false;
    }
    walk_verify(&proof.nodes, key, proof.value.as_deref())
}

#[cfg(test)]
mod tests {
    use super::super::Trie;
    use super::*;

    #[test]
    fn inclusion_verifier_has_no_trie() {
        let mut t = Trie::new();
        t.put(b"hello", b"world".to_vec());
        t.put(b"help", b"desk".to_vec());
        let root = t.root();
        let proof = prove(&t, b"hello").unwrap();
        assert_eq!(proof.value.as_deref(), Some(&b"world"[..]));
        drop(t);
        assert!(verify(b"hello", &proof, &root));
        let mut bad = proof.clone();
        bad.nodes[0][0] ^= 0xff;
        assert!(!verify(b"hello", &bad, &root));
        assert!(!verify(b"hello", &proof, &[0u8; 32]));
    }

    #[test]
    fn exclusion_and_wrong_value() {
        let mut t = Trie::new();
        t.put(b"a", b"1".to_vec());
        let root = t.root();
        let ex = prove_exclusion(&t, b"zzz").unwrap();
        assert!(ex.value.is_none());
        assert!(verify(b"zzz", &ex, &root));
        assert!(prove_exclusion(&t, b"a").is_none());
        let inc = prove(&t, b"a").unwrap();
        let mut wrong = inc.clone();
        wrong.value = Some(b"nope".to_vec());
        assert!(!verify(b"a", &wrong, &root));
        let mut bad_merkle = prove(&t, b"a").unwrap();
        if !bad_merkle.chain_merkle.siblings.is_empty() {
            bad_merkle.chain_merkle.siblings[0][0] ^= 1;
            assert!(!verify(b"a", &bad_merkle, &root));
        }
    }
}
