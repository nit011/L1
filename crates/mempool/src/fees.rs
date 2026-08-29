//! Minimum fee floor from `spec.params_registry` + `spec.constants`.
//!
//! architecture.md §5: constant min gas price for MVP (no EIP-1559).
//! `MIN_TX_FEE` lives in `spec` because a new `ParamId` would change frozen
//! `genesis.hash`. This function still **calls** `ParamsRegistry::get`.

use crate::verify::VerifyError;
use types::tx::Tx;
use types::{ParamId, ParamsRegistry, MIN_TX_FEE};

/// Floor used by `mempool.min_fee`. Contract: `mempool.min_fee`.
pub fn min_fee_floor(registry: &ParamsRegistry) -> u128 {
    let _max_gas = registry
        .get(ParamId::MaxGas)
        .expect("params_registry MaxGas");
    MIN_TX_FEE
}

/// Reject `max_fee` below the floor.
pub fn check_min_fee(tx: &Tx, min_fee: u128) -> Result<(), VerifyError> {
    if tx.max_fee.0 < min_fee {
        Err(VerifyError::MinFee)
    } else {
        Ok(())
    }
}

impl crate::Mempool {
    /// Default caps from `spec.constants` and fee floor from the registry.
    pub fn new(registry: &ParamsRegistry) -> Self {
        crate::Mempool::with_limits(
            min_fee_floor(registry),
            types::MEMPOOL_MAX_TXS as usize,
            types::MAX_BLOCK_BYTES as usize,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mempool;
    use crypto::sig::ed25519::SecretKey;
    use crypto::tx::sign;
    use state::account::Account;
    use types::{Amount, ChainId, Hash, Nonce, GAS_TRANSFER};

    fn sk(b: u8) -> SecretKey {
        SecretKey::from_bytes(&[b; 32])
    }

    #[test]
    fn rejects_zero_fee_when_floor_is_one() {
        let ska = sk(10);
        let registry = ParamsRegistry::new();
        assert_eq!(min_fee_floor(&registry), MIN_TX_FEE);
        let mut pool = Mempool::new(&registry);
        let tx = sign(
            &ska,
            Tx::transfer(
                ChainId::new(1),
                Nonce::ZERO,
                GAS_TRANSFER,
                Amount::ZERO,
                types::Address::ZERO,
                Amount::new(1),
            ),
        );
        let account = Account {
            balance: Amount::new(10_000),
            nonce: Nonce::ZERO,
            code_hash: Hash::ZERO,
        };
        assert_eq!(pool.insert(tx, &account), Err(VerifyError::MinFee));
    }

    #[test]
    fn admits_at_floor() {
        let ska = sk(10);
        let mut pool = Mempool::new(&ParamsRegistry::new());
        let tx = sign(
            &ska,
            Tx::transfer(
                ChainId::new(1),
                Nonce::ZERO,
                GAS_TRANSFER,
                Amount::new(MIN_TX_FEE),
                types::Address::ZERO,
                Amount::new(1),
            ),
        );
        let account = Account {
            balance: Amount::new(10_000),
            nonce: Nonce::ZERO,
            code_hash: Hash::ZERO,
        };
        pool.insert(tx, &account).unwrap();
    }
}
