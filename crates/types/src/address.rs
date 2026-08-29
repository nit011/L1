//! Account address newtype (development-plan.md §1: 32-byte addresses).

use crate::error::TypesError;
use crate::spec::ADDRESS_SIZE;
use serde::{Deserialize, Serialize};

/// 32-byte account address. Contract: `types.address`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Address(pub [u8; ADDRESS_SIZE]);

impl Address {
    /// All-zero address (not a valid spend key; used in tests).
    pub const ZERO: Self = Self([0u8; ADDRESS_SIZE]);

    /// Wrap raw bytes.
    pub fn from_bytes(bytes: [u8; ADDRESS_SIZE]) -> Self {
        Self(bytes)
    }

    /// Borrow the raw bytes.
    pub fn as_bytes(&self) -> &[u8; ADDRESS_SIZE] {
        &self.0
    }
}

impl std::fmt::Debug for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Address({:02x}..)", self.0[0])
    }
}

impl TryFrom<&[u8]> for Address {
    type Error = TypesError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let arr: [u8; ADDRESS_SIZE] = value.try_into().map_err(|_| TypesError::BadLength {
            expected: ADDRESS_SIZE,
            actual: value.len(),
        })?;
        Ok(Self(arr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_bytes() {
        let a = Address::from_bytes([9u8; ADDRESS_SIZE]);
        assert_eq!(Address::try_from(&a.as_bytes()[..]).unwrap(), a);
    }

    #[test]
    fn rejects_short_slice() {
        assert!(Address::try_from(&[0u8; 8][..]).is_err());
    }
}
