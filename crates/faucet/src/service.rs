//! Testnet faucet: fund an address with a native transfer.
//!
//! # Example
//!
//! ```no_run
//! use crypto::sig::ed25519::keygen;
//! use faucet::service::Faucet;
//! use types::{Address, Amount, ChainId};
//! # fn demo(inner: &mut rpc::server::RpcInner) {
//! let mut faucet = Faucet::new(keygen(), ChainId::new(1), Amount::new(1_000));
//! let hash = faucet.drip(inner, Address::ZERO).unwrap();
//! println!("{hash:?}");
//! # }
//! ```
//!
//! Builds [`types::tx::Tx::transfer`] (`tx.transfer`) and submits via
//! [`rpc::tx::submit_tx`] (`l1_submitTx`). Contract: `faucet.service`.

use crypto::address::from_ed25519;
use crypto::sig::ed25519::SecretKey;
use crypto::tx::sign;
use rpc::server::{encode_hex, RpcInner};
use rpc::tx::submit_tx;
use serde_json::json;
use storage::codec::encode_signed_tx;
use thiserror::Error;
use types::tx::Tx;
use types::{Address, Amount, ChainId, Hash, Nonce, GAS_TRANSFER, MIN_TX_FEE};

/// Faucet errors (RPC / transport).
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum FaucetError {
    /// JSON-RPC.
    #[error("rpc {0}")]
    Rpc(String),
    /// HTTP.
    #[error("http {0}")]
    Http(String),
}

/// Faucet-controlled funded account.
pub struct Faucet {
    /// Faucet spending key.
    pub sk: SecretKey,
    /// Chain id for `tx.transfer`.
    pub chain_id: ChainId,
    /// Amount sent per drip.
    pub amount: Amount,
    next_nonce: u64,
}

impl Faucet {
    /// New faucet; nonce starts at 0 (genesis account).
    pub fn new(sk: SecretKey, chain_id: ChainId, amount: Amount) -> Self {
        Self {
            sk,
            chain_id,
            amount,
            next_nonce: 0,
        }
    }

    /// Faucet address (`address.from_ed25519`).
    pub fn address(&self) -> Address {
        from_ed25519(&self.sk.verifying_key())
    }

    /// Build and sign a `tx.transfer` to `to`.
    pub fn signed_transfer(&self, to: Address, nonce: u64) -> types::tx::SignedTx {
        let tx = Tx::transfer(
            self.chain_id,
            Nonce(nonce),
            GAS_TRANSFER,
            Amount::new(MIN_TX_FEE),
            to,
            self.amount,
        );
        sign(&self.sk, tx)
    }

    /// Submit a drip through in-process `l1_submitTx`.
    pub fn drip(&mut self, inner: &mut RpcInner, to: Address) -> Result<Hash, FaucetError> {
        let signed = self.signed_transfer(to, self.next_nonce);
        let hex = encode_hex(&encode_signed_tx(&signed));
        let v = submit_tx(inner, &json!({"tx": hex}))
            .map_err(|e| FaucetError::Rpc(format!("{e:?}")))?;
        self.next_nonce += 1;
        hash_from(v)
    }

    /// Submit a drip through HTTP JSON-RPC (`l1_submitTx` on the server).
    pub fn drip_http(&mut self, url: &str, to: Address) -> Result<Hash, FaucetError> {
        let signed = self.signed_transfer(to, self.next_nonce);
        let hex = encode_hex(&encode_signed_tx(&signed));
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "l1_submitTx",
            "params": {"tx": hex},
        });
        let resp: serde_json::Value = ureq::post(url)
            .set("Content-Type", "application/json")
            .timeout(std::time::Duration::from_secs(5))
            .send_json(&body)
            .map_err(|e| FaucetError::Http(e.to_string()))?
            .into_json()
            .map_err(|e| FaucetError::Http(e.to_string()))?;
        if let Some(err) = resp.get("error") {
            return Err(FaucetError::Rpc(
                err.get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("error")
                    .to_string(),
            ));
        }
        self.next_nonce += 1;
        hash_from(
            resp.get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        )
    }
}

fn hash_from(v: serde_json::Value) -> Result<Hash, FaucetError> {
    let s = v
        .get("hash")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| FaucetError::Rpc("missing hash".into()))?;
    let raw = rpc::server::decode_hex(s).map_err(|_| FaucetError::Rpc("bad hash".into()))?;
    if raw.len() != 32 {
        return Err(FaucetError::Rpc("bad hash len".into()));
    }
    let mut a = [0u8; 32];
    a.copy_from_slice(&raw);
    Ok(Hash::from_bytes(a))
}

#[cfg(test)]
mod tests {
    use super::*;
    use consensus::propose::round_vrf_source;
    use consensus::vrf as cons_vrf;
    use crypto::from_bls;
    use crypto::sig::bls;
    use crypto::sig::ed25519::keygen;
    use crypto::vrf::public_key_from_seed;
    use mempool::Mempool;
    use network::discovery::BootstrapList;
    use network::identity;
    use node::config::NodeConfig;
    use node::wire::{init_store, wire_commit, wire_precommit, wire_propose, wire_vote, TraceSink};
    use rpc::state::get_account;
    use rpc::status::observe_finalized;
    use types::collections::Map;
    use types::genesis::{Genesis, GenesisAccount};
    use types::{Epoch, Height, Round, TestClock, ValidatorId, VotingPower};

    type Val = (blst::min_pk::SecretKey, ValidatorId, [u8; 32], [u8; 32]);

    fn setup() -> (
        RpcInner,
        Vec<Val>,
        Map<ValidatorId, [u8; 32]>,
        crypto::sig::ed25519::SecretKey,
        Address,
    ) {
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
        let mut inner = RpcInner::from_config(NodeConfig::new(
            g,
            BootstrapList::new(),
            identity::generate().unwrap(),
            std::path::PathBuf::from("/tmp/faucet-svc"),
        ));
        let cfg = inner.cfg.clone();
        init_store(&mut inner.store, &cfg).unwrap();
        (inner, keys, vrf_pks, faucet_sk, dest)
    }

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
        let pool = std::mem::replace(
            &mut inner.pool,
            Mempool::new(&inner.cfg.genesis.params.registry),
        );
        let mut pool = pool;
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
    fn drip_increases_balance_after_commit() {
        let (mut inner, keys, vrf_pks, faucet_sk, dest) = setup();
        let mut faucet = Faucet::new(faucet_sk, ChainId::new(1), Amount::new(42));
        faucet.drip(&mut inner, dest).unwrap();
        produce(&mut inner, &keys, &vrf_pks);
        let acc = get_account(&inner, &json!({"address": encode_hex(dest.as_bytes())})).unwrap();
        assert_eq!(acc["balance"], "42");
    }

    #[test]
    fn drip_to_unknown_rpc_shape_is_transfer() {
        let sk = keygen();
        let f = Faucet::new(sk, ChainId::new(1), Amount::new(1));
        let s = f.signed_transfer(Address::ZERO, 0);
        assert!(matches!(s.tx.payload, types::tx::TxPayload::Transfer(_)));
    }
}
