//! Capstone: fund via faucet, wait for finality, confirm balance on a **live** RPC server.
//!
//! Contract: `sdk.e2e_integration_test`.
//!
//! Spins an Axum JSON-RPC server (`rpc.server`) over a real TCP port and a
//! four-validator in-process BFT round (`node.wire.*` / `cons.commit`) — the
//! same stack as a local devnet, not a mocked RPC result map.

use consensus::propose::round_vrf_source;
use consensus::vrf as cons_vrf;
use crypto::address::from_ed25519;
use crypto::from_bls;
use crypto::sig::bls;
use crypto::sig::ed25519::keygen;
use crypto::vrf::public_key_from_seed;
use faucet::service::Faucet;
use mempool::Mempool;
use network::discovery::BootstrapList;
use network::identity;
use node::config::NodeConfig;
use node::wire::{init_store, wire_commit, wire_precommit, wire_propose, wire_vote, TraceSink};
use rpc::server::{encode_hex, RpcInner, RpcServer};
use rpc::status::observe_finalized;
use sdk::finality::wait_status_http;
use sdk::sign::SignedFrom;
use sdk::submit::rpc_call;
use serde_json::json;
use std::time::{Duration, Instant};
use types::collections::Map;
use types::genesis::{Genesis, GenesisAccount};
use types::{
    Address, Amount, ChainId, Epoch, Height, Nonce, Round, TestClock, ValidatorId, VotingPower,
};

type Val = (blst::min_pk::SecretKey, ValidatorId, [u8; 32], [u8; 32]);

fn produce(inner: &mut RpcInner, keys: &[Val], vrf_pks: &Map<ValidatorId, [u8; 32]>) {
    let src = round_vrf_source(&inner.cfg.genesis.validators, Round::ZERO).unwrap();
    let src_row = keys.iter().find(|k| k.1 == src).unwrap();
    let seed = cons_vrf::derive_seed(&[1u8; 32], Epoch::ZERO);
    let (_, proof) = cons_vrf::leader_prove(&src_row.2, &seed, &src).unwrap();
    let winner = cons_vrf::weighted_leader(
        &src_row.3,
        &seed,
        &src,
        &proof,
        &inner.cfg.genesis.validators,
    )
    .unwrap();
    let w = keys.iter().find(|k| k.1 == winner).unwrap();
    let clock = TestClock::new(1_000);
    let mut sink = TraceSink::default();
    let mesh = network::validator_mesh::ValidatorMesh::from_genesis(&inner.cfg.genesis);
    let mut pool = std::mem::replace(
        &mut inner.pool,
        Mempool::new(&inner.cfg.genesis.params.registry),
    );
    let (proposal, built) = wire_propose(
        &inner.cfg,
        &w.0,
        winner,
        src,
        &vrf_pks.get(&src).copied().unwrap(),
        &proof,
        &seed,
        Height::GENESIS,
        Round::ZERO,
        &clock,
        0,
        inner.world.clone(),
        &mut pool,
        &mut sink,
    )
    .unwrap();
    inner.pool = pool;
    inner.world = built.world.clone();
    let mut log = consensus::vote::VoteReplayLog::new();
    let mut prevotes = Vec::new();
    for k in keys {
        prevotes.push(
            wire_vote(
                &mesh,
                &k.0,
                k.1,
                &proposal,
                vrf_pks,
                &seed,
                Height::GENESIS,
                Round::ZERO,
                None,
                &mut log,
                &mut sink,
            )
            .unwrap(),
        );
    }
    let mut precommits = Vec::new();
    for k in keys {
        let (pc, _) = wire_precommit(
            &mesh,
            &k.0,
            k.1,
            Height::GENESIS,
            Round::ZERO,
            &prevotes,
            &proposal.header,
            &mut log,
            &mut sink,
        )
        .unwrap();
        precommits.push(pc);
    }
    let rec: Vec<_> = built.receipts.iter().map(|r| r.encode()).collect();
    let f = wire_commit(
        &precommits,
        &inner.cfg.genesis.validators,
        VotingPower(4),
        &proposal,
        &mut inner.commits,
        &mut inner.store,
        &built.block,
        &rec,
        &mut sink,
    )
    .unwrap()
    .unwrap();
    observe_finalized(inner, f);
}

#[test]
fn e2e_fund_wait_finality_get_account_on_live_http() {
    let faucet_sk = keygen();
    let dest = from_ed25519(&keygen().verifying_key());
    let mut g = Genesis::new(ChainId::new(1));
    g.insert_alloc(
        from_ed25519(&faucet_sk.verifying_key()),
        GenesisAccount {
            balance: Amount::new(1_000_000),
            nonce: Nonce::ZERO,
            code_hash: types::Hash::ZERO,
        },
    );
    let mut keys = Vec::new();
    let mut vrf_pks = Map::new();
    for i in 0..4u8 {
        let sk = bls::keygen().unwrap();
        let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(1));
        g.insert_validator(id, VotingPower(1));
        let vrf_sk = [i + 1; 32];
        let vrf_pk = public_key_from_seed(&vrf_sk);
        vrf_pks.insert(id, vrf_pk);
        keys.push((sk, id, vrf_sk, vrf_pk));
    }
    let cfg = NodeConfig::new(
        g,
        BootstrapList::new(),
        identity::generate().unwrap(),
        std::path::PathBuf::from("/tmp/sdk-e2e"),
    );
    let srv = RpcServer::new(cfg);
    {
        let mut inner = srv.inner.lock().unwrap();
        let cfg = inner.cfg.clone();
        init_store(&mut inner.store, &cfg).unwrap();
    }

    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}/");
    let srv_http = srv.clone();
    rt.spawn(async move {
        axum::serve(listener, rpc::server::router(srv_http))
            .await
            .ok();
    });

    let mut faucet = Faucet::new(faucet_sk, ChainId::new(1), Amount::new(77));
    let from = faucet.address();
    let envelope = faucet.signed_transfer(dest, 0);
    let t_submit = Instant::now();
    let tx_hash = faucet.drip_http(&url, dest).expect("faucet http submit");
    {
        let mut inner = srv.inner.lock().unwrap();
        produce(&mut inner, &keys, &vrf_pks);
    }
    let rec = wait_status_http(
        &url,
        SignedFrom {
            signed: envelope,
            from,
        },
        tx_hash,
        Duration::from_secs(10),
    )
    .expect("finality");
    let waited = rec.waited;
    let acc = rpc_call(
        &url,
        "l1_getAccount",
        json!({"address": encode_hex(dest.as_bytes())}),
    )
    .expect("getAccount");
    assert_eq!(
        acc["balance"].as_str().unwrap(),
        "77",
        "independent l1_getAccount must show faucet amount"
    );
    let submit_to_finality_ms = t_submit.elapsed().as_millis();
    eprintln!(
        "sdk.e2e_integration_test REAL HTTP node: funding_tx={} height={} wait_status={:?} submit_to_account_ms={} dest={}",
        hex::encode(tx_hash.as_bytes()),
        rec.height,
        waited,
        submit_to_finality_ms,
        hex::encode(dest.as_bytes()),
    );
    let _ = Address::ZERO;
}
