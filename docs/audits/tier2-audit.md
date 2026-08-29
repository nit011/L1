# Tier 2 audit

**Date:** 2026-08-29  
**Scope:** 9 contracts in `docs/dependency-graph.json` → `tiers.tier_2`  
**Rubric:** 30 correctness + 20 tests + 15 isolation + 10 docs + 15 lint/CI + 10 path fidelity = 100

## Part A — Per-contract scores

| Contract id | Score /100 | ≥90 |
|---|---:|:---:|
| cons.timeout.config | 94 | pass |
| cons.clock.bind | 95 | pass |
| header.timestamp.bounds | 95 | pass |
| vrf.seed.derive | 95 | pass |
| vrf.leader.prove | 94 | pass |
| vrf.leader.verify | 94 | pass |
| vrf.leader.weighted | 93 | pass |
| cons.replay.vote | 94 | pass |
| cons.round_robin.testdouble | 94 | pass |

**Sum:** 848 / 900  
**Tier 2 average audit score: 94.2% — PASS**

## Notes (not blocking)

- **vrf.leader.weighted (93):** Statistical test `weighted_frequency_tracks_stake` uses 20_000 seeds, weights 1:1:2, acceptance **±4 percentage points** absolute (expected 25%/25%/50%). Runtime ~40s in debug. Forged proof and empty set are the failure cases.
- **cons.round_robin.testdouble:** Documented TEST-ONLY; production path is `vrf.leader.weighted`.

## Part B — Tier 0/1 ↔ Tier 2 integration

### 1. Dependency-by-dependency

| Tier 2 dep | Called symbol | Evidence |
|---|---|---|
| spec.constants | `TIMEOUT_PROPOSE_MS`, `TIMEOUT_PREVOTE_MS`, `TIMEOUT_PRECOMMIT_MS`, `TIMEOUT_DELTA_MS`, `MAX_TIMESTAMP_DRIFT_MS` | `timeout.rs`, `time.rs` tests |
| spec.params_registry | `ParamsRegistry::get` / `ParamId::Timeout*` | `TimeoutConfig::from_params` |
| clock.injected | `types::Clock::now_millis`, `TestClock::{new,advance}` | `BoundClock`, `timestamp_in_bounds` |
| types.height | `types::Height`, `Height::GENESIS` | `time.rs`, `replay.rs` |
| types.round | `types::Round` | timeouts, replay, round-robin |
| types.epoch | `types::Epoch` / `epoch.0.to_be_bytes()` | `derive_seed` |
| types.validator_id | `ValidatorId::as_bytes` / `from_bytes` | VRF alpha, replay, round-robin |
| types.voting_power | `VotingPower.0` | `weighted_leader` |
| hash.blake3 | `crypto::hash::blake3::hash_to_array` | seed, replay key/hash |
| domain.tag.apply | `crypto::apply_domain` + `DomainTag::Vrf` / `DomainTag::Vote` | seed, replay |
| vrf.ecvrf.prove | `crypto::vrf::prove` | `leader_prove` |
| vrf.ecvrf.verify | `crypto::vrf::verify` | `leader_verify` (also called from `weighted_leader`) |
| determinism.sorted_maps | `types::collections::Map` | validator set in `weighted_leader` |

No shadow BLAKE3/VRF. Seed formula is `hash_to_array(apply_domain(Vrf, hash \|\| epoch_be))`.

### 2. No regression

**Before Tier 2** (Tier 1 audit): types 31, crypto 26, da 5, node 2, state 23, storage 4 → **91**.

**After Tier 2** (`cargo test --workspace --all-targets`): types **31**, crypto **26**, da **5**, node **2**, state **23**, storage **4**, consensus **14** → **105**.

No prior tests deleted. Types count unchanged (new spec asserts live inside the existing `placeholders_are_nonzero_but_documented` test).

### 3. Cross-boundary determinism

`scripts/check_no_hashmap.sh` green. Weighted selection iterates `Map<ValidatorId, VotingPower>` (BTreeMap). Round-robin sorts ids before indexing.

### 4. Domain separation

| Contract | Tag |
|---|---|
| `vrf.seed.derive` | `DomainTag::Vrf` (`b"vrf"`) |
| `cons.replay.vote` | `DomainTag::Vote` (`b"vote"`) |

No new ad hoc tags. Seed test asserts the result differs from untagged `blake3(hash \|\| epoch)`.

### 5. Consensus-crate purity

`crates/consensus/Cargo.toml` depends only on `types`, `crypto`, `thiserror`.

Grep `libp2p` / `rocksdb` under `crates/consensus`: no matches except a doc comment in `lib.rs` stating the purity rule. No dependency on `network` or `storage`.

Removed the unused `state` path dep that was in the Tier 0 scaffold so the crate stays message-type-pure (development-plan.md §3.1).

### 6. Full workspace regression

| Check | Result |
|---|---|
| `cargo build --workspace` | green |
| `cargo test --workspace --all-targets` | green (105 tests) |
| `cargo clippy --workspace --all-targets -- -D warnings` | green |
| `cargo fmt --all -- --check` | green |
| `python3 scripts/gen_dependency_graph.py` | `docs/dependency-graph.json` **unchanged** |

### Tier 0/1 code touched during Tier 2

| Change | Why | Signature break? |
|---|---|---|
| `spec.rs` timeout + `MAX_TIMESTAMP_DRIFT_MS` | `cons.timeout.config` must not hardcode magic numbers; values live in `spec.constants` | **No** — new constants |
| `ParamId` timeout/drift + registry defaults | so `from_params` reads the same numbers | **No** — new enum variants |
| `types` re-exports those constants and `ParamId` | consensus imports from `types` | **No** |
| `crypto::vrf::public_key_from_seed` | tests/verify need the Edwards pk matching `prove`; avoids copying `expand_sk` into consensus | **No** — new helper; `prove`/`verify` unchanged |

No Tier 1 public APIs changed.

## Part C — Overall verdict

- **Tier 2 average audit score: 94.2% — PASS**
- **Tier 0/1 integration status: CLEAN**
- **Consensus-crate purity: CLEAN**

Tier 2 is complete at this gate. Working tree left uncommitted; no git commit/push.
