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

/// Optional EIP-1559 base-fee update (architecture.md §9 / development-plan.md §1).
///
/// **Default OFF:** operators must pass `enabled = true` (chain config). When
/// off, the return value is exactly `min_floor` from [`mempool::min_fee_floor`]
/// — the Tier 4 flat fee. When on, the next base fee moves with utilization
/// against `limits.max_gas` (`gas_used / max_gas`), never below `min_floor`.
///
/// This crate cannot import `node` or `mempool` (cycles); callers pass those
/// values. Contract: `fee.1559_optional`.
pub fn next_base_fee(
    current_base: u128,
    gas_used: u64,
    max_gas: u64,
    enabled: bool,
    min_floor: u128,
) -> u128 {
    if !enabled {
        return min_floor;
    }
    let cap = max_gas.max(1);
    let used = gas_used.min(cap);
    let target = cap / 2;
    let mut next = current_base.max(min_floor);
    if used > target {
        let delta = next.saturating_mul(u128::from(used - target)) / u128::from(target.max(1)) / 8;
        next = next.saturating_add(delta.max(1));
    } else if used < target {
        let delta = next.saturating_mul(u128::from(target - used)) / u128::from(target.max(1)) / 8;
        next = next.saturating_sub(delta).max(min_floor);
    }
    next
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

    #[test]
    fn eip1559_off_is_flat_min_floor() {
        let g = types::genesis::Genesis::new(ChainId::new(1));
        let floor = mempool::fees::min_fee_floor(&g.params.registry);
        let max_gas = types::MAX_GAS;
        assert_eq!(next_base_fee(9, max_gas, max_gas, false, floor), floor);
        assert_eq!(next_base_fee(9, 0, max_gas, false, floor), floor);
    }

    #[test]
    fn eip1559_on_moves_with_max_gas_utilization() {
        let g = types::genesis::Genesis::new(ChainId::new(1));
        let floor = mempool::fees::min_fee_floor(&g.params.registry);
        let max_gas = types::MAX_GAS;
        let busy = next_base_fee(8, max_gas, max_gas, true, floor);
        let idle = next_base_fee(8, 0, max_gas, true, floor);
        assert!(busy > idle, "{busy} vs {idle}");
        assert!(idle >= floor);
    }
}
