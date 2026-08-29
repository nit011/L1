# Designing a Layer-1 Blockchain

**System Architecture, Consensus Design & MVP Plan (Rust)**

---

## Table of Contents

1. [Architecture Overview](#1-architecture-overview)
2. [Consensus Layer](#2-consensus-layer)
3. [Execution Layer — Parallelism Without Sacrificing Determinism](#3-execution-layer--parallelism-without-sacrificing-determinism)
4. [State Layer](#4-state-layer)
5. [Networking / P2P Layer](#5-networking--p2p-layer)
6. [Data Availability](#6-data-availability)
7. [Cryptography Choices](#7-cryptography-choices)
8. [Privacy Layer — FHE / ZK Execution](#8-privacy-layer--fhe--zk-execution)
9. [Decentralization: Hardware & Economic Design](#9-decentralization-hardware--economic-design)
10. [Design Targets](#10-design-targets)
11. [End-to-End Data Flow](#11-end-to-end-data-flow)

---

## 1. Architecture Overview

```
                         ╔═══════════════════════════════════╗
                         ║            L1 BLOCKCHAIN            ║
                         ║      Secure • Scalable • Trustless  ║
                         ╚═══════════════════╤═════════════════╝
                                              │
                ┌─────────────────────────────┼─────────────────────────────┐
                │                             │                             │
                ▼                             ▼                             ▼
          CONSENSUS LAYER               EXECUTION LAYER                DATA LAYER
          PoS / BFT Validators          Parallel TX Execution          DA / P2P Storage
          • Proposal                    • TX Validation                • Data Broadcast
          • Voting                      • TX Execution                 • Availability
          • Finality                    • State Updates                • Persistence
                │                             │                             │
                └─────────────────────────────┼─────────────────────────────┘
                                              ▼
                                    ┌────────────────────┐
                                    │  STATE TRANSITION  │
                                    │  Deterministic     │
                                    └──────────┬─────────┘
                                               ▼
                                    ╔════════════════════╗
                                    ║   FINALIZED STATE    ║
                                    ║ Immutable • Verifiable║
                                    ║      • Consistent    ║
                                    ╚════════════════════╝
```

### Core pipeline

```
┌───────────────────────────────────────────────────────────┐
│                        P2P NETWORK                          │
└───────────────────────────┬─────────────────────────────────┘
                             │
                 ┌───────────▼───────────┐
                 │   Mempool / Gossip     │  txn validation, fee
                 │                        │  prioritization, DoS
                 └───────────┬───────────┘  rate-limiting
                             │
                 ┌───────────▼───────────┐
                 │    Block Proposer      │  leader election (VRF)
                 └───────────┬───────────┘
                             │
                 ┌───────────▼───────────┐
                 │  Parallel Execution     │  optimistic concurrency,
                 │   (STM / OCC engine)    │  dependency graph
                 └───────────┬───────────┘
                             │
                 ┌───────────▼───────────┐
                 │   State Transition      │  Merkleized account/
                 │   & Commitment          │  contract state, proofs
                 └───────────┬───────────┘
                             │
                 ┌───────────▼───────────┐
                 │    BFT Consensus        │  propose → prevote →
                 │  (Tendermint-style)     │  precommit → commit
                 └───────────┬───────────┘
                             │
                 ┌───────────▼───────────┐
                 │  Data Availability      │  erasure coding +
                 │  & Finality Layer       │  sampling, checkpoints
                 └────────────────────────┘
```

> Each layer below gets its own section: the concrete algorithm, the data structures, and the attack it must resist.

---

## 2. Consensus Layer

### 2.1 Choice: PoS + BFT (Tendermint/HotStuff family), not longest-chain PoW/PoS

Longest-chain protocols (Nakamoto PoW, Ouroboros-style) give **probabilistic finality** — a transaction is more final the longer it ages, but never absolutely final.

BFT-style protocols (Tendermint, HotStuff, and derivatives used by Cosmos, Diem/Aptos, Sui) give **single-slot deterministic finality**: once 2/3+ of voting power precommits a block, it cannot be reverted without a provable equivocation.

### 2.2 Round structure

```
Height h, Round r
──────────────────
1. PROPOSE   leader (VRF-selected) broadcasts block B
2. PREVOTE   validators broadcast prevote(B) or prevote(nil)
3. PRECOMMIT if 2/3+ prevotes on B seen → broadcast precommit(B)
             else → precommit(nil), move to round r+1
4. COMMIT    if 2/3+ precommits on B seen → B is finalized,
             height h+1 begins
```

### 2.3 Leader election

Use a **Verifiable Random Function (VRF)** seeded by the previous block's finalized hash plus epoch number, weighted by stake.

- VRF beats simple round-robin because round-robin leaks the next N leaders in advance — a free target list for DoS/eclipse attacks against upcoming proposers.
- Weighted-by-stake beats unweighted because it ties proposer probability to at-risk capital (matters for slashing, below).

### 2.4 Byzantine fault handling

| Attack / Failure | Mechanism that stops it |
|---|---|
| Double-signing (equivocation) | **Slashing**: two conflicting signed votes at the same height/round are cryptographic proof of fault; slash a large % of stake + permanent tombstone. |
| Long-range attack (rewriting history from an old key) | **Weak subjectivity checkpoints**: new/offline nodes bootstrap trust from a recent, socially-verified checkpoint hash, not genesis alone. |
| Nothing-at-stake (voting on multiple forks for free) | Slashing for conflicting votes removes the economic "free option." |
| Grinding on VRF seed to bias leader selection | Seed derived from a finalized, unpredictable prior VRF output, not a validator-influenceable value like a raw timestamp. |
| Network partition / <2/3 online | Protocol **halts** (safety over liveness) rather than finalizing on a minority fork; resumes once quorum returns. |
| Censorship by proposer | Rotating leaders each height/round + (phase 2) encrypted mempool / threshold decryption so the proposer can't selectively drop txs by content. |
| Validator centralization via many small validators | Minimum self-bond + delegation cap per validator, so voting power can't be laundered around slashing accountability. |

### 2.5 Validator lifecycle

```
Bond stake → Join validator set (next epoch)
     → Propose / Vote each height
     → (optional) Get slashed on fault
     → Signal unbond → Unbonding period (e.g. 21 days)
     → Stake withdrawable
```

```
Validators → Stake → Validator Selection → Proposal
   → Vote / Pre-commit → Quorum (2/3+ voting power) → Finality
```

**Core mechanisms checklist:**
- Slashing
- Double-sign protection
- Validator rotation
- Unbonding period
- Stake delegation
- Byzantine fault handling
- Weak-subjectivity / checkpoints where appropriate
- Secure randomness for validator selection

---

## 3. Execution Layer — Parallelism Without Sacrificing Determinism

### 3.1 Why not "just increase block size"

Bigger blocks raise bandwidth and state-growth requirements linearly, which raises the hardware floor for a validator — a direct, mechanical decentralization loss.

```
Increase block size → More transactions/block → Higher TPS   (naive)

Large blocks → More bandwidth → Fewer machines can validate → Centralization  (real cost)
```

### 3.2 Parallel execution model

```
Block
 ├── TX1 → Account A
 ├── TX2 → Account B
 ├── TX3 → Account C
 ├── TX4 → Account D
 │
 ▼
Dependency Analysis
 ├── Independent → Execute in parallel
 └── Conflicting → Execute sequentially
```

```
Transaction Pool
      │
      ▼
Dependency Detection
      │
      ▼
   Scheduler
      │
 ┌────┼────┬────┐
CPU1 CPU2 CPU3 CPU4
 └────┼────┴────┘
      │
      ▼
State Validation
      │
      ▼
    Commit
```

### 3.3 Optimistic concurrency / Software Transactional Memory (STM)

```
Transaction batch
      │
      ▼
Static/declared read-write set per tx (accounts touched)
      │
      ▼
Build conflict graph: edge if txA.writes ∩ txB.(reads ∪ writes) ≠ ∅
      │
      ▼
Schedule independent txs onto worker threads (N = CPU cores)
      │
      ▼
Execute optimistically in parallel
      │
      ▼
Validate: did any tx read a value another parallel tx overwrote?
   ├─ No conflict → commit result, keep in final block order
   └─ Conflict    → re-execute sequentially, re-validate
```

This is the **Block-STM** approach (used by Aptos/Diem; conceptually similar to Solana's Sealevel with explicit account locking).

### 3.4 Getting the read/write set — two approaches

| Approach | Pro | Con |
|---|---|---|
| Explicit declaration (Solana-style) | Scheduler knows conflicts before executing anything — highest parallelism | Pushes complexity to SDK layer; wrong declarations = runtime failure |
| Speculative + optimistic validation (Block-STM style) | Transparent to app/VM layer, no special SDK requirement | Wasted work on misspeculation under high contention |

**Recommended default:** speculative/optimistic (Block-STM style).

### 3.5 The unavoidable bottleneck: hot accounts

Any account touched by many transactions in one block (a popular AMM pool, a stablecoin's global supply counter) **serializes regardless of engine**, because those transactions genuinely conflict. Parallel execution raises average-case throughput, not worst-case throughput — design should assume hot-account contention will exist.

---

## 4. State Layer

### 4.1 Merkleized state

```
                State Root (published in block header)
                        │
         ┌──────────────┴──────────────┐
    Accounts Trie                 Contracts Trie
         │                               │
   ┌─────┴─────┐                  ┌─────┴─────┐
 Acct A      Acct B           Code+Storage  Code+Storage
 balance     balance            (contract1)   (contract2)
 nonce       nonce
```

Use a **Merkle Patricia Trie (MPT)** as the authenticated data structure baseline, with a **Verkle tree** as the forward-looking upgrade.

#### Merkle Patricia Trie — mechanics

The MPT combines a radix/Patricia trie (path-compressed by shared key prefixes) with Merkle hashing at every node, so lookups are `O(key length)` and the root hash commits to the entire state.

```
Node types:
  Leaf      — [encoded_path, value]                     (end of key)
  Extension — [shared_nibble_prefix, next_node_hash]     (path compression)
  Branch    — [16 child_slots (one per nibble) + value]  (fan-out point)

Key lookup:
  key (bytes) → nibbles (hex) → walk trie nibble-by-nibble
  → Extension nodes skip shared prefixes
  → Branch nodes select the next nibble's child
  → Leaf node returns the value

Proof of inclusion:
  path of node hashes from leaf → root
  verifier only needs: (leaf value, sibling hashes, root hash)
  → does NOT need the rest of the state
```

Why it matters here:
1. **Light clients** — any node can prove a specific account's value against the block header's state root without holding full state.
2. **Cheap sync/diffing** — state transitions are content-addressed, so only changed subtrees need to be re-hashed and transmitted.
3. **Downstream proofs** — fraud/validity proofs (Section 7/8) require this authenticated structure to exist.
4. **Verkle upgrade path** — replacing the 16-ary branch hashing with vector commitments (Pedersen/KZG) shrinks proof size from `O(log₁₆ n) × 32 bytes` per branch to near-constant size, at the cost of needing a trusted setup or newer crypto assumptions.

### 4.2 State growth is a decentralization problem, not just an execution problem

Unbounded state growth eventually forces validators onto enterprise-grade storage — the same centralization failure as oversized blocks.

**Mitigations:**
- State rent / storage-cost-reflecting gas pricing
- Periodic state expiry/archival with a re-activation path (Merkle proof of prior state)
- Separating "hot" recent state from "cold" archival state most validators don't keep online

### 4.3 Versioned state for parallel execution

Each account/storage slot is versioned (`state_n → state_n+1`). The STM engine reads a specific version and validates at commit time that the version it read is still latest at that point in the schedule — this is how the optimistic-concurrency validation in 3.3 is implemented at the data-structure level.

---

## 5. Networking / P2P Layer

| Concern | Design |
|---|---|
| Transport | libp2p over **QUIC** (multiplexed streams, built-in encryption, better head-of-line-blocking behavior than TCP for gossip fan-out) |
| Peer discovery | Kademlia DHT + bootstrap nodes; validators also maintain a dedicated low-latency mesh with expected next-round peers to keep the consensus critical path off general gossip |
| Block/tx propagation | gossipsub + erasure-coded block chunks so a node can reconstruct a block from any sufficient subset of peers — reduces propagation-time variance, which matters for short block times |
| Mempool DoS resistance | per-peer rate limiting, fee-based prioritization, replace-by-fee rules, minimum gas price floor |
| Eclipse-attack mitigation | bound the fraction of a node's peer slots from a single IP range/ASN; periodic peer rotation |

---

## 6. Data Availability

Consensus can finalize a block header cheaply, but finality is meaningless if the block's transaction data isn't actually retrievable — a malicious or offline proposer could finalize a header while withholding the data behind it (**the data availability problem**).

```
Block data
    │
    ▼
Erasure coding (Reed-Solomon): split into k data + m parity chunks,
      any k of (k+m) reconstruct the block
    │
    ▼
Distribute chunks across the network
    │
    ▼
Data Availability Sampling: light nodes randomly request a handful
      of chunks; if enough independent samples succeed, the full
      block is available with high probability WITHOUT any single
      node downloading all of it
```

---

## 7. Cryptography Choices

| Component | Choice | Why |
|---|---|---|
| Validator signatures | BLS12-381 | Aggregatable — 500+ validator signatures compress into one, keeping block headers small regardless of validator-set size |
| Leader election randomness | VRF (e.g. ECVRF) | Unpredictable but publicly verifiable; prevents proposer-grinding and pre-targeted DoS |
| State commitments | Merkle (Keccak/Blake3) → Verkle (Pedersen/KZG) later | Merkle first for simplicity/tooling maturity; Verkle later for ~O(1) proof sizes |
| Long-term verification scalability | zk-SNARK/STARK validity proofs (roadmap) | Lets light clients and other validators verify a state transition without re-executing every transaction |

---

## 8. Privacy Layer — FHE / ZK Execution

> Roadmap layer — not required for MVP, but the architecture below should be able to slot this in without a redesign.

### 8.1 Why privacy needs its own layer

Standard smart-contract execution is fully transparent: every input, every intermediate state, and every output is visible to anyone re-executing the block. That's fine for most DeFi primitives but breaks down for confidential balances, sealed-bid auctions, private voting, or any workflow where **the computation must happen without revealing the inputs** — even to the validators executing it.

Two complementary tools solve different halves of this:

| Tool | Solves | Cost |
|---|---|---|
| **ZK proofs** (SNARK/STARK) | Prove a computation was done correctly *without re-executing it* — verification is cheap, but the prover still needs to see the plaintext inputs to generate the proof | Proof generation is expensive; verification is cheap |
| **FHE** (Fully Homomorphic Encryption) | Let a computation run *directly on encrypted data* — the executor never sees plaintext at all | Homomorphic operations are orders of magnitude slower than plaintext execution |

### 8.2 FHE execution pipeline (confidential contracts)

```
User encrypts inputs client-side (public key of network's threshold FHE scheme)
      │
      ▼
Encrypted transaction submitted to mempool
      │
      ▼
Validators execute the contract's FHE circuit DIRECTLY on ciphertexts
      (add/mul/compare gates evaluated homomorphically — no decryption)
      │
      ▼
Result remains ciphertext; committed to state as an encrypted value
      │
      ▼
Threshold decryption: only when a party proves authorization
      (e.g. the account owner, or a public-output branch of the contract)
      → t-of-n validator committee jointly decrypts, no single validator
        ever holds the full decryption key
```

### 8.3 Practical integration notes

- **Scheme choice:** TFHE or CKKS-family schemes are the current practical options; TFHE suits boolean/integer circuits (account balances, comparisons), CKKS suits approximate real-number arithmetic.
- **Where it sits in the pipeline:** FHE execution is its own lane parallel to the plaintext Block-STM lane (Section 3) — encrypted-contract transactions are routed to FHE-capable executor nodes, not the general parallel scheduler, since ciphertext operations don't have a meaningful "read/write set" for conflict detection the same way plaintext ones do.
- **Key management:** a **threshold key** for the network's FHE public/private keypair, generated via distributed key generation (DKG) among a validator committee, mirrors how BLS threshold signatures already work in Section 7 — no single validator (or the deployer) should ever hold the full private key.
- **Combine with ZK:** validators can additionally attach a succinct proof that the FHE circuit was evaluated correctly, so light clients don't need to trust the executing committee's homomorphic arithmetic blindly — this is the "confidential + verifiable" combination several current research L1s (Fhenix, Inco, Zama's fhEVM) are converging on.
- **Cost reality check:** FHE gas costs are currently 100–10,000x plaintext gas depending on circuit depth. Treat this as an opt-in execution lane for specific confidential contracts, not the default VM.

---

## 9. Decentralization: Hardware & Economic Design

### 9.1 Design principle

Optimize for the **weakest machine you still want to be able to validate on**, and derive every other number (block size, gas limits, state growth rate) from that hardware target — not the other way around.

| Resource | Target validator spec | Rationale |
|---|---|---|
| CPU | 8–16 cores, commodity server-grade | Enough for parallel execution scheduler without requiring a data-center-only chip |
| RAM | 32–64 GB | Fits hot-state working set in memory for the STM engine |
| Storage | 1–2 TB NVMe SSD, pruning/archival split | Recent state + recent blocks only; historical data pushed to optional archive nodes |
| Network | 100 Mbps–1 Gbps symmetric | Realistic for a well-connected home/office/small-datacenter line |
| Validators (target) | 500+ at mainnet maturity | Large enough that no small coalition casually reaches 1/3 (liveness) or 2/3 (safety) voting power |

### 9.2 Economic decentralization levers

- Minimum self-bond per validator (prevents one operator running many nominally-separate validators to farm delegation)
- Soft cap on voting power per validator (delegation beyond the cap stops earning proposer weight)
- Slashing + tombstoning severe enough to matter economically, with a **narrow, precisely defined trigger set** (equivocation, provable downtime) — not vague "malicious behavior" clauses

---

## 10. Design Targets

| Metric | Target | Notes |
|---|---|---|
| Block time | 1–2 sec | Lower bound set by network propagation + BFT round latency |
| Time to finality | < 5 sec | Single-slot deterministic finality under the BFT model (Section 2) |
| Validator count at maturity | 500+ | See 9.1 |
| TPS | 10k+ (simple transfers), lower for heavy contract calls | Depends heavily on contention/hot-account rate |
| Hardware floor | Commodity server (Section 9.1) | The number you optimize everything else around |
| Execution model | Parallel, optimistic (Block-STM style) | Section 3 |
| State model | Merkleized (MPT → Verkle), versioned, prunable | Section 4 |
| Consensus | PoS + BFT (Tendermint/HotStuff family) | Section 2 |
| Privacy | FHE-capable confidential execution lane (roadmap) | Section 8 |
| Cross-chain | Light-client / IBC-style verification | Leverages Merkleized state + BLS-aggregated signatures |

---

## 11. End-to-End Data Flow

```
                    ┌─────────────────┐
                    │      CLIENT       │
                    └────────┬─────────┘
                             │ JSON-RPC
                    ┌────────▼─────────┐
                    │        RPC        │
                    └────────┬─────────┘
                             │
                    ┌────────▼─────────┐
                    │      MEMPOOL       │
                    └────────┬─────────┘
                             │
                    ┌────────▼─────────┐
                    │   BLOCK BUILDER    │
                    └────────┬─────────┘
                             │
                    ┌────────▼─────────┐
                    │ PARALLEL EXECUTOR  │  ← plaintext lane (Block-STM)
                    │  + FHE EXEC LANE   │  ← confidential lane (Section 8)
                    └────────┬─────────┘
                             │
                    ┌────────▼─────────┐
                    │   STATE ENGINE     │
                    │  MPT/Verkle + DB   │
                    └────────┬─────────┘
                             │
                    ┌────────▼─────────┐
                    │    CONSENSUS       │
                    │    PoS + BFT       │
                    └────────┬─────────┘
                             │
                    ┌────────▼─────────┐
                    │     FINALITY       │
                    └────────┬─────────┘
                             │
                    ┌────────▼─────────┐
                    │  BLOCK STORAGE     │
                    └────────┬─────────┘
                             │
              ╔══════════════▼══════════════╗
              ║        P2P NETWORK           ║
              ║  TX │ BLOCK │ VOTE │ SYNC     ║
              ╚═══════════════════════════════╝
```