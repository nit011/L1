//! VRF seed and stake-weighted leader lottery (architecture.md §2.3, §2.4).
//!
//! Seed is `blake3(domain.tag.apply(Vrf, last_finalized_block_hash || epoch))`.
//! It must not include timestamps or local randomness (grinding resistance).

use crypto::hash::blake3::hash_to_array;
use crypto::vrf::{self, Proof as VrfProof, VrfError};
use crypto::{apply_domain, DomainTag};
use types::collections::Map;
use types::{Epoch, ValidatorId, VotingPower};

/// 32-byte leader-election seed.
pub type VrfSeed = [u8; 32];

/// Derive the epoch seed. Contract: `vrf.seed.derive`.
///
/// Frozen formula (development-plan.md): `H(last_finalized_block_hash || epoch)`.
pub fn derive_seed(last_finalized_block_hash: &[u8; 32], epoch: Epoch) -> VrfSeed {
    let mut msg = Vec::with_capacity(32 + 8);
    msg.extend_from_slice(last_finalized_block_hash);
    msg.extend_from_slice(&epoch.0.to_be_bytes());
    hash_to_array(&apply_domain(DomainTag::Vrf, &msg))
}

fn leader_alpha(seed: &VrfSeed, validator_id: &ValidatorId) -> Vec<u8> {
    let mut alpha = Vec::with_capacity(32 + 48);
    alpha.extend_from_slice(seed);
    alpha.extend_from_slice(validator_id.as_bytes());
    alpha
}

/// Prove over `derive_seed` output bound to `validator_id`.
/// Contract: `vrf.leader.prove`.
pub fn leader_prove(
    vrf_sk: &[u8; 32],
    seed: &VrfSeed,
    validator_id: &ValidatorId,
) -> Result<(vrf::Output, VrfProof), VrfError> {
    vrf::prove(vrf_sk, &leader_alpha(seed, validator_id))
}

/// Verify a leader proof against the derived seed and identity.
/// Contract: `vrf.leader.verify`.
pub fn leader_verify(
    vrf_pk: &[u8; 32],
    seed: &VrfSeed,
    validator_id: &ValidatorId,
    proof: &VrfProof,
) -> Result<vrf::Output, VrfError> {
    vrf::verify(vrf_pk, &leader_alpha(seed, validator_id), proof)
}

/// Weighted-leader errors.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum WeightedError {
    /// Proof did not verify.
    #[error("leader vrf: {0}")]
    Vrf(VrfError),
    /// Empty set or all-zero voting power.
    #[error("empty or zero-power validator set")]
    EmptySet,
}

/// Stake-weighted leader from a verified VRF output (architecture.md §2.3).
///
/// Validators are walked in `ValidatorId` order (`types::collections::Map`).
/// `ticket = first 16 bytes of VRF output as u128`; leader is the first
/// validator whose cumulative voting power exceeds `ticket % total_power`.
///
/// Contract: `vrf.leader.weighted`. Always calls [`leader_verify`] first.
pub fn weighted_leader(
    vrf_pk: &[u8; 32],
    seed: &VrfSeed,
    source_id: &ValidatorId,
    proof: &VrfProof,
    validators: &Map<ValidatorId, VotingPower>,
) -> Result<ValidatorId, WeightedError> {
    let output = leader_verify(vrf_pk, seed, source_id, proof).map_err(WeightedError::Vrf)?;
    let total: u128 = validators.values().map(|p| u128::from(p.0)).sum();
    if total == 0 {
        return Err(WeightedError::EmptySet);
    }
    let mut ticket_bytes = [0u8; 16];
    ticket_bytes.copy_from_slice(&output[..16]);
    let ticket = u128::from_be_bytes(ticket_bytes) % total;
    let mut acc = 0u128;
    for (id, power) in validators {
        acc = acc.saturating_add(u128::from(power.0));
        if ticket < acc {
            return Ok(*id);
        }
    }
    Ok(*validators
        .keys()
        .next_back()
        .expect("non-empty after total > 0"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::vrf::public_key_from_seed;

    fn vid(b: u8) -> ValidatorId {
        ValidatorId::from_bytes([b; 48])
    }

    #[test]
    fn seed_deterministic_and_input_sensitive() {
        let h1 = [1u8; 32];
        let h2 = [2u8; 32];
        let e0 = Epoch(0);
        let e1 = Epoch(1);
        assert_eq!(derive_seed(&h1, e0), derive_seed(&h1, e0));
        assert_ne!(derive_seed(&h1, e0), derive_seed(&h1, e1));
        assert_ne!(derive_seed(&h1, e0), derive_seed(&h2, e0));
        let untagged = hash_to_array(&{
            let mut m = Vec::new();
            m.extend_from_slice(&h1);
            m.extend_from_slice(&e0.0.to_be_bytes());
            m
        });
        assert_ne!(derive_seed(&h1, e0), untagged);
    }

    #[test]
    fn leader_prove_verify_round_trip_and_wrong_id() {
        let sk = [7u8; 32];
        let pk = public_key_from_seed(&sk);
        let seed = derive_seed(&[9u8; 32], Epoch(3));
        let a = vid(1);
        let b = vid(2);
        let (out, proof) = leader_prove(&sk, &seed, &a).unwrap();
        assert_eq!(leader_verify(&pk, &seed, &a, &proof).unwrap(), out);
        assert!(leader_verify(&pk, &seed, &b, &proof).is_err());
        let mut bad = proof.clone();
        bad.0[0] ^= 0x01;
        assert!(leader_verify(&pk, &seed, &a, &bad).is_err());
    }

    #[test]
    fn weighted_rejects_forged_and_empty() {
        let sk = [3u8; 32];
        let pk = public_key_from_seed(&sk);
        let seed = derive_seed(&[1u8; 32], Epoch::ZERO);
        let src = vid(9);
        let (_, proof) = leader_prove(&sk, &seed, &src).unwrap();
        let mut empty: Map<ValidatorId, VotingPower> = Map::new();
        assert!(matches!(
            weighted_leader(&pk, &seed, &src, &proof, &empty),
            Err(WeightedError::EmptySet)
        ));
        empty.insert(vid(1), VotingPower(1));
        let mut forged = proof.clone();
        forged.0[5] ^= 0xff;
        assert!(weighted_leader(&pk, &seed, &src, &forged, &empty).is_err());
    }

    /// 20_000 independent seeds. Weights 1:1:2 → expected 25%/25%/50%.
    /// Accept ±4 percentage points (absolute) at this sample size.
    #[test]
    fn weighted_frequency_tracks_stake() {
        let sk = [11u8; 32];
        let pk = public_key_from_seed(&sk);
        let src = vid(0);
        let mut set: Map<ValidatorId, VotingPower> = Map::new();
        let a = vid(1);
        let b = vid(2);
        let c = vid(3);
        set.insert(a, VotingPower(1));
        set.insert(b, VotingPower(1));
        set.insert(c, VotingPower(2));
        let n = 20_000u32;
        let mut ca = 0u32;
        let mut cb = 0u32;
        let mut cc = 0u32;
        for i in 0..n {
            let mut hash = [0u8; 32];
            hash[..4].copy_from_slice(&i.to_be_bytes());
            let seed = derive_seed(&hash, Epoch::ZERO);
            let (_, proof) = leader_prove(&sk, &seed, &src).unwrap();
            let winner = weighted_leader(&pk, &seed, &src, &proof, &set).unwrap();
            if winner == a {
                ca += 1;
            } else if winner == b {
                cb += 1;
            } else if winner == c {
                cc += 1;
            }
        }
        let fa = f64::from(ca) / f64::from(n);
        let fb = f64::from(cb) / f64::from(n);
        let fc = f64::from(cc) / f64::from(n);
        assert!(
            (0.21..=0.29).contains(&fa)
                && (0.21..=0.29).contains(&fb)
                && (0.46..=0.54).contains(&fc),
            "frequencies a={fa} b={fb} c={fc} (expect ~0.25, 0.25, 0.50; ±0.04)"
        );
    }
}
