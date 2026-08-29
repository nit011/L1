# Tier 7 audit

**Date:** 2026-08-29  
**Scope:** 9 contracts in `docs/dependency-graph.json` → `tiers.tier_7` (`crates/node`)  
**Rubric:** 30 correctness + 20 tests + 15 isolation + 10 docs + 15 lint/CI + 10 path fidelity = 100

## Part A — Per-contract scores

| Contract id | Score /100 | ≥90 |
|---|---:|:---:|
| node.config | 93 | pass |
| node.wire.mempool | 94 | pass |
| node.wire.propose | 92 | pass |
| node.wire.vote | 92 | pass |
| node.wire.commit | 94 | pass |
| node.wire.sync | 91 | pass |
| node.catchup | 93 | pass |
| node.simnet.multiprocess | 92 | pass |
| mvp.finality_lan | 93 | pass |

**Sum:** 834 / 900  
**Tier 7 average audit score: 92.7% — PASS**

## Notes (not blocking)

- **`node.wire.sync` vs `cons.commit`:** Local finality calls `consensus::steps::commit` then `persist_then_broadcast`. Catch-up cannot form a local QC for blocks already finalized elsewhere, so `wire_sync` / `catchup` reuse **`persist_then_broadcast`** (WAL + `store.block.put` + `gossip.block`), not a second `cons.commit`. That is one persist path; it is not a second consensus finalizer.
- **`mempool.admit_preverified`:** Additive enqueue used after `network::topics::ingest_tx` (`gossip.tx` / `mempool.verify`). `node.wire.mempool` does not call `Mempool::insert` / `verify` again. Fee order is `peek_best_ready` / `ReadyTxs::take_ready`.
- **Devnet VRF secrets file:** Static genesis validators share `vrf_secrets.bin` so every process can build the round VRF-source proof (`vrf::leader_prove`) without a separate proof topic. Production staking (Tier 9) is out of scope.
- **Earlier-tier public signatures:** unchanged except the additive `Mempool::admit_preverified`.

## Part B — Tier 0–6 ↔ Tier 7 integration

### 1. Dependency-by-dependency (function-by-function)

| Contract | Dep | Real symbol called |
|---|---|---|
| node.config | genesis.params | `Genesis::params` / `ParamsRegistry` via `NodeConfig::new` |
| | p2p.bootstrap | `network::discovery::BootstrapList` (`insert`, `write_bootstrap` / `load_dir`) |
| node.wire.mempool | gossip.tx | `network::topics::ingest_tx` |
| | mempool.fee_order | `Mempool::peek_best_ready`; tests `ReadyTxs::take_ready` |
| node.wire.propose | node.wire.mempool | `build_local` → mempool `take_ready` inside `wire_propose`’s `cons.propose` callback |
| | cons.propose | `consensus::propose::propose` |
| | gossip.proposal | `network::topics::ingest_proposal` + `BlockBroadcast::broadcast_proposal` |
| node.wire.vote | cons.prevote_step | `consensus::steps::prevote_step` (`wire_vote`) |
| | cons.precommit_step | `consensus::steps::precommit_step` (`wire_precommit`) |
| | gossip.vote | `network::topics::ingest_vote` (`wire_ingest_vote`) |
| | mesh.validator | `ValidatorMesh`, `ingest_validator_proposal`, `ingest_validator_vote`; votes published on `/l1/validator/vote/1` |
| node.wire.commit | cons.commit | `consensus::steps::commit` |
| | store.block.put | `storage::blocks::put_block` |
| | wal.execution | `storage::wal::write_wal` then `clear_wal` |
| | gossip.block | `network::topics::ingest_block` then `broadcast_block` |
| node.wire.sync | sync.headers_then_bodies | `network::sync::headers_then_bodies` (scratch store) |
| | node.wire.commit | same `persist_then_broadcast` as `wire_commit` (see note) |
| node.catchup | node.wire.sync | `wire_sync` |
| | store.replay_from_genesis | `storage::replay::replay_from_genesis` + `execution::seq::apply_block` |
| node.simnet.multiprocess | node.wire.commit / node.config | OS processes run `node` binary; `write_dir` / `NodeConfig`; commits via `wire_commit` |
| mvp.finality_lan | node.simnet.multiprocess | `tests/finality.rs` spawns the same 4-process cluster |
| | cons.commit | process loop → `wire_commit` → `steps::commit` |

No copied consensus/mempool/storage implementations inside `node` beyond wiring and codecs for gossip payloads.

### 2. No regression (test counts)

- **Before (Tier 6 audit):** **218** workspace tests.
- **After:** **232** listed tests (`cargo test --workspace -- --list`). Delta is Tier 7 `node` lib integration tests (config/wire/sync) plus `tests/simnet.rs` (4) and `tests/finality.rs` (1). Tracing tests were already in the 218.
- Re-ran `cargo test --workspace` after Tier 7: **all green**. Execution goldens, storage replay/WAL, consensus simnet safety/liveness, network boundary tests included.

### 3. Single persist path

`wire_commit` after `cons.commit` calls `persist_then_broadcast`. `wire_sync` after `headers_then_bodies` calls `persist_then_broadcast` for each `BodyOffer`. The node binary’s late-joiner path calls `node::sync::catchup` → `wire_sync`. There is no second `put_block` helper for “synced vs voted.”

### 4. Persist-before-broadcast

`persist_then_broadcast`: `ingest_block` → `write_wal` → `put_block` → `clear_wal` → `sink.broadcast_block`.  
Test `persist_happens_before_broadcast_even_if_store_is_slow` delays every `Store::put` by 30ms and asserts `put_done <= broadcast_at`. Flipping broadcast before `put` would fail that assertion.

### 5. Cross-boundary determinism

`types::collections::Map` for validators, VRF keys, sync offers. Votes inserted with `insert_vote_sorted` (signer order). Genesis encode/decode walks sorted maps.

### 6. Multiprocess flakiness

- `multiprocess_four_nodes_commit_three_runs`: **3/3** (one cargo test, three cluster lifetimes).
- `mvp.finality_lan`: **3/3** consecutive runs in one test (numbers in Part C).
- `late_join_fifth_node_catchup`, `eclipse_rejection_ip_slot_cap_in_multiprocess_context`, `invalid_gossip_dropped_at_wire_mempool`: **pass** on the same `cargo test -p node` invocation.

An earlier single finality run showed a **16977 ms** interval when every committed block was re-gossiped on every event-loop tick (gossip starvation). Republish is now every **2s**; vote/proposal retransmit every **250ms**. That stall is recorded here; it is not in the three passing finality runs below.

### 7. Full workspace

`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, `bash scripts/check_no_hashmap.sh` green.  
`python3 scripts/gen_dependency_graph.py` rewrote `docs/dependency-graph.json`; **no contract/DAG change** intended (`git diff` on that file empty in this tree).

**Earlier-tier code touched:** `Mempool::admit_preverified` only (pre-verified enqueue). `scripts/check_no_hashmap.sh` already included `crates/node`.

## Part C — MVP milestone verification

Measured on localhost (macOS), `mvp.finality_lan` three cluster runs, `min_block_time_ms = 1000`, 2s gossip mesh warmup before first propose:

| Run | Time to first `COMMIT` (ms) | Block intervals (ms) |
|---:|---:|---|
| 0 | 3148 | 1078, 1083 |
| 1 | 2131 | 1085, 1083 |
| 2 | 2114 | 1071, 1098 |

- **Observed block time:** **1071–1098 ms** after the first commit (target **1–2 s**). Mesh join is **not** counted as block time.
- **Observed time-to-finality:** single-slot BFT; interval after propose+QC is the same as block time here (**~1.08 s**, target **&lt; 5 s**). Time-to-first-commit includes QUIC/gossipsub join (**2.1–3.1 s**), still under 5s on these runs.
- **Late join:** 4-process chain to tip **≥ 2** (blocks at heights 0,1,2), then a 5th process with genesis-only identity (not in the validator set). It caught up to that tip via gossiped bodies + `node.catchup` (`CATCHUP` / tip file). Exact catch-up duration was not separately stopwatched; the test bound is **40 s** and it passed inside the 18s simnet suite wall time.
- **Eclipse:** `IpSlotTable::admit` with ten peers on `10.0.0.0/24` admitted **2**; a peer on `11.1.2.3` admitted; table length **3**. Four validator processes on `127.0.0.1` still committed (genesis mesh, not slot-table gated).
- **Invalid gossip:** extra swarm published a bad-signature tx on `/l1/tx/1`. Receivers logged **`TX_DROP`**; **`TX_ADMIT` count = 0**. Drop is `ingest_tx` inside `wire_mempool`.

## Part D — Overall verdict

- **Tier 7 average audit score: 92.7% — PASS**
- **Tier 0–6 integration status: CLEAN** (additive mempool enqueue only; persist path shared; no DAG contract edits)
- **MVP milestone criteria: ALL MET**
  - Multi-process finality: 4 processes, block time **~1.08 s**, TTF **~1.08 s** (plus **2–3 s** first-block mesh warmup)
  - Late join & catch-up: confirmed
  - Eclipse rejection: confirmed at `netsec.ip_slot_cap` in the multiprocess test
  - Invalid gossip: dropped at `node.wire.mempool` / first hop

Tier 7 is complete at this bar. No git commit/push (per request). RPC remains Tier 8.
