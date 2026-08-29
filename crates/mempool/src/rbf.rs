//! Replace-by-fee (architecture.md §5).
//!
//! A replacement is accepted only if `tx.fee_priority` is at least 10% higher
//! than the queued tx with the same (`account`, `nonce`).

use crate::verify::VerifyError;
use execution::fees::fee_priority;
use types::tx::SignedTx;

/// Minimum bump: `new * 10 >= old * 11` (10% integer). Contract: `mempool.rbf`.
pub fn rbf_allowed(old: &SignedTx, new: &SignedTx) -> Result<(), VerifyError> {
    let old_p = fee_priority(&old.tx).map_err(VerifyError::from)?;
    let new_p = fee_priority(&new.tx).map_err(VerifyError::from)?;
    if old_p == 0 {
        if new_p > 0 {
            return Ok(());
        }
        return Err(VerifyError::RbfTooLow);
    }
    if new_p.saturating_mul(10) >= old_p.saturating_mul(11) {
        Ok(())
    } else {
        Err(VerifyError::RbfTooLow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mempool;
    use crypto::sig::ed25519::SecretKey;
    use crypto::tx::sign;
    use state::account::Account;
    use types::{Amount, ChainId, Hash, Nonce, ParamsRegistry, GAS_TRANSFER};

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
    fn equal_or_lower_fee_rejected_higher_accepted() {
        let ska = sk(7);
        let mut pool = Mempool::new(&ParamsRegistry::new());
        let mk = |fee: u128| {
            sign(
                &ska,
                types::tx::Tx::transfer(
                    ChainId::new(1),
                    Nonce::ZERO,
                    GAS_TRANSFER,
                    Amount::new(fee),
                    types::Address::ZERO,
                    Amount::new(1),
                ),
            )
        };
        pool.insert(mk(10), &acct()).unwrap();
        assert_eq!(pool.insert(mk(10), &acct()), Err(VerifyError::RbfTooLow));
        assert_eq!(pool.insert(mk(9), &acct()), Err(VerifyError::RbfTooLow));
        pool.insert(mk(12), &acct()).unwrap();
        let addr = crate::verify::sender_address(
            pool.queued
                .values()
                .next()
                .unwrap()
                .values()
                .next()
                .unwrap(),
        )
        .unwrap();
        let queued = pool.queued.get(&addr).unwrap().get(&Nonce::ZERO).unwrap();
        assert_eq!(queued.tx.max_fee.0, 12);
    }
}
