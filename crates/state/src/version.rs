//! Versioned slots for OCC (architecture.md §4.3).
//!
//! Data-structure only: not wired into an execution engine or Block-STM (Tier 10).

use storage::{BatchOp, Store};
use types::collections::Map;
use types::TypesError;

/// Monotonic version number for one slot.
pub type SlotVersion = u64;

/// Latest version plus the value stored at that version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionedRead {
    /// Version observed.
    pub version: SlotVersion,
    /// Value at that version (`None` if the slot was empty).
    pub value: Option<Vec<u8>>,
}

/// In-memory versioned key-value overlay on a [`Store`].
///
/// Layout: `{key}` holds the latest version as 8 BE bytes; `{key}||0x00||ver`
/// holds the value for that version.
pub struct VersionedSlots<S: Store> {
    store: S,
}

impl<S: Store + Clone> Clone for VersionedSlots<S> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
        }
    }
}

impl<S: Store> std::fmt::Debug for VersionedSlots<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VersionedSlots").finish_non_exhaustive()
    }
}

impl<S: Store + Default> Default for VersionedSlots<S> {
    fn default() -> Self {
        Self::new(S::default())
    }
}

const SEP: u8 = 0x00;

fn version_key(key: &[u8], version: SlotVersion) -> Vec<u8> {
    let mut k = key.to_vec();
    k.push(SEP);
    k.extend_from_slice(&version.to_be_bytes());
    k
}

impl<S: Store> VersionedSlots<S> {
    /// Wrap an existing store.
    pub fn new(store: S) -> Self {
        Self { store }
    }

    fn latest(&self, key: &[u8]) -> Result<SlotVersion, TypesError> {
        match self.store.get(key)? {
            None => Ok(0),
            Some(b) if b.len() == 8 => {
                let arr: [u8; 8] = b.as_slice().try_into().map_err(|_| TypesError::Kv("len"))?;
                Ok(u64::from_be_bytes(arr))
            }
            Some(_) => Err(TypesError::Kv("corrupt version header")),
        }
    }

    /// Read a specific version (0 = empty). Contract: `state.versioned_slot.read`.
    pub fn read(&self, key: &[u8], version: SlotVersion) -> Result<VersionedRead, TypesError> {
        if version == 0 {
            return Ok(VersionedRead {
                version: 0,
                value: None,
            });
        }
        let value = self.store.get(&version_key(key, version))?;
        Ok(VersionedRead { version, value })
    }

    /// Write the next version. Contract: `state.versioned_slot.write`.
    pub fn write(&mut self, key: &[u8], value: Vec<u8>) -> Result<SlotVersion, TypesError> {
        let next = self.latest(key)?.saturating_add(1);
        if next == 0 {
            return Err(TypesError::Kv("version overflow"));
        }
        let ops = [
            BatchOp::Put {
                key: key.to_vec(),
                value: next.to_be_bytes().to_vec(),
            },
            BatchOp::Put {
                key: version_key(key, next),
                value,
            },
        ];
        self.store.apply_batch(&ops)?;
        Ok(next)
    }

    /// True iff `observed` is still the latest version (OCC validate).
    /// Contract: `state.versioned_slot.validate`.
    pub fn validate(&self, key: &[u8], observed: SlotVersion) -> Result<bool, TypesError> {
        Ok(self.latest(key)? == observed)
    }

    /// Borrow the inner store (tests).
    pub fn inner(&self) -> &S {
        &self.store
    }
}

/// Two hypothetical readers racing: the second writer invalidates the first.
#[allow(dead_code)]
fn simulate_race<S: Store>(
    slots: &mut VersionedSlots<S>,
    key: &[u8],
) -> Result<(bool, bool), TypesError> {
    let r1 = slots.read(key, slots.latest(key)?)?;
    let r2 = slots.read(key, slots.latest(key)?)?;
    slots.write(key, b"from-r1".to_vec())?;
    let v1 = slots.validate(key, r1.version)?;
    slots.write(key, b"from-r2".to_vec())?;
    let v2 = slots.validate(key, r2.version)?;
    Ok((v1, v2))
}

/// Occupied snapshot of latest versions (for tests).
pub fn latest_map<S: Store>(
    slots: &VersionedSlots<S>,
    keys: &[&[u8]],
) -> Result<Map<Vec<u8>, SlotVersion>, TypesError> {
    let mut m = Map::new();
    for k in keys {
        m.insert(k.to_vec(), slots.latest(k)?);
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use storage::MemoryStore;

    #[test]
    fn read_write_validate_happy() {
        let mut slots = VersionedSlots::new(MemoryStore::new());
        assert_eq!(slots.read(b"k", 0).unwrap().value, None);
        let v = slots.write(b"k", b"one".to_vec()).unwrap();
        assert_eq!(v, 1);
        assert_eq!(
            slots.read(b"k", 1).unwrap().value.as_deref(),
            Some(&b"one"[..])
        );
        assert!(slots.validate(b"k", 1).unwrap());
        assert!(!slots.validate(b"k", 0).unwrap());
    }

    #[test]
    fn two_readers_race() {
        let mut slots = VersionedSlots::new(MemoryStore::new());
        slots.write(b"slot", b"v0".to_vec()).unwrap();
        let observed = slots.read(b"slot", 1).unwrap().version;
        assert!(slots.validate(b"slot", observed).unwrap());
        slots.write(b"slot", b"v1".to_vec()).unwrap();
        assert!(!slots.validate(b"slot", observed).unwrap());
        let (first_ok, second_ok) =
            simulate_race(&mut VersionedSlots::new(MemoryStore::new()), b"x").unwrap();
        assert!(!first_ok);
        assert!(!second_ok);
    }

    #[test]
    fn missing_version_is_none() {
        let slots = VersionedSlots::new(MemoryStore::new());
        assert!(slots.read(b"k", 9).unwrap().value.is_none());
    }
}
