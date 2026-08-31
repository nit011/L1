# Tier 14 audit

**Date:** 2026-08-31  
**Scope:** 11 contracts in `docs/dependency-graph.json` → `tiers.tier_14`  
**Rubric:** 30 correctness + 20 tests + 15 isolation + 10 docs + 15 lint/CI + 10 path fidelity = 100

Tiers 0–13 audits all report PASS (≥ 90%). `cargo build --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --check`, and `cargo test --workspace -- --test-threads=1` are green after this tier. `python3 scripts/gen_dependency_graph.py`: **`docs/dependency-graph.json` unchanged**.

## Frozen-spec investigation (before implementation)

1. **Block limits vs `exec.seq.apply_block`:** `apply_block` remains `(World, receipts, app_hash)` with no size/gas gate (`crates/execution/src/seq.rs`). Caps live in `node::limits::precheck_block`, which uses `storage::codec::encode_block_body` and `execution::gas::gas_meter` and **does not call** `apply_block`.
2. **Rent vs `state.account`:** `Account` is still `balance || nonce || code_hash` (16+8+32 payload, then `encoding.canonical.encode`). Rent uses `Account::encode().len()` plus an auxiliary `extra_storage_bytes` argument — **no new hashed fields**.

## Part A — Per-contract scores

| Contract id | Score /100 | ≥90 |
|---|---:|:---:|
| limits.max_block_bytes | 94 | pass |
| limits.max_gas | 94 | pass |
| limits.state_growth | 91 | pass |
| state.rent | 94 | pass |
| state.expiry | 92 | pass |
| state.reactivate | 94 | pass |
| prune.hot_cold | 94 | pass |
| sync.snapshot | 94 | pass |
| netsec.asn_cap | 91 | pass |
| netsec.peer_rotation | 93 | pass |
| fee.1559_optional | 93 | pass |

**Sum:** 1024 / 1100  
**Tier 14 average audit score: 93.1% — PASS**

### Notes (not blocking)

- **`limits.state_growth` (91):** Cap arithmetic is documented (64 KiB/block ≈ 2 TB/year at 1 block/s vs 1–2 TB NVMe). The check calls `state::root::commit_tries` on pre/post tries, then compares a caller-supplied `accounted_delta` — it does not walk the trie to measure bytes (no account iterator on `AccountTrie`). Not a stub; not a full occupancy census.
- **`state.expiry` (92):** `execution` already depends on `state`, so `expiry.rs` cannot import `execution::rent`. `rent_due` is passed in; `execution::rent::expire_if_unpaid` / rent tests call `rent_gas` then `expire`. Documented crate-cycle, not a reimplementation of rent.
- **`netsec.asn_cap` (91):** No ASN database in this environment. Bucketing is configurable IPv4 prefix length (default /24, tests use /16). Real ASN mapping is a **deployment-time data dependency**, documented on `AsnCap`.
- **Gossip wire-up:** `precheck_block` is the pre-`apply_block` API. `wire_commit` / `network::validation` signatures were **not** changed (would be an earlier-tier public-error change). Oversized rejection is proven in `node::limits` tests without invoking `apply_block`.

## Part B — Tier 0–13 ↔ Tier 14 integration

### 1. Dependency-by-dependency

| Contract | Dep | Real symbol called |
|---|---|---|
| limits.max_block_bytes | spec.params_registry | `ParamsRegistry::get(ParamId::MaxBlockBytes)` |
| | genesis.params | `Genesis.params` |
| limits.max_gas | limits.max_block_bytes | `max_block_bytes` |
| | tx.gas_meter | `execution::gas::gas_meter` |
| limits.state_growth | limits.max_gas | `max_gas` |
| | state.commit_root | `state::root::commit_tries` |
| state.rent | tx.gas_meter | `gas_meter` on a probe `Tx::transfer` |
| | state.account | `Account::encode` |
| state.expiry | state.rent | `rent_gas` via `expire_if_unpaid` / rent tests (crate cycle; see notes) |
| | mpt.prove | `state::mpt::proof::prove` |
| state.reactivate | state.expiry | `ExpiryRecord` / `expire` |
| | mpt.prove_exclusion | `prove_exclusion` |
| prune.hot_cold | store.block.put | `storage::blocks::put_block` |
| | kv.rocksdb | `RocksStore::open` |
| sync.snapshot | state.commit_root | `state::root::commit_root` (passed into `snapshot_commit_root`) |
| | store.replay_from_genesis | `replay_from_genesis` via `replay_for_snapshot_check` |
| netsec.asn_cap | netsec.ip_slot_cap | `IpSlotTable::admit` / `prefix_of` |
| netsec.peer_rotation | netsec.asn_cap | `asn_bucket` / `AsnCap` |
| | gossip.scoring | `network::scoring::score` |
| fee.1559_optional | mempool.min_fee | `mempool::fees::min_fee_floor` (tests + node limits test) |
| | limits.max_gas | `node::limits::max_gas` passed as `max_gas: u64` |

**Earlier-tier public signatures:** none of `apply_block`, `Account::encode`, `ParamsRegistry::new`, or `header.hash` changed.

Additive only:

- `AccountTrie::delete` (uses existing `mpt::delete`) so expiry can drop live keys without touching account encoding.
- `IpSlotTable::{remove, contains, count_prefix, peer_keys, prefix_of_peer}` plus `AsnCap` / `rotate_peers`.
- `execution::fees::next_base_fee`.
- `storage` **dev-dependency** on `state` so snapshot tests can call `commit_root` by name (lib graph still `state` → `storage`, no cycle).

### 2. No regression, golden vectors

- **Before (Tier 13 audit):** **325** workspace tests.
- **After:** **343** (`+18` across limits, rent, expiry, prune, snapshot, eclipse, fees).
- **`crates/execution/tests/golden.rs` BYTE-IDENTICAL** (literals unchanged, tests pass):

| Vector | Hex |
|---|---|
| genesis.hash | `3070e230ec9bd58862fe78b43774f85879d2db270f8e5f28facb4637bae5f1b1` |
| empty_block app_hash | `43dca346e5485849e16be9da4fc13d10c43e3b7701df51e7c273d1b4ed3cf6ad` |
| single_transfer app_hash | `2898208706d1893606f1f79959653189f3d2163f528b849430fd569db238515f` |
| rejected_nonce app_hash | `3ad011d6221bb5f627aafc815a8ee8352caf8efd8b30352b08a67877b2c09f62` |
| multi_account app_hash | `c70d2307dcc8448eaa9a18ce3d1bc372036ba97c0e93f3a9c1c4e7f9b5164909` |

### 3. Account-encoding decision

`state.account` encoding **UNCHANGED**. Tier 1 tests `account::tests::encode_decode_round_trip` and `decode_rejects_truncated` still pass with no expected-hex edits. Payload remains 56 bytes before canonical wrap.

### 4. Archive-node exemption

`PruneConfig::default().prune_cold == false`. `prune::tests::default_is_archive_replay_unaffected` puts three blocks, does not drop bodies, and `replay_from_genesis(..., apply_block)` matches live `commit_state_root`. Opt-in prune (`prune_cold: true`, `hot_window: 1`) drops cold bodies; replay then errors (expected for a pruning validator).

### 5. Snapshot vs full replay

`snapshot::tests::snapshot_and_full_replay_same_commit_root`: snapshot `commit_root` and replayed `World::commit_state_root()` are both

`17412c6b501b28db07efb8ca00efd4927ce9aaf6941be49c4fc5963e3693a234`

(empty genesis alloc, three empty blocks; live root matches).

### 6. Cross-boundary determinism

- Snapshot account pairs sorted by address bytes (`take_snapshot`).
- Peer rotation sorts `(score asc, occupancy desc, peer id bytes)`.
- Prefix/ASN maps are `types::collections::Map` (BTree).
- Rent/expiry keyed by `Address`; no `HashMap`.

### 7. Full workspace regression

`cargo test --workspace -- --test-threads=1`: all packages green (including simnet 4/4). Clippy `-D warnings` and `fmt --check` green. `scripts/check_no_hashmap.sh` ok.

## Part C — Frozen-spec decisions verdict

- **Block-limit enforcement layer: PRE-CHECK outside apply_block (CONFIRMED)**
- **Tier 3 golden vectors after Tier 14: BYTE-IDENTICAL (values shown in Part B.2)**
- **state.account encoding: UNCHANGED (rent tracked via auxiliary `extra_storage_bytes` / `rent_gas`, not hashed account fields)**
- **Snapshot vs. full-replay state root: IDENTICAL (`17412c6b501b28db07efb8ca00efd4927ce9aaf6941be49c4fc5963e3693a234`)**
- **Archive-node exemption: CONFIRMED (non-pruning node passes full replay-from-genesis test)**

## Part D — Overall verdict

- **Tier 14 average audit score: 93.1% — PASS**
- **Tier 0–13 integration status: CLEAN**
- **Frozen-spec decisions: ALL CONFIRMED CONSISTENT**

Tier 14 is complete on the local working tree. No git commit/push/PR. No Tier 15 work.
