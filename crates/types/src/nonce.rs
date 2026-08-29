//! Account nonce (replay protection).

use serde::{Deserialize, Serialize};

/// Monotonic per-account nonce. Contract: `types.nonce`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Nonce(pub u64);

impl Nonce {
    /// Zero nonce (first transaction).
    pub const ZERO: Self = Self(0);

    /// Next nonce, wrapping documented as an error at the execution layer.
    pub fn checked_add(self, n: u64) -> Option<Self> {
        self.0.checked_add(n).map(Self)
    }

    /// True if `self` is exactly `expected` (strict equality for tx admission).
    pub fn matches(self, expected: Self) -> bool {
        self == expected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn increment() {
        let n = Nonce::ZERO.checked_add(1).unwrap();
        assert_eq!(n, Nonce(1));
        assert!(n.matches(Nonce(1)));
        assert!(!n.matches(Nonce(2)));
    }

    #[test]
    fn overflow() {
        assert!(Nonce(u64::MAX).checked_add(1).is_none());
    }
}
