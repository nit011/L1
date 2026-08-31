#!/usr/bin/env bash
# iac.node_bootstrap_scripts
#
# Container (or host) entrypoint:
#   1. Load the shared genesis file from iac.genesis_config (`genesis.bin`)
#      — never generate a new genesis here.
#   2. Ensure bootstrap.bin is present (p2p.bootstrap: PeerId → Multiaddr).
#   3. Exec the node binary with `--dir` (node.config on disk).
#
# Required env:
#   L1_DATA     node data directory (NodeConfig.data_dir)
#   L1_GENESIS  path to the single shared genesis.bin
# Optional:
#   L1_LISTEN   libp2p multiaddr (see crates/node/src/main.rs)

set -euo pipefail

DATA="${L1_DATA:-/data}"
GENESIS_SRC="${L1_GENESIS:-/genesis/genesis.bin}"
NODE_BIN="${L1_NODE_BIN:-/usr/local/bin/node}"

mkdir -p "$DATA"

if [[ ! -f "$GENESIS_SRC" ]]; then
  echo "bootstrap: missing genesis at $GENESIS_SRC (iac.genesis_config)" >&2
  exit 1
fi

# Always overlay the shared bytes so a stale per-node copy cannot diverge.
cp -f "$GENESIS_SRC" "$DATA/genesis.bin"

if [[ ! -f "$DATA/bootstrap.bin" ]]; then
  echo "bootstrap: missing $DATA/bootstrap.bin (p2p.bootstrap)" >&2
  exit 1
fi

if [[ ! -f "$DATA/ed25519" ]] || [[ ! -f "$DATA/bls_sk" ]]; then
  echo "bootstrap: missing node.config identity files in $DATA" >&2
  exit 1
fi

if [[ ! -x "$NODE_BIN" ]]; then
  echo "bootstrap: node binary not executable: $NODE_BIN" >&2
  exit 1
fi

exec "$NODE_BIN" --dir "$DATA"
