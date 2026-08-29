//! BLS votes (architecture.md §2.1–§2.2, §2.4).
//!
//! A vote signs `(height, round, block-id)` under `bls.domain` (`DST`) after
//! `domain.tag.apply(Vote, …)`. [`VoteBlock::Nil`] is an explicit variant, not
//! an omitted hash — nil must never collide with a real [`Header::hash`].

use crate::replay::{replay_key, vote_hash, VoteKind};
use blst::min_pk::{PublicKey, SecretKey, Signature};
use crypto::sig::bls::{self, DST};
use crypto::{apply_domain, DomainTag};
use types::collections::Map;
use types::header::Header;
use types::{Hash, Height, Round, ValidatorId};

pub use blst::min_pk::SecretKey as BlsSecretKey;

/// What the vote is for. Contract: `vote.nil` is [`VoteBlock::Nil`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum VoteBlock {
    /// Explicit nil prevote/precommit (architecture.md §2.2).
    Nil,
    /// Vote for `header.hash`.
    Block(Hash),
}

impl VoteBlock {
    fn encode(self) -> Vec<u8> {
        match self {
            VoteBlock::Nil => vec![0],
            VoteBlock::Block(h) => {
                let mut v = vec![1];
                v.extend_from_slice(h.as_bytes());
                v
            }
        }
    }
}

/// Signed prevote or precommit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vote {
    /// `types.height`.
    pub height: Height,
    /// `types.round`.
    pub round: Round,
    /// Prevote vs precommit.
    pub kind: VoteKind,
    /// Block or explicit nil.
    pub block: VoteBlock,
    /// Signer (`validator.from_bls` id).
    pub signer: ValidatorId,
    /// Compressed BLS G2 signature (96 bytes).
    pub signature: [u8; 96],
}

/// Bytes signed by `bls.sign` (includes `bls.domain` via [`bls::sign`]).
pub fn signed_message(height: Height, round: Round, kind: VoteKind, block: VoteBlock) -> Vec<u8> {
    let _dst = DST;
    let mut p = Vec::new();
    p.extend_from_slice(&height.0.to_be_bytes());
    p.extend_from_slice(&round.0.to_be_bytes());
    p.push(kind as u8);
    p.extend_from_slice(&block.encode());
    apply_domain(DomainTag::Vote, &p)
}

fn sign_vote(
    sk: &SecretKey,
    signer: ValidatorId,
    height: Height,
    round: Round,
    kind: VoteKind,
    block: VoteBlock,
) -> Vote {
    let msg = signed_message(height, round, kind, block);
    let sig = bls::sign(sk, &msg);
    Vote {
        height,
        round,
        kind,
        block,
        signer,
        signature: sig.to_bytes(),
    }
}

/// Sign a prevote for `header.hash`. Contract: `vote.prevote`.
pub fn prevote(
    sk: &SecretKey,
    signer: ValidatorId,
    height: Height,
    round: Round,
    header: &Header,
) -> Vote {
    sign_vote(
        sk,
        signer,
        height,
        round,
        VoteKind::Prevote,
        VoteBlock::Block(header.hash()),
    )
}

/// Sign a precommit for `header.hash`. Contract: `vote.precommit`.
pub fn precommit(
    sk: &SecretKey,
    signer: ValidatorId,
    height: Height,
    round: Round,
    header: &Header,
) -> Vote {
    sign_vote(
        sk,
        signer,
        height,
        round,
        VoteKind::Precommit,
        VoteBlock::Block(header.hash()),
    )
}

/// Sign a nil vote (prevote or precommit). Contract: `vote.nil`.
///
/// Implemented via the same path as [`prevote`] / [`precommit`] with
/// [`VoteBlock::Nil`].
pub fn nil(
    sk: &SecretKey,
    signer: ValidatorId,
    height: Height,
    round: Round,
    kind: VoteKind,
) -> Vote {
    sign_vote(sk, signer, height, round, kind, VoteBlock::Nil)
}

/// Seen replay keys → vote body hash. Used by [`verify`].
#[derive(Clone, Debug, Default)]
pub struct VoteReplayLog {
    seen: Map<[u8; 32], [u8; 32]>,
}

impl VoteReplayLog {
    /// Empty log.
    pub fn new() -> Self {
        Self { seen: Map::new() }
    }

    /// Record or detect equivocation via `cons.replay.vote`.
    pub fn observe(&mut self, vote: &Vote) -> Result<(), VerifyError> {
        let key = replay_key(&vote.signer, vote.height, vote.round, vote.kind);
        let body = vote_hash(
            &vote.signer,
            vote.height,
            vote.round,
            vote.kind,
            &vote.block.encode(),
        );
        if let Some(prev) = self.seen.get(&key) {
            if *prev != body {
                return Err(VerifyError::Equivocation);
            }
            return Ok(());
        }
        self.seen.insert(key, body);
        Ok(())
    }
}

/// Vote verification failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifyError {
    /// `bls.verify` failed.
    Signature,
    /// Vote height/round does not match the round being processed
    /// (`cons.replay.vote` key would not match this slot).
    ReplaySlot,
    /// Same replay key, different body.
    Equivocation,
}

fn parse_pk(id: &ValidatorId) -> Result<PublicKey, VerifyError> {
    PublicKey::from_bytes(id.as_bytes()).map_err(|_| VerifyError::Signature)
}

fn parse_sig(bytes: &[u8; 96]) -> Result<Signature, VerifyError> {
    Signature::from_bytes(bytes).map_err(|_| VerifyError::Signature)
}

/// BLS-verify a vote without round-slot checks (for evidence).
pub fn verify_signature(vote: &Vote) -> Result<(), VerifyError> {
    let pk = parse_pk(&vote.signer)?;
    let sig = parse_sig(&vote.signature)?;
    let msg = signed_message(vote.height, vote.round, vote.kind, vote.block);
    bls::verify(&pk, &msg, &sig).map_err(|_| VerifyError::Signature)
}

/// Verify signature, slot, and replay log. Contract: `vote.verify`.
pub fn verify(
    vote: &Vote,
    expected_height: Height,
    expected_round: Round,
    log: &mut VoteReplayLog,
) -> Result<(), VerifyError> {
    verify_signature(vote)?;
    if vote.height != expected_height || vote.round != expected_round {
        return Err(VerifyError::ReplaySlot);
    }
    log.observe(vote)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::from_bls;
    use types::header::{HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{TestClock, VotingPower};

    fn header() -> Header {
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
            tx_root: Hash::ZERO,
            state_root: Hash::ZERO,
            receipts_root: Hash::ZERO,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        }
    }

    #[test]
    fn prevote_round_trip_and_nil_distinct() {
        let sk = bls::keygen().unwrap();
        let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(1));
        let h = header();
        let v = prevote(&sk, id, Height::GENESIS, Round::ZERO, &h);
        let mut log = VoteReplayLog::new();
        verify(&v, Height::GENESIS, Round::ZERO, &mut log).unwrap();
        let n = nil(&sk, id, Height::GENESIS, Round::ZERO, VoteKind::Prevote);
        assert_ne!(v.block, n.block);
        assert!(matches!(n.block, VoteBlock::Nil));
        assert_ne!(v.block, VoteBlock::Block(Hash::ZERO));
    }

    #[test]
    fn well_signed_wrong_height_fails_replay_slot() {
        let sk = bls::keygen().unwrap();
        let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(1));
        let h = header();
        let v = prevote(&sk, id, Height(2), Round::ZERO, &h);
        assert!(verify_signature(&v).is_ok());
        let mut log = VoteReplayLog::new();
        assert_eq!(
            verify(&v, Height(1), Round::ZERO, &mut log),
            Err(VerifyError::ReplaySlot)
        );
    }

    #[test]
    fn precommit_bad_signature() {
        let sk = bls::keygen().unwrap();
        let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(1));
        let h = header();
        let mut v = precommit(&sk, id, Height::GENESIS, Round::ZERO, &h);
        v.signature[0] ^= 1;
        let mut log = VoteReplayLog::new();
        assert_eq!(
            verify(&v, Height::GENESIS, Round::ZERO, &mut log),
            Err(VerifyError::Signature)
        );
    }
}
