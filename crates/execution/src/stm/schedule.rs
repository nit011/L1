//! Schedule non-conflicting transactions onto OS threads (architecture.md §3.3).
//!
//! Waves are greedy independent sets in block order. Within a wave, work runs
//! on real [`std::thread`]s. **Final receipts stay in block order** regardless
//! of completion order.

use crate::stm::graph::ConflictGraph;
use std::thread::{self, ThreadId};
use types::collections::Set;

/// Waves of pairwise non-adjacent tx indices (sorted inside each wave).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    /// Parallel waves; concatenate in order for a valid serialisation of waves.
    pub waves: Vec<Vec<usize>>,
}

/// Greedy coloring / independent-set waves. Contract: `stm.schedule`.
pub fn schedule(graph: &ConflictGraph) -> Schedule {
    let n = graph.adj.len();
    let mut remaining: Set<usize> = (0..n).collect();
    let mut waves = Vec::new();
    while !remaining.is_empty() {
        let mut wave = Vec::new();
        let mut blocked = Set::new();
        let cand: Vec<usize> = remaining.iter().copied().collect();
        for i in cand {
            if blocked.contains(&i) {
                continue;
            }
            wave.push(i);
            if let Some(nbrs) = graph.adj.get(&i) {
                for n in nbrs {
                    blocked.insert(*n);
                }
            }
            remaining.remove(&i);
        }
        wave.sort_unstable();
        waves.push(wave);
    }
    Schedule { waves }
}

/// Run `work(index)` on a dedicated OS thread per member of each wave.
/// Returns worker thread ids (may contain duplicates across waves).
pub fn run_waves<F>(sched: &Schedule, work: F) -> Vec<ThreadId>
where
    F: Fn(usize) + Sync,
{
    let mut ids = Vec::new();
    for wave in &sched.waves {
        if wave.is_empty() {
            continue;
        }
        thread::scope(|scope| {
            let mut joins = Vec::new();
            let work = &work;
            for &i in wave {
                joins.push(scope.spawn(move || {
                    work(i);
                    thread::current().id()
                }));
            }
            for j in joins {
                ids.push(j.join().unwrap());
            }
        });
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stm::graph::{conflict_graph, ConflictGraph};
    use crate::stm::rwset::SpecTx;
    use crate::{receipt::Receipt, seq::World};
    use types::collections::Map;

    fn spec(index: usize, writes: &[&[u8]]) -> SpecTx {
        let mut w = Set::new();
        let mut r = Set::new();
        for k in writes {
            w.insert(k.to_vec());
            r.insert(k.to_vec());
        }
        SpecTx {
            index,
            reads: r,
            writes: w,
            observed: Map::new(),
            spec_world: World::default(),
            receipt: Receipt {
                success: true,
                gas_used: 0,
                events: vec![],
                reason: None,
            },
        }
    }

    #[test]
    fn independent_txs_share_a_wave() {
        let specs = vec![spec(0, &[b"a"]), spec(1, &[b"b"]), spec(2, &[b"c"])];
        let g = conflict_graph(&specs);
        let s = schedule(&g);
        assert_eq!(s.waves.len(), 1);
        assert_eq!(s.waves[0], vec![0, 1, 2]);
    }

    #[test]
    fn chain_of_conflicts_is_serialized() {
        let specs = vec![spec(0, &[b"x"]), spec(1, &[b"x"]), spec(2, &[b"x"])];
        let g = conflict_graph(&specs);
        let s = schedule(&g);
        assert!(s.waves.len() >= 3);
        let flat: Vec<_> = s.waves.iter().flatten().copied().collect();
        let mut sorted = flat.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![0, 1, 2]);
    }

    #[test]
    fn run_waves_uses_os_threads() {
        let g = ConflictGraph::isolated(4);
        let s = schedule(&g);
        let ids = run_waves(&s, |_| {});
        let mut uniq = ids.clone();
        uniq.sort_by_key(|id| format!("{id:?}"));
        uniq.dedup();
        assert!(
            uniq.len() > 1,
            "low-contention wave must spawn multiple OS threads, got {ids:?}"
        );
    }
}
