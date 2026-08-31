//! Sign a transaction envelope for a local keypair.
//!
//! # Example
//!
//! ```no_run
//! use crypto::sig::ed25519::keygen;
//! use sdk::sign_tx;
//! use types::tx::Tx;
//! use types::{Address, Amount, ChainId, Nonce, GAS_TRANSFER};
//!
//! let sk = keygen();
//! let tx = Tx::transfer(
//!     ChainId::new(1),
//!     Nonce::ZERO,
//!     GAS_TRANSFER,
//!     Amount::new(1),
//!     Address::ZERO,
//!     Amount::new(10),
//! );
//! let signed = sign_tx(&sk, tx);
//! println!("sending from {:?}", signed.from);
//! ```
//!
//! Contract: `sdk.sign_tx`.

use crypto::address::from_ed25519;
use crypto::sig::ed25519::SecretKey;
use crypto::tx::sign;
use types::tx::{SignedTx, Tx};
use types::Address;

/// A signed envelope plus the derived sender address.
#[derive(Clone, Debug)]
pub struct SignedFrom {
    /// Canonical signed tx (`tx.envelope` + Ed25519).
    pub signed: SignedTx,
    /// `address.from_ed25519` of the signing key.
    pub from: Address,
}

/// Sign `tx` with `sk` via [`crypto::tx::sign`] (`tx.sign`).
///
/// Also derives [`SignedFrom::from`] with [`crypto::address::from_ed25519`].
pub fn sign_tx(sk: &SecretKey, tx: Tx) -> SignedFrom {
    let from = from_ed25519(&sk.verifying_key());
    let signed = sign(sk, tx);
    SignedFrom { signed, from }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::sig::ed25519::keygen;
    use crypto::tx::verify_ed25519;
    use types::{Amount, ChainId, Nonce, GAS_TRANSFER};

    #[test]
    fn signed_tx_verifies_and_from_matches_key() {
        let sk = keygen();
        let tx = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            Address::ZERO,
            Amount::new(7),
        );
        let out = sign_tx(&sk, tx);
        verify_ed25519(&out.signed).unwrap();
        assert_eq!(out.from, from_ed25519(&sk.verifying_key()));
        assert_eq!(out.signed.public_key, *sk.verifying_key().as_bytes());
    }

    #[test]
    fn flipped_signature_fails_independent_verify() {
        let sk = keygen();
        let tx = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            Address::ZERO,
            Amount::new(1),
        );
        let mut out = sign_tx(&sk, tx);
        out.signed.signature[0] ^= 1;
        assert!(verify_ed25519(&out.signed).is_err());
    }
}
