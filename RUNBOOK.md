# Runbook — run this chain on your laptop (and film a demo)

This file is the **copy-paste operations guide**. For “how a transaction becomes final,” read [README.md](README.md).

You do **not** need a wallet app, MetaMask, or a public RPC URL. This project’s demo of “send money and see it land” is a **real HTTP JSON-RPC server + four in-process validators**, started by one test command. A **second** demo shows **four Docker validator processes** agreeing on blocks about once per second (P2P only — no JSON-RPC on those containers).

All commands are from the **repository root** (the folder that contains `Cargo.toml` and this file).

---

## What a layman should expect

| You might expect | What this repo actually does |
|---|---|
| Double-click an app and click “Send” | You run **terminal commands**. There is no GUI. |
| One `node` process that also speaks JSON-RPC | The **`node` binary is P2P only**. RPC lives in crate `rpc`. The faucet + send + balance demo uses `cargo test -p sdk --test e2e`. |
| Docker Compose = full product (wallet + RPC + chain) | Compose = **four gossip validators**. You watch `COMMIT` in log files. You do **not** `curl l1_submitTx` at those containers. |
| Instant 10,000 transactions per second | Local compose commits about **one block per second**. That is expected. |

**:** Scene A (tools) → Scene B (build) → Scene C (**the transaction**) → Scene D (optional: four Docker nodes) → Scene E (stop).

---

## 0. Prerequisites (install once)

### Hardware

- A laptop with **several GB free disk** (the first `cargo build` / Docker image can use multiple GB).
- Internet for the first Rust crate download and (optional) Docker image build.

### Software

**1. Git** — to clone the repo if you do not already have the folder.

**2. Rust 1.93.0** — this repo pins the compiler in `rust-toolchain.toml`. Install [rustup](https://rustup.rs/), then in the repo folder run:

```bash
rustc --version
```

You want a line containing **`1.93.0`**. The first `cargo` command in this folder will download that toolchain automatically if rustup is installed.

**3. (Optional but needed for the 4-node video) Docker**

- macOS: Docker Desktop **or** Colima (`colima start`).
- Check:

```bash
docker info
```

If that fails, you can still film **Scene C** (transaction via the SDK e2e test). Skip Scene D.

**4. Open a terminal** in the project root:

```bash
cd /path/to/L1
ls Cargo.toml README.md RUNBOOK.md
```

You should see those three files.

---

## 1. First-time build (Scene B in the video)

This compiles every crate. **First run can take 10–30+ minutes.** Later runs are faster.

```bash
cargo build --workspace
```

Success looks like: `Finished dev ...` with **no** `error:`.

Optional quality checks (not required to demo a tx):

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

---

## 2. THE TRANSACTION  (Scene C)

This is the **only** built-in path that:

1. Creates keys  
2. Funds an account through a **real faucet transaction**  
3. Submits over **real HTTP JSON-RPC** (`l1_submitTx`)  
4. Runs **real BFT** (four validators, 2/3+ votes, commit)  
5. Reads the balance with **`l1_getAccount`**

### Command (copy exactly)

```bash
cargo test -p sdk --test e2e -- --nocapture --test-threads=1
```


### What you must see (pass)

```
test e2e_fund_wait_finality_get_account_on_live_http ... 
sdk.e2e_integration_test REAL HTTP node: funding_tx=... height=0 ... dest=...
ok
test result: ok. 1 passed
```



- **`funding_tx=`** — hash of the signed transfer (the “transaction id”).
- **`height=`** — block height after finality (this fixture uses height **0**, the first block).
- **`dest=`** — the receiver address (32-byte hex).
- The test also asserts the RPC balance is **`"77"`**.

Example values from a real run (yours **will differ** every time because keys are random):

```
funding_tx=609962261650b20faae599ad77d3e689772bc725cce562e7c8af6fcecee097a5
height=0
dest=52630e7475c383bfe540aae47181982a8f6383351f8807d439da8f12587d0423
balance 77
```

### If it fails

- `error: could not compile` → go back to `cargo build --workspace`; free disk space.
- `connection refused` / transport errors → rare; re-run the same command.
- Test ignored or filtered → you used the wrong package; it must be `-p sdk --test e2e`.

---

## 3. Four-node network (Scene D, optional)

Use this  **several computers (containers) agreeing on a chain**.

### 3.1 Build the node image (once)

From repo root (slow the first time):

```bash
docker compose -f infra/docker-compose.yml build
```

Or: `bash infra/build.sh` (also checks deterministic hashes; needs more disk).

You want image **`l1-node:devnet`**:

```bash
docker images l1-node
```

### 3.2 Write genesis files all nodes share

```bash
cargo run -p iac --bin l1-genesis -- infra/data
```

This writes `infra/data/shared/genesis.bin` and per-node folders `node0`…`node3`. Every validator must use the **same genesis bytes** or they are on different chains.

### 3.3 Start four validators

Publish ports so a Mac/Windows host can reach QUIC (needed for stress tests):

```bash
docker compose -p l1demo \
  -f infra/docker-compose.yml \
  -f tests/stress/compose.override.yml \
  up -d --no-build
```

Check:

```bash
docker compose -p l1demo -f infra/docker-compose.yml ps
```

You should see `node0`…`node3` running.

### 3.4 Watch finality (the “blocks are happening” shot)

Wait ~5 seconds, then:

```bash
cat infra/data/node0/tip
echo "----"
tail -20 infra/data/node0/events.log
```

**:** “`tip` is the latest committed height. `COMMIT n` is written only after BFT commit — two-thirds of validators voted yes.”

Heights should **increase** if you `cat tip` again after a few seconds (about **one new height per second**).

Follow container logs:

```bash
docker compose -p l1demo -f infra/docker-compose.yml logs -f node0
```

Ctrl+C stops following logs; containers keep running.

### 3.5 Optional: automated load + numbers

```bash
cargo test -p stress --lib docker_consensus_4node_p99 -- --ignored --nocapture --test-threads=1
```

You should see a line like `stress.consensus_4node p50=... p99=... commits=...`. That test **starts and stops** its own compose project named `l1stress` — stop `l1demo` first if ports clash:

```bash
docker compose -p l1demo -f infra/docker-compose.yml -f tests/stress/compose.override.yml down
```

### 3.6 You cannot send JSON-RPC to these containers

There is **no** `http://127.0.0.1:8545`. 

---

## 4. Other useful commands 

### In-process 4-validator simnet (no Docker)

```bash
cargo test -p consensus --test simnet -- --test-threads=1
```

Expect **4 passed**. This is consensus-only (safety/liveness), not the faucet UI story.

### WASM contracts (no GUI)

```bash
cargo test -p execution --test stm_equiv -- --test-threads=1
```

Expect **6 passed**. Mixes deploy/call with transfers and staking **inside the execution engine**, not Docker.

### Full test suite (slow, ~2+ minutes)

```bash
cargo test --workspace -- --test-threads=1
```

Docker tests stay **skipped** unless you add `--ignored`.

### Full stress suite (needs Docker, ~2 minutes)

```bash
cargo test -p stress -- --ignored --nocapture --test-threads=1
```

### Single P2P process (will **not** finalize alone)

Genesis has **four** validators. One process cannot reach 2/3+ votes.

```bash
cargo run -p node -- --dir infra/data/node0
```

Use this only to show “the node binary starts and listens.” Then Ctrl+C.

---

## 5. Stop everything and wipe (Scene E)

```bash
docker compose -p l1demo -f infra/docker-compose.yml -f tests/stress/compose.override.yml --profile join down
docker compose -p l1stress -f infra/docker-compose.yml -f tests/stress/compose.override.yml --profile join down
```

Wipe runtime logs (keep genesis if you want):

```bash
rm -f infra/data/node{0,1,2,3}/events.log infra/data/node{0,1,2,3}/tip \
      infra/data/joiner/events.log infra/data/joiner/tip
```

Recreate genesis from scratch:

```bash
cargo run -p iac --bin l1-genesis -- infra/data
```

---
