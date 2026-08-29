//! Derive an account address from an Ed25519 public key (architecture.md §7).
//!
//! # Consensus-critical derivation
//!
//! `address = blake3(ed25519_pubkey_bytes)` truncated to [`types::ADDRESS_SIZE`]
//! (32). BLAKE3-256 is already 32 bytes, so this is the full digest.
//! Domain tags are **not** applied: this is a raw `hash.blake3` of the
//! compressed 32-byte verifying key.

use crate::hash::blake3::hash_to_array;
use crate::sig::ed25519::{self, PublicKey};
use types::{Address, ADDRESS_SIZE};

/// Derive [`Address`] from an Ed25519 public key. Contract: `address.from_ed25519`.
pub fn from_ed25519(pk: &PublicKey) -> Address {
    let digest = hash_to_array(pk.as_bytes());
    debug_assert_eq!(ADDRESS_SIZE, 32);
    Address::from_bytes(digest)
}

/// Generate a key and derive its address (tests / wallets).
pub fn from_new_keypair() -> (ed25519::SecretKey, Address) {
    let sk = ed25519::keygen();
    let pk = sk.verifying_key();
    (sk, from_ed25519(&pk))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sig::ed25519::keygen;

    #[test]
    fn derivation_is_32_bytes_and_deterministic() {
        let sk = keygen();
        let pk = sk.verifying_key();
        let a = from_ed25519(&pk);
        let b = from_ed25519(&pk);
        assert_eq!(a, b);
        assert_eq!(a.as_bytes().len(), ADDRESS_SIZE);
    }

    #[test]
    fn different_keys_differ() {
        let a = from_ed25519(&keygen().verifying_key());
        let b = from_ed25519(&keygen().verifying_key());
        assert_ne!(a, b);
    }
}
