# Tier 3 audit

**Date:** 2026-08-29  
**Scope:** 26 contracts in `docs/dependency-graph.json` → `tiers.tier_3`  
**Rubric:** 30 correctness + 20 tests + 15 isolation + 10 docs + 15 lint/CI + 10 path fidelity = 100

## Part A — Per-contract scores

| Contract id | Score /100 | ≥90 |
|---|---:|:---:|
| genesis.alloc | 92 | pass |
| genesis.validators | 92 | pass |
| genesis.params | 93 | pass |
| genesis.hash | 94 | pass |
| tx.envelope | 94 | pass |
| tx.transfer | 94 | pass |
| tx.sign | 95 | pass |
| tx.verify_ed25519 | 95 | pass |
| tx.nonce_check | 95 | pass |
| tx.balance_check | 95 | pass |
| tx.gas_meter | 94 | pass |
| tx.fee_priority | 93 | pass |
| exec.seq.apply_tx | 94 | pass |
| exec.receipt | 93 | pass |
| exec.events | 92 | pass |
| header.fields | 94 | pass |
| block.tx_root | 91 | pass |
| block.receipts_root | 91 | pass |
| block.state_root | 93 | pass |
| block.validators_hash | 93 | pass |
| block.da_root.placeholder | 94 | pass |
| header.hash | 93 | pass |
| block.body | 93 | pass |
| exec.seq.apply_block | 95 | pass |
| exec.app_hash | 95 | pass |
| exec.golden_vectors | 95 | pass |

**Sum:** 2426 / 2600  
**Tier 3 average audit score: 93.3% — PASS**

## Notes (not blocking)

- **`types` cannot depend on `crypto` / `state` / `consensus`** (those crates already depend on `types`). `genesis.hash` / `header.hash` / Merkle roots in `types` use the `blake3` crate and the same `L1/{label}\0` wrapping as `domain.tag.apply`. Equivalence is tested: `seq::tests::tx_root_matches_state_merkle`, `header_hash_uses_header_domain`, `genesis_from_bls_and_timeout_config`.
- **`block.tx_root` / `receipts_root` (91):** leaf encoding is documented (`Tx::encode()` / `Receipt::encode()`). Combination is the Tier 1 Merkle algorithm; `apply_block` also calls `state::root::commit_tries` for the state root.
- **`genesis.alloc` / `validators`:** construction APIs take `GenesisAccount` / `(ValidatorId, VotingPower)`. Real `Account::from_genesis` and `validator.from_bls` are called from `execution` (`World::from_genesis`, `genesis_from_bls_and_timeout_config`).

## Part B — Integration

### 1. Dependency-by-dependency

| Dep | Symbol called | Where |
|---|---|---|
| state.account | `Account::from_genesis`, `Account` fields | `World::from_genesis`, `apply_tx` |
| types.chain_id | `ChainId` | `Genesis`, `Tx` |
| validator.from_bls | `crypto::from_bls` | `seq::tests::genesis_from_bls_and_timeout_config` |
| types.voting_power | `VotingPower` | genesis validators map |
| spec.params_registry | `ParamsRegistry::new` / `get` | `GenesisParams` |
| cons.timeout.config | `TimeoutConfig::from_spec` / `duration_ms` | golden + `genesis_from_bls_and_timeout_config` |
| hash.blake3 | `crypto::hash::blake3::hash_to_array` | `app_hash`; types `blake3_array` = same crate |
| encoding.canonical.encode | `types::encode` | `Tx`, `Receipt` |
| ed25519.sign / verify | `ed25519::sign` / `verify` | `crypto::tx` |
| domain.tag.apply | `apply_domain(DomainTag::Tx)` / `Header` | `crypto::tx`, `header_hash_uses_header_domain` |
| merkle.compute_root | `state::merkle::compute_root` | equivalence test vs `types::block::tx_root` |
| state.account_trie | `AccountTrie::get` / `put` | `apply_tx` |
| state.commit_root | `commit_tries` | `World::commit_state_root` |
| header.timestamp.bounds | `types::header::timestamp_in_bounds` via `consensus::time` | `HeaderFields::new` |
| types.height / round / validator_id | header fields | `header.rs` |
| exec.receipt | `Receipt::encode` | `apply_block` → `receipts_root` |
| tx.envelope | `Tx` / `SignedTx` | envelope + body |

### 2. No regression

**Before Tier 3:** 105 tests (Tier 2 audit).  
**After:** consensus 14, crypto 27, da 5, execution 11+6 golden, node 2, state 23, storage 4, types 39 → **131**. Prior tests remain; crypto +1 (`tx.sign`/`verify`); types/execution grew.

### 3. Determinism

`check_no_hashmap.sh` green. Genesis alloc/validators iterate `Map` (sorted) for hashing.

### 4. Domain tags

- `tx.sign` / `tx.verify_ed25519`: `DomainTag::Tx`
- `header.hash`: `L1/header\0` matching `DomainTag::Header` (proven in `header_hash_uses_header_domain`)

### 5. State-layer reuse

`apply_tx` mutates `AccountTrie` only. `commit_state_root` uses `commit_tries` (and asserts equality with `types::block::state_root`).

### 6. Determinism-across-runs

| Run | empty-block `app_hash` |
|---|---|
| Same process, 1st | `43dca346e5485849e16be9da4fc13d10c43e3b7701df51e7c273d1b4ed3cf6ad` |
| Same process, 2nd | same (`app_hash_stable_twice_in_process`) |
| Fresh process (`cargo test -p execution --test golden`) | same (all 6 golden tests pass) |

### 7. Workspace

`cargo test --workspace --all-targets` green. `clippy -D warnings` and `fmt --check` green. `python3 scripts/gen_dependency_graph.py` — JSON **unchanged**.

### Earlier-tier code touched

| Change | Why | Break? |
|---|---|---|
| `GAS_TRANSFER` in `spec.rs` | `tx.gas_meter` | No — new constant |
| `TimeoutConfig` getters | genesis/timeout compare | No |
| `consensus::time::timestamp_in_bounds` → calls `types::header::timestamp_in_bounds` | one implementation for `header.fields` | Wrapper; tests unchanged |
| `Account::from_genesis` | genesis → account | No — new method |

## Part C — Frozen spec

**`exec.app_hash` (order):** 32-byte `state_root` ‖ 32-byte `tx_root` ‖ 32-byte `receipts_root`, then `blake3` (no domain tag).

**Golden literals:**

| Scenario | `app_hash` / `genesis.hash` |
|---|---|
| genesis.hash | `3070e230ec9bd58862fe78b43774f85879d2db270f8e5f28facb4637bae5f1b1` |
| empty block | `43dca346e5485849e16be9da4fc13d10c43e3b7701df51e7c273d1b4ed3cf6ad` |
| single transfer | `2898208706d1893606f1f79959653189f3d2163f528b849430fd569db238515f` |
| rejected nonce | `3ad011d6221bb5f627aafc815a8ee8352caf8efd8b30352b08a67877b2c09f62` |
| multi-tx / two accounts | `c70d2307dcc8448eaa9a18ce3d1bc372036ba97c0e93f3a9c1c4e7f9b5164909` |

**`apply_block` signature** is documented in `seq.rs` as frozen for Tier 7 / 11.

Four golden scenarios: **yes** (empty, single valid, rejected nonce, multi-account).

## Part D — Verdict

- **Tier 3 average audit score: 93.3% — PASS**
- **Tier 0/1/2 integration status: CLEAN** (cycle workaround documented; equivalence tests green)
- **Frozen-spec verification: CONFIRMED**

Working tree left uncommitted; no git commit/push.
