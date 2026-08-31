# iac.dockerfile
#
# Multi-stage image for the L1 `node` binary (crates/node).
# Build stage compiler is pinned to the exact channel in
# `rust-toolchain.toml` (`tooling.rust_toolchain` = 1.93.0). Base tags are versioned.
# Runtime is a slim Debian image with only the binary (RocksDB is not
# enabled in the default `node` build; storage uses in-memory + files).
#
# Used by: infra/build.sh (deterministic hash), infra/docker-compose.yml,
# infra/terraform/main.tf (cloud example).

# syntax=docker/dockerfile:1.7

FROM rust:1.93.0-bookworm AS build

# Reproducible rustc / linker knobs (see infra/build.sh).
ARG SOURCE_DATE_EPOCH=1600000000
ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}
ENV CARGO_INCREMENTAL=0
ENV CARGO_TERM_COLOR=never
ENV RUSTFLAGS="-C debuginfo=0 -C strip=symbols -C link-arg=-Wl,--build-id=none --remap-path-prefix=/src=/src"
WORKDIR /src

# Do not copy rust-toolchain.toml: rustup inside this image already is 1.93.0,
# and a copied toml can trip "unsupported metadata version".
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY infra/Cargo.toml ./infra/Cargo.toml
COPY infra/genesis.rs ./infra/genesis.rs

RUN rustc --version | grep -F '1.93.0' \
    && cargo build --locked --release --bin node --manifest-path Cargo.toml

FROM debian:bookworm-20250407-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/node /usr/local/bin/node
COPY infra/bootstrap.sh /usr/local/bin/bootstrap.sh
RUN chmod +x /usr/local/bin/bootstrap.sh /usr/local/bin/node
# Compose / terraform mount genesis + per-node config at /data and /genesis.
ENTRYPOINT ["/usr/local/bin/bootstrap.sh"]
