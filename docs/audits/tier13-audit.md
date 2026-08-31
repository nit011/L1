# Tier 13 audit

**Date:** 2026-08-31  
**Scope:** 5 contracts in `docs/dependency-graph.json` → `tiers.tier_13` (`crates/light/src/{header,account,sync,ibc}.rs`)  
**Rubric:** 30 correctness + 20 tests + 15 isolation + 10 docs + 15 lint/CI + 10 path fidelity = 100

Tiers 0–12 audits all report PASS (≥ 90%). Step 0: `cargo build --workspace`, clippy `-D warnings`, `fmt --check` green. Full `cargo test --workspace` hit pre-existing simnet flakes under `--test-threads=8`; the same simnet file passed with `--test-threads=1` (all 4 tests). Integration re-run after Tier 13: `light` 12/12, `rpc` 14/14, `node` lib 14/14, simnet 4/4.

Tier 12 Part C: **`header.hash` LEFT UNCHANGED** (preimage still includes `da_root:32`; committed headers keep `DA_ROOT_PLACEHOLDER`). This tier’s QC fixtures use that definition.

## Part A — Per-contract scores

| Contract id | Score /100 | ≥90 |
|---|---:|:---:|
| light.verify_qc | 94 | pass |
| light.verify_account | 94 | pass |
| light.sync_checkpoints | 94 | pass |
| ibc.commitment | 93 | pass |
| ibc.verify_packet | 94 | pass |

**Sum:** 469 / 500  
**Tier 13 average audit score: 93.8% — PASS**

### Notes (not blocking)

- **`ibc.commitment` (93):** Merkle + MPT dual binding is the ICS-*shaped* primitive, not channels/connections/handshakes (Tier 20). Documented in `ibc.rs`.
- **`light.verify_account`:** `getProof` proves the *account trie*. The client binds `accountRoot || storageRoot` to the QC-verified `header.state_root` via `types::block::state_root` (same formula as `state.commit_root`) so the RPC cannot nominate an unbound trie root. RPC `stateRoot` in JSON is ignored.
- **No quorum math / MPT walk in `crates/light`:** `qc::verify` and `mpt::proof::verify` only.

## Part B — Tier 0–12 ↔ Tier 13 integration

### 1. Dependency-by-dependency

| Contract | Dep | Real symbol called |
|---|---|---|
| light.verify_qc | qc.verify | `consensus::qc::verify` |
| | header.hash | `types::header::Header::hash` (compares to `qc.block`) |
| light.verify_account | light.verify_qc | `header::verify_qc` |
| | mpt.verify | `state::mpt::proof::verify` |
| | service.l1.jsonrpc.getProof | `rpc::state::get_proof` via `GetProof` (honest `RpcInner` and tampering wrappers both call it) |
| light.sync_checkpoints | light.verify_qc | `verify_qc` on each header from the checkpoint |
| | ws.checkpoint | `consensus::checkpoint::Checkpoint` + `record_checkpoint` in tests |
| ibc.commitment | merkle.compute_root | `state::merkle::compute_root` over packet leaves |
| | light.verify_qc | `verify_qc` before committing |
| ibc.verify_packet | ibc.commitment | `IbcCommitment` / `commitment(...)` |
| | mpt.verify | `state::mpt::proof::verify` |

**Earlier-tier public signatures:** none changed.

### 2. No regression

- **Before (Tier 12 audit):** **313** workspace tests.
- **After:** **325** (`+12` in `crates/light`).
- Golden vectors / `header.hash` preimage layout: unchanged.

### 3. Trust-boundary (adversarial source)

See Part C. Honest-only paths exist but are not the only tests.

### 4. Frozen-header consistency

Fixtures set `da_root: DA_ROOT_PLACEHOLDER`. `verify_qc` uses `Header::hash()`, whose preimage still ends with `da_root:32` (Tier 12 did not extend the layout). `header::tests::happy_path_qc_covers_header_hash` asserts the placeholder and calls `hash_preimage()`.

### 5. Checkpoint chain-of-trust

`sync_checkpoints` requires a header at `checkpoint.height` whose `header.hash()` equals `checkpoint.header_hash`. A valid QC on a *different* genesis header (`alternate_history_not_through_checkpoint_fails`) is rejected as `LightError::Checkpoint` even though `qc::verify` on that fake QC succeeds. Omitting the checkpoint height also fails. This is the verification-side counterpart of `ws.bootstrap` refusing a missing/wrong checkpoint hash.

### 6. Full workspace regression

`cargo test --workspace` covering Tiers 0–13: library tests green; simnet is timing-sensitive under high parallelism and was re-run green single-threaded. Clippy `-D warnings` and `fmt --check` green. `python3 scripts/gen_dependency_graph.py`: **`docs/dependency-graph.json` unchanged**.

## Part C — Trust-minimization verdict

| Contract | Adversarial scenario | Crypto reject? | Success-looking source insufficient? |
|---|---|---|---|
| light.verify_qc | QC aggregated for header A, presented with header B (`qc.verify` still Ok on A) | **PASS** `HeaderMismatch` | QC bytes well-formed ≠ accept |
| light.verify_qc | Flipped `agg_sig` | **PASS** `Qc` via `qc.verify` | — |
| light.verify_account | Wrapper calls `get_proof` (RPC success) then XORs proof `nodes[0]` | **PASS** `Proof` (`mpt.verify`) | Yes: JSON result still `Ok` |
| light.verify_account | Same, but lies about `accountRoot` so it no longer combines to `header.state_root` | **PASS** `Proof` | RPC `stateRoot` ignored |
| light.sync_checkpoints | Alternate H0 with valid QC, hash ≠ checkpoint | **PASS** `Checkpoint` | Peer can gossip valid QCs |
| ibc.verify_packet | Tampered packet value and/or Merkle sibling after honest `commitment` | **PASS** `Proof` | Presenter identity unused |
| ibc.verify_packet | Proof against wrong `mpt_root` | **PASS** `Proof` | — |

No path treats HTTP/JSON success, `GetProof::get_proof` `Ok`, or a passing isolated `qc.verify` as enough without the matching `header.hash` / MPT / checkpoint hash checks.

## Part D — Overall verdict

- **Tier 13 average audit score: 93.8% — PASS**
- **Tier 0–12 integration status: CLEAN**
- **Trust-minimization verification: ALL CONFIRMED**

Tier 13 is complete on the audit bar. No git commit/push was made.
