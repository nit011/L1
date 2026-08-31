//! OpenTelemetry-shaped spans around RPC (architecture.md §10).
//!
//! Contract: `obs.otel_tracing`.
//!
//! Spans wrap [`rpc::tx::submit_tx`] (`service.l1.jsonrpc.submitTx`) and
//! record onto [`crate::prometheus::Metrics`]. The inner `Result` is returned
//! unchanged — a tracing/metrics failure cannot swallow or remap RPC errors.

use crate::logging::{span_name, LAYER_RPC};
use crate::prometheus::Metrics;
use rpc::server::RpcInner;
use rpc::tx::{submit_tx, TxRpcError};
use serde_json::Value;

/// Trace `l1_submitTx` then return the **same** [`TxRpcError`] / JSON as the
/// untraced call would.
pub fn submit_tx_traced(
    metrics: &Metrics,
    inner: &mut RpcInner,
    params: &Value,
) -> Result<Value, TxRpcError> {
    let name = span_name(LAYER_RPC, "submitTx");
    let span = ::tracing::info_span!("rpc::submitTx", span = %name);
    let _enter = span.enter();
    let out = submit_tx(inner, params);
    metrics.record_rpc_submit(out.is_ok());
    let _ = metrics.scrape();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prometheus::Metrics;
    use crypto::from_ed25519;
    use crypto::sig::ed25519::SecretKey as EdSk;
    use crypto::tx::sign;
    use network::discovery::BootstrapList;
    use network::identity;
    use node::config::NodeConfig;
    use node::tracing as conv;
    use rpc::server::{encode_hex, RpcInner};
    use serde_json::json;
    use storage::codec::encode_signed_tx;
    use types::genesis::{Genesis, GenesisAccount};
    use types::tx::Tx;
    use types::{Amount, ChainId, Nonce, GAS_TRANSFER};

    fn inner_funded() -> (RpcInner, EdSk) {
        let ska = EdSk::from_bytes(&[3u8; 32]);
        let from = from_ed25519(&ska.verifying_key());
        let mut g = Genesis::new(ChainId::new(1));
        g.insert_alloc(
            from,
            GenesisAccount {
                balance: Amount::new(1_000_000),
                nonce: Nonce::ZERO,
                code_hash: types::Hash::ZERO,
            },
        );
        let cfg = NodeConfig::new(
            g,
            BootstrapList::new(),
            identity::generate().unwrap(),
            std::path::PathBuf::from("/tmp/obs-rpc"),
        );
        (RpcInner::from_config(cfg), ska)
    }

    #[test]
    fn traced_error_matches_untraced() {
        let m = Metrics::new();
        let (mut a, _) = inner_funded();
        let (mut b, _) = inner_funded();
        let raw = submit_tx(&mut a, &json!({}));
        let traced = submit_tx_traced(&m, &mut b, &json!({}));
        assert!(matches!(raw, Err(TxRpcError::Params)));
        assert!(matches!(traced, Err(TxRpcError::Params)));
        assert!(m.render().contains("l1_rpc_submit_err_total 1"));
        assert_eq!(
            conv::span_name(conv::SPAN_CONSENSUS, "commit"),
            "consensus::commit"
        );
    }

    #[test]
    fn traced_success_still_returns_hash() {
        let m = Metrics::new();
        m.set_exporter_up(false);
        let (mut inner, ska) = inner_funded();
        let from = from_ed25519(&ska.verifying_key());
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
        let v = submit_tx_traced(&m, &mut inner, &json!({"tx": hex})).unwrap();
        assert!(v.get("hash").is_some());
        assert_eq!(m.scrape(), Err(crate::prometheus::ExporterDown));
    }
}
