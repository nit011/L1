//! Ed25519 account signatures (architecture.md §7; development-plan.md §1).

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use thiserror::Error;

/// Ed25519 secret key (32-byte seed).
pub type SecretKey = SigningKey;

/// Ed25519 public key.
pub type PublicKey = VerifyingKey;

/// Signing errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum Ed25519Error {
    /// Public key bytes were not a valid compressed Edwards point.
    #[error("invalid ed25519 public key")]
    PublicKey,
    /// Signature bytes were not 64 bytes or failed verification.
    #[error("invalid ed25519 signature")]
    Signature,
}

/// Generate a random keypair. Contract: `ed25519.keygen`.
pub fn keygen() -> SecretKey {
    SigningKey::generate(&mut OsRng)
}

/// Sign `msg`. Contract: `ed25519.sign`.
pub fn sign(sk: &SecretKey, msg: &[u8]) -> Signature {
    sk.sign(msg)
}

/// Verify `sig` over `msg`. Contract: `ed25519.verify`.
pub fn verify(pk: &PublicKey, msg: &[u8], sig: &Signature) -> Result<(), Ed25519Error> {
    pk.verify(msg, sig).map_err(|_| Ed25519Error::Signature)
}

/// Parse a public key from 32 bytes.
pub fn public_key_from_bytes(bytes: &[u8; 32]) -> Result<PublicKey, Ed25519Error> {
    VerifyingKey::from_bytes(bytes).map_err(|_| Ed25519Error::PublicKey)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 8032 test vector 1 (empty message).
    #[test]
    fn rfc8032_vector_1() {
        let sk_bytes =
            hex::decode("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
                .unwrap();
        let sk = SigningKey::from_bytes(&sk_bytes.try_into().unwrap());
        let expected_pk =
            hex::decode("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
                .unwrap();
        assert_eq!(sk.verifying_key().as_bytes().as_slice(), expected_pk);
        let sig = sign(&sk, b"");
        verify(&sk.verifying_key(), b"", &sig).unwrap();
        assert_eq!(
            hex::encode(sig.to_bytes()),
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
             5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
        );
        assert!(verify(&sk.verifying_key(), b"x", &sig).is_err());
    }

    #[test]
    fn sign_verify_round_trip() {
        let sk = keygen();
        let pk = sk.verifying_key();
        let msg = b"l1-ed25519";
        let sig = sign(&sk, msg);
        verify(&pk, msg, &sig).unwrap();
        assert!(verify(&pk, b"tampered", &sig).is_err());
    }

    #[test]
    fn flipped_signature_byte_fails() {
        let sk = keygen();
        let pk = sk.verifying_key();
        let mut bytes = sign(&sk, b"m").to_bytes();
        bytes[0] ^= 0xff;
        let bad = Signature::from_bytes(&bytes);
        assert!(verify(&pk, b"m", &bad).is_err());
    }

    #[test]
    fn invalid_public_key_bytes() {
        let mut bad = [0u8; 32];
        bad[31] = 0xff;
        bad[30] = 0xff;
        let _ = public_key_from_bytes(&bad);
    }
}
