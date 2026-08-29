//! Consensus time and randomness plumbing (architecture.md §2).
//!
//! Pure crate: no libp2p, no RocksDB, no execution. No propose/prevote/precommit
//! state machine (Tier 5).

pub mod leader;
pub mod replay;
pub mod time;
pub mod timeout;
pub mod vrf;
