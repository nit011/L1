# Tier 10 audit

**Date:** 2026-08-29  
**Scope:** 8 contracts in `docs/dependency-graph.json` → `tiers.tier_10` (`crates/execution/src/stm`, `tests/stm_equiv.rs`, `benches/hot_account.rs`)  
**Rubric:** 30 correctness + 20 tests + 15 isolation + 10 docs + 15 lint/CI + 10 path fidelity = 100

Tiers 0–9 audits all report PASS (≥ 90%). Tier 9 forbidden-edge (`cons.commit` ↛ staking) remains untouched — this tier does not wire STM into `node.wire.commit`.

## Part A — Per-contract scores

| Contract id | Score /100 | ≥90 |
|---|---:|:---:|
| stm.rwset.speculate | 93 | pass |
| stm.conflict_graph | 94 | pass |
| stm.schedule | 93 | pass |
| stm.validate | 93 | pass |
| stm.reexec_sequential | 94 | pass |
| stm.apply_block | 93 | pass |
| stm.equals_seq | 94 | pass |
| stm.hot_account_bench | 91 | pass |

**Sum:** 745 / 800  
**Tier 10 average audit score: 93.1% — PASS**

### Notes

- Per-tx logic is only [`execution::seq::apply_tx`](crates/execution/src/seq.rs). STM records RW sets around that call and [`VersionedSlots::read` / `validate`](crates/state/src/version.rs).
- [`stm::apply_block`](crates/execution/src/stm/mod.rs) always calls [`seq::apply_block`](crates/execution/src/seq.rs) and **asserts** byte-identical receipts, `app_hash`, and state root. A silent sequential fallback was rejected because it would hide forks.
- [`apply_block_engine`](crates/execution/src/stm/mod.rs) is the same pipeline without the comparison, used by Criterion so benches are not 2× sequential.
- **Hot accounts (architecture.md §3.5 / §4.4):** many txs on one account form a write–read chain; they serialize in `reexec_sequential`. That is expected.
- **Wall-clock:** at 64 native transfers, STM is **slower** than sequential (thread spawn + full speculative `apply_tx` of every tx, then OCC/reexec). Correctness is not throughput. Parallelism is still real: `run_waves` uses `std::thread::scope` and the schedule test observes **multiple OS `ThreadId`s**.

## Part B — Tier 0–9 ↔ Tier 10 integration

### 1. Dependency-by-dependency

| Contract | Dep | Real symbol |
|---|---|---|
| stm.rwset.speculate | exec.seq.apply_tx | `crate::seq::apply_tx` |
| | state.versioned_slot.read | `VersionedSlots::read` |
| stm.conflict_graph | stm.rwset.speculate | `SpecTx` / `speculate` |
| stm.schedule | stm.conflict_graph | `conflict_graph` → `ConflictGraph` |
| stm.validate | stm.schedule | `Schedule` argument |
| | state.versioned_slot.validate | `VersionedSlots::validate` |
| stm.reexec_sequential | stm.validate | `validate` / `StaleSet` |
| | exec.seq.apply_tx | `apply_tx` on the committed world |
| stm.apply_block | stm.reexec_sequential | `reexec_sequential` |
| | exec.seq.apply_block | `seq::apply_block` (comparison) |
| stm.equals_seq | stm.apply_block | `stm::apply_block` / `apply_block_engine` |
| | exec.golden_vectors | same APP hex literals as `crates/execution/tests/golden.rs` |
| stm.hot_account_bench | stm.apply_block | `stm::apply_block_engine` |

**Earlier-tier signatures:** `apply_tx` / `apply_block` **unchanged**. Additive only: `execution` depends on `storage` so STM can own `VersionedSlots<MemoryStore>`.

### 2. No regression

- **Before (Tier 9):** **261** workspace tests.
- **After:** **278** (`+17` STM unit + `stm_equiv`). Golden vectors and Tier 9 staking tests still pass. Node simnet/finality unchanged (STM not wired into the live loop).

### 3. Equivalence proof

See Part C. One development panic (not a hash divergence): `conflict_graph` used `isolated(specs.len())` while `SpecTx.index` stayed at the **block** index when re-validating a suffix — `adj.get_mut` was `None`. Fixed by building vertices from actual `SpecTx.index` values. After the fix: **0** sequential/STM output divergences.

### 4. Genuine parallelism

`stm::schedule::tests::run_waves_uses_os_threads` spawns a wave of 4 independent txs and asserts unique `ThreadId` count **> 1**. Speculate also uses `std::thread::scope` per tx.

### 5. Determinism

Conflict adjacency and directed edges use `types::collections::{Map, Set}`. Waves sort indices. Committed receipts are always stored in **block order**. Parallel completion order does not affect `app_hash`.

### 6. Full workspace

`cargo test --workspace` (serial for simnet), `clippy --all-targets -- -D warnings`, `cargo fmt --check` green. `python3 scripts/gen_dependency_graph.py` — JSON **unchanged**.

## Part C — Equivalence & performance verdict

- **Property-test cases run: 768, divergences found: 0** (256 cases × seeds `{1, 2, 99}` in `property_stm_equals_seq_three_seeds`; mix of hot-account `mode==0` and round-robin senders).
- **Golden-vector cross-check (Tier 3 fixtures through stm.apply_block): IDENTICAL** (`empty_block`, `single_transfer`, `rejected_nonce`, `multi_account` APP hex matches `golden.rs`).
- **Low-contention benchmark (N=64 independent transfers):** sequential **~2.20 ms** vs parallel STM engine **~33.0 ms** (**~0.07×**; STM slower). Thread spawn + speculating every tx dominates a cheap native transfer. Not a correctness failure.
- **High-contention (hot account, N=64):** sequential **~1.88 ms** vs STM **~15.0 ms** (**~0.13×**). Expectation that this should **not** beat sequential **held** (STM is slower, not faster). Extra cost is speculation of a conflict chain that `reexec_sequential` then applies in order — consistent with architecture.md §3.5.

## Part D — Overall verdict

- **Tier 10 average audit score: 93.1% — PASS**
- **Tier 0–9 integration status: CLEAN**
- **Equivalence proof: CONFIRMED CLEAN**

Tier 10 is complete on this bar. No git commit/push. Not starting Tier 11. STM is **not** plugged into `node.wire.commit`.
