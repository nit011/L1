# Concepts

Terms below are **this repository’s** meanings. Citations are contract ids from `docs/dependency-graph.json`, source files, or `docs/architecture.md` sections.

---

## Part 1 — Plain language

**What this chain is.** A blockchain here is a sequence of **blocks**. Each block is a batch of **transactions** (transfers, stake ops, WASM deploy/call) plus a **header** that fingerprints the batch and the resulting **state**. Validators agree on one chain of headers using votes. Once they agree hard enough, that block is **final** — they will not later pick a conflicting block at the same height.

**Consensus and 2/3+.** Imagine a committee that can only pass a motion if more than two-thirds of weighted votes say yes. If up to one-third of members are dishonest or offline, the rest can still agree, and two conflicting motions cannot both pass. That is the BFT model in architecture.md §2: **2/3+ voting power** on prevotes/precommits. Implementation: `cons.qc` / `cons.commit` in `crates/consensus/src/qc.rs` and `steps.rs`.

**Transaction, block, finality.** A transaction is a signed intent (`tx.sign`). A block is an ordered list of those plus header roots. **Finality** is a **quorum certificate** on that header (`cons.commit`), not “wait N blocks.” One successful commit at a height is stored in `CommitLog` (`cons.safety`) so a second hash at that height is rejected.

**Validators, staking, slashing.** A validator is identified by a BLS public key (`types.validator_id`, 48 bytes). They put up **self-bond** and may take **delegation** (capped). If they **equivocate** (two different votes in the same round/step), evidence can **slash** a percentage of bond (`execution/src/slash.rs`, `SLASH_PERCENT` in `spec.rs`). That is “skin in the game”: cheating should cost money. **Tombstoning** keeps a slashed validator from immediately rejoining as if nothing happened (slash module tests).

**Smart contracts and WASM.** A contract is a WASM module stored under an address (`tx.deploy` / `execution/src/wasm/deploy.rs`). Calls run in Wasmtime with **fuel** as gas (`wasm.call`). Host **sload/sstore** touch versioned storage. This is not the EVM.

**Parallel execution (intuition).** Running every transaction one-by-one is simple but slow. Running them all at once is faster **if** they do not fight over the same account. If they do, you must detect the fight and re-run the losers in order. That is Block-STM (`stm.apply_block`): speculate, conflict graph, waves, OCC validate, re-exec. If STM disagrees with sequential apply, it **panics** rather than silently fork.

**Merkle proofs.** The state is a tree of hashes. A **proof** is the siblings needed to recompute the root. If that root matches a header you already trust (because of a QC), you do not have to download every account. That is `mpt.prove` / `light.verify_account`.

**Data availability.** “This block is valid” is not the same as “everyone can download the bytes.” An attacker can withhold chunks. Light clients **sample** a few chunks (`das.sample`); if samples fail, availability is **NotAvailable** (`das.fail_closed`) — never “timeout = available.” Full nodes that already have the body can keep committing; DAS does not stop `cons.commit` (architecture.md §6).

---

## Part 2 — Engineer’s glossary (this codebase)

**BFT.** Safety if &lt;1/3 voting power is Byzantine; liveness if &gt;2/3 is timely. Encoded as `halt_no_quorum` and QC thresholds in `consensus/src/steps.rs` (`cons.commit`). architecture.md §2.

**Tendermint-style rounds.** Height + round; steps propose → prevote → precommit → commit. Timeouts from `TimeoutConfig::from_spec` (`cons.timeout.config`). Propose: `consensus/src/propose.rs`. Votes: `vote.rs` / `steps.rs`. `node.wire.propose|vote|precommit|commit` glue this to gossip.

**Single-slot vs probabilistic finality.** A QC at height h **is** finality (`Finalized` in `cons.commit`). There is no “6 confirmations.” architecture.md §2 / §10 “&lt; 5 sec”.

**VRF and leader election.** `vrf.ecvrf.prove/verify` (`crypto/src/vrf.rs`); consensus wraps `derive_seed` + `weighted_leader` (`consensus/src/vrf.rs`, `cons.vrf`). Ticket is hash-mod total power. Missing this would make leaders predictable or unfair.

**BLS aggregation.** `bls.sign/verify/aggregate/verifyAggregate` (`crypto/src/sig/bls.rs`). QC is an aggregate of precommits (`cons.qc`). Without aggregation, vote bandwidth would scale with N.

**Quorum certificate.** `QuorumCertificate` in `consensus/src/qc.rs`. Verify against the validator map. Light clients use `light.verify_qc`.

**Equivocation and slashing.** Two conflicting votes → `evidence::equivocation` → `execution::slash::apply`. `SLASH_PERCENT` in `types/src/spec.rs`. Missing this: cheap double-signing.

**Tombstoning.** Slash bookkeeping prevents immediate full restake as an honest member; see `execution/src/slash.rs` tests.

**Weak subjectivity and checkpoints.** `cons.checkpoint` / `CHECKPOINT_INTERVAL` (`spec.rs`). Late joiners need a trusted recent hash (`node.catchup`, `sync.headers_then_bodies`). architecture.md §2/§9.

**Epoch and validator-set rotation.** `staking.epoch_set_update` (`execution/src/staking.rs`) at epoch boundaries; VRF leaders change with the set (Tier 9 tests).

**Delegation and caps.** `DELEGATION_CAP` / `effective_power` in staking. architecture.md §9.2. Prevents one nominator dominating a validator’s vote weight without bound.

**MPT.** `state/src/mpt/` — leaf/extension/branch (`mpt.node`), nibble path (`mpt.path`), `prove`/`verify` (`mpt.proof`). Hash: domain `MptNode` + BLAKE3. Inclusion vs exclusion: `prove` vs `prove_exclusion`. Wrong MPT → wrong `state_root` → fork.

**Header roots.** `types/src/header.rs`: `tx_root`, `state_root`, `receipts_root`, `validators_hash`, `da_root` (placeholder zeros until `apply_da_root`). Frozen preimage: height‖round‖proposer‖ts‖those 32-byte fields (`header.hash`). `exec.app_hash` is blake3(state‖tx‖receipts) **without** domain tag (`execution/src/seq.rs`) — golden vectors in `crates/execution/tests/golden.rs`.

**Versioned slots and OCC.** `state/src/version.rs`. Speculative txs record versions; STM `validate` aborts stale reads (`stm.validate`). Without this, parallel apply would commit lost updates.

**Block-STM.** `crates/execution/src/stm/`: speculate (`rwset`), graph (`graph`), schedule (`schedule`), OCC (`validate`), `reexec_sequential`. Contract `stm.apply_block`. **Not** called from `node` `main`/`build_local`.

**Hot-account contention.** Many txs on one nonce chain serialize in the conflict graph. architecture.md §3.5; stress `throughput.rs` hot vs independent profiles.

**Gas metering.** `GAS_TRANSFER/DEPLOY/CALL`, `MAX_GAS` (`spec.rs`); `execution/src/gas.rs`; WASM fuel (`wasm/call.rs`). Missing this: unbounded loops (`loop_wat` test).

**Nonce / replay.** `checks::nonce_check` (`execution/src/checks.rs`); account nonce in MPT. Wrong nonce → rejected receipt (golden `rejected_nonce_app_hash`).

**Mempool / RBF.** `mempool/src/{verify,rbf,order,limits,fees}.rs`. Fee order feeds `ReadyTxs` for `build_local`. RBF replaces same-nonce if fee rules pass.

**WASM host / sload-sstore.** `execution/src/wasm/host.rs` reads/writes `versioned` slots and contract storage trie. Boundary: no arbitrary host FS.

**Reentrancy policy.** **No reentrancy** — `World::executing` (`wasm/call.rs`). `host.reenter` is rejected. Frozen for contract authors.

**Reed-Solomon.** `da/src/rs.rs` + `chunk.rs`: **k=4 data, m=2 parity**. Reconstruct from any k. Wrong k/m → DA bandwidth/safety change.

**KZG.** Toy SRS `kzg.setup` (`crypto/src/kzg.rs`); `DaRoot.kzg` additional to Merkle (`da/src/root.rs`). Merkle is authoritative for `header.da_root` and DAS.

**DAS fail-closed.** `SAMPLE_COUNT=3` of 6 shards; withhold queried indices → `NotAvailable` (`da/src/das.rs`). Never treat timeout as available.

**Light clients / IBC-shaped.** `crates/light`: `verify_qc`, `verify_account`, `ibc.verify_packet`. Proofs against MPT + QC. Not Cosmos IBC wire protocol.

**Gossipsub / scoring.** `network/src/gossip.rs`, mesh swarm, topics `TOPIC_TX` / proposal / vote / DA. Peer scoring in gossipsub config (Tier 6).

**Eclipse / prefix cap.** `network` eclipse module: one IP prefix cannot fill the table (Tier 6 audit / tests).

**State rent / expiry.** `execution/src/rent.rs`, `state/src/expiry.rs` — expire unpaid accounts; reactivate with exclusion proof.

**Hot/cold pruning / archive.** Limits and snapshot path (`node/src/limits.rs`, `storage/src/snapshot.rs`). Not a full archival node product.

**Snapshot vs full replay.** `sync.snapshot` (`storage/src/snapshot.rs`) vs `replay_from_genesis` / `headers_then_bodies`. Must same `commit_root`. Joiner uses catchup (replay), not snapshot blobs, in compose.

**Deterministic builds.** `iac.deterministic_build` / `infra/build.sh` + Dockerfile pins. Linux `node` hash identical across two `--no-cache` builds (Tier 18). Darwin is not.

**Canonical encoding / domain separation.** `encoding.canonical` (`types/src/encoding.rs`); `domain.tag.apply` (`crypto/src/domain.rs`) prefixes `L1/{label}\0`. Mixing untagged hashes is a type confusion / replay risk (VRF seed test compares tagged vs raw blake3).

If any of these were missing or faked with stubs, later tiers that declare them as `dependencies` would be lying about the DAG. This audit re-checked that the named functions still exist and still pass tests.
