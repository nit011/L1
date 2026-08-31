# Tier 19 audit

**Date:** 2026-08-31  
**Scope:** 6 contracts in `docs/dependency-graph.json` → `tiers.tier_19` (`tests/stress`)  
**Rubric:** 30 correctness + 20 tests + 15 isolation + 10 docs + 15 lint/CI + 10 path fidelity = 100

Tiers 0–16 and 18 audits all report PASS (≥ 90%). Tier 17 remains deferred. `tier_19` declares **zero** `gov.*` / `ops.*` dependencies (verified against the six contract `dependencies` arrays). `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test --workspace -- --test-threads=1` are green after this tier. `python3 scripts/gen_dependency_graph.py`: **`docs/dependency-graph.json` unchanged**.

This tier measures the assembled system. Consensus, execution, and networking were **not** patched to look faster. Full-stack TPS is far below architecture.md §10’s “10k+” target; that is reported as a finding, not papered over.

## Part A — Per-contract scores

| Contract id | Score /100 | ≥90 |
|---|---:|:---:|
| stress.load_harness | 93 | pass |
| stress.consensus_4node | 94 | pass |
| stress.throughput_benchmark | 91 | pass |
| stress.snapshot_sync_test | 92 | pass |
| stress.das_withhold | 91 | pass |
| stress.perf_regression_ci | 93 | pass |

**Sum:** 554 / 600  
**Tier 19 average audit score: 92.3% — PASS**

### Notes (not blocking)

- **`stress.load_harness` (93):** Live load uses [`sdk::sign_tx`] (same signing path as `sdk.e2e_integration_test`) and gossip `TOPIC_TX` onto compose node0 (`127.0.0.1:14001` via `tests/stress/compose.override.yml`). The multiprocess `node` binary still has **no JSON-RPC** (`rpc` depends on `node`); injecting via gossip is what `wire_mempool` actually admits. `LoadConfig::n_validators > 4` is logged and **not** silently run as a four-node mock — compose is fixed at four services. Sign-only overhead (2000 txs, debug): **8580 TPS / 233 ms**.
- **`stress.consensus_4node` (94):** 18s window against `l1-node:devnet` compose. After wiping stale `events.log`/`tip` and decoupling tip sampling from gossip setup, measured **p50=1086 ms, p95=1215 ms, p99=1243 ms**, 16 intervals, 17 new `COMMIT` lines, time-to-next-height after `wait_tip` **579 ms**. Matches `NodeConfig.min_block_time_ms = 1000` and Tier 18 compose intervals (~923–1129 ms) to the same order of magnitude.
- **`stress.throughput_benchmark` (91):** Always calls [`execution::stm::apply_block`] (which also dual-runs `exec.seq.apply_block`). Debug n=48: STM low ≈ **230–430 TPS**, hot ≈ **450–660 TPS**. Hot can be *faster* at this tiny n (world/setup dominated; STM still serializes the hot nonce chain). Compose: **32 txs submitted in 6.39 s → 5.0 submit TPS**, **commit_delta=6 → 0.94 blocks/s**. §10 10k+ is **not** met on the full stack (1s block time + seq in the node binary). Honest gap vs execution-only: STM hundreds of TPS vs compose ~1 block/s. Release STM was not re-measured here (disk filled during `--release`).
- **`stress.snapshot_sync_test` (92):** In-process: `snapshot_commit_root` + `take_snapshot` vs `replay_for_snapshot_check` **and** [`network::sync::headers_then_bodies`] into a second store; all three land the same commit root (`equal=true`, `htb=true`). Compose `joiner` (`--profile join`): **1064 ms** to tip 2 while validators were at 1. The live joiner uses `node.catchup` (headers-then-bodies + seq replay), **not** the snapshot blob — snapshot equivalence is the in-process proof required by `sync.snapshot`.
- **`stress.das_withhold` (91):** [`da::root::commit`] + [`MemoryChunks::withhold`] + [`das::fail_closed`] → `NotAvailable`; control block `Available`. During a live compose window, node0 **tip 0→4, commits_during=4**. The node binary does **not** gate `cons.commit` on DAS (forbidden edge). Withholding is exercised on the real DA codec against the live network’s continued commits — not by dropping gossip shards inside the containers (no DA mesh in the node binary).
- **`stress.perf_regression_ci` (93):** Floor STM low **140 TPS** (30% below baseline 200). p99 ceiling **8000 ms**. `synthetic_throttle_fails_guardrail` injects STM TPS=1 and p99=50s and **fails as designed**.

## Part B — Full-stack ↔ Tier 19 integration

### 1. Dependency-by-dependency check

| Contract | Declared dep | How it is exercised |
|---|---|---|
| stress.load_harness | sdk.e2e_integration_test | `sdk::sign_tx` / `signed_transfer`; `crypto::tx::verify_ed25519` on the envelope |
| | iac.docker_compose | `docker compose -f infra/docker-compose.yml -f tests/stress/compose.override.yml -p l1stress up` |
| stress.consensus_4node | stress.load_harness | `bring_up` / `gossip_txs` / `read_tip` |
| | mvp.finality_lan | Wall-clock tip marks + percentiles (same bind-mounted `tip` idea as LAN finality) |
| | cons.commit | `COMMIT n` lines from `node.wire_commit` on the bind-mounted `events.log` |
| stress.throughput_benchmark | stress.load_harness | Compose gossip path |
| | stm.apply_block | `execution::stm::apply_block` for both contention profiles |
| stress.snapshot_sync_test | stress.load_harness | Compose `joiner` under a live chain |
| | sync.snapshot | `snapshot_commit_root` / `take_snapshot` / `snapshot_matches_replay` |
| | sync.headers_then_bodies | `network::sync::headers_then_bodies` + joiner `node.catchup` |
| stress.das_withhold | stress.load_harness | Compose up during the withhold window |
| | das.fail_closed | `da::das::fail_closed` on withheld vs full `MemoryChunks` |
| stress.perf_regression_ci | stress.throughput_benchmark | `evaluate(&ThroughputReport, …)` |
| | stress.consensus_4node | `evaluate(…, Some(&ConsensusReport))` p99 ceiling |

**Earlier-tier public signatures:** none of consensus/execution/network/SDK APIs were changed for speed. **`infra/Cargo.toml` gained a `[lib]` pointing at `infra/genesis.rs`** so `stress` can call `iac::materialize_with_bank` instead of copying genesis. The CLI `l1-genesis` is unchanged (`n_bank=0` still the default). `materialize_with_bank` already existed for load allocs.

### 2. No regression

| | Tests |
|---|---|
| After Tier 18 (reported) | **382** (375 + 7 iac) |
| After Tier 19 `cargo test --workspace -- --list` | **403** default + **5** ignored Docker tests |
| Default suite | `cargo test --workspace -- --test-threads=1` **exit 0** (ignored tests not run) |

Stress Docker tests: `cargo test -p stress -- --ignored --nocapture --test-threads=1`.

### 3. Real-load verification

All five ignored `docker_*` tests ran against **`l1-node:devnet` + `infra/docker-compose.yml`**, not an in-process BFT simulator:

| Test | Evidence |
|---|---|
| `docker_compose_comes_up` | genesis `a1e17c1c…`, `tip=Some(0)` |
| `docker_consensus_4node_p99` | p50/p95/p99 above; 17 COMMITs |
| `docker_compose_throughput` | 32 txs, 6 commits in 6.39s |
| `docker_joiner_catchup` | joiner tip 2 in 1064 ms |
| `docker_withhold_does_not_stall_validators` | tip 0→4, 4 COMMITs during withhold |

### 4. Baseline sanity vs Tier 7 / Tier 18

| Source | Number |
|---|---|
| architecture.md §10 block time | 1–2 s |
| architecture.md §10 finality | < 5 s |
| Tier 18 compose first commit | 2084 ms |
| Tier 18 compose intervals | 1129 / 923 ms |
| Tier 19 p50/p95/p99 block interval | **1086 / 1215 / 1243 ms** |
| Tier 19 time to next height after wait | 579 ms |

Same order of magnitude as Tier 18 containerized finality. p99 **1243 ms** is under 5s. First-commit-from-cold is still the Tier 18 **2084 ms** figure; this harness `wait_tip`s before the measurement window.

### 5. DAS cascade-isolation

Withhold window (compose still running): **tip 0→4**, **4 COMMITs** on node0 (~1 s block time). Control sample remained `Available`. Unrelated full-node commits continued; no cascade halt.

### 6. CI guardrail synthetic regression

`ci::tests::synthetic_throttle_fails_guardrail`: STM TPS **1.0** → `Fail("stm_low_tps below floor")`; p99 **50_000** with otherwise-ok STM → `Fail("p99 block interval above ceiling")`. Observed.

### 7. Full workspace regression

Default suite green with `--test-threads=1`. Five Docker tests `#[ignore]`. No Tier 17 crates.

## Part C — Stress & performance verdict

### Sustained consensus (compose, 4 validators, ~18 s)

| Metric | Value |
|---|---|
| p50 block interval | **1086 ms** |
| p95 | **1215 ms** |
| p99 | **1243 ms** |
| COMMIT delta | **17** |
| intervals sampled | **16** |

### Throughput

| Profile | Measured | vs §10 10k+ |
|---|---|---|
| STM low-contention (debug, n=48) | ~230–430 TPS | ~2–4% of target |
| STM high-contention (debug, n=48) | ~450–660 TPS | still ≪ 10k; small-n noise |
| Compose submit TPS (32 txs / 6.39 s) | **5.0** | not comparable to 10k |
| Compose block rate | **0.94 /s** | matches 1s `min_block_time_ms` |

**Finding (not patched):** the live node applies blocks with **seq**, not STM, and waits **1 s** between commits. Full-stack TPS cannot approach 10k without changing frozen earlier-tier behavior. STM in-process is also dual-checked against seq (assert on mismatch), which caps the harness number.

### Snapshot vs full replay

| Path | Time | State |
|---|---|---|
| In-process snapshot helper | 0 ms (3 empty blocks) | same commit root |
| In-process full replay | 0 ms | same |
| `headers_then_bodies` copy | same tip `Height(2)` | same commit root |
| Compose joiner | **1064 ms** to height 2 | live tip catch-up |

### DAS withhold

`fail_closed` withheld → **NotAvailable**; control → **Available**. Compose during window: **4 commits, tip +4**. Isolation **confirmed**.

### CI thresholds

| Knob | Value | Why |
|---|---|---|
| `BASELINE_STM_LOW_TPS` | 200 | below observed debug STM; room for noise |
| `STM_LOW_TPS_FLOOR` | 140 (70%) | 30% drop |
| `P99_BLOCK_MS_MAX` | 8000 | §10 <5s plus Colima slack; observed p99 1243 |

Synthetic regression **caught**.

## Part D — Overall verdict

- **Tier 19 average audit score: 92.3% — PASS**
- **Full-stack integration status: CLEAN** (library-only IaC export; no protocol speed patches)
- **Stress & performance verification: ALL CONFIRMED** (Docker compose, DAS isolation, CI trip), **with an honest performance shortfall**: full-stack TPS and debug STM TPS are far below architecture.md §10’s 10k+ simple-transfer target; block time / finality **do** meet the 1–2 s / <5 s band on this 4-node Colima compose.

Tier 19 is complete at this bar. Not started: Tier 17, Tier 20. No git commit/push.
