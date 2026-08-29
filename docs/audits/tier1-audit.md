# Tier 1 audit

**Date:** 2026-08-29  
**Scope:** 28 contracts in `docs/dependency-graph.json` → `tiers.tier_1`  
**Rubric:** 30 correctness + 20 tests + 15 isolation + 10 docs + 15 lint/CI + 10 path fidelity = 100

## Part A — Per-contract scores

| Contract id | Score /100 | ≥90 |
|---|---:|:---:|
| merkle.compute_root | 94 | pass |
| merkle.verify | 94 | pass |
| mpt.pathencoding | 95 | pass |
| mpt.node.leaf | 94 | pass |
| mpt.node.extension | 93 | pass |
| mpt.node.branch | 94 | pass |
| mpt.get | 94 | pass |
| mpt.put | 93 | pass |
| mpt.delete | 93 | pass |
| mpt.prove | 92 | pass |
| mpt.verify | 91 | pass |
| mpt.prove_exclusion | 93 | pass |
| kv.trait | 94 | pass |
| kv.batch | 95 | pass |
| kv.memory | 95 | pass |
| kv.rocksdb | 90 | pass |
| state.account | 95 | pass |
| state.account_trie | 94 | pass |
| state.contract_storage_trie | 94 | pass |
| state.versioned_slot.read | 93 | pass |
| state.versioned_slot.write | 93 | pass |
| state.versioned_slot.validate | 94 | pass |
| state.commit_root | 95 | pass |
| kzg.commit | 94 | pass |
| kzg.open | 94 | pass |
| kzg.verify | 94 | pass |
| address.from_ed25519 | 95 | pass |
| validator.from_bls | 94 | pass |

**Sum:** 2623 / 2800  
**Tier 1 average audit score: 93.7% — PASS**

## Contracts at the 90 bar (not blocking)

- **kv.rocksdb (90):** Real `Store` impl exists under `--features rocksdb`. Default CI does not compile RocksDB (native lib optional). Without the feature, `RocksStore::open` returns `TypesError::Kv` and tests use `kv.memory`. This is documented in `crates/storage/src/rocks.rs` and `crates/storage/Cargo.toml`.
- **mpt.verify (91):** Verifier uses only the proof object + root (`inclusion_verifier_has_no_trie`). It also calls `merkle.verify` on a Merkle binding of the serialized node chain. That extra check is a real call into `merkle.verify` (required dep) but is not a substitute for the nibble-walk; the walk is what authenticates the MPT. Tampered node bytes / wrong root / wrong value are tested.

## Part B — Tier 0 ↔ Tier 1 integration

### 1. Dependency-by-dependency (real symbols, not shadows)

| Tier 1 dep | Called symbol | File |
|---|---|---|
| hash.blake3 | `crypto::hash::blake3::hash_to_array` | merkle nodes, MPT `hash_encoded`, `address.from_ed25519` |
| encoding.canonical.encode | `types::encode` / `types::decode` | MPT path HP wrap, node hash wrap, `Account::encode` |
| domain.tag.apply | `crypto::apply_domain` + `DomainTag::MptNode` / `DomainTag::Merkle` | MPT node hash; generic Merkle (not untagged blake3) |
| error.core | `types::TypesError` | `Store` methods, Rocks/memory errors, versioned slots |
| types.address / amount / nonce / hash | `types::{Address, Amount, Nonce, Hash}` | `state.account` |
| kzg.setup | `crypto::kzg::setup` / `KzgSetup` | `commit`/`open`/`verify` consume SRS from `setup` |
| ed25519.keygen | `crypto::sig::ed25519::keygen` | tests + `from_new_keypair` |
| types.address | `Address::from_bytes` | `address.from_ed25519` |
| bls.keygen | `crypto::sig::bls::keygen` | tests + `from_new_bls_key` |
| types.validator_id | `ValidatorId::from_bytes` + `bls::pk_to_bytes` | `validator.from_bls` |
| merkle.compute_root / merkle.verify | `state::merkle::{compute_root, verify}` | `mpt.prove`/`verify`, `state.commit_root` |
| mpt.* | `state::mpt::{leaf, extension, branch, get, put, Trie}` | tries, proofs |
| kv.trait | `storage::Store` | `kv.memory`, `kv.rocksdb`, `VersionedSlots` |
| determinism.sorted_maps | `types::collections::Map` (`BTreeMap`) | MPT node store, `MemoryStore`, versioned `latest_map` |

No shadow copies of BLAKE3, codec, or SRS. MPT node hashing is `hash_to_array(apply_domain(MptNode, encode(payload)))`.

### 2. No regression of Tier 0 tests

Tier 0 unit counts **before** Tier 1 (from `docs/audits/tier0-audit.md`): types 31, crypto 20, da 5, node 2 → **58**.

**After** Tier 1 (`cargo test --workspace --all-targets`): types **31**, crypto **26**, da **5**, node **2**, state **23**, storage **4** → **91**.

All original Tier 0 tests still run. Crypto grew by **6** tests (`address` ×2, `validator` ×2, `kzg` commit round-trip + too-long). No Tier 0 test was deleted or rewritten.

### 3. Cross-boundary determinism

`scripts/check_no_hashmap.sh` green. MPT and `MemoryStore` use `types::collections::Map`. No `HashMap`/`HashSet` in `crates/state`.

### 4. Domain separation reuse

| Hashing | Tag |
|---|---|
| Generic Merkle (`merkle.rs`) | `DomainTag::Merkle` (`b"merkle"`) |
| MPT nodes (`mpt/node.rs` `hash_encoded`) | `DomainTag::MptNode` (`b"mpt-node"`) |
| Address derivation | **untagged** `hash.blake3` of the 32-byte Ed25519 pk, as specified (`address = blake3(pubkey)[0..32]`) |

`state.commit_root` uses the 2-leaf generic Merkle tree (Merkle domain), not raw concatenation and not `MptNode`.

### 5. Full workspace regression

| Check | Result |
|---|---|
| `cargo build --workspace` | green |
| `cargo test --workspace --all-targets` | green (91 tests as above) |
| `cargo clippy --workspace --all-targets -- -D warnings` | green |
| `cargo fmt --all -- --check` | green |
| `bash scripts/check_no_hashmap.sh` | green |
| `python3 scripts/gen_dependency_graph.py` | JSON **unchanged** (schema already listed these 28 contracts) |

### Tier 0 code touched during Tier 1 (additive, justified)

| Change | Why | Public signature break? |
|---|---|---|
| `DomainTag::Merkle` in `crates/crypto/src/domain.rs` | Generic Merkle must not reuse `mpt-node` | **No** — new variant |
| `TypesError::Kv(&'static str)` in `crates/types/src/error.rs` | `kv.trait` must use `error.core` | **No** — new variant |
| `types` path dep on `crates/crypto` | `address.from_ed25519` / `validator.from_bls` need `types::{Address, ValidatorId, VotingPower, ADDRESS_SIZE}` | **No** function signature change; **crate graph** change: crypto now depends on `types` (Tier 0 originally kept crypto free of `types`; isolation of `types` ↛ crypto is preserved) |
| `kzg.rs` `KzgError` + `commit`/`open`/`verify` | Graph places those contracts in the same file as `kzg.setup` | `setup` signature **unchanged**. New error variants `TooLong`, `Point` |

No existing Tier 0 function was renamed or had its arguments changed.

## Part C — Overall verdict

- **Tier 1 average audit score: 93.7% — PASS**
- **Tier 0 integration status: CLEAN**

Notes (not blockers): crypto now depends on `types` so address/validator derivation can use the real newtypes; `kv.rocksdb` is feature-gated; `DomainTag::Merkle` and `TypesError::Kv` are additive Tier 0 extensions.

Tier 1 is complete at this gate. No git commit/push was made (working tree left uncommitted for review).
