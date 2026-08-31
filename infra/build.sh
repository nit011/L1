#!/usr/bin/env bash
# iac.deterministic_build
#
# Build the node binary twice in independent contexts and require the
# content hashes to match. Uses `infra/Dockerfile` (iac.dockerfile) and
# the exact compiler channel from `rust-toolchain.toml`
# (tooling.rust_toolchain).
#
# Usage (from repo root):
#   bash infra/build.sh
#
# Environment:
#   L1_SKIP_DOCKER=1  — native isolated cargo builds only (no image).
#   L1_DOCKER_ONLY=1  — fail if docker is missing (default when docker exists).

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

CHANNEL="$(awk -F'"' '/^channel/ {print $2; exit}' rust-toolchain.toml)"
if [[ "$CHANNEL" != "1.93.0" ]]; then
  echo "error: tooling.rust_toolchain channel is '$CHANNEL', expected 1.93.0" >&2
  exit 1
fi

RUSTC_VER="$(rustc --version || true)"
if [[ "$RUSTC_VER" != *"1.93.0"* ]]; then
  echo "error: rustc is not 1.93.0 (got: $RUSTC_VER)" >&2
  exit 1
fi

export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1600000000}"
export CARGO_INCREMENTAL=0
export ZERO_AR_DATE=1

hash_file() {
  local f="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$f" | awk '{print $1}'
  else
    shasum -a 256 "$f" | awk '{print $1}'
  fi
}

native_pair() {
  local node_a ha hb
  node_a="$(mktemp "${TMPDIR:-/tmp}/l1-node-a.XXXXXX")"
  # Sequential isolated trees: two full compiles, first target dir deleted
  # before the second starts (keeps disk use to one release tree).
  local flags="-C debuginfo=0 -C strip=symbols --remap-path-prefix=${ROOT}=/l1"
  if [[ "$(uname -s)" != "Darwin" ]]; then
    flags+=" -C link-arg=-Wl,--build-id=none"
  fi
  local a b
  a="$(mktemp -d "${TMPDIR:-/tmp}/l1-det-a.XXXXXX")"
  echo "native isolated build A in $a"
  RUSTFLAGS="$flags" CARGO_TARGET_DIR="$a" cargo build --locked --release --bin node
  cp "$a/release/node" "$node_a"
  ha="$(hash_file "$node_a")"
  rm -rf "$a"
  b="$(mktemp -d "${TMPDIR:-/tmp}/l1-det-b.XXXXXX")"
  echo "native isolated build B in $b"
  RUSTFLAGS="$flags" CARGO_TARGET_DIR="$b" cargo build --locked --release --bin node
  hb="$(hash_file "$b/release/node")"
  rm -rf "$b"
  rm -f "$node_a"
  echo "native run1 $ha"
  echo "native run2 $hb"
  if [[ "$ha" != "$hb" ]]; then
    if [[ "$(uname -s)" == "Darwin" ]]; then
      echo "warning: native Darwin hashes differ (Mach-O UUID); use docker pair on Linux" >&2
    else
      echo "error: native binaries differ" >&2
      echo "  A=$ha" >&2
      echo "  B=$hb" >&2
      exit 1
    fi
  else
    echo "DETERMINISTIC_NATIVE_HASH=$ha"
  fi
}

docker_pair() {
  if ! command -v docker >/dev/null 2>&1; then
    echo "error: docker not found (install Docker / colima)" >&2
    exit 1
  fi
  local tag1="l1-node:det1-$$"
  local tag2="l1-node:det2-$$"
  echo "docker build --no-cache (1/2) using infra/Dockerfile"
  docker build --no-cache --pull=false \
    --build-arg SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" \
    -f infra/Dockerfile -t "$tag1" "$ROOT"
  c1="$(docker create "$tag1")"
  d="$(mktemp -d "${TMPDIR:-/tmp}/l1-docker-bin.XXXXXX")"
  docker cp "$c1:/usr/local/bin/node" "$d/node1"
  docker rm -f "$c1" >/dev/null
  docker rmi "$tag1" >/dev/null || true
  echo "docker build --no-cache (2/2) using infra/Dockerfile"
  docker build --no-cache --pull=false \
    --build-arg SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" \
    -f infra/Dockerfile -t "$tag2" "$ROOT"
  c2="$(docker create "$tag2")"
  docker cp "$c2:/usr/local/bin/node" "$d/node2"
  docker rm -f "$c2" >/dev/null
  docker rmi "$tag2" >/dev/null || true
  h1="$(hash_file "$d/node1")"
  h2="$(hash_file "$d/node2")"
  echo "docker run1 $h1"
  echo "docker run2 $h2"
  if [[ "$h1" != "$h2" ]]; then
    echo "error: docker binaries differ" >&2
    echo "  1=$h1" >&2
    echo "  2=$h2" >&2
    exit 1
  fi
  echo "DETERMINISTIC_DOCKER_HASH=$h1"
}

case "${1:-all}" in
  native)
    native_pair
    ;;
  docker)
    docker_pair
    ;;
  all|*)
    native_pair
    if command -v docker >/dev/null 2>&1 && [[ "${L1_SKIP_DOCKER:-}" != "1" ]]; then
      docker_pair
    elif [[ "${L1_DOCKER_ONLY:-}" == "1" ]]; then
      echo "error: docker required" >&2
      exit 1
    else
      echo "note: docker not available; native pair only"
    fi
    ;;
esac
