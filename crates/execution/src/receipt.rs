//! Transaction receipts (architecture.md §3).
//!
//! # Frozen encoding (`encoding.canonical.encode` of payload)
//!
//! `success:u8 || gas_used:u64 || reason:u8 || event_count:u32 || events…`
//!
//! Transfer event: `tag 0 || from:32 || to:32 || amount:u128`.
//! `reason`: 0 = none/success, 1 = wrong nonce, 2 = insufficient balance,
//! 3 = gas, 4 = signature.

use crate::checks::CheckError;
use crate::events::Event;
use crate::gas::GasError;
use types::encoding::encode;

/// Why a tx failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    /// Bad Ed25519 signature.
    Signature,
    /// [`CheckError::WrongNonce`].
    WrongNonce,
    /// [`CheckError::InsufficientBalance`].
    InsufficientBalance,
    /// Gas meter rejected the envelope.
    Gas,
    /// Staking: below `staking.min_self_bond` (architecture.md §9.2).
    StakeMinBond,
    /// Staking: tombstoned validator key (`slash.tombstone`).
    StakeTombstone,
    /// Staking: withdraw before `staking.unbonding_period`.
    StakeUnbonding,
    /// Staking: insufficient bonded/delegated balance.
    StakeInsufficient,
    /// WASM bytecode failed validation (`wasm.deploy`).
    WasmInvalid,
    /// WASM fuel exhausted (`wasm.meter`).
    WasmGas,
    /// Frozen no-reentrancy policy (`wasm.call`).
    WasmReentrancy,
    /// Missing contract code.
    WasmNoCode,
}

/// Receipt. Contract: `exec.receipt`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    /// Execution succeeded.
    pub success: bool,
    /// Gas consumed (intrinsic on success; 0 on reject).
    pub gas_used: u64,
    /// Events (empty on failure).
    pub events: Vec<Event>,
    /// Set on failure.
    pub reason: Option<RejectReason>,
}

impl Receipt {
    /// Canonical bytes for `block.receipts_root`.
    pub fn encode(&self) -> Vec<u8> {
        let mut p = Vec::new();
        p.push(u8::from(self.success));
        p.extend_from_slice(&self.gas_used.to_be_bytes());
        p.push(match self.reason {
            None => 0,
            Some(RejectReason::WrongNonce) => 1,
            Some(RejectReason::InsufficientBalance) => 2,
            Some(RejectReason::Gas) => 3,
            Some(RejectReason::Signature) => 4,
            Some(RejectReason::StakeMinBond) => 5,
            Some(RejectReason::StakeTombstone) => 6,
            Some(RejectReason::StakeUnbonding) => 7,
            Some(RejectReason::StakeInsufficient) => 8,
            Some(RejectReason::WasmInvalid) => 9,
            Some(RejectReason::WasmGas) => 10,
            Some(RejectReason::WasmReentrancy) => 11,
            Some(RejectReason::WasmNoCode) => 12,
        });
        p.extend_from_slice(&(self.events.len() as u32).to_be_bytes());
        for e in &self.events {
            match e {
                Event::Transfer { from, to, amount } => {
                    p.push(0);
                    p.extend_from_slice(from.as_bytes());
                    p.extend_from_slice(to.as_bytes());
                    p.extend_from_slice(&amount.0.to_be_bytes());
                }
                Event::Stake { from, amount } => {
                    p.push(1);
                    p.extend_from_slice(from.as_bytes());
                    p.extend_from_slice(&amount.0.to_be_bytes());
                }
                Event::Wasm { from, contract } => {
                    p.push(2);
                    p.extend_from_slice(from.as_bytes());
                    p.extend_from_slice(contract.as_bytes());
                }
            }
        }
        encode(&p)
    }
}

impl From<CheckError> for RejectReason {
    fn from(e: CheckError) -> Self {
        match e {
            CheckError::WrongNonce => Self::WrongNonce,
            CheckError::InsufficientBalance => Self::InsufficientBalance,
        }
    }
}

impl From<GasError> for RejectReason {
    fn from(_: GasError) -> Self {
        Self::Gas
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Address, Amount};

    #[test]
    fn encode_success_and_failure_differ() {
        let ok = Receipt {
            success: true,
            gas_used: 21_000,
            events: vec![Event::Transfer {
                from: Address::ZERO,
                to: Address::from_bytes([1u8; 32]),
                amount: Amount::new(1),
            }],
            reason: None,
        };
        let bad = Receipt {
            success: false,
            gas_used: 0,
            events: vec![],
            reason: Some(RejectReason::WrongNonce),
        };
        assert_ne!(ok.encode(), bad.encode());
        assert!(!ok.encode().is_empty());
    }
}
