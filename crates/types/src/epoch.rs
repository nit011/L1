//! Validator-set epoch (architecture.md §2.5).

use serde::{Deserialize, Serialize};

/// Epoch index. Contract: `types.epoch`. Length is PLACEHOLDER in [`crate::spec`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Epoch(pub u64);

impl Epoch {
    /// First epoch.
    pub const ZERO: Self = Self(0);

    /// Next epoch.
    pub fn saturating_next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_epoch() {
        assert_eq!(Epoch::ZERO.saturating_next(), Epoch(1));
    }

    #[test]
    fn saturates() {
        assert_eq!(Epoch(u64::MAX).saturating_next(), Epoch(u64::MAX));
    }
}
