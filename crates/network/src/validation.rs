//! Network-layer gates for gossiped blocks (architecture.md §5).
//!
//! Finality is **not** decided here. [`valid_block_consensus`] only requires
//! Tier 5 `qc.verify` to succeed (and that the QC names this header).
//! [`valid_block_reorg_safety`] additionally records the height in
//! `cons.safety.no_two_commits` (`CommitLog`). No independent quorum counting.

use crate::topics::{ingest_block, TopicError};
use consensus::qc::{self, QcError, QuorumCertificate};
use consensus::safety::{CommitLog, SafetyError};
use consensus::steps::Finalized;
use consensus::vote::VoteBlock;
use types::block::Block;
use types::collections::Map;
use types::header::Header;
use types::{Hash, ValidatorId, VotingPower};

/// Rejection at the network gate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidError {
    /// `gossip.block` rejected the body.
    Gossip(TopicError),
    /// `qc.verify` failed (including no quorum — still consensus's rule).
    Qc(QcError),
    /// QC does not certify this header hash.
    QcMismatch,
    /// `cons.safety.no_two_commits` would be violated.
    Reorg(SafetyError),
}

/// Gate on `qc.verify`. Contract: `valid.block.consensus`.
pub fn valid_block_consensus(
    header: &Header,
    block: &Block,
    receipt_leaves: &[Vec<u8>],
    qc: &QuorumCertificate,
    validators: &Map<ValidatorId, VotingPower>,
) -> Result<(), ValidError> {
    ingest_block(header, block, receipt_leaves).map_err(ValidError::Gossip)?;
    qc::verify(qc, validators).map_err(ValidError::Qc)?;
    let want = VoteBlock::Block(header.hash());
    if qc.block != want || qc.height != header.fields.height {
        return Err(ValidError::QcMismatch);
    }
    Ok(())
}

/// Additional `CommitLog` gate. Contract: `valid.block.reorg_safety`.
pub fn valid_block_reorg_safety(
    header: &Header,
    block: &Block,
    receipt_leaves: &[Vec<u8>],
    qc: &QuorumCertificate,
    validators: &Map<ValidatorId, VotingPower>,
    commits: &mut CommitLog,
    app_hash: Hash,
) -> Result<(), ValidError> {
    valid_block_consensus(header, block, receipt_leaves, qc, validators)?;
    let f = Finalized {
        height: header.fields.height,
        round: qc.round,
        block_hash: header.hash(),
        app_hash,
    };
    commits.record(&f).map_err(ValidError::Reorg)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use consensus::qc::aggregate;
    use consensus::vote::precommit;
    use crypto::from_bls;
    use crypto::sig::bls;
    use state::merkle;
    use types::header::{HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{Height, Round, TestClock};

    fn empty_header() -> Header {
        let clock = TestClock::new(1_000);
        let fields = HeaderFields::new(
            &clock,
            Height::GENESIS,
            Round::ZERO,
            ValidatorId::ZERO,
            0,
            1,
        )
        .unwrap();
        let empty = Hash::from_bytes(merkle::compute_root(&[]));
        Header {
            fields,
            tx_root: empty,
            state_root: Hash::ZERO,
            receipts_root: empty,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        }
    }

    struct ThreeVals {
        validators: Map<ValidatorId, VotingPower>,
        keys: Vec<(blst::min_pk::SecretKey, ValidatorId)>,
        header: Header,
        block: Block,
    }

    fn three_validators() -> ThreeVals {
        let header = empty_header();
        let mut validators = Map::new();
        let mut keys = Vec::new();
        for _ in 0..3 {
            let sk = bls::keygen().unwrap();
            let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(10));
            validators.insert(id, VotingPower(10));
            keys.push((sk, id));
        }
        let block = Block {
            header_fields: header.fields.clone(),
            txs: vec![],
        };
        ThreeVals {
            validators,
            keys,
            header,
            block,
        }
    }

    #[test]
    fn qc_verify_required_no_local_quorum_math() {
        let tv = three_validators();
        let votes: Vec<_> = tv
            .keys
            .iter()
            .map(|(sk, id)| precommit(sk, *id, Height::GENESIS, Round::ZERO, &tv.header))
            .collect();
        let qc = aggregate(&votes, &tv.validators).unwrap();
        valid_block_consensus(&tv.header, &tv.block, &[], &qc, &tv.validators).unwrap();

        let qc2 = aggregate(&votes[..2], &tv.validators).unwrap();
        assert_eq!(
            valid_block_consensus(&tv.header, &tv.block, &[], &qc2, &tv.validators),
            Err(ValidError::Qc(QcError::NoQuorum))
        );
    }

    #[test]
    fn reorg_safety_rejects_second_hash_at_height() {
        let tv = three_validators();
        let votes: Vec<_> = tv
            .keys
            .iter()
            .map(|(sk, id)| precommit(sk, *id, Height::GENESIS, Round::ZERO, &tv.header))
            .collect();
        let qc = aggregate(&votes, &tv.validators).unwrap();
        let mut log = CommitLog::new();
        valid_block_reorg_safety(
            &tv.header,
            &tv.block,
            &[],
            &qc,
            &tv.validators,
            &mut log,
            Hash::ZERO,
        )
        .unwrap();

        let mut other = tv.header.clone();
        other.state_root = Hash::from_bytes([1u8; 32]);
        let votes2: Vec<_> = tv
            .keys
            .iter()
            .map(|(sk, id)| precommit(sk, *id, Height::GENESIS, Round::ZERO, &other))
            .collect();
        let qc_b = aggregate(&votes2, &tv.validators).unwrap();
        let block_b = Block {
            header_fields: other.fields.clone(),
            txs: vec![],
        };
        let err = valid_block_reorg_safety(
            &other,
            &block_b,
            &[],
            &qc_b,
            &tv.validators,
            &mut log,
            Hash::ZERO,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ValidError::Reorg(SafetyError::TwoCommits { .. })
        ));
    }
}
