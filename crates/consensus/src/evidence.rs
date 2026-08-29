//! Equivocation evidence (architecture.md §2.4).
//!
//! Two votes that both pass `vote.verify`'s signature check, share a
//! `cons.replay.vote` key, and differ in body. Encoded with
//! `encoding.canonical.encode` so a third party can verify without trust.

use crate::replay::{replay_key, vote_hash};
use crate::vote::{verify_signature, VerifyError, Vote};
use types::encoding::encode;

/// Portable equivocation proof. Contract: `evidence.equivocation`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evidence {
    /// First vote.
    pub a: Vote,
    /// Conflicting vote.
    pub b: Vote,
}

fn vote_payload(v: &Vote) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(v.signer.as_bytes());
    p.extend_from_slice(&v.height.0.to_be_bytes());
    p.extend_from_slice(&v.round.0.to_be_bytes());
    p.push(v.kind as u8);
    match v.block {
        crate::vote::VoteBlock::Nil => p.push(0),
        crate::vote::VoteBlock::Block(h) => {
            p.push(1);
            p.extend_from_slice(h.as_bytes());
        }
    }
    p.extend_from_slice(&v.signature);
    p
}

impl Evidence {
    /// Canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut p = Vec::new();
        let a = vote_payload(&self.a);
        let b = vote_payload(&self.b);
        p.extend_from_slice(&(a.len() as u32).to_be_bytes());
        p.extend_from_slice(&a);
        p.extend_from_slice(&(b.len() as u32).to_be_bytes());
        p.extend_from_slice(&b);
        encode(&p)
    }
}

/// Build evidence if `a` and `b` are a double-sign. Both signatures must verify.
pub fn equivocation(a: &Vote, b: &Vote) -> Result<Evidence, VerifyError> {
    verify_signature(a)?;
    verify_signature(b)?;
    if a.signer != b.signer || a.height != b.height || a.round != b.round || a.kind != b.kind {
        return Err(VerifyError::ReplaySlot);
    }
    let ka = replay_key(&a.signer, a.height, a.round, a.kind);
    let kb = replay_key(&b.signer, b.height, b.round, b.kind);
    if ka != kb {
        return Err(VerifyError::ReplaySlot);
    }
    let ba = vote_hash(&a.signer, a.height, a.round, a.kind, &block_bytes(a));
    let bb = vote_hash(&b.signer, b.height, b.round, b.kind, &block_bytes(b));
    if ba == bb {
        return Err(VerifyError::ReplaySlot);
    }
    Ok(Evidence {
        a: a.clone(),
        b: b.clone(),
    })
}

fn block_bytes(v: &Vote) -> Vec<u8> {
    match v.block {
        crate::vote::VoteBlock::Nil => vec![0],
        crate::vote::VoteBlock::Block(h) => {
            let mut x = vec![1];
            x.extend_from_slice(h.as_bytes());
            x
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::VoteKind;
    use crate::vote::{nil, prevote};
    use crypto::from_bls;
    use crypto::sig::bls;
    use types::header::{Header, HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{Hash, Height, Round, TestClock, ValidatorId, VotingPower};

    fn hdr(tag: u8) -> Header {
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
        Header {
            fields,
            tx_root: Hash::from_bytes([tag; 32]),
            state_root: Hash::ZERO,
            receipts_root: Hash::ZERO,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        }
    }

    #[test]
    fn conflicting_prevotes_encode() {
        let sk = bls::keygen().unwrap();
        let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(1));
        let a = prevote(&sk, id, Height::GENESIS, Round::ZERO, &hdr(1));
        let b = prevote(&sk, id, Height::GENESIS, Round::ZERO, &hdr(2));
        let e = equivocation(&a, &b).unwrap();
        assert!(!e.encode().is_empty());
        assert!(equivocation(&a, &a).is_err());
        let n = nil(&sk, id, Height::GENESIS, Round::ZERO, VoteKind::Prevote);
        let e2 = equivocation(&a, &n).unwrap();
        assert_ne!(e.encode(), e2.encode());
    }
}
