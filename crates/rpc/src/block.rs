//! `l1_getBlock`.

use crate::server::{decode_hex, encode_hex, RpcInner};
use serde_json::{json, Value};
use storage::blocks::{get_block as store_get_block, get_header, height_by_header_hash, tip};
use storage::codec::encode_block_body;
use types::{Hash, Height};

/// Block lookup errors.
#[derive(Debug)]
pub enum GetBlockError {
    /// Bad params.
    Params,
    /// Missing.
    Unknown,
}

/// `l1_getBlock`
///
/// Request: `{ "height": n }` or `{ "hash": "0x…" }`.
/// Response: `{ "height", "hash", "headerHash", "body" }` (body is canonical hex).
/// Storage written by `store.block.put`.
pub fn get_block(inner: &RpcInner, params: &Value) -> Result<Value, GetBlockError> {
    let height = if let Some(h) = params.get("height").and_then(Value::as_u64) {
        Height(h)
    } else if let Some(hs) = params.get("hash").and_then(Value::as_str) {
        let raw = decode_hex(hs).map_err(|_| GetBlockError::Params)?;
        if raw.len() != 32 {
            return Err(GetBlockError::Params);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&raw);
        let hash = Hash::from_bytes(arr);
        height_by_header_hash(&inner.store, &hash)
            .map_err(|_| GetBlockError::Unknown)?
            .ok_or(GetBlockError::Unknown)?
    } else {
        tip(&inner.store)
            .map_err(|_| GetBlockError::Unknown)?
            .ok_or(GetBlockError::Unknown)?
    };
    let header = get_header(&inner.store, height)
        .map_err(|_| GetBlockError::Unknown)?
        .ok_or(GetBlockError::Unknown)?;
    let block = store_get_block(&inner.store, height)
        .map_err(|_| GetBlockError::Unknown)?
        .ok_or(GetBlockError::Unknown)?;
    Ok(json!({
        "height": height.0,
        "hash": encode_hex(header.hash().as_bytes()),
        "stateRoot": encode_hex(header.state_root.as_bytes()),
        "body": encode_hex(&encode_block_body(&block)),
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
    use storage::blocks::put_block;
    use types::block::Block;
    use types::genesis::Genesis;
    use types::header::{Header, HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{ChainId, Round, TestClock, ValidatorId};

    fn inner() -> RpcInner {
        RpcInner::from_config(NodeConfig::new(
            Genesis::new(ChainId::new(1)),
            BootstrapList::new(),
            identity::generate().unwrap(),
            std::path::PathBuf::from("/tmp/rpc-block"),
        ))
    }

    fn put_empty(inner: &mut RpcInner) -> Header {
        let clock = TestClock::new(2_000);
        let fields = HeaderFields::new(
            &clock,
            Height::GENESIS,
            Round::ZERO,
            ValidatorId::ZERO,
            0,
            1,
        )
        .unwrap();
        let header = Header {
            fields: fields.clone(),
            tx_root: Hash::ZERO,
            state_root: Hash::ZERO,
            receipts_root: Hash::ZERO,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        };
        let block = Block {
            header_fields: fields,
            txs: vec![],
        };
        put_block(&mut inner.store, &header, &block, &[], &Hash::ZERO).unwrap();
        header
    }

    #[test]
    fn get_block_by_height_and_hash() {
        let mut inner = inner();
        let header = put_empty(&mut inner);
        let by_h = dispatch(&mut inner, "l1_getBlock", &json!({"height": 0})).unwrap();
        assert_eq!(by_h["height"], 0);
        let by_hash = dispatch(
            &mut inner,
            "l1_getBlock",
            &json!({"hash": encode_hex(header.hash().as_bytes())}),
        )
        .unwrap();
        assert_eq!(by_hash["hash"], encode_hex(header.hash().as_bytes()));
        let miss = dispatch(&mut inner, "l1_getBlock", &json!({"height": 99})).unwrap_err();
        assert!(miss.message.contains("unknown"));
    }
}
