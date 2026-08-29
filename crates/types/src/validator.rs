//! Validator identity and voting power (architecture.md §2, §7).

use crate::error::TypesError;
use crate::spec::VALIDATOR_ID_SIZE;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Compressed BLS12-381 G1 public key (48 bytes). Contract: `types.validator_id`.
///
/// TODO(tier_1): wire to crypto once types+crypto boundary is confirmed.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ValidatorId(pub [u8; VALIDATOR_ID_SIZE]);

impl Serialize for ValidatorId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for ValidatorId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let v: Vec<u8> = Deserialize::deserialize(deserializer)?;
        let arr: [u8; VALIDATOR_ID_SIZE] = v
            .try_into()
            .map_err(|_| serde::de::Error::custom("validator id must be 48 bytes"))?;
        Ok(Self(arr))
    }
}

impl ValidatorId {
    /// All-zero (invalid) id for tests.
    pub const ZERO: Self = Self([0u8; VALIDATOR_ID_SIZE]);

    /// Wrap a compressed G1 key.
    pub fn from_bytes(bytes: [u8; VALIDATOR_ID_SIZE]) -> Self {
        Self(bytes)
    }

    /// Borrow bytes.
    pub fn as_bytes(&self) -> &[u8; VALIDATOR_ID_SIZE] {
        &self.0
    }
}

impl std::fmt::Debug for ValidatorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ValidatorId({:02x}..)", self.0[0])
    }
}

impl TryFrom<&[u8]> for ValidatorId {
    type Error = TypesError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let arr: [u8; VALIDATOR_ID_SIZE] = value.try_into().map_err(|_| TypesError::BadLength {
            expected: VALIDATOR_ID_SIZE,
            actual: value.len(),
        })?;
        Ok(Self(arr))
    }
}

/// Stake-weighted voting power. Contract: `types.voting_power`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct VotingPower(pub u64);

impl VotingPower {
    /// Zero power (jailed / tombstoned).
    pub const ZERO: Self = Self(0);

    /// Saturating add for tallying votes.
    pub fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// Two-thirds of `total` using integer math: `2 * self > total` is not used;
    /// quorum is `self * 3 >= 2 * total` at the consensus layer. Helper for tests:
    /// return whether `self` is strictly more than `2/3` of `total`.
    pub fn exceeds_two_thirds(self, total: Self) -> bool {
        self.0.saturating_mul(3) > total.0.saturating_mul(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validator_id_length() {
        let id = ValidatorId::from_bytes([1u8; VALIDATOR_ID_SIZE]);
        assert!(ValidatorId::try_from(&id.as_bytes()[..]).is_ok());
        assert!(ValidatorId::try_from(&[0u8; 4][..]).is_err());
    }

    #[test]
    fn voting_power_quorum() {
        let total = VotingPower(30);
        assert!(!VotingPower(20).exceeds_two_thirds(total));
        assert!(VotingPower(21).exceeds_two_thirds(total));
        assert_eq!(
            VotingPower(1).saturating_add(VotingPower(2)),
            VotingPower(3)
        );
    }
}
