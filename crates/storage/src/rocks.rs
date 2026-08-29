//! RocksDB-backed [`Store`].
//!
//! Compiled only with `--features rocksdb`. Without that feature this module
//! still exists so `kv.rocksdb` has a `rust_file`, but [`RocksStore::open`]
//! returns [`TypesError::Kv`] explaining that the native library is optional.
//! All tests that do not specifically cover persistence use [`crate::MemoryStore`].

use crate::kv::Store;
use types::TypesError;

/// Persistent store. Contract: `kv.rocksdb`.
pub struct RocksStore {
    #[cfg(feature = "rocksdb")]
    db: rocksdb::DB,
    #[cfg(not(feature = "rocksdb"))]
    _priv: (),
}

impl RocksStore {
    /// Open (or fail if the `rocksdb` feature is off).
    pub fn open(path: &std::path::Path) -> Result<Self, TypesError> {
        #[cfg(feature = "rocksdb")]
        {
            let db = rocksdb::DB::open_default(path)
                .map_err(|_| TypesError::Kv("rocksdb open failed"))?;
            return Ok(Self { db });
        }
        #[cfg(not(feature = "rocksdb"))]
        {
            let _ = path;
            Err(TypesError::Kv(
                "rocksdb feature disabled (native lib optional); use kv.memory",
            ))
        }
    }
}

impl Store for RocksStore {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, TypesError> {
        #[cfg(feature = "rocksdb")]
        {
            return self
                .db
                .get(key)
                .map_err(|_| TypesError::Kv("rocksdb get failed"));
        }
        #[cfg(not(feature = "rocksdb"))]
        {
            let _ = key;
            Err(TypesError::Kv("rocksdb feature disabled"))
        }
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), TypesError> {
        #[cfg(feature = "rocksdb")]
        {
            return self
                .db
                .put(key, value)
                .map_err(|_| TypesError::Kv("rocksdb put failed"));
        }
        #[cfg(not(feature = "rocksdb"))]
        {
            let _ = (key, value);
            Err(TypesError::Kv("rocksdb feature disabled"))
        }
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), TypesError> {
        #[cfg(feature = "rocksdb")]
        {
            return self
                .db
                .delete(key)
                .map_err(|_| TypesError::Kv("rocksdb delete failed"));
        }
        #[cfg(not(feature = "rocksdb"))]
        {
            let _ = key;
            Err(TypesError::Kv("rocksdb feature disabled"))
        }
    }

    fn prefix(&self, prefix: &[u8]) -> Result<Vec<crate::kv::KvEntry>, TypesError> {
        #[cfg(feature = "rocksdb")]
        {
            use rocksdb::{Direction, IteratorMode};
            let mut out = Vec::new();
            let iter = self
                .db
                .iterator(IteratorMode::From(prefix, Direction::Forward));
            for item in iter {
                let (k, v) = item.map_err(|_| TypesError::Kv("rocksdb iter failed"))?;
                if !k.starts_with(prefix) {
                    break;
                }
                out.push((k.to_vec(), v.to_vec()));
            }
            return Ok(out);
        }
        #[cfg(not(feature = "rocksdb"))]
        {
            let _ = prefix;
            Err(TypesError::Kv("rocksdb feature disabled"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_without_feature_fails_cleanly() {
        #[cfg(not(feature = "rocksdb"))]
        {
            assert!(RocksStore::open(std::path::Path::new("/tmp/l1-rocks-test")).is_err());
        }
        #[cfg(feature = "rocksdb")]
        {
            let dir = std::env::temp_dir().join(format!("l1-rocks-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let mut s = RocksStore::open(&dir).unwrap();
            s.put(b"k", b"v").unwrap();
            assert_eq!(s.get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
            s.delete(b"k").unwrap();
            assert!(s.get(b"k").unwrap().is_none());
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
