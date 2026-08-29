//! Core types for the L1 (architecture.md §1, development-plan.md Tier 0).
//!
//! This crate must not depend on `crypto`. Hashing and signatures live next door
//! and are wired in later tiers (`// TODO(tier_1): wire to crypto once types+crypto
//! boundary is confirmed` where a type would otherwise need a digest).

pub mod address;
pub mod amount;
pub mod block;
pub mod chain_id;
pub mod clock;
pub mod collections;
pub mod encoding;
pub mod epoch;
pub mod error;
pub mod genesis;
pub mod hash;
pub mod hashing;
pub mod header;
pub mod height;
pub mod nonce;
pub mod params;
pub mod round;
pub mod spec;
pub mod tx;
pub mod validator;

pub use address::Address;
pub use amount::Amount;
pub use chain_id::ChainId;
pub use clock::{Clock, SystemClock, TestClock};
pub use collections::{Map, Set};
pub use encoding::{decode, encode, CODEC_VERSION};
pub use epoch::Epoch;
pub use error::TypesError;
pub use hash::Hash;
pub use height::Height;
pub use nonce::Nonce;
pub use params::{ParamId, ParamsRegistry};
pub use round::Round;
pub use spec::{
    ADDRESS_SIZE, GAS_TRANSFER, HASH_SIZE, MAX_BLOCK_BYTES, MAX_GAS, MAX_TIMESTAMP_DRIFT_MS,
    MAX_TX_BYTES, MEMPOOL_MAX_TXS, MIN_TX_FEE, TIMEOUT_DELTA_MS, TIMEOUT_PRECOMMIT_MS,
    TIMEOUT_PREVOTE_MS, TIMEOUT_PROPOSE_MS,
};
pub use validator::{ValidatorId, VotingPower};

#[cfg(test)]
mod tooling_files {
    #[test]
    fn rust_toolchain_is_pinned() {
        let s = include_str!("../../../rust-toolchain.toml");
        assert!(s.contains("1.93.0"), "{s}");
        assert!(s.contains("clippy"));
        assert!(s.contains("rustfmt"));
    }

    #[test]
    fn ci_runs_clippy_fmt_and_hashmap_check() {
        let s = include_str!("../../../.github/workflows/ci.yml");
        assert!(s.contains("-D warnings"), "{s}");
        assert!(s.contains("cargo fmt"));
        assert!(s.contains("check_no_hashmap"));
    }
}
