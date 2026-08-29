//! Gas metering against `spec.constants` (architecture.md §3).

use types::tx::Tx;
use types::{GAS_TRANSFER, MAX_GAS};

/// Gas metering error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GasError {
    /// Declared `gas_limit` exceeds [`MAX_GAS`].
    LimitExceedsBlock,
    /// Limit below intrinsic cost.
    LimitTooLow,
}

/// Metered cost for this envelope. Contract: `tx.gas_meter`.
pub fn gas_meter(tx: &Tx) -> Result<u64, GasError> {
    if tx.gas_limit > MAX_GAS {
        return Err(GasError::LimitExceedsBlock);
    }
    let cost = match &tx.payload {
        types::tx::TxPayload::Transfer(_) => GAS_TRANSFER,
    };
    if tx.gas_limit < cost {
        return Err(GasError::LimitTooLow);
    }
    Ok(cost)
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Address, Amount, ChainId, Nonce};

    #[test]
    fn transfer_cost_and_oversize() {
        let ok = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::ZERO,
            Address::ZERO,
            Amount::ZERO,
        );
        assert_eq!(gas_meter(&ok).unwrap(), GAS_TRANSFER);
        let mut big = ok.clone();
        big.gas_limit = MAX_GAS + 1;
        assert_eq!(gas_meter(&big), Err(GasError::LimitExceedsBlock));
        let mut tiny = ok;
        tiny.gas_limit = GAS_TRANSFER - 1;
        assert_eq!(gas_meter(&tiny), Err(GasError::LimitTooLow));
    }
}
