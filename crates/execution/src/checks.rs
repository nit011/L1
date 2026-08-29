//! Nonce and balance admission checks (architecture.md §3).

use state::account::Account;
use types::tx::{Transfer, Tx};
use types::Amount;

/// Distinguishable rejection reasons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckError {
    /// Account nonce does not match `tx.nonce`.
    WrongNonce,
    /// Balance cannot cover amount + fee.
    InsufficientBalance,
}

/// Nonce must equal the account nonce. Contract: `tx.nonce_check`.
pub fn nonce_check(tx: &Tx, account: &Account) -> Result<(), CheckError> {
    if account.nonce.matches(tx.nonce) {
        Ok(())
    } else {
        Err(CheckError::WrongNonce)
    }
}

/// Sender can pay `transfer.amount + tx.max_fee`. Contract: `tx.balance_check`.
pub fn balance_check(tx: &Tx, transfer: &Transfer, account: &Account) -> Result<(), CheckError> {
    value_balance_check(tx, transfer.amount, account)
}

/// Sender can pay `amount + tx.max_fee` (staking and transfers).
pub fn value_balance_check(tx: &Tx, amount: Amount, account: &Account) -> Result<(), CheckError> {
    let need = amount
        .checked_add(tx.max_fee)
        .ok_or(CheckError::InsufficientBalance)?;
    if account.balance.checked_sub(need).is_some() {
        Ok(())
    } else {
        Err(CheckError::InsufficientBalance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Address, Amount, ChainId, Hash, Nonce};

    fn acct(bal: u128, nonce: u64) -> Account {
        Account {
            balance: Amount::new(bal),
            nonce: Nonce(nonce),
            code_hash: Hash::ZERO,
        }
    }

    fn tx(nonce: u64, amount: u128, fee: u128) -> Tx {
        Tx::transfer(
            ChainId::new(1),
            Nonce(nonce),
            21_000,
            Amount::new(fee),
            Address::from_bytes([2u8; 32]),
            Amount::new(amount),
        )
    }

    #[test]
    fn nonce_ok_and_wrong() {
        let a = acct(100, 3);
        let t = tx(3, 1, 1);
        nonce_check(&t, &a).unwrap();
        let bad = tx(4, 1, 1);
        assert_eq!(nonce_check(&bad, &a), Err(CheckError::WrongNonce));
    }

    #[test]
    fn balance_ok_and_insufficient() {
        let a = acct(10, 0);
        let t = tx(0, 5, 1);
        let tr = t.as_transfer().unwrap();
        balance_check(&t, tr, &a).unwrap();
        let t2 = tx(0, 10, 1);
        let tr2 = t2.as_transfer().unwrap();
        assert_eq!(
            balance_check(&t2, tr2, &a),
            Err(CheckError::InsufficientBalance)
        );
    }
}
