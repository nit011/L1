//! Process event loop: mempool ↔ exec ↔ BFT ↔ store ↔ gossip (Tier 7).

pub mod config;
pub mod sync;
pub mod tracing;
pub mod wire;
pub mod ws;
