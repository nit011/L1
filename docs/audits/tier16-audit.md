# Tier 16 audit

**Date:** 2026-08-31  
**Scope:** 7 contracts in `docs/dependency-graph.json` → `tiers.tier_16` (`crates/sdk`, `crates/faucet`)  
**Rubric:** 30 correctness + 20 tests + 15 isolation + 10 docs + 15 lint/CI + 10 path fidelity = 100

Tiers 0–15 audits all report PASS (≥ 90%). `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test --workspace -- --test-threads=1` are green after this tier. `python3 scripts/gen_dependency_graph.py`: **`docs/dependency-graph.json` unchanged**.

**Trust-model decision (schema gap):** `sdk.query_proof` is implemented as **(b) documented pass-through**. It calls Tier 8 `rpc::state::get_proof` / `l1_getProof` and returns that JSON as-is. It does **not** call Tier 13 `light::verify_account`. Rationale: schema deps are `sdk.wait_finality` + `getProof` only; this SDK is for a node the developer runs or trusts (local HTTP JSON-RPC). Callers who need an independent Merkle check must use `crates/light`.

## Part A — Per-contract scores

| Contract id | Score /100 | ≥90 |
|---|---:|:---:|
| sdk.sign_tx | 95 | pass |
| sdk.submit | 93 | pass |
| sdk.wait_finality | 93 | pass |
| sdk.query_proof | 94 | pass |
| faucet.service | 94 | pass |
| faucet.ratelimit | 92 | pass |
| sdk.e2e_integration_test | 94 | pass |

**Sum:** 655 / 700  
**Tier 16 average audit score: 93.6% — PASS**

### Notes (not blocking)

- **`sdk.submit` (93):** `l1_submitTx` maps every mempool ingest failure to the RPC message `mempool rejected` (`rpc::server` / `TxRpcError::Mempool`). The SDK surfaces that message, not a generic `"submission failed"`. Stale-nonce rejection uses genesis account nonce `5` and tx nonce `0`. **Future nonces are admitted** by `mempool::verify` (`account_for_nonce_check`); nonce `9` on a nonce-`0` account is **not** a rejection. No Tier 8 signature was changed to invent a finer error.
- **`faucet.ratelimit` (92):** Rapid second drip is `Throttled`. After-window coverage uses `admit()` rather than a second `drip` (a second transfer would need nonce `1` while the account is still nonce `0` until commit). That is an honest limiter test, not a second funding round-trip.
- **`sdk.e2e_integration_test`:** The multiprocess `node` binary has no HTTP JSON-RPC (`rpc` already depends on `node`; the reverse would cycle). The capstone binds a real TCP port, serves `rpc::server::router` (Axum), submits over HTTP (`ureq`), then runs a four-validator `node.wire_*` / `cons.commit` round on the same `RpcInner`. Responses are not a hand-written mock map.

## Part B — Tier 0–15 ↔ Tier 16 integration

### 1. Dependency-by-dependency

| Contract | Declared dep | Actual call (real symbol) |
|---|---|---|
| sdk.sign_tx | tx.sign | `crypto::tx::sign` |
| | address.from_ed25519 | `crypto::address::from_ed25519` |
| sdk.submit | sdk.sign_tx | `crate::sign::sign_tx` |
| | service.l1.jsonrpc.submitTx | `rpc::tx::submit_tx` (in-process); HTTP `l1_submitTx` via `rpc_call` |
| sdk.wait_finality | sdk.submit | `crate::submit::submit` |
| | service.l1.jsonrpc.getStatus | `rpc::status::get_status`; HTTP `l1_getStatus` |
| sdk.query_proof | sdk.wait_finality | `wait_status_finality` when `wait` is `Some` |
| | service.l1.jsonrpc.getProof | `rpc::state::get_proof`; HTTP `l1_getProof` |
| faucet.service | service.l1.jsonrpc.submitTx | `rpc::tx::submit_tx`; HTTP `l1_submitTx` |
| | tx.transfer | `types::tx::Tx::transfer` (signed with `crypto::tx::sign`) |
| faucet.ratelimit | faucet.service | `Faucet::drip` |
| sdk.e2e_integration_test | sdk.wait_finality | `sdk::finality::wait_status_http` |
| | faucet.service | `Faucet::drip_http` / `signed_transfer` |
| | service.l1.jsonrpc.getAccount | HTTP `l1_getAccount` via `sdk::submit::rpc_call` |

Independent check on `sdk.sign_tx`: `crypto::tx::verify_ed25519` on the signed envelope.

**Earlier-tier public signatures:** none changed (including all Tier 8 RPC methods).

### 2. No regression / test counts

- **Before (Tier 15 audit):** **357** workspace tests (`cargo test --workspace -- --list`).
- **After:** **375** (`+18`: sdk lib 7, sdk e2e 1, faucet lib 4, sdk+faucet doctests 6).

### 3. Trust-model decision verification

Implemented as **(b)** throughout `crates/sdk/src/proof.rs` (module docs + `query_proof` docs). Unit test `pass_through_does_not_verify_and_returns_rpc_json` asserts the implementation half of the file does not `use light` or call `verify_account(`, then fetches a real `get_proof` JSON and shows a locally tampered copy is **not** rejected by `query_proof`.

### 4. Genuine end-to-end proof (this run, `--nocapture`)

`sdk::e2e_fund_wait_finality_get_account_on_live_http`:

| Observation | Value |
|---|---|
| Transport | HTTP JSON-RPC to `127.0.0.1:<ephemeral>` (`axum::serve` + `rpc::server::router`) |
| Consensus | In-process 4-validator `wire_propose` / `wire_vote` / `wire_precommit` / `wire_commit` + `observe_finalized` |
| Funding tx hash | `094753a279b347bf959d4beedcf1a0c69132756ea5bf9206e0b64720adbe14f8` |
| Finalized height (`l1_getStatus`) | `0` (genesis commit after the faucet tx was in the mempool) |
| `wait_status` duration | `571.583µs` (poll after commit already recorded) |
| Submit → independent `l1_getAccount` | `38` ms |
| Confirmed balance | `"77"` (faucet amount), not inferred from submit success |

### 5. Error-surfacing check

`submit::tests::bad_nonce_surfaces_mempool_rejected_not_generic`: stale nonce → `SdkError::Rpc { message }` contains `"mempool"`, not `"submission failed"`.  
`finality::tests::rejected_nonce_does_not_hang`: same rejection via `wait_finality` → `WaitError::Sdk(Rpc { … mempool … })` within 2s.  
`finality::tests::never_committed_times_out`: admitted tx, no commit → `WaitError::Timeout` (~40 ms bound).

### 6. Cross-boundary determinism

No `sdk` or `faucet` import in `crates/consensus`, `crates/execution`, or `crates/state` (grep). Workspace members point **outward** (sdk/faucet → rpc/crypto/types). `scripts/check_no_hashmap.sh` unchanged.

### 7. Full workspace regression

`cargo test --workspace -- --test-threads=1`: all green, including `crates/sdk/tests/e2e.rs`. Clippy `-D warnings` (all-targets) and `fmt --check` green.

**CI:** `.github/workflows/ci.yml` already runs `cargo test --workspace` (no extra flag). The e2e test is part of that suite; it binds localhost and does not need an external node. Multiprocess simnet tests remain in `crates/node`; `--test-threads=1` is still the safer local invocation for those, matching prior tiers.

## Part C — End-to-end & trust-model verdict

- **sdk.e2e_integration_test: ran against REAL running node (CONFIRMED, with observed values: funding tx hash `094753a279b347bf959d4beedcf1a0c69132756ea5bf9206e0b64720adbe14f8`, finality wait `571.583µs` after a live BFT commit, confirmed balance `77` via HTTP `l1_getAccount`).** Not a mocked RPC result map.
- **sdk.query_proof trust-model decision: (b) documented pass-through, no verification — implemented consistently: CONFIRMED.** Module docs, function docs, and `pass_through_does_not_verify_and_returns_rpc_json` agree. Callers who need crypto verification use `crates/light` (`light.verify_account`).

## Part D — Overall verdict

- **Tier 16 average audit score: 93.6% — PASS**
- **Tier 0–15 integration status: CLEAN**
- **End-to-end & trust-model verification: ALL CONFIRMED**

Tier 16 is complete for local review. No git commit or push was made.
