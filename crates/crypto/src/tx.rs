//! Transaction signatures (architecture.md §7).
//!
//! Sign/verify `Tx::encode()` after `domain.tag.apply(Tx, …)`.

use crate::hash::blake3::hash_to_array;
use crate::sig::ed25519::{self, public_key_from_bytes, Ed25519Error, SecretKey};
use crate::{apply_domain, DomainTag};
use ed25519_dalek::Signature;
use types::tx::{SignedTx, Tx};

/// Domain-separated message that is signed.
pub fn signed_message(tx: &Tx) -> Vec<u8> {
    apply_domain(DomainTag::Tx, &tx.encode())
}

/// Sign an envelope. Contract: `tx.sign`.
pub fn sign(sk: &SecretKey, tx: Tx) -> SignedTx {
    let msg = signed_message(&tx);
    let sig = ed25519::sign(sk, &msg);
    SignedTx {
        tx,
        signature: sig.to_bytes(),
        public_key: *sk.verifying_key().as_bytes(),
    }
}

/// Verify a signed envelope. Contract: `tx.verify_ed25519`.
pub fn verify_ed25519(signed: &SignedTx) -> Result<(), Ed25519Error> {
    let pk = public_key_from_bytes(&signed.public_key)?;
    let sig = Signature::from_bytes(&signed.signature);
    let msg = signed_message(&signed.tx);
    ed25519::verify(&pk, &msg, &sig)
}

/// Hash of the domain-wrapped envelope (tests / indexing).
pub fn tx_signing_digest(tx: &Tx) -> [u8; 32] {
    hash_to_array(&signed_message(tx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sig::ed25519::keygen;
    use types::{Address, Amount, ChainId, Nonce};

    fn sample_tx(n: u64) -> Tx {
        Tx::transfer(
            ChainId::new(1),
            Nonce(n),
            21_000,
            Amount::new(1),
            Address::from_bytes([3u8; 32]),
            Amount::new(1),
        )
    }

    #[test]
    fn sign_verify_and_wrong_tx() {
        let sk = keygen();
        let tx_a = sample_tx(0);
        let tx_b = sample_tx(1);
        let signed = sign(&sk, tx_a);
        verify_ed25519(&signed).unwrap();
        let mut other = signed.clone();
        other.tx = tx_b;
        assert!(verify_ed25519(&other).is_err());
        let mut bad_sig = signed.clone();
        bad_sig.signature[0] ^= 1;
        assert!(verify_ed25519(&bad_sig).is_err());
    }
}
