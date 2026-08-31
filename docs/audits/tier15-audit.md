# Tier 15 audit

**Date:** 2026-08-31  
**Scope:** 6 contracts in `docs/dependency-graph.json` → `tiers.tier_15` (`crates/observability`, `configs/grafana/dashboards.json`)  
**Rubric:** 30 correctness + 20 tests + 15 isolation + 10 docs + 15 lint/CI + 10 path fidelity = 100

Tiers 0–14 audits all report PASS (≥ 90%). `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test --workspace -- --test-threads=1` are green after this tier. `python3 scripts/gen_dependency_graph.py`: **`docs/dependency-graph.json` unchanged**.

## Part A — Per-contract scores

| Contract id | Score /100 | ≥90 |
|---|---:|:---:|
| obs.structured_logging | 94 | pass |
| obs.prometheus_exporter | 94 | pass |
| obs.otel_tracing | 94 | pass |
| obs.slo_definitions | 93 | pass |
| obs.alert_rules | 94 | pass |
| obs.grafana_dashboards | 94 | pass |

**Sum:** 563 / 600  
**Tier 15 average audit score: 93.8% — PASS**

### Notes (not blocking)

- **`obs.structured_logging`:** No earlier-tier imports (schema). JSON events use `layer::operation` naming aligned with Tier 0 `tracing.conventions`; `init_json` uses `try_init` so logging failure cannot abort the process.
- **`obs.slo_definitions` (93):** Thresholds are the §10 table (1–2s / <5s). Cross-check with `mvp.finality_lan` is `include_str` of `crates/node/tests/finality.rs` plus live simnet samples in that same test — not a second LAN harness.
- **No Prometheus HTTP server:** scrape is an in-process text exposition + `exporter_up` flag. That is enough to prove a down backend cannot stall `cons.commit`; it is not a production Bind+Pull deployment.
- **Earlier-tier files touched:** `crates/node/tests/finality.rs` and `simnet.rs` only (dev-dep `observability`). No changes to `commit`, `mesh_config`, `ensure_room`/`check_tx_bytes`, or `submit_tx` signatures.

## Part B — Tier 0–14 ↔ Tier 15 integration

### 1. Dependency-by-dependency (non-invasive)

| Contract | Dep | Hook (call, not a signature change) |
|---|---|---|
| obs.structured_logging | (none) | JSON `span`/`message` fields only |
| obs.prometheus_exporter | cons.commit | `observe_commit` → `consensus::steps::commit`; `Result` returned verbatim |
| | gossip.mesh | `observe_mesh` → `mesh_config`, `all_topics`, `gossipsub_behaviour` |
| | mempool.size_limits | `observe_size_limits` → `check_tx_bytes`, `ensure_room`; `VerifyError` unchanged |
| obs.otel_tracing | obs.prometheus_exporter | `Metrics::record_rpc_submit` / `scrape` |
| | service.l1.jsonrpc.submitTx | `submit_tx_traced` → `rpc::tx::submit_tx` |
| obs.slo_definitions | obs.prometheus_exporter | `evaluate(&Metrics)` |
| | mvp.finality_lan | same 5s / 800–2500 ms constants; live intervals recorded in `finality.rs` |
| obs.alert_rules | obs.slo_definitions | `evaluate` / `SloReport` / `for_samples` |
| obs.grafana_dashboards | obs.prometheus_exporter | panel `expr` uses `l1_*` names from `Metrics::render` |

**Earlier-tier public signatures:** none changed.

### 2. No regression / finality numbers

- **Before (Tier 14 audit):** **343** workspace tests.
- **After:** **357** (`+14` in `crates/observability`).
- **`mvp.finality_lan` (this run, instrumentation after `marks` were taken):**

| Run | time_to_first_commit_ms (mesh warmup) | intervals_ms |
|---|---:|---|
| 0 | 3468 | 1078, 1096 |
| 1 | 2138 | 1082, 1086 |
| 2 | 2136 | 1085, 1112 |

Protocol intervals are ~1.08–1.11 s (inside 1–2 s). Finality samples used for SLO are those intervals, all **< 5 s**. Recording into `Metrics` happens after the `Instant` marks, so exporter/SLO code is not on the timed path. Consensus safety tests (`consensus` 19) still pass.

### 3. Non-interference (exporter down)

`prometheus::tests::exporter_down_does_not_fail_commit`: `set_exporter_up(false)` → `scrape() == Err(ExporterDown)` while `observe_commit` still returns `Ok(Some(Finalized))` at genesis. Same test still snapshots `gossip.mesh`. `tracing::tests::traced_success_still_returns_hash` submits a tx with the exporter down. `node/tests/simnet.rs` `multiprocess_four_nodes_commit_three_runs` marks the exporter down after a real multiprocess commit and still asserts split-free tips. `node/tests/finality.rs` evaluates SLOs with the exporter down; LAN commits already happened.

### 4. SLO/alert vs real simnet

`finality_lan_three_runs_against_architecture_targets` feeds the **measured** intervals into `slo::evaluate` and `alerts::evaluate_history` (`for_samples: 3`). History of in-band samples → `AlertState::Silent`. Synthetic 8 s finality in `slo::tests::eight_second_finality_is_a_breach` → breach; three consecutive breaches fire; a blip stays silent.

### 5. Cross-boundary determinism

No `observability` import in `execution::seq` or `consensus::steps::commit`. Metrics are atomics/`try_lock` outside the hashed path. `scripts/check_no_hashmap.sh` unchanged (observability is not a consensus-critical crate).

### 6. Full workspace regression

`cargo test --workspace -- --test-threads=1`: all green, including simnet 4/4 and finality 1/1. Clippy `-D warnings` (all-targets) and `fmt --check` green.

## Part C — Non-interference verdict

- **Observability backend failure simulated: CONFIRMED chain operation (commit, propagation, tx submission) unaffected**
- **Consensus-path presence check: ZERO observability code in the `exec.app_hash` / `cons.commit` hashed computation path (CONFIRMED)**
- **Finality overhead: pre-Tier-15 finality = post-Tier-15 measured intervals (1078–1112 ms) recorded before metrics write; within target: yes**

## Part D — Overall verdict

- **Tier 15 average audit score: 93.8% — PASS**
- **Tier 0–14 integration status: CLEAN**
- **Non-interference status: CLEAN**

Tier 15 is complete on the local working tree. No git commit/push/PR. No Tier 16 work.
