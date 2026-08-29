//! `l1_getStatus`.

use crate::server::{encode_hex, RpcInner};
use consensus::steps::Finalized;
use serde_json::{json, Value};

/// `l1_getStatus`
///
/// Request: `{}`.
/// Response: `{ "height", "round", "syncing", "peerCount", "blockHash" }`.
///
/// Height and round are copied from the last [`Finalized`] produced by
/// `cons.commit` (via `node.wire.commit`). This module does not increment a counter.
pub fn get_status(inner: &RpcInner) -> Value {
    let peer_count = inner.cfg.bootstrap.peers.len() as u64;
    match &inner.last_finalized {
        Some(f) => json!({
            "height": f.height.0,
            "round": f.round.0,
            "syncing": false,
            "peerCount": peer_count,
            "blockHash": encode_hex(f.block_hash.as_bytes()),
            "appHash": encode_hex(f.app_hash.as_bytes()),
        }),
        None => json!({
            "height": serde_json::Value::Null,
            "round": 0,
            "syncing": true,
            "peerCount": peer_count,
            "blockHash": serde_json::Value::Null,
        }),
    }
}

/// Record a [`Finalized`] from `cons.commit` / `wire_commit` (node wiring).
pub fn observe_finalized(inner: &mut RpcInner, f: Finalized) {
    inner.last_finalized = Some(f);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{dispatch, RpcInner};
    use consensus::propose::round_vrf_source;
    use consensus::vrf as cons_vrf;
    use crypto::from_bls;
    use crypto::sig::bls;
    use crypto::vrf::public_key_from_seed;
    use execution::seq::World;
    use mempool::Mempool;
    use network::discovery::BootstrapList;
    use network::identity;
    use node::config::NodeConfig;
    use node::wire::{init_store, wire_commit, wire_precommit, wire_propose, wire_vote, TraceSink};
    use serde_json::json;
    use types::collections::Map;
    use types::genesis::Genesis;
    use types::{ChainId, Epoch, Height, Round, TestClock, ValidatorId, VotingPower};

    #[allow(clippy::type_complexity)]
    fn four_keys(
        genesis: &mut Genesis,
    ) -> (
        Vec<(blst::min_pk::SecretKey, ValidatorId, [u8; 32], [u8; 32])>,
        Map<ValidatorId, [u8; 32]>,
    ) {
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
        (keys, vrf_pks)
    }

    #[test]
    fn status_before_commit_is_syncing() {
        let cfg = NodeConfig::new(
            Genesis::new(ChainId::new(1)),
            BootstrapList::new(),
            identity::generate().unwrap(),
            std::path::PathBuf::from("/tmp/rpc-st"),
        );
        let mut inner = RpcInner::from_config(cfg);
        let s = dispatch(&mut inner, "l1_getStatus", &json!({})).unwrap();
        assert_eq!(s["syncing"], true);
        assert!(s["height"].is_null());
    }

    #[test]
    fn status_height_follows_cons_commit_not_rpc_counter() {
        let mut genesis = Genesis::new(ChainId::new(1));
        let (keys, vrf_pks) = four_keys(&mut genesis);
        let cfg = NodeConfig::new(
            genesis,
            BootstrapList::new(),
            identity::generate().unwrap(),
            std::path::PathBuf::from("/tmp/rpc-st2"),
        );
        let mut inner = RpcInner::from_config(cfg.clone());
        init_store(&mut inner.store, &cfg).unwrap();
        let src = round_vrf_source(&cfg.genesis.validators, Round::ZERO).unwrap();
        let src_row = keys.iter().find(|k| k.1 == src).unwrap();
        let seed = cons_vrf::derive_seed(&[1u8; 32], Epoch::ZERO);
        let (_, proof) = cons_vrf::leader_prove(&src_row.2, &seed, &src).unwrap();
        let winner =
            cons_vrf::weighted_leader(&src_row.3, &seed, &src, &proof, &cfg.genesis.validators)
                .unwrap();
        let w = keys.iter().find(|k| k.1 == winner).unwrap();
        let clock = TestClock::new(1_000);
        let mut pool = Mempool::new(&cfg.genesis.params.registry);
        let mut sink = TraceSink::default();
        let mesh = network::validator_mesh::ValidatorMesh::from_genesis(&cfg.genesis);
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
        let mut log = consensus::vote::VoteReplayLog::new();
        let mut prevotes = Vec::new();
        for k in &keys {
            prevotes.push(
                wire_vote(
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
                .unwrap(),
            );
        }
        let mut precommits = Vec::new();
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
        let rec: Vec<_> = built.receipts.iter().map(|r| r.encode()).collect();
        let f = wire_commit(
            &precommits,
            &cfg.genesis.validators,
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
        observe_finalized(&mut inner, f.clone());
        let s = dispatch(&mut inner, "l1_getStatus", &json!({})).unwrap();
        assert_eq!(s["height"], f.height.0);
        assert_eq!(s["round"], f.round.0);
        assert_eq!(s["syncing"], false);
        assert_eq!(storage::blocks::tip(&inner.store).unwrap(), Some(f.height));
    }
}
