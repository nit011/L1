//! Block header fields and hashes (architecture.md §4; development-plan.md header table).
//!
//! # Frozen `header.hash` preimage
//!
//! `blake3(domain.tag.apply(Header, payload))` where payload is:
//! `height:u64 || round:u32 || proposer:48 || timestamp_ms:u64 ||
//!  tx_root:32 || state_root:32 || receipts_root:32 || validators_hash:32 ||
//!  da_root:32`
//!
//! `da_root` is [`DA_ROOT_PLACEHOLDER`] (all zeros) until a caller invokes
//! [`apply_da_root`] (contract `header.da_root`). **Frozen-header decision:
//! `header.hash` is LEFT UNCHANGED** — the preimage layout is exactly what
//! Tier 3 froze (`da_root:32` already hashed). Filling the slot with a real
//! `da.root` Merkle digest changes that header's hash *value* but does not
//! add or remove hashed fields. Tier 3 golden vectors keep using the
//! placeholder and therefore keep the same hashes. Validators authenticate a
//! real DA root either (1) by writing it here *before* voting so the QC
//! covers it, or (2) by recomputing `da.root` from `block.body` they already
//! have (full nodes). This tier does not rewrite `hash_preimage`.

use crate::collections::Map;
use crate::hashing::{blake3_array, domain_wrap, merkle_root};
use crate::{Clock, Hash, Height, Round, ValidatorId, VotingPower, MAX_TIMESTAMP_DRIFT_MS};

/// PLACEHOLDER DA commitment (Tier 12). Never treat zero as a real DA root.
pub const DA_ROOT_PLACEHOLDER: Hash = Hash::ZERO;

/// Timestamp bounds (same rules as `consensus::time::timestamp_in_bounds`).
///
/// Contract used by `header.fields`. Implementation lives here so `types`
/// does not depend on `consensus`; `consensus/src/time.rs` calls this function.
pub fn timestamp_in_bounds<C: Clock>(
    clock: &C,
    height: Height,
    prev_timestamp_ms: u64,
    proposed_ms: u64,
    max_drift_ms: u64,
) -> bool {
    if height != Height::GENESIS && proposed_ms < prev_timestamp_ms {
        return false;
    }
    let now = clock.now_millis();
    let max_future = now.saturating_add(max_drift_ms);
    proposed_ms <= max_future
}

/// Non-root header fields. Contract: `header.fields`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderFields {
    /// Block height.
    pub height: Height,
    /// Consensus round.
    pub round: Round,
    /// Proposer `types.validator_id`.
    pub proposer: ValidatorId,
    /// Unix ms, validated via [`timestamp_in_bounds`].
    pub timestamp_ms: u64,
}

impl HeaderFields {
    /// Construct if the timestamp is in bounds.
    pub fn new<C: Clock>(
        clock: &C,
        height: Height,
        round: Round,
        proposer: ValidatorId,
        prev_timestamp_ms: u64,
        timestamp_ms: u64,
    ) -> Option<Self> {
        if !timestamp_in_bounds(
            clock,
            height,
            prev_timestamp_ms,
            timestamp_ms,
            MAX_TIMESTAMP_DRIFT_MS,
        ) {
            return None;
        }
        Some(Self {
            height,
            round,
            proposer,
            timestamp_ms,
        })
    }
}

/// Merkle root over the validator set. Contract: `block.validators_hash`.
///
/// Leaf encoding: `validator_id:48 || voting_power:u64 BE`, in `ValidatorId` order.
pub fn validators_hash(validators: &Map<ValidatorId, VotingPower>) -> Hash {
    let leaves: Vec<Vec<u8>> = validators
        .iter()
        .map(|(id, p)| {
            let mut l = Vec::with_capacity(56);
            l.extend_from_slice(id.as_bytes());
            l.extend_from_slice(&p.0.to_be_bytes());
            l
        })
        .collect();
    Hash::from_bytes(merkle_root(&leaves))
}

/// Full header including roots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// Height/round/proposer/timestamp.
    pub fields: HeaderFields,
    /// Tx Merkle root.
    pub tx_root: Hash,
    /// `state.commit_root` after the block.
    pub state_root: Hash,
    /// Receipt Merkle root.
    pub receipts_root: Hash,
    /// Validator-set Merkle root.
    pub validators_hash: Hash,
    /// DA Merkle root. Zero until [`apply_da_root`] (`block.da_root.placeholder`).
    pub da_root: Hash,
}

impl Header {
    /// Bytes hashed under the `header` domain.
    pub fn hash_preimage(&self) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&self.fields.height.0.to_be_bytes());
        p.extend_from_slice(&self.fields.round.0.to_be_bytes());
        p.extend_from_slice(self.fields.proposer.as_bytes());
        p.extend_from_slice(&self.fields.timestamp_ms.to_be_bytes());
        p.extend_from_slice(self.tx_root.as_bytes());
        p.extend_from_slice(self.state_root.as_bytes());
        p.extend_from_slice(self.receipts_root.as_bytes());
        p.extend_from_slice(self.validators_hash.as_bytes());
        p.extend_from_slice(self.da_root.as_bytes());
        p
    }

    /// Domain-tagged header hash. Contract: `header.hash`.
    pub fn hash(&self) -> Hash {
        Hash::from_bytes(blake3_array(&domain_wrap(b"header", &self.hash_preimage())))
    }
}

/// Write `da.root`'s Merkle commitment into the Tier 3 placeholder field.
///
/// Does **not** change [`Header::hash_preimage`]'s layout. Contract: `header.da_root`.
pub fn apply_da_root(header: &mut Header, da_merkle_root: Hash) {
    header.da_root = da_merkle_root;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestClock;

    #[test]
    fn fields_reject_bad_timestamp() {
        let clock = TestClock::new(1_000_000);
        let p = ValidatorId::from_bytes([1u8; 48]);
        assert!(HeaderFields::new(&clock, Height(1), Round::ZERO, p, 900_000, 1_000_100).is_some());
        assert!(HeaderFields::new(&clock, Height(1), Round::ZERO, p, 500_000, 400_000).is_none());
    }

    #[test]
    fn da_root_is_zero_placeholder() {
        assert_eq!(DA_ROOT_PLACEHOLDER, Hash::ZERO);
    }

    #[test]
    fn hash_changes_with_tx_root() {
        let clock = TestClock::new(1_000_000);
        let fields = HeaderFields::new(
            &clock,
            Height::GENESIS,
            Round::ZERO,
            ValidatorId::ZERO,
            0,
            1_000,
        )
        .unwrap();
        let mut h = Header {
            fields: fields.clone(),
            tx_root: Hash::ZERO,
            state_root: Hash::ZERO,
            receipts_root: Hash::ZERO,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        };
        let a = h.hash();
        h.tx_root = Hash::from_bytes([1u8; 32]);
        assert_ne!(a, h.hash());
        assert_eq!(h.da_root, DA_ROOT_PLACEHOLDER);
    }

    #[test]
    fn apply_da_root_fills_placeholder_without_changing_preimage_layout() {
        let clock = TestClock::new(1_000_000);
        let fields = HeaderFields::new(
            &clock,
            Height::GENESIS,
            Round::ZERO,
            ValidatorId::ZERO,
            0,
            1_000,
        )
        .unwrap();
        let mut h = Header {
            fields,
            tx_root: Hash::ZERO,
            state_root: Hash::ZERO,
            receipts_root: Hash::ZERO,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        };
        let before = h.hash();
        let pre_len = h.hash_preimage().len();
        apply_da_root(&mut h, Hash::from_bytes([9u8; 32]));
        assert_eq!(h.da_root, Hash::from_bytes([9u8; 32]));
        assert_eq!(h.hash_preimage().len(), pre_len);
        assert_ne!(h.hash(), before);
        h.da_root = DA_ROOT_PLACEHOLDER;
        assert_eq!(h.hash(), before);
    }
}
