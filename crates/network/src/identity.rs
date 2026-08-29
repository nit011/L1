//! libp2p identity derived from Tier 0 `ed25519.keygen` (architecture.md §5 Transport).
//!
//! One keypair scheme: the node's [`PeerId`] is the libp2p encoding of the same
//! Ed25519 seed produced by [`crypto::ed25519::keygen`]. A second, unrelated
//! identity key is forbidden.

use crypto::ed25519::{self, SecretKey};
use libp2p::identity::Keypair;
use libp2p::PeerId;
use thiserror::Error;

/// Node identity: chain Ed25519 secret plus libp2p [`Keypair`] / [`PeerId`].
///
/// Contract: `p2p.identity`.
#[derive(Clone)]
pub struct NodeIdentity {
    /// Secret from `ed25519.keygen` (or an equivalent 32-byte seed).
    pub ed25519: SecretKey,
    /// libp2p keypair (same seed).
    pub keypair: Keypair,
    /// libp2p peer id derived from [`Self::keypair`].
    pub peer_id: PeerId,
}

/// Identity construction errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum IdentityError {
    /// libp2p rejected the Ed25519 seed (should not happen for dalek keys).
    #[error("ed25519 seed is not a valid libp2p identity")]
    Keypair,
}

/// Build identity from an existing Ed25519 secret (`ed25519.keygen` output).
pub fn from_ed25519_secret(sk: SecretKey) -> Result<NodeIdentity, IdentityError> {
    let mut seed = sk.to_bytes();
    let keypair = Keypair::ed25519_from_bytes(&mut seed).map_err(|_| IdentityError::Keypair)?;
    let peer_id = PeerId::from(keypair.public());
    Ok(NodeIdentity {
        ed25519: sk,
        keypair,
        peer_id,
    })
}

/// Generate a node identity via `ed25519.keygen`. Contract: `p2p.identity`.
pub fn generate() -> Result<NodeIdentity, IdentityError> {
    from_ed25519_secret(ed25519::keygen())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_id_comes_from_ed25519_keygen_seed() {
        let sk = ed25519::keygen();
        let a = from_ed25519_secret(sk.clone()).unwrap();
        let b = from_ed25519_secret(sk).unwrap();
        assert_eq!(a.peer_id, b.peer_id);
        assert_ne!(a.peer_id.to_bytes(), vec![0u8; 32]);
    }

    #[test]
    fn generate_produces_distinct_peer_ids() {
        let a = generate().unwrap();
        let b = generate().unwrap();
        assert_ne!(a.peer_id, b.peer_id);
    }

    #[test]
    fn unrelated_seed_is_a_different_peer() {
        let sk = SecretKey::from_bytes(&[7u8; 32]);
        let id = from_ed25519_secret(sk).unwrap();
        let other = from_ed25519_secret(SecretKey::from_bytes(&[8u8; 32])).unwrap();
        assert_ne!(id.peer_id, other.peer_id);
    }
}
