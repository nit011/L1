//! Bootstrap from a weak-subjectivity checkpoint, then verify headers forward
//! (architecture.md §4.1; verification half of Tier 9 `ws.bootstrap`).
//!
//! The light client treats [`Checkpoint`] (`ws.checkpoint`) as the trust
//! anchor. An alternate history that never includes that height/hash is
//! rejected even if every header carries a locally well-formed QC.

use crate::header::verify_qc;
use crate::LightError;
use consensus::checkpoint::Checkpoint;
use consensus::qc::QuorumCertificate;
use types::collections::Map;
use types::header::Header;
use types::{ValidatorId, VotingPower};

/// One advertised header plus its claimed QC (untrusted until [`verify_qc`]).
#[derive(Clone, Debug)]
pub struct AnnouncedHeader {
    /// Candidate header.
    pub header: Header,
    /// Claimed quorum certificate.
    pub qc: QuorumCertificate,
}

/// Verify `headers` from `checkpoint` onward. Contract: `light.sync_checkpoints`.
pub fn sync_checkpoints(
    checkpoint: &Checkpoint,
    headers: &[AnnouncedHeader],
    validators: &Map<ValidatorId, VotingPower>,
) -> Result<Header, LightError> {
    let mut anchor = None;
    for (i, item) in headers.iter().enumerate() {
        if item.header.fields.height == checkpoint.height {
            if item.header.hash() != checkpoint.header_hash {
                return Err(LightError::Checkpoint);
            }
            anchor = Some(i);
            break;
        }
    }
    let start = anchor.ok_or(LightError::Checkpoint)?;
    let slice = &headers[start..];
    let mut prev_height = None;
    let mut last: Option<Header> = None;
    for item in slice {
        verify_qc(&item.header, &item.qc, validators)?;
        if let Some(ph) = prev_height {
            if item.header.fields.height.0 != ph + 1 {
                return Err(LightError::Gap);
            }
        }
        prev_height = Some(item.header.fields.height.0);
        last = Some(item.header.clone());
    }
    last.ok_or(LightError::Checkpoint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{header_with, keys_and_set, sign_header};
    use consensus::checkpoint::record_checkpoint;
    use consensus::steps::Finalized;
    use types::{Hash, Height, ValidatorId};

    fn announced_from(signed: crate::fixtures::SignedHeader) -> AnnouncedHeader {
        AnnouncedHeader {
            header: signed.header,
            qc: signed.qc,
        }
    }

    fn checkpoint_for(header: &Header) -> Checkpoint {
        let f = Finalized {
            height: header.fields.height,
            round: header.fields.round,
            block_hash: header.hash(),
            app_hash: Hash::ZERO,
        };
        record_checkpoint(&f, header).expect("height 0 is on CHECKPOINT_INTERVAL")
    }

    #[test]
    fn sync_from_checkpoint_then_next_header() {
        let (keys, validators) = keys_and_set();
        let h0 = header_with(Height::GENESIS, Hash::ZERO, Hash::ZERO, ValidatorId::ZERO);
        let s0 = sign_header(h0.clone(), &keys, &validators);
        let cp = checkpoint_for(&s0.header);
        assert_eq!(cp.header_hash, s0.header.hash());
        let h1 = header_with(
            Height(1),
            Hash::from_bytes([1u8; 32]),
            Hash::ZERO,
            ValidatorId::ZERO,
        );
        let s1 = sign_header(h1, &keys, &validators);
        let tip =
            sync_checkpoints(&cp, &[announced_from(s0), announced_from(s1)], &validators).unwrap();
        assert_eq!(tip.fields.height, Height(1));
    }

    #[test]
    fn alternate_history_not_through_checkpoint_fails() {
        let (keys, validators) = keys_and_set();
        let honest = header_with(Height::GENESIS, Hash::ZERO, Hash::ZERO, ValidatorId::ZERO);
        let s_honest = sign_header(honest, &keys, &validators);
        let cp = checkpoint_for(&s_honest.header);

        let fake0 = header_with(
            Height::GENESIS,
            Hash::from_bytes([9u8; 32]),
            Hash::ZERO,
            ValidatorId::ZERO,
        );
        let s_fake = sign_header(fake0, &keys, &validators);
        assert_ne!(s_fake.header.hash(), cp.header_hash);
        consensus::qc::verify(&s_fake.qc, &validators).unwrap();
        assert_eq!(
            sync_checkpoints(&cp, &[announced_from(s_fake)], &validators),
            Err(LightError::Checkpoint)
        );
    }

    #[test]
    fn missing_checkpoint_height_fails() {
        let (keys, validators) = keys_and_set();
        let h0 = header_with(Height::GENESIS, Hash::ZERO, Hash::ZERO, ValidatorId::ZERO);
        let s0 = sign_header(h0, &keys, &validators);
        let cp = checkpoint_for(&s0.header);
        let h1 = header_with(
            Height(1),
            Hash::from_bytes([3u8; 32]),
            Hash::ZERO,
            ValidatorId::ZERO,
        );
        let s1 = sign_header(h1, &keys, &validators);
        assert_eq!(
            sync_checkpoints(&cp, &[announced_from(s1)], &validators),
            Err(LightError::Checkpoint)
        );
    }
}
