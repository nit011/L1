//! Equivocation slashing and tombstones (architecture.md §2.4 / §2.5).
//!
//! Evidence is self-certifying (`evidence.submission` + `bls.verify`). There is
//! no appeal path in this tier.

use crate::staking::{min_self_bond_amount, StakingState};
use consensus::evidence::{submit_evidence, Evidence};
use crypto::hash::blake3::hash_to_array;
use types::collections::Set;
use types::{Amount, Hash, ParamId, ParamsRegistry, ValidatorId, SLASH_PERCENT};

/// Slash bookkeeping. Contract: `slash.apply` / `slash.tombstone`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlashState {
    /// Canonical hashes of evidence already applied (idempotency).
    pub applied: Set<Hash>,
}

fn evidence_hash(ev: &Evidence) -> Hash {
    Hash::from_bytes(hash_to_array(&ev.encode()))
}

/// Slash percent from `spec.params_registry` (same source family as min self-bond).
pub fn slash_percent(registry: &ParamsRegistry) -> u64 {
    registry.get(ParamId::SlashPercent).unwrap_or(SLASH_PERCENT)
}

/// Reduce the offender's self-bond. Duplicate evidence is a no-op.
/// Contract: `slash.apply`.
pub fn apply(
    staking: &mut StakingState,
    slash: &mut SlashState,
    registry: &ParamsRegistry,
    evidence: &Evidence,
) -> Result<Amount, consensus::vote::VerifyError> {
    min_self_bond_amount(registry);
    submit_evidence(evidence)?;
    let h = evidence_hash(evidence);
    if slash.applied.contains(&h) {
        return Ok(Amount::ZERO);
    }
    let id: ValidatorId = evidence.a.signer;
    let pct = slash_percent(registry);
    let cur = *staking.self_bond.get(&id).unwrap_or(&Amount::ZERO);
    let cut = Amount::new(cur.0.saturating_mul(u128::from(pct)) / 100);
    let next = cur.checked_sub(cut).unwrap_or(Amount::ZERO);
    staking.self_bond.insert(id, next);
    slash.applied.insert(h);
    Ok(cut)
}

/// Permanently bar the validator key from future sets / re-bond.
/// Contract: `slash.tombstone`.
pub fn tombstone(staking: &mut StakingState, id: ValidatorId) {
    staking.tombstones.insert(id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receipt::RejectReason;
    use crate::staking::{apply_stake_tx, check_min_self_bond, epoch_set_update};
    use consensus::evidence::equivocation;
    use consensus::steps::Finalized;
    use consensus::vote::prevote;
    use crypto::from_bls;
    use crypto::sig::bls;
    use types::header::{Header, HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{Address, Height, Round, TestClock, VotingPower};

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

    fn evidence_pair() -> (ValidatorId, Evidence) {
        let sk = bls::keygen().unwrap();
        let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(1));
        let a = prevote(&sk, id, Height::GENESIS, Round::ZERO, &hdr(1));
        let b = prevote(&sk, id, Height::GENESIS, Round::ZERO, &hdr(2));
        let ev = equivocation(&a, &b).unwrap();
        (id, ev)
    }

    #[test]
    fn slash_percent_exact_and_idempotent() {
        let (id, ev) = evidence_pair();
        let mut staking = StakingState::default();
        staking.self_bond.insert(id, Amount::new(100));
        let mut slash = SlashState::default();
        let registry = ParamsRegistry::new();
        assert_eq!(slash_percent(&registry), 5);
        let cut = apply(&mut staking, &mut slash, &registry, &ev).unwrap();
        assert_eq!(cut, Amount::new(5));
        assert_eq!(*staking.self_bond.get(&id).unwrap(), Amount::new(95));
        let cut2 = apply(&mut staking, &mut slash, &registry, &ev).unwrap();
        assert_eq!(cut2, Amount::ZERO);
        assert_eq!(*staking.self_bond.get(&id).unwrap(), Amount::new(95));
    }

    #[test]
    fn tombstone_rejects_rebond_and_drops_from_next_set() {
        let (id, ev) = evidence_pair();
        let mut staking = StakingState::default();
        staking.self_bond.insert(id, Amount::new(200));
        staking.operator.insert(id, Address::ZERO);
        let mut slash = SlashState::default();
        let registry = ParamsRegistry::new();
        apply(&mut staking, &mut slash, &registry, &ev).unwrap();
        tombstone(&mut staking, id);
        let tx = types::tx::Tx::stake_bond(
            types::ChainId::new(1),
            types::Nonce::ZERO,
            types::GAS_TRANSFER,
            Amount::new(1),
            id,
            Amount::new(10_000),
        );
        let err = apply_stake_tx(
            &mut staking,
            &registry,
            &Address::from_bytes([9u8; 32]),
            &tx,
        )
        .unwrap_err();
        assert_eq!(err, RejectReason::StakeTombstone);
        check_min_self_bond(Amount::new(10_000), &registry).unwrap();
        let f = Finalized {
            height: Height(99),
            round: Round::ZERO,
            block_hash: Hash::ZERO,
            app_hash: Hash::ZERO,
        };
        let out = epoch_set_update(&mut staking, &registry, &f);
        if let Some((_, set)) = out {
            assert!(!set.contains_key(&id));
        }
    }
}
