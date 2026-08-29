//! `l1_submitTx` / `l1_getTransaction`.

use crate::server::{decode_hex, encode_hex, RpcInner};
use network::rate_limit::PeerRateLimiter;
use node::wire::wire_mempool;
use serde_json::{json, Value};
use storage::codec::{decode_signed_tx, encode_signed_tx};
use storage::index::get_tx_by_hash;
use types::Hash;

/// Tx RPC errors.
#[derive(Debug)]
pub enum TxRpcError {
    /// Bad JSON / hex.
    Params,
    /// `netsec.peer_rate_limit`.
    RateLimit,
    /// `node.wire.mempool` rejected (validity lives there).
    Mempool,
    /// Unknown hash.
    Unknown,
}

/// `l1_submitTx`
///
/// Request: `{ "tx": "0x…" }` — hex of canonical `encode_signed_tx`.
/// Response: `{ "hash": "0x…" }`.
///
/// Rate-limited with [`PeerRateLimiter`] then [`wire_mempool`] (same path as gossip).
pub fn submit_tx(inner: &mut RpcInner, params: &Value) -> Result<Value, TxRpcError> {
    let _ = inner
        .cfg
        .genesis
        .params
        .registry
        .get(types::ParamId::MaxTxBytes);
    let tx_hex = params
        .get("tx")
        .and_then(Value::as_str)
        .ok_or(TxRpcError::Params)?;
    let bytes = decode_hex(tx_hex).map_err(|_| TxRpcError::Params)?;
    let signed = decode_signed_tx(&bytes).map_err(|_| TxRpcError::Params)?;
    if !inner.limiter.allow(&inner.cfg.identity.peer_id) {
        let _ = PeerRateLimiter::new();
        return Err(TxRpcError::RateLimit);
    }
    let account = match mempool::sender_address(&signed) {
        Ok(a) => inner.world.account(&a),
        Err(_) => state::account::Account::empty(),
    };
    wire_mempool(&signed, &account, &mut inner.pool).map_err(|_| TxRpcError::Mempool)?;
    let hash = storage::codec::tx_hash(&signed.tx);
    Ok(json!({"hash": encode_hex(hash.as_bytes())}))
}

/// `l1_getTransaction`
///
/// Request: `{ "hash": "0x…" }`.
/// Response: `{ "height", "index", "tx" }` or error if missing.
/// Read-only [`get_tx_by_hash`] (`store.tx.by_hash`).
pub fn get_tx(inner: &RpcInner, params: &Value) -> Result<Value, TxRpcError> {
    let h = params
        .get("hash")
        .and_then(Value::as_str)
        .ok_or(TxRpcError::Params)?;
    let raw = decode_hex(h).map_err(|_| TxRpcError::Params)?;
    if raw.len() != 32 {
        return Err(TxRpcError::Params);
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&raw);
    let hash = Hash::from_bytes(arr);
    let Some((height, index, signed)) =
        get_tx_by_hash(&inner.store, &hash).map_err(|_| TxRpcError::Unknown)?
    else {
        return Err(TxRpcError::Unknown);
    };
    Ok(json!({
        "height": height.0,
        "index": index,
        "tx": encode_hex(&encode_signed_tx(&signed)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{dispatch, RpcInner};
    use crypto::from_ed25519;
    use crypto::sig::ed25519::SecretKey as EdSk;
    use crypto::tx::sign;
    use network::discovery::BootstrapList;
    use network::identity;
    use node::config::NodeConfig;
    use serde_json::json;
    use state::account::Account;
    use storage::blocks::put_block;
    use types::block::Block;
    use types::genesis::{Genesis, GenesisAccount};
    use types::header::{Header, HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::tx::Tx;
    use types::{Amount, ChainId, Nonce, TestClock, ValidatorId, GAS_TRANSFER};

    fn cfg_funded(from: types::Address) -> NodeConfig {
        let mut g = Genesis::new(ChainId::new(1));
        g.insert_alloc(
            from,
            GenesisAccount {
                balance: Amount::new(1_000_000),
                nonce: Nonce::ZERO,
                code_hash: types::Hash::ZERO,
            },
        );
        NodeConfig::new(
            g,
            BootstrapList::new(),
            identity::generate().unwrap(),
            std::path::PathBuf::from("/tmp/rpc-tx"),
        )
    }

    #[test]
    fn submit_happy_and_bad_params() {
        let ska = EdSk::from_bytes(&[3u8; 32]);
        let from = from_ed25519(&ska.verifying_key());
        let mut inner = RpcInner::from_config(cfg_funded(from));
        let tx = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            from,
            Amount::new(10),
        );
        let signed = sign(&ska, tx);
        let hex = encode_hex(&encode_signed_tx(&signed));
        let ok = dispatch(&mut inner, "l1_submitTx", &json!({"tx": hex})).unwrap();
        assert!(ok.get("hash").is_some());
        let bad = dispatch(&mut inner, "l1_submitTx", &json!({})).unwrap_err();
        assert_eq!(bad.code, -32602);
        let mut bad_sig = signed.clone();
        bad_sig.signature[0] ^= 1;
        let rej = dispatch(
            &mut inner,
            "l1_submitTx",
            &json!({"tx": encode_hex(&encode_signed_tx(&bad_sig))}),
        )
        .unwrap_err();
        assert_eq!(rej.code, -32000);
    }

    #[test]
    fn submit_flood_is_rate_limited() {
        let ska = EdSk::from_bytes(&[4u8; 32]);
        let from = from_ed25519(&ska.verifying_key());
        let mut inner = RpcInner::from_config(cfg_funded(from));
        let tx = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            from,
            Amount::new(1),
        );
        let signed = sign(&ska, tx);
        let hex = encode_hex(&encode_signed_tx(&signed));
        let cap = network::rate_limit::peer_msg_limit();
        let mut saw_limit = false;
        for _ in 0..cap.saturating_add(2) {
            match dispatch(&mut inner, "l1_submitTx", &json!({"tx": hex.clone()})) {
                Err(e) if e.message.contains("rate limited") => {
                    saw_limit = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(saw_limit);
    }

    #[test]
    fn get_tx_from_store_index() {
        let ska = EdSk::from_bytes(&[5u8; 32]);
        let from = from_ed25519(&ska.verifying_key());
        let mut inner = RpcInner::from_config(cfg_funded(from));
        let tx = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            from,
            Amount::new(1),
        );
        let signed = sign(&ska, tx);
        let h = storage::codec::tx_hash(&signed.tx);
        let clock = TestClock::new(1_000);
        let fields = HeaderFields::new(
            &clock,
            types::Height::GENESIS,
            types::Round::ZERO,
            ValidatorId::ZERO,
            0,
            1,
        )
        .unwrap();
        let header = Header {
            fields: fields.clone(),
            tx_root: types::Hash::ZERO,
            state_root: types::Hash::ZERO,
            receipts_root: types::Hash::ZERO,
            validators_hash: types::Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        };
        let block = Block {
            header_fields: fields,
            txs: vec![signed.clone()],
        };
        put_block(
            &mut inner.store,
            &header,
            &block,
            &[vec![]],
            &types::Hash::ZERO,
        )
        .unwrap();
        let got = dispatch(
            &mut inner,
            "l1_getTransaction",
            &json!({"hash": encode_hex(h.as_bytes())}),
        )
        .unwrap();
        assert_eq!(got["height"], 0);
        let miss = dispatch(
            &mut inner,
            "l1_getTransaction",
            &json!({"hash": encode_hex(&[9u8; 32])}),
        )
        .unwrap_err();
        assert!(miss.message.contains("unknown"));
        let _ = Account::empty();
    }
}
