//! Dedicated validator mesh (architecture.md §5 Peer discovery).
//!
//! Validators additionally maintain a low-latency mesh with expected next-round
//! peers so proposal/vote stay off the general gossip network. Membership is
//! [`Genesis::validators`] (`genesis.validators`). Proposal/vote ingest still
//! uses [`crate::topics`].

use crate::gossip::ident_topic;
use crate::topics::{ingest_proposal, ingest_vote, TopicError};
use consensus::propose::Proposal;
use consensus::vote::{Vote, VoteReplayLog};
use libp2p::gossipsub::IdentTopic;
use libp2p::PeerId;
use types::collections::Map;
use types::genesis::Genesis;
use types::ValidatorId;

/// Dedicated topics (not the general `/l1/proposal/1` mesh).
pub const VALIDATOR_PROPOSAL_TOPIC: &str = "/l1/validator/proposal/1";
/// Dedicated vote topic.
pub const VALIDATOR_VOTE_TOPIC: &str = "/l1/validator/vote/1";

/// Validator-only overlay. Contract: `mesh.validator`.
#[derive(Clone, Debug)]
pub struct ValidatorMesh {
    /// From `genesis.validators` (sorted map).
    pub validators: Map<ValidatorId, types::VotingPower>,
    /// Optional libp2p ids for those validators (sorted by [`ValidatorId`]).
    pub peer_ids: Map<ValidatorId, PeerId>,
}

impl ValidatorMesh {
    /// Build from genesis. Contract: `mesh.validator`.
    pub fn from_genesis(genesis: &Genesis) -> Self {
        Self {
            validators: genesis.validators.clone(),
            peer_ids: Map::new(),
        }
    }

    /// Bind a validator BLS id to a gossip peer.
    pub fn bind_peer(&mut self, id: ValidatorId, peer: PeerId) {
        if self.validators.contains_key(&id) {
            self.peer_ids.insert(id, peer);
        }
    }

    /// True if `id` is in `genesis.validators`.
    pub fn is_validator(&self, id: &ValidatorId) -> bool {
        self.validators.contains_key(id)
    }

    /// True if this libp2p peer is bound to a genesis validator.
    pub fn is_validator_peer(&self, peer: &PeerId) -> bool {
        self.peer_ids.values().any(|p| p == peer)
    }
}

/// Proposal topic on the validator mesh.
pub fn validator_proposal_topic() -> IdentTopic {
    ident_topic(VALIDATOR_PROPOSAL_TOPIC)
}

/// Vote topic on the validator mesh.
pub fn validator_vote_topic() -> IdentTopic {
    ident_topic(VALIDATOR_VOTE_TOPIC)
}

/// Ingest a proposal only if the proposer is in the genesis set.
pub fn ingest_validator_proposal(
    mesh: &ValidatorMesh,
    proposal: &Proposal,
) -> Result<(), TopicError> {
    if !mesh.is_validator(&proposal.proposer) {
        return Err(TopicError::Proposal);
    }
    let _ = validator_proposal_topic();
    ingest_proposal(proposal)
}

/// Ingest a vote only if the signer is in the genesis set.
pub fn ingest_validator_vote(
    mesh: &ValidatorMesh,
    vote: &Vote,
    log: &mut VoteReplayLog,
) -> Result<(), TopicError> {
    if !mesh.is_validator(&vote.signer) {
        return Err(TopicError::Vote(consensus::vote::VerifyError::Signature));
    }
    let _ = validator_vote_topic();
    ingest_vote(vote, log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use consensus::vote::nil;
    use crypto::from_bls;
    use crypto::sig::bls;
    use types::{ChainId, Height, Round, VotingPower};

    #[test]
    fn only_genesis_validators_on_mesh() {
        let sk = bls::keygen().unwrap();
        let (id, power) = from_bls(&sk.sk_to_pk(), VotingPower(1));
        let mut genesis = Genesis::new(ChainId::new(1));
        genesis.insert_validator(id, power);
        let mesh = ValidatorMesh::from_genesis(&genesis);
        assert!(mesh.is_validator(&id));
        let outsider = ValidatorId::from_bytes([9u8; 48]);
        assert!(!mesh.is_validator(&outsider));
        let mut log = VoteReplayLog::new();
        let v = nil(
            &sk,
            id,
            Height::GENESIS,
            Round::ZERO,
            consensus::replay::VoteKind::Prevote,
        );
        ingest_validator_vote(&mesh, &v, &mut log).unwrap();
        let mut bad = v.clone();
        bad.signer = outsider;
        assert!(ingest_validator_vote(&mesh, &bad, &mut log).is_err());
    }

    #[test]
    fn unknown_peer_is_not_validator_peer() {
        let genesis = Genesis::new(ChainId::new(1));
        let mesh = ValidatorMesh::from_genesis(&genesis);
        let p = crate::identity::generate().unwrap().peer_id;
        assert!(!mesh.is_validator_peer(&p));
        assert_ne!(
            validator_proposal_topic().hash(),
            crate::topics::proposal_topic().hash()
        );
    }
}
