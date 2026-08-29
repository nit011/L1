//! Chain constants that must never change after Tier 2.
//!
//! See development-plan.md §1 (decisions table) and Tier 0 work items.

/// Digest size in bytes (BLAKE3-256). Matches `types.hash`.
pub const HASH_SIZE: usize = 32;

/// Account address size in bytes. Independent of `crypto` at this tier.
pub const ADDRESS_SIZE: usize = 32;

/// Compressed BLS12-381 G1 public key size (validator identity).
pub const VALIDATOR_ID_SIZE: usize = 48;

/// Maximum serialized block size. Derived from the commodity-validator
/// bandwidth floor in architecture.md §9 (tune in Tier 8).
pub const MAX_BLOCK_BYTES: u32 = 2 * 1024 * 1024;

/// Maximum serialized transaction size.
pub const MAX_TX_BYTES: u32 = 64 * 1024;

/// Maximum number of transactions held in the local mempool (architecture.md §5).
pub const MEMPOOL_MAX_TXS: u32 = 4_096;

/// Flat minimum `max_fee` for mempool admission (architecture.md §5).
/// Not a `ParamId`: adding one would change frozen `genesis.hash`.
pub const MIN_TX_FEE: u128 = 1;

/// Maximum gas per block.
pub const MAX_GAS: u64 = 50_000_000;

/// Intrinsic gas for a native transfer (architecture.md §3 metering).
pub const GAS_TRANSFER: u64 = 21_000;

/// PLACEHOLDER: epoch length in heights. Finalized with staking (Tier 6 / 9).
pub const EPOCH_LENGTH: u64 = 100;

/// PLACEHOLDER: unbonding period in heights. Finalized with staking (Tier 6 / 9).
pub const UNBONDING_PERIOD: u64 = 1000;

/// Propose-step timeout at round 0, in milliseconds ([`crate::Clock::now_millis`] units).
pub const TIMEOUT_PROPOSE_MS: u64 = 3_000;

/// Prevote-step timeout at round 0, in milliseconds.
pub const TIMEOUT_PREVOTE_MS: u64 = 1_000;

/// Precommit-step timeout at round 0, in milliseconds.
pub const TIMEOUT_PRECOMMIT_MS: u64 = 1_000;

/// Additive extra milliseconds per consensus round (Tendermint-style).
pub const TIMEOUT_DELTA_MS: u64 = 500;

/// Maximum header timestamp drift ahead of local time, in milliseconds.
pub const MAX_TIMESTAMP_DRIFT_MS: u64 = 15_000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_match_blake3_and_bls_g1() {
        const {
            assert!(HASH_SIZE == 32);
            assert!(ADDRESS_SIZE == 32);
            assert!(VALIDATOR_ID_SIZE == 48);
            assert!(MAX_TX_BYTES < MAX_BLOCK_BYTES);
            assert!(MEMPOOL_MAX_TXS > 0);
            assert!(MIN_TX_FEE > 0);
            assert!(MAX_GAS > 0);
            assert!(GAS_TRANSFER > 0);
            assert!(GAS_TRANSFER < MAX_GAS);
        }
    }

    #[test]
    fn placeholders_are_nonzero_but_documented() {
        const {
            assert!(EPOCH_LENGTH > 0);
            assert!(UNBONDING_PERIOD > EPOCH_LENGTH);
            assert!(TIMEOUT_PROPOSE_MS > 0);
            assert!(TIMEOUT_DELTA_MS > 0);
            assert!(MAX_TIMESTAMP_DRIFT_MS > 0);
        }
    }
}
