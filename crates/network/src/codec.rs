//! Shared wire codec for all gossip topics (architecture.md §5 Block/tx propagation).
//!
//! One schema: [`types::encode`] / [`types::decode`] (`encoding.canonical.encode`).
//! Topic-specific payloads are length-prefixed inside that envelope — not a
//! bespoke codec per topic.

use crate::gossip::{
    ident_topic, TOPIC_BLOCK, TOPIC_EVIDENCE, TOPIC_HEADERS, TOPIC_PROPOSAL, TOPIC_TX, TOPIC_VOTE,
};
use consensus::propose::Proposal;
use consensus::replay::VoteKind;
use consensus::vote::{Vote, VoteBlock};
use crypto::vrf::Proof as VrfProof;
use libp2p::gossipsub::IdentTopic;
use storage::codec::header_from_preimage;
use types::encoding::{decode, encode};
use types::header::Header;
use types::{Hash, Height, Round, TypesError, ValidatorId};

/// Topic discriminant stored as the first payload byte (inside canonical encode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum GossipKind {
    /// `gossip.tx`
    Tx = 1,
    /// `gossip.proposal`
    Proposal = 2,
    /// `gossip.vote`
    Vote = 3,
    /// `gossip.block`
    Block = 4,
    /// `gossip.evidence`
    Evidence = 5,
    /// `gossip.headers_first`
    Header = 6,
}

impl GossipKind {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Tx),
            2 => Some(Self::Proposal),
            3 => Some(Self::Vote),
            4 => Some(Self::Block),
            5 => Some(Self::Evidence),
            6 => Some(Self::Header),
            _ => None,
        }
    }

    /// gossipsub topic for this kind (`gossip.mesh`).
    pub fn topic(self) -> IdentTopic {
        ident_topic(match self {
            Self::Tx => TOPIC_TX,
            Self::Proposal => TOPIC_PROPOSAL,
            Self::Vote => TOPIC_VOTE,
            Self::Block => TOPIC_BLOCK,
            Self::Evidence => TOPIC_EVIDENCE,
            Self::Header => TOPIC_HEADERS,
        })
    }
}

/// Frame: kind byte + inner bytes, then `encoding.canonical.encode`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GossipFrame {
    /// Topic kind.
    pub kind: GossipKind,
    /// Inner payload (tx bytes, vote bytes, …).
    pub inner: Vec<u8>,
}

/// Encode a frame. Contract: `gossip.schema`.
pub fn encode_frame(kind: GossipKind, inner: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(1 + inner.len());
    p.push(kind as u8);
    p.extend_from_slice(inner);
    encode(&p)
}

/// Decode a frame. Rejects unknown kinds and truncated buffers.
pub fn decode_frame(buf: &[u8]) -> Result<GossipFrame, TypesError> {
    let p = decode(buf)?;
    if p.is_empty() {
        return Err(TypesError::CodecTruncated);
    }
    let kind = GossipKind::from_u8(p[0]).ok_or(TypesError::CodecTruncated)?;
    Ok(GossipFrame {
        kind,
        inner: p[1..].to_vec(),
    })
}

/// Canonical vote payload (same field order as `evidence.equivocation` encoding).
pub fn encode_vote(v: &Vote) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(v.signer.as_bytes());
    p.extend_from_slice(&v.height.0.to_be_bytes());
    p.extend_from_slice(&v.round.0.to_be_bytes());
    p.push(v.kind as u8);
    match v.block {
        VoteBlock::Nil => p.push(0),
        VoteBlock::Block(h) => {
            p.push(1);
            p.extend_from_slice(h.as_bytes());
        }
    }
    p.extend_from_slice(&v.signature);
    encode(&p)
}

/// Inverse of [`encode_vote`].
pub fn decode_vote(buf: &[u8]) -> Result<Vote, TypesError> {
    let p = decode(buf)?;
    if p.len() < 48 + 8 + 4 + 1 + 1 + 96 {
        return Err(TypesError::CodecTruncated);
    }
    let mut signer_b = [0u8; 48];
    signer_b.copy_from_slice(&p[0..48]);
    let height = Height(u64::from_be_bytes(p[48..56].try_into().unwrap()));
    let round = Round(u32::from_be_bytes(p[56..60].try_into().unwrap()));
    let kind = match p[60] {
        0 => VoteKind::Prevote,
        1 => VoteKind::Precommit,
        _ => return Err(TypesError::CodecTruncated),
    };
    let (block, sig_off) = match p[61] {
        0 => (VoteBlock::Nil, 62usize),
        1 => {
            if p.len() < 62 + 32 + 96 {
                return Err(TypesError::CodecTruncated);
            }
            let mut h = [0u8; 32];
            h.copy_from_slice(&p[62..94]);
            (VoteBlock::Block(Hash::from_bytes(h)), 94usize)
        }
        _ => return Err(TypesError::CodecTruncated),
    };
    if p.len() != sig_off + 96 {
        return Err(TypesError::CodecTruncated);
    }
    let mut signature = [0u8; 96];
    signature.copy_from_slice(&p[sig_off..]);
    Ok(Vote {
        height,
        round,
        kind,
        block,
        signer: ValidatorId::from_bytes(signer_b),
        signature,
    })
}

/// Encode a [`Proposal`] from `cons.propose`.
pub fn encode_proposal(p: &Proposal) -> Vec<u8> {
    let pre = p.header.hash_preimage();
    let mut raw = Vec::new();
    raw.extend_from_slice(&p.height.0.to_be_bytes());
    raw.extend_from_slice(&p.round.0.to_be_bytes());
    raw.extend_from_slice(&(pre.len() as u32).to_be_bytes());
    raw.extend_from_slice(&pre);
    raw.extend_from_slice(p.app_hash.as_bytes());
    raw.extend_from_slice(p.proposer.as_bytes());
    raw.extend_from_slice(p.vrf_source.as_bytes());
    raw.extend_from_slice(&p.vrf_proof.0);
    raw.extend_from_slice(&p.signature);
    encode(&raw)
}

/// Inverse of [`encode_proposal`].
pub fn decode_proposal(buf: &[u8]) -> Result<Proposal, TypesError> {
    let p = decode(buf)?;
    if p.len() < 8 + 4 + 4 {
        return Err(TypesError::CodecTruncated);
    }
    let height = Height(u64::from_be_bytes(p[0..8].try_into().unwrap()));
    let round = Round(u32::from_be_bytes(p[8..12].try_into().unwrap()));
    let pre_len = u32::from_be_bytes(p[12..16].try_into().unwrap()) as usize;
    let need = 16 + pre_len + 32 + 48 + 48 + 80 + 96;
    if p.len() != need {
        return Err(TypesError::CodecTruncated);
    }
    let mut off = 16;
    let header = header_from_preimage(&p[off..off + pre_len])?;
    off += pre_len;
    let mut app = [0u8; 32];
    app.copy_from_slice(&p[off..off + 32]);
    off += 32;
    let mut proposer = [0u8; 48];
    proposer.copy_from_slice(&p[off..off + 48]);
    off += 48;
    let mut vrf_source = [0u8; 48];
    vrf_source.copy_from_slice(&p[off..off + 48]);
    off += 48;
    let mut proof = [0u8; 80];
    proof.copy_from_slice(&p[off..off + 80]);
    off += 80;
    let mut signature = [0u8; 96];
    signature.copy_from_slice(&p[off..off + 96]);
    Ok(Proposal {
        height,
        round,
        header,
        app_hash: Hash::from_bytes(app),
        proposer: ValidatorId::from_bytes(proposer),
        vrf_source: ValidatorId::from_bytes(vrf_source),
        vrf_proof: VrfProof(proof),
        signature,
    })
}

/// Header-only gossip (headers-first).
pub fn encode_header(header: &Header) -> Vec<u8> {
    encode(&header.hash_preimage())
}

/// Inverse of [`encode_header`].
pub fn decode_header(buf: &[u8]) -> Result<Header, TypesError> {
    let p = decode(buf)?;
    header_from_preimage(&p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use consensus::vote::nil;
    use crypto::from_bls;
    use crypto::sig::bls;
    use types::header::{Header, HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{TestClock, VotingPower};

    fn dummy_header() -> Header {
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
            tx_root: Hash::ZERO,
            state_root: Hash::ZERO,
            receipts_root: Hash::ZERO,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        }
    }

    #[test]
    fn frame_round_trip_uses_canonical_encode() {
        let inner = b"hello";
        let buf = encode_frame(GossipKind::Tx, inner);
        let f = decode_frame(&buf).unwrap();
        assert_eq!(f.kind, GossipKind::Tx);
        assert_eq!(f.inner, inner);
        assert_eq!(f.kind.topic().hash(), ident_topic(TOPIC_TX).hash());
    }

    #[test]
    fn decode_rejects_malformed_and_unknown_kind() {
        assert!(decode_frame(&[0xff]).is_err());
        let mut bad = encode_frame(GossipKind::Tx, b"x");
        bad[0] = 99;
        assert!(decode_frame(&bad).is_err());
        let kind_bad = encode(&[99, 1]);
        assert!(decode_frame(&kind_bad).is_err());
        let _ = kind_bad;
    }

    #[test]
    fn vote_round_trip() {
        let sk = bls::keygen().unwrap();
        let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(1));
        let v = nil(&sk, id, Height::GENESIS, Round::ZERO, VoteKind::Prevote);
        let buf = encode_vote(&v);
        assert_eq!(decode_vote(&buf).unwrap(), v);
        let mut truncated = buf.clone();
        truncated.truncate(buf.len() - 3);
        assert!(decode_vote(&truncated).is_err());
        let _ = dummy_header();
    }
}
