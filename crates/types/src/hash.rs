//! 32-byte content digest. Does not hash; see `crypto::hash::blake3` for BLAKE3.
//!
//! TODO(tier_1): wire to crypto once types+crypto boundary is confirmed.

use crate::spec::HASH_SIZE;
use serde::{Deserialize, Serialize};

/// Fixed-size hash (BLAKE3-256 width). Contract: `types.hash`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Hash(pub [u8; HASH_SIZE]);

impl Hash {
    /// All-zero digest.
    pub const ZERO: Self = Self([0u8; HASH_SIZE]);

    /// Wrap raw bytes.
    pub fn from_bytes(bytes: [u8; HASH_SIZE]) -> Self {
        Self(bytes)
    }

    /// Borrow the raw bytes.
    pub fn as_bytes(&self) -> &[u8; HASH_SIZE] {
        &self.0
    }
}

impl std::fmt::Debug for Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Hash({})", hex_encode(&self.0))
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl AsRef<[u8]> for Hash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl TryFrom<&[u8]> for Hash {
    type Error = crate::error::TypesError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let arr: [u8; HASH_SIZE] =
            value
                .try_into()
                .map_err(|_| crate::error::TypesError::BadLength {
                    expected: HASH_SIZE,
                    actual: value.len(),
                })?;
        Ok(Self(arr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_bytes_and_try_from() {
        let raw = [7u8; HASH_SIZE];
        let h = Hash::from_bytes(raw);
        assert_eq!(h.as_bytes(), &raw);
        assert_eq!(Hash::try_from(&raw[..]).unwrap(), h);
    }

    #[test]
    fn try_from_rejects_wrong_len() {
        assert!(Hash::try_from(&[1u8, 2][..]).is_err());
    }
}
