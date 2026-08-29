//! Toy KZG structured reference string (powers of tau).
//!
//! Contract: `kzg.setup` only. `kzg.commit` / `open` / `verify` are Tier 1.
//! A production trusted-setup ceremony is out of scope for Tier 0.

use ark_bls12_381::{Fr, G1Affine, G1Projective, G2Affine, G2Projective};
use ark_ec::{CurveGroup, PrimeGroup};
use ark_ff::{Field, PrimeField};
use ark_serialize::CanonicalSerialize;
use thiserror::Error;

/// Public parameters: `[τ^i] G1` for `i = 0..=degree` and `[τ] G2`.
#[derive(Clone, Debug)]
pub struct KzgSetup {
    /// Maximum polynomial degree.
    pub degree: usize,
    /// Serialized uncompressed G1 affine powers.
    pub g1_powers: Vec<Vec<u8>>,
    /// Serialized uncompressed G2 affine `[τ]G2`.
    pub g2_tau: Vec<u8>,
}

/// Setup errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum KzgError {
    /// Degree must be at least 1.
    #[error("kzg degree must be >= 1")]
    Degree,
}

/// Deterministic toy SRS from `seed`. Contract: `kzg.setup`.
///
/// `tau` is derived from BLAKE3(seed). This is **not** a multi-party ceremony.
pub fn setup(degree: usize, seed: &[u8]) -> Result<KzgSetup, KzgError> {
    if degree < 1 {
        return Err(KzgError::Degree);
    }
    let digest = blake3::hash(seed);
    let tau = Fr::from_le_bytes_mod_order(digest.as_bytes());

    let mut acc = Fr::ONE;
    let mut g1_powers = Vec::with_capacity(degree + 1);
    let g1 = G1Projective::generator();
    for _ in 0..=degree {
        let aff: G1Affine = (g1 * acc).into_affine();
        let mut buf = Vec::new();
        aff.serialize_uncompressed(&mut buf).expect("g1 serialize");
        g1_powers.push(buf);
        acc *= tau;
    }

    let g2_tau: G2Affine = (G2Projective::generator() * tau).into_affine();
    let mut g2_bytes = Vec::new();
    g2_tau
        .serialize_uncompressed(&mut g2_bytes)
        .expect("g2 serialize");

    Ok(KzgSetup {
        degree,
        g1_powers,
        g2_tau: g2_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_is_deterministic() {
        let a = setup(4, b"ceremony-seed").unwrap();
        let b = setup(4, b"ceremony-seed").unwrap();
        assert_eq!(a.degree, 4);
        assert_eq!(a.g1_powers.len(), 5);
        assert_eq!(a.g1_powers, b.g1_powers);
        assert_eq!(a.g2_tau, b.g2_tau);
        assert!(!a.g2_tau.is_empty());
    }

    #[test]
    fn different_seeds_differ() {
        let a = setup(2, b"a").unwrap();
        let b = setup(2, b"b").unwrap();
        assert_ne!(a.g1_powers, b.g1_powers);
    }

    #[test]
    fn rejects_zero_degree() {
        assert!(matches!(setup(0, b"x"), Err(KzgError::Degree)));
    }
}
