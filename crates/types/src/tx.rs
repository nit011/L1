//! Canonical transaction envelope (architecture.md §3; development-plan.md §1).
//!
//! # Frozen wire layout (`encoding.canonical.encode` of the payload)
//!
//! Payload bytes (big-endian, no JSON):
//! `chain_id:u64 || nonce:u64 || gas_limit:u64 || max_fee:u128 ||
//!  tag:u8 || …variant`
//!
//! Transfer (`tag = 0`): `to:32 || amount:u128`.
//! Changing this layout forks `tx.sign` and `block.tx_root`.

use crate::encoding::{decode, encode};
use crate::{Address, Amount, ChainId, Nonce, TypesError};

/// Native transfer payload. Contract: `tx.transfer`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    /// Recipient.
    pub to: Address,
    /// Amount moved.
    pub amount: Amount,
}

/// Payload variants. Only `Transfer` exists at this tier (no WASM).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TxPayload {
    /// Plain transfer.
    Transfer(Transfer),
}

/// Unsigned envelope. Contract: `tx.envelope`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tx {
    /// Replay domain.
    pub chain_id: ChainId,
    /// Sender nonce (must match account).
    pub nonce: Nonce,
    /// Gas limit declared by the sender.
    pub gas_limit: u64,
    /// Maximum fee (native token) the sender will pay.
    pub max_fee: Amount,
    /// Body.
    pub payload: TxPayload,
}

impl Tx {
    /// Transfer helper.
    pub fn transfer(
        chain_id: ChainId,
        nonce: Nonce,
        gas_limit: u64,
        max_fee: Amount,
        to: Address,
        amount: Amount,
    ) -> Self {
        Self {
            chain_id,
            nonce,
            gas_limit,
            max_fee,
            payload: TxPayload::Transfer(Transfer { to, amount }),
        }
    }

    /// Canonical bytes for hashing/signing (`encoding.canonical.encode`).
    pub fn encode(&self) -> Vec<u8> {
        encode(&self.payload_bytes())
    }

    fn payload_bytes(&self) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&self.chain_id.0.to_be_bytes());
        p.extend_from_slice(&self.nonce.0.to_be_bytes());
        p.extend_from_slice(&self.gas_limit.to_be_bytes());
        p.extend_from_slice(&self.max_fee.0.to_be_bytes());
        match &self.payload {
            TxPayload::Transfer(t) => {
                p.push(0);
                p.extend_from_slice(t.to.as_bytes());
                p.extend_from_slice(&t.amount.0.to_be_bytes());
            }
        }
        p
    }

    /// Decode a buffer produced by [`Tx::encode`].
    pub fn decode(buf: &[u8]) -> Result<Self, TypesError> {
        let raw = decode(buf)?;
        if raw.len() < 8 + 8 + 8 + 16 + 1 {
            return Err(TypesError::BadLength {
                expected: 41,
                actual: raw.len(),
            });
        }
        let chain_id = ChainId(u64::from_be_bytes(raw[0..8].try_into().unwrap()));
        let nonce = Nonce(u64::from_be_bytes(raw[8..16].try_into().unwrap()));
        let gas_limit = u64::from_be_bytes(raw[16..24].try_into().unwrap());
        let max_fee = Amount(u128::from_be_bytes(raw[24..40].try_into().unwrap()));
        let tag = raw[40];
        match tag {
            0 => {
                if raw.len() != 41 + 32 + 16 {
                    return Err(TypesError::BadLength {
                        expected: 89,
                        actual: raw.len(),
                    });
                }
                let mut to = [0u8; 32];
                to.copy_from_slice(&raw[41..73]);
                let amount = Amount(u128::from_be_bytes(raw[73..89].try_into().unwrap()));
                Ok(Self {
                    chain_id,
                    nonce,
                    gas_limit,
                    max_fee,
                    payload: TxPayload::Transfer(Transfer {
                        to: Address::from_bytes(to),
                        amount,
                    }),
                })
            }
            _ => Err(TypesError::Kv("unknown tx payload tag")),
        }
    }

    /// Transfer view if this envelope is a transfer.
    pub fn as_transfer(&self) -> Option<&Transfer> {
        match &self.payload {
            TxPayload::Transfer(t) => Some(t),
        }
    }
}

/// Signed envelope (signature is not part of [`Tx::encode`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedTx {
    /// Unsigned body.
    pub tx: Tx,
    /// Ed25519 signature (64 bytes).
    pub signature: [u8; 64],
    /// Compressed Ed25519 public key (32 bytes).
    pub public_key: [u8; 32],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_transfer() {
        let tx = Tx::transfer(
            ChainId::new(1),
            Nonce(2),
            21_000,
            Amount::new(5),
            Address::from_bytes([9u8; 32]),
            Amount::new(7),
        );
        assert_eq!(Tx::decode(&tx.encode()).unwrap(), tx);
        assert!(tx.as_transfer().is_some());
    }

    #[test]
    fn decode_rejects_truncated() {
        assert!(Tx::decode(&[1, 0, 0]).is_err());
    }
}
