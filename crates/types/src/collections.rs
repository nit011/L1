//! Deterministic map and set types for consensus-critical code.
//!
//! `HashMap` / `HashSet` iteration order is randomized. Hashing a structure that
//! walked a `HashMap` would make `app_hash` diverge across nodes. Later
//! `state`, `execution`, and `consensus` code must import [`Map`] / [`Set`]
//! from here — not `std::collections::HashMap`. See development-plan.md Tier 0
//! (determinism policy) and CI `scripts/check_no_hashmap.sh`.

/// Ordered map. The only map type later consensus/execution/state crates should use.
pub type Map<K, V> = std::collections::BTreeMap<K, V>;

/// Ordered set. The only set type later consensus/execution/state crates should use.
pub type Set<T> = std::collections::BTreeSet<T>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_iteration_is_sorted() {
        let mut m: Map<&str, u8> = Map::new();
        m.insert("b", 1);
        m.insert("a", 2);
        let keys: Vec<_> = m.keys().copied().collect();
        assert_eq!(keys, ["a", "b"]);
    }

    #[test]
    fn set_iteration_is_sorted() {
        let mut s: Set<u8> = Set::new();
        s.insert(3);
        s.insert(1);
        let v: Vec<_> = s.iter().copied().collect();
        assert_eq!(v, [1, 3]);
    }
}
