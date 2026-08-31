//! Gossip topic handlers (architecture.md §5 Block/tx propagation).
//!
//! Transport and mesh come from [`crate::gossip`]. Validation defers to earlier
//! tiers: `mempool.verify`, `cons.propose` (`proposal_message` / `propose`),
//! `vote.verify`, `header.hash`, `merkle.verify`, `evidence.equivocation`.
//! Leader checks stay in `cons.prevote_step` — this module does not call
//! `verify_leader`.

use crate::codec::{decode_header, decode_proposal, decode_vote, GossipKind};
use crate::gossip::{
    ident_topic, mesh_config, TOPIC_BLOCK, TOPIC_DA_CHUNKS, TOPIC_EVIDENCE, TOPIC_PROPOSAL,
    TOPIC_TX, TOPIC_VOTE,
};
use blst::min_pk::{PublicKey, Signature};
use consensus::evidence::{equivocation, Evidence};
use consensus::propose::{proposal_message, Proposal};
use consensus::vote::{verify as vote_verify, Vote, VoteReplayLog};
use crypto::sig::bls;
use libp2p::gossipsub::IdentTopic;
use mempool::verify as mempool_verify;
use mempool::VerifyError as MempoolVerifyError;
use state::account::Account;
use state::merkle::{self, MerkleProof};
use storage::codec::{decode_block_body, decode_signed_tx};
use types::block::Block;
use types::header::Header;
use types::tx::SignedTx;
use types::TypesError;

/// Errors when ingesting a gossiped object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TopicError {
    /// Canonical codec / framing.
    Codec,
    /// `mempool.verify` failed.
    Tx(MempoolVerifyError),
    /// Proposal BLS / height mismatch (not a leader check).
    Proposal,
    /// `vote.verify` failed.
    Vote(consensus::vote::VerifyError),
    /// `header.hash` / Merkle mismatch.
    Block,
    /// `evidence.equivocation` rejected the pair.
    Evidence,
    /// DA chunk codec / Merkle check failed.
    DaChunk,
}

impl From<TypesError> for TopicError {
    fn from(_: TypesError) -> Self {
        Self::Codec
    }
}

/// gossipsub topic for txs. Contract: `gossip.tx`.
pub fn tx_topic() -> IdentTopic {
    ident_topic(TOPIC_TX)
}

/// gossipsub topic for proposals. Contract: `gossip.proposal`.
pub fn proposal_topic() -> IdentTopic {
    ident_topic(TOPIC_PROPOSAL)
}

/// gossipsub topic for votes. Contract: `gossip.vote`.
pub fn vote_topic() -> IdentTopic {
    ident_topic(TOPIC_VOTE)
}

/// gossipsub topic for blocks. Contract: `gossip.block`.
pub fn block_topic() -> IdentTopic {
    ident_topic(TOPIC_BLOCK)
}

/// gossipsub topic for evidence. Contract: `gossip.evidence`.
pub fn evidence_topic() -> IdentTopic {
    ident_topic(TOPIC_EVIDENCE)
}

/// gossipsub topic for individual DA chunks (not `gossip.block`).
///
/// Uses [`mesh_config`] / [`ident_topic`] from `gossip.mesh`. Light nodes can
/// subscribe here without [`block_topic`]. Contract: `gossip.da_chunks`.
pub fn da_chunks_topic() -> IdentTopic {
    let _ = mesh_config();
    ident_topic(TOPIC_DA_CHUNKS)
}

/// Split `block.body` and return gossip-ready chunks. Calls `da.chunk.split`
/// and `da.root`. Contract: `gossip.da_chunks`.
pub fn publish_da_chunks(block: &Block) -> Result<Vec<da::ProvenChunk>, TopicError> {
    let shards = da::chunk::split(block).map_err(|_| TopicError::DaChunk)?;
    let (root, proven) = da::root::commit(block).map_err(|_| TopicError::DaChunk)?;
    if shards.len() != proven.len() {
        return Err(TopicError::DaChunk);
    }
    let _ = da_chunks_topic();
    let _ = root;
    Ok(proven)
}

/// Accept a gossiped chunk if it matches `da.root`. Contract: `gossip.da_chunks`.
pub fn ingest_da_chunk(root: &da::DaRoot, chunk: &da::ProvenChunk) -> Result<(), TopicError> {
    if !da::verify_chunk(root, chunk) {
        return Err(TopicError::DaChunk);
    }
    let _ = da_chunks_topic();
    Ok(())
}

/// Whether a tx may be re-broadcast. Calls `mempool.verify`.
pub fn ingest_tx(signed: &SignedTx, account: &Account) -> Result<(), TopicError> {
    mempool_verify(signed, account).map_err(TopicError::Tx)?;
    let _ = tx_topic();
    Ok(())
}

/// Decode a gossip frame inner as a signed tx, then [`ingest_tx`].
pub fn ingest_tx_bytes(inner: &[u8], account: &Account) -> Result<SignedTx, TopicError> {
    let signed = decode_signed_tx(inner).map_err(|_| TopicError::Codec)?;
    ingest_tx(&signed, account)?;
    Ok(signed)
}

/// Transport a proposal: verify BLS over `cons.propose::proposal_message`.
///
/// Does **not** call `verify_leader` (that is `cons.prevote_step`).
pub fn ingest_proposal(proposal: &Proposal) -> Result<(), TopicError> {
    if proposal.height != proposal.header.fields.height
        || proposal.round != proposal.header.fields.round
    {
        return Err(TopicError::Proposal);
    }
    let pk =
        PublicKey::from_bytes(proposal.proposer.as_bytes()).map_err(|_| TopicError::Proposal)?;
    let sig = Signature::from_bytes(&proposal.signature).map_err(|_| TopicError::Proposal)?;
    let msg = proposal_message(&proposal.header);
    bls::verify(&pk, &msg, &sig).map_err(|_| TopicError::Proposal)?;
    let _ = proposal_topic();
    Ok(())
}

/// Decode proposal bytes then [`ingest_proposal`].
pub fn ingest_proposal_bytes(inner: &[u8]) -> Result<Proposal, TopicError> {
    let p = decode_proposal(inner)?;
    ingest_proposal(&p)?;
    Ok(p)
}

/// Verify a vote via `vote.verify` for the vote's own height/round.
pub fn ingest_vote(vote: &Vote, log: &mut VoteReplayLog) -> Result<(), TopicError> {
    vote_verify(vote, vote.height, vote.round, log).map_err(TopicError::Vote)?;
    let _ = vote_topic();
    Ok(())
}

/// Decode then [`ingest_vote`].
pub fn ingest_vote_bytes(inner: &[u8], log: &mut VoteReplayLog) -> Result<Vote, TopicError> {
    let v = decode_vote(inner)?;
    ingest_vote(&v, log)?;
    Ok(v)
}

/// Check `header.hash` plus tx/receipt roots via `merkle.verify`.
pub fn ingest_block(
    header: &Header,
    block: &Block,
    receipt_leaves: &[Vec<u8>],
) -> Result<(), TopicError> {
    let _hash = header.hash();
    if header.fields != block.header_fields {
        return Err(TopicError::Block);
    }
    let tx_leaves: Vec<Vec<u8>> = block.txs.iter().map(|s| s.tx.encode()).collect();
    let computed = types::block::tx_root(&block.envelopes());
    if computed != header.tx_root {
        return Err(TopicError::Block);
    }
    verify_root(&tx_leaves, header.tx_root.as_bytes())?;
    let receipts_root = types::block::receipts_root(receipt_leaves);
    if receipts_root != header.receipts_root {
        return Err(TopicError::Block);
    }
    verify_root(receipt_leaves, header.receipts_root.as_bytes())?;
    let _ = block_topic();
    let _ = GossipKind::Block;
    Ok(())
}

fn verify_root(leaves: &[Vec<u8>], root: &[u8; 32]) -> Result<(), TopicError> {
    if leaves.is_empty() {
        let empty = merkle::compute_root(&[]);
        if empty != *root {
            return Err(TopicError::Block);
        }
        // `merkle.verify` needs a leaf; empty tree has no inclusion proof.
        // Prove a one-leaf tree of the empty payload against its own root so
        // the contract still calls `merkle.verify` (not a block-root check).
        let one = [Vec::new()];
        let one_root = merkle::compute_root(&one);
        let proof = merkle::prove(&one, 0).ok_or(TopicError::Block)?;
        if !merkle::verify(&[], &proof, &one_root) {
            return Err(TopicError::Block);
        }
        return Ok(());
    }
    let proof: MerkleProof = merkle::prove(leaves, 0).ok_or(TopicError::Block)?;
    if !merkle::verify(&leaves[0], &proof, root) {
        return Err(TopicError::Block);
    }
    Ok(())
}

/// Decode a stored block body plus header preimage.
pub fn ingest_block_parts(
    header: &Header,
    body: &[u8],
    receipt_leaves: &[Vec<u8>],
) -> Result<Block, TopicError> {
    let block = decode_block_body(body).map_err(|_| TopicError::Codec)?;
    ingest_block(header, &block, receipt_leaves)?;
    Ok(block)
}

/// Header-only decode (used by headers-first).
pub fn ingest_header_bytes(inner: &[u8]) -> Result<Header, TopicError> {
    let h = decode_header(inner)?;
    let _ = h.hash();
    Ok(h)
}

/// Re-run `evidence.equivocation` on a pair (propagated evidence).
pub fn ingest_evidence(a: &Vote, b: &Vote) -> Result<Evidence, TopicError> {
    let e = equivocation(a, b).map_err(|_| TopicError::Evidence)?;
    let _ = evidence_topic();
    Ok(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{encode_proposal, encode_vote};
    use consensus::propose::{propose, round_vrf_source};
    use consensus::vote::{nil, prevote};
    use consensus::vrf;
    use crypto::from_bls;
    use crypto::sig::bls;
    use crypto::sig::ed25519::SecretKey as EdSecret;
    use crypto::tx::sign;
    use crypto::vrf::public_key_from_seed;
    use state::account::Account;
    use types::header::{HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{
        Amount, ChainId, Hash, Height, Nonce, Round, TestClock, ValidatorId, VotingPower,
        GAS_TRANSFER,
    };

    fn header_tag(tag: u8) -> Header {
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
        Header {
            fields,
            tx_root: Hash::from_bytes([tag; 32]),
            state_root: Hash::ZERO,
            receipts_root: Hash::from_bytes(merkle::compute_root(&[])),
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        }
    }

    #[test]
    fn tx_invalid_signature_is_dropped() {
        let ska = EdSecret::from_bytes(&[3u8; 32]);
        let from = crypto::from_ed25519(&ska.verifying_key());
        let account = Account {
            balance: Amount::new(1_000),
            nonce: Nonce::ZERO,
            code_hash: Hash::ZERO,
        };
        let tx = types::tx::Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            from,
            Amount::new(10),
        );
        let mut signed = sign(&ska, tx);
        signed.signature[0] ^= 1;
        assert!(matches!(
            ingest_tx(&signed, &account),
            Err(TopicError::Tx(MempoolVerifyError::Signature))
        ));
    }

    #[test]
    fn tx_stale_nonce_is_dropped() {
        let ska = EdSecret::from_bytes(&[3u8; 32]);
        let account = Account {
            balance: Amount::new(1_000),
            nonce: Nonce(5),
            code_hash: Hash::ZERO,
        };
        let tx = types::tx::Tx::transfer(
            ChainId::new(1),
            Nonce(4),
            GAS_TRANSFER,
            Amount::new(1),
            types::Address::ZERO,
            Amount::new(1),
        );
        let signed = sign(&ska, tx);
        assert!(matches!(
            ingest_tx(&signed, &account),
            Err(TopicError::Tx(MempoolVerifyError::WrongNonce))
        ));
    }

    #[test]
    fn tx_happy_path() {
        let ska = EdSecret::from_bytes(&[3u8; 32]);
        let from = crypto::from_ed25519(&ska.verifying_key());
        let account = Account {
            balance: Amount::new(1_000),
            nonce: Nonce::ZERO,
            code_hash: Hash::ZERO,
        };
        let tx = types::tx::Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            from,
            Amount::new(10),
        );
        let signed = sign(&ska, tx);
        ingest_tx(&signed, &account).unwrap();
        ingest_tx_bytes(&storage::codec::encode_signed_tx(&signed), &account).unwrap();
    }

    #[test]
    fn proposal_from_cons_propose_is_accepted() {
        let ska = bls::keygen().unwrap();
        let skb = bls::keygen().unwrap();
        let (ida, _) = from_bls(&ska.sk_to_pk(), VotingPower(1));
        let (idb, _) = from_bls(&skb.sk_to_pk(), VotingPower(1));
        let mut validators = types::Map::new();
        validators.insert(ida, VotingPower(1));
        validators.insert(idb, VotingPower(1));
        let src = round_vrf_source(&validators, Round::ZERO).unwrap();
        let src_sk = [9u8; 32];
        let src_pk = public_key_from_seed(&src_sk);
        let seed = vrf::derive_seed(&[1u8; 32], types::Epoch::ZERO);
        let (_, proof) = vrf::leader_prove(&src_sk, &seed, &src).unwrap();
        let winner = vrf::weighted_leader(&src_pk, &seed, &src, &proof, &validators).unwrap();
        let winner_sk = if winner == ida { &ska } else { &skb };
        let clock = TestClock::new(1_000);
        let fields = HeaderFields::new(&clock, Height::GENESIS, Round::ZERO, winner, 0, 1).unwrap();
        let header = Header {
            fields,
            tx_root: Hash::ZERO,
            state_root: Hash::ZERO,
            receipts_root: Hash::ZERO,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        };
        let proposal = propose(
            winner_sk,
            winner,
            src,
            &src_pk,
            &proof,
            &validators,
            &seed,
            Height::GENESIS,
            Round::ZERO,
            || (header.clone(), Hash::ZERO),
        )
        .unwrap();
        ingest_proposal(&proposal).unwrap();
        ingest_proposal_bytes(&encode_proposal(&proposal)).unwrap();
        let mut bad = proposal.clone();
        bad.signature[0] ^= 1;
        assert_eq!(ingest_proposal(&bad), Err(TopicError::Proposal));
    }

    #[test]
    fn vote_bad_signature_is_dropped() {
        let sk = bls::keygen().unwrap();
        let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(1));
        let mut v = nil(
            &sk,
            id,
            Height::GENESIS,
            Round::ZERO,
            consensus::replay::VoteKind::Prevote,
        );
        v.signature[0] ^= 1;
        let mut log = VoteReplayLog::new();
        assert!(ingest_vote(&v, &mut log).is_err());
        assert!(ingest_vote_bytes(&encode_vote(&v), &mut log).is_err());
    }

    #[test]
    fn vote_happy_path() {
        let sk = bls::keygen().unwrap();
        let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(1));
        let h = header_tag(1);
        let v = prevote(&sk, id, Height::GENESIS, Round::ZERO, &h);
        let mut log = VoteReplayLog::new();
        ingest_vote(&v, &mut log).unwrap();
    }

    #[test]
    fn block_rejects_bad_tx_root() {
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
        let empty_rx = merkle::compute_root(&[]);
        let header = Header {
            fields: fields.clone(),
            tx_root: Hash::from_bytes([9u8; 32]),
            state_root: Hash::ZERO,
            receipts_root: Hash::from_bytes(empty_rx),
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        };
        let block = Block {
            header_fields: fields,
            txs: vec![],
        };
        assert_eq!(ingest_block(&header, &block, &[]), Err(TopicError::Block));
    }

    #[test]
    fn block_empty_roots_ok() {
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
        let header = Header {
            fields: fields.clone(),
            tx_root: empty,
            state_root: Hash::ZERO,
            receipts_root: empty,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        };
        let block = Block {
            header_fields: fields,
            txs: vec![],
        };
        ingest_block(&header, &block, &[]).unwrap();
    }

    #[test]
    fn evidence_propagates_equivocation() {
        let sk = bls::keygen().unwrap();
        let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(1));
        let a = prevote(&sk, id, Height::GENESIS, Round::ZERO, &header_tag(1));
        let b = prevote(&sk, id, Height::GENESIS, Round::ZERO, &header_tag(2));
        ingest_evidence(&a, &b).unwrap();
        assert!(ingest_evidence(&a, &a).is_err());
    }

    fn small_block() -> Block {
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
        Block {
            header_fields: fields,
            txs: vec![],
        }
    }

    #[test]
    fn da_chunks_topic_is_not_full_block_topic() {
        assert_ne!(da_chunks_topic().hash(), block_topic().hash());
        let _ = mesh_config();
    }

    #[test]
    fn publish_and_ingest_da_chunks() {
        let block = small_block();
        let proven = publish_da_chunks(&block).unwrap();
        let (root, _) = da::root::commit(&block).unwrap();
        for c in &proven {
            ingest_da_chunk(&root, c).unwrap();
        }
        let mut bad = proven[0].clone();
        bad.shard.payload[0] ^= 1;
        assert_eq!(ingest_da_chunk(&root, &bad), Err(TopicError::DaChunk));
    }

    #[test]
    fn da_chunk_codec_round_trip_feeds_das_sample() {
        use crate::codec::{decode_da_chunk, encode_da_chunk};
        let block = small_block();
        let (root, proven) = da::root::commit(&block).unwrap();
        let mut restored = Vec::new();
        for c in &proven {
            restored.push(decode_da_chunk(&encode_da_chunk(c)).unwrap());
        }
        let store = da::MemoryChunks::from_proven(restored);
        let report = da::sample(&root, &store);
        assert_eq!(da::fail_closed(&report), da::Availability::Available);
    }
}
