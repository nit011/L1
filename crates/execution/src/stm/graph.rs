//! Conflict graph from speculative RW sets (architecture.md §3.3).
//!
//! Edge `A → B` iff `txA.writes ∩ txB.(reads ∪ writes) ≠ ∅`.

use crate::stm::rwset::SpecTx;
use types::collections::{Map, Set};

/// Undirected conflict adjacency (keys sorted). Contract: `stm.conflict_graph`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConflictGraph {
    /// `i →` neighbors `j` (both directions stored).
    pub adj: Map<usize, Set<usize>>,
    /// Directed edges `(writer, reader_or_writer)` matching the §3.3 rule.
    pub directed: Set<(usize, usize)>,
}

impl ConflictGraph {
    /// Isolated vertices `0..n`.
    pub fn isolated(n: usize) -> Self {
        let mut adj = Map::new();
        for i in 0..n {
            adj.insert(i, Set::new());
        }
        Self {
            adj,
            directed: Set::new(),
        }
    }
}

fn union_rw(s: &SpecTx) -> Set<Vec<u8>> {
    s.reads.union(&s.writes).cloned().collect()
}

/// Build the graph. Contract: `stm.conflict_graph`.
pub fn conflict_graph(specs: &[SpecTx]) -> ConflictGraph {
    let mut g = ConflictGraph {
        adj: Map::new(),
        directed: Set::new(),
    };
    for s in specs {
        g.adj.entry(s.index).or_default();
    }
    for a in specs {
        for b in specs {
            if a.index == b.index {
                continue;
            }
            let rw_b = union_rw(b);
            let hit: Set<_> = a.writes.intersection(&rw_b).cloned().collect();
            if !hit.is_empty() {
                g.directed.insert((a.index, b.index));
                g.adj.get_mut(&a.index).unwrap().insert(b.index);
                g.adj.get_mut(&b.index).unwrap().insert(a.index);
            }
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receipt::Receipt;
    use crate::seq::World;
    use crate::stm::rwset::SpecTx;

    fn spec(index: usize, reads: &[&[u8]], writes: &[&[u8]]) -> SpecTx {
        let mut r = Set::new();
        let mut w = Set::new();
        for k in reads {
            r.insert(k.to_vec());
        }
        for k in writes {
            w.insert(k.to_vec());
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
    fn hand_constructed_edges_match_exactly() {
        // A writes x; B reads x → edge A→B
        // B writes y; C writes y → edges B→C and C→B
        // D isolated
        let specs = vec![
            spec(0, &[b"x"], &[b"x"]),
            spec(1, &[b"x"], &[b"y"]),
            spec(2, &[b"y"], &[b"y"]),
            spec(3, &[b"z"], &[b"z"]),
        ];
        let g = conflict_graph(&specs);
        assert!(g.directed.contains(&(0, 1)));
        assert!(g.directed.contains(&(1, 2)));
        assert!(g.directed.contains(&(2, 1)));
        assert!(!g.directed.contains(&(0, 3)));
        assert!(g.adj.get(&3).unwrap().is_empty());
        assert!(g.adj.get(&0).unwrap().contains(&1));
    }

    #[test]
    fn read_read_is_not_an_edge() {
        let specs = vec![spec(0, &[b"k"], &[]), spec(1, &[b"k"], &[])];
        let g = conflict_graph(&specs);
        assert!(g.directed.is_empty());
    }
}
