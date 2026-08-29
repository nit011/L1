//! Consensus round within a height (architecture.md §2.2).

use serde::{Deserialize, Serialize};

/// Tendermint-style round number. Contract: `types.round`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Round(pub u32);

impl Round {
    /// First round of a height.
    pub const ZERO: Self = Self(0);

    /// Next round after a timeout / nil vote.
    pub fn saturating_next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_round() {
        assert_eq!(Round::ZERO.saturating_next(), Round(1));
    }

    #[test]
    fn saturates() {
        assert_eq!(Round(u32::MAX).saturating_next(), Round(u32::MAX));
    }
}
