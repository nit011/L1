//! `stm.equals_seq`: STM output byte-identical to `exec.seq.apply_block`
//! and to frozen `exec.golden_vectors`.

use crypto::address::from_ed25519;
use crypto::sig::ed25519::SecretKey;
use crypto::tx::sign;
use execution::seq::{self, World};
use execution::stm;
use proptest::prelude::*;
use proptest::test_runner::{RngSeed, TestRunner};
use types::genesis::{Genesis, GenesisAccount};
use types::header::HeaderFields;
use types::tx::{SignedTx, Tx};
use types::{
    Address, Amount, ChainId, Hash, Height, Nonce, Round, TestClock, ValidatorId, GAS_CALL,
    GAS_DEPLOY, GAS_TRANSFER,
};

fn sk(b: u8) -> SecretKey {
    SecretKey::from_bytes(&[b; 32])
}

fn fields() -> HeaderFields {
    let clock = TestClock::new(1_700_000_000_000);
    HeaderFields::new(
        &clock,
        Height::GENESIS,
        Round::ZERO,
        ValidatorId::from_bytes([7u8; 48]),
        0,
        1_700_000_000_000,
    )
    .unwrap()
}

fn fixture_genesis() -> (Genesis, Address, Address) {
    let ska = sk(1);
    let skb = sk(2);
    let a = from_ed25519(&ska.verifying_key());
    let b = from_ed25519(&skb.verifying_key());
    let mut g = Genesis::new(ChainId::new(1));
    g.insert_alloc(
        a,
        GenesisAccount {
            balance: Amount::new(1_000_000),
            nonce: Nonce::ZERO,
            code_hash: Hash::ZERO,
        },
    );
    g.insert_alloc(
        b,
        GenesisAccount {
            balance: Amount::new(50_000),
            nonce: Nonce::ZERO,
            code_hash: Hash::ZERO,
        },
    );
    g.insert_validator(ValidatorId::from_bytes([7u8; 48]), types::VotingPower(10));
    (g, a, b)
}

fn hex32(h: &Hash) -> String {
    hex::encode(h.as_bytes())
}

fn assert_eq_seq_stm(pre: World, block: &types::block::Block) {
    let seq = seq::apply_block(pre.clone(), block);
    let par = stm::apply_block_engine(pre, block);
    assert_eq!(
        par.1.iter().map(|r| r.encode()).collect::<Vec<_>>(),
        seq.1.iter().map(|r| r.encode()).collect::<Vec<_>>(),
        "receipt encodings"
    );
    assert_eq!(par.2, seq.2, "app_hash");
    assert_eq!(
        par.0.commit_state_root(),
        seq.0.commit_state_root(),
        "state_root"
    );
}

#[test]
fn golden_empty_block() {
    let (g, _, _) = fixture_genesis();
    let world = World::from_genesis(&g);
    let block = types::block::Block {
        header_fields: fields(),
        txs: vec![],
    };
    let (_, _, app) = stm::apply_block(world, &block);
    const APP: &str = "43dca346e5485849e16be9da4fc13d10c43e3b7701df51e7c273d1b4ed3cf6ad";
    assert_eq!(hex32(&app), APP);
}

#[test]
fn golden_single_transfer() {
    let (g, _a, baddr) = fixture_genesis();
    let ska = sk(1);
    let world = World::from_genesis(&g);
    let tx = Tx::transfer(
        ChainId::new(1),
        Nonce::ZERO,
        GAS_TRANSFER,
        Amount::new(10),
        baddr,
        Amount::new(100),
    );
    let mut block = types::block::Block {
        header_fields: fields(),
        txs: vec![],
    };
    block.txs = vec![sign(&ska, tx)];
    let (_, recs, app) = stm::apply_block(world, &block);
    assert!(recs[0].success);
    const APP: &str = "2898208706d1893606f1f79959653189f3d2163f528b849430fd569db238515f";
    assert_eq!(hex32(&app), APP);
}

#[test]
fn golden_rejected_nonce() {
    let (g, _a, baddr) = fixture_genesis();
    let ska = sk(1);
    let world = World::from_genesis(&g);
    let tx = Tx::transfer(
        ChainId::new(1),
        Nonce(9),
        GAS_TRANSFER,
        Amount::new(10),
        baddr,
        Amount::new(1),
    );
    let block = types::block::Block {
        header_fields: fields(),
        txs: vec![sign(&ska, tx)],
    };
    let (_, recs, app) = stm::apply_block(world, &block);
    assert!(!recs[0].success);
    const APP: &str = "3ad011d6221bb5f627aafc815a8ee8352caf8efd8b30352b08a67877b2c09f62";
    assert_eq!(hex32(&app), APP);
}

#[test]
fn golden_multi_account() {
    let (g, _a, baddr) = fixture_genesis();
    let ska = sk(1);
    let skb = sk(2);
    let world = World::from_genesis(&g);
    let tx1 = Tx::transfer(
        ChainId::new(1),
        Nonce::ZERO,
        GAS_TRANSFER,
        Amount::new(10),
        baddr,
        Amount::new(100),
    );
    let a_addr = from_ed25519(&ska.verifying_key());
    let tx2 = Tx::transfer(
        ChainId::new(1),
        Nonce::ZERO,
        GAS_TRANSFER,
        Amount::new(5),
        a_addr,
        Amount::new(20),
    );
    let block = types::block::Block {
        header_fields: fields(),
        txs: vec![sign(&ska, tx1), sign(&skb, tx2)],
    };
    let (_, recs, app) = stm::apply_block(world, &block);
    assert!(recs[0].success && recs[1].success);
    const APP: &str = "c70d2307dcc8448eaa9a18ce3d1bc372036ba97c0e93f3a9c1c4e7f9b5164909";
    assert_eq!(hex32(&app), APP);
}

fn funded_genesis(n_acct: u8) -> (Genesis, Vec<SecretKey>, Vec<Address>) {
    let mut g = Genesis::new(ChainId::new(1));
    let mut keys = Vec::new();
    let mut addrs = Vec::new();
    for i in 1..=n_acct {
        let k = sk(i);
        let a = from_ed25519(&k.verifying_key());
        g.insert_alloc(
            a,
            GenesisAccount {
                balance: Amount::new(1_000_000),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        keys.push(k);
        addrs.push(a);
    }
    (g, keys, addrs)
}

/// 256 cases per seed × 3 seeds = 768 property trials (see audit).
fn run_prop_seed(seed: u64) {
    let mut config = ProptestConfig::with_cases(256);
    config.rng_seed = RngSeed::Fixed(seed);
    config.failure_persistence = None;
    let mut runner = TestRunner::new(config);
    runner
        .run(
            &(2u8..9u8, 1usize..17usize, 0u8..3u8),
            |(n_acct, n_tx, mode)| {
                let (g, keys, addrs) = funded_genesis(n_acct);
                let pre = World::from_genesis(&g);
                let mut nonces = vec![0u64; keys.len()];
                let mut txs: Vec<SignedTx> = Vec::new();
                for t in 0..n_tx {
                    let from = if mode == 0 { 0usize } else { t % keys.len() };
                    let to_i = (from + 1 + t) % addrs.len();
                    let tx = Tx::transfer(
                        ChainId::new(1),
                        Nonce(nonces[from]),
                        GAS_TRANSFER,
                        Amount::new(1),
                        addrs[to_i],
                        Amount::new(1),
                    );
                    nonces[from] += 1;
                    txs.push(sign(&keys[from], tx));
                }
                let block = types::block::Block {
                    header_fields: fields(),
                    txs,
                };
                assert_eq_seq_stm(pre, &block);
                Ok(())
            },
        )
        .unwrap();
}

#[test]
fn property_stm_equals_seq_three_seeds() {
    for seed in [1u64, 2, 99] {
        run_prop_seed(seed);
    }
}

fn counter_wasm() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (import "host" "sload" (func $sload (param i32) (result i64)))
            (import "host" "sstore" (func $sstore (param i32 i64)))
            (import "host" "reenter" (func $reenter))
            (func (export "call")
                (local $v i64)
                (local.set $v (i64.add (call $sload (i32.const 0)) (i64.const 1)))
                (call $sstore (i32.const 0) (local.get $v))
            )
        )"#,
    )
    .unwrap()
}

/// Mix of transfers, staking, and WASM deploy/call (overlapping storage).
/// 256 cases × 3 seeds = 768 additional property trials.
fn run_prop_wasm_seed(seed: u64) {
    let mut config = ProptestConfig::with_cases(256);
    config.rng_seed = RngSeed::Fixed(seed);
    config.failure_persistence = None;
    let mut runner = TestRunner::new(config);
    let code = counter_wasm();
    runner
        .run(
            &(2u8..6u8, 3usize..12usize, 0u8..4u8),
            |(n_acct, n_tx, mode)| {
                let (mut g, keys, addrs) = funded_genesis(n_acct);
                let vid = ValidatorId::from_bytes([4u8; 48]);
                g.insert_validator(vid, types::VotingPower(1));
                let pre = World::from_genesis(&g);
                let mut nonces = vec![0u64; keys.len()];
                let mut txs: Vec<SignedTx> = Vec::new();
                let deploy = Tx::deploy(
                    ChainId::new(1),
                    Nonce(nonces[0]),
                    GAS_DEPLOY,
                    Amount::new(1),
                    code.clone(),
                );
                nonces[0] += 1;
                txs.push(sign(&keys[0], deploy));
                let contract = execution::wasm::deploy::create_address(&addrs[0], Nonce::ZERO);
                for t in 1..n_tx {
                    let from = if mode == 0 { 0usize } else { t % keys.len() };
                    let kind = (t + mode as usize) % 3;
                    let tx = if kind == 0 {
                        Tx::call(
                            ChainId::new(1),
                            Nonce(nonces[from]),
                            GAS_CALL + 80_000,
                            Amount::new(1),
                            contract,
                            vec![],
                        )
                    } else if kind == 1 {
                        let to_i = (from + 1 + t) % addrs.len();
                        Tx::transfer(
                            ChainId::new(1),
                            Nonce(nonces[from]),
                            GAS_TRANSFER,
                            Amount::new(1),
                            addrs[to_i],
                            Amount::new(1),
                        )
                    } else {
                        Tx::stake_bond(
                            ChainId::new(1),
                            Nonce(nonces[from]),
                            GAS_TRANSFER,
                            Amount::new(1),
                            vid,
                            Amount::new(200),
                        )
                    };
                    nonces[from] += 1;
                    txs.push(sign(&keys[from], tx));
                }
                let block = types::block::Block {
                    header_fields: fields(),
                    txs,
                };
                assert_eq_seq_stm(pre, &block);
                Ok(())
            },
        )
        .unwrap();
}

#[test]
fn property_stm_equals_seq_wasm_three_seeds() {
    for seed in [1u64, 2, 99] {
        run_prop_wasm_seed(seed);
    }
}
