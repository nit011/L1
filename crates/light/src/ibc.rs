//! IBC-shaped commitments: a packet of app state anchored to a QC-verified
//! header (architecture.md §10).
//!
//! This is the verification *primitive* a counterparty chain would use. It is
//! **not** ICS-20/ICS-3 channel/connection/handshake productization — that is
//! Tier 20 roadmap (`IBC-style light-client verification productized`).
//!
//! [`commitment`] hashes packet payloads with [`state::merkle::compute_root`]
//! and stores the same bytes in an MPT so [`verify_packet`] can call
//! [`state::mpt::proof::verify`].

use crate::header::verify_qc;
use crate::LightError;
use consensus::qc::QuorumCertificate;
use state::merkle::compute_root;
use state::mpt::proof::{verify as mpt_verify, MptProof};
use state::mpt::Trie;
use types::collections::Map;
use types::header::Header;
use types::{Hash, Height, ValidatorId, VotingPower};

/// Cross-chain commitment bound to a verified header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IbcCommitment {
    /// `header.hash` after [`verify_qc`].
    pub header_hash: Hash,
    /// Header height.
    pub height: Height,
    /// `merkle.compute_root` over packet payload leaves (index order).
    pub merkle_root: Hash,
    /// MPT root of `packet_key(i) → payload` (for [`verify_packet`]).
    pub mpt_root: [u8; 32],
}

/// Key for packet `i` in the commitment trie.
pub fn packet_key(index: u32) -> Vec<u8> {
    let mut k = b"ibc/pkt/".to_vec();
    k.extend_from_slice(&index.to_be_bytes());
    k
}

/// Build a commitment to `packets` after verifying the header QC.
/// Contract: `ibc.commitment`.
pub fn commitment(
    header: &Header,
    qc: &QuorumCertificate,
    validators: &Map<ValidatorId, VotingPower>,
    packets: &[Vec<u8>],
) -> Result<(IbcCommitment, Trie), LightError> {
    verify_qc(header, qc, validators)?;
    let merkle_root = Hash::from_bytes(compute_root(packets));
    let mut trie = Trie::new();
    for (i, p) in packets.iter().enumerate() {
        state::mpt::put(&mut trie, &packet_key(i as u32), p.clone());
    }
    Ok((
        IbcCommitment {
            header_hash: header.hash(),
            height: header.fields.height,
            merkle_root,
            mpt_root: trie.root(),
        },
        trie,
    ))
}

/// Verify a claimed packet against [`IbcCommitment`] via `mpt.verify`.
/// Contract: `ibc.verify_packet`.
pub fn verify_packet(
    commit: &IbcCommitment,
    index: u32,
    proof: &MptProof,
    expected: &[u8],
) -> Result<(), LightError> {
    if !mpt_verify(&packet_key(index), proof, &commit.mpt_root) {
        return Err(LightError::Proof);
    }
    match &proof.value {
        Some(v) if v.as_slice() == expected => Ok(()),
        _ => Err(LightError::Proof),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{header_with, keys_and_set, sign_header};
    use state::mpt::proof::prove;
    use types::{Hash, Height, ValidatorId};

    #[test]
    fn commitment_anchors_to_verified_header() {
        let (keys, validators) = keys_and_set();
        let h = header_with(Height::GENESIS, Hash::ZERO, Hash::ZERO, ValidatorId::ZERO);
        let signed = sign_header(h, &keys, &validators);
        let packets = vec![b"transfer/1".to_vec(), b"ack/1".to_vec()];
        let (c, trie) =
            commitment(&signed.header, &signed.qc, &signed.validators, &packets).unwrap();
        assert_eq!(c.header_hash, signed.header.hash());
        assert_eq!(c.merkle_root, Hash::from_bytes(compute_root(&packets)));
        let proof = prove(&trie, &packet_key(0)).unwrap();
        verify_packet(&c, 0, &proof, b"transfer/1").unwrap();
    }

    #[test]
    fn tampered_packet_proof_is_rejected() {
        let (keys, validators) = keys_and_set();
        let h = header_with(Height::GENESIS, Hash::ZERO, Hash::ZERO, ValidatorId::ZERO);
        let signed = sign_header(h, &keys, &validators);
        let packets = vec![b"pkt-a".to_vec(), b"pkt-b".to_vec()];
        let (c, trie) =
            commitment(&signed.header, &signed.qc, &signed.validators, &packets).unwrap();
        let mut proof = prove(&trie, &packet_key(1)).unwrap();
        if let Some(v) = proof.value.as_mut() {
            v[0] ^= 1;
        }
        assert_eq!(
            verify_packet(&c, 1, &proof, b"pkt-b"),
            Err(LightError::Proof)
        );
        let mut sib = prove(&trie, &packet_key(1)).unwrap();
        if !sib.chain_merkle.siblings.is_empty() {
            sib.chain_merkle.siblings[0][0] ^= 1;
            assert_eq!(verify_packet(&c, 1, &sib, b"pkt-b"), Err(LightError::Proof));
        } else {
            sib.nodes[0][0] ^= 0xff;
            assert_eq!(verify_packet(&c, 1, &sib, b"pkt-b"), Err(LightError::Proof));
        }
    }

    #[test]
    fn wrong_mpt_root_is_rejected() {
        let (keys, validators) = keys_and_set();
        let h = header_with(Height::GENESIS, Hash::ZERO, Hash::ZERO, ValidatorId::ZERO);
        let signed = sign_header(h, &keys, &validators);
        let packets = vec![b"x".to_vec()];
        let (mut c, trie) =
            commitment(&signed.header, &signed.qc, &signed.validators, &packets).unwrap();
        let proof = prove(&trie, &packet_key(0)).unwrap();
        c.mpt_root = [0xab; 32];
        assert_eq!(verify_packet(&c, 0, &proof, b"x"), Err(LightError::Proof));
    }
}
