//! Leader proposal (architecture.md §2.2–§2.3).
//!
//! A canonical VRF **source** (typically the min `ValidatorId` at this round)
//! proves; `vrf.leader.weighted` selects the proposer; that proposer calls
//! `block.builder.local` and `bls.sign`.

use crate::vrf::{self, VrfSeed};
use blst::min_pk::SecretKey;
use crypto::sig::bls;
use crypto::vrf::Proof as VrfProof;
use crypto::{apply_domain, DomainTag};
use types::collections::Map;
use types::header::Header;
use types::{Hash, Height, Round, ValidatorId, VotingPower};

/// Signed proposal.
#[derive(Clone, Debug)]
pub struct Proposal {
    /// Height.
    pub height: Height,
    /// Round.
    pub round: Round,
    /// Header (includes `header.hash` preimage fields).
    pub header: Header,
    /// Frozen `exec.app_hash` from the builder (not re-hashed here).
    pub app_hash: Hash,
    /// Proposer (VRF winner).
    pub proposer: ValidatorId,
    /// Validator whose VRF output drove the lottery.
    pub vrf_source: ValidatorId,
    /// VRF proof from `vrf_source`.
    pub vrf_proof: VrfProof,
    /// BLS signature bytes.
    pub signature: [u8; 96],
}

/// Proposal errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProposeError {
    /// Weighted lottery did not select `our_id`.
    NotLeader,
    /// VRF / empty set.
    Vrf,
}

/// Bytes signed for a proposal (`bls.sign` + header domain).
pub fn proposal_message(header: &Header) -> Vec<u8> {
    apply_domain(DomainTag::Header, &header.hash_preimage())
}

/// VRF source for `round`: validators in id order, index `round % n`.
pub fn round_vrf_source(
    validators: &Map<ValidatorId, VotingPower>,
    round: Round,
) -> Option<ValidatorId> {
    let n = validators.len();
    if n == 0 {
        return None;
    }
    validators.keys().nth((round.0 as usize) % n).copied()
}

/// Build and sign a proposal if we are the VRF leader. Contract: `cons.propose`.
#[allow(clippy::too_many_arguments)]
///
/// `build` must be `execution::builder::build_local` (or return its header/app_hash).
pub fn propose<F>(
    bls_sk: &SecretKey,
    our_id: ValidatorId,
    vrf_source: ValidatorId,
    source_vrf_pk: &[u8; 32],
    source_proof: &VrfProof,
    validators: &Map<ValidatorId, VotingPower>,
    seed: &VrfSeed,
    height: Height,
    round: Round,
    build: F,
) -> Result<Proposal, ProposeError>
where
    F: FnOnce() -> (Header, Hash),
{
    let winner = vrf::weighted_leader(source_vrf_pk, seed, &vrf_source, source_proof, validators)
        .map_err(|_| ProposeError::Vrf)?;
    if winner != our_id {
        return Err(ProposeError::NotLeader);
    }
    let (header, app_hash) = build();
    let sig = bls::sign(bls_sk, &proposal_message(&header));
    Ok(Proposal {
        height,
        round,
        header,
        app_hash,
        proposer: our_id,
        vrf_source,
        vrf_proof: source_proof.clone(),
        signature: sig.to_bytes(),
    })
}

/// True iff the proposal's VRF lottery selects the claimed proposer.
pub fn verify_leader(
    proposal: &Proposal,
    source_vrf_pk: &[u8; 32],
    seed: &VrfSeed,
    validators: &Map<ValidatorId, VotingPower>,
) -> bool {
    match vrf::weighted_leader(
        source_vrf_pk,
        seed,
        &proposal.vrf_source,
        &proposal.vrf_proof,
        validators,
    ) {
        Ok(id) => id == proposal.proposer,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::from_bls;
    use crypto::vrf::public_key_from_seed;
    use types::header::{HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{TestClock, VotingPower};

    fn dummy_header(id: ValidatorId) -> Header {
        let clock = TestClock::new(1_000);
        let fields = HeaderFields::new(&clock, Height::GENESIS, Round::ZERO, id, 0, 1).unwrap();
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
    fn non_leader_cannot_propose() {
        let ska = bls::keygen().unwrap();
        let skb = bls::keygen().unwrap();
        let (ida, _) = from_bls(&ska.sk_to_pk(), VotingPower(1));
        let (idb, _) = from_bls(&skb.sk_to_pk(), VotingPower(1));
        let mut validators = Map::new();
        validators.insert(ida, VotingPower(1));
        validators.insert(idb, VotingPower(1));
        let src = round_vrf_source(&validators, Round::ZERO).unwrap();
        let src_sk = [9u8; 32];
        let src_pk = public_key_from_seed(&src_sk);
        let seed = vrf::derive_seed(&[1u8; 32], types::Epoch::ZERO);
        let (_, proof) = vrf::leader_prove(&src_sk, &seed, &src).unwrap();
        let winner = vrf::weighted_leader(&src_pk, &seed, &src, &proof, &validators).unwrap();
        let loser = if winner == ida { idb } else { ida };
        let loser_sk = if loser == ida { &ska } else { &skb };
        let r = propose(
            loser_sk,
            loser,
            src,
            &src_pk,
            &proof,
            &validators,
            &seed,
            Height::GENESIS,
            Round::ZERO,
            || (dummy_header(loser), Hash::ZERO),
        );
        assert_eq!(r.unwrap_err(), ProposeError::NotLeader);
    }
}
