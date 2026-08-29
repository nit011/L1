#!/usr/bin/env python3
"""Generate docs/dependency-graph.json for this Rust L1.

Shape (tiers, contract ids, dependencies, schedule, critical_path) follows a
generic DAG template. Field names are Cargo/Rust only — no Go keys.
"""
from __future__ import annotations

import json
from collections import OrderedDict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs" / "dependency-graph.json"


def C(id, deps, rust_file, description=None, jsonrpc_method=None):
    d = {"id": id, "dependencies": deps, "rust_file": rust_file}
    if jsonrpc_method:
        d["jsonrpc_method"] = jsonrpc_method
    if description:
        d["description"] = description
    return d


tiers = OrderedDict()

tiers["tier_0"] = {
    "name": "Foundation Primitives",
    "description": "Zero dependencies - can start immediately",
    "rust_crate": "crates/types, crates/crypto",
    "contracts": [
        C("hash.blake3", [], "crates/crypto/src/hash/blake3.rs"),
        C("encoding.canonical.encode", [], "crates/types/src/encoding.rs"),
        C("encoding.canonical.decode", [], "crates/types/src/encoding.rs"),
        C("domain.tag.apply", [], "crates/crypto/src/domain.rs"),
        C("ed25519.sign", [], "crates/crypto/src/sig/ed25519.rs"),
        C("ed25519.verify", [], "crates/crypto/src/sig/ed25519.rs"),
        C("ed25519.keygen", [], "crates/crypto/src/sig/ed25519.rs"),
        C("bls.keygen", [], "crates/crypto/src/sig/bls.rs"),
        C("bls.sign", [], "crates/crypto/src/sig/bls.rs"),
        C("bls.verify", [], "crates/crypto/src/sig/bls.rs"),
        C("bls.aggregate", [], "crates/crypto/src/sig/bls.rs"),
        C("bls.verifyAggregate", [], "crates/crypto/src/sig/bls.rs"),
        C("bls.domain", [], "crates/crypto/src/sig/bls.rs"),
        C("vrf.ecvrf.prove", [], "crates/crypto/src/vrf.rs"),
        C("vrf.ecvrf.verify", [], "crates/crypto/src/vrf.rs"),
        C("reed_solomon.encode", [], "crates/da/src/rs.rs"),
        C("reed_solomon.decode", [], "crates/da/src/rs.rs"),
        C("kzg.setup", [], "crates/crypto/src/kzg.rs"),
        C("clock.injected", [], "crates/types/src/clock.rs"),
        C("spec.constants", [], "crates/types/src/spec.rs"),
        C("spec.params_registry", [], "crates/types/src/params.rs"),
        C("types.hash", [], "crates/types/src/hash.rs"),
        C("types.address", [], "crates/types/src/address.rs"),
        C("types.amount", [], "crates/types/src/amount.rs"),
        C("types.nonce", [], "crates/types/src/nonce.rs"),
        C("types.height", [], "crates/types/src/height.rs"),
        C("types.round", [], "crates/types/src/round.rs"),
        C("types.epoch", [], "crates/types/src/epoch.rs"),
        C("types.chain_id", [], "crates/types/src/chain_id.rs"),
        C("types.validator_id", [], "crates/types/src/validator.rs"),
        C("types.voting_power", [], "crates/types/src/validator.rs"),
        C("error.core", [], "crates/types/src/error.rs"),
        C("tooling.rust_toolchain", [], "rust-toolchain.toml"),
        C("tooling.clippy_ci", [], ".github/workflows/ci.yml"),
        C("determinism.sorted_maps", [], "crates/types/src/collections.rs"),
        C("tracing.conventions", [], "crates/node/src/tracing.rs"),
    ],
}

tiers["tier_1"] = {
    "name": "Core Data Structures",
    "description": "Depends only on Tier 0",
    "rust_crate": "crates/state, crates/storage",
    "contracts": [
        C("merkle.compute_root", ["hash.blake3"], "crates/state/src/merkle.rs"),
        C("merkle.verify", ["hash.blake3"], "crates/state/src/merkle.rs"),
        C("mpt.pathencoding", ["encoding.canonical.encode"], "crates/state/src/mpt/path.rs"),
        C("mpt.node.leaf", ["hash.blake3", "encoding.canonical.encode"], "crates/state/src/mpt/node.rs"),
        C("mpt.node.extension", ["hash.blake3", "encoding.canonical.encode"], "crates/state/src/mpt/node.rs"),
        C("mpt.node.branch", ["hash.blake3", "encoding.canonical.encode"], "crates/state/src/mpt/node.rs"),
        C("mpt.get", ["hash.blake3", "mpt.node.leaf", "mpt.node.branch", "mpt.pathencoding"], "crates/state/src/mpt/mod.rs"),
        C("mpt.put", ["hash.blake3", "encoding.canonical.encode", "mpt.node.leaf", "mpt.node.extension", "mpt.node.branch"], "crates/state/src/mpt/mod.rs"),
        C("mpt.delete", ["mpt.put", "mpt.get"], "crates/state/src/mpt/mod.rs"),
        C("mpt.prove", ["hash.blake3", "mpt.get", "merkle.compute_root"], "crates/state/src/mpt/proof.rs"),
        C("mpt.verify", ["hash.blake3", "merkle.verify", "mpt.prove"], "crates/state/src/mpt/proof.rs"),
        C("mpt.prove_exclusion", ["mpt.prove"], "crates/state/src/mpt/proof.rs"),
        C("kv.trait", ["error.core"], "crates/storage/src/kv.rs"),
        C("kv.memory", ["kv.trait"], "crates/storage/src/memory.rs"),
        C("kv.rocksdb", ["kv.trait"], "crates/storage/src/rocks.rs"),
        C("kv.batch", ["kv.trait"], "crates/storage/src/kv.rs"),
        C("state.account", ["types.address", "types.amount", "types.nonce", "types.hash"], "crates/state/src/account.rs"),
        C("state.account_trie", ["mpt.put", "mpt.get", "state.account"], "crates/state/src/tries.rs"),
        C("state.contract_storage_trie", ["mpt.put", "mpt.get"], "crates/state/src/tries.rs"),
        C("state.versioned_slot.read", ["kv.trait"], "crates/state/src/version.rs"),
        C("state.versioned_slot.write", ["state.versioned_slot.read"], "crates/state/src/version.rs"),
        C("state.versioned_slot.validate", ["state.versioned_slot.write"], "crates/state/src/version.rs"),
        C("state.commit_root", ["state.account_trie", "state.contract_storage_trie"], "crates/state/src/root.rs"),
        C("kzg.commit", ["kzg.setup"], "crates/crypto/src/kzg.rs"),
        C("kzg.open", ["kzg.commit"], "crates/crypto/src/kzg.rs"),
        C("kzg.verify", ["kzg.commit", "kzg.open"], "crates/crypto/src/kzg.rs"),
        C("address.from_ed25519", ["ed25519.keygen", "hash.blake3", "types.address"], "crates/crypto/src/address.rs"),
        C("validator.from_bls", ["bls.keygen", "types.validator_id"], "crates/crypto/src/validator.rs"),
    ],
}

tiers["tier_2"] = {
    "name": "Consensus Time & Randomness",
    "description": "Injected clock, timeouts, VRF seed and weighted leader lottery",
    "rust_crate": "crates/consensus",
    "contracts": [
        C("cons.timeout.config", ["spec.constants", "clock.injected"], "crates/consensus/src/timeout.rs"),
        C("cons.clock.bind", ["clock.injected", "cons.timeout.config"], "crates/consensus/src/timeout.rs"),
        C("header.timestamp.bounds", ["clock.injected", "types.height"], "crates/consensus/src/time.rs"),
        C("vrf.seed.derive", ["hash.blake3", "domain.tag.apply", "types.epoch"], "crates/consensus/src/vrf.rs"),
        C("vrf.leader.prove", ["vrf.ecvrf.prove", "vrf.seed.derive", "types.validator_id"], "crates/consensus/src/vrf.rs"),
        C("vrf.leader.verify", ["vrf.ecvrf.verify", "vrf.seed.derive"], "crates/consensus/src/vrf.rs"),
        C("vrf.leader.weighted", ["vrf.leader.verify", "types.voting_power"], "crates/consensus/src/vrf.rs"),
        C("cons.replay.vote", ["hash.blake3", "domain.tag.apply", "types.height", "types.round"], "crates/consensus/src/replay.rs"),
        C("cons.round_robin.testdouble", ["types.validator_id", "types.round"], "crates/consensus/src/leader.rs"),
    ],
}

tiers["tier_3"] = {
    "name": "Genesis, Transactions & Sequential Execution",
    "description": "Canonical apply_block spec - STM/WASM/FHE must match these roots",
    "rust_crate": "crates/execution, crates/types",
    "contracts": [
        C("genesis.alloc", ["state.account", "types.chain_id"], "crates/types/src/genesis.rs"),
        C("genesis.validators", ["validator.from_bls", "types.voting_power"], "crates/types/src/genesis.rs"),
        C("genesis.params", ["spec.params_registry", "cons.timeout.config"], "crates/types/src/genesis.rs"),
        C("genesis.hash", ["genesis.alloc", "genesis.validators", "genesis.params", "hash.blake3"], "crates/types/src/genesis.rs"),
        C("tx.envelope", ["types.chain_id", "types.nonce", "types.amount", "encoding.canonical.encode"], "crates/types/src/tx.rs"),
        C("tx.transfer", ["tx.envelope", "types.address"], "crates/types/src/tx.rs"),
        C("tx.sign", ["ed25519.sign", "tx.envelope", "domain.tag.apply"], "crates/crypto/src/tx.rs"),
        C("tx.verify_ed25519", ["ed25519.verify", "tx.envelope", "domain.tag.apply"], "crates/crypto/src/tx.rs"),
        C("tx.nonce_check", ["tx.envelope", "state.account"], "crates/execution/src/checks.rs"),
        C("tx.balance_check", ["tx.transfer", "state.account"], "crates/execution/src/checks.rs"),
        C("tx.gas_meter", ["spec.constants", "tx.envelope"], "crates/execution/src/gas.rs"),
        C("tx.fee_priority", ["tx.envelope", "tx.gas_meter"], "crates/execution/src/fees.rs"),
        C("exec.seq.apply_tx", ["tx.verify_ed25519", "tx.nonce_check", "tx.balance_check", "tx.gas_meter", "state.account_trie"], "crates/execution/src/seq.rs"),
        C("exec.receipt", ["exec.seq.apply_tx", "encoding.canonical.encode"], "crates/execution/src/receipt.rs"),
        C("exec.events", ["exec.seq.apply_tx"], "crates/execution/src/events.rs"),
        C("header.fields", ["types.height", "types.round", "types.validator_id", "header.timestamp.bounds"], "crates/types/src/header.rs"),
        C("block.tx_root", ["merkle.compute_root", "tx.envelope"], "crates/types/src/block.rs"),
        C("block.receipts_root", ["merkle.compute_root", "exec.receipt"], "crates/types/src/block.rs"),
        C("block.state_root", ["state.commit_root"], "crates/types/src/block.rs"),
        C("block.validators_hash", ["merkle.compute_root", "genesis.validators"], "crates/types/src/header.rs"),
        C("block.da_root.placeholder", ["types.hash"], "crates/types/src/header.rs"),
        C("header.hash", ["header.fields", "block.tx_root", "block.state_root", "block.receipts_root", "hash.blake3"], "crates/types/src/header.rs"),
        C("block.body", ["tx.envelope", "header.fields"], "crates/types/src/block.rs"),
        C("exec.seq.apply_block", ["exec.seq.apply_tx", "block.body", "block.state_root", "block.tx_root", "block.receipts_root"], "crates/execution/src/seq.rs"),
        C("exec.app_hash", ["exec.seq.apply_block", "hash.blake3"], "crates/execution/src/seq.rs"),
        C("exec.golden_vectors", ["exec.app_hash", "genesis.hash"], "crates/execution/tests/golden.rs"),
    ],
}

tiers["tier_4"] = {
    "name": "Chain Storage, WAL & Mempool",
    "description": "Durable chain and local tx admission",
    "rust_crate": "crates/storage, crates/mempool",
    "contracts": [
        C("store.header.put", ["kv.batch", "header.hash"], "crates/storage/src/blocks.rs"),
        C("store.block.put", ["store.header.put", "block.body"], "crates/storage/src/blocks.rs"),
        C("store.tx.by_hash", ["store.block.put", "tx.envelope"], "crates/storage/src/index.rs"),
        C("store.receipt.put", ["store.block.put", "exec.receipt"], "crates/storage/src/index.rs"),
        C("store.replay_from_genesis", ["store.block.put", "exec.seq.apply_block", "genesis.hash"], "crates/storage/src/replay.rs"),
        C("wal.execution", ["kv.batch", "exec.seq.apply_block"], "crates/storage/src/wal.rs"),
        C("mempool.verify", ["tx.verify_ed25519", "tx.nonce_check", "tx.balance_check", "tx.gas_meter"], "crates/mempool/src/verify.rs"),
        C("mempool.nonce_queue", ["mempool.verify", "types.nonce"], "crates/mempool/src/queue.rs"),
        C("mempool.fee_order", ["mempool.nonce_queue", "tx.fee_priority"], "crates/mempool/src/order.rs"),
        C("mempool.rbf", ["mempool.fee_order"], "crates/mempool/src/rbf.rs"),
        C("mempool.size_limits", ["spec.constants", "mempool.verify"], "crates/mempool/src/limits.rs"),
        C("mempool.min_fee", ["spec.params_registry", "mempool.verify"], "crates/mempool/src/fees.rs"),
        C("block.builder.local", ["mempool.fee_order", "exec.seq.apply_block", "genesis.params"], "crates/execution/src/builder.rs"),
    ],
}

tiers["tier_5"] = {
    "name": "BFT Engine (In-Process)",
    "description": "Tendermint propose/prevote/precommit/commit with static genesis validators",
    "rust_crate": "crates/consensus",
    "contracts": [
        C("vote.prevote", ["bls.sign", "bls.domain", "header.hash", "types.height", "types.round"], "crates/consensus/src/vote.rs"),
        C("vote.precommit", ["bls.sign", "bls.domain", "header.hash", "types.height", "types.round"], "crates/consensus/src/vote.rs"),
        C("vote.nil", ["vote.prevote"], "crates/consensus/src/vote.rs"),
        C("vote.verify", ["bls.verify", "vote.prevote", "cons.replay.vote"], "crates/consensus/src/vote.rs"),
        C("qc.aggregate", ["bls.aggregate", "vote.precommit", "types.voting_power"], "crates/consensus/src/qc.rs"),
        C("qc.verify", ["bls.verifyAggregate", "qc.aggregate"], "crates/consensus/src/qc.rs"),
        C("cons.propose", ["vrf.leader.weighted", "block.builder.local", "bls.sign"], "crates/consensus/src/propose.rs"),
        C("cons.lock", ["vote.precommit", "types.height", "types.round"], "crates/consensus/src/state.rs"),
        C("cons.round_change", ["cons.clock.bind", "vote.nil"], "crates/consensus/src/state.rs"),
        C("cons.prevote_step", ["cons.propose", "vote.prevote", "vote.verify"], "crates/consensus/src/steps.rs"),
        C("cons.precommit_step", ["cons.prevote_step", "vote.precommit"], "crates/consensus/src/steps.rs"),
        C("cons.commit", ["cons.precommit_step", "qc.verify", "exec.app_hash"], "crates/consensus/src/steps.rs"),
        C("cons.halt_no_quorum", ["cons.precommit_step", "types.voting_power"], "crates/consensus/src/safety.rs"),
        C("cons.safety.no_two_commits", ["cons.commit"], "crates/consensus/src/safety.rs"),
        C("evidence.equivocation", ["vote.verify", "encoding.canonical.encode"], "crates/consensus/src/evidence.rs"),
        C("wal.consensus", ["cons.propose", "vote.prevote", "kv.batch"], "crates/consensus/src/wal.rs"),
        C("wal.no_double_sign", ["wal.consensus", "evidence.equivocation"], "crates/consensus/src/wal.rs"),
        C("simnet.in_process", ["cons.commit", "genesis.validators", "store.block.put"], "crates/consensus/tests/simnet.rs"),
    ],
}

tiers["tier_6"] = {
    "name": "Networking & Gossip",
    "description": "libp2p QUIC, discovery, gossipsub, eclipse/DoS minimums",
    "rust_crate": "crates/network",
    "contracts": [
        C("p2p.identity", ["ed25519.keygen"], "crates/network/src/identity.rs"),
        C("p2p.quic", ["p2p.identity"], "crates/network/src/transport.rs"),
        C("p2p.kademlia", ["p2p.quic"], "crates/network/src/discovery.rs"),
        C("p2p.bootstrap", ["p2p.kademlia"], "crates/network/src/discovery.rs"),
        C("gossip.mesh", ["p2p.quic"], "crates/network/src/gossip.rs"),
        C("gossip.scoring", ["gossip.mesh"], "crates/network/src/scoring.rs"),
        C("gossip.schema", ["gossip.mesh", "encoding.canonical.encode"], "crates/network/src/codec.rs"),
        C("gossip.tx", ["gossip.mesh", "mempool.verify"], "crates/network/src/topics.rs"),
        C("gossip.proposal", ["gossip.mesh", "cons.propose"], "crates/network/src/topics.rs"),
        C("gossip.vote", ["gossip.mesh", "vote.verify"], "crates/network/src/topics.rs"),
        C("gossip.block", ["gossip.mesh", "header.hash", "merkle.verify"], "crates/network/src/topics.rs"),
        C("gossip.evidence", ["gossip.mesh", "evidence.equivocation"], "crates/network/src/topics.rs"),
        C("gossip.headers_first", ["gossip.block", "store.header.put"], "crates/network/src/blocks.rs"),
        C("mesh.validator", ["gossip.proposal", "gossip.vote", "genesis.validators"], "crates/network/src/validator_mesh.rs"),
        C("netsec.peer_rate_limit", ["gossip.mesh", "spec.constants"], "crates/network/src/rate_limit.rs"),
        C("netsec.ip_slot_cap", ["p2p.kademlia"], "crates/network/src/eclipse.rs"),
        C("sync.locator", ["header.hash", "store.header.put"], "crates/network/src/sync.rs"),
        C("sync.headers_then_bodies", ["sync.locator", "gossip.headers_first", "store.block.put"], "crates/network/src/sync.rs"),
        C("valid.block.consensus", ["gossip.block", "qc.verify"], "crates/network/src/validation.rs"),
        C("valid.block.reorg_safety", ["valid.block.consensus", "cons.safety.no_two_commits"], "crates/network/src/validation.rs"),
    ],
}

tiers["tier_7"] = {
    "name": "Node Wiring (MVP Chain)",
    "description": "Process event loop: mempool ↔ exec ↔ BFT ↔ store ↔ gossip. First multi-process chain.",
    "rust_crate": "crates/node",
    "contracts": [
        C("node.config", ["genesis.params", "p2p.bootstrap"], "crates/node/src/config.rs"),
        C("node.wire.mempool", ["mempool.fee_order", "gossip.tx"], "crates/node/src/wire.rs"),
        C("node.wire.propose", ["node.wire.mempool", "cons.propose", "gossip.proposal"], "crates/node/src/wire.rs"),
        C("node.wire.vote", ["cons.prevote_step", "cons.precommit_step", "gossip.vote", "mesh.validator"], "crates/node/src/wire.rs"),
        C("node.wire.commit", ["cons.commit", "store.block.put", "wal.execution", "gossip.block"], "crates/node/src/wire.rs"),
        C("node.wire.sync", ["sync.headers_then_bodies", "node.wire.commit"], "crates/node/src/wire.rs"),
        C("node.catchup", ["node.wire.sync", "store.replay_from_genesis"], "crates/node/src/sync.rs"),
        C("node.simnet.multiprocess", ["node.wire.commit", "node.config"], "crates/node/tests/simnet.rs"),
        C("mvp.finality_lan", ["node.simnet.multiprocess", "cons.commit"], "crates/node/tests/finality.rs"),
    ],
}

tiers["tier_8"] = {
    "name": "JSON-RPC API Gateway",
    "description": "Client submit tx and query state; JSON only at this boundary",
    "rust_crate": "crates/rpc",
    "client_method_scheme": "service.<namespace>.jsonrpc.<method> => <namespace>.<method>",
    "client_api_notes": [
        "Expose mpt.prove as l1_getProof alongside submit/query for light and SDK clients."
    ],
    "contracts": [
        C("rpc.server", ["node.config"], "crates/rpc/src/server.rs", "JSON-RPC HTTP/WS server"),
        C("service.l1.jsonrpc.submitTx", ["rpc.server", "node.wire.mempool", "netsec.peer_rate_limit"], "crates/rpc/src/tx.rs", "Submit signed tx into mempool", "l1_submitTx"),
        C("service.l1.jsonrpc.getTx", ["rpc.server", "store.tx.by_hash"], "crates/rpc/src/tx.rs", "Get tx by hash", "l1_getTransaction"),
        C("service.l1.jsonrpc.getBlock", ["rpc.server", "store.block.put"], "crates/rpc/src/block.rs", "Get block by height or hash", "l1_getBlock"),
        C("service.l1.jsonrpc.getAccount", ["rpc.server", "state.account_trie"], "crates/rpc/src/state.rs", "Get account balance/nonce", "l1_getAccount"),
        C("service.l1.jsonrpc.getProof", ["rpc.server", "mpt.prove", "block.state_root"], "crates/rpc/src/state.rs", "Account/storage Merkle proof", "l1_getProof"),
        C("service.l1.jsonrpc.getStatus", ["rpc.server", "cons.commit"], "crates/rpc/src/status.rs", "Height, round, syncing, peer count", "l1_getStatus"),
        C("service.l1.jsonrpc.subscribe", ["rpc.server", "gossip.mesh"], "crates/rpc/src/sub.rs", "Subscribe new heads / logs", "l1_subscribe"),
        C("service.l1.jsonrpc.unsubscribe", ["service.l1.jsonrpc.subscribe"], "crates/rpc/src/sub.rs", "Cancel subscription", "l1_unsubscribe"),
    ],
}

tiers["tier_9"] = {
    "name": "Staking, Slashing & Weak Subjectivity",
    "description": "Validator lifecycle, delegation caps, evidence execution, checkpoints",
    "rust_crate": "crates/execution, crates/consensus",
    "contracts": [
        C("tx.stake.bond", ["tx.envelope", "types.amount"], "crates/types/src/staking.rs"),
        C("tx.stake.unbond", ["tx.stake.bond"], "crates/types/src/staking.rs"),
        C("tx.stake.delegate", ["tx.stake.bond"], "crates/types/src/staking.rs"),
        C("tx.stake.undelegate", ["tx.stake.delegate"], "crates/types/src/staking.rs"),
        C("tx.stake.withdraw", ["tx.stake.unbond"], "crates/types/src/staking.rs"),
        C("staking.min_self_bond", ["tx.stake.bond", "spec.params_registry"], "crates/execution/src/staking.rs"),
        C("staking.delegation_cap", ["tx.stake.delegate", "spec.params_registry"], "crates/execution/src/staking.rs"),
        C("staking.unbonding_period", ["tx.stake.unbond", "types.epoch"], "crates/execution/src/staking.rs"),
        C("staking.epoch_set_update", ["staking.min_self_bond", "cons.commit", "block.validators_hash"], "crates/execution/src/staking.rs"),
        C("evidence.submission", ["evidence.equivocation", "bls.verify"], "crates/consensus/src/evidence.rs"),
        C("slash.apply", ["evidence.submission", "staking.min_self_bond"], "crates/execution/src/slash.rs"),
        C("slash.tombstone", ["slash.apply"], "crates/execution/src/slash.rs"),
        C("ws.checkpoint", ["cons.commit", "header.hash", "spec.constants"], "crates/consensus/src/checkpoint.rs"),
        C("ws.bootstrap", ["ws.checkpoint", "node.catchup"], "crates/node/src/ws.rs"),
        C("service.l1.jsonrpc.getCheckpoint", ["rpc.server", "ws.checkpoint"], "crates/rpc/src/status.rs", "Latest weak-subjectivity checkpoint", "l1_getCheckpoint"),
    ],
}

tiers["tier_10"] = {
    "name": "Parallel Execution (Block-STM)",
    "description": "Speculative RW sets; must equal sequential apply_block",
    "rust_crate": "crates/execution",
    "contracts": [
        C("stm.rwset.speculate", ["exec.seq.apply_tx", "state.versioned_slot.read"], "crates/execution/src/stm/rwset.rs"),
        C("stm.conflict_graph", ["stm.rwset.speculate"], "crates/execution/src/stm/graph.rs"),
        C("stm.schedule", ["stm.conflict_graph"], "crates/execution/src/stm/schedule.rs"),
        C("stm.validate", ["stm.schedule", "state.versioned_slot.validate"], "crates/execution/src/stm/validate.rs"),
        C("stm.reexec_sequential", ["stm.validate", "exec.seq.apply_tx"], "crates/execution/src/stm/reexec.rs"),
        C("stm.apply_block", ["stm.reexec_sequential", "exec.seq.apply_block"], "crates/execution/src/stm/mod.rs"),
        C("stm.equals_seq", ["stm.apply_block", "exec.golden_vectors"], "crates/execution/tests/stm_equiv.rs"),
        C("stm.hot_account_bench", ["stm.apply_block"], "crates/execution/benches/hot_account.rs"),
    ],
}

tiers["tier_11"] = {
    "name": "WASM Contracts VM",
    "description": "Gas-metered WASM using contracts trie; sequential spec extended",
    "rust_crate": "crates/execution",
    "contracts": [
        C("tx.deploy", ["tx.envelope"], "crates/types/src/tx.rs"),
        C("tx.call", ["tx.envelope", "types.address"], "crates/types/src/tx.rs"),
        C("wasm.meter", ["tx.gas_meter"], "crates/execution/src/wasm/gas.rs"),
        C("wasm.host.sload", ["state.contract_storage_trie", "state.versioned_slot.read"], "crates/execution/src/wasm/host.rs"),
        C("wasm.host.sstore", ["wasm.host.sload", "state.versioned_slot.write"], "crates/execution/src/wasm/host.rs"),
        C("wasm.deploy", ["tx.deploy", "wasm.meter", "state.account_trie"], "crates/execution/src/wasm/deploy.rs"),
        C("wasm.call", ["tx.call", "wasm.deploy", "wasm.host.sstore"], "crates/execution/src/wasm/call.rs"),
        C("exec.seq.apply_tx.wasm", ["wasm.call", "exec.seq.apply_tx"], "crates/execution/src/seq.rs"),
        C("stm.apply_block.wasm", ["exec.seq.apply_tx.wasm", "stm.apply_block"], "crates/execution/src/stm/mod.rs"),
    ],
}

tiers["tier_12"] = {
    "name": "Data Availability",
    "description": "Erasure-coded chunks, DA root in header, sampling for light nodes",
    "rust_crate": "crates/da",
    "contracts": [
        C("da.chunk.split", ["reed_solomon.encode", "block.body"], "crates/da/src/chunk.rs"),
        C("da.chunk.reconstruct", ["reed_solomon.decode", "da.chunk.split"], "crates/da/src/chunk.rs"),
        C("da.root", ["da.chunk.split", "merkle.compute_root", "kzg.commit"], "crates/da/src/root.rs"),
        C("header.da_root", ["da.root", "block.da_root.placeholder"], "crates/types/src/header.rs"),
        C("gossip.da_chunks", ["gossip.mesh", "da.chunk.split"], "crates/network/src/topics.rs"),
        C("das.sample", ["gossip.da_chunks", "da.root"], "crates/da/src/das.rs"),
        C("das.fail_closed", ["das.sample"], "crates/da/src/das.rs"),
        C("node.wire.da", ["node.wire.commit", "header.da_root", "gossip.da_chunks"], "crates/node/src/wire.rs"),
    ],
}

tiers["tier_13"] = {
    "name": "Light Client & IBC-Shaped Verification",
    "description": "Verify BLS QC + MPT proofs; packet-shaped interop later productized",
    "rust_crate": "crates/light",
    "contracts": [
        C("light.verify_qc", ["qc.verify", "header.hash"], "crates/light/src/header.rs"),
        C("light.verify_account", ["light.verify_qc", "mpt.verify", "service.l1.jsonrpc.getProof"], "crates/light/src/account.rs"),
        C("light.sync_checkpoints", ["light.verify_qc", "ws.checkpoint"], "crates/light/src/sync.rs"),
        C("ibc.commitment", ["merkle.compute_root", "light.verify_qc"], "crates/light/src/ibc.rs"),
        C("ibc.verify_packet", ["ibc.commitment", "mpt.verify"], "crates/light/src/ibc.rs"),
    ],
}

tiers["tier_14"] = {
    "name": "Hardware Limits, Pruning & Netsec Hardening",
    "description": "Commodity validator budgets from architecture §9; state rent/expiry",
    "rust_crate": "crates/node, crates/storage, crates/network",
    "contracts": [
        C("limits.max_block_bytes", ["spec.params_registry", "genesis.params"], "crates/node/src/limits.rs"),
        C("limits.max_gas", ["limits.max_block_bytes", "tx.gas_meter"], "crates/node/src/limits.rs"),
        C("limits.state_growth", ["limits.max_gas", "state.commit_root"], "crates/node/src/limits.rs"),
        C("state.rent", ["tx.gas_meter", "state.account"], "crates/execution/src/rent.rs"),
        C("state.expiry", ["state.rent", "mpt.prove"], "crates/state/src/expiry.rs"),
        C("state.reactivate", ["state.expiry", "mpt.prove_exclusion"], "crates/state/src/expiry.rs"),
        C("prune.hot_cold", ["store.block.put", "kv.rocksdb"], "crates/storage/src/prune.rs"),
        C("sync.snapshot", ["state.commit_root", "store.replay_from_genesis"], "crates/storage/src/snapshot.rs"),
        C("netsec.asn_cap", ["netsec.ip_slot_cap"], "crates/network/src/eclipse.rs"),
        C("netsec.peer_rotation", ["netsec.asn_cap", "gossip.scoring"], "crates/network/src/eclipse.rs"),
        C("fee.1559_optional", ["mempool.min_fee", "limits.max_gas"], "crates/execution/src/fees.rs"),
    ],
}

tiers["tier_15"] = {
    "name": "Observability",
    "description": "Prometheus, tracing, SLOs against 1-2s blocks and <5s finality",
    "rust_crate": "crates/observability",
    "contracts": [
        C("obs.structured_logging", [], "crates/observability/src/logging.rs"),
        C("obs.prometheus_exporter", ["cons.commit", "gossip.mesh", "mempool.size_limits"], "crates/observability/src/prometheus.rs"),
        C("obs.otel_tracing", ["obs.prometheus_exporter", "service.l1.jsonrpc.submitTx"], "crates/observability/src/tracing.rs"),
        C("obs.slo_definitions", ["obs.prometheus_exporter", "mvp.finality_lan"], "crates/observability/src/slo.rs"),
        C("obs.alert_rules", ["obs.slo_definitions"], "crates/observability/src/alerts.rs"),
        C("obs.grafana_dashboards", ["obs.prometheus_exporter"], "configs/grafana/dashboards.json"),
    ],
}

tiers["tier_16"] = {
    "name": "SDK, Faucet & Client Integration",
    "description": "Sign, submit, wait for finality; testnet faucet",
    "rust_crate": "crates/sdk, crates/faucet",
    "contracts": [
        C("sdk.sign_tx", ["tx.sign", "address.from_ed25519"], "crates/sdk/src/sign.rs"),
        C("sdk.submit", ["sdk.sign_tx", "service.l1.jsonrpc.submitTx"], "crates/sdk/src/submit.rs"),
        C("sdk.wait_finality", ["sdk.submit", "service.l1.jsonrpc.getStatus"], "crates/sdk/src/finality.rs"),
        C("sdk.query_proof", ["sdk.wait_finality", "service.l1.jsonrpc.getProof"], "crates/sdk/src/proof.rs"),
        C("faucet.service", ["service.l1.jsonrpc.submitTx", "tx.transfer"], "crates/faucet/src/service.rs"),
        C("faucet.ratelimit", ["faucet.service"], "crates/faucet/src/ratelimit.rs"),
        C("sdk.e2e_integration_test", ["sdk.wait_finality", "faucet.service", "service.l1.jsonrpc.getAccount"], "crates/sdk/tests/e2e.rs"),
    ],
}

tiers["tier_17"] = {
    "name": "Operations & Emergency Controls",
    "description": "Pause, RBAC, audit log; params already in spec.params_registry",
    "rust_crate": "crates/node",
    "contracts": [
        C("gov.pause", ["spec.params_registry"], "crates/execution/src/pause.rs"),
        C("ops.pause_cli", ["gov.pause"], "crates/node/src/ops/pause.rs"),
        C("ops.rbac", ["ops.pause_cli", "ed25519.verify"], "crates/node/src/ops/rbac.rs"),
        C("ops.audit_log", ["ops.rbac", "obs.structured_logging"], "crates/node/src/ops/audit.rs"),
        C("ops.config_toggle", ["spec.params_registry", "ops.rbac"], "crates/node/src/ops/config.rs"),
    ],
}

tiers["tier_18"] = {
    "name": "IaC, Docker & Deterministic Build",
    "description": "Reproducible multi-node bring-up",
    "rust_crate": "infra/",
    "contracts": [
        C("iac.dockerfile", [], "infra/Dockerfile"),
        C("iac.deterministic_build", ["iac.dockerfile", "tooling.rust_toolchain"], "infra/build.sh"),
        C("iac.docker_compose", ["iac.dockerfile", "node.config"], "infra/docker-compose.yml"),
        C("iac.genesis_config", ["iac.docker_compose", "genesis.hash"], "infra/genesis.rs"),
        C("iac.node_bootstrap_scripts", ["iac.genesis_config", "p2p.bootstrap"], "infra/bootstrap.sh"),
        C("iac.terraform_cloud", ["iac.dockerfile"], "infra/terraform/main.tf"),
    ],
}

tiers["tier_19"] = {
    "name": "Stress Testing & Benchmarks",
    "description": "Multi-node load, STM vs seq, snapshot sync, hardware-budget checks",
    "rust_crate": "tests/stress",
    "contracts": [
        C("stress.load_harness", ["sdk.e2e_integration_test", "iac.docker_compose"], "tests/stress/harness.rs"),
        C("stress.consensus_4node", ["stress.load_harness", "mvp.finality_lan", "cons.commit"], "tests/stress/consensus.rs"),
        C("stress.throughput_benchmark", ["stress.load_harness", "stm.apply_block"], "tests/stress/throughput.rs"),
        C("stress.snapshot_sync_test", ["stress.load_harness", "sync.snapshot", "sync.headers_then_bodies"], "tests/stress/sync.rs"),
        C("stress.das_withhold", ["stress.load_harness", "das.fail_closed"], "tests/stress/das.rs"),
        C("stress.perf_regression_ci", ["stress.throughput_benchmark", "stress.consensus_4node"], "tests/stress/ci.rs"),
    ],
}

tiers["tier_20"] = {
    "name": "Roadmap - Verkle, ZK, Encrypted Mempool, FHE, HotStuff",
    "description": "Architecture-deferred; do not gate MVP. Slot into existing lanes.",
    "rust_crate": "crates/state, crates/mempool, crates/execution, crates/consensus",
    "contracts": [
        C("verkle.commit", ["kzg.commit", "state.account_trie"], "crates/state/src/verkle.rs"),
        C("verkle.proof", ["verkle.commit"], "crates/state/src/verkle.rs"),
        C("zk.validity.apply_block", ["exec.seq.apply_block", "kzg.verify"], "crates/zk/src/validity.rs"),
        C("zk.light_verify", ["zk.validity.apply_block", "light.verify_qc"], "crates/zk/src/light.rs"),
        C("enc_mempool.threshold_keygen", ["bls.keygen"], "crates/mempool/src/encrypted.rs"),
        C("enc_mempool.encrypt", ["enc_mempool.threshold_keygen"], "crates/mempool/src/encrypted.rs"),
        C("enc_mempool.decrypt_on_propose", ["enc_mempool.encrypt", "cons.propose"], "crates/mempool/src/encrypted.rs"),
        C("fhe.keygen.dkg", ["bls.keygen", "staking.epoch_set_update"], "crates/fhe/src/dkg.rs"),
        C("fhe.encrypt_input", ["fhe.keygen.dkg"], "crates/fhe/src/tx.rs"),
        C("fhe.eval_circuit", ["fhe.encrypt_input", "exec.seq.apply_tx"], "crates/fhe/src/eval.rs"),
        C("fhe.threshold_decrypt", ["fhe.eval_circuit", "fhe.keygen.dkg"], "crates/fhe/src/decrypt.rs"),
        C("fhe.zk_of_eval", ["fhe.eval_circuit", "zk.validity.apply_block"], "crates/fhe/src/prove.rs"),
        C("cons.hotstuff.pipeline", ["cons.commit"], "crates/consensus/src/hotstuff.rs"),
    ],
}

# --- assemble ---
all_ids = {}
independent = 0
for tname, t in tiers.items():
    for c in t["contracts"]:
        if c["id"] in all_ids:
            raise SystemExit(f"duplicate id {c['id']}")
        all_ids[c["id"]] = tname
        if not c["dependencies"]:
            independent += 1

missing = []
for tname, t in tiers.items():
    for c in t["contracts"]:
        for d in c["dependencies"]:
            if d not in all_ids:
                missing.append((c["id"], d))
if missing:
    raise SystemExit("missing deps:\n" + "\n".join(f"  {a} -> {b}" for a, b in missing))

total = len(all_ids)

schedule = OrderedDict()
sprint = 1
for i, (tname, t) in enumerate(tiers.items()):
    n = len(t["contracts"])
    key = f"sprint_{sprint}_{sprint + 1}"
    pkgs = t["rust_crate"]
    if isinstance(pkgs, str):
        pkg_list = [p.strip() for p in pkgs.split(",")]
    else:
        pkg_list = pkgs
    teams = max(2, min(8, (n + 3) // 4))
    schedule[key] = {
        "tier": i,
        "description": f"Tier {i} - {t['name']}",
        "contracts_count": n,
        "parallel_teams": teams,
        "rust_crates": pkg_list,
    }
    sprint += 2

doc = OrderedDict(
    [
        ("version", "1.1"),
        ("project", "L1"),
        ("language", "Rust"),
        ("total_contracts", total),
        ("independent_contracts", independent),
        ("cargo_workspace", "l1"),
        ("rust_edition", "2021"),
        ("tiers", tiers),
        ("development_schedule", schedule),
        (
            "critical_path",
            [
                "hash.blake3",
                "bls.sign",
                "vrf.ecvrf.verify",
                "mpt.put",
                "state.commit_root",
                "genesis.hash",
                "tx.transfer",
                "exec.seq.apply_block",
                "exec.app_hash",
                "store.block.put",
                "mempool.fee_order",
                "block.builder.local",
                "vrf.leader.weighted",
                "cons.propose",
                "cons.prevote_step",
                "cons.precommit_step",
                "cons.commit",
                "qc.verify",
                "p2p.quic",
                "gossip.mesh",
                "node.wire.commit",
                "mvp.finality_lan",
                "service.l1.jsonrpc.submitTx",
                "staking.epoch_set_update",
                "slash.apply",
                "ws.checkpoint",
                "stm.equals_seq",
                "wasm.call",
                "da.root",
                "das.fail_closed",
                "light.verify_account",
                "limits.max_gas",
                "sdk.e2e_integration_test",
                "iac.docker_compose",
                "stress.consensus_4node",
            ],
        ),
        (
            "architecture_coverage",
            {
                "description": "architecture.md sections mapped to contract tiers",
                "sections": [
                    {"id": "§2 consensus", "tiers": ["tier_2", "tier_5", "tier_9"]},
                    {"id": "§3 execution / STM", "tiers": ["tier_3", "tier_10", "tier_11"]},
                    {"id": "§4 state / MPT / prune", "tiers": ["tier_1", "tier_14"]},
                    {"id": "§5 networking", "tiers": ["tier_6", "tier_14"]},
                    {"id": "§6 data availability", "tiers": ["tier_12"]},
                    {"id": "§7 cryptography", "tiers": ["tier_0", "tier_20"]},
                    {"id": "§8 FHE privacy", "tiers": ["tier_20"]},
                    {"id": "§9 hardware / economics", "tiers": ["tier_9", "tier_14"]},
                    {"id": "§10 design targets", "tiers": ["tier_7", "tier_15", "tier_19"]},
                    {"id": "§11 E2E client/RPC", "tiers": ["tier_8", "tier_16"]},
                ],
            },
        ),
        (
            "phase_status",
            {
                "description": "MVP vs roadmap vs ops from docs/development-plan.md",
                "items": [
                    {"id": "MVP-1", "name": "Sequential spec + store + mempool", "status": "planned", "tier": "tier_3,tier_4"},
                    {"id": "MVP-2", "name": "In-process BFT + VRF", "status": "planned", "tier": "tier_5"},
                    {"id": "MVP-3", "name": "Networked chain + RPC (devnet)", "status": "planned", "tier": "tier_6,tier_7,tier_8"},
                    {"id": "TNET-1", "name": "Staking / slash / weak subjectivity", "status": "planned", "tier": "tier_9"},
                    {"id": "SCALE-1", "name": "Block-STM == sequential spec", "status": "planned", "tier": "tier_10"},
                    {"id": "SCALE-2", "name": "WASM contracts", "status": "planned", "tier": "tier_11"},
                    {"id": "DA-1", "name": "Erasure coding + DAS", "status": "planned", "tier": "tier_12"},
                    {"id": "LC-1", "name": "Light client + IBC-shaped verify", "status": "planned", "tier": "tier_13"},
                    {"id": "OPS-1", "name": "Observability, faucet, IaC, stress", "status": "planned", "tier": "tier_15,tier_16,tier_18,tier_19"},
                    {"id": "RMAP-1", "name": "Verkle / zk / encrypted mempool / FHE / HotStuff", "status": "roadmap", "tier": "tier_20"},
                ],
            },
        ),
        (
            "forbidden_edges",
            {
                "description": "Edges that would create implementation gaps",
                "rules": [
                    "stm.apply_block must not ship without stm.equals_seq",
                    "cons.commit must not depend on tx.stake.bond (static genesis validators first)",
                    "gossip.* must not own finality rules (qc.verify / cons.commit do)",
                    "das.sample must not block first cons.commit (full blocks first)",
                    "rpc must not define tx validity (mempool.verify + exec.seq.apply_tx do)",
                    "FHE / Verkle / zk / enc_mempool must not gate mvp.finality_lan",
                ],
            },
        ),
        (
            "metadata",
            {
                "version": "1.1",
                "last_updated": "2026-08-29",
                "language": "Rust 2021",
                "cargo_workspace": "l1",
                "sources": [
                    "docs/architecture.md",
                    "docs/development-plan.md",
                ],
                "changelog": [
                    {
                        "version": "1.1",
                        "date": "2026-08-29",
                        "changes": "Rust-only keys: rust_file, rust_crate, rust_crates, cargo_workspace (no language-foreign path fields).",
                    },
                    {
                        "version": "1.0",
                        "date": "2026-08-29",
                        "changes": "Initial L1 contract DAG from architecture.md and development-plan.md.",
                    }
                ],
            },
        ),
    ]
)

# critical path ids must exist
for cid in doc["critical_path"]:
    if cid not in all_ids:
        raise SystemExit(f"critical_path unknown id {cid}")

OUT.write_text(json.dumps(doc, indent=2) + "\n")
print(f"wrote {OUT}")
print(f"total_contracts={total} independent_contracts={independent} tiers={len(tiers)}")
