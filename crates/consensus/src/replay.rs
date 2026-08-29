//! Vote replay keys (architecture.md §2.4 double-signing).
//!
//! This module only builds the key and digest. It does not slash or store
//! evidence (Tier 5 / 9). Uses `domain.tag.apply` with [`DomainTag::Vote`].

use crypto::hash::blake3::hash_to_array;
use crypto::{apply_domain, DomainTag};
use types::{Height, Round, ValidatorId};

/// Vote kind tag for replay (not a BFT state-machine step).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum VoteKind {
    /// Prevote.
    Prevote = 0,
    /// Precommit.
    Precommit = 1,
}

/// Replay-detection key: same signer + height + round + kind → same key.
/// Contract: `cons.replay.vote`.
pub fn replay_key(signer: &ValidatorId, height: Height, round: Round, kind: VoteKind) -> [u8; 32] {
    let mut msg = Vec::with_capacity(48 + 8 + 4 + 1);
    msg.extend_from_slice(signer.as_bytes());
    msg.extend_from_slice(&height.0.to_be_bytes());
    msg.extend_from_slice(&round.0.to_be_bytes());
    msg.push(kind as u8);
    hash_to_array(&apply_domain(DomainTag::Vote, &msg))
}

/// Hash of a specific vote body (block id / payload) under the vote domain.
pub fn vote_hash(
    signer: &ValidatorId,
    height: Height,
    round: Round,
    kind: VoteKind,
    block_id: &[u8],
) -> [u8; 32] {
    let mut msg = Vec::with_capacity(48 + 8 + 4 + 1 + block_id.len());
    msg.extend_from_slice(signer.as_bytes());
    msg.extend_from_slice(&height.0.to_be_bytes());
    msg.extend_from_slice(&round.0.to_be_bytes());
    msg.push(kind as u8);
    msg.extend_from_slice(block_id);
    hash_to_array(&apply_domain(DomainTag::Vote, &msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(x: u8) -> ValidatorId {
        ValidatorId::from_bytes([x; 48])
    }

    #[test]
    fn same_slot_same_key_different_hashes() {
        let s = id(1);
        let h = Height(4);
        let r = Round(2);
        let k1 = replay_key(&s, h, r, VoteKind::Prevote);
        let k2 = replay_key(&s, h, r, VoteKind::Prevote);
        assert_eq!(k1, k2);
        let ha = vote_hash(&s, h, r, VoteKind::Prevote, b"block-a");
        let hb = vote_hash(&s, h, r, VoteKind::Prevote, b"block-b");
        assert_ne!(ha, hb);
    }

    #[test]
    fn different_signer_height_round_kind() {
        let s1 = id(1);
        let s2 = id(2);
        let h = Height(1);
        let r = Round(0);
        assert_ne!(
            replay_key(&s1, h, r, VoteKind::Prevote),
            replay_key(&s2, h, r, VoteKind::Prevote)
        );
        assert_ne!(
            replay_key(&s1, Height(2), r, VoteKind::Prevote),
            replay_key(&s1, h, r, VoteKind::Prevote)
        );
        assert_ne!(
            replay_key(&s1, h, Round(1), VoteKind::Prevote),
            replay_key(&s1, h, r, VoteKind::Prevote)
        );
        assert_ne!(
            replay_key(&s1, h, r, VoteKind::Precommit),
            replay_key(&s1, h, r, VoteKind::Prevote)
        );
    }
}
