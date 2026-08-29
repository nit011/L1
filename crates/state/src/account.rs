//! Account record stored in the accounts MPT (architecture.md §4.1).

use types::encoding::{decode, encode};
use types::{Address, Amount, Hash, Nonce, TypesError};

/// Account fields committed in the accounts trie.
///
/// Contract: `state.account`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    /// Native token balance (`types.amount`).
    pub balance: Amount,
    /// Replay-protection nonce (`types.nonce`).
    pub nonce: Nonce,
    /// Hash of contract bytecode, or [`Hash::ZERO`] for EOAs.
    pub code_hash: Hash,
}

impl Account {
    /// Empty EOA.
    pub fn empty() -> Self {
        Self {
            balance: Amount::ZERO,
            nonce: Nonce::ZERO,
            code_hash: Hash::ZERO,
        }
    }

    /// Canonical encoding via Tier 0 `encoding.canonical.encode`.
    pub fn encode(&self) -> Vec<u8> {
        let mut payload = Vec::with_capacity(16 + 8 + 32);
        payload.extend_from_slice(&self.balance.0.to_be_bytes());
        payload.extend_from_slice(&self.nonce.0.to_be_bytes());
        payload.extend_from_slice(self.code_hash.as_bytes());
        encode(&payload)
    }

    /// Inverse of [`Account::encode`].
    pub fn decode(buf: &[u8]) -> Result<Self, TypesError> {
        let payload = decode(buf)?;
        if payload.len() != 16 + 8 + 32 {
            return Err(TypesError::BadLength {
                expected: 56,
                actual: payload.len(),
            });
        }
        let mut bal = [0u8; 16];
        bal.copy_from_slice(&payload[0..16]);
        let mut nonce = [0u8; 8];
        nonce.copy_from_slice(&payload[16..24]);
        let mut ch = [0u8; 32];
        ch.copy_from_slice(&payload[24..56]);
        Ok(Self {
            balance: Amount(u128::from_be_bytes(bal)),
            nonce: Nonce(u64::from_be_bytes(nonce)),
            code_hash: Hash::from_bytes(ch),
        })
    }

    /// Build from genesis allocation fields (same struct layout).
    pub fn from_genesis(g: &types::genesis::GenesisAccount) -> Self {
        Self {
            balance: g.balance,
            nonce: g.nonce,
            code_hash: g.code_hash,
        }
    }
}

/// Address used as the accounts-trie key (raw 32 bytes).
pub fn account_key(addr: &Address) -> Vec<u8> {
    addr.as_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip() {
        let a = Account {
            balance: Amount::new(99),
            nonce: Nonce(3),
            code_hash: Hash::from_bytes([7u8; 32]),
        };
        assert_eq!(Account::decode(&a.encode()).unwrap(), a);
        let _ = account_key(&Address::ZERO);
    }

    #[test]
    fn decode_rejects_truncated() {
        assert!(Account::decode(&[1, 0, 0]).is_err());
        assert!(Account::decode(&encode(&[0u8; 4])).is_err());
    }
}
