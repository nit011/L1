//! BLS12-381 min_pk signatures (pubkey G1, signature G2).
//!
//! Aggregatable validator signatures (architecture.md §7). IETF DST via
//! [`DST`].

use blst::min_pk::{AggregatePublicKey, AggregateSignature, PublicKey, SecretKey, Signature};
use blst::BLST_ERROR;
use rand::RngCore;
use thiserror::Error;

/// Domain separation tag for hash-to-curve (IETF BLS Signature draft, POP ciphersuite).
/// Contract: `bls.domain`.
pub const DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_";

/// BLS errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BlsError {
    /// Key generation failed (bad IKM).
    #[error("bls keygen failed")]
    Keygen,
    /// Signature verification failed.
    #[error("bls verify failed")]
    Verify,
    /// Aggregation failed.
    #[error("bls aggregate failed")]
    Aggregate,
}

/// Generate a secret key from OS entropy. Contract: `bls.keygen`.
pub fn keygen() -> Result<SecretKey, BlsError> {
    let mut ikm = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut ikm);
    SecretKey::key_gen(&ikm, &[]).map_err(|_| BlsError::Keygen)
}

/// Sign `msg` under [`DST`]. Contract: `bls.sign`.
pub fn sign(sk: &SecretKey, msg: &[u8]) -> Signature {
    sk.sign(msg, DST, &[])
}

/// Verify one signature. Contract: `bls.verify`.
pub fn verify(pk: &PublicKey, msg: &[u8], sig: &Signature) -> Result<(), BlsError> {
    let err = sig.verify(true, msg, DST, &[], pk, true);
    if err == BLST_ERROR::BLST_SUCCESS {
        Ok(())
    } else {
        Err(BlsError::Verify)
    }
}

/// Combine `N` signatures into one. Contract: `bls.aggregate`.
pub fn aggregate(sigs: &[&Signature]) -> Result<Signature, BlsError> {
    if sigs.is_empty() {
        return Err(BlsError::Aggregate);
    }
    let mut agg = AggregateSignature::from_signature(sigs[0]);
    for sig in sigs.iter().skip(1) {
        agg.add_signature(sig, true)
            .map_err(|_| BlsError::Aggregate)?;
    }
    Ok(agg.to_signature())
}

/// Verify an aggregated signature over distinct `(pk, msg)` pairs.
/// Contract: `bls.verifyAggregate`.
pub fn verify_aggregate(
    agg: &Signature,
    pks: &[&PublicKey],
    msgs: &[&[u8]],
) -> Result<(), BlsError> {
    if pks.len() != msgs.len() || pks.is_empty() {
        return Err(BlsError::Verify);
    }
    let err = agg.aggregate_verify(true, msgs, DST, pks, true);
    if err == BLST_ERROR::BLST_SUCCESS {
        Ok(())
    } else {
        Err(BlsError::Verify)
    }
}

/// Compressed G1 public key bytes (48).
pub fn pk_to_bytes(pk: &PublicKey) -> [u8; 48] {
    pk.to_bytes()
}

/// Fast aggregate verify when every signer signed the **same** message.
pub fn verify_fast_aggregate(
    agg: &Signature,
    pks: &[&PublicKey],
    msg: &[u8],
) -> Result<(), BlsError> {
    let mut apk = AggregatePublicKey::from_public_key(pks[0]);
    for pk in pks.iter().skip(1) {
        apk.add_public_key(pk, true)
            .map_err(|_| BlsError::Aggregate)?;
    }
    let pk = apk.to_public_key();
    verify(&pk, msg, agg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dst_is_ietf_pop() {
        assert!(DST.starts_with(b"BLS_SIG_BLS12381G2"));
        assert!(std::str::from_utf8(DST).unwrap().contains("POP"));
    }

    #[test]
    fn sign_verify_round_trip() {
        let sk = keygen().unwrap();
        let pk = sk.sk_to_pk();
        let msg = b"validator-vote";
        let sig = sign(&sk, msg);
        verify(&pk, msg, &sig).unwrap();
        assert!(verify(&pk, b"other", &sig).is_err());
    }

    #[test]
    fn flipped_sig_byte_fails() {
        let sk = keygen().unwrap();
        let pk = sk.sk_to_pk();
        let mut raw = sign(&sk, b"m").to_bytes();
        raw[10] ^= 0x01;
        if let Ok(bad) = Signature::from_bytes(&raw) {
            assert!(verify(&pk, b"m", &bad).is_err());
        }
    }

    #[test]
    fn aggregate_distinct_messages() {
        let sk1 = keygen().unwrap();
        let sk2 = keygen().unwrap();
        let pk1 = sk1.sk_to_pk();
        let pk2 = sk2.sk_to_pk();
        let m1 = b"height-1";
        let m2 = b"height-2";
        let s1 = sign(&sk1, m1);
        let s2 = sign(&sk2, m2);
        let agg = aggregate(&[&s1, &s2]).unwrap();
        verify_aggregate(&agg, &[&pk1, &pk2], &[m1.as_ref(), m2.as_ref()]).unwrap();
        assert!(verify_aggregate(&agg, &[&pk1, &pk2], &[b"x".as_ref(), m2.as_ref()]).is_err());
    }

    #[test]
    fn aggregate_empty_fails() {
        assert!(aggregate(&[]).is_err());
    }
}
