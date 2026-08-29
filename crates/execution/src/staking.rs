//! Staking execution (architecture.md §2.5 validator lifecycle, §9.2 caps).
//!
//! Bond → join set → propose/vote → optional slash → unbond → unbonding
//! period → withdraw. This module **observes** [`Finalized`] from `cons.commit`
//! and never sits on `cons.commit`'s call graph.

use crate::receipt::RejectReason;
use consensus::steps::Finalized;
use types::collections::{Map, Set};
use types::header::validators_hash;
use types::spec::{DELEGATION_CAP, EPOCH_LENGTH, MIN_SELF_BOND, UNBONDING_PERIOD};
use types::staking::StakeKind;
use types::tx::Tx;
use types::{
    Address, Amount, Epoch, Hash, Height, ParamId, ParamsRegistry, ValidatorId, VotingPower,
};

/// One pending unbond (`staking.unbonding_period`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnbondEntry {
    /// Account that called `tx.stake.unbond`.
    pub owner: Address,
    /// Validator whose self-bond is leaving.
    pub validator: ValidatorId,
    /// Tokens.
    pub amount: Amount,
    /// First height at which `tx.stake.withdraw` may succeed.
    pub unlock_height: Height,
    /// Epoch of [`Self::unlock_height`] (`types.epoch`).
    pub unlock_epoch: Epoch,
}

/// Sorted staking ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StakingState {
    /// Self-bond per validator id.
    pub self_bond: Map<ValidatorId, Amount>,
    /// Account that controls a validator's self-bond.
    pub operator: Map<ValidatorId, Address>,
    /// Delegation `(delegator, validator) → amount`.
    pub delegations: Map<(Address, ValidatorId), Amount>,
    /// Pending unbonds keyed by insertion id.
    pub pending_unbond: Map<u64, UnbondEntry>,
    next_unbond: u64,
    /// Tombstoned BLS keys (`slash.tombstone`).
    pub tombstones: Set<ValidatorId>,
    /// Set used for the **current** epoch's rounds (not retroactive).
    pub current_set: Map<ValidatorId, VotingPower>,
    /// Last observed `cons.commit` height.
    pub last_commit_height: Height,
    /// Last observed `cons.commit` block hash.
    pub last_commit_hash: Hash,
}

impl Default for StakingState {
    fn default() -> Self {
        Self {
            self_bond: Map::new(),
            operator: Map::new(),
            delegations: Map::new(),
            pending_unbond: Map::new(),
            next_unbond: 0,
            tombstones: Set::new(),
            current_set: Map::new(),
            last_commit_height: Height::GENESIS,
            last_commit_hash: Hash::ZERO,
        }
    }
}

/// Minimum self-bond from `spec.params_registry` (architecture.md §9.2).
/// Contract: `staking.min_self_bond`.
pub fn min_self_bond_amount(registry: &ParamsRegistry) -> Amount {
    Amount::new(u128::from(
        registry.get(ParamId::MinSelfBond).unwrap_or(MIN_SELF_BOND),
    ))
}

/// Reject a bond below the minimum. Distinguishes [`RejectReason::StakeMinBond`].
pub fn check_min_self_bond(amount: Amount, registry: &ParamsRegistry) -> Result<(), RejectReason> {
    if amount < min_self_bond_amount(registry) {
        Err(RejectReason::StakeMinBond)
    } else {
        Ok(())
    }
}

/// Delegation voting-power cap from `spec.params_registry` (architecture.md §9.2).
/// Contract: `staking.delegation_cap`.
pub fn delegation_cap(registry: &ParamsRegistry) -> u64 {
    registry
        .get(ParamId::DelegationCap)
        .unwrap_or(DELEGATION_CAP)
}

fn amount_power(a: Amount) -> u64 {
    u64::try_from(a.0.min(u128::from(u64::MAX))).unwrap_or(u64::MAX)
}

fn delegated_to(book: &StakingState, id: &ValidatorId) -> Amount {
    book.delegations
        .iter()
        .filter(|((_, v), _)| v == id)
        .fold(Amount::ZERO, |acc, (_, a)| {
            acc.checked_add(*a).unwrap_or(acc)
        })
}

/// Effective proposer/voting weight: self-bond plus delegation **up to the cap**.
/// Stake above the cap stays recorded in [`StakingState::delegations`].
pub fn effective_power(
    book: &StakingState,
    id: &ValidatorId,
    registry: &ParamsRegistry,
) -> VotingPower {
    let self_p = amount_power(*book.self_bond.get(id).unwrap_or(&Amount::ZERO));
    let del_p = amount_power(delegated_to(book, id));
    let capped_del = del_p.min(delegation_cap(registry));
    VotingPower(self_p.saturating_add(capped_del))
}

/// Unlock height/epoch after `tx.stake.unbond` (`types.epoch` + registry period).
/// Contract: `staking.unbonding_period`.
pub fn unbonding_unlock(last_commit: Height, registry: &ParamsRegistry) -> (Height, Epoch) {
    let period = registry
        .get(ParamId::UnbondingPeriod)
        .unwrap_or(UNBONDING_PERIOD);
    let epoch_len = registry.get(ParamId::EpochLength).unwrap_or(EPOCH_LENGTH);
    let unlock = Height(last_commit.0.saturating_add(period));
    let unlock_epoch = Epoch(if epoch_len == 0 {
        0
    } else {
        unlock.0 / epoch_len
    });
    (unlock, unlock_epoch)
}

fn matured_withdrawable(book: &StakingState, owner: &Address) -> Amount {
    book.pending_unbond
        .values()
        .filter(|e| e.owner == *owner && e.unlock_height.0 <= book.last_commit_height.0)
        .fold(Amount::ZERO, |acc, e| {
            acc.checked_add(e.amount).unwrap_or(acc)
        })
}

/// Apply a staking payload. Does not move account balances (see `exec.seq.apply_tx`).
pub fn apply_stake_tx(
    book: &mut StakingState,
    registry: &ParamsRegistry,
    from: &Address,
    tx: &Tx,
) -> Result<(), RejectReason> {
    let s = tx.as_stake().ok_or(RejectReason::Gas)?;
    match s.kind {
        StakeKind::Bond => {
            let id = s.validator.ok_or(RejectReason::Gas)?;
            if book.tombstones.contains(&id) {
                return Err(RejectReason::StakeTombstone);
            }
            check_min_self_bond(s.amount, registry)?;
            if let Some(op) = book.operator.get(&id) {
                if op != from {
                    return Err(RejectReason::StakeInsufficient);
                }
            } else {
                book.operator.insert(id, *from);
            }
            let cur = *book.self_bond.get(&id).unwrap_or(&Amount::ZERO);
            let next = cur
                .checked_add(s.amount)
                .ok_or(RejectReason::StakeInsufficient)?;
            book.self_bond.insert(id, next);
            Ok(())
        }
        StakeKind::Unbond => {
            let id = s.validator.ok_or(RejectReason::Gas)?;
            if book.operator.get(&id) != Some(from) {
                return Err(RejectReason::StakeInsufficient);
            }
            let cur = *book.self_bond.get(&id).unwrap_or(&Amount::ZERO);
            let next = cur
                .checked_sub(s.amount)
                .ok_or(RejectReason::StakeInsufficient)?;
            let min = min_self_bond_amount(registry);
            if next != Amount::ZERO && next < min {
                return Err(RejectReason::StakeMinBond);
            }
            book.self_bond.insert(id, next);
            let (unlock_height, unlock_epoch) = unbonding_unlock(book.last_commit_height, registry);
            let seq = book.next_unbond;
            book.next_unbond = book.next_unbond.saturating_add(1);
            book.pending_unbond.insert(
                seq,
                UnbondEntry {
                    owner: *from,
                    validator: id,
                    amount: s.amount,
                    unlock_height,
                    unlock_epoch,
                },
            );
            Ok(())
        }
        StakeKind::Delegate => {
            let id = s.validator.ok_or(RejectReason::Gas)?;
            if book.tombstones.contains(&id) {
                return Err(RejectReason::StakeTombstone);
            }
            let key = (*from, id);
            let cur = *book.delegations.get(&key).unwrap_or(&Amount::ZERO);
            let next = cur
                .checked_add(s.amount)
                .ok_or(RejectReason::StakeInsufficient)?;
            book.delegations.insert(key, next);
            Ok(())
        }
        StakeKind::Undelegate => {
            let id = s.validator.ok_or(RejectReason::Gas)?;
            let key = (*from, id);
            let cur = *book.delegations.get(&key).unwrap_or(&Amount::ZERO);
            let next = cur
                .checked_sub(s.amount)
                .ok_or(RejectReason::StakeInsufficient)?;
            if next == Amount::ZERO {
                book.delegations.remove(&key);
            } else {
                book.delegations.insert(key, next);
            }
            Ok(())
        }
        StakeKind::Withdraw => {
            let avail = matured_withdrawable(book, from);
            if s.amount > avail {
                let any_pending = book.pending_unbond.values().any(|e| e.owner == *from);
                if any_pending && avail < s.amount {
                    return Err(RejectReason::StakeUnbonding);
                }
                return Err(RejectReason::StakeInsufficient);
            }
            let mut left = s.amount;
            let keys: Vec<u64> = book.pending_unbond.keys().copied().collect();
            for k in keys {
                if left == Amount::ZERO {
                    break;
                }
                let Some(e) = book.pending_unbond.get(&k).cloned() else {
                    continue;
                };
                if e.owner != *from || e.unlock_height.0 > book.last_commit_height.0 {
                    continue;
                }
                if e.amount <= left {
                    left = left.checked_sub(e.amount).unwrap();
                    book.pending_unbond.remove(&k);
                } else {
                    let rem = e.amount.checked_sub(left).unwrap();
                    left = Amount::ZERO;
                    book.pending_unbond
                        .insert(k, UnbondEntry { amount: rem, ..e });
                }
            }
            Ok(())
        }
    }
}

/// Observe a `cons.commit` outcome. Does not call into consensus.
pub fn observe_commit(book: &mut StakingState, finalized: &Finalized) {
    book.last_commit_height = finalized.height;
    book.last_commit_hash = finalized.block_hash;
}

fn compute_next_set(
    book: &StakingState,
    registry: &ParamsRegistry,
) -> Map<ValidatorId, VotingPower> {
    let min = min_self_bond_amount(registry);
    let mut set = Map::new();
    for (id, amt) in &book.self_bond {
        if book.tombstones.contains(id) {
            continue;
        }
        if *amt < min {
            continue;
        }
        let p = effective_power(book, id, registry);
        if p.0 > 0 {
            set.insert(*id, p);
        }
    }
    set
}

/// At an epoch boundary (from observed `cons.commit` heights), recompute the
/// next validator set and [`validators_hash`] (`block.validators_hash`).
///
/// The new set is installed as [`StakingState::current_set`] only after the last
/// height of epoch N, i.e. it is used starting in epoch N+1 — never retroactively.
/// Contract: `staking.epoch_set_update`.
pub fn epoch_set_update(
    book: &mut StakingState,
    registry: &ParamsRegistry,
    finalized: &Finalized,
) -> Option<(Hash, Map<ValidatorId, VotingPower>)> {
    observe_commit(book, finalized);
    let epoch_len = registry.get(ParamId::EpochLength).unwrap_or(EPOCH_LENGTH);
    if epoch_len == 0 {
        return None;
    }
    let this_ep = finalized.height.0 / epoch_len;
    let next_ep = finalized.height.0.saturating_add(1) / epoch_len;
    if this_ep == next_ep {
        return None;
    }
    let set = compute_next_set(book, registry);
    let h = validators_hash(&set);
    book.current_set = set.clone();
    Some((h, set))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receipt::RejectReason;
    use crate::seq::{apply_tx, World};
    use consensus::vrf;
    use crypto::address::from_ed25519;
    use crypto::from_bls;
    use crypto::sig::bls;
    use crypto::sig::ed25519::SecretKey;
    use crypto::tx::sign;
    use crypto::vrf::public_key_from_seed;
    use types::genesis::GenesisAccount;
    use types::tx::Tx;
    use types::{ChainId, Nonce, GAS_TRANSFER};

    fn ed_sk(b: u8) -> SecretKey {
        SecretKey::from_bytes(&[b; 32])
    }

    fn dummy_finalized(h: u64) -> Finalized {
        Finalized {
            height: Height(h),
            round: types::Round::ZERO,
            block_hash: Hash::from_bytes([h as u8; 32]),
            app_hash: Hash::ZERO,
        }
    }

    #[test]
    fn min_self_bond_rejects_below_threshold() {
        let registry = ParamsRegistry::new();
        let min = min_self_bond_amount(&registry);
        assert_eq!(min, Amount::new(u128::from(MIN_SELF_BOND)));
        assert_eq!(
            check_min_self_bond(Amount::new(min.0 - 1), &registry),
            Err(RejectReason::StakeMinBond)
        );
        check_min_self_bond(min, &registry).unwrap();
    }

    #[test]
    fn bond_below_min_rejected_via_apply_tx() {
        let sk = ed_sk(3);
        let from = from_ed25519(&sk.verifying_key());
        let vid = ValidatorId::from_bytes([7u8; 48]);
        let mut g = types::genesis::Genesis::new(ChainId::new(1));
        g.insert_alloc(
            from,
            GenesisAccount {
                balance: Amount::new(10_000),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        let mut world = World::from_genesis(&g);
        let tx = Tx::stake_bond(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            vid,
            Amount::new(u128::from(MIN_SELF_BOND) - 1),
        );
        let r = apply_tx(&mut world, &sign(&sk, tx));
        assert!(!r.success);
        assert_eq!(r.reason, Some(RejectReason::StakeMinBond));
    }

    #[test]
    fn delegation_cap_boundary_1000_plus_1() {
        // Cap X=1000 (DELEGATION_CAP). Delegation X+1=1001. Effective power from
        // delegation stays X; the extra token remains in the ledger.
        let mut book = StakingState::default();
        let registry = ParamsRegistry::new();
        let id = ValidatorId::from_bytes([1u8; 48]);
        let del = Address::from_bytes([2u8; 32]);
        book.self_bond.insert(id, Amount::ZERO);
        book.delegations
            .insert((del, id), Amount::new(u128::from(DELEGATION_CAP) + 1));
        let p = effective_power(&book, &id, &registry);
        assert_eq!(p.0, DELEGATION_CAP);
        assert_eq!(
            delegated_to(&book, &id),
            Amount::new(u128::from(DELEGATION_CAP) + 1)
        );
        book.delegations
            .insert((del, id), Amount::new(u128::from(DELEGATION_CAP)));
        assert_eq!(effective_power(&book, &id, &registry).0, DELEGATION_CAP);
        book.delegations
            .insert((del, id), Amount::new(u128::from(DELEGATION_CAP) - 1));
        assert_eq!(effective_power(&book, &id, &registry).0, DELEGATION_CAP - 1);
    }

    #[test]
    fn delegate_and_undelegate_ledger() {
        let mut book = StakingState::default();
        let registry = ParamsRegistry::new();
        let from = Address::from_bytes([8u8; 32]);
        let id = ValidatorId::from_bytes([4u8; 48]);
        let d = Tx::stake_delegate(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            id,
            Amount::new(40),
        );
        apply_stake_tx(&mut book, &registry, &from, &d).unwrap();
        assert_eq!(delegated_to(&book, &id), Amount::new(40));
        let u = Tx::stake_undelegate(
            ChainId::new(1),
            Nonce(1),
            GAS_TRANSFER,
            Amount::new(1),
            id,
            Amount::new(10),
        );
        apply_stake_tx(&mut book, &registry, &from, &u).unwrap();
        assert_eq!(delegated_to(&book, &id), Amount::new(30));
    }

    #[test]
    fn unbond_not_withdrawable_until_period_elapses() {
        let sk = ed_sk(4);
        let from = from_ed25519(&sk.verifying_key());
        let bls_sk = bls::keygen().unwrap();
        let (vid, _) = from_bls(&bls_sk.sk_to_pk(), VotingPower(1));
        let mut g = types::genesis::Genesis::new(ChainId::new(1));
        g.params.registry.set(ParamId::UnbondingPeriod, 10);
        g.insert_alloc(
            from,
            GenesisAccount {
                balance: Amount::new(10_000),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        let mut world = World::from_genesis(&g);
        observe_commit(&mut world.staking, &dummy_finalized(0));
        let bond = Tx::stake_bond(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            vid,
            Amount::new(200),
        );
        assert!(apply_tx(&mut world, &sign(&sk, bond)).success);
        let unbond = Tx::stake_unbond(
            ChainId::new(1),
            Nonce(1),
            GAS_TRANSFER,
            Amount::new(1),
            vid,
            Amount::new(200),
        );
        assert!(apply_tx(&mut world, &sign(&sk, unbond)).success);
        let (unlock, ep) = unbonding_unlock(Height(0), &world.params);
        assert_eq!(unlock, Height(10));
        assert_eq!(ep, Epoch(0));
        let early = Tx::stake_withdraw(
            ChainId::new(1),
            Nonce(2),
            GAS_TRANSFER,
            Amount::new(1),
            Amount::new(200),
        );
        let r = apply_tx(&mut world, &sign(&sk, early));
        assert!(!r.success);
        assert_eq!(r.reason, Some(RejectReason::StakeUnbonding));
        observe_commit(&mut world.staking, &dummy_finalized(10));
        let late = Tx::stake_withdraw(
            ChainId::new(1),
            Nonce(2),
            GAS_TRANSFER,
            Amount::new(1),
            Amount::new(200),
        );
        assert!(apply_tx(&mut world, &sign(&sk, late)).success);
    }

    #[test]
    fn epoch_set_update_applies_next_epoch_only_and_changes_leader() {
        let mut registry = ParamsRegistry::new();
        registry.set(ParamId::EpochLength, 2);
        let mut book = StakingState::default();
        let a_sk = bls::keygen().unwrap();
        let b_sk = bls::keygen().unwrap();
        let c_sk = bls::keygen().unwrap();
        let (a, _) = from_bls(&a_sk.sk_to_pk(), VotingPower(1));
        let (b, _) = from_bls(&b_sk.sk_to_pk(), VotingPower(1));
        let (c, _) = from_bls(&c_sk.sk_to_pk(), VotingPower(1));
        book.self_bond.insert(a, Amount::new(100));
        book.self_bond.insert(b, Amount::new(100));
        book.current_set.insert(a, VotingPower(100));
        book.current_set.insert(b, VotingPower(100));
        let old = book.current_set.clone();
        assert!(epoch_set_update(&mut book, &registry, &dummy_finalized(0)).is_none());
        assert_eq!(book.current_set, old);
        book.self_bond.insert(c, Amount::new(50_000));
        let mid = epoch_set_update(&mut book, &registry, &dummy_finalized(1));
        assert!(mid.is_some());
        let (_vh, new_set) = mid.unwrap();
        assert!(new_set.contains_key(&c));
        assert_eq!(book.current_set, new_set);
        let vrf_sk = [9u8; 32];
        let vrf_pk = public_key_from_seed(&vrf_sk);
        let seed = vrf::derive_seed(&[1u8; 32], Epoch::ZERO);
        let (_, proof) = vrf::leader_prove(&vrf_sk, &seed, &a).unwrap();
        let lead_old = vrf::weighted_leader(&vrf_pk, &seed, &a, &proof, &old).unwrap();
        let lead_new = vrf::weighted_leader(&vrf_pk, &seed, &a, &proof, &new_set).unwrap();
        assert_ne!(lead_old, lead_new, "vrf.leader.weighted on the new set");
        assert!(new_set.get(&c).copied().unwrap().0 >= 50_000);
    }
}
