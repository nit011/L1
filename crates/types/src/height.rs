//! Block height.

use serde::{Deserialize, Serialize};

/// Chain height. Contract: `types.height`. Genesis is height 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Height(pub u64);

impl Height {
    /// Genesis height.
    pub const GENESIS: Self = Self(0);

    /// Successor height.
    pub fn saturating_next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_from_genesis() {
        assert_eq!(Height::GENESIS.saturating_next(), Height(1));
    }

    #[test]
    fn saturates_at_max() {
        assert_eq!(Height(u64::MAX).saturating_next(), Height(u64::MAX));
    }
}
