//! Weak-subjectivity checkpoints (architecture.md §2.4 long-range attacks).
//!
//! Produced only from a [`Finalized`] (`cons.commit`) whose `block_hash` matches
//! [`Header::hash`]. Interval is [`CHECKPOINT_INTERVAL`] (`spec.constants`).

use crate::steps::Finalized;
use types::header::Header;
use types::{Hash, Height, CHECKPOINT_INTERVAL};

/// Checkpoint: finalized height + `header.hash`. Contract: `ws.checkpoint`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    /// Finalized height.
    pub height: Height,
    /// `header.hash` of that block.
    pub header_hash: Hash,
}

/// Record a checkpoint if `finalized` matches `header` and the height hits the interval.
pub fn record_checkpoint(finalized: &Finalized, header: &Header) -> Option<Checkpoint> {
    let header_hash = header.hash();
    if header_hash != finalized.block_hash {
        return None;
    }
    if header.fields.height != finalized.height {
        return None;
    }
    if CHECKPOINT_INTERVAL == 0 {
        return None;
    }
    if !finalized.height.0.is_multiple_of(CHECKPOINT_INTERVAL) {
        return None;
    }
    Some(Checkpoint {
        height: finalized.height,
        header_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::header::{HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{Round, TestClock, ValidatorId};

    fn header_at(height: u64) -> Header {
        let clock = TestClock::new(1_000);
        let fields =
            HeaderFields::new(&clock, Height(height), Round::ZERO, ValidatorId::ZERO, 0, 1)
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

    fn fin(header: &Header) -> Finalized {
        Finalized {
            height: header.fields.height,
            round: header.fields.round,
            block_hash: header.hash(),
            app_hash: Hash::ZERO,
        }
    }

    #[test]
    fn checkpoints_at_interval_only() {
        let h0 = header_at(0);
        assert!(record_checkpoint(&fin(&h0), &h0).is_some());
        let h1 = header_at(1);
        assert!(record_checkpoint(&fin(&h1), &h1).is_none());
        let h10 = header_at(CHECKPOINT_INTERVAL);
        let c = record_checkpoint(&fin(&h10), &h10).unwrap();
        assert_eq!(c.height, Height(CHECKPOINT_INTERVAL));
        assert_eq!(c.header_hash, h10.hash());
    }

    #[test]
    fn never_for_mismatched_non_finalized_hash() {
        let h = header_at(0);
        let mut f = fin(&h);
        f.block_hash = Hash::from_bytes([9u8; 32]);
        assert!(record_checkpoint(&f, &h).is_none());
    }
}
