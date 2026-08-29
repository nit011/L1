//! In-process [`Store`] backed by a sorted map (Tier 0 determinism policy).

use crate::kv::{BatchOp, Store};
use types::collections::Map;
use types::TypesError;

/// RAM store for tests. Contract: `kv.memory`.
#[derive(Clone, Debug, Default)]
pub struct MemoryStore {
    map: Map<Vec<u8>, Vec<u8>>,
}

impl MemoryStore {
    /// Empty store.
    pub fn new() -> Self {
        Self { map: Map::new() }
    }
}

impl Store for MemoryStore {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, TypesError> {
        Ok(self.map.get(key).cloned())
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), TypesError> {
        self.map.insert(key.to_vec(), value.to_vec());
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), TypesError> {
        self.map.remove(key);
        Ok(())
    }

    fn prefix(&self, prefix: &[u8]) -> Result<Vec<crate::kv::KvEntry>, TypesError> {
        Ok(self
            .map
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }
}

impl MemoryStore {
    /// Expose batch on the concrete type.
    pub fn apply_batch(&mut self, ops: &[BatchOp]) -> Result<(), TypesError> {
        Store::apply_batch(self, ops)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_put_delete_prefix() {
        let mut s = MemoryStore::new();
        assert!(s.get(b"k").unwrap().is_none());
        s.put(b"aa", b"1").unwrap();
        s.put(b"ab", b"2").unwrap();
        s.put(b"b", b"3").unwrap();
        assert_eq!(s.prefix(b"a").unwrap().len(), 2);
        s.delete(b"aa").unwrap();
        assert!(s.get(b"aa").unwrap().is_none());
        assert_eq!(s.get(b"ab").unwrap().as_deref(), Some(&b"2"[..]));
    }
}
