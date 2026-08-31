//! Light-client and IBC-shaped verification (architecture.md §4.1, §7, §10).
//!
//! Trust model: data may come from an untrusted RPC or peer. Acceptance is
//! only via `qc.verify` + `header.hash` and `mpt.verify` against a root the
//! client already trusts. Full ICS productization is Tier 20.

pub mod account;
pub mod header;
pub mod ibc;
pub mod sync;

pub use account::{verify_account, GetProof};
pub use header::verify_qc;
pub use ibc::{commitment as ibc_commitment, verify_packet, IbcCommitment};
pub use sync::sync_checkpoints;

/// Light-client verification errors.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LightError {
    /// `qc.verify` rejected the certificate.
    #[error("light: qc")]
    Qc,
    /// QC is well-formed but covers a different `header.hash`.
    #[error("light: qc does not cover this header.hash")]
    HeaderMismatch,
    /// Untrusted proof failed `mpt.verify` or did not bind to the trusted root.
    #[error("light: proof")]
    Proof,
    /// RPC/params could not even be parsed (still not treated as success).
    #[error("light: untrusted source encoding")]
    Source,
    /// Offered history does not pass through the trusted `ws.checkpoint`.
    #[error("light: checkpoint")]
    Checkpoint,
    /// Heights do not form a forward chain from the checkpoint.
    #[error("light: header gap")]
    Gap,
}

impl From<consensus::qc::QcError> for LightError {
    fn from(_: consensus::qc::QcError) -> Self {
        Self::Qc
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use blst::min_pk::SecretKey;
    use consensus::qc::{self, QuorumCertificate};
    use consensus::vote::{precommit, Vote};
    use crypto::from_bls;
    use crypto::sig::bls;
    use types::collections::Map;
    use types::header::{Header, HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{Hash, Height, Round, TestClock, ValidatorId, VotingPower};

    pub struct SignedHeader {
        pub header: Header,
        pub qc: QuorumCertificate,
        pub validators: Map<ValidatorId, VotingPower>,
    }

    pub fn keys_and_set() -> (Vec<(SecretKey, ValidatorId)>, Map<ValidatorId, VotingPower>) {
        let mut keys = Vec::new();
        let mut validators = Map::new();
        for _ in 0..3 {
            let sk = bls::keygen().unwrap();
            let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(10));
            validators.insert(id, VotingPower(10));
            keys.push((sk, id));
        }
        (keys, validators)
    }

    /// Header matching post-Tier-12 `header.hash`: `da_root:32` is in the
    /// preimage; tests use [`DA_ROOT_PLACEHOLDER`] unless noted.
    pub fn header_with(
        height: Height,
        tx_root: Hash,
        state_root: Hash,
        proposer: ValidatorId,
    ) -> Header {
        let clock = TestClock::new(1_000);
        let fields = HeaderFields::new(&clock, height, Round::ZERO, proposer, 0, 1).unwrap();
        Header {
            fields,
            tx_root,
            state_root,
            receipts_root: Hash::ZERO,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        }
    }

    pub fn sign_header(
        header: Header,
        keys: &[(SecretKey, ValidatorId)],
        validators: &Map<ValidatorId, VotingPower>,
    ) -> SignedHeader {
        let votes: Vec<Vote> = keys
            .iter()
            .map(|(sk, id)| precommit(sk, *id, header.fields.height, header.fields.round, &header))
            .collect();
        let qc = qc::aggregate(&votes, validators).unwrap();
        SignedHeader {
            header,
            qc,
            validators: validators.clone(),
        }
    }
}
