//! ECVRF-EDWARDS25519-SHA512-TAI (RFC 9381 suite 0x03).
//!
//! Leader-election randomness (architecture.md §2.3, §7).

use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
use curve25519_dalek::edwards::{CompressedEdwardsY, EdwardsPoint};
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::traits::IsIdentity;
use sha2::{Digest, Sha512};
use thiserror::Error;

/// Suite string for ECVRF-EDWARDS25519-SHA512-TAI.
const SUITE: u8 = 0x03;
const C_LEN: usize = 16;
/// `encode(Gamma) || c || s` = 32 + 16 + 32.
const PROOF_LEN: usize = 80;

/// 80-byte VRF proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proof(pub [u8; PROOF_LEN]);

/// 64-byte VRF output (proof_to_hash).
pub type Output = [u8; 64];

/// VRF errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum VrfError {
    /// Proof encoding or group check failed.
    #[error("invalid vrf proof")]
    Proof,
    /// Verification equation failed.
    #[error("vrf verification failed")]
    Verify,
}

/// Secret scalar from a 32-byte seed using Ed25519 clamping (RFC 8032).
fn expand_sk(seed: &[u8; 32]) -> (Scalar, [u8; 32]) {
    let mut h = Sha512::digest(seed);
    let mut prefix = [0u8; 32];
    prefix.copy_from_slice(&h[32..64]);
    h[0] &= 248;
    h[31] &= 63;
    h[31] |= 64;
    let mut scalar_bytes = [0u8; 32];
    scalar_bytes.copy_from_slice(&h[..32]);
    (Scalar::from_bytes_mod_order(scalar_bytes), prefix)
}

fn encode_point(p: &EdwardsPoint) -> [u8; 32] {
    p.compress().to_bytes()
}

fn decode_point(bytes: &[u8; 32]) -> Option<EdwardsPoint> {
    CompressedEdwardsY(*bytes).decompress()
}

fn hash_to_curve(pk: &[u8; 32], alpha: &[u8]) -> Option<EdwardsPoint> {
    for ctr in 0u8..=255 {
        let mut hasher = Sha512::new();
        hasher.update([SUITE, 0x01]);
        hasher.update(pk);
        hasher.update(alpha);
        hasher.update([ctr]);
        let digest = hasher.finalize();
        let mut candidate = [0u8; 32];
        candidate.copy_from_slice(&digest[..32]);
        if let Some(p) = decode_point(&candidate) {
            let h = p.mul_by_cofactor();
            if !h.is_identity() {
                return Some(h);
            }
        }
    }
    None
}

fn hash_points(points: &[&EdwardsPoint]) -> Scalar {
    let mut hasher = Sha512::new();
    hasher.update([SUITE, 0x02]);
    for p in points {
        hasher.update(encode_point(p));
    }
    let digest = hasher.finalize();
    let mut c_bytes = [0u8; 32];
    c_bytes[..C_LEN].copy_from_slice(&digest[..C_LEN]);
    Scalar::from_bytes_mod_order(c_bytes)
}

fn proof_to_hash(gamma: &EdwardsPoint) -> Output {
    let mut hasher = Sha512::new();
    hasher.update([SUITE, 0x03]);
    hasher.update(encode_point(&gamma.mul_by_cofactor()));
    let d = hasher.finalize();
    let mut out = [0u8; 64];
    out.copy_from_slice(&d);
    out
}

/// Prove: `(output, proof)`. Contract: `vrf.ecvrf.prove`.
pub fn prove(seed: &[u8; 32], alpha: &[u8]) -> Result<(Output, Proof), VrfError> {
    let (x, prefix) = expand_sk(seed);
    let y = ED25519_BASEPOINT_POINT * x;
    let pk = encode_point(&y);
    let h = hash_to_curve(&pk, alpha).ok_or(VrfError::Proof)?;
    let gamma = h * x;

    let mut nonce_hash = Sha512::new();
    nonce_hash.update(prefix);
    nonce_hash.update(encode_point(&h));
    let mut wide = [0u8; 64];
    wide.copy_from_slice(&nonce_hash.finalize());
    let k = Scalar::from_bytes_mod_order_wide(&wide);

    let k_b = ED25519_BASEPOINT_POINT * k;
    let k_h = h * k;
    let c = hash_points(&[&h, &gamma, &k_b, &k_h]);
    let s = k + c * x;

    let mut pi = [0u8; PROOF_LEN];
    pi[..32].copy_from_slice(&encode_point(&gamma));
    let c_bytes = c.to_bytes();
    pi[32..32 + C_LEN].copy_from_slice(&c_bytes[..C_LEN]);
    pi[32 + C_LEN..].copy_from_slice(&s.to_bytes());

    Ok((proof_to_hash(&gamma), Proof(pi)))
}

/// Verify and return the VRF output. Contract: `vrf.ecvrf.verify`.
pub fn verify(pk_bytes: &[u8; 32], alpha: &[u8], proof: &Proof) -> Result<Output, VrfError> {
    let y = decode_point(pk_bytes).ok_or(VrfError::Proof)?;
    if y.is_identity() {
        return Err(VrfError::Proof);
    }
    let mut gamma_bytes = [0u8; 32];
    gamma_bytes.copy_from_slice(&proof.0[..32]);
    let gamma = decode_point(&gamma_bytes).ok_or(VrfError::Proof)?;

    let mut c_bytes = [0u8; 32];
    c_bytes[..C_LEN].copy_from_slice(&proof.0[32..32 + C_LEN]);
    let c = Scalar::from_bytes_mod_order(c_bytes);

    let mut s_bytes = [0u8; 32];
    s_bytes.copy_from_slice(&proof.0[32 + C_LEN..]);
    let s = Scalar::from_canonical_bytes(s_bytes);
    let s = Option::<Scalar>::from(s).ok_or(VrfError::Proof)?;

    let h = hash_to_curve(pk_bytes, alpha).ok_or(VrfError::Proof)?;

    let u = ED25519_BASEPOINT_POINT * s - y * c;
    let v = h * s - gamma * c;
    let c2 = hash_points(&[&h, &gamma, &u, &v]);
    if c != c2 {
        return Err(VrfError::Verify);
    }
    Ok(proof_to_hash(&gamma))
}

/// Edwards public key for `seed` (same expansion as [`prove`]).
pub fn public_key_from_seed(seed: &[u8; 32]) -> [u8; 32] {
    let (x, _) = expand_sk(seed);
    encode_point(&(ED25519_BASEPOINT_POINT * x))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prove_verify_round_trip() {
        let seed = [7u8; 32];
        let alpha = b"leader-seed";
        let (out, proof) = prove(&seed, alpha).unwrap();
        let (x, _) = expand_sk(&seed);
        let pk = encode_point(&(ED25519_BASEPOINT_POINT * x));
        let out2 = verify(&pk, alpha, &proof).unwrap();
        assert_eq!(out, out2);
        assert_eq!(proof.0.len(), 80);
    }

    #[test]
    fn flipped_proof_byte_fails() {
        let seed = [9u8; 32];
        let alpha = b"x";
        let (x, _) = expand_sk(&seed);
        let pk = encode_point(&(ED25519_BASEPOINT_POINT * x));
        let (out, mut proof) = prove(&seed, alpha).unwrap();
        proof.0[4] ^= 0x01;
        assert!(verify(&pk, alpha, &proof).is_err());
        let _ = out;
    }

    #[test]
    fn wrong_message_fails() {
        let seed = [3u8; 32];
        let (x, _) = expand_sk(&seed);
        let pk = encode_point(&(ED25519_BASEPOINT_POINT * x));
        let (_, proof) = prove(&seed, b"alpha").unwrap();
        assert!(verify(&pk, b"other", &proof).is_err());
    }
}
