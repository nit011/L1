//! Tendermint steps (architecture.md §2.2).
//!
//! Prevote a valid proposal (respecting `cons.lock`); precommit on a prevote
//! polka; commit on a verified QC and surface `exec.app_hash` unchanged.

use crate::propose::{verify_leader, Proposal};
use crate::qc;
use crate::replay::VoteKind;
use crate::safety::{halt_no_quorum, CommitLog, SafetyError};
use crate::state::{prevote_allowed, Lock};
use crate::vote::{self, prevote, VerifyError, Vote, VoteBlock, VoteReplayLog};
use crate::vrf::VrfSeed;
use blst::min_pk::SecretKey;
use types::collections::Map;
use types::header::Header;
use types::{Hash, Height, Round, ValidatorId, VotingPower};

/// Finalized height. `app_hash` is the sequential `exec.app_hash`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finalized {
    /// Height.
    pub height: Height,
    /// Round of the QC.
    pub round: Round,
    /// `header.hash`.
    pub block_hash: Hash,
    /// `exec.app_hash` from the proposal/builder — not re-derived here.
    pub app_hash: Hash,
}

/// Tally voting power for a block id.
pub fn tally<'a>(
    votes: impl Iterator<Item = &'a Vote>,
    validators: &Map<ValidatorId, VotingPower>,
) -> Map<VoteBlock, VotingPower> {
    let mut t: Map<VoteBlock, VotingPower> = Map::new();
    for v in votes {
        let Some(p) = validators.get(&v.signer) else {
            continue;
        };
        let e = t.entry(v.block).or_insert(VotingPower::ZERO);
        *e = e.saturating_add(*p);
    }
    t
}

fn total_power(validators: &Map<ValidatorId, VotingPower>) -> VotingPower {
    validators
        .values()
        .fold(VotingPower::ZERO, |a, p| a.saturating_add(*p))
}

/// Highest-power value if it has quorum.
pub fn polka(votes: &[Vote], validators: &Map<ValidatorId, VotingPower>) -> Option<VoteBlock> {
    let total = total_power(validators);
    let t = tally(votes.iter(), validators);
    t.into_iter()
        .find(|(_, p)| qc::has_quorum(*p, total))
        .map(|(b, _)| b)
}

/// Accept a proposal and emit a prevote if the lock allows. Contract: `cons.prevote_step`.
#[allow(clippy::too_many_arguments)]
pub fn prevote_step(
    sk: &SecretKey,
    our_id: ValidatorId,
    proposal: &Proposal,
    vrf_pks: &Map<ValidatorId, [u8; 32]>,
    seed: &VrfSeed,
    validators: &Map<ValidatorId, VotingPower>,
    expected_height: Height,
    expected_round: Round,
    lock: Option<Lock>,
    unlock_polka: Option<(Round, Hash)>,
    log: &mut VoteReplayLog,
) -> Result<Vote, PrevoteError> {
    if proposal.height != expected_height || proposal.round != expected_round {
        return Err(PrevoteError::Slot);
    }
    let Some(src_pk) = vrf_pks.get(&proposal.vrf_source) else {
        return Err(PrevoteError::NotLeader);
    };
    if !verify_leader(proposal, src_pk, seed, validators) {
        return Err(PrevoteError::NotLeader);
    }
    let candidate = VoteBlock::Block(proposal.header.hash());
    if !prevote_allowed(lock, expected_height, candidate, unlock_polka) {
        let v = vote::nil(
            sk,
            our_id,
            expected_height,
            expected_round,
            VoteKind::Prevote,
        );
        vote::verify(&v, expected_height, expected_round, log).ok();
        return Ok(v);
    }
    let v = prevote(
        sk,
        our_id,
        expected_height,
        expected_round,
        &proposal.header,
    );
    vote::verify(&v, expected_height, expected_round, log).map_err(PrevoteError::Verify)?;
    Ok(v)
}

/// Prevote-step errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrevoteError {
    /// Height/round mismatch.
    Slot,
    /// VRF lottery does not select the proposer.
    NotLeader,
    /// `vote.verify` failed.
    Verify(VerifyError),
}

/// Precommit given a prevote set. Contract: `cons.precommit_step`.
#[allow(clippy::too_many_arguments)]
pub fn precommit_step(
    sk: &SecretKey,
    our_id: ValidatorId,
    height: Height,
    round: Round,
    prevotes: &[Vote],
    validators: &Map<ValidatorId, VotingPower>,
    header_for: &Header,
    log: &mut VoteReplayLog,
) -> (Vote, Option<Lock>) {
    match polka(prevotes, validators) {
        Some(VoteBlock::Block(h)) if h == header_for.hash() => {
            let v = vote::precommit(sk, our_id, height, round, header_for);
            let _ = vote::verify(&v, height, round, log);
            (
                v,
                Some(Lock {
                    height,
                    round,
                    block_hash: h,
                }),
            )
        }
        _ => {
            let v = vote::nil(sk, our_id, height, round, VoteKind::Precommit);
            let _ = vote::verify(&v, height, round, log);
            (v, None)
        }
    }
}

/// Commit on a verified QC. Contract: `cons.commit`.
///
/// `app_hash` must be the `exec.app_hash` from sequential execution / the
/// local builder — consensus does not re-hash state.
pub fn commit(
    precommits: &[Vote],
    validators: &Map<ValidatorId, VotingPower>,
    reachable: VotingPower,
    proposal: &Proposal,
    log: &mut CommitLog,
) -> Result<Option<Finalized>, CommitError> {
    let total = total_power(validators);
    if halt_no_quorum(reachable, total) {
        return Ok(None);
    }
    let Some(VoteBlock::Block(h)) = polka(precommits, validators) else {
        return Ok(None);
    };
    if h != proposal.header.hash() {
        return Ok(None);
    }
    let qc = qc::aggregate(precommits, validators).map_err(CommitError::Qc)?;
    qc::verify(&qc, validators).map_err(CommitError::Qc)?;
    let f = Finalized {
        height: proposal.height,
        round: proposal.round,
        block_hash: h,
        app_hash: proposal.app_hash,
    };
    log.record(&f).map_err(CommitError::Safety)?;
    Ok(Some(f))
}

/// Commit errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitError {
    /// QC.
    Qc(qc::QcError),
    /// Two commits.
    Safety(SafetyError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::propose::propose;
    use crate::vrf;
    use crypto::from_bls;
    use crypto::sig::bls as bls_mod;
    use crypto::vrf::public_key_from_seed;
    use types::header::{Header, HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{Epoch, TestClock};

    #[allow(clippy::type_complexity)]
    fn one_val() -> (
        SecretKey,
        ValidatorId,
        Map<ValidatorId, VotingPower>,
        [u8; 32],
        [u8; 32],
        VrfSeed,
        Header,
    ) {
        let sk = bls_mod::keygen().unwrap();
        let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(1));
        let mut validators = Map::new();
        validators.insert(id, VotingPower(1));
        let vrf_sk = [4u8; 32];
        let vrf_pk = public_key_from_seed(&vrf_sk);
        let seed = vrf::derive_seed(&[9u8; 32], Epoch::ZERO);
        let clock = TestClock::new(1_000);
        let fields = HeaderFields::new(&clock, Height::GENESIS, Round::ZERO, id, 0, 1).unwrap();
        let header = Header {
            fields,
            tx_root: Hash::ZERO,
            state_root: Hash::ZERO,
            receipts_root: Hash::ZERO,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        };
        (sk, id, validators, vrf_sk, vrf_pk, seed, header)
    }

    #[test]
    fn non_leader_proposal_rejected() {
        let (sk, id, validators, vrf_sk, vrf_pk, seed, header) = one_val();
        let (_, proof) = vrf::leader_prove(&vrf_sk, &seed, &id).unwrap();
        let p = propose(
            &sk,
            id,
            id,
            &vrf_pk,
            &proof,
            &validators,
            &seed,
            Height::GENESIS,
            Round::ZERO,
            || (header.clone(), Hash::from_bytes([3u8; 32])),
        )
        .unwrap();
        let mut fake = p.clone();
        fake.proposer = ValidatorId::from_bytes([9u8; 48]);
        let mut log = VoteReplayLog::new();
        let mut vrf_pks = Map::new();
        vrf_pks.insert(id, vrf_pk);
        let err = prevote_step(
            &sk,
            id,
            &fake,
            &vrf_pks,
            &seed,
            &validators,
            Height::GENESIS,
            Round::ZERO,
            None,
            None,
            &mut log,
        )
        .unwrap_err();
        assert_eq!(err, PrevoteError::NotLeader);
    }

    #[test]
    fn commit_surfaces_builder_app_hash() {
        let (sk, id, validators, vrf_sk, vrf_pk, seed, header) = one_val();
        let app = Hash::from_bytes([7u8; 32]);
        let (_, proof) = vrf::leader_prove(&vrf_sk, &seed, &id).unwrap();
        let p = propose(
            &sk,
            id,
            id,
            &vrf_pk,
            &proof,
            &validators,
            &seed,
            Height::GENESIS,
            Round::ZERO,
            || (header.clone(), app),
        )
        .unwrap();
        let mut log = VoteReplayLog::new();
        let mut vrf_pks = Map::new();
        vrf_pks.insert(id, vrf_pk);
        let pv = prevote_step(
            &sk,
            id,
            &p,
            &vrf_pks,
            &seed,
            &validators,
            Height::GENESIS,
            Round::ZERO,
            None,
            None,
            &mut log,
        )
        .unwrap();
        let (pc, lock) = precommit_step(
            &sk,
            id,
            Height::GENESIS,
            Round::ZERO,
            std::slice::from_ref(&pv),
            &validators,
            &p.header,
            &mut log,
        );
        assert!(lock.is_some());
        let mut clog = CommitLog::new();
        let f = commit(
            std::slice::from_ref(&pc),
            &validators,
            VotingPower(1),
            &p,
            &mut clog,
        )
        .unwrap()
        .unwrap();
        assert_eq!(f.app_hash, app);
        assert_eq!(f.block_hash, p.header.hash());
    }
}
