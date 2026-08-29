//! Toy KZG structured reference string (powers of tau).
//!
//! Contract: `kzg.setup` only. `kzg.commit` / `open` / `verify` are Tier 1.
//! A production trusted-setup ceremony is out of scope for Tier 0.

use ark_bls12_381::{Bls12_381, Fr, G1Affine, G1Projective, G2Affine, G2Projective};
use ark_ec::pairing::Pairing;
use ark_ec::{AffineRepr, CurveGroup, PrimeGroup};
use ark_ff::{Field, PrimeField, Zero};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
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
    /// Polynomial longer than the SRS.
    #[error("kzg polynomial exceeds SRS degree")]
    TooLong,
    /// Affine deserialize failed.
    #[error("kzg invalid curve point")]
    Point,
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

/// Serialized G1 commitment. Contract: `kzg.commit`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KzgCommitment {
    /// Uncompressed G1 affine bytes.
    pub bytes: Vec<u8>,
}

/// Opening at `z` with evaluation `y`. Contract: `kzg.open`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KzgOpening {
    /// Evaluation point.
    pub z: Fr,
    /// Claimed `p(z)`.
    pub y: Fr,
    /// Uncompressed G1 proof (commitment to the quotient).
    pub proof: Vec<u8>,
}

fn g1_from_bytes(bytes: &[u8]) -> Result<G1Affine, KzgError> {
    G1Affine::deserialize_uncompressed(&mut &*bytes).map_err(|_| KzgError::Point)
}

fn g2_from_bytes(bytes: &[u8]) -> Result<G2Affine, KzgError> {
    G2Affine::deserialize_uncompressed(&mut &*bytes).map_err(|_| KzgError::Point)
}

fn serialize_g1(p: G1Affine) -> Vec<u8> {
    let mut buf = Vec::new();
    p.serialize_uncompressed(&mut buf).expect("g1 serialize");
    buf
}

fn msm(setup: &KzgSetup, coeffs: &[Fr]) -> Result<G1Projective, KzgError> {
    if coeffs.len() > setup.g1_powers.len() {
        return Err(KzgError::TooLong);
    }
    let mut acc = G1Projective::zero();
    for (i, c) in coeffs.iter().enumerate() {
        if c.is_zero() {
            continue;
        }
        let g = g1_from_bytes(&setup.g1_powers[i])?;
        acc += g * c;
    }
    Ok(acc)
}

/// Commit to coefficients `p[i] = coeff of x^i`. Uses [`setup`] parameters.
pub fn commit(setup: &KzgSetup, coeffs: &[Fr]) -> Result<KzgCommitment, KzgError> {
    let c = msm(setup, coeffs)?;
    Ok(KzgCommitment {
        bytes: serialize_g1(c.into_affine()),
    })
}

fn eval_poly(coeffs: &[Fr], z: Fr) -> Fr {
    let mut acc = Fr::zero();
    for c in coeffs.iter().rev() {
        acc = acc * z + c;
    }
    acc
}

/// Quotient `q` such that `p(x) - p(z) = (x - z) q(x)`.
fn quotient(p: &[Fr], z: Fr) -> Vec<Fr> {
    let n = p.len();
    if n <= 1 {
        return Vec::new();
    }
    let mut q = vec![Fr::zero(); n - 1];
    q[n - 2] = p[n - 1];
    for i in (0..n - 2).rev() {
        q[i] = p[i + 1] + z * q[i + 1];
    }
    q
}

/// Open `p` at `z`. Contract: `kzg.open`.
pub fn open(setup: &KzgSetup, coeffs: &[Fr], z: Fr) -> Result<KzgOpening, KzgError> {
    let y = eval_poly(coeffs, z);
    let q = quotient(coeffs, z);
    let proof = commit(setup, &q)?;
    Ok(KzgOpening {
        z,
        y,
        proof: proof.bytes,
    })
}

/// Verify an opening against a commitment. Contract: `kzg.verify`.
pub fn verify(
    setup: &KzgSetup,
    commitment: &KzgCommitment,
    opening: &KzgOpening,
) -> Result<bool, KzgError> {
    let c = g1_from_bytes(&commitment.bytes)?;
    let pi = g1_from_bytes(&opening.proof)?;
    let g1 = G1Projective::generator();
    let g2 = G2Projective::generator();
    let tau_g2 = g2_from_bytes(&setup.g2_tau)?;
    let lhs_g1 = (c.into_group() - g1 * opening.y).into_affine();
    let rhs_g2 = (tau_g2.into_group() - g2 * opening.z).into_affine();
    let left = Bls12_381::pairing(lhs_g1, G2Projective::generator().into_affine());
    let right = Bls12_381::pairing(pi, rhs_g2);
    Ok(left == right)
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

    #[test]
    fn commit_open_verify_round_trip() {
        let srs = setup(8, b"kzg-tier1").unwrap();
        let coeffs = [Fr::from(3u64), Fr::from(5u64), Fr::from(7u64)];
        let c = commit(&srs, &coeffs).unwrap();
        let z = Fr::from(11u64);
        let op = open(&srs, &coeffs, z).unwrap();
        assert!(verify(&srs, &c, &op).unwrap());
        let mut bad = op.clone();
        bad.y += Fr::from(1u64);
        assert!(!verify(&srs, &c, &bad).unwrap());
    }

    #[test]
    fn commit_rejects_too_long() {
        let srs = setup(1, b"s").unwrap();
        let coeffs = vec![Fr::from(1u64); 5];
        assert!(matches!(commit(&srs, &coeffs), Err(KzgError::TooLong)));
    }
}
