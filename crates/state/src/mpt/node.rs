//! MPT node types: Leaf, Extension, Branch (architecture.md §4.1).
//!
//! Node hashes are `blake3(domain.tag.apply(MptNode, encoding.canonical.encode(payload)))`.

use super::path::pack_hex_prefix;
use crypto::hash::blake3::hash_to_array;
use crypto::{apply_domain, DomainTag};
use types::encoding;

/// One node in the hexary Patricia trie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// Empty (null) node.
    Empty,
    /// Terminal: remaining nibble path + value.
    Leaf { path: Vec<u8>, value: Vec<u8> },
    /// Shared nibble prefix + next node hash.
    Extension { path: Vec<u8>, next: [u8; 32] },
    /// 16 children + optional value at this key.
    Branch {
        children: Box<[Option<[u8; 32]>; 16]>,
        value: Option<Vec<u8>>,
    },
}

fn tag_payload(kind: u8, rest: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(1 + rest.len());
    p.push(kind);
    p.extend_from_slice(rest);
    p
}

/// Serialize a node to a codec payload (before domain hash).
pub fn serialize_node(node: &Node) -> Vec<u8> {
    match node {
        Node::Empty => tag_payload(0, &[]),
        Node::Leaf { path, value } => {
            let mut rest = Vec::new();
            let hp = pack_hex_prefix(path, true);
            rest.extend_from_slice(&(hp.len() as u32).to_be_bytes());
            rest.extend_from_slice(&hp);
            rest.extend_from_slice(&(value.len() as u32).to_be_bytes());
            rest.extend_from_slice(value);
            tag_payload(1, &rest)
        }
        Node::Extension { path, next } => {
            let mut rest = Vec::new();
            let hp = pack_hex_prefix(path, false);
            rest.extend_from_slice(&(hp.len() as u32).to_be_bytes());
            rest.extend_from_slice(&hp);
            rest.extend_from_slice(next);
            tag_payload(2, &rest)
        }
        Node::Branch { children, value } => {
            let mut rest = Vec::new();
            for c in children.iter() {
                match c {
                    None => rest.push(0),
                    Some(h) => {
                        rest.push(1);
                        rest.extend_from_slice(h);
                    }
                }
            }
            match value {
                None => rest.push(0),
                Some(v) => {
                    rest.push(1);
                    rest.extend_from_slice(&(v.len() as u32).to_be_bytes());
                    rest.extend_from_slice(v);
                }
            }
            tag_payload(3, &rest)
        }
    }
}

/// Deserialize a node payload.
pub fn deserialize_node(payload: &[u8]) -> Option<Node> {
    if payload.is_empty() {
        return None;
    }
    match payload[0] {
        0 => Some(Node::Empty),
        1 => {
            let rest = &payload[1..];
            if rest.len() < 4 {
                return None;
            }
            let hlen = u32::from_be_bytes(rest[0..4].try_into().ok()?) as usize;
            if rest.len() < 4 + hlen + 4 {
                return None;
            }
            let hp = &rest[4..4 + hlen];
            let (path, is_leaf) = super::path::unpack_hex_prefix(hp).ok()?;
            if !is_leaf {
                return None;
            }
            let voff = 4 + hlen;
            let vlen = u32::from_be_bytes(rest[voff..voff + 4].try_into().ok()?) as usize;
            let value = rest.get(voff + 4..voff + 4 + vlen)?.to_vec();
            Some(Node::Leaf { path, value })
        }
        2 => {
            let rest = &payload[1..];
            if rest.len() < 4 {
                return None;
            }
            let hlen = u32::from_be_bytes(rest[0..4].try_into().ok()?) as usize;
            if rest.len() < 4 + hlen + 32 {
                return None;
            }
            let hp = &rest[4..4 + hlen];
            let (path, is_leaf) = super::path::unpack_hex_prefix(hp).ok()?;
            if is_leaf {
                return None;
            }
            let mut next = [0u8; 32];
            next.copy_from_slice(&rest[4 + hlen..4 + hlen + 32]);
            Some(Node::Extension { path, next })
        }
        3 => {
            let mut rest = &payload[1..];
            let mut children = [None; 16];
            for child in &mut children {
                if rest.is_empty() {
                    return None;
                }
                if rest[0] == 0 {
                    rest = &rest[1..];
                    *child = None;
                } else if rest[0] == 1 && rest.len() >= 33 {
                    let mut h = [0u8; 32];
                    h.copy_from_slice(&rest[1..33]);
                    *child = Some(h);
                    rest = &rest[33..];
                } else {
                    return None;
                }
            }
            if rest.is_empty() {
                return None;
            }
            let value = if rest[0] == 0 {
                None
            } else if rest[0] == 1 && rest.len() >= 5 {
                let vlen = u32::from_be_bytes(rest[1..5].try_into().ok()?) as usize;
                Some(rest.get(5..5 + vlen)?.to_vec())
            } else {
                return None;
            };
            Some(Node::Branch {
                children: Box::new(children),
                value,
            })
        }
        _ => None,
    }
}

/// Hash a node: encode payload then domain-separate as `mpt-node`.
pub fn hash_node(node: &Node) -> [u8; 32] {
    hash_encoded(&serialize_node(node))
}

/// Hash already-serialized node payload (for proofs that only carry bytes).
pub fn hash_encoded(payload: &[u8]) -> [u8; 32] {
    let wrapped = encoding::encode(payload);
    hash_to_array(&apply_domain(DomainTag::MptNode, &wrapped))
}

/// Empty-tree root.
pub fn empty_root() -> [u8; 32] {
    hash_node(&Node::Empty)
}

/// Leaf constructor (contract `mpt.node.leaf`).
pub fn leaf(path: Vec<u8>, value: Vec<u8>) -> Node {
    Node::Leaf { path, value }
}

/// Extension constructor (contract `mpt.node.extension`).
pub fn extension(path: Vec<u8>, next: [u8; 32]) -> Node {
    Node::Extension { path, next }
}

/// Branch constructor (contract `mpt.node.branch`).
pub fn branch(children: [Option<[u8; 32]>; 16], value: Option<Vec<u8>>) -> Node {
    Node::Branch {
        children: Box::new(children),
        value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_hash_uses_domain_and_encode() {
        let n = leaf(vec![1, 2], b"v".to_vec());
        let h1 = hash_node(&n);
        let h2 = hash_node(&n);
        assert_eq!(h1, h2);
        let untagged = hash_to_array(&serialize_node(&n));
        assert_ne!(h1, untagged);
        let _ = crate::mpt::encode_path(&[1, 2], true);
    }

    #[test]
    fn serialize_round_trip_all_kinds() {
        let leaf_n = leaf(vec![0xa], b"x".to_vec());
        assert_eq!(deserialize_node(&serialize_node(&leaf_n)).unwrap(), leaf_n);
        let ext = extension(vec![1, 2], [3u8; 32]);
        assert_eq!(deserialize_node(&serialize_node(&ext)).unwrap(), ext);
        let mut ch = [None; 16];
        ch[4] = Some([9u8; 32]);
        let br = branch(ch, Some(b"at-branch".to_vec()));
        assert_eq!(deserialize_node(&serialize_node(&br)).unwrap(), br);
        assert!(deserialize_node(&[99]).is_none());
    }
}
