//! Structured logging foundation (architecture.md §10 operational visibility).
//!
//! Contract: `obs.structured_logging`. **No earlier-tier deps** — this crate's
//! other modules build on these field names. Span names follow Tier 0
//! `tracing.conventions`: `layer::operation`. A log/trace call must never
//! decide hash-sensitive control flow.

use tracing_subscriber::EnvFilter;

/// Consensus layer name (same convention as `node::tracing::SPAN_CONSENSUS`).
pub const LAYER_CONSENSUS: &str = "consensus";
/// Execution layer name.
pub const LAYER_EXECUTION: &str = "execution";
/// RPC layer name (`service.l1.jsonrpc.*`).
pub const LAYER_RPC: &str = "rpc";
/// Gossip layer name.
pub const LAYER_GOSSIP: &str = "gossip";
/// Mempool layer name.
pub const LAYER_MEMPOOL: &str = "mempool";

/// Compose `layer::op` (JSON field `span`).
pub fn span_name(layer: &str, op: &str) -> String {
    format!("{layer}::{op}")
}

/// Install a JSON `tracing` subscriber (`RUST_LOG`, default `info`).
///
/// Idempotent: a second call is ignored so tests and the node binary can
/// both invoke it. Failure to init logging is swallowed — logging must not
/// abort the process (non-interference).
pub fn init_json() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .with_current_span(true)
        .with_span_list(false)
        .try_init();
}

/// One structured event as key/value pairs (JSON object). Used by tests and
/// by callers that need a buffer instead of the global subscriber.
pub fn json_event(span: &str, message: &str, fields: &[(&str, serde_json::Value)]) -> String {
    let mut obj = serde_json::Map::new();
    obj.insert("span".into(), serde_json::Value::String(span.to_string()));
    obj.insert(
        "message".into(),
        serde_json::Value::String(message.to_string()),
    );
    for (k, v) in fields {
        obj.insert((*k).into(), v.clone());
    }
    serde_json::Value::Object(obj).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_event_is_parseable_and_named() {
        let line = json_event(
            &span_name(LAYER_CONSENSUS, "commit"),
            "finalized",
            &[("height", serde_json::json!(1))],
        );
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["span"], "consensus::commit");
        assert_eq!(v["height"], 1);
        assert_eq!(v["message"], "finalized");
    }

    #[test]
    fn malformed_json_is_rejected() {
        assert!(serde_json::from_str::<serde_json::Value>("{not json").is_err());
        init_json();
        init_json();
    }
}
