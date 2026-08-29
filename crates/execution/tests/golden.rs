//! Golden vectors for `exec.app_hash` / `genesis.hash`.
//!
//! Expected hex strings are literals — not computed in this test and
//! compared to themselves.

use consensus::timeout::{TimeoutConfig, TimeoutStep};
use crypto::address::from_ed25519;
use crypto::sig::ed25519::SecretKey;
use crypto::tx::sign;
use execution::seq::{apply_block, apply_block_with_roots, World};
use types::genesis::{Genesis, GenesisAccount};
use types::header::HeaderFields;
use types::tx::Tx;
use types::{
    Address, Amount, ChainId, Hash, Height, Nonce, Round, TestClock, ValidatorId, GAS_TRANSFER,
};

fn sk(b: u8) -> SecretKey {
    SecretKey::from_bytes(&[b; 32])
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
    let cfg = TimeoutConfig::from_spec();
    assert_eq!(
        g.params.timeouts.propose_ms,
        cfg.duration_ms(TimeoutStep::Propose, Round::ZERO)
    );
    (g, a, b)
}

fn empty_block() -> types::block::Block {
    let clock = TestClock::new(1_700_000_000_000);
    let fields = HeaderFields::new(
        &clock,
        Height::GENESIS,
        Round::ZERO,
        ValidatorId::from_bytes([7u8; 48]),
        0,
        1_700_000_000_000,
    )
    .unwrap();
    types::block::Block {
        header_fields: fields,
        txs: vec![],
    }
}

fn hex32(h: &Hash) -> String {
    hex::encode(h.as_bytes())
}

#[test]
fn genesis_hash_literal() {
    let (g, _, _) = fixture_genesis();
    const EXPECT: &str = "3070e230ec9bd58862fe78b43774f85879d2db270f8e5f28facb4637bae5f1b1";
    let got = hex32(&g.hash());
    assert_eq!(got, EXPECT, "update EXPECT to {got}");
}

#[test]
fn empty_block_app_hash() {
    let (g, _, _) = fixture_genesis();
    let world = World::from_genesis(&g);
    let block = empty_block();
    let (_, _, app, st, tx, rec) = apply_block_with_roots(world, &block);
    const APP: &str = "43dca346e5485849e16be9da4fc13d10c43e3b7701df51e7c273d1b4ed3cf6ad";
    assert_eq!(hex32(&app), APP);
    let _ = (st, tx, rec);
}

#[test]
fn single_transfer_app_hash() {
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
    let signed = sign(&ska, tx);
    let mut block = empty_block();
    block.txs = vec![signed];
    let (_, recs, app) = apply_block(world, &block);
    assert!(recs[0].success);
    const APP: &str = "2898208706d1893606f1f79959653189f3d2163f528b849430fd569db238515f";
    assert_eq!(hex32(&app), APP);
}

#[test]
fn rejected_nonce_app_hash() {
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
    let signed = sign(&ska, tx);
    let mut block = empty_block();
    block.txs = vec![signed];
    let (_, recs, app) = apply_block(world, &block);
    assert!(!recs[0].success);
    const APP: &str = "3ad011d6221bb5f627aafc815a8ee8352caf8efd8b30352b08a67877b2c09f62";
    assert_eq!(hex32(&app), APP);
}

#[test]
fn multi_account_app_hash() {
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
    let mut block = empty_block();
    block.txs = vec![sign(&ska, tx1), sign(&skb, tx2)];
    let (_, recs, app) = apply_block(world, &block);
    assert!(recs[0].success && recs[1].success);
    const APP: &str = "c70d2307dcc8448eaa9a18ce3d1bc372036ba97c0e93f3a9c1c4e7f9b5164909";
    assert_eq!(hex32(&app), APP);
}

#[test]
fn app_hash_stable_twice_in_process() {
    let (g, _, _) = fixture_genesis();
    let world = World::from_genesis(&g);
    let block = empty_block();
    let h1 = apply_block(world.clone(), &block).2;
    let h2 = apply_block(world, &block).2;
    assert_eq!(h1, h2);
}
