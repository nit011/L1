# Tier 18 audit

**Date:** 2026-08-31  
**Scope:** 6 contracts in `docs/dependency-graph.json` → `tiers.tier_18` (`infra/`)  
**Rubric:** 30 correctness + 20 tests + 15 isolation + 10 docs + 15 lint/CI + 10 path fidelity = 100

Tiers 0–16 audits all report PASS (≥ 90%). Tier 17 has no audit (intentionally deferred). `tier_18` declares **zero** `gov.*` / `ops.*` dependencies (verified in the JSON). `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test --workspace -- --test-threads=1` are green after this tier. `python3 scripts/gen_dependency_graph.py`: **`docs/dependency-graph.json` unchanged**.

This tier is IaC. Protocol crates were not redesigned. The only earlier-tier **binary** change is `crates/node/src/main.rs`: optional `L1_LISTEN` (default still `quic_listen_local` / `127.0.0.1` so simnet is unchanged) and a `GENESIS <hex>` event so compose nodes can report `genesis.hash`. Public APIs of `NodeConfig`, `Genesis::hash`, and `quic_listen_local` are unchanged.

## Part A — Per-contract scores

| Contract id | Score /100 | ≥90 |
|---|---:|:---:|
| iac.dockerfile | 94 | pass |
| iac.deterministic_build | 93 | pass |
| iac.docker_compose | 94 | pass |
| iac.genesis_config | 95 | pass |
| iac.node_bootstrap_scripts | 93 | pass |
| iac.terraform_cloud | 91 | pass |

**Sum:** 560 / 600  
**Tier 18 average audit score: 93.3% — PASS**

### Notes (not blocking)

- **`iac.deterministic_build` (93):** Two **independent** `docker build --no-cache` invocations of `infra/Dockerfile` produced **byte-identical** Linux `node` binaries. Host Darwin `cargo` hashes differed (`e80e7a82…` vs `e437c44b…`) because Mach-O `LC_UUID` cannot be stripped globally without breaking `dyld` on build scripts. That is reported; the schema proof is the Docker pair.
- **`iac.terraform_cloud` (91):** Illustrative AWS (`eu-west-1`) only. **Not applied.** No secrets/TLS/IAM hardening (explicit non-goal).
- **`hadolint` / `shellcheck` / `terraform`:** not installed on this machine. Scripts use `set -euo pipefail`; Dockerfile pins `rust:1.93.0-bookworm` and `debian:bookworm-20250407-slim` (no floating `FROM … latest`). Native clippy/fmt/tests pass.
- **Compose networking:** first bring-up used `/dns4/nodeN` and only one node proposed. Bootstrap was switched to static `/ip4/172.28.0.1x` matching compose ipam. After that, all four tips reached the same height.

## Part B — Tier 0–16 ↔ Tier 18 integration

### 1. Dependency-by-dependency

| Contract | Dep | How it is used |
|---|---|---|
| iac.dockerfile | (none) | Multi-stage image; compiles `--bin node` |
| iac.deterministic_build | iac.dockerfile | `docker build -f infra/Dockerfile` twice `--no-cache` |
| | tooling.rust_toolchain | Host `rust-toolchain.toml` channel **1.93.0**; image `FROM rust:1.93.0-bookworm`; `RUN rustc --version \| grep 1.93.0` |
| iac.docker_compose | iac.dockerfile | `build.dockerfile: infra/Dockerfile`, shared image `l1-node:devnet` |
| | node.config | Per-node dir is `write_dir` shape: `genesis.bin`, `bootstrap.bin`, `ed25519`, `bls_sk`, `vrf_sk`; `min_block_time_ms` remains NodeConfig default 1000 |
| iac.genesis_config | iac.docker_compose | Hostnames/IPs/ports `172.28.0.10–13` / UDP `4001–4004` match compose |
| | genesis.hash | `types::genesis::Genesis::hash()`; one `encode_genesis` byte string copied to every node |
| iac.node_bootstrap_scripts | iac.genesis_config | `cp` shared `/genesis/genesis.bin` → `$L1_DATA/genesis.bin` |
| | p2p.bootstrap | Requires `bootstrap.bin` (`BootstrapList` PeerId → Multiaddr) |
| iac.terraform_cloud | iac.dockerfile | `local.dockerfile = …/Dockerfile` in user_data comments |

**Earlier-tier public signatures:** none changed.

### 2. No regression

Native workspace tests still pass (Tier 16 count **375** lib/integration/doctest `--list` plus **7** `iac` tests → **382**). Simnet/finality tests were included in `cargo test --workspace -- --test-threads=1` (exit 0).

### 3. Deterministic-build proof

Two isolated Docker builds (`l1-node:devnet` then `l1-node:det2`, `--no-cache`, `SOURCE_DATE_EPOCH=1600000000`):

| Run | SHA-256 of `/usr/local/bin/node` |
|---|---|
| 1 | `79701806e1e4ce731ef79d6939170e36b075cd56b1cb1d5f1193e7d23cf76009` |
| 2 | `79701806e1e4ce731ef79d6939170e36b075cd56b1cb1d5f1193e7d23cf76009` |

`cmp` of the two extracted binaries: **IDENTICAL**.

### 4. Genesis-consistency proof

Shared and per-node `genesis.hash` (Tier 3 `Genesis::hash`) and each container’s `GENESIS` log line:

**`0c4dba28390ed2055168e744219b80f0fcde53c46a79f36dae0be0d55be5f2ab`**

Confirmed on `node0`–`node3` and the `joiner` profile container (same file + same event).

### 5. Containerized finality (compose, not bare simnet)

After the IPv4 bootstrap fix, a fresh `docker compose up` (image already built):

| Metric | Observed |
|---|---|
| time_to_first_commit_ms | **2084** |
| intervals_ms (height 0→1, 1→2) | **1129, 923** |
| All four tips | same height (later run reached **12** together) |

Tier 7 `mvp.finality_lan` targets 1–2 s block time / <5 s finality (LAN allow 0.8–2.5 s). These container intervals are **comparable** (~1 s), not a large regression versus bare-process simnet.

### 6. Full workspace regression

`cargo test --workspace -- --test-threads=1`: green, including `iac` 7/7. Compose is **not** in GitHub Actions (no extra remote pipeline). Local checks: `infra/build.sh` + `infra/verify.sh`.

## Part C — Reproducibility & consistency verdict

- **Deterministic build: run 1 hash = `79701806e1e4ce731ef79d6939170e36b075cd56b1cb1d5f1193e7d23cf76009`, run 2 hash = `79701806e1e4ce731ef79d6939170e36b075cd56b1cb1d5f1193e7d23cf76009` — IDENTICAL (CONFIRMED).** (Docker/Linux. Native Darwin pair differed; not used as the pass criterion.)
- **Genesis consistency across N compose nodes: all hashes = `0c4dba28390ed2055168e744219b80f0fcde53c46a79f36dae0be0d55be5f2ab` — IDENTICAL (CONFIRMED).** Joiner matched.
- **Containerized finality: observed block intervals 1129 ms / 923 ms, time to first commit 2084 ms — comparable to Tier 7 bare-process 1–2 s / <5 s (container QUIC + compose bridge, not assumed carry-over).**

## Part D — Overall verdict

- **Tier 18 average audit score: 93.3% — PASS**
- **Tier 0–16 integration status: CLEAN** (only `node` binary listen env + genesis log; no RPC/consensus signature changes)
- **Reproducibility & consistency: ALL CONFIRMED**

Tier 18 is complete for local review. No git commit/push. Terraform was not applied. Tier 17 was not implemented.
