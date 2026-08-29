//! Consensus time, VRF, and in-process BFT (architecture.md §2).
//!
//! Pure crate: no libp2p, no RocksDB feature, no `network` crate.

pub mod checkpoint;
pub mod evidence;
pub mod leader;
pub mod propose;
pub mod qc;
pub mod replay;
pub mod safety;
pub mod state;
pub mod steps;
pub mod time;
pub mod timeout;
pub mod vote;
pub mod vrf;
pub mod wal;
