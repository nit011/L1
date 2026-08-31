//! Verify a header using only a QC and a known validator set (architecture.md §4.1, §7).
//!
//! BLS aggregation (`qc.verify`) keeps the certificate small as validator
//! counts grow. The light client computes [`Header::hash`] itself (Tier 3
//! preimage, including the `da_root:32` slot as frozen after Tier 12) and
//! refuses a QC that covers any other hash — even if that QC would pass
//! `qc.verify` in isolation.

use crate::LightError;
use consensus::qc::{self, QuorumCertificate};
use consensus::vote::VoteBlock;
use types::collections::Map;
use types::header::Header;
use types::{ValidatorId, VotingPower};

/// Verify `qc` covers this header's `header.hash`. Contract: `light.verify_qc`.
///
/// Inputs: header, QC, validator set from a prior verified header or checkpoint.
/// No full chain state and no trusted full node.
pub fn verify_qc(
    header: &Header,
    qc: &QuorumCertificate,
    validators: &Map<ValidatorId, VotingPower>,
) -> Result<(), LightError> {
    let hash = header.hash();
    match qc.block {
        VoteBlock::Block(h) if h == hash => {}
        _ => return Err(LightError::HeaderMismatch),
    }
    if qc.height != header.fields.height || qc.round != header.fields.round {
        return Err(LightError::HeaderMismatch);
    }
    qc::verify(qc, validators)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{header_with, keys_and_set, sign_header};
    use types::{Hash, Height, ValidatorId};

    #[test]
    fn happy_path_qc_covers_header_hash() {
        let (keys, validators) = keys_and_set();
        let h = header_with(Height::GENESIS, Hash::ZERO, Hash::ZERO, ValidatorId::ZERO);
        let signed = sign_header(h, &keys, &validators);
        verify_qc(&signed.header, &signed.qc, &signed.validators).unwrap();
        assert_eq!(signed.qc.block, VoteBlock::Block(signed.header.hash()));
        assert_eq!(signed.header.da_root, types::header::DA_ROOT_PLACEHOLDER);
        let _ = signed.header.hash_preimage();
    }

    #[test]
    fn qc_for_other_header_is_rejected_even_if_qc_verify_passes() {
        let (keys, validators) = keys_and_set();
        let a = header_with(
            Height::GENESIS,
            Hash::from_bytes([1u8; 32]),
            Hash::ZERO,
            ValidatorId::ZERO,
        );
        let b = header_with(
            Height::GENESIS,
            Hash::from_bytes([2u8; 32]),
            Hash::ZERO,
            ValidatorId::ZERO,
        );
        assert_ne!(a.hash(), b.hash());
        let signed_a = sign_header(a, &keys, &validators);
        consensus::qc::verify(&signed_a.qc, &validators).unwrap();
        assert_eq!(
            verify_qc(&b, &signed_a.qc, &validators),
            Err(LightError::HeaderMismatch)
        );
    }

    #[test]
    fn forged_aggregate_is_rejected_by_qc_verify() {
        let (keys, validators) = keys_and_set();
        let h = header_with(Height::GENESIS, Hash::ZERO, Hash::ZERO, ValidatorId::ZERO);
        let mut signed = sign_header(h, &keys, &validators);
        signed.qc.agg_sig[0] ^= 0xff;
        assert_eq!(
            verify_qc(&signed.header, &signed.qc, &signed.validators),
            Err(LightError::Qc)
        );
    }
}
