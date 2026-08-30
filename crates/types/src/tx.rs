//! Canonical transaction envelope (architecture.md §3; development-plan.md §1).
//!
//! # Frozen wire layout (`encoding.canonical.encode` of the payload)
//!
//! Payload bytes (big-endian, no JSON):
//! `chain_id:u64 || nonce:u64 || gas_limit:u64 || max_fee:u128 ||
//!  tag:u8 || …variant`
//!
//! Transfer (`tag = 0`): `to:32 || amount:u128`.
//! Staking (`tags 1–5`): see [`crate::staking`].
//! Deploy (`tag = 6`): `code_len:u32 || code`. Call (`tag = 7`): `to:32 || data_len:u32 || data`.

use crate::encoding::{decode, encode};
use crate::staking::{StakeKind, StakePayload};
use crate::{Address, Amount, ChainId, Nonce, TypesError, ValidatorId};

/// Native transfer payload. Contract: `tx.transfer`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    /// Recipient.
    pub to: Address,
    /// Amount moved.
    pub amount: Amount,
}

/// Payload variants. Transfer tag `0` is frozen; staking is tags `1–5`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TxPayload {
    /// Plain transfer.
    Transfer(Transfer),
    /// Staking (`tx.stake.bond` … `tx.stake.withdraw`).
    Stake(StakePayload),
    /// Create a contract (`tx.deploy`).
    Deploy(Deploy),
    /// Call a contract (`tx.call`).
    Call(Call),
}

/// WASM deploy payload. Contract: `tx.deploy`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deploy {
    /// Guest bytecode.
    pub code: Vec<u8>,
}

/// WASM call payload. Contract: `tx.call`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    /// Contract account (`types.address`).
    pub to: Address,
    /// Calldata (may be empty).
    pub data: Vec<u8>,
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
            TxPayload::Stake(s) => {
                p.push(s.kind as u8);
                match s.kind {
                    StakeKind::Withdraw => {
                        p.extend_from_slice(&s.amount.0.to_be_bytes());
                    }
                    StakeKind::Bond
                    | StakeKind::Unbond
                    | StakeKind::Delegate
                    | StakeKind::Undelegate => {
                        let id = s.validator.expect("stake target");
                        p.extend_from_slice(id.as_bytes());
                        p.extend_from_slice(&s.amount.0.to_be_bytes());
                    }
                }
            }
            TxPayload::Deploy(d) => {
                p.push(6);
                p.extend_from_slice(&(d.code.len() as u32).to_be_bytes());
                p.extend_from_slice(&d.code);
            }
            TxPayload::Call(c) => {
                p.push(7);
                p.extend_from_slice(c.to.as_bytes());
                p.extend_from_slice(&(c.data.len() as u32).to_be_bytes());
                p.extend_from_slice(&c.data);
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
            1..=4 => {
                if raw.len() != 41 + 48 + 16 {
                    return Err(TypesError::BadLength {
                        expected: 105,
                        actual: raw.len(),
                    });
                }
                let mut vid = [0u8; 48];
                vid.copy_from_slice(&raw[41..89]);
                let amount = Amount(u128::from_be_bytes(raw[89..105].try_into().unwrap()));
                let kind = match tag {
                    1 => StakeKind::Bond,
                    2 => StakeKind::Unbond,
                    3 => StakeKind::Delegate,
                    4 => StakeKind::Undelegate,
                    _ => unreachable!(),
                };
                Ok(Self {
                    chain_id,
                    nonce,
                    gas_limit,
                    max_fee,
                    payload: TxPayload::Stake(StakePayload {
                        kind,
                        validator: Some(ValidatorId::from_bytes(vid)),
                        amount,
                    }),
                })
            }
            5 => {
                if raw.len() != 41 + 16 {
                    return Err(TypesError::BadLength {
                        expected: 57,
                        actual: raw.len(),
                    });
                }
                let amount = Amount(u128::from_be_bytes(raw[41..57].try_into().unwrap()));
                Ok(Self {
                    chain_id,
                    nonce,
                    gas_limit,
                    max_fee,
                    payload: TxPayload::Stake(StakePayload {
                        kind: StakeKind::Withdraw,
                        validator: None,
                        amount,
                    }),
                })
            }
            6 => {
                if raw.len() < 45 {
                    return Err(TypesError::BadLength {
                        expected: 45,
                        actual: raw.len(),
                    });
                }
                let n = u32::from_be_bytes(raw[41..45].try_into().unwrap()) as usize;
                if raw.len() != 45 + n {
                    return Err(TypesError::BadLength {
                        expected: 45 + n,
                        actual: raw.len(),
                    });
                }
                Ok(Self {
                    chain_id,
                    nonce,
                    gas_limit,
                    max_fee,
                    payload: TxPayload::Deploy(Deploy {
                        code: raw[45..].to_vec(),
                    }),
                })
            }
            7 => {
                if raw.len() < 41 + 32 + 4 {
                    return Err(TypesError::BadLength {
                        expected: 77,
                        actual: raw.len(),
                    });
                }
                let mut to = [0u8; 32];
                to.copy_from_slice(&raw[41..73]);
                let n = u32::from_be_bytes(raw[73..77].try_into().unwrap()) as usize;
                if raw.len() != 77 + n {
                    return Err(TypesError::BadLength {
                        expected: 77 + n,
                        actual: raw.len(),
                    });
                }
                Ok(Self {
                    chain_id,
                    nonce,
                    gas_limit,
                    max_fee,
                    payload: TxPayload::Call(Call {
                        to: Address::from_bytes(to),
                        data: raw[77..].to_vec(),
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
            TxPayload::Stake(_) | TxPayload::Deploy(_) | TxPayload::Call(_) => None,
        }
    }

    /// Deploy helper. Contract: `tx.deploy` (payload on [`Tx`] / `tx.envelope`).
    pub fn deploy(
        chain_id: ChainId,
        nonce: Nonce,
        gas_limit: u64,
        max_fee: Amount,
        code: Vec<u8>,
    ) -> Self {
        Self {
            chain_id,
            nonce,
            gas_limit,
            max_fee,
            payload: TxPayload::Deploy(Deploy { code }),
        }
    }

    /// Call helper. Contract: `tx.call`.
    pub fn call(
        chain_id: ChainId,
        nonce: Nonce,
        gas_limit: u64,
        max_fee: Amount,
        to: Address,
        data: Vec<u8>,
    ) -> Self {
        Self {
            chain_id,
            nonce,
            gas_limit,
            max_fee,
            payload: TxPayload::Call(Call { to, data }),
        }
    }

    /// Deploy view.
    pub fn as_deploy(&self) -> Option<&Deploy> {
        match &self.payload {
            TxPayload::Deploy(d) => Some(d),
            _ => None,
        }
    }

    /// Call view. Contract: `tx.call` (`types.address` on [`Call::to`]).
    pub fn as_call(&self) -> Option<&Call> {
        match &self.payload {
            TxPayload::Call(c) => Some(c),
            _ => None,
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
    fn encode_decode_deploy_and_call() {
        let d = Tx::deploy(
            ChainId::new(1),
            Nonce(1),
            50_000,
            Amount::new(1),
            vec![0, 1, 2],
        );
        assert_eq!(Tx::decode(&d.encode()).unwrap(), d);
        let c = Tx::call(
            ChainId::new(1),
            Nonce(2),
            21_000,
            Amount::new(1),
            Address::from_bytes([3u8; 32]),
            vec![9],
        );
        assert_eq!(Tx::decode(&c.encode()).unwrap(), c);
        assert_eq!(c.as_call().unwrap().to, Address::from_bytes([3u8; 32]));
    }

    #[test]
    fn decode_rejects_truncated() {
        assert!(Tx::decode(&[1, 0, 0]).is_err());
    }
}
