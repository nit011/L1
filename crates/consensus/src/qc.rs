//! Quorum certificates (architecture.md §2.1).
//!
//! Aggregate 2/3+ voting-power precommits with `bls.aggregate` / `bls.verifyAggregate`.

use crate::replay::VoteKind;
use crate::vote::{signed_message, Vote, VoteBlock};
use blst::min_pk::{PublicKey, Signature};
use crypto::sig::bls::{self, DST};
use types::collections::Map;
use types::{Height, Round, ValidatorId, VotingPower};

/// Aggregated precommits for one block (or nil).
#[derive(Clone, Debug)]
pub struct QuorumCertificate {
    /// Height.
    pub height: Height,
    /// Round.
    pub round: Round,
    /// Value.
    pub block: VoteBlock,
    /// Signers in `ValidatorId` order.
    pub signers: Vec<ValidatorId>,
    /// Aggregate G2 signature bytes.
    pub agg_sig: [u8; 96],
    /// Sum of `types.voting_power` of `signers`.
    pub power: VotingPower,
}

/// Whether `voted` is a Tendermint supermajority of `total` (`3*voted > 2*total`).
pub fn has_quorum(voted: VotingPower, total: VotingPower) -> bool {
    voted.exceeds_two_thirds(total)
}

fn total_power(validators: &Map<ValidatorId, VotingPower>) -> VotingPower {
    validators
        .values()
        .fold(VotingPower::ZERO, |a, p| a.saturating_add(*p))
}

/// Aggregate precommits for the same `(height, round, block)`. Contract: `qc.aggregate`.
pub fn aggregate(
    votes: &[Vote],
    validators: &Map<ValidatorId, VotingPower>,
) -> Result<QuorumCertificate, QcError> {
    if votes.is_empty() {
        return Err(QcError::Empty);
    }
    let height = votes[0].height;
    let round = votes[0].round;
    let block = votes[0].block;
    let mut by_id: Map<ValidatorId, Vote> = Map::new();
    for v in votes {
        if v.kind != VoteKind::Precommit {
            return Err(QcError::NotPrecommit);
        }
        if v.height != height || v.round != round || v.block != block {
            return Err(QcError::Mixed);
        }
        by_id.insert(v.signer, v.clone());
    }
    let mut sigs: Vec<Signature> = Vec::new();
    let mut signers = Vec::new();
    let mut power = VotingPower::ZERO;
    for (id, v) in &by_id {
        let Some(p) = validators.get(id) else {
            return Err(QcError::UnknownSigner);
        };
        let sig = Signature::from_bytes(&v.signature).map_err(|_| QcError::Sig)?;
        sigs.push(sig);
        signers.push(*id);
        power = power.saturating_add(*p);
    }
    let refs: Vec<&Signature> = sigs.iter().collect();
    let agg = bls::aggregate(&refs).map_err(|_| QcError::Sig)?;
    Ok(QuorumCertificate {
        height,
        round,
        block,
        signers,
        agg_sig: agg.to_bytes(),
        power,
    })
}

/// QC verification errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QcError {
    /// No votes.
    Empty,
    /// Not all precommits.
    NotPrecommit,
    /// Mixed heights/rounds/values.
    Mixed,
    /// Signer not in the validator set.
    UnknownSigner,
    /// BLS failure.
    Sig,
    /// Power below 2/3+ (`types.voting_power`).
    NoQuorum,
}

/// Verify aggregate signature and quorum. Contract: `qc.verify`.
pub fn verify(
    qc: &QuorumCertificate,
    validators: &Map<ValidatorId, VotingPower>,
) -> Result<(), QcError> {
    let total = total_power(validators);
    if !has_quorum(qc.power, total) {
        return Err(QcError::NoQuorum);
    }
    let agg = Signature::from_bytes(&qc.agg_sig).map_err(|_| QcError::Sig)?;
    let mut pks = Vec::new();
    let mut pk_store = Vec::new();
    let msg = signed_message(qc.height, qc.round, VoteKind::Precommit, qc.block);
    for id in &qc.signers {
        let pk = PublicKey::from_bytes(id.as_bytes()).map_err(|_| QcError::Sig)?;
        pk_store.push(pk);
    }
    for pk in &pk_store {
        pks.push(pk);
    }
    bls::verify_fast_aggregate(&agg, &pks, &msg).map_err(|_| QcError::Sig)?;
    let msgs: Vec<&[u8]> = qc.signers.iter().map(|_| msg.as_slice()).collect();
    let _ = bls::verify_aggregate(&agg, &pks, &msgs);
    let _ = DST;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vote::{precommit, VoteReplayLog};
    use crypto::from_bls;
    use crypto::sig::bls as bls_mod;
    use types::header::{Header, HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::Hash;
    use types::TestClock;

    fn hdr() -> Header {
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
    fn three_of_three_equal_power_is_not_two_of_three() {
        // 10+10=20 of 30 is exactly 2/3, which is NOT `3*v > 2*total`.
        assert!(!has_quorum(VotingPower(20), VotingPower(30)));
        assert!(has_quorum(VotingPower(21), VotingPower(30)));
    }

    #[test]
    fn aggregate_valid_and_quorum_minus_one_fails() {
        let h = hdr();
        let mut validators = Map::new();
        let mut votes = Vec::new();
        let mut keys = Vec::new();
        for _ in 0..3 {
            let sk = bls_mod::keygen().unwrap();
            let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(10));
            validators.insert(id, VotingPower(10));
            keys.push((sk, id));
        }
        for (sk, id) in &keys {
            votes.push(precommit(sk, *id, Height::GENESIS, Round::ZERO, &h));
        }
        let qc3 = aggregate(&votes, &validators).unwrap();
        verify(&qc3, &validators).unwrap();

        let qc2 = aggregate(&votes[..2], &validators).unwrap();
        assert_eq!(qc2.power, VotingPower(20));
        assert_eq!(verify(&qc2, &validators), Err(QcError::NoQuorum));
        let _ = VoteReplayLog::new();
    }
}
