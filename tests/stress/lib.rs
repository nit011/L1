//! Stress suite (Tier 19). Default `cargo test` runs fast unit checks.
//! Docker-Compose load: `cargo test -p stress -- --ignored --nocapture --test-threads=1`.

pub mod ci;
pub mod consensus;
pub mod das;
pub mod harness;
pub mod sync;
pub mod throughput;
