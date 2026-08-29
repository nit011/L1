//! Durable and in-memory key-value stores.

pub mod kv;
pub mod memory;
pub mod rocks;

pub use kv::{apply_batch, BatchOp, Store};
pub use memory::MemoryStore;
pub use rocks::RocksStore;
