//! Fetch an account Merkle proof from the node.
//!
//! # Trust model (explicit)
//!
//! **This function does not independently verify the proof.** It returns the
//! JSON from [`rpc::state::get_proof`] (`l1_getProof`) as-is. That matches a
//! **trusted-node** SDK (local devnet / your own RPC), not a public light
//! client. Callers who need cryptographic verification against a QC-checked
//! header must use crate `light` (`light.verify_account`) — this module
//! does **not** call it.
//!
//! Optionally waits for a finalized height via [`crate::wait_status_finality`]
//! when `wait` is `Some`.
//!
//! # Example
//!
//! ```no_run
//! use sdk::query_proof;
//! # use rpc::server::RpcInner;
//! # use types::Address;
//! # fn demo(inner: &mut RpcInner, addr: Address) {
//! let proof = query_proof(inner, &addr, None).unwrap();
//! println!("{}", proof);
//! # }
//! ```
//!
//! Contract: `sdk.query_proof`.

use crate::finality::{wait_status_finality, WaitError};
use crate::sign::SignedFrom;
use crate::submit::{rpc_call, SdkError};
use rpc::server::{encode_hex, RpcInner};
use rpc::state::get_proof;
use serde_json::{json, Value};
use std::time::Duration;
use types::Address;
use types::Hash;

/// Fetch `l1_getProof` for `address` (pass-through, **not** `light.verify_account`).
pub fn query_proof(
    inner: &mut RpcInner,
    address: &Address,
    wait: Option<(SignedFrom, Hash, Duration)>,
) -> Result<Value, WaitError> {
    if let Some((signed, hash, timeout)) = wait {
        wait_status_finality(inner, signed, hash, timeout)?;
    }
    get_proof(inner, &json!({"address": encode_hex(address.as_bytes())})).map_err(|e| {
        WaitError::Sdk(SdkError::Rpc {
            code: -32000,
            message: format!("{e:?}"),
        })
    })
}

/// HTTP pass-through of `l1_getProof`.
pub fn query_proof_http(url: &str, address: &Address) -> Result<Value, SdkError> {
    rpc_call(
        url,
        "l1_getProof",
        json!({"address": encode_hex(address.as_bytes())}),
    )
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
    use types::{Amount, ChainId, Nonce};

    #[test]
    fn pass_through_does_not_verify_and_returns_rpc_json() {
        let src = include_str!("proof.rs");
        let impl_src = src.split("#[cfg(test)]").next().unwrap();
        assert!(
            impl_src.contains("does not independently verify"),
            "trust-model doc must stay on query_proof"
        );
        assert!(
            !impl_src.contains("use light") && !impl_src.contains("verify_account("),
            "must not import or call light.verify_account (decision b)"
        );
        assert!(!impl_src.contains("mpt_verify") && !impl_src.contains("mpt::proof::verify"));

        let sk = keygen();
        let addr = from_ed25519(&sk.verifying_key());
        let mut g = Genesis::new(ChainId::new(1));
        g.insert_alloc(
            addr,
            GenesisAccount {
                balance: Amount::new(9),
                nonce: Nonce::ZERO,
                code_hash: types::Hash::ZERO,
            },
        );
        let mut inner = RpcInner::from_config(NodeConfig::new(
            g,
            BootstrapList::new(),
            identity::generate().unwrap(),
            std::path::PathBuf::from("/tmp/sdk-proof"),
        ));
        let v = query_proof(&mut inner, &addr, None).unwrap();
        assert!(v.get("proof").is_some());
        assert!(v.get("accountRoot").is_some());
        // Tampering the returned JSON is the caller's problem: we do not reject it.
        let mut tampered = v.clone();
        if let Some(serde_json::Value::Array(nodes)) = tampered.pointer_mut("/proof/nodes") {
            if let Some(serde_json::Value::String(s)) = nodes.first_mut() {
                s.push('0');
            }
        }
        assert_ne!(tampered, v);
        let again = query_proof(&mut inner, &addr, None).unwrap();
        assert_eq!(again["accountRoot"], v["accountRoot"]);
    }
}
