#!/usr/bin/env bash
# Local verification for Tier 18: genesis materialize, bootstrap failure,
# optional docker compose consensus + genesis.hash compare + finality timings.
#
# From repo root: bash infra/verify.sh

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"

echo "== materialize genesis (iac.genesis_config)"
cargo run -q -p iac --bin l1-genesis -- "$ROOT/infra/data"
HASH="$(tr -d '[:space:]' < infra/data/shared/genesis.hash)"
echo "shared genesis.hash=$HASH"
for i in 0 1 2 3; do
  h="$(tr -d '[:space:]' < "infra/data/node${i}/genesis.hash")"
  b_shared="$(if command -v sha256sum >/dev/null; then sha256sum infra/data/shared/genesis.bin | awk '{print $1}'; else shasum -a 256 infra/data/shared/genesis.bin | awk '{print $1}'; fi)"
  b_node="$(if command -v sha256sum >/dev/null; then sha256sum "infra/data/node${i}/genesis.bin" | awk '{print $1}'; else shasum -a 256 "infra/data/node${i}/genesis.bin" | awk '{print $1}'; fi)"
  [[ "$h" == "$HASH" ]] || { echo "hash mismatch node$i $h vs $HASH"; exit 1; }
  [[ "$b_shared" == "$b_node" ]] || { echo "genesis.bin mismatch node$i"; exit 1; }
done
echo "genesis.bin sha256 identical on all 4 nodes + shared ($b_shared)"

echo "== bootstrap.sh missing-genesis failure"
tmpdir="$(mktemp -d)"
export L1_DATA="$tmpdir/data"
export L1_GENESIS="$tmpdir/missing.bin"
export L1_NODE_BIN="/bin/true"
mkdir -p "$L1_DATA"
if bash infra/bootstrap.sh; then
  echo "expected bootstrap failure"; exit 1
else
  echo "bootstrap correctly exited non-zero without genesis"
fi
unset L1_NODE_BIN

wait_tip() {
  local dir="$1" min="$2" deadline="$3"
  while true; do
    if [[ -f "$dir/tip" ]]; then
      local h
      h="$(head -n1 "$dir/tip" || true)"
      if [[ -n "$h" ]] && [[ "$h" -ge "$min" ]]; then
        echo "$h"
        return 0
      fi
    fi
    if [[ "$(date +%s)" -ge "$deadline" ]]; then
      echo "timeout waiting tip>=$min in $dir" >&2
      [[ -f "$dir/events.log" ]] && cat "$dir/events.log" >&2
      return 1
    fi
    sleep 0.2
  done
}

echo "== native 4-node using generated dirs + bootstrap.sh (host QUIC)"
# Host path does not use L1_LISTEN (default 127.0.0.1) like simnet; after
# listen files exist we rewrite bootstrap.bin to those multiaddrs.
NODE_BIN="$ROOT/target/debug/node"
if [[ ! -x "$NODE_BIN" ]]; then
  cargo build -q --bin node
fi
HOST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/l1-host-compose.XXXXXX")"
cargo run -q -p iac --bin l1-genesis -- "$HOST_ROOT"
# Clear compose dns4 bootstrap; nodes will listen on 127.0.0.1:0 then we rewire.
for i in 0 1 2 3; do
  : > "$HOST_ROOT/node$i/bootstrap.bin" || true
  # empty valid list: 0 peers
  printf '%s' $'\x00\x00\x00\x00' > "$HOST_ROOT/node$i/bootstrap.bin"
done

PIDS=()
cleanup_host() {
  for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null || true; done
}
trap cleanup_host EXIT

for i in 0 1 2 3; do
  (
    export L1_DATA="$HOST_ROOT/node$i"
    export L1_GENESIS="$HOST_ROOT/shared/genesis.bin"
    export L1_NODE_BIN="$NODE_BIN"
    unset L1_LISTEN || true
    bash "$ROOT/infra/bootstrap.sh"
  ) >"$HOST_ROOT/node$i/stdout.log" 2>"$HOST_ROOT/node$i/stderr.log" &
  PIDS+=("$!")
done

# Wait for listen addrs then write p2p.bootstrap the same way simnet does
# (binary format from a tiny python-less rewrite using the node's later dial loop).
python3 - "$HOST_ROOT" <<'PY'
import os, time, sys, pathlib, subprocess
root = pathlib.Path(sys.argv[1])
deadline = time.time() + 20
addrs = {}
while time.time() < deadline:
    ok = True
    for i in range(4):
        p = root / f"node{i}" / "listen"
        if not p.exists() or not p.read_text().strip():
            ok = False
            break
        addrs[i] = p.read_text().strip()
    if ok:
        break
    time.sleep(0.05)
else:
    sys.exit("no listen addrs")
print("listen", addrs)
PY
echo "== rewire p2p.bootstrap from listen files"
cargo run -q -p iac --bin l1-genesis -- rewire "$HOST_ROOT"
deadline=$(( $(date +%s) + 45 ))
for i in 0 1 2 3; do
  wait_tip "$HOST_ROOT/node$i" 0 "$deadline"
done
echo "host tips: $(head -n1 "$HOST_ROOT/node0/tip") (all nodes reached commit)"
for i in 0 1 2 3; do
  if ! grep -q '^GENESIS ' "$HOST_ROOT/node$i/events.log" 2>/dev/null; then
    echo "node$i missing GENESIS log" >&2
    cat "$HOST_ROOT/node$i/stderr.log" >&2 || true
    cat "$HOST_ROOT/node$i/events.log" >&2 || true
    exit 1
  fi
done
G0="$(awk '/^GENESIS /{print $2}' "$HOST_ROOT/node0/events.log" | head -1)"
for i in 1 2 3; do
  gi="$(awk '/^GENESIS /{print $2}' "$HOST_ROOT/node$i/events.log" | head -1)"
  [[ "$gi" == "$G0" ]] || { echo "GENESIS log mismatch $i"; exit 1; }
done
echo "host GENESIS logs identical: $G0 (matches shared $HASH)"
[[ "$G0" == "$HASH" ]] || { echo "log hash != shared file"; exit 1; }
cleanup_host
trap - EXIT
PIDS=()

if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  echo "== docker compose up (4 validators)"
  docker compose -f infra/docker-compose.yml build
  docker compose -f infra/docker-compose.yml up -d
  # Wait for COMMIT on node0
  deadline=$((SECONDS + 90))
  while [[ $SECONDS -lt $deadline ]]; do
    if docker compose -f infra/docker-compose.yml exec -T node0 sh -c 'test -f /data/tip && test "$(head -n1 /data/tip)" -ge 0'; then
      break
    fi
    sleep 1
  done
  echo "--- genesis.hash per container ---"
  hashes=()
  for s in node0 node1 node2 node3; do
    gh="$(docker compose -f infra/docker-compose.yml exec -T "$s" sh -c 'tr -d "[:space:]" < /data/genesis.hash')"
    ev="$(docker compose -f infra/docker-compose.yml exec -T "$s" sh -c 'grep ^GENESIS /data/events.log | head -1' || true)"
    echo "$s file=$gh events=$ev"
    hashes+=("$gh")
  done
  for h in "${hashes[@]}"; do
    [[ "$h" == "${hashes[0]}" ]] || { echo "compose genesis.hash DIVERGED"; docker compose -f infra/docker-compose.yml down; exit 1; }
  done
  echo "compose genesis.hash IDENTICAL ${hashes[0]}"

  echo "--- containerized finality (tip heights / timestamps) ---"
  python3 - <<'PY'
import time, subprocess, pathlib
def sh(args):
    return subprocess.check_output(args, text=True)
marks = []
t0 = time.time()
deadline = t0 + 60
last = None
while time.time() < deadline:
    try:
        tip = sh(["docker","compose","-f","infra/docker-compose.yml","exec","-T","node0","sh","-c","head -n1 /data/tip 2>/dev/null || true"]).strip()
    except subprocess.CalledProcessError:
        tip = ""
    if tip.isdigit():
        h = int(tip)
        if last != h:
            marks.append((h, int((time.time()-t0)*1000)))
            last = h
        if h >= 2:
            break
    time.sleep(0.2)
print("containerized_marks_ms", marks)
if len(marks) >= 2:
    iv = [marks[i][1]-marks[i-1][1] for i in range(1,len(marks))]
    print("containerized_intervals_ms", iv)
    print("containerized_time_to_first_ms", marks[0][1])
else:
    print("containerized_finality_incomplete", marks)
    raise SystemExit(1)
PY
  echo "== join profile (fifth container, same genesis)"
  docker compose -f infra/docker-compose.yml --profile join up -d joiner
  sleep 3
  jh="$(docker compose -f infra/docker-compose.yml exec -T joiner sh -c 'tr -d "[:space:]" < /data/genesis.hash')"
  [[ "$jh" == "${hashes[0]}" ]] || { echo "joiner genesis diverged"; exit 1; }
  echo "joiner genesis.hash matches validators"
  docker compose -f infra/docker-compose.yml --profile join down
else
  echo "note: docker daemon not running; skipped compose/finality"
fi

echo "verify.sh ok"
