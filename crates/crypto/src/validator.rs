//! Derive a validator identity from a BLS public key (architecture.md §7).

use crate::sig::bls::{self, pk_to_bytes};
use blst::min_pk::{PublicKey, SecretKey};
use types::{ValidatorId, VotingPower};

/// Pair a BLS public key with voting power. Contract: `validator.from_bls`.
pub fn from_bls(pk: &PublicKey, power: VotingPower) -> (ValidatorId, VotingPower) {
    (ValidatorId::from_bytes(pk_to_bytes(pk)), power)
}

/// Keygen then derive (tests).
pub fn from_new_bls_key(
    power: VotingPower,
) -> Result<(SecretKey, ValidatorId, VotingPower), bls::BlsError> {
    let sk = bls::keygen()?;
    let pk = sk.sk_to_pk();
    let (id, p) = from_bls(&pk, power);
    Ok((sk, id, p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sig::bls::keygen;

    #[test]
    fn id_matches_pk_bytes() {
        let sk = keygen().unwrap();
        let pk = sk.sk_to_pk();
        let (id, power) = from_bls(&pk, VotingPower(10));
        assert_eq!(id.as_bytes(), &pk_to_bytes(&pk));
        assert_eq!(power, VotingPower(10));
    }

    #[test]
    fn different_keys_differ() {
        let a = from_bls(&keygen().unwrap().sk_to_pk(), VotingPower(1));
        let b = from_bls(&keygen().unwrap().sk_to_pk(), VotingPower(1));
        assert_ne!(a.0, b.0);
    }
}
