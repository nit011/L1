//! Generic Merkle root over leaves (byte-identical to `state::merkle::compute_root`).
//!
//! `types` cannot depend on `state` (cycle). Leaves and domain wrapping match
//! `crates/state/src/merkle.rs`: `blake3(L1/merkle\\0 || 0x00||leaf)` / branch `0x01`.

fn dhash(payload: &[u8]) -> [u8; 32] {
    let mut tagged = Vec::with_capacity(10 + payload.len());
    tagged.extend_from_slice(b"L1/merkle\0");
    tagged.extend_from_slice(payload);
    *blake3::hash(&tagged).as_bytes()
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

/// Same contract as `merkle.compute_root` (Tier 1).
pub fn merkle_root(leaves: &[Vec<u8>]) -> [u8; 32] {
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

/// `blake3` of `data` (same as `crypto::hash::blake3::hash_to_array`).
pub fn blake3_array(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

/// `L1/{label}\\0{msg}` — must match `crypto::apply_domain`.
pub fn domain_wrap(label: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(3 + label.len() + 1 + msg.len());
    out.extend_from_slice(b"L1/");
    out.extend_from_slice(label);
    out.push(0);
    out.extend_from_slice(msg);
    out
}
