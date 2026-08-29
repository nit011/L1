//! Domain-separation tags. Callers cannot pass an arbitrary string.
//!
//! Tags named in development-plan.md Tier 0: tx, header, vote, vrf, mpt-node.
//! Prepending a tag before hashing prevents cross-protocol collisions
//! (architecture.md §7).

/// Fixed domains. A typo cannot silently invent a new domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomainTag {
    /// Transaction bytes.
    Tx,
    /// Block header bytes.
    Header,
    /// Consensus vote bytes.
    Vote,
    /// VRF message / seed material.
    Vrf,
    /// Merkle Patricia trie node.
    MptNode,
    /// Generic Merkle tree (tx/receipt roots, two-leaf state-root combine).
    /// Additive in Tier 1 so generic `merkle.compute_root` is domain-separated
    /// without overloading `MptNode`.
    Merkle,
}

impl DomainTag {
    /// Canonical ASCII label.
    pub fn label(self) -> &'static [u8] {
        match self {
            Self::Tx => b"tx",
            Self::Header => b"header",
            Self::Vote => b"vote",
            Self::Vrf => b"vrf",
            Self::MptNode => b"mpt-node",
            Self::Merkle => b"merkle",
        }
    }
}

/// Prepend `tag` to `msg` as `L1/{label}\\0{msg}` so labels cannot prefix-collide.
///
/// Contract: `domain.tag.apply`.
pub fn apply(tag: DomainTag, msg: &[u8]) -> Vec<u8> {
    let label = tag.label();
    let mut out = Vec::with_capacity(3 + label.len() + 1 + msg.len());
    out.extend_from_slice(b"L1/");
    out.extend_from_slice(label);
    out.push(0);
    out.extend_from_slice(msg);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::blake3::hash_to_array;

    #[test]
    fn tags_are_distinct_after_hash() {
        let msg = b"same";
        let hashes: Vec<_> = [
            DomainTag::Tx,
            DomainTag::Header,
            DomainTag::Vote,
            DomainTag::Vrf,
            DomainTag::MptNode,
            DomainTag::Merkle,
        ]
        .into_iter()
        .map(|t| hash_to_array(&apply(t, msg)))
        .collect();
        for i in 0..hashes.len() {
            for j in 0..hashes.len() {
                if i != j {
                    assert_ne!(hashes[i], hashes[j]);
                }
            }
        }
    }

    #[test]
    fn prefix_does_not_collide_tx_vs_header() {
        // Without a delimiter, "tx" || "header..." could theoretically confuse labels.
        let a = apply(DomainTag::Tx, b"header-payload");
        let b = apply(DomainTag::Header, b"payload");
        assert_ne!(a, b);
        assert!(apply(DomainTag::Tx, b"").starts_with(b"L1/tx\0"));
    }
}
