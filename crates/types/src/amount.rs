//! Token amount. Uses checked arithmetic so execution cannot wrap silently.

use serde::{Deserialize, Serialize};

/// Native token amount (128-bit). Contract: `types.amount`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Amount(pub u128);

impl Amount {
    /// Zero coins.
    pub const ZERO: Self = Self(0);

    /// Construct from a raw integer.
    pub const fn new(v: u128) -> Self {
        Self(v)
    }

    /// Checked addition.
    pub fn checked_add(self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Self)
    }

    /// Checked subtraction.
    pub fn checked_sub(self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_sub_round_trip() {
        let a = Amount::new(10);
        let b = Amount::new(3);
        assert_eq!(a.checked_sub(b).unwrap(), Amount::new(7));
        assert_eq!(Amount::new(7).checked_add(b).unwrap(), a);
    }

    #[test]
    fn overflow_and_underflow() {
        assert!(Amount::new(u128::MAX).checked_add(Amount::new(1)).is_none());
        assert!(Amount::ZERO.checked_sub(Amount::new(1)).is_none());
    }
}
