//! Queryable named parameters with defaults from [`crate::spec`].
//!
//! Constants that must never change post-Tier-2 live in `spec`. This registry
//! is the mutable superset (genesis / governance later). Epoch length and
//! unbonding period are PLACEHOLDER until staking (Tier 6 / 9).

use crate::collections::Map;
use crate::spec::{EPOCH_LENGTH, MAX_BLOCK_BYTES, MAX_GAS, MAX_TX_BYTES, UNBONDING_PERIOD};

/// Named chain parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ParamId {
    /// Maximum block size in bytes.
    MaxBlockBytes,
    /// Maximum transaction size in bytes.
    MaxTxBytes,
    /// Maximum gas per block.
    MaxGas,
    /// PLACEHOLDER epoch length (heights).
    EpochLength,
    /// PLACEHOLDER unbonding period (heights).
    UnbondingPeriod,
}

/// Mutable parameter map. Uses [`crate::collections::Map`] so iteration is ordered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamsRegistry {
    values: Map<ParamId, u64>,
}

impl Default for ParamsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ParamsRegistry {
    /// Defaults from [`crate::spec`].
    pub fn new() -> Self {
        let mut values = Map::new();
        values.insert(ParamId::MaxBlockBytes, u64::from(MAX_BLOCK_BYTES));
        values.insert(ParamId::MaxTxBytes, u64::from(MAX_TX_BYTES));
        values.insert(ParamId::MaxGas, MAX_GAS);
        values.insert(ParamId::EpochLength, EPOCH_LENGTH);
        values.insert(ParamId::UnbondingPeriod, UNBONDING_PERIOD);
        Self { values }
    }

    /// Read a parameter.
    pub fn get(&self, id: ParamId) -> Option<u64> {
        self.values.get(&id).copied()
    }

    /// Set a parameter. `MaxBlockBytes` / `MaxTxBytes` / `MaxGas` should not
    /// change after Tier 2; later code must refuse that. This layer only stores.
    pub fn set(&mut self, id: ParamId, value: u64) {
        self.values.insert(id, value);
    }

    /// Ordered iterator (deterministic).
    pub fn iter(&self) -> impl Iterator<Item = (ParamId, u64)> + '_ {
        self.values.iter().map(|(k, v)| (*k, *v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let p = ParamsRegistry::new();
        assert_eq!(p.get(ParamId::MaxGas), Some(MAX_GAS));
        assert_eq!(p.get(ParamId::EpochLength), Some(EPOCH_LENGTH));
    }

    #[test]
    fn set_and_ordered_iter() {
        let mut p = ParamsRegistry::new();
        p.set(ParamId::EpochLength, 7);
        assert_eq!(p.get(ParamId::EpochLength), Some(7));
        let keys: Vec<_> = p.iter().map(|(k, _)| k).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }
}
