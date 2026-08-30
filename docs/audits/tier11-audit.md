# Tier 11 audit

**Date:** 2026-08-30  
**Scope:** 9 contracts in `docs/dependency-graph.json` → `tiers.tier_11` (`tx.deploy` / `tx.call` in `crates/types/src/tx.rs`; `crates/execution/src/wasm/`; `exec.seq.apply_tx.wasm`; `stm.apply_block.wasm`)  
**Rubric:** 30 correctness + 20 tests + 15 isolation + 10 docs + 15 lint/CI + 10 path fidelity = 100

Tiers 0–10 audits all report PASS (≥ 90%). Tier 10 STM/seq equivalence (768 transfer-only property cases, 0 divergences) is unchanged as a test and still green.

## Part A — Per-contract scores

| Contract id | Score /100 | ≥90 |
|---|---:|:---:|
| tx.deploy | 93 | pass |
| tx.call | 93 | pass |
| wasm.meter | 93 | pass |
| wasm.host.sload | 93 | pass |
| wasm.host.sstore | 93 | pass |
| wasm.deploy | 94 | pass |
| wasm.call | 94 | pass |
| exec.seq.apply_tx.wasm | 94 | pass |
| stm.apply_block.wasm | 93 | pass |

**Sum:** 840 / 900  
**Tier 11 average audit score: 93.3% — PASS**

### Notes

- Runtime is **wasmtime 26** with `Config::consume_fuel(true)`. Not a stub interpreter.
- **Frozen reentrancy policy: no reentrancy.** [`wasm::call::execute`](crates/execution/src/wasm/call.rs) rejects a second entry for the same contract address via `World.executing`. The `host.reenter` import exists only so a guest can attempt a nested `wasm.call`; it is not a contract stdlib.
- Host storage is `sload`/`sstore` only. Guests never receive a trie. Values are committed with [`ContractStorageTrie::get`/`put`](crates/state/src/tries.rs) and versioned with [`VersionedSlots::read`/`write`](crates/state/src/version.rs).
- Sequential `apply_tx` still runs signature → nonce → `gas_meter` → balance **before** `wasm.deploy` / `wasm.call`. Invalid WASM is rejected with `RejectReason::WasmInvalid` and **no** nonce/fee mutation.
- STM speculation still calls the same `apply_tx`. After each speculative run, `storage_reads` / `storage_writes` from the host are unioned into `SpecTx` RW sets (64-byte keys). `merge_spec` / `bump_writes` / `seed_and_read` now handle those keys so overlapping slot accesses conflict and re-execute.

## Part B — Tier 0–10 ↔ Tier 11 integration

### 1. Dependency-by-dependency

| Contract | Dep | Real symbol called |
|---|---|---|
| tx.deploy | tx.envelope | `Tx` / `TxPayload::Deploy` tags `6` on the existing envelope |
| tx.call | tx.envelope | `TxPayload::Call` tag `7` |
| | types.address | `Call.to: Address` |
| wasm.meter | tx.gas_meter | `crate::gas::gas_meter` (same function as transfers) |
| wasm.host.sload | state.contract_storage_trie | `ContractStorageTrie::get` |
| | state.versioned_slot.read | `VersionedSlots::read` on `World.versioned` |
| wasm.host.sstore | wasm.host.sload | `sstore` calls `sload` then writes |
| | state.versioned_slot.write | `VersionedSlots::write` |
| wasm.deploy | tx.deploy | `Tx` / `Deploy` via `prepare` |
| | wasm.meter | `wasm::gas::meter` → `gas_meter` |
| | state.account_trie | `AccountTrie::put` with `code_hash` |
| wasm.call | tx.call | `Call` payload |
| | wasm.deploy | `deploy::install` output (`World.code`) + `validate_wasm` |
| | wasm.host.sstore | linker import `host.sstore` |
| exec.seq.apply_tx.wasm | wasm.call | `wasm::call::call` after existing checks |
| | exec.seq.apply_tx | same `apply_tx` function, extended (not a parallel entry) |
| stm.apply_block.wasm | exec.seq.apply_tx.wasm | `speculate` / `reexec` still call `seq::apply_tx` |
| | stm.apply_block | same `stm::apply_block` / `apply_block_engine` |

**Earlier-tier signatures touched (justified):**

- `gas_meter`: match extended with `Deploy`/`Call` intrinsic costs (`GAS_DEPLOY` / `GAS_CALL`). Required for exhaustiveness; transfer/stake arms unchanged (`GAS_TRANSFER`).
- `RejectReason` / `Event`: additive variants (receipt reason bytes 9–12, event tag 2). Transfer/stake encodings for bytes 0–8 unchanged.
- `World`: additive fields (`code`, `executing`, storage RW sets, `versioned`, `wasm_fuel_left`). `from_genesis` still only fills accounts from genesis.
- `VersionedSlots`: added `Clone` / `Debug` / `Default`. **`read` / `write` / `validate` signatures unchanged.** Needed so `World` can clone for STM speculation and WASM rollback.
- STM `seed_and_read` / `merge_spec` / `bump_writes`: 64-byte storage keys (previously skipped when `len != 32`). Transfer/staking 32-byte account keys and `STAKING_SLOT` behavior unchanged.
- Mempool `From<RejectReason>`: new WASM reasons map to `VerifyError::Gas` so the match stays exhaustive.

### 2. No regression on non-WASM paths

- **Before (Tier 10):** **278** workspace tests.
- **After:** **294** (`+16`).
- `crates/execution/tests/golden.rs` APP hex literals **unchanged** and still pass (`empty_block`, `single_transfer`, `rejected_nonce`, `multi_account`).
- `property_stm_equals_seq_three_seeds` (transfer-only, 256×3 = **768** cases) still **0** divergences.

### 3. STM/WASM equivalence proof

`property_stm_equals_seq_wasm_three_seeds`: blocks mix **deploy**, **call** (same contract, overlapping slot 0), **transfers**, and **stake_bond**. Compared `seq::apply_block` vs `stm::apply_block_engine` (receipt encodings, `app_hash`, state root).

**768 property-test cases run, 0 divergences.**

### 4. Reentrancy policy proof

`wasm::call::tests::reentrancy_rejected`: a module that calls `host.reenter` is rejected with `RejectReason::WasmReentrancy` on the first run and **8** repeats. Enforced in `execute` via `World.executing`, not by convention.

### 5. Gas-exhaustion determinism

`wasm::call::tests::gas_exhaustion_deterministic`: infinite-loop module, fuel `1000`, **8** reruns. Halt reason is always `WasmGas`; `wasm_fuel_left` is **identical** (`0`) every time.

### 6. Cross-boundary determinism

Storage keys are `contract:32 || 28×0x00 || slot:u32 BE` (byte-sorted). Host does not iterate `HashMap`. `types::collections::{Map,Set}` only. `python3 scripts/gen_dependency_graph.py` — JSON **unchanged**. `scripts/check_no_hashmap.sh` green.

### 7. Full workspace

`cargo test --workspace` (serial), `clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check` green. WASM STM property is in the default `stm_equiv` run (no feature flag).

## Part C — WASM-specific verdict

- **Non-WASM regression check (Tier 3 golden vectors + Tier 10 property test unchanged): CLEAN**
- **Extended STM/WASM equivalence: 768 property-test cases run, 0 divergences** (plus the original 768 transfer-only cases still green)
- **Reentrancy policy chosen: no-reentrancy, enforced: CONFIRMED** (specific `WasmReentrancy`, 8/8 identical rejects)
- **Gas-exhaustion determinism: CONFIRMED (identical halt point across 8 runs)** — remaining fuel `0` each time

## Part D — Overall verdict

- **Tier 11 average audit score: 93.3% — PASS**
- **Tier 0–10 integration status: CLEAN**
- **WASM-specific verification: ALL CONFIRMED**

Tier 11 is complete on this bar. No git commit/push. Not starting Tier 12. No RPC `getContractStorage`. No FHE. STM remains compared against sequential `apply_block` (now covering WASM txs through the same `apply_tx`).
