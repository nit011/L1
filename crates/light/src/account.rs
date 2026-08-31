//! Account proofs from untrusted RPC, checked against a QC-verified header
//! (architecture.md §4.1).
//!
//! `service.l1.jsonrpc.getProof` supplies bytes only. A successful RPC result
//! is never enough: the account trie root must reconstruct [`Header::state_root`]
//! (trusted after [`crate::verify_qc`]), then [`state::mpt::proof::verify`]
//! must succeed on that root.

use crate::header::verify_qc;
use crate::LightError;
use consensus::qc::QuorumCertificate;
use rpc::server::{decode_hex, encode_hex, RpcInner};
use rpc::state::{get_proof, proof_from_json, StateRpcError};
use serde_json::{json, Value};
use state::account::{account_key, Account};
use state::mpt::proof::verify as mpt_verify;
use types::collections::Map;
use types::header::Header;
use types::{Address, ValidatorId, VotingPower};

/// Untrusted `l1_getProof` source. Honest impls call [`get_proof`]; tests may
/// wrap it and still return JSON-RPC success with a forged proof.
pub trait GetProof {
    /// Same contract as `service.l1.jsonrpc.getProof`.
    fn get_proof(&self, params: &Value) -> Result<Value, StateRpcError>;
}

impl GetProof for RpcInner {
    fn get_proof(&self, params: &Value) -> Result<Value, StateRpcError> {
        get_proof(self, params)
    }
}

fn parse_root32(v: &Value, key: &str) -> Result<[u8; 32], LightError> {
    let s = v
        .get(key)
        .and_then(Value::as_str)
        .ok_or(LightError::Source)?;
    let b = decode_hex(s).map_err(|_| LightError::Source)?;
    if b.len() != 32 {
        return Err(LightError::Source);
    }
    let mut a = [0u8; 32];
    a.copy_from_slice(&b);
    Ok(a)
}

/// Fetch a proof from `source` and verify it against `header` (already QC-checked).
/// Contract: `light.verify_account`.
pub fn verify_account<S: GetProof>(
    header: &Header,
    qc: &QuorumCertificate,
    validators: &Map<ValidatorId, VotingPower>,
    source: &S,
    address: &Address,
) -> Result<Account, LightError> {
    verify_qc(header, qc, validators)?;
    let params = json!({
        "address": encode_hex(address.as_bytes()),
        "height": header.fields.height.0,
    });
    let resp = source.get_proof(&params).map_err(|_| LightError::Source)?;
    let _claimed_state = resp.get("stateRoot");
    let account_root = parse_root32(&resp, "accountRoot")?;
    let storage_root = parse_root32(&resp, "storageRoot")?;
    let bound = types::block::state_root(&account_root, &storage_root);
    if bound != header.state_root {
        return Err(LightError::Proof);
    }
    let proof_val = resp.get("proof").ok_or(LightError::Source)?;
    let proof = proof_from_json(proof_val).map_err(|_| LightError::Source)?;
    let key = account_key(address);
    if !mpt_verify(&key, &proof, &account_root) {
        return Err(LightError::Proof);
    }
    let raw = proof.value.ok_or(LightError::Proof)?;
    Account::decode(&raw).map_err(|_| LightError::Proof)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{header_with, keys_and_set, sign_header};
    use execution::seq::World;
    use network::discovery::BootstrapList;
    use network::identity;
    use node::config::NodeConfig;
    use types::genesis::{Genesis, GenesisAccount};
    use types::{Address, Amount, ChainId, Hash, Height, Nonce};

    struct Honest<'a>(&'a RpcInner);
    impl GetProof for Honest<'_> {
        fn get_proof(&self, params: &Value) -> Result<Value, StateRpcError> {
            get_proof(self.0, params)
        }
    }

    struct TamperProof<'a> {
        inner: &'a RpcInner,
        flip_node: bool,
        lie_account_root: bool,
    }
    impl GetProof for TamperProof<'_> {
        fn get_proof(&self, params: &Value) -> Result<Value, StateRpcError> {
            let mut v = get_proof(self.inner, params)?;
            if self.flip_node {
                let nodes = v["proof"]["nodes"].as_array_mut().unwrap();
                let s = nodes[0].as_str().unwrap().to_string();
                let mut raw = decode_hex(&s).unwrap();
                raw[0] ^= 0xff;
                nodes[0] = Value::String(encode_hex(&raw));
            }
            if self.lie_account_root {
                v["accountRoot"] = Value::String(encode_hex(&[0x11u8; 32]));
            }
            Ok(v)
        }
    }

    fn setup() -> (RpcInner, Address, SignedHeaderLike) {
        let addr = Address::from_bytes([7u8; 32]);
        let (keys, validators) = keys_and_set();
        let mut g = Genesis::new(ChainId::new(1));
        g.insert_alloc(
            addr,
            GenesisAccount {
                balance: Amount::new(42),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        for (_, id) in &keys {
            g.insert_validator(*id, VotingPower(10));
        }
        let inner = RpcInner::from_config(NodeConfig::new(
            g.clone(),
            BootstrapList::new(),
            identity::generate().unwrap(),
            std::path::PathBuf::from("/tmp/light-account"),
        ));
        let state_root = World::from_genesis(&g).commit_state_root();
        let header = header_with(Height::GENESIS, Hash::ZERO, state_root, ValidatorId::ZERO);
        let signed = sign_header(header, &keys, &validators);
        (inner, addr, SignedHeaderLike { signed })
    }

    struct SignedHeaderLike {
        signed: crate::fixtures::SignedHeader,
    }

    #[test]
    fn honest_rpc_proof_verifies_against_qc_header() {
        let (inner, addr, wrap) = setup();
        let acc = verify_account(
            &wrap.signed.header,
            &wrap.signed.qc,
            &wrap.signed.validators,
            &Honest(&inner),
            &addr,
        )
        .unwrap();
        assert_eq!(acc.balance, Amount::new(42));
    }

    #[test]
    fn tampered_proof_from_successful_rpc_is_rejected() {
        let (inner, addr, wrap) = setup();
        let src = TamperProof {
            inner: &inner,
            flip_node: true,
            lie_account_root: false,
        };
        let err = verify_account(
            &wrap.signed.header,
            &wrap.signed.qc,
            &wrap.signed.validators,
            &src,
            &addr,
        )
        .unwrap_err();
        assert_eq!(err, LightError::Proof);
    }

    #[test]
    fn rpc_account_root_not_bound_to_header_state_root_is_rejected() {
        let (inner, addr, wrap) = setup();
        let src = TamperProof {
            inner: &inner,
            flip_node: false,
            lie_account_root: true,
        };
        assert_eq!(
            verify_account(
                &wrap.signed.header,
                &wrap.signed.qc,
                &wrap.signed.validators,
                &src,
                &addr,
            ),
            Err(LightError::Proof)
        );
    }
}
