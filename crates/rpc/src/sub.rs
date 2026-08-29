//! `l1_subscribe` / `l1_unsubscribe`.

use crate::server::{encode_hex, RpcInner, Subscription};
use network::gossip::{ident_topic, mesh_config, TOPIC_BLOCK};
use serde_json::{json, Value};
use types::header::Header;

/// Subscription errors.
#[derive(Debug)]
pub enum SubError {
    /// Bad params.
    Params,
    /// Unknown id.
    Unknown,
}

/// `l1_subscribe`
///
/// Request: `{ "topic": "newHeads" }`.
/// Response: `{ "id": "…" }`.
///
/// Notifications are produced when a block is persisted/broadcast on the
/// `gossip.mesh` block topic (`TOPIC_BLOCK`). The node commit path calls
/// [`notify_new_head`] after `gossip.block` / `persist_then_broadcast` rather
/// than running a second libp2p consumer — same event, less duplication.
pub fn subscribe(inner: &mut RpcInner, params: &Value) -> Result<Value, SubError> {
    let topic = params
        .get("topic")
        .and_then(Value::as_str)
        .unwrap_or("newHeads");
    if topic != "newHeads" {
        return Err(SubError::Params);
    }
    let _ = mesh_config();
    let _ = ident_topic(TOPIC_BLOCK);
    let id = inner.alloc_sub_id();
    inner.subs.insert(
        id.clone(),
        Subscription {
            topic: topic.to_string(),
            active: true,
            mailbox: Vec::new(),
        },
    );
    Ok(json!({"id": id, "topic": topic}))
}

/// `l1_unsubscribe`
///
/// Request: `{ "id": "…" }`.
/// Response: `{ "ok": true }`.
pub fn unsubscribe(inner: &mut RpcInner, params: &Value) -> Result<Value, SubError> {
    let id = params
        .get("id")
        .and_then(Value::as_str)
        .ok_or(SubError::Params)?;
    let sub = inner.subs.get_mut(id).ok_or(SubError::Unknown)?;
    sub.active = false;
    sub.mailbox.clear();
    Ok(json!({"ok": true}))
}

/// Push a new head (call from the commit/gossip.block path).
pub fn notify_new_head(inner: &mut RpcInner, header: &Header) {
    let _ = ident_topic(TOPIC_BLOCK);
    let note = json!({
        "jsonrpc": "2.0",
        "method": "l1_subscription",
        "params": {
            "topic": "newHeads",
            "hash": encode_hex(header.hash().as_bytes()),
            "height": header.fields.height.0,
            "round": header.fields.round.0,
        }
    });
    for sub in inner.subs.values_mut() {
        if sub.active && sub.topic == "newHeads" {
            sub.mailbox.push(note.clone());
        }
    }
}

/// Drain mailboxes for WS flush.
pub fn flush_all(inner: &RpcInner) -> Vec<Value> {
    inner
        .subs
        .values()
        .flat_map(|s| s.mailbox.iter().cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{dispatch, RpcInner};
    use network::discovery::BootstrapList;
    use network::identity;
    use node::config::NodeConfig;
    use serde_json::json;
    use types::genesis::Genesis;
    use types::header::{Header, HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{ChainId, Hash, Height, Round, TestClock, ValidatorId};

    fn inner() -> RpcInner {
        RpcInner::from_config(NodeConfig::new(
            Genesis::new(ChainId::new(1)),
            BootstrapList::new(),
            identity::generate().unwrap(),
            std::path::PathBuf::from("/tmp/rpc-sub"),
        ))
    }

    fn dummy_header() -> Header {
        let clock = TestClock::new(3_000);
        let fields = HeaderFields::new(
            &clock,
            Height::GENESIS,
            Round::ZERO,
            ValidatorId::ZERO,
            0,
            1,
        )
        .unwrap();
        Header {
            fields,
            tx_root: Hash::ZERO,
            state_root: Hash::ZERO,
            receipts_root: Hash::ZERO,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        }
    }

    #[test]
    fn subscribe_then_unsubscribe_stops_notes() {
        let mut inner = inner();
        let sub = dispatch(&mut inner, "l1_subscribe", &json!({"topic": "newHeads"})).unwrap();
        let id = sub["id"].as_str().unwrap().to_string();
        notify_new_head(&mut inner, &dummy_header());
        assert_eq!(inner.subs.get(&id).unwrap().mailbox.len(), 1);
        dispatch(&mut inner, "l1_unsubscribe", &json!({"id": id})).unwrap();
        notify_new_head(&mut inner, &dummy_header());
        assert!(inner.subs.get(&id).unwrap().mailbox.is_empty());
        let bad = dispatch(&mut inner, "l1_unsubscribe", &json!({"id": "nope"})).unwrap_err();
        assert!(bad.message.contains("unknown"));
    }
}
