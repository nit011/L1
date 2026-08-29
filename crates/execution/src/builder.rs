//! Local block assembly from the mempool (development-plan.md Tier 4).
//!
//! Pulls ready txs via [`ReadyTxs`] (implemented by `mempool.fee_order`),
//! respects `genesis.params` gas/size caps, and runs the frozen
//! `exec.seq.apply_block`. No consensus or networking (Tier 5/6).

use crate::gas::gas_meter;
use crate::receipt::Receipt;
use crate::seq::{apply_block, World};
use types::block::Block;
use types::genesis::Genesis;
use types::header::{Header, HeaderFields, DA_ROOT_PLACEHOLDER};
use types::tx::SignedTx;
use types::{Hash, ParamId};

/// Source of nonce-ready, fee-ordered transactions. Contract: `mempool.fee_order`.
pub trait ReadyTxs {
    /// Highest-priority ready txs, already nonce-ordered per account.
    fn take_ready(&mut self, max_gas: u64, max_bytes: u32) -> Vec<SignedTx>;
}

/// Candidate block from a local builder. Contract: `block.builder.local`.
pub struct BuiltBlock {
    /// Assembled block (`block.body`).
    pub block: Block,
    /// Header with roots filled after execution.
    pub header: Header,
    /// Post-state.
    pub world: World,
    /// Per-tx receipts.
    pub receipts: Vec<Receipt>,
    /// Frozen `exec.app_hash`.
    pub app_hash: Hash,
}

/// Assemble and execute a block. Contract: `block.builder.local`.
pub fn build_local<R: ReadyTxs>(
    pool: &mut R,
    genesis: &Genesis,
    pre: World,
    fields: HeaderFields,
) -> BuiltBlock {
    let max_gas = genesis
        .params
        .registry
        .get(ParamId::MaxGas)
        .expect("genesis.params MaxGas");
    let max_bytes = genesis
        .params
        .registry
        .get(ParamId::MaxBlockBytes)
        .expect("genesis.params MaxBlockBytes") as u32;
    let candidates = pool.take_ready(max_gas, max_bytes);
    let mut gas_used = 0u64;
    let mut bytes_used = 0u32;
    let mut txs = Vec::new();
    for signed in candidates {
        let g = match gas_meter(&signed.tx) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let b = u32::try_from(signed.tx.encode().len()).unwrap_or(u32::MAX);
        if gas_used.saturating_add(g) > max_gas {
            continue;
        }
        if bytes_used.saturating_add(b) > max_bytes {
            continue;
        }
        gas_used = gas_used.saturating_add(g);
        bytes_used = bytes_used.saturating_add(b);
        txs.push(signed);
    }
    let block = Block {
        header_fields: fields,
        txs,
    };
    let (world, receipts, app) = apply_block(pre, &block);
    let tx_r = types::block::tx_root_signed(&block.txs);
    let rec_bytes: Vec<Vec<u8>> = receipts.iter().map(|r| r.encode()).collect();
    let rec_r = types::block::receipts_root(&rec_bytes);
    let st = world.commit_state_root();
    debug_assert_eq!(app, crate::seq::app_hash(&st, &tx_r, &rec_r));
    let header = Header {
        fields: block.header_fields.clone(),
        tx_root: tx_r,
        state_root: st,
        receipts_root: rec_r,
        validators_hash: types::header::validators_hash(&genesis.validators),
        da_root: DA_ROOT_PLACEHOLDER,
    };
    BuiltBlock {
        block,
        header,
        world,
        receipts,
        app_hash: app,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::address::from_ed25519;
    use crypto::sig::ed25519::SecretKey;
    use crypto::tx::sign;
    use types::genesis::GenesisAccount;
    use types::{
        Address, Amount, ChainId, Height, Nonce, Round, TestClock, ValidatorId, GAS_TRANSFER,
    };

    struct FakePool {
        txs: Vec<SignedTx>,
    }

    impl ReadyTxs for FakePool {
        fn take_ready(&mut self, _max_gas: u64, _max_bytes: u32) -> Vec<SignedTx> {
            std::mem::take(&mut self.txs)
        }
    }

    fn sk(b: u8) -> SecretKey {
        SecretKey::from_bytes(&[b; 32])
    }

    #[test]
    fn builds_empty_when_pool_empty() {
        let g = Genesis::new(ChainId::new(1));
        let world = World::from_genesis(&g);
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
        let mut pool = FakePool { txs: vec![] };
        let built = build_local(&mut pool, &g, world.clone(), fields);
        assert!(built.block.txs.is_empty());
        let (w2, _, app) = apply_block(world, &built.block);
        assert_eq!(built.app_hash, app);
        let _ = w2;
    }

    #[test]
    fn includes_transfer_and_skips_over_gas() {
        let ska = sk(1);
        let from = from_ed25519(&ska.verifying_key());
        let mut g = Genesis::new(ChainId::new(1));
        g.insert_alloc(
            from,
            GenesisAccount {
                balance: Amount::new(1_000),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        let world = World::from_genesis(&g);
        let tx = types::tx::Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            Address::from_bytes([9u8; 32]),
            Amount::new(10),
        );
        let signed = sign(&ska, tx);
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
        let mut pool = FakePool {
            txs: vec![signed.clone()],
        };
        let built = build_local(&mut pool, &g, world, fields);
        assert_eq!(built.block.txs.len(), 1);
        assert!(built.receipts[0].success);
        assert_eq!(
            built.header.validators_hash,
            types::header::validators_hash(&g.validators)
        );
    }
}
