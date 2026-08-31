//! Wait until JSON-RPC status reports a finalized height.
//!
//! # Example
//!
//! ```no_run
//! use std::time::Duration;
//! use sdk::wait_finality;
//! # use rpc::server::RpcInner;
//! # fn demo(inner: &mut RpcInner, sk: &crypto::sig::ed25519::SecretKey, tx: types::tx::Tx) {
//! let rec = wait_finality(inner, sk, tx, Duration::from_secs(15)).unwrap();
//! println!("included by height {}", rec.height);
//! # }
//! ```
//!
//! Polls [`rpc::status::get_status`] (`l1_getStatus`) after [`crate::submit`].
//! A rejected tx fails at submit (specific RPC error). A submitted tx that is
//! never committed hits [`WaitError::Timeout`] instead of blocking forever.
//! Contract: `sdk.wait_finality`.

use crate::sign::SignedFrom;
use crate::submit::{rpc_call, submit, SdkError};
use crypto::sig::ed25519::SecretKey;
use rpc::server::RpcInner;
use rpc::status::get_status;
use serde_json::json;
use std::time::{Duration, Instant};
use thiserror::Error;
use types::tx::Tx;
use types::Hash;

/// Outcome of waiting for finality.
#[derive(Clone, Debug)]
pub struct FinalityRecord {
    /// Tx hash from `l1_submitTx`.
    pub tx_hash: Hash,
    /// Sender + signed envelope.
    pub signed: SignedFrom,
    /// `l1_getStatus` height when the wait completed.
    pub height: u64,
    /// Wall time spent polling.
    pub waited: Duration,
}

/// Wait errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WaitError {
    /// Submit / RPC.
    #[error(transparent)]
    Sdk(#[from] SdkError),
    /// Status never showed a committed height before `timeout`.
    #[error("finality timeout after {0:?}")]
    Timeout(Duration),
}

fn height_from_status(v: &serde_json::Value) -> Option<u64> {
    v.get("height").and_then(serde_json::Value::as_u64)
}

/// Submit then poll in-process [`get_status`] until `height` is present.
pub fn wait_finality(
    inner: &mut RpcInner,
    sk: &SecretKey,
    tx: Tx,
    timeout: Duration,
) -> Result<FinalityRecord, WaitError> {
    let (signed, tx_hash) = submit(inner, sk, tx)?;
    wait_status_finality(inner, signed, tx_hash, timeout)
}

/// Poll [`get_status`] only (tx already submitted).
pub fn wait_status_finality(
    inner: &mut RpcInner,
    signed: SignedFrom,
    tx_hash: Hash,
    timeout: Duration,
) -> Result<FinalityRecord, WaitError> {
    let t0 = Instant::now();
    loop {
        let st = get_status(inner);
        if let Some(h) = height_from_status(&st) {
            return Ok(FinalityRecord {
                tx_hash,
                signed,
                height: h,
                waited: t0.elapsed(),
            });
        }
        if t0.elapsed() >= timeout {
            return Err(WaitError::Timeout(timeout));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// HTTP: poll `l1_getStatus` until a height is reported.
pub fn wait_status_http(
    url: &str,
    signed: SignedFrom,
    tx_hash: Hash,
    timeout: Duration,
) -> Result<FinalityRecord, WaitError> {
    let t0 = Instant::now();
    loop {
        let st = rpc_call(url, "l1_getStatus", json!({}))?;
        if let Some(h) = height_from_status(&st) {
            return Ok(FinalityRecord {
                tx_hash,
                signed,
                height: h,
                waited: t0.elapsed(),
            });
        }
        if t0.elapsed() >= timeout {
            return Err(WaitError::Timeout(timeout));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::submit::submit;
    use crypto::address::from_ed25519;
    use crypto::sig::ed25519::keygen;
    use network::discovery::BootstrapList;
    use network::identity;
    use node::config::NodeConfig;
    use types::genesis::{Genesis, GenesisAccount};
    use types::tx::Tx;
    use types::{Amount, ChainId, Nonce, GAS_TRANSFER};

    fn inner_funded(sk: &SecretKey, nonce: Nonce) -> RpcInner {
        let from = from_ed25519(&sk.verifying_key());
        let mut g = Genesis::new(ChainId::new(1));
        g.insert_alloc(
            from,
            GenesisAccount {
                balance: Amount::new(1_000_000),
                nonce,
                code_hash: types::Hash::ZERO,
            },
        );
        RpcInner::from_config(NodeConfig::new(
            g,
            BootstrapList::new(),
            identity::generate().unwrap(),
            std::path::PathBuf::from("/tmp/sdk-fin"),
        ))
    }

    #[test]
    fn rejected_nonce_does_not_hang() {
        let sk = keygen();
        let mut inner = inner_funded(&sk, Nonce(5));
        let tx = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            types::Address::ZERO,
            Amount::new(1),
        );
        let err = wait_finality(&mut inner, &sk, tx, Duration::from_secs(2)).unwrap_err();
        match err {
            WaitError::Sdk(SdkError::Rpc { message, .. }) => {
                assert!(message.contains("mempool"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn never_committed_times_out() {
        let sk = keygen();
        let mut inner = inner_funded(&sk, Nonce::ZERO);
        let tx = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            types::Address::ZERO,
            Amount::new(1),
        );
        let (signed, hash) = submit(&mut inner, &sk, tx).unwrap();
        let err =
            wait_status_finality(&mut inner, signed, hash, Duration::from_millis(40)).unwrap_err();
        assert!(matches!(err, WaitError::Timeout(_)));
    }
}
