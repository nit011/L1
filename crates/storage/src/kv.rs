//! Key-value store trait (architecture.md §4). Errors use Tier 0 `error.core`.

use types::TypesError;

/// One key/value pair from a prefix scan.
pub type KvEntry = (Vec<u8>, Vec<u8>);

/// Atomic batch operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatchOp {
    /// Insert or replace.
    Put { key: Vec<u8>, value: Vec<u8> },
    /// Remove a key if present.
    Delete { key: Vec<u8> },
}

/// Byte-oriented store. Contract: `kv.trait`.
pub trait Store {
    /// Lookup.
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, TypesError>;
    /// Insert or replace.
    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), TypesError>;
    /// Remove.
    fn delete(&mut self, key: &[u8]) -> Result<(), TypesError>;
    /// Keys starting with `prefix`, sorted (BTree order).
    fn prefix(&self, prefix: &[u8]) -> Result<Vec<KvEntry>, TypesError>;

    /// Apply all ops or restore prior values. Contract: `kv.batch`.
    fn apply_batch(&mut self, ops: &[BatchOp]) -> Result<(), TypesError> {
        apply_batch(self, ops)
    }
}

/// Snapshot keys, apply ops, roll back on the first error (including a
/// simulated mid-batch failure via [`BatchOp`] plus [`failing_put`]).
pub fn apply_batch<S: Store + ?Sized>(store: &mut S, ops: &[BatchOp]) -> Result<(), TypesError> {
    let mut snap: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();
    for op in ops {
        let key = match op {
            BatchOp::Put { key, .. } | BatchOp::Delete { key } => key.clone(),
        };
        if !snap.iter().any(|(k, _)| k == &key) {
            snap.push((key.clone(), store.get(&key)?));
        }
    }
    for op in ops {
        let r = match op {
            BatchOp::Put { key, value } => store.put(key, value),
            BatchOp::Delete { key } => store.delete(key),
        };
        if let Err(e) = r {
            restore(store, &snap);
            return Err(e);
        }
    }
    Ok(())
}

fn restore<S: Store + ?Sized>(store: &mut S, snap: &[(Vec<u8>, Option<Vec<u8>>)]) {
    for (k, v) in snap {
        match v {
            Some(val) => {
                let _ = store.put(k, val);
            }
            None => {
                let _ = store.delete(k);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryStore;

    struct FailAfter {
        inner: MemoryStore,
        remaining: usize,
    }

    impl Store for FailAfter {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, TypesError> {
            self.inner.get(key)
        }
        fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), TypesError> {
            if self.remaining == 0 {
                return Err(TypesError::Kv("simulated mid-batch failure"));
            }
            self.remaining -= 1;
            self.inner.put(key, value)
        }
        fn delete(&mut self, key: &[u8]) -> Result<(), TypesError> {
            self.inner.delete(key)
        }
        fn prefix(&self, prefix: &[u8]) -> Result<Vec<KvEntry>, TypesError> {
            self.inner.prefix(prefix)
        }
    }

    #[test]
    fn batch_all_or_nothing_on_simulated_failure() {
        let mut s = FailAfter {
            inner: MemoryStore::new(),
            remaining: 1,
        };
        s.put(b"keep", b"old").unwrap();
        let err = s
            .apply_batch(&[
                BatchOp::Put {
                    key: b"keep".to_vec(),
                    value: b"new".to_vec(),
                },
                BatchOp::Put {
                    key: b"other".to_vec(),
                    value: b"x".to_vec(),
                },
            ])
            .unwrap_err();
        assert!(matches!(err, TypesError::Kv(_)));
        assert_eq!(s.get(b"keep").unwrap().as_deref(), Some(&b"old"[..]));
        assert!(s.get(b"other").unwrap().is_none());
    }

    #[test]
    fn batch_success() {
        let mut s = MemoryStore::new();
        s.apply_batch(&[
            BatchOp::Put {
                key: b"a".to_vec(),
                value: b"1".to_vec(),
            },
            BatchOp::Delete {
                key: b"missing".to_vec(),
            },
        ])
        .unwrap();
        assert_eq!(s.get(b"a").unwrap().as_deref(), Some(&b"1"[..]));
    }
}
