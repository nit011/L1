//! Hot-account vs low-contention throughput (architecture.md §3.5, §4.4).
//!
//! Under a single hot account, Block-STM is expected to approach sequential
//! speed because those transactions conflict. That is not a regression.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use crypto::address::from_ed25519;
use crypto::sig::ed25519::SecretKey;
use crypto::tx::sign;
use execution::seq::{self, World};
use execution::stm;
use types::genesis::{Genesis, GenesisAccount};
use types::header::HeaderFields;
use types::tx::{SignedTx, Tx};
use types::{
    Address, Amount, ChainId, Hash, Height, Nonce, Round, TestClock, ValidatorId, GAS_TRANSFER,
};

fn fields() -> HeaderFields {
    let clock = TestClock::new(1_000);
    HeaderFields::new(
        &clock,
        Height::GENESIS,
        Round::ZERO,
        ValidatorId::ZERO,
        0,
        1,
    )
    .unwrap()
}

fn low_contention_block(n: usize) -> (World, types::block::Block) {
    let mut g = Genesis::new(ChainId::new(1));
    let mut txs: Vec<SignedTx> = Vec::new();
    for i in 0..n {
        let mut seed = [0u8; 32];
        seed[0] = ((i / 256) % 254 + 1) as u8;
        seed[1] = (i % 256) as u8;
        seed[2] = 7;
        let k = SecretKey::from_bytes(&seed);
        let a = from_ed25519(&k.verifying_key());
        g.insert_alloc(
            a,
            GenesisAccount {
                balance: Amount::new(1_000_000),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        let to = Address::from_bytes({
            let mut d = [0u8; 32];
            d[0] = 0xee;
            d[1] = seed[1];
            d
        });
        txs.push(sign(
            &k,
            Tx::transfer(
                ChainId::new(1),
                Nonce::ZERO,
                GAS_TRANSFER,
                Amount::new(1),
                to,
                Amount::new(1),
            ),
        ));
    }
    (
        World::from_genesis(&g),
        types::block::Block {
            header_fields: fields(),
            txs,
        },
    )
}

fn hot_account_block(n: usize) -> (World, types::block::Block) {
    let k = SecretKey::from_bytes(&[11u8; 32]);
    let a = from_ed25519(&k.verifying_key());
    let dest = Address::from_bytes([0xaa; 32]);
    let mut g = Genesis::new(ChainId::new(1));
    g.insert_alloc(
        a,
        GenesisAccount {
            balance: Amount::new(10_000_000),
            nonce: Nonce::ZERO,
            code_hash: Hash::ZERO,
        },
    );
    let mut txs = Vec::new();
    for i in 0..n {
        txs.push(sign(
            &k,
            Tx::transfer(
                ChainId::new(1),
                Nonce(i as u64),
                GAS_TRANSFER,
                Amount::new(1),
                dest,
                Amount::new(1),
            ),
        ));
    }
    (
        World::from_genesis(&g),
        types::block::Block {
            header_fields: fields(),
            txs,
        },
    )
}

fn bench_stm(c: &mut Criterion) {
    const N: usize = 64;
    let (w_low, b_low) = low_contention_block(N);
    let (w_hot, b_hot) = hot_account_block(N);

    c.bench_function("seq_low_contention", |bencher| {
        bencher.iter(|| {
            seq::apply_block(black_box(w_low.clone()), black_box(&b_low));
        });
    });
    c.bench_function("stm_low_contention", |bencher| {
        bencher.iter(|| {
            stm::apply_block_engine(black_box(w_low.clone()), black_box(&b_low));
        });
    });
    c.bench_function("seq_hot_account", |bencher| {
        bencher.iter(|| {
            seq::apply_block(black_box(w_hot.clone()), black_box(&b_hot));
        });
    });
    c.bench_function("stm_hot_account", |bencher| {
        bencher.iter(|| {
            stm::apply_block_engine(black_box(w_hot.clone()), black_box(&b_hot));
        });
    });
}

criterion_group!(benches, bench_stm);
criterion_main!(benches);
