//! Data-availability sampling for light nodes (architecture.md §6).
//!
//! Light nodes randomly request a handful of chunks; if enough independent
//! samples succeed against `da.root`, the full block is available with high
//! probability **without** downloading all of it. This module never participates
//! in `cons.commit` / `node.wire.commit` (forbidden edge: DAS must not gate
//! first finality).
//!
//! Chunk bytes are obtained through a [`ChunkFetch`] implementation. Production
//! wiring uses `gossip.da_chunks` (`/l1/da-chunks/1`); unit tests use an
//! in-memory map so `da` does not depend on `network` (that crate depends on
//! `da`).
//!
//! # Sample count and confidence
//!
//! [`SAMPLE_COUNT`] is **3** distinct indices out of `k+m = 6` (half the
//! codeword). Sampling is without replacement, seeded by `DaRoot::merkle`.
//! If an adversary withholds 3 of 6 shards (reconstruction impossible), the
//! chance that all 3 light-client samples land in the available half is
//! `C(3,3)/C(6,3) = 1/20`. [`fail_closed`] still requires 3 successful
//! Merkle checks — a timeout, withhold, or tamper yields **not available**,
//! never a default-to-available outcome (development-plan.md:
//! "withhold data -> light samples fail").

use crate::chunk::{DATA_SHARDS, PARITY_SHARDS};
use crate::root::{verify_chunk, DaRoot, ProvenChunk};

/// Distinct chunks a light node must verify. See module docs for confidence.
pub const SAMPLE_COUNT: usize = 3;

/// Gossip topic name owned with `gossip.da_chunks` (stable string).
pub const TOPIC_DA_CHUNKS: &str = "/l1/da-chunks/1";

/// Fetch a chunk by RS index (gossip or test harness).
pub trait ChunkFetch {
    /// `None` if the peer withheld or the request timed out.
    fn fetch(&self, index: u16) -> Option<ProvenChunk>;
}

/// Result of [`sample`] before fail-closed interpretation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleReport {
    /// Successful Merkle checks.
    pub successes: usize,
    /// Indices queried.
    pub queried: Vec<u16>,
    /// Required successes ([`SAMPLE_COUNT`]).
    pub required: usize,
}

/// Fail-closed availability. Contract: `das.fail_closed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// Enough honest samples matched `da.root`.
    Available,
    /// Withheld, tampered, or too few replies — never a timeout-as-success.
    NotAvailable,
}

fn shuffle_indices(n: usize, seed: &[u8; 32]) -> Vec<u16> {
    let mut idx: Vec<u16> = (0..n as u16).collect();
    let mut s0 = u64::from_le_bytes(seed[0..8].try_into().unwrap());
    let mut s1 = u64::from_le_bytes(seed[8..16].try_into().unwrap());
    if s0 == 0 && s1 == 0 {
        s0 = 1;
    }
    for i in (1..idx.len()).rev() {
        s0 = s0.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(s1);
        s1 = s1.rotate_left(7) ^ s0;
        let j = (s0 as usize) % (i + 1);
        idx.swap(i, j);
    }
    idx
}

/// Indices a light node will request (deterministic given `root.merkle`).
pub fn sample_indices(root: &DaRoot) -> Vec<u16> {
    let n = root.data_shards + root.parity_shards;
    let mut idx = shuffle_indices(n, root.merkle.as_bytes());
    let take = SAMPLE_COUNT.min(n);
    idx.truncate(take);
    idx.sort_unstable();
    idx
}

/// Request a random subset via [`ChunkFetch`] and check each against `da.root`.
///
/// Callers pass a fetch backed by `gossip.da_chunks`. Contract: `das.sample`.
pub fn sample<F: ChunkFetch>(root: &DaRoot, gossip: &F) -> SampleReport {
    let _ = TOPIC_DA_CHUNKS;
    let queried = sample_indices(root);
    let mut successes = 0usize;
    for i in &queried {
        if let Some(chunk) = gossip.fetch(*i) {
            if chunk.shard.index == *i && verify_chunk(root, &chunk) {
                successes += 1;
            }
        }
    }
    SampleReport {
        successes,
        queried,
        required: SAMPLE_COUNT,
    }
}

/// Map a sample report to availability. Incomplete sampling is **not** available.
/// Contract: `das.fail_closed`.
pub fn fail_closed(report: &SampleReport) -> Availability {
    if report.successes >= report.required {
        Availability::Available
    } else {
        Availability::NotAvailable
    }
}

/// In-memory chunk map (tests; also used by node tests as a gossip stand-in).
#[derive(Clone, Default)]
pub struct MemoryChunks {
    inner: Vec<Option<ProvenChunk>>,
}

impl MemoryChunks {
    /// Store proven chunks at their RS indices.
    pub fn from_proven(chunks: Vec<ProvenChunk>) -> Self {
        let n = DATA_SHARDS + PARITY_SHARDS;
        let mut inner = vec![None; n];
        for c in chunks {
            let i = c.shard.index as usize;
            if i < n {
                inner[i] = Some(c);
            }
        }
        Self { inner }
    }

    /// Drop a set of indices (withhold).
    pub fn withhold(&mut self, indices: &[u16]) {
        for i in indices {
            if let Some(slot) = self.inner.get_mut(*i as usize) {
                *slot = None;
            }
        }
    }

    /// Corrupt payload at `index` if present (bad proof / tamper).
    pub fn tamper(&mut self, index: u16) {
        if let Some(Some(c)) = self.inner.get_mut(index as usize) {
            if !c.shard.payload.is_empty() {
                c.shard.payload[0] ^= 0x5a;
            }
        }
    }
}

impl ChunkFetch for MemoryChunks {
    fn fetch(&self, index: u16) -> Option<ProvenChunk> {
        self.inner.get(index as usize).cloned().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::tests::test_block;
    use crate::root::commit;

    #[test]
    fn sample_happy_path_available() {
        let block = test_block(2);
        let (root, proven) = commit(&block).unwrap();
        let store = MemoryChunks::from_proven(proven);
        let report = sample(&root, &store);
        assert_eq!(report.queried.len(), SAMPLE_COUNT);
        assert_eq!(fail_closed(&report), Availability::Available);
    }

    #[test]
    fn withhold_data_light_samples_fail_closed() {
        let block = test_block(3);
        let (root, proven) = commit(&block).unwrap();
        let queried = sample_indices(&root);
        let mut store = MemoryChunks::from_proven(proven);
        store.withhold(&queried);
        let report = sample(&root, &store);
        assert_eq!(report.successes, 0);
        assert_eq!(fail_closed(&report), Availability::NotAvailable);
        assert_ne!(fail_closed(&report), Availability::Available);
    }

    #[test]
    fn insufficient_samples_fail_closed() {
        let block = test_block(1);
        let (root, proven) = commit(&block).unwrap();
        let queried = sample_indices(&root);
        let mut store = MemoryChunks::from_proven(proven);
        store.withhold(&queried[..queried.len().saturating_sub(1)]);
        let report = sample(&root, &store);
        assert!(report.successes < SAMPLE_COUNT);
        assert_eq!(fail_closed(&report), Availability::NotAvailable);
    }

    #[test]
    fn light_client_rejects_bad_proof() {
        let block = test_block(2);
        let (root, proven) = commit(&block).unwrap();
        let queried = sample_indices(&root);
        let mut store = MemoryChunks::from_proven(proven);
        for i in &queried {
            store.tamper(*i);
        }
        let report = sample(&root, &store);
        assert_eq!(report.successes, 0);
        assert_eq!(fail_closed(&report), Availability::NotAvailable);
    }
}
