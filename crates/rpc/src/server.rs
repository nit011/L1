//! JSON-RPC 2.0 HTTP + WebSocket server. Contract: `rpc.server`.
//!
//! Built from Tier 7 [`node::config::NodeConfig`]. Method names are registered
//! in [`dispatch`] only — add a `l1_*` method there, not in the HTTP layer.
//!
//! JSON exists only at this crate boundary. Handlers decode into canonical
//! types (`SignedTx`, `Hash`, …) and call earlier tiers.

use crate::block::{get_block, GetBlockError};
use crate::state::{get_account, get_proof, StateRpcError};
use crate::status::{get_checkpoint, get_status};
use crate::sub::{subscribe, unsubscribe, SubError};
use crate::tx::{get_tx, submit_tx, TxRpcError};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use consensus::safety::CommitLog;
use consensus::steps::Finalized;
use execution::seq::World;
use mempool::Mempool;
use network::rate_limit::PeerRateLimiter;
use node::config::NodeConfig;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use storage::memory::MemoryStore;
use types::collections::Map;

/// Live node view the RPC process reads. Height/round come from [`Finalized`]
/// (`cons.commit`), not an RPC-owned counter.
pub struct RpcInner {
    /// `node.config`.
    pub cfg: NodeConfig,
    /// Durable store (`store.block.put` / indexes).
    pub store: MemoryStore,
    /// Mempool (`node.wire.mempool`).
    pub pool: Mempool,
    /// Live accounts (`state.account_trie`).
    pub world: World,
    /// Safety log from `cons.commit`.
    pub commits: CommitLog,
    /// Last `cons.commit` outcome (authoritative height/round).
    pub last_finalized: Option<Finalized>,
    /// Latest `ws.checkpoint`.
    pub checkpoint: Option<consensus::checkpoint::Checkpoint>,
    /// `netsec.peer_rate_limit` for `l1_submitTx`.
    pub limiter: PeerRateLimiter,
    /// Active JSON-RPC subscriptions (`l1_subscribe`).
    pub subs: Map<String, Subscription>,
    next_sub: u64,
}

/// One `l1_subscribe` client.
#[derive(Clone, Debug)]
pub struct Subscription {
    /// Topic (`newHeads`).
    pub topic: String,
    /// Still delivering.
    pub active: bool,
    /// Queued notifications (WS flush / tests).
    pub mailbox: Vec<Value>,
}

impl RpcInner {
    /// Construct from `node.config`. Contract: `rpc.server`.
    pub fn from_config(cfg: NodeConfig) -> Self {
        let world = World::from_genesis(&cfg.genesis);
        let pool = Mempool::new(&cfg.genesis.params.registry);
        Self {
            cfg,
            store: MemoryStore::new(),
            pool,
            world,
            commits: CommitLog::new(),
            last_finalized: None,
            checkpoint: None,
            limiter: PeerRateLimiter::new(),
            subs: Map::new(),
            next_sub: 1,
        }
    }

    pub(crate) fn alloc_sub_id(&mut self) -> String {
        let id = self.next_sub;
        self.next_sub = self.next_sub.saturating_add(1);
        id.to_string()
    }
}

/// Shared server state.
#[derive(Clone)]
pub struct RpcServer {
    /// Mutex so HTTP and WS share one chain view.
    pub inner: Arc<Mutex<RpcInner>>,
}

impl RpcServer {
    /// Bind JSON-RPC to a [`NodeConfig`].
    pub fn new(cfg: NodeConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RpcInner::from_config(cfg))),
        }
    }
}

/// JSON-RPC application error (code + message).
#[derive(Debug)]
pub struct RpcError {
    /// JSON-RPC error code.
    pub code: i64,
    /// Message.
    pub message: String,
}

impl RpcError {
    /// Invalid params (-32602).
    pub fn invalid_params(msg: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: msg.into(),
        }
    }

    /// Method not found (-32601).
    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: format!("method not found: {method}"),
        }
    }

    /// Application (-32000).
    pub fn app(msg: impl Into<String>) -> Self {
        Self {
            code: -32000,
            message: msg.into(),
        }
    }
}

impl From<TxRpcError> for RpcError {
    fn from(e: TxRpcError) -> Self {
        match e {
            TxRpcError::Params => Self::invalid_params("submitTx/getTx params"),
            TxRpcError::RateLimit => Self::app("rate limited"),
            TxRpcError::Mempool => Self::app("mempool rejected"),
            TxRpcError::Unknown => Self::app("unknown transaction"),
        }
    }
}

impl From<GetBlockError> for RpcError {
    fn from(e: GetBlockError) -> Self {
        match e {
            GetBlockError::Params => Self::invalid_params("getBlock params"),
            GetBlockError::Unknown => Self::app("unknown block"),
        }
    }
}

impl From<StateRpcError> for RpcError {
    fn from(e: StateRpcError) -> Self {
        match e {
            StateRpcError::Params => Self::invalid_params("state params"),
            StateRpcError::Unknown => Self::app("unknown account/block"),
            StateRpcError::Proof => Self::app("proof unavailable"),
        }
    }
}

impl From<SubError> for RpcError {
    fn from(e: SubError) -> Self {
        match e {
            SubError::Params => Self::invalid_params("subscribe params"),
            SubError::Unknown => Self::app("unknown subscription"),
        }
    }
}

/// Decode `0x` hex into bytes.
pub fn decode_hex(s: &str) -> Result<Vec<u8>, RpcError> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(s).map_err(|_| RpcError::invalid_params("hex"))
}

/// Hex with `0x` prefix.
pub fn encode_hex(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

/// Single JSON-RPC method table. Contract: `rpc.server`.
pub fn dispatch(inner: &mut RpcInner, method: &str, params: &Value) -> Result<Value, RpcError> {
    match method {
        "l1_submitTx" => submit_tx(inner, params).map_err(Into::into),
        "l1_getTransaction" => get_tx(inner, params).map_err(Into::into),
        "l1_getBlock" => get_block(inner, params).map_err(Into::into),
        "l1_getAccount" => get_account(inner, params).map_err(Into::into),
        "l1_getProof" => get_proof(inner, params).map_err(Into::into),
        "l1_getStatus" => Ok(get_status(inner)),
        "l1_getCheckpoint" => Ok(get_checkpoint(inner)),
        "l1_subscribe" => subscribe(inner, params).map_err(Into::into),
        "l1_unsubscribe" => unsubscribe(inner, params).map_err(Into::into),
        other => Err(RpcError::method_not_found(other)),
    }
}

fn envelope_ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn envelope_err(id: Value, err: RpcError) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": err.code, "message": err.message}
    })
}

/// Handle one JSON-RPC 2.0 object (HTTP body or WS text).
pub fn handle_object(inner: &mut RpcInner, v: &Value) -> Value {
    if v.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return envelope_err(
            Value::Null,
            RpcError::invalid_params("jsonrpc 2.0 required"),
        );
    }
    let id = v.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = v.get("method").and_then(Value::as_str) else {
        return envelope_err(id, RpcError::invalid_params("method"));
    };
    let params = v.get("params").cloned().unwrap_or(json!({}));
    match dispatch(inner, method, &params) {
        Ok(r) => envelope_ok(id, r),
        Err(e) => envelope_err(id, e),
    }
}

async fn http_rpc(State(srv): State<RpcServer>, Json(body): Json<Value>) -> impl IntoResponse {
    let mut inner = srv.inner.lock().expect("rpc lock");
    Json(handle_object(&mut inner, &body))
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(srv): State<RpcServer>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_loop(socket, srv))
}

async fn ws_loop(mut socket: WebSocket, srv: RpcServer) {
    while let Some(Ok(msg)) = socket.recv().await {
        let Message::Text(text) = msg else { continue };
        let Ok(body) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let reply = {
            let mut inner = srv.inner.lock().expect("rpc lock");
            handle_object(&mut inner, &body)
        };
        if socket
            .send(Message::Text(reply.to_string().into()))
            .await
            .is_err()
        {
            break;
        }
        let notes = {
            let inner = srv.inner.lock().expect("rpc lock");
            crate::sub::flush_all(&inner)
        };
        for n in notes {
            if socket
                .send(Message::Text(n.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    }
}

/// Axum router: `POST /` JSON-RPC, `GET /ws` subscriptions.
pub fn router(srv: RpcServer) -> Router {
    Router::new()
        .route("/", post(http_rpc))
        .route("/ws", get(ws_upgrade))
        .with_state(srv)
}

/// Bind HTTP+WS. Contract: `rpc.server`.
pub async fn serve(srv: RpcServer, addr: SocketAddr) -> Result<(), std::io::Error> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(srv))
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))
}

/// Unused path kept so `StatusCode` is referenced if extractors change.
pub fn health_code() -> StatusCode {
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use network::discovery::BootstrapList;
    use network::identity;
    use types::genesis::Genesis;
    use types::ChainId;

    fn cfg() -> NodeConfig {
        NodeConfig::new(
            Genesis::new(ChainId::new(1)),
            BootstrapList::new(),
            identity::generate().unwrap(),
            std::path::PathBuf::from("/tmp/rpc-test"),
        )
    }

    #[test]
    fn dispatch_unknown_method() {
        let mut inner = RpcInner::from_config(cfg());
        let e = dispatch(&mut inner, "l1_nope", &json!({})).unwrap_err();
        assert_eq!(e.code, -32601);
    }

    #[test]
    fn jsonrpc_requires_version() {
        let mut inner = RpcInner::from_config(cfg());
        let r = handle_object(&mut inner, &json!({"method": "l1_getStatus"}));
        assert!(r.get("error").is_some());
    }

    #[test]
    fn server_holds_node_config() {
        let srv = RpcServer::new(cfg());
        let g = srv.inner.lock().unwrap().cfg.genesis.chain_id;
        assert_eq!(g, ChainId::new(1));
        let _ = health_code();
        let _ = mesh_topic_bound();
    }

    #[tokio::test]
    async fn http_jsonrpc_get_status() {
        let srv = RpcServer::new(cfg());
        let app = router(srv);
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                json!({"jsonrpc":"2.0","id":1,"method":"l1_getStatus","params":{}}).to_string(),
            ))
            .unwrap();
        let resp = tower::ServiceExt::oneshot(app, req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["result"]["syncing"], true);
    }

    fn mesh_topic_bound() -> bool {
        let _ = network::gossip::mesh_config();
        true
    }
}
