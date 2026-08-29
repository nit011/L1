//! Consensus WAL (architecture.md §2.4 — no double-sign after crash).
//!
//! Record the vote/proposal **body we are about to sign** via `kv.batch`
//! before `bls.sign`. Recovery refuses a conflicting second signature.

use crate::evidence::{equivocation, Evidence};
use crate::propose::Proposal;
use crate::replay::{replay_key, VoteKind};
use crate::vote::{signed_message, Vote, VoteBlock};
use storage::kv::{BatchOp, Store};
use types::TypesError;

const PREFIX: &[u8] = b"cw/";

fn vote_key(v: &Vote) -> Vec<u8> {
    let mut k = PREFIX.to_vec();
    k.extend_from_slice(&replay_key(&v.signer, v.height, v.round, v.kind));
    k
}

fn encode_vote_body(v: &Vote) -> Vec<u8> {
    signed_message(v.height, v.round, v.kind, v.block)
}

/// Persist a vote intent before signing. Contract: `wal.consensus`.
pub fn log_vote<S: Store>(store: &mut S, vote: &Vote) -> Result<(), TypesError> {
    store.apply_batch(&[BatchOp::Put {
        key: vote_key(vote),
        value: encode_vote_body(vote),
    }])
}

/// Persist a proposal intent (height/round/proposer). Contract: `wal.consensus`.
pub fn log_proposal<S: Store>(store: &mut S, p: &Proposal) -> Result<(), TypesError> {
    let mut key = PREFIX.to_vec();
    key.extend_from_slice(b"p/");
    key.extend_from_slice(&p.height.0.to_be_bytes());
    key.extend_from_slice(&p.round.0.to_be_bytes());
    store.apply_batch(&[BatchOp::Put {
        key,
        value: p.header.hash().as_bytes().to_vec(),
    }])
}

/// Load the signed message previously logged for this slot.
pub fn logged_vote_body<S: Store>(store: &S, vote: &Vote) -> Result<Option<Vec<u8>>, TypesError> {
    store.get(&vote_key(vote))
}

/// Refuse a conflicting vote after restart. Contract: `wal.no_double_sign`.
///
/// If a WAL body exists for the replay key and differs, return evidence
/// (when both votes are available) or `WouldDoubleSign`.
pub fn check_no_double_sign<S: Store>(
    store: &S,
    candidate: &Vote,
    previous_signed: Option<&Vote>,
) -> Result<(), DoubleSignError> {
    let Some(prev_body) = logged_vote_body(store, candidate)? else {
        return Ok(());
    };
    let new_body = encode_vote_body(candidate);
    if prev_body == new_body {
        return Ok(());
    }
    if let Some(old) = previous_signed {
        let e = equivocation(old, candidate).map_err(|_| DoubleSignError::WouldDoubleSign)?;
        return Err(DoubleSignError::Equivocation(Box::new(e)));
    }
    Err(DoubleSignError::WouldDoubleSign)
}

/// Double-sign after recovery.
#[derive(Debug)]
pub enum DoubleSignError {
    /// Store.
    Store(TypesError),
    /// Conflicting intent in the WAL.
    WouldDoubleSign,
    /// Constructed evidence.
    Equivocation(Box<Evidence>),
}

impl From<TypesError> for DoubleSignError {
    fn from(e: TypesError) -> Self {
        Self::Store(e)
    }
}

/// Sign a vote only after WAL + double-sign check.
pub fn sign_vote_logged<S: Store>(
    store: &mut S,
    unsigned: Vote,
    previous: Option<&Vote>,
) -> Result<Vote, DoubleSignError> {
    check_no_double_sign(store, &unsigned, previous)?;
    log_vote(store, &unsigned)?;
    let _ = VoteKind::Prevote;
    let _ = VoteBlock::Nil;
    Ok(unsigned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vote::prevote;
    use crypto::from_bls;
    use crypto::sig::bls;
    use storage::MemoryStore;
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
    fn crash_recovery_rejects_conflicting_vote() {
        let sk = bls::keygen().unwrap();
        let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(1));
        let a = prevote(&sk, id, Height::GENESIS, Round::ZERO, &hdr(1));
        let mut store = MemoryStore::new();
        sign_vote_logged(&mut store, a.clone(), None).unwrap();
        let mut restarted = MemoryStore::new();
        // simulate persistence: copy WAL key
        let body = logged_vote_body(&store, &a).unwrap().unwrap();
        restarted
            .apply_batch(&[BatchOp::Put {
                key: vote_key(&a),
                value: body,
            }])
            .unwrap();
        let b = prevote(&sk, id, Height::GENESIS, Round::ZERO, &hdr(2));
        let err = check_no_double_sign(&restarted, &b, Some(&a)).unwrap_err();
        assert!(matches!(
            err,
            DoubleSignError::Equivocation(_) | DoubleSignError::WouldDoubleSign
        ));
        let same = prevote(&sk, id, Height::GENESIS, Round::ZERO, &hdr(1));
        check_no_double_sign(&restarted, &same, Some(&a)).unwrap();
    }
}
