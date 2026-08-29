//! Durable and in-memory key-value stores.

pub mod blocks;
pub mod codec;
pub mod index;
pub mod kv;
pub mod memory;
pub mod replay;
pub mod rocks;
pub mod wal;

pub use kv::{apply_batch, BatchOp, Store};
pub use memory::MemoryStore;
pub use rocks::RocksStore;
