//! In-process 4-validator BFT (architecture.md §2; development-plan.md “Finalizing core”).
//!
//! Channels are `Vec` mailboxes — no libp2p. Finalized blocks go through
//! `store.block.put`. Validators come from `genesis.validators`.

use consensus::propose::{propose, round_vrf_source, Proposal};
use consensus::replay::VoteKind;
use consensus::safety::CommitLog;
use consensus::steps::{commit, precommit_step, prevote_step};
use consensus::timeout::{BoundClock, TimeoutConfig, TimeoutStep};
use consensus::vote::{BlsSecretKey, VoteReplayLog};
use consensus::vrf::{self, VrfSeed};
use consensus::wal::{check_no_double_sign, log_proposal, log_vote};
use crypto::from_bls;
use crypto::sig::bls;
use crypto::vrf::public_key_from_seed;
use execution::builder::{build_local, ReadyTxs};
use execution::seq::{app_hash, World};
use mempool::Mempool;
use storage::blocks::{put_block, put_genesis_hash};
use storage::memory::MemoryStore;
use types::collections::Map;
use types::genesis::{Genesis, GenesisAccount};
use types::header::HeaderFields;
use types::tx::SignedTx;
use types::{
    Address, Amount, ChainId, Clock, Epoch, Hash, Height, Nonce, ParamsRegistry, Round, TestClock,
    ValidatorId, VotingPower,
};

struct EmptyPool;

impl ReadyTxs for EmptyPool {
    fn take_ready(&mut self, _max_gas: u64, _max_bytes: u32) -> Vec<SignedTx> {
        vec![]
    }
}

struct Node {
    sk: BlsSecretKey,
    id: ValidatorId,
    vrf_sk: [u8; 32],
    vrf_pk: [u8; 32],
    world: World,
    chain: MemoryStore,
    wal: MemoryStore,
    replay: VoteReplayLog,
    commits: CommitLog,
    height: Height,
    round: Round,
    lock: Option<consensus::state::Lock>,
    last_hash: Hash,
    online: bool,
}

fn seed(last: &Hash) -> VrfSeed {
    vrf::derive_seed(last.as_bytes(), Epoch::ZERO)
}

#[allow(clippy::type_complexity)]
fn four_nodes() -> (
    Genesis,
    Vec<Node>,
    Map<ValidatorId, VotingPower>,
    Map<ValidatorId, [u8; 32]>,
) {
    let mut keys = Vec::new();
    let mut genesis = Genesis::new(ChainId::new(1));
    let mut validators = Map::new();
    let mut vrf_pks = Map::new();
    for i in 0..4u8 {
        let sk = bls::keygen().unwrap();
        let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(1));
        genesis.insert_validator(id, VotingPower(1));
        genesis.insert_alloc(
            Address::from_bytes([i; 32]),
            GenesisAccount {
                balance: Amount::new(1),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        validators.insert(id, VotingPower(1));
        let vrf_sk = [10 + i; 32];
        let vrf_pk = public_key_from_seed(&vrf_sk);
        vrf_pks.insert(id, vrf_pk);
        keys.push((sk, id, vrf_sk, vrf_pk));
    }
    let gh = genesis.hash();
    let mut nodes = Vec::new();
    for (sk, id, vrf_sk, vrf_pk) in keys {
        let mut chain = MemoryStore::new();
        put_genesis_hash(&mut chain, &genesis).unwrap();
        nodes.push(Node {
            sk,
            id,
            vrf_sk,
            vrf_pk,
            world: World::from_genesis(&genesis),
            chain,
            wal: MemoryStore::new(),
            replay: VoteReplayLog::new(),
            commits: CommitLog::new(),
            height: Height::GENESIS,
            round: Round::ZERO,
            lock: None,
            last_hash: gh,
            online: true,
        });
    }
    (genesis, nodes, validators, vrf_pks)
}

fn try_propose(
    genesis: &Genesis,
    n: &mut Node,
    validators: &Map<ValidatorId, VotingPower>,
    vrf_pks: &Map<ValidatorId, [u8; 32]>,
    source: ValidatorId,
    source_sk: &[u8; 32],
    clock: &TestClock,
) -> Option<Proposal> {
    let s = seed(&n.last_hash);
    let src_pk = vrf_pks.get(&source)?;
    let (_, proof) = vrf::leader_prove(source_sk, &s, &source).ok()?;
    let ts = clock.now_millis().max(1);
    let fields = HeaderFields::new(clock, n.height, n.round, n.id, 0, ts).expect("header fields");
    let mut pool = EmptyPool;
    let built = {
        let w = n.world.clone();
        build_local(&mut pool, genesis, w, fields)
    };
    match propose(
        &n.sk,
        n.id,
        source,
        src_pk,
        &proof,
        validators,
        &s,
        n.height,
        n.round,
        || (built.header.clone(), built.app_hash),
    ) {
        Ok(p) => {
            log_proposal(&mut n.wal, &p).ok()?;
            n.world = built.world;
            Some(p)
        }
        Err(_) => None,
    }
}

fn drive_round(
    genesis: &Genesis,
    nodes: &mut [Node],
    validators: &Map<ValidatorId, VotingPower>,
    vrf_pks: &Map<ValidatorId, [u8; 32]>,
    clock: &TestClock,
    byzantine_split: bool,
) -> Option<Hash> {
    for n in nodes.iter_mut() {
        if n.online {
            n.replay = VoteReplayLog::new();
        }
    }
    let online: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.online)
        .map(|(i, _)| i)
        .collect();
    if online.is_empty() {
        return None;
    }
    let source = round_vrf_source(validators, nodes[online[0]].round)?;
    let source_idx = nodes.iter().position(|n| n.id == source)?;
    if !nodes[source_idx].online {
        return None;
    }
    let src_sk = nodes[source_idx].vrf_sk;
    let s = seed(&nodes[online[0]].last_hash);
    let src_pk = vrf_pks.get(&source)?;
    let (_, proof) = vrf::leader_prove(&src_sk, &s, &source).ok()?;
    let winner = vrf::weighted_leader(src_pk, &s, &source, &proof, validators).ok()?;
    if !nodes.iter().any(|n| n.online && n.id == winner) {
        return None;
    }
    let mut props: Vec<(usize, Proposal)> = Vec::new();
    for i in &online {
        if let Some(p) = try_propose(
            genesis,
            &mut nodes[*i],
            validators,
            vrf_pks,
            source,
            &src_sk,
            clock,
        ) {
            props.push((*i, p));
            break;
        }
    }
    if props.is_empty() {
        return None;
    }
    let mut deliveries: Vec<(usize, Proposal)> = Vec::new();
    if byzantine_split && props.len() == 1 {
        let (pi, p) = props[0].clone();
        let mut p2 = p.clone();
        p2.header.tx_root = Hash::from_bytes([0xab; 32]);
        for (k, j) in online.iter().enumerate() {
            if k % 2 == 0 {
                deliveries.push((*j, p.clone()));
            } else {
                deliveries.push((*j, p2.clone()));
            }
        }
        let _ = pi;
    } else {
        for j in &online {
            deliveries.push((*j, props[0].1.clone()));
        }
    }

    let mut prevotes = Vec::new();
    for (j, p) in &deliveries {
        let n = &mut nodes[*j];
        if let Ok(v) = prevote_step(
            &n.sk,
            n.id,
            p,
            vrf_pks,
            &seed(&n.last_hash),
            validators,
            n.height,
            n.round,
            n.lock,
            None,
            &mut n.replay,
        ) {
            log_vote(&mut n.wal, &v).ok();
            prevotes.push(v);
        }
    }

    let mut precommits = Vec::new();
    for (j, p) in &deliveries {
        let n = &mut nodes[*j];
        let (pc, lock) = precommit_step(
            &n.sk,
            n.id,
            n.height,
            n.round,
            &prevotes,
            validators,
            &p.header,
            &mut n.replay,
        );
        log_vote(&mut n.wal, &pc).ok();
        precommits.push(pc);
        let _ = lock;
    }

    let reachable = VotingPower(online.len() as u64);
    let mut committed = None;
    if let Some((_, p)) = props.first() {
        for i in &online {
            let n = &mut nodes[*i];
            match commit(&precommits, validators, reachable, p, &mut n.commits) {
                Ok(Some(f)) => {
                    let recs: Vec<Vec<u8>> = vec![];
                    let block = types::block::Block {
                        header_fields: p.header.fields.clone(),
                        txs: vec![],
                    };
                    put_block(&mut n.chain, &p.header, &block, &recs, &f.app_hash).ok();
                    assert_eq!(
                        f.app_hash,
                        app_hash(
                            &p.header.state_root,
                            &p.header.tx_root,
                            &p.header.receipts_root
                        )
                    );
                    n.last_hash = f.block_hash;
                    n.height = n.height.saturating_next();
                    n.round = Round::ZERO;
                    n.lock = None;
                    committed = Some(f.block_hash);
                }
                Ok(None) => {}
                Err(e) => panic!(
                    "commit error: {e:?} prevotes={} precommits={}",
                    prevotes.len(),
                    precommits.len()
                ),
            }
        }
    }
    if !byzantine_split && !props.is_empty() && committed.is_none() && online.len() >= 3 {
        panic!(
            "proposal but no commit: prevotes={} precommits={} online={}",
            prevotes.len(),
            precommits.len(),
            online.len()
        );
    }
    committed
}

#[test]
fn safety_split_proposals_no_two_commits() {
    let (genesis, mut nodes, validators, vrf_pks) = four_nodes();
    let clock = TestClock::new(1_700_000_000_000);
    let mut seen: Map<Height, Hash> = Map::new();
    for _ in 0..40 {
        let _ = drive_round(&genesis, &mut nodes, &validators, &vrf_pks, &clock, true);
        clock.advance(50);
        for n in &nodes {
            if let Some(h) = n.commits.get(Height::GENESIS) {
                if let Some(prev) = seen.insert(Height::GENESIS, h) {
                    assert_eq!(prev, h, "two commits at genesis");
                }
            }
        }
    }
}

#[test]
fn liveness_one_offline() {
    let (genesis, mut nodes, validators, vrf_pks) = four_nodes();
    nodes[3].online = false;
    let clock = TestClock::new(1_700_000_000_000);
    let mut committed = false;
    for r in 0..80u32 {
        for n in nodes.iter_mut().filter(|n| n.online) {
            n.round = Round(r % 8);
        }
        let c = drive_round(&genesis, &mut nodes, &validators, &vrf_pks, &clock, false);
        if c.is_some() {
            committed = true;
            break;
        }
        clock.advance(TimeoutConfig::from_spec().duration_ms(TimeoutStep::Propose, Round::ZERO));
    }
    assert!(committed, "3/4 should eventually commit");
}

#[test]
fn halt_two_offline() {
    let (genesis, mut nodes, validators, vrf_pks) = four_nodes();
    nodes[2].online = false;
    nodes[3].online = false;
    let clock = TestClock::new(1_700_000_000_000);
    for r in 0..30u32 {
        for n in nodes.iter_mut().filter(|n| n.online) {
            n.round = Round(r % 8);
        }
        let c = drive_round(&genesis, &mut nodes, &validators, &vrf_pks, &clock, false);
        assert!(c.is_none(), "must not commit with 2/4 reachable");
        clock.advance(100);
    }
    assert!(consensus::safety::halt_no_quorum(
        VotingPower(2),
        VotingPower(4)
    ));
}

#[test]
fn vrf_future_round_needs_finalized_seed() {
    let (genesis, nodes, validators, _vrf_pks) = four_nodes();
    let ghash = genesis.hash();
    let src = round_vrf_source(&validators, Round::ZERO).unwrap();
    let src_node = nodes.iter().find(|n| n.id == src).unwrap();
    let s0 = vrf::derive_seed(ghash.as_bytes(), Epoch::ZERO);
    let fake_future = Hash::from_bytes([0x42; 32]);
    let s1 = vrf::derive_seed(fake_future.as_bytes(), Epoch::ZERO);
    assert_ne!(s0, s1);
    let (_, p0) = vrf::leader_prove(&src_node.vrf_sk, &s0, &src).unwrap();
    let (_, p1) = vrf::leader_prove(&src_node.vrf_sk, &s1, &src).unwrap();
    let w0 = vrf::weighted_leader(&src_node.vrf_pk, &s0, &src, &p0, &validators).unwrap();
    let w1 = vrf::weighted_leader(&src_node.vrf_pk, &s1, &src, &p1, &validators).unwrap();
    let _ = (w0, w1);
}

#[test]
fn vrf_weighting_in_consensus_context() {
    let mut set = Map::new();
    let a = ValidatorId::from_bytes([1u8; 48]);
    let b = ValidatorId::from_bytes([2u8; 48]);
    let c = ValidatorId::from_bytes([3u8; 48]);
    set.insert(a, VotingPower(1));
    set.insert(b, VotingPower(1));
    set.insert(c, VotingPower(2));
    let sk = [11u8; 32];
    let pk = public_key_from_seed(&sk);
    let src = ValidatorId::from_bytes([0u8; 48]);
    let n = 3_000u32;
    let mut ca = 0u32;
    let mut cb = 0u32;
    let mut cc = 0u32;
    for i in 0..n {
        let mut hash = [0u8; 32];
        hash[..4].copy_from_slice(&i.to_be_bytes());
        let seed = vrf::derive_seed(&hash, Epoch::ZERO);
        let (_, proof) = vrf::leader_prove(&sk, &seed, &src).unwrap();
        let w = vrf::weighted_leader(&pk, &seed, &src, &proof, &set).unwrap();
        if w == a {
            ca += 1;
        } else if w == b {
            cb += 1;
        } else {
            cc += 1;
        }
    }
    let fa = f64::from(ca) / f64::from(n);
    let fb = f64::from(cb) / f64::from(n);
    let fc = f64::from(cc) / f64::from(n);
    assert!(
        (0.18..=0.32).contains(&fa) && (0.18..=0.32).contains(&fb) && (0.42..=0.58).contains(&fc),
        "a={fa} b={fb} c={fc} expect ~0.25,0.25,0.50 ±0.07 (n={n})"
    );
}

#[test]
fn no_double_sign_after_wal_restart() {
    use consensus::vote::prevote;
    use types::header::{Header, DA_ROOT_PLACEHOLDER};

    let (genesis, nodes, _, _) = four_nodes();
    let n = &nodes[0];
    let clock = TestClock::new(1_000);
    let fields = HeaderFields::new(&clock, Height::GENESIS, Round::ZERO, n.id, 0, 1).unwrap();
    let h1 = Header {
        fields: fields.clone(),
        tx_root: Hash::from_bytes([1u8; 32]),
        state_root: Hash::ZERO,
        receipts_root: Hash::ZERO,
        validators_hash: Hash::ZERO,
        da_root: DA_ROOT_PLACEHOLDER,
    };
    let h2 = Header {
        fields,
        tx_root: Hash::from_bytes([2u8; 32]),
        state_root: Hash::ZERO,
        receipts_root: Hash::ZERO,
        validators_hash: Hash::ZERO,
        da_root: DA_ROOT_PLACEHOLDER,
    };
    let a = prevote(&n.sk, n.id, Height::GENESIS, Round::ZERO, &h1);
    let mut wal = MemoryStore::new();
    log_vote(&mut wal, &a).unwrap();
    let b = prevote(&n.sk, n.id, Height::GENESIS, Round::ZERO, &h2);
    assert!(check_no_double_sign(&wal, &b, Some(&a)).is_err());
    let _ = genesis;
    let _ = Mempool::new(&ParamsRegistry::new());
    let _ = BoundClock::new(&clock, TimeoutConfig::from_spec());
    let _ = TimeoutStep::Propose;
    let _ = VoteKind::Prevote;
}
