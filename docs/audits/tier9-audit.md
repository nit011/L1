# Tier 9 audit

**Date:** 2026-08-29  
**Scope:** 15 contracts in `docs/dependency-graph.json` → `tiers.tier_9`  
**Rubric:** 30 correctness + 20 tests + 15 isolation + 10 docs + 15 lint/CI + 10 path fidelity = 100

Tiers 0–8 audits (`docs/audits/tier0-audit.md` … `tier8-audit.md`) all report PASS (average ≥ 90%). Workspace regression after this tier: `cargo test --workspace` **261** passing tests (was **245** after Tier 8; **+16**). `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` green. `python3 scripts/gen_dependency_graph.py` rewrote `docs/dependency-graph.json` with **no contract edits** (`git diff` empty).

## Part A — Per-contract scores

| Contract id | Score /100 | ≥90 |
|---|---:|:---:|
| tx.stake.bond | 93 | pass |
| tx.stake.unbond | 93 | pass |
| tx.stake.delegate | 93 | pass |
| tx.stake.undelegate | 93 | pass |
| tx.stake.withdraw | 93 | pass |
| staking.min_self_bond | 94 | pass |
| staking.delegation_cap | 94 | pass |
| staking.unbonding_period | 94 | pass |
| staking.epoch_set_update | 93 | pass |
| evidence.submission | 93 | pass |
| slash.apply | 94 | pass |
| slash.tombstone | 93 | pass |
| ws.checkpoint | 93 | pass |
| ws.bootstrap | 92 | pass |
| service.l1.jsonrpc.getCheckpoint | 93 | pass |

**Sum:** 1398 / 1500  
**Tier 9 average audit score: 93.2% — PASS**

### Notes (not blocking)

- Staking payloads use **tags 1–5** on the frozen `tx.envelope` (tag `0` transfer unchanged). Signing is still `crypto::tx::sign` / `verify_ed25519` over `Tx::encode()`.
- `ParamId::{MinSelfBond, DelegationCap, SlashPercent}` exist for `ParamsRegistry::set` in tests; they are **not** inserted in `ParamsRegistry::new()`, so frozen `genesis.hash` is unchanged (`execution/tests/golden.rs` `genesis_hash_literal` still passes). Defaults: `MIN_SELF_BOND=100`, `DELEGATION_CAP=1000`, `SLASH_PERCENT=5`, `CHECKPOINT_INTERVAL=10` in `spec.constants`.
- `World` gained `staking` + `params` sidecar fields. They are **not** in `state.commit_root` / `exec.app_hash` this tier (sequential transfer golden vectors unchanged).
- `ws.bootstrap` still replays from genesis via `node.catchup` after verifying the offered headers contain the checkpoint height/hash. That is sequential-spec replay, not a light-client snapshot (Tier 13).

## Part B — Tier 0–8 ↔ Tier 9 integration

### 1. Dependency-by-dependency

| Contract | Dep | Real symbol called |
|---|---|---|
| tx.stake.bond | tx.envelope | `Tx` / `Tx::encode` / `Tx::decode`; constructors set `TxPayload::Stake` |
| | types.amount | `StakePayload.amount: Amount` |
| tx.stake.unbond | tx.stake.bond | same envelope + `StakeKind::Unbond` |
| tx.stake.delegate | tx.stake.bond | `StakeKind::Delegate` |
| tx.stake.undelegate | tx.stake.delegate | `StakeKind::Undelegate` |
| tx.stake.withdraw | tx.stake.unbond | `StakeKind::Withdraw` |
| staking.min_self_bond | tx.stake.bond | `Tx::as_stake` / `apply_stake_tx` Bond arm |
| | spec.params_registry | `ParamsRegistry::get(ParamId::MinSelfBond)` else `MIN_SELF_BOND` |
| staking.delegation_cap | tx.stake.delegate | delegation map written by Delegate |
| | spec.params_registry | `ParamId::DelegationCap` else `DELEGATION_CAP` |
| staking.unbonding_period | tx.stake.unbond | Unbond arm + `unbonding_unlock` |
| | types.epoch | `Epoch(unlock_height / epoch_len)` |
| staking.epoch_set_update | staking.min_self_bond | `min_self_bond_amount` / skip if self-bond `< min` |
| | cons.commit | **`Finalized`** only (`observe_commit` / `epoch_set_update`); **does not call `commit`** |
| | block.validators_hash | `types::header::validators_hash` |
| evidence.submission | evidence.equivocation | `equivocation` then `Evidence` |
| | bls.verify | `crypto::sig::bls::verify` in `independent_bls_verify` |
| slash.apply | evidence.submission | `submit_evidence` |
| | staking.min_self_bond | `min_self_bond_amount` (same registry family) |
| slash.tombstone | slash.apply | `tombstone` after `apply`; set consulted by `epoch_set_update` / bond |
| ws.checkpoint | cons.commit | `Finalized.height` / `Finalized.block_hash` |
| | header.hash | `Header::hash()` must equal `block_hash` |
| | spec.constants | `CHECKPOINT_INTERVAL` |
| ws.bootstrap | ws.checkpoint | `Checkpoint` height + `header_hash` |
| | node.catchup | `node::sync::catchup` |
| l1_getCheckpoint | rpc.server | `dispatch` → `get_checkpoint` |
| | ws.checkpoint | `RpcInner.checkpoint` from `record_checkpoint` / `observe_checkpoint` |

**Earlier-tier public surface (additive, justified):**

- `TxPayload::Stake` tags 1–5 (transfer tag 0 frozen).
- `ParamId` variants 10–12 + `node::config` / `genesis::param_id_byte` exhaustive matches.
- `RejectReason` receipt bytes 5–8; `Event::Stake` tag 1.
- `World.{staking,params}`; `value_balance_check`; mempool `verify` admits staking envelopes (still calls Tier 3 `nonce_check` / `gas_meter` / `value_balance_check`, not a parallel signer).
- `submit_evidence` added next to existing `equivocation` (does not change `equivocation`’s signature).
- **`cons.commit` signature unchanged.**

### 2. No regression

- **Before (Tier 8):** **245** workspace tests.
- **After:** **261** (`+16`). Includes consensus simnet safety/liveness, node multiprocess/finality LAN, and RPC. Node simnet is load-sensitive if run in parallel with other multiprocess tests; serial `--test-threads=1` is green.

### 3. Forbidden-edge trace (`cons.commit`)

Production `commit` in `crates/consensus/src/steps.rs`:

```
commit(precommits, validators, reachable, proposal, log)
  → total_power(validators)                    // local fold over Map
  → halt_no_quorum(reachable, total)          // safety.rs: !qc::has_quorum
  → polka(precommits, validators)             // tally + qc::has_quorum
  → proposal.header.hash()                    // types::header::Header::hash
  → qc::aggregate(precommits, validators)     // bls.aggregate
  → qc::verify(&qc, validators)               // bls.verify_aggregate
  → Finalized { height, round, block_hash, app_hash: proposal.app_hash }
  → log.record(&f)                            // CommitLog Map<Height, Hash>
```

Callees: `crates/consensus/src/{safety,qc,vote}` and `types::header`. **No** `crates/execution`, **no** `staking::*`, **no** `tx.stake.*`.

`node::wire::wire_commit` calls `commit` then `persist_then_broadcast` (store + gossip). Staking `epoch_set_update` is invoked from execution tests / (optionally) node-adjacent observers with a **`Finalized` value already produced**; it is not in `commit`’s stack.

`crates/consensus/src` production modules do not `use execution`. Consensus tests/simnet use `execution::builder` for proposals only, not from `commit`.

**Result: CLEAN.**

### 4. Epoch-boundary correctness

`epoch_set_update` compares `height / epoch_len` vs `(height+1) / epoch_len`. With `EpochLength=2`, commit at height **0** returns `None` and leaves `current_set` unchanged; commit at height **1** installs the next set. That set is for **epoch 1** rounds, not retroactive to already-finalized height 0. Test: `epoch_set_update_applies_next_epoch_only_and_changes_leader`.

### 5. Cross-boundary determinism

Validator sets, self-bonds, delegations, pending unbonds, tombstones, and slash idempotency keys use `types::collections::{Map, Set}` (ordered). `validators_hash` walks the map in `ValidatorId` order.

### 6. Full workspace

`cargo test --workspace` (serial) all green, **261** tests, including the five required scenarios below.

## Part C — Forbidden-edge & scenario verification

**cons.commit → staking call graph: CLEAN (zero calls found)**

| Scenario | Result | Evidence |
|---|---|---|
| 1. Epoch rotation changes leaders | **PASS** | `epoch_set_update` at height 1 (`epoch_len=2`) adds validator `C` with self-bond `50000`. `vrf::weighted_leader` on the old `{A:100,B:100}` set vs the new set **differs**. |
| 2. Slash on double-sign | **PASS** | Tier 5-style conflicting `prevote`s → `equivocation` → `submit_evidence` → `slash::apply`. Stake **100**, `SLASH_PERCENT=5` → cut **5**, remainder **95**. Second apply of the same evidence: cut **0**, remainder still **95**. |
| 3. Unbond not withdrawable early | **PASS** | `UnbondingPeriod=10`. Withdraw at last-commit height **0** → `RejectReason::StakeUnbonding`. After observe height **10**, withdraw **200** succeeds. Unlock `(Height(10), Epoch(0))`. |
| 4. Checkpoint-based sync | **PASS** | `CHECKPOINT_INTERVAL=10`; height **0** is on-interval. `bootstrap` + `catchup` matches `built.app_hash`. Wrong `header_hash` (`0xab…`) → `WireError`. Non-matching `Finalized.block_hash` → no checkpoint. |
| 5. Delegation cap math | **PASS** | Cap **X = 1000** (`DELEGATION_CAP`). Delegation **X+1 = 1001** recorded in the ledger; `effective_power` = **1000**. Delegation **1000** → power **1000**; **999** → power **999**. Self-bond **0** in this boundary test (cap applies to delegation). |

## Part D — Overall verdict

- **Tier 9 average audit score: 93.2% — PASS**
- **Tier 0–8 integration status: CLEAN** (additive APIs only; `cons.commit` signature unchanged; genesis.hash frozen)
- **Forbidden-edge status: CLEAN**
- **Required scenarios: ALL CONFIRMED**

Tier 9 is complete on this bar. No git commit/push (left uncommitted for review). Not starting Tier 10.
