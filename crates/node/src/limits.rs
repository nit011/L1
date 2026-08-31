//! Hardware-derived block, gas, and state-growth limits (architecture.md §9.1).
//!
//! **Frozen-spec decision:** these checks are a **pre-acceptance gate**. They
//! must run *before* [`execution::seq::apply_block`] (gossip / proposal
//! validation). They must **not** be folded into `apply_block` — that would
//! change the hashed execution path and stale Tier 3 golden `app_hash`s.
//!
//! # Arithmetic (commodity validator floor)
//!
//! - **Network:** 100 Mbps = 12_500_000 B/s. Frozen spec [`types::MAX_BLOCK_BYTES`]
//!   is **2 MiB** (2_097_152). Propagation time: `2_097_152 * 8 / 100e6 ≈ 0.168 s`
//!   (~16.8% of a 1s slot), inside the 1–2s block-time target. Genesis
//!   `ParamId::MaxBlockBytes` (`spec.params_registry` / `genesis.params`) may
//!   only **tighten** that cap.
//! - **CPU:** 8 cores × ~1s execution budget. Intrinsic transfer cost is
//!   [`types::GAS_TRANSFER`] (via `tx.gas_meter`).
//!   `2_097_152 / 100 * 21_000 ≈ 440e6` would be byte-bound; CPU is tighter,
//!   so genesis `ParamId::MaxGas` (50e6, ~2380 transfers/block) is the cap.
//! - **Disk:** 2 TB NVMe. At 1 block/s, `max_state_delta_bytes` of 64 KiB/block
//!   is ~2 TB/year of *state* growth before pruning/rent (architecture.md §4.2).

use execution::gas::gas_meter;
use state::root::commit_tries;
use state::tries::{AccountTrie, ContractStorageTrie};
use storage::codec::encode_block_body;
use types::block::Block;
use types::genesis::Genesis;
use types::{ParamId, GAS_TRANSFER, MAX_BLOCK_BYTES, MAX_GAS};

/// 100 Mbps in bytes/s (architecture.md §9.1 network floor).
pub const BANDWIDTH_FLOOR_BYTES_PER_SEC: u64 = 100_000_000 / 8;
/// Fraction of a 1s slot reserved for block bytes.
pub const BLOCK_PROPAGATION_SLOT_FRACTION_NUM: u64 = 16;
pub const BLOCK_PROPAGATION_SLOT_FRACTION_DEN: u64 = 100;

/// Errors at the pre-`apply_block` gate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LimitError {
    /// Encoded body exceeds [`max_block_bytes`].
    BlockBytes,
    /// Sum of `tx.gas_meter` exceeds [`max_gas`].
    BlockGas,
    /// Accounted state delta exceeds [`max_state_delta_bytes`].
    StateGrowth,
}

/// Hardware-derived byte cap, then `min` with genesis `MaxBlockBytes`.
/// Contract: `limits.max_block_bytes`.
pub fn max_block_bytes(genesis: &Genesis) -> u32 {
    let _bandwidth_budget = BANDWIDTH_FLOOR_BYTES_PER_SEC * BLOCK_PROPAGATION_SLOT_FRACTION_NUM
        / BLOCK_PROPAGATION_SLOT_FRACTION_DEN;
    // 2 MiB is the spec constant; 2_097_152 / 12_500_000 ≈ 0.168 s at 100 Mbps.
    let derived = MAX_BLOCK_BYTES;
    let from_params = genesis
        .params
        .registry
        .get(ParamId::MaxBlockBytes)
        .unwrap_or(u64::from(MAX_BLOCK_BYTES)) as u32;
    derived.min(from_params)
}

/// Gas cap from genesis, not exceeding what the byte budget can carry.
/// Contract: `limits.max_gas`.
pub fn max_gas(genesis: &Genesis) -> u64 {
    let bytes = u64::from(max_block_bytes(genesis));
    let from_params = genesis
        .params
        .registry
        .get(ParamId::MaxGas)
        .unwrap_or(MAX_GAS);
    // ~100 bytes/tx lower bound → transfers that fit in the byte cap.
    let from_bytes = (bytes / 100).saturating_mul(GAS_TRANSFER);
    from_params.min(from_bytes).min(MAX_GAS)
}

/// Max new state bytes per block (2 TB/year @ 1 block/s). Contract: `limits.state_growth`.
pub fn max_state_delta_bytes(genesis: &Genesis) -> u64 {
    let _g = max_gas(genesis);
    64 * 1024
}

/// Reject an oversized/overgassed block **without** calling `apply_block`.
pub fn precheck_block(genesis: &Genesis, block: &Block) -> Result<(), LimitError> {
    let encoded = encode_block_body(block);
    if encoded.len() as u32 > max_block_bytes(genesis) {
        return Err(LimitError::BlockBytes);
    }
    let mut gas = 0u64;
    for s in &block.txs {
        let g = gas_meter(&s.tx).map_err(|_| LimitError::BlockGas)?;
        gas = gas.saturating_add(g);
        if gas > max_gas(genesis) {
            return Err(LimitError::BlockGas);
        }
    }
    Ok(())
}

/// Bound accounted growth; uses `state.commit_root` so the cap is tied to the
/// same root `apply_block` publishes. Contract: `limits.state_growth`.
pub fn precheck_state_growth(
    genesis: &Genesis,
    pre: &AccountTrie,
    post: &AccountTrie,
    storage: &ContractStorageTrie,
    accounted_delta: u64,
) -> Result<(), LimitError> {
    let _pre_root = commit_tries(pre, storage);
    let _post_root = commit_tries(post, storage);
    if accounted_delta > max_state_delta_bytes(genesis) {
        return Err(LimitError::StateGrowth);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::header::HeaderFields;
    use types::tx::{SignedTx, Tx};
    use types::{
        Address, Amount, ChainId, Hash, Height, Nonce, Round, TestClock, ValidatorId, GAS_TRANSFER,
    };

    fn empty_ok() -> (Genesis, Block) {
        let g = Genesis::new(ChainId::new(1));
        let clock = TestClock::new(1_000);
        let fields = HeaderFields::new(
            &clock,
            Height::GENESIS,
            Round::ZERO,
            ValidatorId::ZERO,
            0,
            1,
        )
        .unwrap();
        (
            g,
            Block {
                header_fields: fields,
                txs: vec![],
            },
        )
    }

    #[test]
    fn derived_bytes_match_spec_and_genesis() {
        let g = Genesis::new(ChainId::new(1));
        assert_eq!(max_block_bytes(&g), MAX_BLOCK_BYTES);
        assert_eq!(
            g.params.registry.get(ParamId::MaxBlockBytes),
            Some(u64::from(MAX_BLOCK_BYTES))
        );
        assert_eq!(max_gas(&g), MAX_GAS);
    }

    #[test]
    fn empty_block_passes_precheck_without_apply() {
        let (g, b) = empty_ok();
        precheck_block(&g, &b).unwrap();
    }

    #[test]
    fn over_gas_block_rejected_before_apply() {
        let mut g = Genesis::new(ChainId::new(1));
        g.params.registry.set(ParamId::MaxGas, GAS_TRANSFER);
        let clock = TestClock::new(1_000);
        let fields = HeaderFields::new(
            &clock,
            Height::GENESIS,
            Round::ZERO,
            ValidatorId::ZERO,
            0,
            1,
        )
        .unwrap();
        let tx = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            Address::ZERO,
            Amount::new(1),
        );
        let signed = SignedTx {
            tx,
            signature: [1u8; 64],
            public_key: [2u8; 32],
        };
        let b = Block {
            header_fields: fields,
            txs: vec![signed; 3],
        };
        assert_eq!(precheck_block(&g, &b), Err(LimitError::BlockGas));
    }

    #[test]
    fn oversized_block_rejected_before_apply_block() {
        let mut g = Genesis::new(ChainId::new(1));
        g.params.registry.set(ParamId::MaxBlockBytes, 200);
        let clock = TestClock::new(1_000);
        let fields = HeaderFields::new(
            &clock,
            Height::GENESIS,
            Round::ZERO,
            ValidatorId::ZERO,
            0,
            1,
        )
        .unwrap();
        let tx = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            Address::ZERO,
            Amount::new(1),
        );
        let signed = SignedTx {
            tx,
            signature: [1u8; 64],
            public_key: [2u8; 32],
        };
        let b = Block {
            header_fields: fields,
            txs: vec![signed; 8],
        };
        assert_eq!(precheck_block(&g, &b), Err(LimitError::BlockBytes));
    }

    #[test]
    fn state_growth_cap_uses_commit_root() {
        let g = Genesis::new(ChainId::new(1));
        let a = AccountTrie::new();
        let s = ContractStorageTrie::new();
        precheck_state_growth(&g, &a, &a, &s, 10).unwrap();
        assert_eq!(
            precheck_state_growth(&g, &a, &a, &s, max_state_delta_bytes(&g) + 1),
            Err(LimitError::StateGrowth)
        );
        let _ = Hash::from_bytes(commit_tries(&a, &s));
    }

    #[test]
    fn eip1559_off_matches_min_fee_floor() {
        let g = Genesis::new(ChainId::new(1));
        let floor = mempool::fees::min_fee_floor(&g.params.registry);
        let mg = max_gas(&g);
        assert_eq!(
            execution::fees::next_base_fee(floor, mg, mg, false, floor),
            floor
        );
        assert_ne!(
            execution::fees::next_base_fee(floor, mg, mg, true, floor),
            execution::fees::next_base_fee(floor, 0, mg, true, floor)
        );
    }
}
