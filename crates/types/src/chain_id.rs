//! Replay-protection chain identifier.

use serde::{Deserialize, Serialize};

/// Network / chain id. Contract: `types.chain_id`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChainId(pub u64);

impl ChainId {
    /// Construct a chain id.
    pub const fn new(id: u64) -> Self {
        Self(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_ids_are_not_equal() {
        assert_ne!(ChainId::new(1), ChainId::new(2));
        assert_eq!(ChainId::new(7), ChainId(7));
    }
}
