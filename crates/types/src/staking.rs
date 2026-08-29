//! Staking transaction payloads (architecture.md §2.5 validator lifecycle, §9.2).
//!
//! Bond → join → vote → optional slash → unbond → unbonding period → withdraw.
//! These ride inside [`crate::tx::Tx`] (`tx.envelope`); signing is still `tx.sign`.

use crate::tx::{Tx, TxPayload};
use crate::{Amount, ChainId, Nonce, ValidatorId};

/// Staking kind. Contract ids: `tx.stake.bond` … `tx.stake.withdraw`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StakeKind {
    /// Self-bond (`tx.stake.bond`).
    Bond = 1,
    /// Begin unbonding self-stake (`tx.stake.unbond`).
    Unbond = 2,
    /// Delegate to a validator (`tx.stake.delegate`).
    Delegate = 3,
    /// Remove delegation (`tx.stake.undelegate`).
    Undelegate = 4,
    /// Claim matured unbonding (`tx.stake.withdraw`).
    Withdraw = 5,
}

/// Payload inside [`TxPayload::Stake`]. Always carries [`Amount`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StakePayload {
    /// Kind.
    pub kind: StakeKind,
    /// Target validator (none for withdraw).
    pub validator: Option<ValidatorId>,
    /// Native tokens (`types.amount`).
    pub amount: Amount,
}

impl Tx {
    fn stake_tx(
        chain_id: ChainId,
        nonce: Nonce,
        gas_limit: u64,
        max_fee: Amount,
        payload: StakePayload,
    ) -> Self {
        Self {
            chain_id,
            nonce,
            gas_limit,
            max_fee,
            payload: TxPayload::Stake(payload),
        }
    }

    /// `tx.stake.bond`.
    pub fn stake_bond(
        chain_id: ChainId,
        nonce: Nonce,
        gas_limit: u64,
        max_fee: Amount,
        validator: ValidatorId,
        amount: Amount,
    ) -> Self {
        Self::stake_tx(
            chain_id,
            nonce,
            gas_limit,
            max_fee,
            StakePayload {
                kind: StakeKind::Bond,
                validator: Some(validator),
                amount,
            },
        )
    }

    /// `tx.stake.unbond`.
    pub fn stake_unbond(
        chain_id: ChainId,
        nonce: Nonce,
        gas_limit: u64,
        max_fee: Amount,
        validator: ValidatorId,
        amount: Amount,
    ) -> Self {
        Self::stake_tx(
            chain_id,
            nonce,
            gas_limit,
            max_fee,
            StakePayload {
                kind: StakeKind::Unbond,
                validator: Some(validator),
                amount,
            },
        )
    }

    /// `tx.stake.delegate`.
    pub fn stake_delegate(
        chain_id: ChainId,
        nonce: Nonce,
        gas_limit: u64,
        max_fee: Amount,
        validator: ValidatorId,
        amount: Amount,
    ) -> Self {
        Self::stake_tx(
            chain_id,
            nonce,
            gas_limit,
            max_fee,
            StakePayload {
                kind: StakeKind::Delegate,
                validator: Some(validator),
                amount,
            },
        )
    }

    /// `tx.stake.undelegate`.
    pub fn stake_undelegate(
        chain_id: ChainId,
        nonce: Nonce,
        gas_limit: u64,
        max_fee: Amount,
        validator: ValidatorId,
        amount: Amount,
    ) -> Self {
        Self::stake_tx(
            chain_id,
            nonce,
            gas_limit,
            max_fee,
            StakePayload {
                kind: StakeKind::Undelegate,
                validator: Some(validator),
                amount,
            },
        )
    }

    /// `tx.stake.withdraw`.
    pub fn stake_withdraw(
        chain_id: ChainId,
        nonce: Nonce,
        gas_limit: u64,
        max_fee: Amount,
        amount: Amount,
    ) -> Self {
        Self::stake_tx(
            chain_id,
            nonce,
            gas_limit,
            max_fee,
            StakePayload {
                kind: StakeKind::Withdraw,
                validator: None,
                amount,
            },
        )
    }

    /// Staking view.
    pub fn as_stake(&self) -> Option<&StakePayload> {
        match &self.payload {
            TxPayload::Stake(s) => Some(s),
            TxPayload::Transfer(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tx::Tx;
    use crate::{Amount, ChainId, ValidatorId, GAS_TRANSFER};

    #[test]
    fn bond_round_trip_in_envelope() {
        let id = ValidatorId::from_bytes([2u8; 48]);
        let tx = Tx::stake_bond(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            id,
            Amount::new(50),
        );
        let d = Tx::decode(&tx.encode()).unwrap();
        assert_eq!(d, tx);
        assert_eq!(d.as_stake().unwrap().amount, Amount::new(50));
        assert!(d.as_transfer().is_none());
    }

    #[test]
    fn withdraw_has_amount_no_validator() {
        let tx = Tx::stake_withdraw(
            ChainId::new(1),
            Nonce(1),
            GAS_TRANSFER,
            Amount::new(1),
            Amount::new(9),
        );
        assert_eq!(
            Tx::decode(&tx.encode()).unwrap().as_stake().unwrap().kind,
            StakeKind::Withdraw
        );
        assert!(tx.as_stake().unwrap().validator.is_none());
    }

    #[test]
    fn delegate_undelegate_unbond_round_trip() {
        let id = ValidatorId::from_bytes([3u8; 48]);
        for tx in [
            Tx::stake_unbond(
                ChainId::new(1),
                Nonce(1),
                GAS_TRANSFER,
                Amount::new(1),
                id,
                Amount::new(2),
            ),
            Tx::stake_delegate(
                ChainId::new(1),
                Nonce(1),
                GAS_TRANSFER,
                Amount::new(1),
                id,
                Amount::new(3),
            ),
            Tx::stake_undelegate(
                ChainId::new(1),
                Nonce(1),
                GAS_TRANSFER,
                Amount::new(1),
                id,
                Amount::new(3),
            ),
        ] {
            assert_eq!(Tx::decode(&tx.encode()).unwrap(), tx);
        }
    }
}
