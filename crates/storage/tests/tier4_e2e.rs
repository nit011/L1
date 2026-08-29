//! End-to-end: mempool → `block.builder.local` → store → replay (Tier 3/4 exit bar).

use crypto::address::from_ed25519;
use crypto::sig::ed25519::SecretKey;
use crypto::tx::sign;
use execution::builder::build_local;
use execution::seq::{apply_block, World};
use mempool::Mempool;
use storage::blocks::{put_block, put_genesis_hash};
use storage::memory::MemoryStore;
use storage::replay::replay_from_genesis;
use types::genesis::{Genesis, GenesisAccount};
use types::header::HeaderFields;
use types::tx::Tx;
use types::{
    Address, Amount, ChainId, Hash, Height, Nonce, ParamsRegistry, Round, TestClock, ValidatorId,
    GAS_TRANSFER,
};

fn sk(b: u8) -> SecretKey {
    SecretKey::from_bytes(&[b; 32])
}

#[test]
fn build_store_restart_replay_roots_match() {
    let ska = sk(1);
    let skb = sk(2);
    let a = from_ed25519(&ska.verifying_key());
    let b = from_ed25519(&skb.verifying_key());
    let mut genesis = Genesis::new(ChainId::new(1));
    genesis.insert_alloc(
        a,
        GenesisAccount {
            balance: Amount::new(1_000_000),
            nonce: Nonce::ZERO,
            code_hash: Hash::ZERO,
        },
    );
    genesis.insert_alloc(
        b,
        GenesisAccount {
            balance: Amount::new(50_000),
            nonce: Nonce::ZERO,
            code_hash: Hash::ZERO,
        },
    );

    let mut store = MemoryStore::new();
    put_genesis_hash(&mut store, &genesis).unwrap();

    let mut world = World::from_genesis(&genesis);
    let mut pool = Mempool::new(&ParamsRegistry::new());
    let clock = TestClock::new(1_000_000);
    let mut last_app = Hash::ZERO;
    let mut live_state = world.commit_state_root();

    for height in 0..3u64 {
        let to = if height % 2 == 0 { b } else { a };
        let from_sk = if height % 2 == 0 { &ska } else { &skb };
        let from = if height % 2 == 0 { a } else { b };
        let acct = world.account(&from);
        let tx = Tx::transfer(
            ChainId::new(1),
            acct.nonce,
            GAS_TRANSFER,
            Amount::new(2 + height as u128),
            to,
            Amount::new(10 + height as u128),
        );
        pool.insert(sign(from_sk, tx), &acct).unwrap();

        let fields = HeaderFields::new(
            &clock,
            Height(height),
            Round::ZERO,
            ValidatorId::ZERO,
            0,
            1_000 + height,
        )
        .unwrap();
        let built = build_local(&mut pool, &genesis, world, fields);
        assert!(!built.block.txs.is_empty());
        let recs: Vec<Vec<u8>> = built.receipts.iter().map(|r| r.encode()).collect();
        put_block(
            &mut store,
            &built.header,
            &built.block,
            &recs,
            &built.app_hash,
        )
        .unwrap();
        last_app = built.app_hash;
        world = built.world;
        live_state = world.commit_state_root();
    }

    let live_app_hex = hex::encode(last_app.as_bytes());
    let live_state_hex = hex::encode(live_state.as_bytes());

    let wiped = World::from_genesis(&genesis);
    let (replayed, replay_app) = replay_from_genesis(&store, &genesis, wiped, apply_block).unwrap();
    let replay_state = replayed.commit_state_root();
    let replay_app_hex = hex::encode(replay_app.as_bytes());
    let replay_state_hex = hex::encode(replay_state.as_bytes());

    println!("TIER4_E2E live_app={live_app_hex} live_state={live_state_hex}");
    println!("TIER4_E2E replay_app={replay_app_hex} replay_state={replay_state_hex}");

    assert_eq!(
        (live_app_hex.as_str(), live_state_hex.as_str()),
        (replay_app_hex.as_str(), replay_state_hex.as_str()),
        "live app={live_app_hex} state={live_state_hex} replay app={replay_app_hex} state={replay_state_hex}"
    );
}

#[test]
fn wal_crash_mid_commit_then_recover() {
    use storage::wal::{commit_with_wal, recover};

    let ska = sk(1);
    let a = from_ed25519(&ska.verifying_key());
    let mut genesis = Genesis::new(ChainId::new(1));
    genesis.insert_alloc(
        a,
        GenesisAccount {
            balance: Amount::new(1_000_000),
            nonce: Nonce::ZERO,
            code_hash: Hash::ZERO,
        },
    );
    let mut store = MemoryStore::new();
    put_genesis_hash(&mut store, &genesis).unwrap();
    let world = World::from_genesis(&genesis);
    let mut pool = Mempool::new(&ParamsRegistry::new());
    let acct = world.account(&a);
    let tx = Tx::transfer(
        ChainId::new(1),
        Nonce::ZERO,
        GAS_TRANSFER,
        Amount::new(1),
        Address::from_bytes([9u8; 32]),
        Amount::new(7),
    );
    pool.insert(sign(&ska, tx), &acct).unwrap();
    let clock = TestClock::new(1_000_000);
    let fields = HeaderFields::new(
        &clock,
        Height::GENESIS,
        Round::ZERO,
        ValidatorId::ZERO,
        0,
        1,
    )
    .unwrap();
    let built = build_local(&mut pool, &genesis, world, fields);
    let recs: Vec<Vec<u8>> = built.receipts.iter().map(|r| r.encode()).collect();
    commit_with_wal(
        &mut store,
        &built.header,
        &built.block,
        &recs,
        &built.app_hash,
        true,
    )
    .unwrap();
    assert!(storage::blocks::get_block(&store, Height::GENESIS)
        .unwrap()
        .is_none());

    let recovered = recover(
        &mut store,
        &genesis,
        World::from_genesis(&genesis),
        apply_block,
    )
    .unwrap();
    assert_eq!(
        storage::blocks::get_app_hash(&store, Height::GENESIS).unwrap(),
        Some(built.app_hash)
    );
    assert_eq!(recovered.commit_state_root(), built.header.state_root);
}
