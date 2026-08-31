//! L1 node binary — P2P only (no RPC; Tier 8).

use consensus::propose::round_vrf_source;
use consensus::replay::VoteKind;
use consensus::safety::CommitLog;
use consensus::vote::{Vote, VoteReplayLog};
use consensus::vrf;
use crypto::from_bls;
use execution::builder::BuiltBlock;
use execution::seq::World;
use libp2p::futures::StreamExt;
use libp2p::gossipsub::IdentTopic;
use libp2p::swarm::SwarmEvent;
use mempool::Mempool;
use network::codec::{decode_frame, decode_proposal, decode_vote, encode_frame, GossipKind};
use network::gossip::{ident_topic, mesh_swarm, TOPIC_BLOCK, TOPIC_PROPOSAL, TOPIC_TX};
use network::sync::BodyOffer;
use network::transport::quic_listen_local;
use network::validator_mesh::{validator_proposal_topic, validator_vote_topic, ValidatorMesh};
use node::config::{load_bootstrap, load_dir, NodeConfig};
use node::sync::catchup;
use node::tracing;
use node::wire::{
    built_from_proposal, decode_body_offer, encode_body_offer, init_store, insert_vote_sorted,
    persist_then_broadcast, round_seed, wire_commit, wire_mempool, wire_precommit, wire_propose,
    wire_vote, BlockBroadcast,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use storage::memory::MemoryStore;
use types::collections::Map;
use types::{Clock, Height, Round, SystemClock, ValidatorId, VotingPower};

struct OutboxSink<'a> {
    out: &'a mut Vec<(IdentTopic, Vec<u8>)>,
}

impl BlockBroadcast for OutboxSink<'_> {
    fn broadcast_block(
        &mut self,
        header: &types::header::Header,
        block: &types::block::Block,
        receipts: &[Vec<u8>],
        app: &types::Hash,
    ) {
        let offer = BodyOffer {
            header: header.clone(),
            block: block.clone(),
            receipts: receipts.to_vec(),
            app_hash: *app,
        };
        self.out.push((
            ident_topic(TOPIC_BLOCK),
            encode_frame(GossipKind::Block, &encode_body_offer(&offer)),
        ));
    }
    fn broadcast_proposal(&mut self, p: &consensus::propose::Proposal) {
        let inner = network::codec::encode_proposal(p);
        let frame = encode_frame(GossipKind::Proposal, &inner);
        self.out.push((ident_topic(TOPIC_PROPOSAL), frame.clone()));
        self.out.push((validator_proposal_topic(), frame));
    }
    fn broadcast_vote(&mut self, v: &Vote) {
        let frame = encode_frame(GossipKind::Vote, &network::codec::encode_vote(v));
        self.out.push((validator_vote_topic(), frame));
    }
}

fn append_event(dir: &Path, line: &str) {
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("events.log"))
        .and_then(|mut f| {
            use std::io::Write;
            writeln!(f, "{line}")
        });
}

fn write_tip(dir: &Path, h: Height, ms: u64) {
    let _ = std::fs::write(dir.join("tip"), format!("{}\n{ms}\n", h.0));
}

struct Live {
    cfg: NodeConfig,
    bls_sk: blst::min_pk::SecretKey,
    our_id: ValidatorId,
    vrf_sk: [u8; 32],
    vrf_secrets: Map<ValidatorId, [u8; 32]>,
    vrf_pks: Map<ValidatorId, [u8; 32]>,
    mesh: ValidatorMesh,
    store: MemoryStore,
    pool: Mempool,
    world: World,
    replay: VoteReplayLog,
    commits: CommitLog,
    height: Height,
    round: Round,
    last_hash: types::Hash,
    last_ts: u64,
    last_commit: Instant,
    pending: Option<(consensus::propose::Proposal, BuiltBlock)>,
    prevotes: Vec<Vote>,
    precommits: Vec<Vote>,
    clock: SystemClock,
    committed: Vec<BodyOffer>,
    sync_offers: Map<u64, BodyOffer>,
    is_validator: bool,
    started: Instant,
    last_repub: Instant,
    last_prop: Instant,
}

impl Live {
    fn total_power(&self) -> VotingPower {
        self.cfg
            .genesis
            .validators
            .values()
            .fold(VotingPower::ZERO, |a, p| a.saturating_add(*p))
    }

    fn handle_bytes(&mut self, data: &[u8], out: &mut Vec<(IdentTopic, Vec<u8>)>, dir: &Path) {
        let Ok(frame) = decode_frame(data) else {
            return;
        };
        match frame.kind {
            GossipKind::Tx => {
                if let Ok(signed) = storage::codec::decode_signed_tx(&frame.inner) {
                    let addr = mempool::sender_address(&signed).ok();
                    let account = addr
                        .map(|a| self.world.account(&a))
                        .unwrap_or_else(state::account::Account::empty);
                    match wire_mempool(&signed, &account, &mut self.pool) {
                        Ok(()) => append_event(dir, "TX_ADMIT"),
                        Err(_) => append_event(dir, "TX_DROP"),
                    }
                } else {
                    append_event(dir, "TX_DROP");
                }
            }
            GossipKind::Vote => {
                if let Ok(v) = decode_vote(&frame.inner) {
                    let _ = node::wire::wire_ingest_vote(&v, &mut self.replay);
                    if v.height == self.height && v.round == self.round {
                        match v.kind {
                            VoteKind::Prevote => insert_vote_sorted(&mut self.prevotes, v),
                            VoteKind::Precommit => insert_vote_sorted(&mut self.precommits, v),
                        }
                    }
                }
            }
            GossipKind::Proposal => {
                if let Ok(p) = decode_proposal(&frame.inner) {
                    if p.height == self.height
                        && v_round_ok(&p, self.round)
                        && self.pending.is_none()
                    {
                        if let Some(b) = built_from_proposal(&self.cfg, &self.world, &p) {
                            self.pending = Some((p, b));
                            self.prevotes.clear();
                            self.precommits.clear();
                        }
                    }
                }
            }
            GossipKind::Block => {
                if let Ok(offer) = decode_body_offer(&frame.inner) {
                    self.sync_offers.insert(offer.header.fields.height.0, offer);
                    if !self.is_validator {
                        self.try_catchup(out, dir);
                    }
                }
            }
            _ => {}
        }
        let _ = TOPIC_TX;
    }

    fn try_catchup(&mut self, out: &mut Vec<(IdentTopic, Vec<u8>)>, dir: &Path) {
        let mut heights: Vec<u64> = self.sync_offers.keys().copied().collect();
        heights.sort_unstable();
        if heights.is_empty() {
            return;
        }
        for (i, h) in heights.iter().enumerate() {
            if *h != i as u64 {
                return;
            }
        }
        let headers: Vec<_> = heights
            .iter()
            .map(|h| self.sync_offers.get(h).unwrap().header.clone())
            .collect();
        let bodies: Vec<_> = heights
            .iter()
            .map(|h| self.sync_offers.get(h).unwrap().clone())
            .collect();
        let mut sink = OutboxSink { out };
        if let Ok(app) = catchup(&self.cfg, &mut self.store, &headers, &bodies, &mut sink) {
            if let Some(last) = bodies.last() {
                if let Ok((w, _)) = storage::replay::replay_from_genesis(
                    &self.store,
                    &self.cfg.genesis,
                    World::from_genesis(&self.cfg.genesis),
                    execution::seq::apply_block,
                ) {
                    self.world = w;
                }
                let tip_h = last.header.fields.height;
                self.height = Height(tip_h.0.saturating_add(1));
                self.last_hash = last.header.hash();
                self.last_ts = last.header.fields.timestamp_ms;
                write_tip(dir, tip_h, self.clock.now_millis());
                append_event(
                    dir,
                    &format!("CATCHUP {} app={}", tip_h.0, hex::encode(app.as_bytes())),
                );
            }
        }
    }

    fn tick(&mut self, out: &mut Vec<(IdentTopic, Vec<u8>)>, dir: &Path) {
        if !self.is_validator {
            return;
        }
        if self.started.elapsed() < Duration::from_secs(2) {
            self.maybe_vote_and_commit(out, dir);
            return;
        }
        if self.last_commit.elapsed() < Duration::from_millis(self.cfg.min_block_time_ms) {
            self.maybe_vote_and_commit(out, dir);
            return;
        }
        let seed = round_seed(&self.last_hash);
        let Some(src) = round_vrf_source(&self.cfg.genesis.validators, self.round) else {
            return;
        };
        let Some(src_pk) = self.vrf_pks.get(&src).copied() else {
            return;
        };
        let prove_sk = self.vrf_secrets.get(&src).unwrap_or(&self.vrf_sk);
        let Ok((_, proof)) = vrf::leader_prove(prove_sk, &seed, &src) else {
            return;
        };
        let Ok(winner) =
            vrf::weighted_leader(&src_pk, &seed, &src, &proof, &self.cfg.genesis.validators)
        else {
            return;
        };
        if winner == self.our_id && self.pending.is_none() {
            let mut sink = OutboxSink { out };
            match wire_propose(
                &self.cfg,
                &self.bls_sk,
                self.our_id,
                src,
                &src_pk,
                &proof,
                &seed,
                self.height,
                self.round,
                &self.clock,
                self.last_ts,
                self.world.clone(),
                &mut self.pool,
                &mut sink,
            ) {
                Ok((p, b)) => {
                    append_event(dir, "PROPOSE_OK");
                    self.pending = Some((p, b));
                    self.prevotes.clear();
                    self.precommits.clear();
                }
                Err(_) => append_event(dir, "PROPOSE_ERR"),
            }
        }
        self.maybe_vote_and_commit(out, dir);
        if self.last_repub.elapsed() >= Duration::from_secs(2) && !self.committed.is_empty() {
            for offer in &self.committed {
                out.push((
                    ident_topic(TOPIC_BLOCK),
                    encode_frame(GossipKind::Block, &encode_body_offer(offer)),
                ));
            }
            self.last_repub = Instant::now();
        }
    }

    fn maybe_vote_and_commit(&mut self, out: &mut Vec<(IdentTopic, Vec<u8>)>, dir: &Path) {
        let seed = round_seed(&self.last_hash);
        if self.pending.is_none() {
            return;
        }
        let prop = self.pending.as_ref().unwrap().0.clone();
        if self.last_prop.elapsed() >= Duration::from_millis(250) {
            let mut sink = OutboxSink { out };
            sink.broadcast_proposal(&prop);
            if let Some(v) = self.prevotes.iter().find(|v| v.signer == self.our_id) {
                sink.broadcast_vote(v);
            }
            if let Some(v) = self.precommits.iter().find(|v| v.signer == self.our_id) {
                sink.broadcast_vote(v);
            }
            self.last_prop = Instant::now();
        }
        if self.prevotes.iter().all(|v| v.signer != self.our_id) {
            let mut sink = OutboxSink { out };
            sink.broadcast_proposal(&prop);
            if let Ok(v) = wire_vote(
                &self.mesh,
                &self.bls_sk,
                self.our_id,
                &prop,
                &self.vrf_pks,
                &seed,
                self.height,
                self.round,
                None,
                &mut self.replay,
                &mut sink,
            ) {
                insert_vote_sorted(&mut self.prevotes, v);
            }
        }
        if self.prevotes.len() >= 3 && self.precommits.iter().all(|v| v.signer != self.our_id) {
            let mut sink = OutboxSink { out };
            if let Ok((pc, _)) = wire_precommit(
                &self.mesh,
                &self.bls_sk,
                self.our_id,
                self.height,
                self.round,
                &self.prevotes,
                &prop.header,
                &mut self.replay,
                &mut sink,
            ) {
                insert_vote_sorted(&mut self.precommits, pc);
            }
        }
        if self.precommits.len() >= 3 {
            let rec: Vec<_> = self
                .pending
                .as_ref()
                .unwrap()
                .1
                .receipts
                .iter()
                .map(|r| r.encode())
                .collect();
            let block = self.pending.as_ref().unwrap().1.block.clone();
            let mut sink = OutboxSink { out };
            if let Ok(Some(f)) = wire_commit(
                &self.precommits,
                &self.cfg.genesis.validators,
                self.total_power(),
                &prop,
                &mut self.commits,
                &mut self.store,
                &block,
                &rec,
                &mut sink,
            ) {
                let built = self.pending.as_ref().unwrap();
                let world = built.1.world.clone();
                let block = built.1.block.clone();
                let receipts: Vec<Vec<u8>> = built.1.receipts.iter().map(|r| r.encode()).collect();
                let app = f.app_hash;
                self.world = world;
                self.last_hash = f.block_hash;
                self.last_ts = prop.header.fields.timestamp_ms;
                self.height = Height(f.height.0.saturating_add(1));
                self.round = Round::ZERO;
                self.last_commit = Instant::now();
                self.pending = None;
                self.prevotes.clear();
                self.precommits.clear();
                self.committed.push(BodyOffer {
                    header: prop.header.clone(),
                    block,
                    receipts,
                    app_hash: app,
                });
                write_tip(dir, f.height, self.clock.now_millis());
                append_event(dir, &format!("COMMIT {}", f.height.0));
            }
        }
        let _ = persist_then_broadcast::<MemoryStore, OutboxSink>;
    }
}

fn v_round_ok(p: &consensus::propose::Proposal, round: Round) -> bool {
    p.round == round
}

fn parse_dir() -> PathBuf {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--dir") {
        PathBuf::from(args.get(2).expect("node --dir PATH"))
    } else {
        PathBuf::from(args.get(1).expect("usage: node --dir PATH | node PATH"))
    }
}

#[tokio::main]
async fn main() {
    tracing::init();
    let dir = parse_dir();
    let (mut cfg, bls_bytes, vrf_sk) = load_dir(&dir).expect("load config");
    cfg.data_dir = dir.clone();
    let bls_sk = blst::min_pk::SecretKey::from_bytes(&bls_bytes).expect("bls");
    let (our_id, _) = from_bls(&bls_sk.sk_to_pk(), VotingPower(1));
    let mut swarm = mesh_swarm(cfg.identity.clone(), &cfg.bootstrap).expect("swarm");
    let _ = swarm
        .behaviour_mut()
        .gossipsub
        .subscribe(&validator_proposal_topic());
    let _ = swarm
        .behaviour_mut()
        .gossipsub
        .subscribe(&validator_vote_topic());
    // `L1_LISTEN` is for container / LAN bind (`/ip4/0.0.0.0/udp/N/quic-v1`).
    // Default remains `p2p.quic`'s `quic_listen_local` (127.0.0.1) so simnet is unchanged.
    let listen_addr = std::env::var("L1_LISTEN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(quic_listen_local);
    swarm.listen_on(listen_addr).expect("listen");
    let mut listen = None;
    while listen.is_none() {
        match swarm.next().await {
            Some(SwarmEvent::NewListenAddr { address, .. }) => {
                listen = Some(address);
            }
            Some(_) => {}
            None => return,
        }
    }
    let addr = listen.unwrap();
    let _ = std::fs::write(dir.join("listen"), addr.to_string());
    for (_, a) in cfg.bootstrap.peers.clone() {
        let _ = swarm.dial(a);
    }

    let vrf_pks_bytes = std::fs::read(dir.join("vrf_pks.bin")).unwrap_or_default();
    let mut vrf_pks = Map::new();
    let mut i = 0usize;
    while i + 80 <= vrf_pks_bytes.len() {
        let mut idb = [0u8; 48];
        idb.copy_from_slice(&vrf_pks_bytes[i..i + 48]);
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&vrf_pks_bytes[i + 48..i + 80]);
        vrf_pks.insert(types::ValidatorId::from_bytes(idb), pk);
        i += 80;
    }
    let vrf_sks_bytes = std::fs::read(dir.join("vrf_secrets.bin")).unwrap_or_default();
    let mut vrf_secrets = Map::new();
    i = 0;
    while i + 80 <= vrf_sks_bytes.len() {
        let mut idb = [0u8; 48];
        idb.copy_from_slice(&vrf_sks_bytes[i..i + 48]);
        let mut sk = [0u8; 32];
        sk.copy_from_slice(&vrf_sks_bytes[i + 48..i + 80]);
        vrf_secrets.insert(types::ValidatorId::from_bytes(idb), sk);
        i += 80;
    }

    let mesh = ValidatorMesh::from_genesis(&cfg.genesis);
    let is_validator = mesh.is_validator(&our_id);
    let mut store = MemoryStore::new();
    init_store(&mut store, &cfg).unwrap();
    append_event(
        &dir,
        &format!("GENESIS {}", hex::encode(cfg.genesis.hash().as_bytes())),
    );
    append_event(
        &dir,
        &format!(
            "BOOT val={} nval={} vrf_pks={} id={}",
            mesh.is_validator(&our_id),
            mesh.validators.len(),
            vrf_pks.len(),
            hex::encode(our_id.as_bytes())
        ),
    );
    let mut live = Live {
        pool: Mempool::new(&cfg.genesis.params.registry),
        world: World::from_genesis(&cfg.genesis),
        replay: VoteReplayLog::new(),
        commits: CommitLog::new(),
        height: Height::GENESIS,
        round: Round::ZERO,
        last_hash: cfg.genesis.hash(),
        last_ts: 0,
        last_commit: Instant::now() - Duration::from_secs(2),
        pending: None,
        prevotes: Vec::new(),
        precommits: Vec::new(),
        clock: SystemClock,
        committed: Vec::new(),
        sync_offers: Map::new(),
        is_validator,
        started: Instant::now(),
        last_repub: Instant::now(),
        last_prop: Instant::now() - Duration::from_secs(1),
        mesh,
        store,
        cfg,
        bls_sk,
        our_id,
        vrf_sk,
        vrf_secrets,
        vrf_pks,
    };

    let mut outbox: Vec<(IdentTopic, Vec<u8>)> = Vec::new();
    let mut last_dial = Instant::now();
    loop {
        while let Some((t, data)) = outbox.pop() {
            let _ = swarm.behaviour_mut().gossipsub.publish(t, data);
        }
        if last_dial.elapsed() > Duration::from_millis(250) {
            if let Ok(boot) = load_bootstrap(&dir) {
                for (_, a) in boot.peers {
                    let _ = swarm.dial(a);
                }
            }
            last_dial = Instant::now();
        }
        if let Ok(SwarmEvent::Behaviour(network::gossip::L1Event::Gossipsub(
            libp2p::gossipsub::Event::Message { message, .. },
        ))) = tokio::time::timeout(Duration::from_millis(15), swarm.select_next_some()).await
        {
            live.handle_bytes(&message.data, &mut outbox, &dir);
        }
        live.tick(&mut outbox, &dir);
    }
}
