# Tier 12 audit

**Date:** 2026-08-31  
**Scope:** 8 contracts in `docs/dependency-graph.json` → `tiers.tier_12` (`crates/da/src/chunk.rs`, `root.rs`, `das.rs`; `crates/types/src/header.rs`; `crates/network/src/topics.rs`; `crates/node/src/wire.rs`)  
**Rubric:** 30 correctness + 20 tests + 15 isolation + 10 docs + 15 lint/CI + 10 path fidelity = 100

Tiers 0–11 audits all report PASS (≥ 90%). Step 0 workspace `cargo test` had one pre-existing flake (`node` simnet `late_join_fifth_node_catchup`); a subsequent full `--workspace` run including that test was green. Clippy `-D warnings` and `cargo fmt --check` are green.

## Part A — Per-contract scores

| Contract id | Score /100 | ≥90 |
|---|---:|:---:|
| da.chunk.split | 94 | pass |
| da.chunk.reconstruct | 95 | pass |
| da.root | 94 | pass |
| header.da_root | 92 | pass |
| gossip.da_chunks | 93 | pass |
| das.sample | 93 | pass |
| das.fail_closed | 94 | pass |
| node.wire.da | 94 | pass |

**Sum:** 749 / 800  
**Tier 12 average audit score: 93.6% — PASS**

### Notes (not blocking)

- **`k=4`, `m=2`:** 50% RS overhead; any 2 of 6 shards may be missing. Shard *count* is fixed; shard *length* is `O(body_len / k)` so a future `limits.max_block_bytes` has a linear handle.
- **`header.da_root` (92):** `types` cannot depend on `da`. [`apply_da_root`](crates/types/src/header.rs) takes the Merkle `Hash` from `da.root`; the real `da::root::commit` call is in `da` / `node` tests and `wire_da`. Layering, not a stub.
- **`das.sample` (93):** `da` cannot import `network` (`network` already depends on `da`). Sampling uses [`ChunkFetch`](crates/da/src/das.rs); production fetch is `gossip.da_chunks`. Topic string is shared (`/l1/da-chunks/1`).
- **Commit headers still carry `DA_ROOT_PLACEHOLDER`:** `build_local` / `wire_commit` are unchanged. `wire_da` applies `apply_da_root` on a **clone** after persist so the QC/`header.hash` of the committed block is not rewritten. See Part C.

## Part B — Tier 0–11 ↔ Tier 12 integration

### 1. Dependency-by-dependency

| Contract | Dep | Real symbol called |
|---|---|---|
| da.chunk.split | reed_solomon.encode | `da::rs::encode` (`crates/da/src/rs.rs`) |
| | block.body | `types::block::Block`; bytes via `storage::codec::encode_block_body` |
| da.chunk.reconstruct | reed_solomon.decode | `da::rs::decode` |
| | da.chunk.split | `split` then `reconstruct` in tests |
| da.root | da.chunk.split | `chunk::split` inside `root::commit` |
| | merkle.compute_root | `state::merkle::compute_root` |
| | kzg.commit | `crypto::kzg::commit` on toy `kzg.setup` (`KZG_SRS_SEED`) |
| header.da_root | da.root | `DaRoot.merkle` passed into `apply_da_root` (same crate cycle avoided) |
| | block.da_root.placeholder | `DA_ROOT_PLACEHOLDER` / `Header.da_root` |
| gossip.da_chunks | gossip.mesh | `ident_topic` + `mesh_config`; `TOPIC_DA_CHUNKS` in `all_topics` |
| | da.chunk.split | `da::chunk::split` in `publish_da_chunks` |
| das.sample | gossip.da_chunks | `TOPIC_DA_CHUNKS`; network tests round-trip codec then `da::sample` |
| | da.root | `verify_chunk` / `DaRoot` |
| das.fail_closed | das.sample | interprets `SampleReport` from `sample` |
| node.wire.da | node.wire.commit | `wire_da` calls `wire_commit` first |
| | header.da_root | `apply_da_root` on a post-commit clone |
| | gossip.da_chunks | `publish_da_chunks` + `ingest_da_chunk` |

**Earlier-tier public signatures:** none changed. Additive only: `apply_da_root`, gossip topic `/l1/da-chunks/1`, `GossipKind::DaChunk = 7`, `all_topics` length 6 → 7, `network`/`node`/`da` Cargo deps. `Header::hash_preimage` layout **unchanged** (`HEADER_PREIMAGE_LEN` still 8+4+48+8+32×5).

### 2. No regression

- **Before (Tier 11 audit):** **294** workspace tests.
- **After:** **313** (`+19`: da RS 5 → 19, plus header / gossip / `wire_da`).
- **`crates/execution/tests/golden.rs` literals CONFIRMED UNCHANGED** and still pass:

| Vector | Hex (identical to Tier 3 / later audits) |
|---|---|
| genesis.hash | `3070e230ec9bd58862fe78b43774f85879d2db270f8e5f28facb4637bae5f1b1` |
| empty_block app_hash | `43dca346e5485849e16be9da4fc13d10c43e3b7701df51e7c273d1b4ed3cf6ad` |
| single_transfer app_hash | `2898208706d1893606f1f79959653189f3d2163f528b849430fd569db238515f` |
| rejected_nonce app_hash | `3ad011d6221bb5f627aafc815a8ee8352caf8efd8b30352b08a67877b2c09f62` |
| multi_account app_hash | `c70d2307dcc8448eaa9a18ce3d1bc372036ba97c0e93f3a9c1c4e7f9b5164909` |

These vectors are `genesis.hash` / `exec.app_hash`, not `header.hash` with a non-zero DA root. `header.hash` with `DA_ROOT_PLACEHOLDER` is the same preimage as Tier 3.

### 3. Forbidden-edge trace (`das.sample` must not block `cons.commit` / `node.wire.commit`)

**`cons.commit`** (`crates/consensus/src/steps.rs`):

```
commit(precommits, validators, reachable, proposal, log)
  → halt_no_quorum?
  → polka(precommits)
  → proposal.header.hash() equality
  → qc::aggregate / qc::verify
  → log.record(Finalized)
```

Zero mentions of `da`, `das`, `sample`, `fail_closed`. Consensus crate has no `da` dependency.

**`node.wire.commit`** (`crates/node/src/wire.rs`):

```
wire_commit(...)
  → consensus::steps::commit(...)     // above
  → persist_then_broadcast(...)       // WAL, put_block, gossip.block
```

Does not call `wire_da`, `publish_da_chunks`, `das::sample`, or `fail_closed`. Test `wire_da_runs_after_commit_and_does_not_gate_it` asserts that source slice.

**`node.wire.da`:**

```
wire_da(...)
  → wire_commit(...)                  // commit completes first
  → publish_da_chunks / da::root::commit / apply_da_root(clone) / ingest_da_chunk
```

No `das::sample` on this path either. Sampling is light-node-only (`crates/da/src/das.rs`).

**DAS-gates-commit forbidden edge: CLEAN (zero calls found).**

### 4. Reconstruction under randomized loss

`chunk::tests::randomized_loss_patterns`: 48 PRNG masks over 6 shards. If `present >= k` (4), reconstruct equals the original `Block`. If `present < k`, `reconstruct` returns `Err` (not a silently wrong body). Plus named cases: parity-only drop (indices 4,5) and mixed data+parity drop (0 and 5).

### 5. Cross-boundary determinism

`root::tests::independent_nodes_agree_on_root`: two `commit` calls on the same body yield identical Merkle roots, KZG bytes, shard index order, and leaf bytes (`index_be16 || payload`). Leaf order is RS index 0..k+m-1.

### 6. Full workspace regression

`cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --check`: green, including:

1. Withhold queried indices → `fail_closed` = `NotAvailable`
2. Reconstruct from any k (parity-only + mixed + random)
3. Tampered chunk fails `das.sample` / `ingest_da_chunk`
4. Shard length grows with body size; count stays 6

`python3 scripts/gen_dependency_graph.py`: **`docs/dependency-graph.json` unchanged** (`git diff` empty).

## Part C — Frozen-header decision & forbidden-edge verdict

**Frozen-header decision: header.hash LEFT UNCHANGED, da.root authenticated separately via (1) the existing hashed `da_root:32` slot when `apply_da_root` is used before voting, and (2) independent recomputation of `da.root` from `block.body` by full nodes.**

Reasoning: Tier 3 already hashed `da_root:32` (always zeros). Extending the preimage would invalidate every stored header and `HEADER_PREIMAGE_LEN`. This tier does **not** add fields to `hash_preimage`. `apply_da_root` writes `da.root`’s Merkle digest into that slot; a header that still holds `DA_ROOT_PLACEHOLDER` hashes exactly as in Tier 3. `wire_commit` / `build_local` still persist the placeholder so finalized `header.hash` / QCs are not silently rewritten after votes. Light DAS checks samples against a `DaRoot` computed from the body (or gossiped with chunks); binding that root into a QC is done by calling `apply_da_root` **before** `cons.propose`, which this tier exposes but does not force on the existing commit path.

**Tier 3 golden vectors: CONFIRMED UNCHANGED, values identical to Tier 3's original audit** (table in Part B.2). Not superseded.

**DAS-gates-commit forbidden edge: CLEAN (zero calls found)** — traces in Part B.3.

## Part D — Overall verdict

- **Tier 12 average audit score: 93.6% — PASS**
- **Tier 0–11 integration status: CLEAN**
- **Frozen-header & forbidden-edge status: CLEAN**

Tier 12 is complete on the audit bar. No git commit/push was made.
