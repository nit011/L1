//! Hexary Merkle Patricia Trie (architecture.md §4.1).
//!
//! Keys are hashed only at the caller's discretion; this trie stores raw key
//! bytes as nibble paths. Node hashes use `hash.blake3` after `encoding.canonical.encode`
//! and `domain.tag.apply(MptNode, ...)`.

mod node;
mod path;
pub mod proof;

pub use node::{
    branch, deserialize_node, empty_root, extension, hash_encoded, hash_node, leaf, serialize_node,
    Node,
};
pub use path::{bytes_to_nibbles, decode_path, encode_path};

use node::serialize_node as ser;
use path::common_prefix_len;
use types::collections::Map;

/// In-memory MPT with hash-addressed nodes.
#[derive(Clone, Debug)]
pub struct Trie {
    nodes: Map<[u8; 32], Vec<u8>>,
    root: [u8; 32],
}

impl Default for Trie {
    fn default() -> Self {
        Self::new()
    }
}

impl Trie {
    /// Empty trie.
    pub fn new() -> Self {
        let h = hash_node(&Node::Empty);
        let mut nodes = Map::new();
        nodes.insert(h, ser(&Node::Empty));
        Self { nodes, root: h }
    }

    /// Current root hash.
    pub fn root(&self) -> [u8; 32] {
        self.root
    }

    fn load(&self, h: &[u8; 32]) -> Node {
        let bytes = self.nodes.get(h).expect("dangling node hash");
        debug_assert_eq!(hash_encoded(bytes), *h);
        deserialize_node(bytes).expect("corrupt node")
    }

    fn store(&mut self, node: Node) -> [u8; 32] {
        let bytes = ser(&node);
        let h = hash_node(&node);
        self.nodes.insert(h, bytes);
        h
    }

    /// Lookup. Contract: `mpt.get`.
    pub fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        let nibbles = bytes_to_nibbles(key);
        self.get_at(self.root, &nibbles)
    }

    fn get_at(&self, h: [u8; 32], key: &[u8]) -> Option<Vec<u8>> {
        match self.load(&h) {
            Node::Empty => None,
            Node::Leaf { path, value } => {
                if path == key {
                    Some(value)
                } else {
                    None
                }
            }
            Node::Extension { path, next } => {
                if key.starts_with(&path) {
                    self.get_at(next, &key[path.len()..])
                } else {
                    None
                }
            }
            Node::Branch { children, value } => {
                if key.is_empty() {
                    value
                } else {
                    let n = key[0] as usize;
                    children[n].and_then(|c| self.get_at(c, &key[1..]))
                }
            }
        }
    }

    /// Insert or replace. Contract: `mpt.put`.
    pub fn put(&mut self, key: &[u8], value: Vec<u8>) {
        let nibbles = bytes_to_nibbles(key);
        self.root = self.insert(self.root, &nibbles, value);
    }

    fn insert(&mut self, h: [u8; 32], key: &[u8], value: Vec<u8>) -> [u8; 32] {
        match self.load(&h) {
            Node::Empty => self.store(leaf(key.to_vec(), value)),
            Node::Leaf {
                path,
                value: old_val,
            } => {
                if path == key {
                    return self.store(leaf(path, value));
                }
                let cp = common_prefix_len(&path, key);
                let branch_h = self.split_to_branch(&path[cp..], old_val, &key[cp..], value);
                if cp == 0 {
                    branch_h
                } else {
                    self.store(extension(path[..cp].to_vec(), branch_h))
                }
            }
            Node::Extension { path, next } => {
                if key.starts_with(&path) {
                    let new_next = self.insert(next, &key[path.len()..], value);
                    return self.store(extension(path, new_next));
                }
                let cp = common_prefix_len(&path, key);
                let mut children = [None; 16];
                debug_assert!(path.len() != cp);
                let ext_nibble = path[cp] as usize;
                let rest_path = &path[cp + 1..];
                let ext_child = if rest_path.is_empty() {
                    next
                } else {
                    self.store(extension(rest_path.to_vec(), next))
                };
                children[ext_nibble] = Some(ext_child);

                if key.len() == cp {
                    let br = branch(children, Some(value));
                    let bh = self.store(br);
                    return if cp == 0 {
                        bh
                    } else {
                        self.store(extension(key.to_vec(), bh))
                    };
                }
                let key_nibble = key[cp] as usize;
                let leaf_h = self.store(leaf(key[cp + 1..].to_vec(), value));
                children[key_nibble] = Some(leaf_h);
                let bh = self.store(branch(children, None));
                if cp == 0 {
                    bh
                } else {
                    self.store(extension(path[..cp].to_vec(), bh))
                }
            }
            Node::Branch {
                mut children,
                value: br_val,
            } => {
                if key.is_empty() {
                    return self.store(branch(*children, Some(value)));
                }
                let n = key[0] as usize;
                let child = children[n].unwrap_or_else(|| hash_node(&Node::Empty));
                let new_c = self.insert(child, &key[1..], value);
                children[n] = Some(new_c);
                self.store(branch(*children, br_val))
            }
        }
    }

    fn split_to_branch(
        &mut self,
        a_rest: &[u8],
        a_val: Vec<u8>,
        b_rest: &[u8],
        b_val: Vec<u8>,
    ) -> [u8; 32] {
        let mut children = [None; 16];
        let mut branch_val = None;
        if a_rest.is_empty() {
            branch_val = Some(a_val);
        } else {
            let n = a_rest[0] as usize;
            children[n] = Some(self.store(leaf(a_rest[1..].to_vec(), a_val)));
        }
        if b_rest.is_empty() {
            debug_assert!(branch_val.is_none());
            branch_val = Some(b_val);
        } else {
            let n = b_rest[0] as usize;
            children[n] = Some(self.store(leaf(b_rest[1..].to_vec(), b_val)));
        }
        self.store(branch(children, branch_val))
    }

    /// Delete a key. Contract: `mpt.delete`.
    pub fn delete(&mut self, key: &[u8]) {
        if self.get(key).is_none() {
            return;
        }
        let nibbles = bytes_to_nibbles(key);
        if let Some(h) = self.remove(self.root, &nibbles) {
            self.root = h;
        } else {
            self.root = self.store(Node::Empty);
        }
    }

    /// Returns `None` if the subtree became empty.
    fn remove(&mut self, h: [u8; 32], key: &[u8]) -> Option<[u8; 32]> {
        match self.load(&h) {
            Node::Empty => None,
            Node::Leaf { path, value } => {
                if path == key {
                    None
                } else {
                    Some(self.store(leaf(path, value)))
                }
            }
            Node::Extension { path, next } => {
                if !key.starts_with(&path) {
                    return Some(self.store(extension(path, next)));
                }
                self.remove(next, &key[path.len()..])
                    .map(|new_next| self.normalize_extension(path, new_next))
            }
            Node::Branch {
                mut children,
                value,
            } => {
                if key.is_empty() {
                    if value.is_none() {
                        return Some(self.store(branch(*children, None)));
                    }
                    return self.collapse_branch(*children, None);
                }
                let n = key[0] as usize;
                let Some(ch) = children[n] else {
                    return Some(self.store(branch(*children, value)));
                };
                match self.remove(ch, &key[1..]) {
                    None => {
                        children[n] = None;
                        self.collapse_branch(*children, value)
                    }
                    Some(new_c) => {
                        children[n] = Some(new_c);
                        Some(self.store(branch(*children, value)))
                    }
                }
            }
        }
    }

    fn collapse_branch(
        &mut self,
        children: [Option<[u8; 32]>; 16],
        value: Option<Vec<u8>>,
    ) -> Option<[u8; 32]> {
        let present: Vec<(usize, [u8; 32])> = children
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.map(|h| (i, h)))
            .collect();
        match (present.len(), value.as_ref()) {
            (0, None) => None,
            (0, Some(v)) => Some(self.store(leaf(vec![], v.clone()))),
            (1, None) => {
                let (nibble, child_h) = present[0];
                Some(self.prefix_child(nibble as u8, child_h))
            }
            _ => {
                let mut ch = [None; 16];
                for (i, h) in present {
                    ch[i] = Some(h);
                }
                Some(self.store(branch(ch, value)))
            }
        }
    }

    fn prefix_child(&mut self, nibble: u8, child_h: [u8; 32]) -> [u8; 32] {
        match self.load(&child_h) {
            Node::Leaf { mut path, value } => {
                let mut np = vec![nibble];
                np.append(&mut path);
                self.store(leaf(np, value))
            }
            Node::Extension { mut path, next } => {
                let mut np = vec![nibble];
                np.append(&mut path);
                self.store(extension(np, next))
            }
            Node::Branch { .. } => self.store(extension(vec![nibble], child_h)),
            Node::Empty => self.store(Node::Empty),
        }
    }

    fn normalize_extension(&mut self, path: Vec<u8>, next: [u8; 32]) -> [u8; 32] {
        match self.load(&next) {
            Node::Empty => self.store(Node::Empty),
            Node::Leaf {
                path: mut lp,
                value,
            } => {
                let mut p = path;
                p.append(&mut lp);
                self.store(leaf(p, value))
            }
            Node::Extension {
                path: mut ep,
                next: nn,
            } => {
                let mut p = path;
                p.append(&mut ep);
                self.store(extension(p, nn))
            }
            Node::Branch { .. } => self.store(extension(path, next)),
        }
    }

    /// Node payload by hash (for proofs).
    pub(crate) fn node_bytes(&self, h: &[u8; 32]) -> Option<&[u8]> {
        self.nodes.get(h).map(|v| v.as_slice())
    }
}

/// Lookup helper used by proofs. Contract re-export of get.
pub fn get(trie: &Trie, key: &[u8]) -> Option<Vec<u8>> {
    trie.get(key)
}

/// Insert helper. Contract: `mpt.put`.
pub fn put(trie: &mut Trie, key: &[u8], value: Vec<u8>) {
    trie.put(key, value);
}

/// Delete helper. Contract: `mpt.delete`.
pub fn delete(trie: &mut Trie, key: &[u8]) {
    trie.delete(key);
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::collections::Map as BTree;

    #[test]
    fn put_get_overwrite_missing() {
        let mut t = Trie::new();
        assert!(t.get(b"missing").is_none());
        t.put(b"alpha", b"1".to_vec());
        assert_eq!(t.get(b"alpha").as_deref(), Some(&b"1"[..]));
        t.put(b"alpha", b"2".to_vec());
        assert_eq!(t.get(b"alpha").as_deref(), Some(&b"2"[..]));
    }

    #[test]
    fn oracle_random_and_delete() {
        let mut oracle: BTree<Vec<u8>, Vec<u8>> = BTree::new();
        let mut t = Trie::new();
        for i in 0u32..40 {
            let k = i.to_be_bytes().to_vec();
            let v = (i.wrapping_mul(17)).to_be_bytes().to_vec();
            oracle.insert(k.clone(), v.clone());
            t.put(&k, v);
        }
        for (k, v) in &oracle {
            assert_eq!(t.get(k).as_ref(), Some(v));
        }
        for i in 0u32..15 {
            let k = i.to_be_bytes().to_vec();
            oracle.remove(&k);
            t.delete(&k);
        }
        for i in 0u32..40 {
            let k = i.to_be_bytes().to_vec();
            assert_eq!(t.get(&k), oracle.get(&k).cloned());
        }
    }

    #[test]
    fn root_deterministic_two_runs() {
        let keys = [b"k1".as_slice(), b"k2", b"abc", b"zzz"];
        let mut a = Trie::new();
        let mut b = Trie::new();
        for k in keys {
            a.put(k, k.to_vec());
            b.put(k, k.to_vec());
        }
        assert_eq!(a.root(), b.root());
        let before = a.root();
        a.put(b"k1", b"changed".to_vec());
        assert_ne!(a.root(), before);
    }

    #[test]
    fn delete_missing_is_noop_root() {
        let mut t = Trie::new();
        t.put(b"keep", b"v".to_vec());
        let r = t.root();
        t.delete(b"absent");
        assert_eq!(t.root(), r);
        t.delete(b"keep");
        assert!(t.get(b"keep").is_none());
        assert_eq!(t.root(), empty_root());
    }
}
