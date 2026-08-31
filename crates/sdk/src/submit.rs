//! Submit a signed transaction through JSON-RPC.
//!
//! # Example
//!
//! ```no_run
//! use crypto::sig::ed25519::keygen;
//! use sdk::submit::submit_signed_http;
//! use sdk::sign_tx;
//! use types::tx::Tx;
//! use types::{Address, Amount, ChainId, Nonce, GAS_TRANSFER};
//!
//! let sk = keygen();
//! let tx = Tx::transfer(
//!     ChainId::new(1),
//!     Nonce::ZERO,
//!     GAS_TRANSFER,
//!     Amount::new(1),
//!     Address::ZERO,
//!     Amount::new(10),
//! );
//! let signed = sign_tx(&sk, tx);
//! let hash = submit_signed_http("http://127.0.0.1:8545/", &signed.signed).unwrap();
//! println!("hash {hash:?}");
//! ```
//!
//! RPC application errors (mempool rejection, rate limit, bad params) are
//! returned as [`SdkError::Rpc`] with the **server message**, not a generic
//! "submission failed." Contract: `sdk.submit`.

use crate::sign::{sign_tx, SignedFrom};
use crypto::sig::ed25519::SecretKey;
use rpc::server::{encode_hex, RpcInner};
use rpc::tx::submit_tx;
use serde_json::{json, Value};
use storage::codec::encode_signed_tx;
use thiserror::Error;
use types::tx::{SignedTx, Tx};
use types::Hash;

/// SDK / JSON-RPC client errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SdkError {
    /// HTTP or JSON transport.
    #[error("transport: {0}")]
    Transport(String),
    /// JSON-RPC error object (`code` + `message` from the node).
    #[error("rpc {code}: {message}")]
    Rpc {
        /// JSON-RPC error code.
        code: i64,
        /// Server message (e.g. `mempool rejected`).
        message: String,
    },
    /// Response JSON missing a required field.
    #[error("bad rpc result")]
    BadResult,
}

/// POST one JSON-RPC 2.0 method to a live HTTP endpoint (`rpc.server`).
pub fn rpc_call(url: &str, method: &str, params: Value) -> Result<Value, SdkError> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let resp: Value = ureq::post(url)
        .set("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(5))
        .send_json(&body)
        .map_err(|e| SdkError::Transport(e.to_string()))?
        .into_json()
        .map_err(|e| SdkError::Transport(e.to_string()))?;
    if let Some(err) = resp.get("error") {
        return Err(SdkError::Rpc {
            code: err.get("code").and_then(Value::as_i64).unwrap_or(-1),
            message: err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("error")
                .to_string(),
        });
    }
    resp.get("result").cloned().ok_or(SdkError::BadResult)
}

fn hash_from_submit_result(v: &Value) -> Result<Hash, SdkError> {
    let s = v
        .get("hash")
        .and_then(Value::as_str)
        .ok_or(SdkError::BadResult)?;
    let raw = rpc::server::decode_hex(s).map_err(|_| SdkError::BadResult)?;
    if raw.len() != 32 {
        return Err(SdkError::BadResult);
    }
    let mut a = [0u8; 32];
    a.copy_from_slice(&raw);
    Ok(Hash::from_bytes(a))
}

/// Sign with [`sign_tx`] then [`submit_tx`] (`l1_submitTx`) on an in-process node.
pub fn submit(
    inner: &mut RpcInner,
    sk: &SecretKey,
    tx: Tx,
) -> Result<(SignedFrom, Hash), SdkError> {
    let signed = sign_tx(sk, tx);
    let hash = submit_signed(inner, &signed.signed)?;
    Ok((signed, hash))
}

/// Submit an already-signed envelope via [`rpc::tx::submit_tx`].
pub fn submit_signed(inner: &mut RpcInner, signed: &SignedTx) -> Result<Hash, SdkError> {
    let hex = encode_hex(&encode_signed_tx(signed));
    match submit_tx(inner, &json!({"tx": hex})) {
        Ok(v) => hash_from_submit_result(&v),
        Err(e) => {
            let rpc_err = rpc::server::RpcError::from(e);
            Err(SdkError::Rpc {
                code: rpc_err.code,
                message: rpc_err.message,
            })
        }
    }
}

/// HTTP equivalent: still `l1_submitTx` on the server.
pub fn submit_signed_http(url: &str, signed: &SignedTx) -> Result<Hash, SdkError> {
    let hex = encode_hex(&encode_signed_tx(signed));
    let v = rpc_call(url, "l1_submitTx", json!({"tx": hex}))?;
    hash_from_submit_result(&v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::address::from_ed25519;
    use crypto::sig::ed25519::keygen;
    use network::discovery::BootstrapList;
    use network::identity;
    use node::config::NodeConfig;
    use types::genesis::{Genesis, GenesisAccount};
    use types::{Amount, ChainId, Nonce, GAS_TRANSFER};

    fn inner_with(sk: &SecretKey, balance: u128, nonce: Nonce) -> RpcInner {
        let from = from_ed25519(&sk.verifying_key());
        let mut g = Genesis::new(ChainId::new(1));
        g.insert_alloc(
            from,
            GenesisAccount {
                balance: Amount::new(balance),
                nonce,
                code_hash: types::Hash::ZERO,
            },
        );
        let cfg = NodeConfig::new(
            g,
            BootstrapList::new(),
            identity::generate().unwrap(),
            std::path::PathBuf::from("/tmp/sdk-submit"),
        );
        RpcInner::from_config(cfg)
    }

    #[test]
    fn happy_submit_returns_hash() {
        let sk = keygen();
        let mut inner = inner_with(&sk, 1_000_000, Nonce::ZERO);
        let tx = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            types::Address::ZERO,
            Amount::new(10),
        );
        let (_, h) = submit(&mut inner, &sk, tx).unwrap();
        assert_ne!(h, Hash::ZERO);
    }

    #[test]
    fn bad_nonce_surfaces_mempool_rejected_not_generic() {
        let sk = keygen();
        let mut inner = inner_with(&sk, 1_000_000, Nonce(5));
        let tx = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            types::Address::ZERO,
            Amount::new(1),
        );
        let err = submit(&mut inner, &sk, tx).unwrap_err();
        match err {
            SdkError::Rpc { message, .. } => {
                assert!(
                    message.contains("mempool"),
                    "expected RPC mempool reason, got {message}"
                );
                assert!(!message.eq_ignore_ascii_case("submission failed"));
            }
            other => panic!("expected Rpc, got {other:?}"),
        }
    }
}
