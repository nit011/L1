//! Fee-per-gas priority key for mempool ordering (Tier 4).
//!
//! Totally ordered `u128`: `max_fee * 2^64 / gas_limit` (integer, no floats).

use crate::gas::gas_meter;
use types::tx::Tx;

/// Priority: higher is better. Contract: `tx.fee_priority`.
pub fn fee_priority(tx: &Tx) -> Result<u128, crate::gas::GasError> {
    let gas = gas_meter(tx)?;
    let g = u128::from(gas.max(1));
    Ok(tx.max_fee.0.saturating_mul(1u128 << 64) / g)
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Address, Amount, ChainId, Nonce, GAS_TRANSFER};

    #[test]
    fn higher_fee_ranks_higher() {
        let low = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            Address::ZERO,
            Amount::ZERO,
        );
        let high = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(100),
            Address::ZERO,
            Amount::ZERO,
        );
        assert!(fee_priority(&high).unwrap() > fee_priority(&low).unwrap());
    }

    #[test]
    fn oversized_gas_fails() {
        let mut tx = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            Address::ZERO,
            Amount::ZERO,
        );
        tx.gas_limit = types::MAX_GAS + 1;
        assert!(fee_priority(&tx).is_err());
    }
}
