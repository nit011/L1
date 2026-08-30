//! WASM fuel metering built on Tier 3 `tx.gas_meter` (architecture.md §3).
//!
//! Intrinsic cost comes from [`crate::gas::gas_meter`]. Remaining
//! `gas_limit - intrinsic` is wasmtime fuel (one unit per wasm operator).
//! Exhaustion is deterministic: the same bytecode and fuel halt at the
//! same remaining-fuel value on every rerun.

use crate::gas::{gas_meter, GasError};
use types::tx::Tx;

/// Intrinsic gas via `tx.gas_meter`. Contract: `wasm.meter`.
pub fn meter(tx: &Tx) -> Result<u64, GasError> {
    gas_meter(tx)
}

/// Wasmtime fuel = declared limit minus intrinsic (never negative).
pub fn fuel_budget(tx: &Tx, intrinsic: u64) -> u64 {
    tx.gas_limit.saturating_sub(intrinsic)
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Address, Amount, ChainId, Nonce, GAS_CALL, GAS_DEPLOY, GAS_TRANSFER};

    #[test]
    fn meter_delegates_to_gas_meter() {
        let t = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::ZERO,
            Address::ZERO,
            Amount::ZERO,
        );
        assert_eq!(meter(&t).unwrap(), GAS_TRANSFER);
        let d = Tx::deploy(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_DEPLOY,
            Amount::ZERO,
            vec![0],
        );
        assert_eq!(meter(&d).unwrap(), GAS_DEPLOY);
        let c = Tx::call(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_CALL,
            Amount::ZERO,
            Address::ZERO,
            vec![],
        );
        assert_eq!(meter(&c).unwrap(), GAS_CALL);
    }

    #[test]
    fn fuel_budget_is_limit_minus_intrinsic() {
        let c = Tx::call(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_CALL + 50,
            Amount::ZERO,
            Address::ZERO,
            vec![],
        );
        assert_eq!(fuel_budget(&c, GAS_CALL), 50);
    }

    #[test]
    fn meter_rejects_limit_too_low() {
        let c = Tx::call(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_CALL - 1,
            Amount::ZERO,
            Address::ZERO,
            vec![],
        );
        assert_eq!(meter(&c), Err(GasError::LimitTooLow));
    }
}
