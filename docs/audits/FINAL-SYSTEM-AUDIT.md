# Final system audit

**Date:** 2026-08-31  
**Scope:** Whole assembled L1 (`docs/dependency-graph.json`), implemented tiers **0–16, 18, 19**.  
**Not implemented (explicit):** **Tier 17** (`gov.pause`, `ops.pause_cli`, `ops.rbac`, `ops.audit_log`, `ops.config_toggle`); **Tier 20** (Verkle, zk, encrypted mempool, FHE, HotStuff pipeline).

This report does **not** treat prior per-tier audits as sufficient on their own. Evidence below is from a fresh workspace run plus targeted re-execution of golden vectors, SDK e2e, STM equivalence (including WASM+staking mix), simnet, and the ignored Docker stress suite.

---

## 1. Tier-by-tier re-confirmation

### Suite (this pass)

| Command | Result |
|---|---|
| `cargo fmt --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass (`CLIPPY_OK`) |
| `cargo build --workspace` | pass (`BUILD_OK`) |
| `cargo test --workspace -- --test-threads=1` | **398 passed, 0 failed, 5 ignored**, exit 0 |
| Stress unit (`tests/stress`, default) | **9 passed, 5 ignored** |
| Stress Docker (`--ignored --test-threads=1`) | **5 passed** (compose + gossip + joiner + DAS window) |

`rust_file` path check: **249 / 249** implemented-tier contracts still point at files that exist on disk. No schema drift.

### Golden vectors (Tier 3 `exec.golden_vectors`) — re-derived this pass

`cargo test -p execution --test golden` (6/6). Literals still match `docs/audits/tier3-audit.md`:

| Vector | Hex (unchanged) |
|---|---|
| `genesis.hash` | `3070e230ec9bd58862fe78b43774f85879d2db270f8e5f28facb4637bae5f1b1` |
| empty-block `app_hash` | `43dca346e5485849e16be9da4fc13d10c43e3b7701df51e7c273d1b4ed3cf6ad` |
| single transfer `app_hash` | `2898208706d1893606f1f79959653189f3d2163f528b849430fd569db238515f` |
| rejected nonce `app_hash` | `3ad011d6221bb5f627aafc815a8ee8352caf8efd8b30352b08a67877b2c09f62` |
| multi-account `app_hash` | `c70d2307dcc8448eaa9a18ce3d1bc372036ba97c0e93f3a9c1c4e7f9b5164909` |

STM golden empty/single/rejected/multi (`execution --test stm_equiv`) match the sequential hashes (6/6 including 768-case WASM+stake+transfer property).

BLAKE3 empty KAT (`hash.blake3`): `af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262` still asserted in `crates/crypto/src/hash/blake3.rs`.

### Status table

Original scores are from each `docs/audits/tierN-audit.md`. Re-confirmation is this pass’s tests + path check + spot-checks (three consensus-critical contracts per tier, listed in notes).

| Tier | Original avg | Re-confirmed | Notes (spot-checks) |
|---|---:|---|---|
| 0 | 94.3% | **RE-CONFIRMED PASS** | `hash.blake3` KAT; `ed25519`/`bls` via `crypto` 27 tests; `reed_solomon` via `da` 19 tests |
| 1 | 93.7% | **RE-CONFIRMED PASS** | `mpt.*` via `state` 25 tests; `kzg.setup` in `da::root::commit`; merkle proofs in RPC/light |
| 2 | 94.2% | **RE-CONFIRMED PASS** | `vrf.weighted_leader` (`consensus` vrf tests); `spec.constants` in `types`; genesis params |
| 3 | 93.3% | **RE-CONFIRMED PASS** | Golden hex above; `nonce_check`/`gas_meter` still used by `seq::apply_tx`; `cons.timeout.config` in golden fixture |
| 4 | 92.8% | **RE-CONFIRMED PASS** | `mempool.insert`/`verify`; `block.builder.local` (`build_local` → `seq::apply_block`); `store.block.put` |
| 5 | 93.0% | **RE-CONFIRMED PASS** | `cons.commit` + QC (`consensus` 30 tests); simnet 4/4; WAL double-sign tests |
| 6 | 92.9% | **RE-CONFIRMED PASS** | gossip codec + `headers_then_bodies` (`network` 42 tests); eclipse/rate-limit tests still in that crate |
| 7 | 92.7% | **RE-CONFIRMED PASS** | simnet 4/4 (`mvp.finality_lan` path); `node.wire.*` 20 lib tests |
| 8 | 93.2% | **RE-CONFIRMED PASS** | `rpc` 14 tests (`l1_getStatus`/`getProof` bounds); `node.config` load_dir |
| 9 | 93.2% | **RE-CONFIRMED PASS** | `execution::staking` / slash tests; epoch set update vs VRF leader |
| 10 | 93.1% | **RE-CONFIRMED PASS** | `stm.apply_block` ≡ seq (stm_equiv 6/6); **not** wired into `node` (seam, §2) |
| 11 | 93.3% | **RE-CONFIRMED PASS** | WASM deploy/call/reentrancy tests; mixed into stm_equiv WASM seed |
| 12 | 93.6% | **RE-CONFIRMED PASS** | `das.fail_closed` unit + Docker withhold window |
| 13 | 93.8% | **RE-CONFIRMED PASS** | `light` 12 tests (`verify_qc` / `verify_account`) |
| 14 | 93.1% | **RE-CONFIRMED PASS** | `sync.snapshot` + replay equality (stress unit + storage snapshot tests) |
| 15 | 93.8% | **RE-CONFIRMED PASS** | `observability` 14 tests |
| 16 | 93.6% | **RE-CONFIRMED PASS** | Live HTTP e2e this pass (tx/balance below) |
| 17 | — | **NOT IMPLEMENTED** | `gov.*` / `ops.*` |
| 18 | 93.3% | **RE-CONFIRMED PASS** | Compose 4-node this pass; `iac` lib tests; genesis materialize |
| 19 | 92.3% | **RE-CONFIRMED PASS** | Docker p50/p99 re-measured (below) |
| 20 | — | **NOT IMPLEMENTED** | Roadmap; must not gate MVP |

**No Section A ISSUE FOUND.** Seams in §2 are architectural (documented cycles / forbidden edges), not silently broken tests.

---

## 2. Integration findings

### 2.1 Five deep call chains (real symbols)

#### A. `sdk.wait_finality`

`sdk::wait_finality` (`crates/sdk/src/finality.rs`)  
→ `sdk::submit` → `rpc::tx::submit_tx` → `mempool::Mempool::insert` → `mempool::verify::verify` → `crypto::tx::verify_ed25519` → `ed25519.verify`  
→ `sdk::sign_tx` → `crypto::tx::sign` → `ed25519.sign` + `hash.blake3` (`signed_message`)  
→ poll `rpc::status::get_status`  
→ height appears only after caller runs `node::wire::wire_commit` → `consensus::steps::commit` → `qc::aggregate` / `qc::verify` → `bls.aggregate` / `bls.verifyAggregate`  
→ `persist_then_broadcast` → `storage::blocks::put_block`.

#### B. `stress.consensus_4node`

`run_consensus_window` (`tests/stress/consensus.rs`)  
→ `harness::bring_up` → `iac::materialize_with_bank` (`infra/genesis.rs`) → `docker compose -f infra/docker-compose.yml`  
→ `sdk::sign_tx` + `gossip_txs` → `network::gossip` `TOPIC_TX`  
→ container `node` `wire_mempool` → mempool insert (same as A)  
→ `wire_propose` → `consensus::propose` + `execution::builder::build_local` → **`execution::seq::apply_block`** (not STM)  
→ `wire_vote` / `wire_precommit` → `cons.commit`  
→ bind-mounted `events.log` `COMMIT n` + `tip`.

#### C. `service.l1.jsonrpc.getProof`

`rpc::state::get_proof` (`crates/rpc/src/state.rs`)  
→ `parse_addr` (32-byte hex)  
→ `storage::blocks::tip` / `get_header`  
→ `state::mpt::proof::prove` on `AccountTrie`  
→ node hash `crypto::hash::blake3::hash_to_array` + `domain.tag.apply` (`DomainTag::MptNode`)  
→ JSON `stateRoot` from header or `World::commit_state_root` → `state::root::commit_root`.

Independent check: `light::verify_account` (`crates/light/src/account.rs`) calls `mpt::proof::verify` against a QC-checked header — **SDK `query_proof` does not** (Tier 16 trust model (b)).

#### D. `das.fail_closed`

`da::das::fail_closed` (`crates/da/src/das.rs`)  
→ `sample` → `ChunkFetch::fetch` + `da::root::verify_chunk` → `state::merkle::verify`  
→ chunks from `da::root::commit` → `da::chunk::split` → `reed_solomon.encode` + `kzg.setup`/`kzg.commit`  
**Does not** call `cons.commit` (forbidden edge). Docker stress: compose still `COMMIT`s while sampling withheld `MemoryChunks`.

#### E. `stm.apply_block`

`execution::stm::apply_block` (`crates/execution/src/stm/mod.rs`)  
→ `seq::apply_block` (comparison, panics on divergence)  
→ `speculate` / `conflict_graph` / `run_waves` / `validate` / `reexec_sequential`  
→ `seq::apply_tx` → `checks::nonce_check` / `gas::gas_meter` / WASM `sload`/`sstore` → `state::versioned_slot`  
→ `app_hash` = `hash.blake3` over `state_root ‖ tx_root ‖ receipts_root`.

**Live node never calls this function.** `build_local` and `node/src/main.rs` use `seq::apply_block` only.

### 2.2 Cold-start end-to-end (observed)

**What a new user can actually run today is two stacks, not one:**

1. **JSON-RPC + faucet + SDK + BFT** — in-process (Axum `rpc::server` + `node.wire_*`). The `node` **binary has no HTTP JSON-RPC** (`rpc` already depends on `node`; reversing that is a crate cycle).  
2. **Four-validator P2P** — Docker Compose (`iac.docker_compose`). Load is gossip `TOPIC_TX`, not `l1_submitTx`.

**Stack 1, this pass** (`sdk` e2e, live TCP, four BLS validators, faucet amount 77):

| Field | Observed |
|---|---|
| Funding tx hash | `609962261650b20faae599ad77d3e689772bc725cce562e7c8af6fcecee097a5` |
| Finalized height | `0` (genesis slot in this fixture) |
| `l1_getAccount` balance | `"77"` |
| Dest address | `52630e7475c383bfe540aae47181982a8f6383351f8807d439da8f12587d0423` |
| Submit → account | **41 ms**; status poll **~720 µs** after `wire_commit` |

**Stack 2, this pass** (ignored stress, `l1-node:devnet`):

| Field | Observed |
|---|---|
| p50 / p95 / p99 block interval | **1087 / 1212 / 1224 ms** |
| COMMIT delta (18s window) | **17** |
| Joiner catch-up | **1075 ms** (behind 1 → tip 2) |
| Compose submit TPS / block TPS | **9.0 / 1.12** (32 txs) |
| Bank genesis (n_bank=2) | `a1e17c1cf9b9df3e14e3ea32608fdacd31e77f149c2868f84faa22b3aafb7f94` |

There is **no** single command that starts compose **and** serves `l1_submitTx` on the same process. That is a seam, not a hidden pass.

### 2.3 Multi-subsystem interaction

**STM + seq, same block, mixed tx kinds** — `property_stm_equals_seq_wasm_three_seeds` (768 cases): each block mixes **WASM deploy/call**, **transfer**, and **`Tx::stake_bond`**. All 6 stm_equiv tests passed this pass.

**Mempool → consensus → execution → storage → RPC in one process** — SDK e2e: faucet HTTP `l1_submitTx` → mempool → `wire_propose`/`wire_vote`/`wire_commit` → `put_block` → `l1_getAccount`. Execution engine on that path is **seq**, not STM. No WASM/staking in that particular e2e.

**Gossip → compose consensus** — stress `docker_consensus_4node_p99` (signed transfers only).

There is **no** test that sends a WASM call and a stake tx through **Docker gossip** into a **containerized** node. Closest full-stack mix is in-process STM equivalence.

### 2.4 Seam gaps

| Gap | Severity | Status |
|---|---|---|
| `node` binary has no JSON-RPC; SDK/faucet need `rpc::server` in another task | should-fix for “one binary devnet” | not fixed (cycle `rpc` → `node`) |
| Live propose/commit uses `seq::apply_block`; `stm.apply_block` is tests/benches only | acceptable-as-is for MVP wiring (Tier 10 audit); honest scale finding | not fixed |
| Compose has no RPC ports; stress injects via gossip | acceptable-as-is given cycle | not fixed |
| `das.fail_closed` does not gate `cons.commit` (by design) | acceptable-as-is (architecture.md §6) | not fixed |
| `sdk.query_proof` is RPC pass-through; `light.verify_account` is separate | acceptable-as-is (Tier 16 trust model b) | not fixed |
| `n_validators > 4` in stress is logged, not launched | acceptable-as-is (compose schema is 4) | not fixed |
| `types::hashing::blake3_array` uses the `blake3` crate directly (`types` cannot depend on `crypto`) | acceptable-as-is; same algorithm, tested against `hash.blake3` | not fixed |
| Node `main` has no SIGTERM/graceful shutdown of the swarm | should-fix | not fixed |
| `build_local` / `wire_propose` `expect` on timestamps/params | should-fix (consensus-adjacent) | not fixed |

---

## 3. Code quality findings

### C1. Duplicates

| Finding | Verdict |
|---|---|
| Single `Store` (`crates/storage/src/kv.rs`); single `Clock` (`crates/types/src/clock.rs`) | acceptable-as-is |
| `types::hashing` vs `crypto::hash::blake3` | legitimate crate-cycle split; same BLAKE3-256 |
| `seq::apply_block` vs `stm::apply_block` | intentional; STM must match seq or panic |
| `MemoryStore` vs file store | same `Store` trait |
| No second MPT outside `state::mpt` | OK |

No duplicate `Store`/`Clock` files.

### C2. Hardcoded values

| Location | What | Verdict |
|---|---|---|
| `crates/types/src/spec.rs` | HASH_SIZE, GAS_*, MAX_*, MIN_TX_FEE | true constants / `spec.constants` |
| `infra/docker-compose.yml` + `NODE_IPS` / `NODE_PORTS` | 172.28.0.10–13, UDP 4001–4004 | must match; IaC pairing |
| `tests/stress/compose.override.yml` | host 14001–14004 | test overlay |
| `DEVNET_CHAIN_ID = 18` | IaC chain id | genesis param, not a leaked TestClock |
| `quic_listen_local` 127.0.0.1 | default bind; override `L1_LISTEN` | configurable |
| SDK docs `http://127.0.0.1:8545/` | example only; e2e uses ephemeral `:0` | OK |
| `NodeConfig.min_block_time_ms` default 1000 | matches §10 1–2s | from node config, not a stray magic in gossip |

No TestClock in `node` production `main` (uses `SystemClock`).

### C3. Production-grade

**Error handling:** Tier 0 clippy `-D warnings` still holds workspace-wide. **unwrap/expect in non-test paths** exist in consensus-critical crates: `consensus::vrf` last-validator `expect`; `execution::seq` `expect("checked")` after earlier checks; STM `expect` on versioned slots; WASM `Engine::new().expect`; `state::mpt` `expect("dangling node hash")`. **Policy not fully respected after Tier 0.** Severity: **should-fix**, not a test failure.

**Resource cleanup:** Compose `tear_down` runs `docker compose down`. Node binary: loop on `swarm.next()`; **no** explicit Ctrl-C cancel. **should-fix**.

**Input validation:** RPC `jsonrpc 2.0` required; address hex length 32; gossip `decode_frame`. WASM deploy prepares/validates module. Spot-check: `get_proof` rejects bad hex.

**Logging:** Node COMMIT via `events.log` lines (not JSON). Observability crate is structured; node binary uses tracing init plus file append. No private keys printed in those event lines. SDK examples use `println!` in doc comments only.

**`cargo audit`:** **not installed** in this environment; not run. **llvm-cov / tarpaulin:** **not available**; no coverage %.

**Test volume:** 398 default + 5 ignored Docker this pass.

---

## 4. Overall system score

Equal weight per **implemented** tier average (19 values: 0–16, 18, 19):

**93.3%**

(1773.2 / 19; arithmetic mean of the original published averages, all of which this pass re-confirmed.)

**The whole system clears the 90% bar** used per tier.

That score does **not** mean mainnet-ready. It means: implemented contracts still test green, paths match the schema, goldens are stable, and integration works **along the paths that exist** (with the seams above).

---

## 5. Final verdict

**SYSTEM VERIFIED — READY FOR NEXT PHASE**

Next phase means **Tier 17 (ops/gov)** or further engineering — **not** depositing real value.

Blocking for **mainnet / real funds** (not blocking this 90% engineering bar): no professional security review, no `cargo audit` this pass, RPC not in the node binary, STM not on the live commit path, ~1 tx/block-second on compose vs §10 10k TPS, no pause/RBAC. See `docs/GAP-ANALYSIS.md`.
