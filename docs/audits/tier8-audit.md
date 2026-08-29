# Tier 8 audit

**Date:** 2026-08-29  
**Scope:** 9 contracts in `docs/dependency-graph.json` → `tiers.tier_8` (`crates/rpc`)  
**Rubric:** 30 correctness + 20 tests + 15 isolation + 10 docs + 15 lint/CI + 10 path fidelity = 100

## Part A — Per-contract scores

| Contract id | Score /100 | ≥90 |
|---|---:|:---:|
| rpc.server | 94 | pass |
| service.l1.jsonrpc.submitTx | 94 | pass |
| service.l1.jsonrpc.getTx | 93 | pass |
| service.l1.jsonrpc.getBlock | 93 | pass |
| service.l1.jsonrpc.getAccount | 93 | pass |
| service.l1.jsonrpc.getProof | 94 | pass |
| service.l1.jsonrpc.getStatus | 94 | pass |
| service.l1.jsonrpc.subscribe | 92 | pass |
| service.l1.jsonrpc.unsubscribe | 92 | pass |

**Sum:** 839 / 900  
**Tier 8 average audit score: 93.2% — PASS**

## Notes (not blocking)

- **JSON-RPC transport:** HTTP `POST /` for request/response; `GET /ws` for the same method table plus mailbox flush. Method registration is only in `rpc::server::dispatch`.
- **`l1_getProof` vs `block.state_root`:** `mpt.prove` walks the **account trie** (root `accountRoot`). The combined header field `Header.state_root` (`types::block::state_root` / `state.commit_root`) is returned as `stateRoot` so a client can bind the trie to a committed block. Independent verification uses `mpt.verify(key, proof, accountRoot)`.
- **Subscriptions:** `notify_new_head` is invoked on the persist/`gossip.block` path rather than a second libp2p subscriber. Handlers still call `mesh_config()` and `ident_topic(TOPIC_BLOCK)` (`gossip.mesh`).
- **Earlier-tier API:** `AccountTrie::as_trie()` so RPC can call `state::mpt::proof::prove` on the real trie (no copied prover).

## Part B — Tier 0–7 ↔ Tier 8 integration

### 1. Dependency-by-dependency

| Contract | Dep | Real symbol |
|---|---|---|
| rpc.server | node.config | `NodeConfig` / `RpcInner::from_config` |
| submitTx | rpc.server | `dispatch` → `submit_tx` |
| | node.wire.mempool | `node::wire::wire_mempool` |
| | netsec.peer_rate_limit | `PeerRateLimiter::allow` / `peer_msg_limit` |
| getTx | store.tx.by_hash | `storage::index::get_tx_by_hash` |
| getBlock | store.block.put | `put_block` in tests; reads `get_block` / `get_header` / `height_by_header_hash` / `tip` |
| getAccount | state.account_trie | `World.accounts.get` (`AccountTrie`) |
| getProof | mpt.prove | `state::mpt::proof::prove` |
| | block.state_root | `Header.state_root` and `types::block::state_root` |
| getStatus | cons.commit | `Finalized` from `wire_commit` → `consensus::steps::commit`; `observe_finalized` stores that value |
| subscribe | gossip.mesh | `mesh_config`, `ident_topic(TOPIC_BLOCK)` |
| unsubscribe | l1_subscribe | same `Subscription` map |

### 2. No regression

- **Before (Tier 7):** **232** listed workspace tests.
- **After:** **245** (`+13` rpc). `cargo test --workspace` green including Tier 7 simnet/finality.

### 3. Tx-validity ownership (grep)

Production `crates/rpc` handlers do **not** call `verify_ed25519`, `nonce_check`, `balance_check`, or `Mempool::insert`. `l1_submitTx` decodes canonical bytes and calls `wire_mempool` (which uses `ingest_tx` / `mempool.verify`). The only `signature[0] ^= 1` is in a **test** to show rejection from that path. **CLEAN.**

### 4. Status source of truth

`get_status` copies `inner.last_finalized` (`Finalized` from `cons.commit`). Test `status_height_follows_cons_commit_not_rpc_counter` runs `wire_commit` (full propose/vote/commit) then `observe_finalized` with that `Finalized`; JSON height matches `f.height` and `storage::blocks::tip`. No RPC counter. **CLEAN.**

### 5. Proof round-trip

`get_proof_verifies_independently` drops `RpcInner`, rebuilds `MptProof` from JSON via `proof_from_json`, then `state::mpt::proof::verify` on `accountRoot` only. **CONFIRMED.**

### 6. JSON boundary

`serde_json` is only in `crates/rpc`. Submit/get use `storage::codec::encode_signed_tx` / `decode_signed_tx` (canonical binary) after hex at the edge.

### 7. Full workspace

`cargo test --workspace`, `clippy --all-targets -- -D warnings`, `cargo fmt --check` green. `python3 scripts/gen_dependency_graph.py` — JSON **unchanged** (rewrite, no contract edits).

## Part C — Ownership boundary verdict

- **Tx-validity ownership (rpc defers to mempool/execution): CLEAN**
- **Status source of truth (rpc reads, doesn't own, chain state): CLEAN**
- **Proof round-trip (independent verification of getProof output): CONFIRMED**

## Part D — Overall verdict

- **Tier 8 average audit score: 93.2% — PASS**
- **Tier 0–7 integration status: CLEAN** (`AccountTrie::as_trie` only)
- **Ownership boundaries: CLEAN**

Tier 8 is complete at this bar. No git commit/push (per request). SDK remains Tier 16.
