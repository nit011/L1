#!/usr/bin/env bash
# Fail if HashMap/HashSet appear in consensus-critical crates
# (determinism.sorted_maps / development-plan.md Tier 0).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MATCHES="$(grep -RIn --include='*.rs' -E '\b(HashMap|HashSet)\b' \
  "$ROOT/crates/state" "$ROOT/crates/execution" "$ROOT/crates/consensus" \
  "$ROOT/crates/mempool" "$ROOT/crates/storage" "$ROOT/crates/network" \
  "$ROOT/crates/node" || true)"
if [[ -n "$MATCHES" ]]; then
  echo "$MATCHES"
  echo "HashMap/HashSet are forbidden in crates/state, crates/execution, crates/consensus, crates/mempool, crates/storage, crates/network, crates/node."
  exit 1
fi
echo "ok: no HashMap/HashSet in state/execution/consensus/mempool/storage/network/node"
