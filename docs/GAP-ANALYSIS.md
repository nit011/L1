# Gap analysis

What exists is scored in `docs/audits/FINAL-SYSTEM-AUDIT.md`. This document lists what is missing, weak, or accepted without a fix.

## 1. Unimplemented schema tiers

### Tier 17 — Operations & emergency controls

Would add on-chain/off-chain **pause**, a **pause CLI**, **RBAC**, an **ops audit log**, and **config toggles** over `spec.params_registry`. That is how operators halt execution or rotate parameters without a hard fork. It is deferred on purpose: later tiers (18–19) were built with **zero** `gov.*`/`ops.*` edges so IaC and stress tests do not pretend pause exists. **Not blocking** for a local/devnet L1 that nobody must emergency-stop; **blocking** for any network that holds other people’s funds.

### Tier 20 — Roadmap (Verkle, zk, encrypted mempool, FHE, HotStuff)

Contracts: `verkle.*`, `zk.*`, `enc_mempool.*`, `fhe.*`, `cons.hotstuff.pipeline`. Architecture.md treats these as **non-gating** for MVP (forbidden to require them for liveness/safety of the current BFT+MPT+seq/STM design). They would change state commitments, validity proofs, mempool privacy, confidential execution, and pipelined consensus. **Not blocking** today’s 4-node compose or SDK e2e.

## 2. Known limitations accepted as-is

| Limitation | Why it was not “fixed” in this audit |
|---|---|
| Node binary has no JSON-RPC | `rpc` depends on `node`; a node→rpc dependency would cycle. E2e hosts Axum beside `RpcInner`. |
| Live blocks execute **seq**, not Block-STM | Tier 10 explicitly did not wire STM into `node.wire.commit`. Stress reports the gap. |
| DAS does not delay `cons.commit` | Forbidden edge; light-client sampling is additive. |
| SDK proofs are pass-through | Tier 16 trust model (b); `light.verify_account` is the independent verifier. |
| Compose is exactly 4 validators | Matches `mvp.finality_lan` / IaC; stress will not fake N>4. |
| `types` hashes with the `blake3` crate | Cannot import `crypto`. |
| Docker stress `#[ignore]` | Default CI stays fast; final pass still ran them. |
| Native Darwin `node` hashes are not reproducible | Tier 18: Linux Docker binary is the determinism proof. |
| STM/debug TPS ≪ 10k | Dual seq+STM apply, debug build, tiny n; 1s block time on the full stack. |
| unwrap/expect in seq/STM/WASM/VRF | Clippy `-D warnings` does not forbid them; converting all is a refactor, not a silent correctness bug in tests. |

## 3. Scale gaps vs architecture.md §10

| Target | Measured (this codebase, this machine) | Gap |
|---|---|---|
| Block time 1–2 s | Compose p50 **~1087 ms** | **Met** at N=4 |
| Finality &lt; 5 s | p99 **~1224 ms** | **Met** at N=4 |
| TPS 10k+ simple transfers | Compose **~1 block/s**, **~5–9 submit TPS** in short windows; debug STM **hundreds of TPS** | **Orders of magnitude below.** Bottleneck: `min_block_time_ms=1000` + sequential apply in the node + gossip admission, not a missing constant. |
| 500+ validators | **4** in compose/simnet | **~125× below** validator-count target. Next bottleneck after N: networking (gossip/QC size), VRF/leader, state growth. |
| Commodity 8–16 cores / 32–64 GB | Dev laptop + Colima | Not a 500-validator rack test. |

Likely **next** bottleneck if you chase TPS without changing consensus period: mempool→propose packing and **seq** execution of the whole block on one thread. Enabling STM on the live path would still be capped by 1-second slots unless block time or batching changes (explicitly out of scope for “don’t optimize frozen tiers”).

## 4. Security gaps

This repository **has not** had a professional external security audit, formal verification of safety/liveness, a bug bounty, or a `cargo audit` run in this environment (tool missing). Fuzzing is limited to property tests (e.g. STM vs seq). Adversarial coverage exists for some cases (equivocation, eclipse prefix caps, DAS withhold, golden rejects) but is **not** a substitute for:

- Consensus safety proofs / TLA+ or similar
- Constant-time / side-channel review of crypto wrappers
- P2P DoS at Internet scale
- WASM interpreter/host-function review beyond the current reentrancy/fuel tests
- Supply-chain review of `Cargo.lock` CVEs

**Do not treat a 93% engineering audit as a security certification.**

## 5. Operational gaps

Tied to Tier 17 and beyond Tier 15/18:

- No pause, RBAC, or ops audit trail
- Terraform in `infra/terraform/main.tf` is **illustrative** (not applied; no TLS/IAM hardening)
- Compose: no TLS, no secrets manager, in-memory+file storage (not production RocksDB)
- No RPC on validators — explorers/wallets cannot talk to compose nodes without a new sidecar
- Monitoring: Prometheus/OTEL crates exist; they are not wired as a full production stack on compose
- No documented key-ceremony, backup, or slashing-alert runbook for operators
- Node process: no graceful shutdown
- Faucet is a library + tests, not a hardened public service (rate limit exists in-process)

Someone can run a **devnet**. They cannot honestly call it a **mainnet**.
