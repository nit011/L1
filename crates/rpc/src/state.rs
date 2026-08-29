//! `l1_getAccount` / `l1_getProof`.

use crate::server::{decode_hex, encode_hex, RpcInner};
use serde_json::{json, Value};
use state::account::account_key;
use state::mpt::proof::{prove, MptProof};
use storage::blocks::{get_header, tip};
use types::{Address, Height};

/// State RPC errors.
#[derive(Debug)]
pub enum StateRpcError {
    /// Bad params.
    Params,
    /// Missing account/block.
    Unknown,
    /// `mpt.prove` failed.
    Proof,
}

fn parse_addr(params: &Value) -> Result<Address, StateRpcError> {
    let s = params
        .get("address")
        .and_then(Value::as_str)
        .ok_or(StateRpcError::Params)?;
    let raw = decode_hex(s).map_err(|_| StateRpcError::Params)?;
    if raw.len() != 32 {
        return Err(StateRpcError::Params);
    }
    let mut a = [0u8; 32];
    a.copy_from_slice(&raw);
    Ok(Address::from_bytes(a))
}

/// `l1_getAccount`
///
/// Request: `{ "address": "0x…" }`.
/// Response: `{ "balance", "nonce", "codeHash" }` or error if absent from the trie.
/// Reads [`state::tries::AccountTrie::get`] only.
pub fn get_account(inner: &RpcInner, params: &Value) -> Result<Value, StateRpcError> {
    let addr = parse_addr(params)?;
    let Some(acc) = inner.world.accounts.get(&addr) else {
        return Err(StateRpcError::Unknown);
    };
    Ok(json!({
        "balance": acc.balance.0.to_string(),
        "nonce": acc.nonce.0,
        "codeHash": encode_hex(acc.code_hash.as_bytes()),
    }))
}

fn proof_json(p: &MptProof) -> Value {
    json!({
        "nodes": p.nodes.iter().map(|n| encode_hex(n)).collect::<Vec<_>>(),
        "value": p.value.as_ref().map(|v| encode_hex(v)),
        "chainMerkleRoot": encode_hex(&p.chain_merkle_root),
        "chainMerkle": {
            "leafIndex": p.chain_merkle.leaf_index,
            "siblings": p.chain_merkle.siblings.iter().map(|s| encode_hex(s)).collect::<Vec<_>>(),
        }
    })
}

/// Reconstruct [`MptProof`] from JSON (tests / independent verify).
pub fn proof_from_json(v: &Value) -> Result<MptProof, StateRpcError> {
    let nodes = v
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or(StateRpcError::Params)?;
    let mut out_nodes = Vec::new();
    for n in nodes {
        let s = n.as_str().ok_or(StateRpcError::Params)?;
        out_nodes.push(decode_hex(s).map_err(|_| StateRpcError::Params)?);
    }
    let value = match v.get("value") {
        None | Some(Value::Null) => None,
        Some(x) => {
            let s = x.as_str().ok_or(StateRpcError::Params)?;
            Some(decode_hex(s).map_err(|_| StateRpcError::Params)?)
        }
    };
    let root_s = v
        .get("chainMerkleRoot")
        .and_then(Value::as_str)
        .ok_or(StateRpcError::Params)?;
    let rb = decode_hex(root_s).map_err(|_| StateRpcError::Params)?;
    if rb.len() != 32 {
        return Err(StateRpcError::Params);
    }
    let mut chain_merkle_root = [0u8; 32];
    chain_merkle_root.copy_from_slice(&rb);
    let cm = v.get("chainMerkle").ok_or(StateRpcError::Params)?;
    let leaf_index = cm
        .get("leafIndex")
        .and_then(Value::as_u64)
        .ok_or(StateRpcError::Params)? as usize;
    let sibs = cm
        .get("siblings")
        .and_then(Value::as_array)
        .ok_or(StateRpcError::Params)?;
    let mut siblings = Vec::new();
    for s in sibs {
        let b = decode_hex(s.as_str().ok_or(StateRpcError::Params)?)
            .map_err(|_| StateRpcError::Params)?;
        if b.len() != 32 {
            return Err(StateRpcError::Params);
        }
        let mut a = [0u8; 32];
        a.copy_from_slice(&b);
        siblings.push(a);
    }
    Ok(MptProof {
        nodes: out_nodes,
        value,
        chain_merkle: state::merkle::MerkleProof {
            leaf_index,
            siblings,
        },
        chain_merkle_root,
    })
}

/// `l1_getProof`
///
/// Request: `{ "address": "0x…", "height"?: n }`.
/// Response: `{ "stateRoot", "accountRoot", "storageRoot", "proof" }`.
///
/// Proof nodes come from [`prove`] (`mpt.prove`) on the account trie.
/// `stateRoot` is the block header field (`block.state_root`).
pub fn get_proof(inner: &RpcInner, params: &Value) -> Result<Value, StateRpcError> {
    let addr = parse_addr(params)?;
    let height = if let Some(h) = params.get("height").and_then(Value::as_u64) {
        Height(h)
    } else {
        tip(&inner.store)
            .map_err(|_| StateRpcError::Unknown)?
            .unwrap_or(Height::GENESIS)
    };
    let header = get_header(&inner.store, height).ok().flatten();
    let state_root = header
        .as_ref()
        .map(|h| h.state_root)
        .unwrap_or_else(|| inner.world.commit_state_root());
    let _ = types::block::state_root(&inner.world.accounts.root(), &inner.world.storage.root());
    let key = account_key(&addr);
    let proof = prove(inner.world.accounts.as_trie(), &key).ok_or(StateRpcError::Proof)?;
    Ok(json!({
        "height": height.0,
        "stateRoot": encode_hex(state_root.as_bytes()),
        "accountRoot": encode_hex(&inner.world.accounts.root()),
        "storageRoot": encode_hex(&inner.world.storage.root()),
        "proof": proof_json(&proof),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{dispatch, RpcInner};
    use network::discovery::BootstrapList;
    use network::identity;
    use node::config::NodeConfig;
    use serde_json::json;
    use state::account::Account;
    use state::mpt::proof::verify;
    use types::genesis::{Genesis, GenesisAccount};
    use types::{Amount, ChainId, Nonce};

    fn inner_with_alloc(addr: Address) -> RpcInner {
        let mut g = Genesis::new(ChainId::new(1));
        g.insert_alloc(
            addr,
            GenesisAccount {
                balance: Amount::new(42),
                nonce: Nonce::ZERO,
                code_hash: types::Hash::ZERO,
            },
        );
        RpcInner::from_config(NodeConfig::new(
            g,
            BootstrapList::new(),
            identity::generate().unwrap(),
            std::path::PathBuf::from("/tmp/rpc-state"),
        ))
    }

    #[test]
    fn get_account_and_missing() {
        let addr = Address::from_bytes([7u8; 32]);
        let mut inner = inner_with_alloc(addr);
        let got = dispatch(
            &mut inner,
            "l1_getAccount",
            &json!({"address": encode_hex(addr.as_bytes())}),
        )
        .unwrap();
        assert_eq!(got["nonce"], 0);
        let miss = dispatch(
            &mut inner,
            "l1_getAccount",
            &json!({"address": encode_hex(&[8u8; 32])}),
        )
        .unwrap_err();
        assert!(miss.message.contains("unknown"));
    }

    #[test]
    fn get_proof_verifies_independently() {
        let addr = Address::from_bytes([7u8; 32]);
        let mut inner = inner_with_alloc(addr);
        let resp = dispatch(
            &mut inner,
            "l1_getProof",
            &json!({"address": encode_hex(addr.as_bytes())}),
        )
        .unwrap();
        let proof_val = resp["proof"].clone();
        let account_root_hex = resp["accountRoot"].as_str().unwrap().to_string();
        drop(inner);
        let proof = proof_from_json(&proof_val).unwrap();
        let root_bytes = decode_hex(&account_root_hex).unwrap();
        let mut root = [0u8; 32];
        root.copy_from_slice(&root_bytes);
        let key = account_key(&addr);
        assert!(verify(&key, &proof, &root));
        let _ = Account::empty();
    }
}
