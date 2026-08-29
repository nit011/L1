//! Mempool occupancy caps from `spec.constants` (architecture.md §5).
//!
//! **Eviction policy:** if the pool is at `MEMPOOL_MAX_TXS` or `MAX_BLOCK_BYTES`
//! and the incoming tx has strictly higher `tx.fee_priority` than the lowest
//! queued tx, evict that lowest tx and admit the new one. Otherwise reject
//! (`VerifyError::MempoolFull`). Gaps in an account's nonce queue after eviction
//! are allowed; `mempool.nonce_queue` will not surface a hole as ready.

use crate::queue::Mempool;
use crate::verify::VerifyError;
use execution::fees::fee_priority;
use types::tx::SignedTx;
use types::{Address, Nonce, MAX_BLOCK_BYTES, MAX_TX_BYTES, MEMPOOL_MAX_TXS};

impl Mempool {
    /// Test helper with tight caps.
    pub fn with_limits(min_fee: u128, max_txs: usize, max_bytes: usize) -> Self {
        Self {
            queued: types::collections::Map::new(),
            account_nonce: types::collections::Map::new(),
            min_fee,
            max_txs,
            max_bytes,
        }
    }
}

/// Reject oversized envelopes (`MAX_TX_BYTES`).
pub fn check_tx_bytes(signed: &SignedTx) -> Result<(), VerifyError> {
    if signed.tx.encode().len() > MAX_TX_BYTES as usize {
        return Err(VerifyError::TxTooLarge);
    }
    let _ = MEMPOOL_MAX_TXS;
    let _ = MAX_BLOCK_BYTES;
    Ok(())
}

fn lowest_priority(pool: &Mempool) -> Option<(Address, Nonce, u128)> {
    let mut low: Option<(u128, Address, Nonce)> = None;
    for (addr, m) in &pool.queued {
        for (n, tx) in m {
            let Ok(p) = fee_priority(&tx.tx) else {
                continue;
            };
            let take = match &low {
                None => true,
                Some((lp, la, ln)) => p < *lp || (p == *lp && (addr, n) < (la, ln)),
            };
            if take {
                low = Some((p, *addr, *n));
            }
        }
    }
    low.map(|(p, a, n)| (a, n, p))
}

/// Evict lowest-priority tx when full if `incoming` ranks higher.
pub fn ensure_room(pool: &mut Mempool, incoming: &SignedTx) -> Result<(), VerifyError> {
    let inc_bytes = incoming.tx.encode().len();
    let inc_p = fee_priority(&incoming.tx).map_err(VerifyError::from)?;
    loop {
        let over_count = pool.tx_count() >= pool.max_txs;
        let over_bytes = pool.byte_size().saturating_add(inc_bytes) > pool.max_bytes;
        if !over_count && !over_bytes {
            return Ok(());
        }
        let Some((addr, nonce, low_p)) = lowest_priority(pool) else {
            return Err(VerifyError::MempoolFull);
        };
        if inc_p <= low_p {
            return Err(VerifyError::MempoolFull);
        }
        pool.remove(&addr, nonce);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::sig::ed25519::SecretKey;
    use crypto::tx::sign;
    use state::account::Account;
    use types::{Amount, ChainId, Hash, GAS_TRANSFER};

    fn sk(b: u8) -> SecretKey {
        SecretKey::from_bytes(&[b; 32])
    }

    fn acct() -> Account {
        Account {
            balance: Amount::new(10_000),
            nonce: Nonce::ZERO,
            code_hash: Hash::ZERO,
        }
    }

    #[test]
    fn evicts_low_fee_when_full() {
        let low_sk = sk(8);
        let high_sk = sk(9);
        let mut pool = Mempool::with_limits(1, 1, usize::MAX);
        let low = sign(
            &low_sk,
            types::tx::Tx::transfer(
                ChainId::new(1),
                Nonce::ZERO,
                GAS_TRANSFER,
                Amount::new(1),
                types::Address::ZERO,
                Amount::new(1),
            ),
        );
        let high = sign(
            &high_sk,
            types::tx::Tx::transfer(
                ChainId::new(1),
                Nonce::ZERO,
                GAS_TRANSFER,
                Amount::new(20),
                types::Address::ZERO,
                Amount::new(1),
            ),
        );
        pool.insert(low, &acct()).unwrap();
        pool.insert(high.clone(), &acct()).unwrap();
        assert_eq!(pool.tx_count(), 1);
        let only = pool
            .queued
            .values()
            .next()
            .unwrap()
            .values()
            .next()
            .unwrap();
        assert_eq!(only.tx.max_fee, high.tx.max_fee);
    }

    #[test]
    fn lower_fee_rejected_when_full() {
        let a = sk(8);
        let b = sk(9);
        let mut pool = Mempool::with_limits(1, 1, usize::MAX);
        let high = sign(
            &a,
            types::tx::Tx::transfer(
                ChainId::new(1),
                Nonce::ZERO,
                GAS_TRANSFER,
                Amount::new(20),
                types::Address::ZERO,
                Amount::new(1),
            ),
        );
        let low = sign(
            &b,
            types::tx::Tx::transfer(
                ChainId::new(1),
                Nonce::ZERO,
                GAS_TRANSFER,
                Amount::new(1),
                types::Address::ZERO,
                Amount::new(1),
            ),
        );
        pool.insert(high, &acct()).unwrap();
        assert_eq!(pool.insert(low, &acct()), Err(VerifyError::MempoolFull));
    }
}
