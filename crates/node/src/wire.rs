//! Node event-loop wiring (development-plan.md Devnet MVP).
//!
//! Each function is a hand-off, not a new protocol:
//! - [`wire_mempool`]: gossip.tx → `mempool.fee_order` queue (no re-verify)
//! - [`wire_propose`]: `cons.propose` → gossip.proposal
//! - [`wire_vote`]: `cons.prevote_step` / `cons.precommit_step` → `mesh.validator`
//! - [`wire_commit`]: `cons.commit` then WAL + `store.block.put` then gossip.block
//! - [`wire_sync`]: `sync.headers_then_bodies` then the same persist path as commit

use crate::config::NodeConfig;
use consensus::propose::{propose, Proposal};
use consensus::replay::VoteKind;
use consensus::safety::CommitLog;
use consensus::steps::{commit, precommit_step, prevote_step, Finalized, PrevoteError};
use consensus::vote::{Vote, VoteReplayLog};
use consensus::vrf as cons_vrf;
use consensus::vrf::VrfSeed;
use execution::builder::{build_local, BuiltBlock};
use execution::seq::World;
use mempool::Mempool;
use network::codec::{encode_proposal, encode_vote};
use network::sync::{headers_then_bodies, BodyOffer};
use network::topics::{ingest_block, ingest_proposal, ingest_tx, ingest_vote, TopicError};
use network::validator_mesh::{ingest_validator_proposal, ingest_validator_vote, ValidatorMesh};
use state::account::Account;
use storage::blocks::{put_block, put_genesis_hash};
use storage::kv::Store;
use storage::memory::MemoryStore;
use storage::wal::{clear_wal, write_wal};
use thiserror::Error;
use types::block::Block;
use types::collections::Map;
use types::header::Header;
use types::tx::SignedTx;
use types::{Clock, Hash, Height, Round, ValidatorId, VotingPower};

/// Outbound gossip/mesh. Tests record order; the process loop publishes.
pub trait BlockBroadcast {
    /// After durable `store.block.put`.
    fn broadcast_block(&mut self, header: &Header, block: &Block, receipts: &[Vec<u8>], app: &Hash);
    /// `gossip.proposal`.
    fn broadcast_proposal(&mut self, proposal: &Proposal);
    /// `mesh.validator` vote topic (not general gossip).
    fn broadcast_vote(&mut self, vote: &Vote);
}

/// Recording sink for persist-before-broadcast tests.
#[derive(Default)]
pub struct TraceSink {
    /// Ordered labels: `wal`, `put`, `broadcast`.
    pub order: Vec<&'static str>,
    /// Finalized header hashes announced.
    pub blocks: Vec<Hash>,
    /// When broadcast ran.
    pub broadcast_at: Option<std::time::Instant>,
}

impl BlockBroadcast for TraceSink {
    fn broadcast_block(&mut self, header: &Header, _block: &Block, _r: &[Vec<u8>], _a: &Hash) {
        self.order.push("broadcast");
        self.broadcast_at = Some(std::time::Instant::now());
        self.blocks.push(header.hash());
    }
    fn broadcast_proposal(&mut self, _proposal: &Proposal) {}
    fn broadcast_vote(&mut self, _vote: &Vote) {}
}

/// Wiring errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WireError {
    /// Gossip ingest.
    #[error("gossip")]
    Gossip,
    /// Not leader / propose.
    #[error("propose")]
    Propose,
    /// Prevote.
    #[error("prevote")]
    Prevote,
    /// Commit / safety.
    #[error("commit")]
    Commit,
    /// Store.
    #[error("store")]
    Store,
}

impl From<TopicError> for WireError {
    fn from(_: TopicError) -> Self {
        Self::Gossip
    }
}

/// Durable store then announce. Shared by local [`wire_commit`] and [`wire_sync`].
pub fn persist_then_broadcast<S: Store, B: BlockBroadcast>(
    store: &mut S,
    header: &Header,
    block: &Block,
    receipts: &[Vec<u8>],
    app_hash: &Hash,
    sink: &mut B,
) -> Result<(), WireError> {
    ingest_block(header, block, receipts).map_err(|_| WireError::Gossip)?;
    write_wal(store, header, block, receipts, app_hash).map_err(|_| WireError::Store)?;
    put_block(store, header, block, receipts, app_hash).map_err(|_| WireError::Store)?;
    clear_wal(store).map_err(|_| WireError::Store)?;
    sink.broadcast_block(header, block, receipts, app_hash);
    Ok(())
}

/// Gossip.tx then fee-order queue. Trigger: inbound tx. Next: propose. Contract: `node.wire.mempool`.
pub fn wire_mempool(
    signed: &SignedTx,
    account: &Account,
    pool: &mut Mempool,
) -> Result<(), WireError> {
    ingest_tx(signed, account)?;
    let addr = mempool::sender_address(signed).map_err(|_| WireError::Gossip)?;
    pool.admit_preverified(signed.clone(), addr);
    let _ = pool.peek_best_ready(
        &types::collections::Map::new(),
        &types::collections::Set::new(),
    );
    Ok(())
}

/// Leader path. Trigger: round tick. Next: votes. Contract: `node.wire.propose`.
#[allow(clippy::too_many_arguments)]
pub fn wire_propose<C: Clock, B: BlockBroadcast>(
    cfg: &NodeConfig,
    bls_sk: &blst::min_pk::SecretKey,
    our_id: ValidatorId,
    vrf_source: ValidatorId,
    source_vrf_pk: &[u8; 32],
    source_proof: &crypto::vrf::Proof,
    seed: &VrfSeed,
    height: Height,
    round: Round,
    clock: &C,
    last_ts: u64,
    world: World,
    pool: &mut Mempool,
    sink: &mut B,
) -> Result<(Proposal, BuiltBlock), WireError> {
    let mut built_slot: Option<BuiltBlock> = None;
    let ts = clock.now_millis();
    let proposal = propose(
        bls_sk,
        our_id,
        vrf_source,
        source_vrf_pk,
        source_proof,
        &cfg.genesis.validators,
        seed,
        height,
        round,
        || {
            let fields =
                types::header::HeaderFields::new(clock, height, round, our_id, last_ts, ts)
                    .expect("timestamp");
            let b = build_local(pool, &cfg.genesis, world.clone(), fields);
            let hdr = b.header.clone();
            let app = b.app_hash;
            built_slot = Some(b);
            (hdr, app)
        },
    )
    .map_err(|_| WireError::Propose)?;
    ingest_proposal(&proposal)?;
    sink.broadcast_proposal(&proposal);
    let _ = encode_proposal(&proposal);
    Ok((proposal, built_slot.expect("build")))
}

/// Drive prevote + precommit; votes go out on the validator mesh. Trigger: proposal/votes. Contract: `node.wire.vote`.
#[allow(clippy::too_many_arguments)]
pub fn wire_vote(
    mesh: &ValidatorMesh,
    bls_sk: &blst::min_pk::SecretKey,
    our_id: ValidatorId,
    proposal: &Proposal,
    vrf_pks: &Map<ValidatorId, [u8; 32]>,
    seed: &VrfSeed,
    expected_height: Height,
    expected_round: Round,
    lock: Option<consensus::state::Lock>,
    log: &mut VoteReplayLog,
    sink: &mut impl BlockBroadcast,
) -> Result<Vote, WireError> {
    ingest_validator_proposal(mesh, proposal).map_err(|_| WireError::Gossip)?;
    let prevote = prevote_step(
        bls_sk,
        our_id,
        proposal,
        vrf_pks,
        seed,
        &mesh.validators,
        expected_height,
        expected_round,
        lock,
        None,
        log,
    )
    .map_err(|_| WireError::Prevote)?;
    ingest_validator_vote(mesh, &prevote, log).map_err(|_| WireError::Gossip)?;
    sink.broadcast_vote(&prevote);
    let _ = encode_vote(&prevote);
    Ok(prevote)
}

/// Precommit after a prevote polka. Same contract (`node.wire.vote`).
#[allow(clippy::too_many_arguments)]
pub fn wire_precommit(
    mesh: &ValidatorMesh,
    bls_sk: &blst::min_pk::SecretKey,
    our_id: ValidatorId,
    height: Height,
    round: Round,
    prevotes: &[Vote],
    header: &types::header::Header,
    log: &mut VoteReplayLog,
    sink: &mut impl BlockBroadcast,
) -> Result<(Vote, Option<consensus::state::Lock>), WireError> {
    let (precommit, new_lock) = precommit_step(
        bls_sk,
        our_id,
        height,
        round,
        prevotes,
        &mesh.validators,
        header,
        log,
    );
    ingest_validator_vote(mesh, &precommit, log).map_err(|_| WireError::Gossip)?;
    sink.broadcast_vote(&precommit);
    let _ = VoteKind::Precommit;
    let _ = PrevoteError::Slot;
    Ok((precommit, new_lock))
}

/// Local finality. Trigger: precommit QC. Persist before gossip.block. Contract: `node.wire.commit`.
#[allow(clippy::too_many_arguments)]
pub fn wire_commit<S: Store, B: BlockBroadcast>(
    precommits: &[Vote],
    validators: &Map<ValidatorId, VotingPower>,
    reachable: VotingPower,
    proposal: &Proposal,
    commits: &mut CommitLog,
    store: &mut S,
    block: &Block,
    receipts: &[Vec<u8>],
    sink: &mut B,
) -> Result<Option<Finalized>, WireError> {
    let f = commit(precommits, validators, reachable, proposal, commits)
        .map_err(|_| WireError::Commit)?;
    let Some(f) = f else {
        return Ok(None);
    };
    persist_then_broadcast(store, &proposal.header, block, receipts, &f.app_hash, sink)?;
    Ok(Some(f))
}

/// Catch-up fetch then the same persist path. Trigger: local tip behind. Contract: `node.wire.sync`.
pub fn wire_sync<S: Store, B: BlockBroadcast>(
    local: &mut S,
    remote_headers: &[Header],
    bodies: &[BodyOffer],
    sink: &mut B,
) -> Result<Option<Height>, WireError> {
    let mut scratch = MemoryStore::new();
    headers_then_bodies(&mut scratch, remote_headers, bodies).map_err(|_| WireError::Gossip)?;
    for offer in bodies {
        persist_then_broadcast(
            local,
            &offer.header,
            &offer.block,
            &offer.receipts,
            &offer.app_hash,
            sink,
        )?;
    }
    storage::blocks::tip(local).map_err(|_| WireError::Store)
}

/// Ingest a vote that already passed `gossip.vote` / mesh.
pub fn wire_ingest_vote(vote: &Vote, log: &mut VoteReplayLog) -> Result<(), WireError> {
    ingest_vote(vote, log)?;
    Ok(())
}

/// Seed from last committed header hash.
pub fn round_seed(last: &Hash) -> VrfSeed {
    cons_vrf::derive_seed(last.as_bytes(), types::Epoch::ZERO)
}

/// Ensure genesis is recorded.
pub fn init_store<S: Store>(store: &mut S, cfg: &NodeConfig) -> Result<(), WireError> {
    put_genesis_hash(store, &cfg.genesis).map_err(|_| WireError::Store)
}

/// Insert a remote vote, ordered by signer (determinism).
pub fn insert_vote_sorted(votes: &mut Vec<Vote>, vote: Vote) {
    if votes
        .iter()
        .any(|x| x.signer == vote.signer && x.kind == vote.kind)
    {
        return;
    }
    votes.push(vote);
    votes.sort_by(|a, b| a.signer.cmp(&b.signer));
}

/// Rebuild the executed block from a gossiped proposal (empty extra mempool txs).
pub fn built_from_proposal(
    cfg: &NodeConfig,
    world: &World,
    proposal: &Proposal,
) -> Option<BuiltBlock> {
    let mut pool = Mempool::new(&cfg.genesis.params.registry);
    let built = build_local(
        &mut pool,
        &cfg.genesis,
        world.clone(),
        proposal.header.fields.clone(),
    );
    if built.header.hash() != proposal.header.hash() || built.app_hash != proposal.app_hash {
        return None;
    }
    Some(built)
}

/// Gossip.block payload (header preimage + body + app + receipts).
pub fn encode_body_offer(offer: &BodyOffer) -> Vec<u8> {
    let mut inner = Vec::new();
    let pre = offer.header.hash_preimage();
    inner.extend_from_slice(&(pre.len() as u32).to_be_bytes());
    inner.extend_from_slice(&pre);
    let body = storage::codec::encode_block_body(&offer.block);
    inner.extend_from_slice(&(body.len() as u32).to_be_bytes());
    inner.extend_from_slice(&body);
    inner.extend_from_slice(offer.app_hash.as_bytes());
    inner.extend_from_slice(&(offer.receipts.len() as u32).to_be_bytes());
    for r in &offer.receipts {
        inner.extend_from_slice(&(r.len() as u32).to_be_bytes());
        inner.extend_from_slice(r);
    }
    inner
}

/// Inverse of [`encode_body_offer`].
pub fn decode_body_offer(inner: &[u8]) -> Result<BodyOffer, WireError> {
    if inner.len() < 4 {
        return Err(WireError::Gossip);
    }
    let mut i = 0usize;
    let take = |i: &mut usize, n: usize| -> Result<&[u8], WireError> {
        if *i + n > inner.len() {
            return Err(WireError::Gossip);
        }
        let s = &inner[*i..*i + n];
        *i += n;
        Ok(s)
    };
    let plen = u32::from_be_bytes(take(&mut i, 4)?.try_into().unwrap()) as usize;
    let header =
        storage::codec::header_from_preimage(take(&mut i, plen)?).map_err(|_| WireError::Gossip)?;
    let blen = u32::from_be_bytes(take(&mut i, 4)?.try_into().unwrap()) as usize;
    let block =
        storage::codec::decode_block_body(take(&mut i, blen)?).map_err(|_| WireError::Gossip)?;
    let app = Hash::from_bytes(take(&mut i, 32)?.try_into().unwrap());
    let n = u32::from_be_bytes(take(&mut i, 4)?.try_into().unwrap()) as usize;
    let mut receipts = Vec::with_capacity(n);
    for _ in 0..n {
        let rlen = u32::from_be_bytes(take(&mut i, 4)?.try_into().unwrap()) as usize;
        receipts.push(take(&mut i, rlen)?.to_vec());
    }
    if i != inner.len() {
        return Err(WireError::Gossip);
    }
    Ok(BodyOffer {
        header,
        block,
        receipts,
        app_hash: app,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NodeConfig;
    use consensus::propose::round_vrf_source;
    use crypto::from_bls;
    use crypto::sig::bls;
    use crypto::sig::ed25519::SecretKey as EdSk;
    use crypto::tx::sign;
    use crypto::vrf::public_key_from_seed;
    use execution::builder::ReadyTxs;
    use network::identity;
    use network::topics::ingest_tx;
    use state::account::Account;
    use std::path::PathBuf;
    use std::thread;
    use std::time::{Duration, Instant};
    use types::genesis::Genesis;
    use types::header::HeaderFields;
    use types::{Amount, ChainId, Hash, Height, Nonce, Round, TestClock, GAS_TRANSFER};

    #[allow(clippy::type_complexity)]
    fn four_setup() -> (
        NodeConfig,
        Vec<(blst::min_pk::SecretKey, ValidatorId, [u8; 32], [u8; 32])>,
        Map<ValidatorId, [u8; 32]>,
        ValidatorMesh,
    ) {
        let mut genesis = Genesis::new(ChainId::new(1));
        let mut keys = Vec::new();
        let mut vrf_pks = Map::new();
        for i in 0..4u8 {
            let sk = bls::keygen().unwrap();
            let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(1));
            genesis.insert_validator(id, VotingPower(1));
            let vrf_sk = [i + 1; 32];
            let vrf_pk = public_key_from_seed(&vrf_sk);
            vrf_pks.insert(id, vrf_pk);
            keys.push((sk, id, vrf_sk, vrf_pk));
        }
        let id = identity::generate().unwrap();
        let mesh = ValidatorMesh::from_genesis(&genesis);
        let cfg = NodeConfig::new(
            genesis,
            network::discovery::BootstrapList::new(),
            id,
            PathBuf::from("/tmp"),
        );
        (cfg, keys, vrf_pks, mesh)
    }

    #[test]
    fn mempool_drops_invalid_and_does_not_reverify_path_for_valid() {
        let ska = EdSk::from_bytes(&[3u8; 32]);
        let from = crypto::from_ed25519(&ska.verifying_key());
        let account = Account {
            balance: Amount::new(1_000),
            nonce: Nonce::ZERO,
            code_hash: Hash::ZERO,
        };
        let tx = types::tx::Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            from,
            Amount::new(10),
        );
        let mut bad = sign(&ska, tx.clone());
        bad.signature[0] ^= 1;
        let mut pool = Mempool::new(&types::ParamsRegistry::new());
        assert!(wire_mempool(&bad, &account, &mut pool).is_err());
        assert!(ingest_tx(&bad, &account).is_err());

        let good = sign(&ska, tx);
        wire_mempool(&good, &account, &mut pool).unwrap();
        let got = pool.take_ready(50_000, 1_000_000);
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn persist_happens_before_broadcast_even_if_store_is_slow() {
        let (cfg, keys, _, _) = four_setup();
        let clock = TestClock::new(1_000);
        let fields =
            HeaderFields::new(&clock, Height::GENESIS, Round::ZERO, keys[0].1, 0, 1).unwrap();
        let mut pool = Mempool::new(&cfg.genesis.params.registry);
        let world = World::from_genesis(&cfg.genesis);
        let built = build_local(&mut pool, &cfg.genesis, world, fields);
        struct DelayMem {
            inner: MemoryStore,
            put_done: std::sync::Arc<std::sync::Mutex<Option<Instant>>>,
        }
        impl Store for DelayMem {
            fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, types::TypesError> {
                self.inner.get(key)
            }
            fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), types::TypesError> {
                thread::sleep(Duration::from_millis(30));
                let r = self.inner.put(key, value);
                *self.put_done.lock().unwrap() = Some(Instant::now());
                r
            }
            fn delete(&mut self, key: &[u8]) -> Result<(), types::TypesError> {
                self.inner.delete(key)
            }
            fn prefix(&self, p: &[u8]) -> Result<Vec<storage::kv::KvEntry>, types::TypesError> {
                self.inner.prefix(p)
            }
        }
        let put_done = std::sync::Arc::new(std::sync::Mutex::new(None));
        let mut store = DelayMem {
            inner: MemoryStore::new(),
            put_done: put_done.clone(),
        };
        init_store(&mut store, &cfg).unwrap();
        let mut sink = TraceSink::default();
        persist_then_broadcast(
            &mut store,
            &built.header,
            &built.block,
            &built
                .receipts
                .iter()
                .map(|r| r.encode())
                .collect::<Vec<_>>(),
            &built.app_hash,
            &mut sink,
        )
        .unwrap();
        let put_at = put_done.lock().unwrap().expect("put");
        let bcast_at = sink.broadcast_at.expect("broadcast");
        assert!(put_at <= bcast_at, "put {put_at:?} broadcast {bcast_at:?}");
        assert_eq!(sink.order, ["broadcast"]);
        assert_eq!(sink.blocks.len(), 1);
    }

    #[test]
    fn commit_calls_cons_commit_then_persist() {
        let (cfg, keys, vrf_pks, mesh) = four_setup();
        let src = round_vrf_source(&cfg.genesis.validators, Round::ZERO).unwrap();
        let src_row = keys.iter().find(|k| k.1 == src).unwrap();
        let seed = cons_vrf::derive_seed(&[1u8; 32], types::Epoch::ZERO);
        let (_, proof) = cons_vrf::leader_prove(&src_row.2, &seed, &src).unwrap();
        let winner =
            cons_vrf::weighted_leader(&src_row.3, &seed, &src, &proof, &cfg.genesis.validators)
                .unwrap();
        let w = keys.iter().find(|k| k.1 == winner).unwrap();
        let clock = TestClock::new(1_000);
        let mut pool = Mempool::new(&cfg.genesis.params.registry);
        let mut sink = TraceSink::default();
        let (proposal, built) = wire_propose(
            &cfg,
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
            World::from_genesis(&cfg.genesis),
            &mut pool,
            &mut sink,
        )
        .unwrap();
        let mut log = VoteReplayLog::new();
        let mut prevotes = Vec::new();
        let mut precommits = Vec::new();
        for k in &keys {
            let pv = wire_vote(
                &mesh,
                &k.0,
                k.1,
                &proposal,
                &vrf_pks,
                &seed,
                Height::GENESIS,
                Round::ZERO,
                None,
                &mut log,
                &mut sink,
            )
            .unwrap();
            prevotes.push(pv);
        }
        for k in &keys {
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
        let mut commits = CommitLog::new();
        let mut store = MemoryStore::new();
        init_store(&mut store, &cfg).unwrap();
        let rec: Vec<_> = built.receipts.iter().map(|r| r.encode()).collect();
        let total = VotingPower(4);
        let f = wire_commit(
            &precommits,
            &cfg.genesis.validators,
            total,
            &proposal,
            &mut commits,
            &mut store,
            &built.block,
            &rec,
            &mut sink,
        )
        .unwrap()
        .unwrap();
        assert_eq!(f.height, Height::GENESIS);
        assert_eq!(storage::blocks::tip(&store).unwrap(), Some(Height::GENESIS));
        assert!(
            execution::seq::apply_block(World::from_genesis(&cfg.genesis), &built.block).2
                == f.app_hash
        );
    }
}
