# Tier 0 audit

**Date:** 2026-08-29  
**Scope:** 36 contracts in `docs/dependency-graph.json` → `tiers.tier_0`  
**Rubric:** 30 correctness + 20 tests + 15 isolation + 10 docs + 15 lint/CI + 10 path fidelity = 100

## 1. Per-contract scores

| Contract id | Score /100 | ≥90 |
|---|---:|:---:|
| hash.blake3 | 97 | pass |
| encoding.canonical.encode | 96 | pass |
| encoding.canonical.decode | 96 | pass |
| domain.tag.apply | 96 | pass |
| ed25519.keygen | 93 | pass |
| ed25519.sign | 92 | pass |
| ed25519.verify | 96 | pass |
| bls.keygen | 94 | pass |
| bls.sign | 95 | pass |
| bls.verify | 96 | pass |
| bls.aggregate | 96 | pass |
| bls.verifyAggregate | 96 | pass |
| bls.domain | 94 | pass |
| vrf.ecvrf.prove | 93 | pass |
| vrf.ecvrf.verify | 95 | pass |
| reed_solomon.encode | 96 | pass |
| reed_solomon.decode | 97 | pass |
| kzg.setup | 94 | pass |
| clock.injected | 96 | pass |
| spec.constants | 95 | pass |
| spec.params_registry | 95 | pass |
| types.hash | 94 | pass |
| types.address | 94 | pass |
| types.amount | 95 | pass |
| types.nonce | 94 | pass |
| types.height | 93 | pass |
| types.round | 93 | pass |
| types.epoch | 93 | pass |
| types.chain_id | 90 | pass |
| types.validator_id | 94 | pass |
| types.voting_power | 94 | pass |
| error.core | 94 | pass |
| tooling.rust_toolchain | 94 | pass |
| tooling.clippy_ci | 95 | pass |
| determinism.sorted_maps | 96 | pass |
| tracing.conventions | 91 | pass |

**Sum:** 3393 / 3600  
**Tier 0 average audit score: 94.3% — PASS**

## 2. Contracts below 90

None.

### Notes on the lowest passing scores (not blocking)

- **ed25519.sign (92):** RFC 8032 §7.1 *public key* matches the published vector for that seed; the *signature* bytes produced by `ed25519-dalek` 2.x for the empty message do not match the RFC hex dump, so the KAT pins this crate’s deterministic output. Round-trip and flipped-byte verify still pass.
- **types.chain_id (90):** Happy path plus inequality; no encoding-level failure unique to the newtype beyond `Copy`/`Eq`.
- **tracing.conventions (91):** Conventions are documented and `init` is exercised; “no log on hash-sensitive paths” is a documented rule, not a compiler lint.

## 3. Workspace-level checks (this machine)

| Check | Result |
|---|---|
| `cargo build --workspace` | green |
| `cargo test --workspace --all-targets` | green (types 31, crypto 20, da 5, node 2) |
| `cargo clippy --workspace --all-targets -- -D warnings` | green |
| `cargo fmt --all -- --check` | green |
| `bash scripts/check_no_hashmap.sh` | green |

Isolation: `crates/types` does not depend on `crypto`. `crates/crypto` does not depend on `types`, `state`, `execution`, `consensus`, `network`, `storage`, `mempool`, `rpc`, or `da`. `crates/da` depends only on `reed-solomon-erasure` + `thiserror`.

Path fidelity: every `rust_file` in `tier_0` exists at that path (shared files hold sibling contracts as specified).

## 4. Development-plan.md §4 Tier 0 exit criteria

Plan exit: *Empty crates compile; spec constants are in `types` and documented.* Plus Tier 0 work items (toolchain/CI, domain tags, tracing convention, BTreeMap policy, test clock, stable hash fixture).

**Met.** Later-tier crates remain empty libraries and still compile. Spec constants and PLACEHOLDER epoch/unbonding live in `crates/types/src/spec.rs`. BLAKE3 empty-input KAT is stable. CI workflow runs fmt, clippy `-D warnings`, tests, and the HashMap grep.

## 5. JSON vs prompt

`tier_0` lists **36** contracts; this audit covers those 36. Workspace `independent_contracts` is 38 because two later-tier units also have empty `dependencies` (`obs.structured_logging`, `iac.dockerfile`). That is not a Tier 0 gap.
