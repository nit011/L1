//! Data-availability: Reed-Solomon (Tier 0) plus chunking, DA root, and DAS
//! (Tier 12, architecture.md §6).

pub mod chunk;
pub mod das;
pub mod root;
pub mod rs;

pub use chunk::{reconstruct, split, ChunkError, DaShard, DATA_SHARDS, PARITY_SHARDS};
pub use das::{fail_closed, sample, Availability, ChunkFetch, MemoryChunks, SampleReport};
pub use root::{commit, verify_chunk, DaRoot, ProvenChunk, RootError};
pub use rs::{decode, encode, RsError};
