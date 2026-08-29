//! Genesis state (development-plan.md §0: static validators first).
//!
//! # Frozen `genesis.hash` preimage
//!
//! Concatenate, then `blake3` (no domain tag — `genesis.hash` lists `hash.blake3` only):
//! 1. `chain_id:u64 BE`
//! 2. alloc count `u32 BE`, then each `(address:32 || balance:u128 || nonce:u64 || code_hash:32)`
//!    in **`Address` sort order** (`types::collections::Map`)
//! 3. validator count `u32 BE`, then each `(validator_id:48 || voting_power:u64)`
//!    in **`ValidatorId` sort order**
//! 4. params: each registry `(param_id:u8 || value:u64)` in `ParamId` order, then
//!    `propose, prevote, precommit, delta` timeout milliseconds (4× u64), matching
//!    [`consensus::timeout::TimeoutConfig::from_spec`] / `from_params`.

use crate::collections::Map;
use crate::hashing::blake3_array;
use crate::{
    Address, Amount, ChainId, Hash, Nonce, ParamId, ParamsRegistry, ValidatorId, VotingPower,
    TIMEOUT_DELTA_MS, TIMEOUT_PRECOMMIT_MS, TIMEOUT_PREVOTE_MS, TIMEOUT_PROPOSE_MS,
};

/// Account fields at genesis — same layout as `state::account::Account`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenesisAccount {
    /// `types.amount`.
    pub balance: Amount,
    /// `types.nonce`.
    pub nonce: Nonce,
    /// `types.hash` (code).
    pub code_hash: Hash,
}

/// Baked timeouts. Contract: `genesis.params` (via `cons.timeout.config` numbers).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenesisTimeouts {
    /// Propose ms at round 0.
    pub propose_ms: u64,
    /// Prevote ms at round 0.
    pub prevote_ms: u64,
    /// Precommit ms at round 0.
    pub precommit_ms: u64,
    /// Delta ms per round.
    pub delta_ms: u64,
}

impl GenesisTimeouts {
    /// Same values as `TimeoutConfig::from_spec()`.
    pub fn from_spec_constants() -> Self {
        Self {
            propose_ms: TIMEOUT_PROPOSE_MS,
            prevote_ms: TIMEOUT_PREVOTE_MS,
            precommit_ms: TIMEOUT_PRECOMMIT_MS,
            delta_ms: TIMEOUT_DELTA_MS,
        }
    }
}

/// Chain parameters at genesis. Contract: `genesis.params`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenesisParams {
    /// `spec.params_registry`.
    pub registry: ParamsRegistry,
    /// Consensus timeouts.
    pub timeouts: GenesisTimeouts,
}

impl GenesisParams {
    /// Registry defaults plus spec timeouts.
    pub fn from_registry(registry: ParamsRegistry) -> Self {
        Self {
            registry,
            timeouts: GenesisTimeouts::from_spec_constants(),
        }
    }
}

/// Full genesis object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Genesis {
    /// `types.chain_id`.
    pub chain_id: ChainId,
    /// Allocations. Contract: `genesis.alloc`.
    pub alloc: Map<Address, GenesisAccount>,
    /// Static validator set. Contract: `genesis.validators`.
    pub validators: Map<ValidatorId, VotingPower>,
    /// Params. Contract: `genesis.params`.
    pub params: GenesisParams,
}

impl Genesis {
    /// Empty alloc/validators, default params.
    pub fn new(chain_id: ChainId) -> Self {
        Self {
            chain_id,
            alloc: Map::new(),
            validators: Map::new(),
            params: GenesisParams::from_registry(ParamsRegistry::new()),
        }
    }

    /// Insert an allocation (fields of `state.account`).
    pub fn insert_alloc(&mut self, addr: Address, account: GenesisAccount) {
        self.alloc.insert(addr, account);
    }

    /// Insert a validator pair as produced by `validator.from_bls`.
    pub fn insert_validator(&mut self, id: ValidatorId, power: VotingPower) {
        self.validators.insert(id, power);
    }

    fn hash_preimage(&self) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&self.chain_id.0.to_be_bytes());
        p.extend_from_slice(&(self.alloc.len() as u32).to_be_bytes());
        for (addr, a) in &self.alloc {
            p.extend_from_slice(addr.as_bytes());
            p.extend_from_slice(&a.balance.0.to_be_bytes());
            p.extend_from_slice(&a.nonce.0.to_be_bytes());
            p.extend_from_slice(a.code_hash.as_bytes());
        }
        p.extend_from_slice(&(self.validators.len() as u32).to_be_bytes());
        for (id, power) in &self.validators {
            p.extend_from_slice(id.as_bytes());
            p.extend_from_slice(&power.0.to_be_bytes());
        }
        let params: Vec<_> = self.params.registry.iter().collect();
        p.extend_from_slice(&(params.len() as u32).to_be_bytes());
        for (id, v) in params {
            p.push(param_id_byte(id));
            p.extend_from_slice(&v.to_be_bytes());
        }
        p.extend_from_slice(&self.params.timeouts.propose_ms.to_be_bytes());
        p.extend_from_slice(&self.params.timeouts.prevote_ms.to_be_bytes());
        p.extend_from_slice(&self.params.timeouts.precommit_ms.to_be_bytes());
        p.extend_from_slice(&self.params.timeouts.delta_ms.to_be_bytes());
        p
    }

    /// Hash of the full genesis object. Contract: `genesis.hash`.
    pub fn hash(&self) -> Hash {
        Hash::from_bytes(blake3_array(&self.hash_preimage()))
    }
}

fn param_id_byte(id: ParamId) -> u8 {
    match id {
        ParamId::MaxBlockBytes => 0,
        ParamId::MaxTxBytes => 1,
        ParamId::MaxGas => 2,
        ParamId::EpochLength => 3,
        ParamId::UnbondingPeriod => 4,
        ParamId::TimeoutProposeMs => 5,
        ParamId::TimeoutPrevoteMs => 6,
        ParamId::TimeoutPrecommitMs => 7,
        ParamId::TimeoutDeltaMs => 8,
        ParamId::MaxTimestampDriftMs => 9,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_changes_with_alloc_and_is_deterministic() {
        let mut a = Genesis::new(ChainId::new(7));
        let mut b = Genesis::new(ChainId::new(7));
        assert_eq!(a.hash(), b.hash());
        a.insert_alloc(
            Address::from_bytes([2u8; 32]),
            GenesisAccount {
                balance: Amount::new(1),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        assert_ne!(a.hash(), b.hash());
        b.insert_alloc(
            Address::from_bytes([2u8; 32]),
            GenesisAccount {
                balance: Amount::new(1),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        assert_eq!(a.hash(), b.hash());
        assert_eq!(a.params.timeouts.propose_ms, TIMEOUT_PROPOSE_MS);
    }
}
