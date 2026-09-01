# L1 — a Rust Layer-1 blockchain (devnet)

This repository is a **from-scratch Layer-1**: computers called **validators** take turns proposing **blocks** of **transactions**, vote until **more than two-thirds** agree, then lock that block in as **final**. Users send value (and optional WASM contracts) with **Ed25519** signatures. It is a **local/research network**, not a live mainnet and not a consumer wallet product.

**To run it on your machine and film a demo**, follow **[RUNBOOK.md](RUNBOOK.md)** (install → build → **one command that actually sends a transaction** → optional four Docker nodes → stop). This README explains **what happens inside** from the first keystroke of a transfer through **finality**.

---

## Table of contents

1. [What you are looking at](#1-what-you-are-looking-at)
2. [Folders (the map of the codebase)](#2-folders-the-map-of-the-codebase)
3. [Two ways the system runs (important)](#3-two-ways-the-system-runs-important)
4. [Life of a transfer: start to finality](#4-life-of-a-transfer-start-to-finality)
5. [What each crate does in that path](#5-what-each-crate-does-in-that-path)
6. [After finality: queries, proofs, light clients](#6-after-finality-queries-proofs-light-clients)
7. [Contracts, staking, data availability](#7-contracts-staking-data-availability)
8. [How Docker / four nodes fit in](#8-how-docker--four-nodes-fit-in)
9. [How to run it locally](#9-how-to-run-it-locally)
10. [Project status and further reading](#10-project-status-and-further-reading)

---

## 1. What you are looking at

In plain language:

- A **transaction** is a signed instruction: “move 77 coins from me to Alice,” or “deploy this WASM program,” or “bond stake to validator X.”
- A **block** is a batch of those instructions plus a **header** (fingerprints of the batch and of the new world state).
- A **validator** is a process that may **propose** a block and **vote**. Identity is a **BLS** key; leader choice uses a **VRF** (unpredictable weighted lottery).
- **Finality** here means: enough precommit votes (a **quorum certificate**) that the chain will **not** later pick a different block at that height. You do **not** wait for “6 confirmations” like Bitcoin.
- **State** is “who owns what,” stored in **Merkle Patricia Tries** so someone can later prove a balance without downloading every account.

Design targets (architecture.md §10): ~1–2 second blocks, finality under 5 seconds, 10k+ TPS as a **long-term** goal. **Today’s 4-node Docker net is about one block per second**, not 10k TPS. That gap is real; see `docs/GAP-ANALYSIS.md`.

---

## 2. Folders (the map of the codebase)

Everything lives in a Cargo **workspace** (`Cargo.toml`).

| Path | Everyday name | Job |
|---|---|---|
| `crates/types` | Types / spec | Addresses, amounts, headers, genesis, gas constants (`spec.rs`) |
| `crates/crypto` | Crypto | BLAKE3, Ed25519, BLS, VRF, KZG toy setup |
| `crates/state` | State | Account trie, storage trie, Merkle proofs, versioned slots |
| `crates/storage` | Disk | Put/get blocks, replay from genesis, snapshots |
| `crates/mempool` | Mempool | Admit txs (signature, nonce, fee, size, RBF), order by fee |
| `crates/execution` | Execution | Apply txs in order (`seq`); optional parallel **Block-STM** that must match seq |
| `crates/consensus` | Consensus | Propose, vote, QC, commit, WAL, checkpoints |
| `crates/network` | Network | QUIC + gossipsub, sync headers-then-bodies |
| `crates/node` | Node binary | **P2P loop**: gossip → mempool → propose/vote/commit → files. **No HTTP RPC** |
| `crates/rpc` | JSON-RPC | `l1_submitTx`, `l1_getStatus`, `l1_getAccount`, `l1_getProof`, `l1_getBlock` |
| `crates/sdk` | SDK | Sign, submit, wait for height, query proof (RPC pass-through) |
| `crates/faucet` | Faucet | Signs a real transfer from a funded genesis account |
| `crates/light` | Light client | Verify QC + Merkle account proof (do **not** blindly trust RPC) |
| `crates/da` | Data availability | Reed-Solomon chunks, DA root, fail-closed sampling |
| `crates/observability` | Metrics/logs | Prometheus / tracing helpers |
| `infra/` | Deploy | Dockerfile, docker-compose (4 validators), genesis tool, example Terraform |
| `tests/stress/` | Stress | Load against Compose; ignored by default `cargo test` |
| `docs/` | Specs & audits | Architecture, dependency graph, per-tier audits |

Contracts (named work items) are listed in `docs/dependency-graph.json` with the **exact file** that must implement them.

```
Wallet / faucet / SDK                 Validators (node processes)
        |                                      |
        |  HTTP JSON-RPC                       |  QUIC gossip (txs, votes, blocks)
        v                                      v
   crates/rpc  ----uses---->  mempool, world, store, wire_*  <---- crates/node
                                      |
                                      +--> consensus (votes, QC, commit)
                                      +--> execution seq (apply block)
                                      +--> storage (persist header+body)
```

---

## 3. Two ways the system runs (important)

This is the #1 confusion for first-time operators.

### Path A — “I sent coins and saw the balance” (what you film for a tx)

Command: `cargo test -p sdk --test e2e -- --nocapture`  
(see [RUNBOOK.md](RUNBOOK.md) Scene C).

The test process:

1. Listens on `127.0.0.1` (random port) with **Axum** (`rpc.server`).
2. Holds the same in-memory **world + mempool + store** as a node.
3. Faucet **HTTP-posts** `l1_submitTx`.
4. Runs **four BLS validators** through `node.wire.propose` → vote → precommit → **`cons.commit`**.
5. Calls `l1_getAccount` and checks balance **77**.

When the test ends, the server **stops**. That is still a **real** stack, not a fake hashmap of “pretend RPC answers.”

### Path B — “Four boxes keep making blocks”

Docker Compose (`infra/docker-compose.yml`): four `node` containers, shared `genesis.bin`, gossip. You watch `infra/data/node0/events.log` for `COMMIT n` and `infra/data/node0/tip` for height.

Those containers **do not expose JSON-RPC**. You cannot MetaMask into them. Load tests inject transactions by **gossiping** on topic `/l1/tx` (see `tests/stress`).

Why two paths? Crate **`rpc` depends on `node`**. The `node` binary **cannot** depend on `rpc` without a dependency cycle. So production-shaped processes are P2P-only until someone adds a sidecar binary. The SDK e2e **is** the sidecar, in-process.

---

## 4. Life of a transfer: start to finality

Follow **one native transfer** (the faucet drip). Same steps apply if a human called `sdk::sign_tx` instead of the faucet.

### Step 0 — Genesis (before anyone sends)

A **genesis** file lists:

- **Chain id** (devnet compose uses **18**; many tests use **1**).
- **Allocations**: which addresses start with coins (the faucet’s address is one of them).
- **Validators**: four BLS ids with voting power 1 each (devnet).
- **Parameters**: max gas, timeouts, etc.

All validators must share **identical genesis bytes**. Compose mounts one `shared/genesis.bin` read-only. If two nodes hash genesis differently, they are not the same chain.

**Code:** `types` genesis, `infra/genesis.rs` (`l1-genesis`), `node.config` on disk.

### Step 1 — Keys and address

The sender has an **Ed25519** secret key. The **address** is not Ethereum’s 20-byte keccak scheme. Here:

`address = BLAKE3(ed25519_public_key)` → 32 bytes (`crypto/src/address.rs`).

The receiver in the e2e test is another random key’s address.

### Step 2 — Build the unsigned transaction

A transfer is a structured `Tx` (`crates/types/src/tx.rs`): chain id, **nonce** (must match the account’s current nonce or it will be rejected), gas limit (`GAS_TRANSFER` = 21_000), max fee, destination, amount.

The faucet sets amount **77** in the e2e test.

### Step 3 — Sign

`sdk::sign_tx` → `crypto::tx::sign`: hash a **domain-separated** preimage (`domain.tag.apply` + BLAKE3), then Ed25519 sign. Output is a `SignedTx` envelope.

**If this step is wrong:** peers reject the tx; it never enters a block.

### Step 4 — Submit (RPC path) or gossip (Compose path)

**RPC path:** HTTP POST JSON-RPC 2.0 method **`l1_submitTx`**, params = hex of the encoded signed tx (`sdk/src/submit.rs` → `rpc/src/tx.rs`).

**Compose path:** publish the same bytes on gossip **`TOPIC_TX`** (`network` codec `GossipKind::Tx`). The node’s `wire_mempool` inserts them.

### Step 5 — Mempool admission

`mempool::Mempool::insert` (`crates/mempool`):

1. Size limit (`MAX_TX_BYTES`).
2. Minimum fee (`MIN_TX_FEE`).
3. **Verify signature** (`tx.verify_ed25519`).
4. Nonce queue / replace-by-fee if the same nonce is already queued.
5. Occupancy cap (`MEMPOOL_MAX_TXS`).

**Rejected here:** you get an RPC error such as `mempool rejected`. The SDK surfaces the **server message**, not a generic “failed.”

The tx is now **pending**: known to this node, **not** final.

### Step 6 — Leader election (whose turn to propose)

For this height/round, a **VRF** proof from a designated source validator produces a **weighted lottery** over voting power (`consensus/src/vrf.rs`). The winner is allowed to propose.

**If this is missing:** everyone proposes or a static leader is easy to DDoS.

### Step 7 — Build a block (execution)

The leader runs **`execution::builder::build_local`**:

1. Pull **ready** txs from the mempool (correct nonce order, highest fee first).
2. Respect block **max gas** and **max bytes** from genesis params.
3. Run **`execution::seq::apply_block`**: for each tx, check nonce/balance/gas, then credit/debit accounts (or deploy/call WASM, or staking). Each tx gets a **receipt** (success or reject reason).
4. Compute **Merkle roots**: transactions, receipts, **state root** (account trie + storage trie combined).
5. Compute **`app_hash`** = BLAKE3(state_root ‖ tx_root ‖ receipts_root) — frozen in golden tests (`crates/execution/tests/golden.rs`).
6. Fill the **header** (height, round, proposer, timestamp, those roots, `da_root` placeholder unless DA is applied later).

**Important:** the **running `node` binary uses sequential apply**, not Block-STM. STM (`crates/execution/src/stm`) is a parallel engine that **must match** seq; tests panic if they diverge. Parallelism is **not** what Compose is doing today.

**If a tx has a bad nonce:** it can still be **in the block** with `success = false` (see golden “rejected nonce”). State does not apply the transfer.

### Step 8 — Propose and vote (BFT rounds)

Roughly Tendermint-style (`docs/architecture.md` §2, `crates/consensus`):

1. **Propose** — leader signs a proposal (header + app_hash). Gossip on proposal topics.
2. **Prevote** — validators check the proposal (including VRF) and prevote the header hash.
3. **Precommit** — after a **polka** (2/3+ prevotes), they precommit.
4. Timeouts from genesis (`cons.timeout.config`) move the round forward if someone is offline.

Votes are **BLS** signatures. Many precommits **aggregate** into one **quorum certificate** (`bls.aggregate`, `cons.qc`).

**2/3+** is of **voting power**, not “3 of 4 machines” in the abstract — on this devnet each of 4 validators has power 1, so you need **3**.

**If only 2 of 4 are up:** the chain **halts** (no QC). That is intentional.

### Step 9 — Commit = finality

`consensus::steps::commit` (`cons.commit`):

- Check there is a precommit QC for **this** header hash.
- Record `(height, block_hash, app_hash)` in a **commit log**. A second different hash at the same height is **rejected** (safety).

Then `node.wire.commit` **writes the block to storage first**, then gossip the block to peers (`persist_then_broadcast`). On disk/files you see:

- `COMMIT n` in `events.log`
- new height in `tip`

**This is finality.** The SDK’s `wait_finality` / e2e `wait_status_http` polls **`l1_getStatus`** until a height is present (in the e2e, after `observe_finalized`).

On Compose, status is the **tip file**, not JSON-RPC. Block time is ~**1000 ms** (`min_block_time_ms`).

### Step 10 — Apply is already done

Execution happened at **propose** time on the leader; others execute/verify as they accept the block. Commit does **not** re-run STM. Catch-up nodes **replay** `seq.apply_block` from genesis or sync headers then bodies (`node.catchup`).

---

## 5. What each crate does in that path

Numbered to match the story:

| Step | Crate | Function-level (where to read) |
|---|---|---|
| 0 | `types`, `infra`, `node::config` | Genesis encode/hash; write `genesis.bin` |
| 1 | `crypto` | `ed25519.keygen`, `address.from_ed25519` |
| 2–3 | `types`, `sdk`, `crypto` | `Tx::transfer`, `sign_tx`, `tx.sign` |
| 4 | `sdk` / `rpc` or `network` + `node` | `l1_submitTx` or gossip `TOPIC_TX` |
| 5 | `mempool` | `insert` → `verify` |
| 6 | `consensus` | `weighted_leader`, `propose` |
| 7 | `execution` | `build_local`, `seq::apply_block`, `app_hash` |
| 8 | `consensus`, `network`, `node` | `wire_vote`, `wire_precommit`, gossip |
| 9 | `consensus`, `storage`, `node` | `commit`, `put_block`, `COMMIT` log |
| 10 | `rpc` or files | `get_status` / `tip` |

---

## 6. After finality: queries, proofs, light clients

**`l1_getAccount`** (`rpc/src/state.rs`): look up the address in the **account trie**. Returns balance, nonce, code hash. This is “ask the node.”

**`l1_getProof`**: same trie, plus an **MPT proof**. A full node can lie in `getAccount`; a proof lets you recompute the root.

**`sdk.query_proof`**: returns that JSON **as-is**. It does **not** call the light client. Trust model: you trust your RPC (local test) **or** you use **`crates/light`** (`verify_qc` then `verify_account`) to check the proof against a header that has a valid QC.

**`l1_getBlock`**: header and body from storage.

Without this step, users only have a server’s word. With light verify, they have **“you don’t have to trust me, check the root.”**

---

## 7. Contracts, staking, data availability

**WASM:** `Tx::deploy` / `Tx::call` (`execution/src/wasm`). Host functions **sload/sstore**. **No reentrancy** (frozen). Gas is Wasmtime **fuel**. There is no block explorer UI; `cargo test -p execution --test stm_equiv` runs mixed deploy/call/stake/transfer.

**Staking:** bond, unbond period, delegation cap, epoch set updates, slash on equivocation (`execution/src/staking.rs`, `slash.rs`). Changes who the VRF can pick next epoch.

**DA:** after commit, the node **may** erasure-code the body (4 data + 2 parity shards) and gossip chunks (`node.wire.da`). Light sampling **must fail closed** if chunks are withheld (`das.fail_closed`). Sampling is **not** allowed to delay `cons.commit` (different concern: “is the block decided” vs “can light clients download data”).

**Block-STM:** parallel speculative execution for throughput. Live node **does not** use it yet. Tests require STM receipts/app_hash **equal** seq.

---

## 8. How Docker / four nodes fit in

```
Host                          Docker network 172.28.0.0/16
----                          ---------------------------
infra/data/shared/genesis.bin --> all containers (read-only)
infra/data/node0          --> node0  172.28.0.10:4001/udp
infra/data/node1          --> node1  172.28.0.11:4002/udp
...
```

`bootstrap.sh` copies shared genesis into `/data`. Multiaddrs are **IPv4**, not DNS names (DNS-only gossip failed to form a quorum in development).

**Dockerfile:** stage 1 compiles `node` with Rust **1.93.0** and reproducible flags; stage 2 is slim Debian + the binary. Two independent Linux builds produced the **same** binary hash (Tier 18). Your Mac binary may not match that hash; that is OK.

**Terraform** under `infra/terraform/` is a **sketch**, not a production cloud. Do not paste secrets into it.

---

## 9. How to run it locally

Shortest path to a **transaction + finality + balance**:

```bash
cargo test -p sdk --test e2e -- --nocapture --test-threads=1
```

Four validators producing blocks in Docker: [RUNBOOK.md](RUNBOOK.md) §3.

Full command list, **demo video shot list**, and troubleshooting: **[RUNBOOK.md](RUNBOOK.md)**.

---

## 10. Project status and further reading

| Tiers | Meaning |
|---|---|
| **0–16, 18, 19** | Implemented (crypto → SDK → Docker → stress). Whole-system audit ~**93%**: `docs/audits/FINAL-SYSTEM-AUDIT.md` |
| **17** | Not built: pause, RBAC, ops audit log |
| **20** | Roadmap only: Verkle, zk, encrypted mempool, FHE, HotStuff — **must not** gate this MVP |

- `docs/architecture.md` — full design  
- `docs/development-plan.md` — frozen decisions  
- `docs/CONCEPTS.md` — glossary (BFT, MPT, STM, DAS, …)  
- `docs/GAP-ANALYSIS.md` — TPS, 500 validators, security review (none yet)  
- `docs/dependency-graph.json` — every contract id and file  

This software has **not** had a professional security audit. Do not put real money on it.
