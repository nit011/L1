//! Mempool admission: call Tier 3 checks, do not reimplement them.
//!
//! architecture.md §5 (mempool DoS resistance).

use crypto::address::from_ed25519;
use crypto::sig::ed25519::public_key_from_bytes;
use crypto::tx::verify_ed25519;
use execution::checks::{balance_check, nonce_check, value_balance_check, CheckError};
use execution::gas::{gas_meter, GasError};
use execution::receipt::RejectReason;
use state::account::Account;
use types::staking::StakeKind;
use types::tx::{SignedTx, Tx};
use types::{Address, Amount};

/// Why a tx was not admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifyError {
    /// `tx.verify_ed25519` failed.
    Signature,
    /// `tx.nonce_check` failed (or nonce is below the account nonce).
    WrongNonce,
    /// `tx.balance_check` failed.
    InsufficientBalance,
    /// `tx.gas_meter` failed.
    Gas,
    /// Below `mempool.min_fee`.
    MinFee,
    /// `mempool.size_limits` rejected (and could not evict).
    MempoolFull,
    /// `mempool.rbf` bump too small.
    RbfTooLow,
    /// Declared size exceeds `spec.constants` `MAX_TX_BYTES`.
    TxTooLarge,
}

impl From<RejectReason> for VerifyError {
    fn from(r: RejectReason) -> Self {
        match r {
            RejectReason::Signature => Self::Signature,
            RejectReason::WrongNonce => Self::WrongNonce,
            RejectReason::InsufficientBalance => Self::InsufficientBalance,
            RejectReason::Gas => Self::Gas,
            RejectReason::StakeMinBond
            | RejectReason::StakeTombstone
            | RejectReason::StakeUnbonding
            | RejectReason::StakeInsufficient => Self::InsufficientBalance,
            RejectReason::WasmInvalid
            | RejectReason::WasmGas
            | RejectReason::WasmReentrancy
            | RejectReason::WasmNoCode => Self::Gas,
        }
    }
}

impl From<CheckError> for VerifyError {
    fn from(e: CheckError) -> Self {
        RejectReason::from(e).into()
    }
}

impl From<GasError> for VerifyError {
    fn from(e: GasError) -> Self {
        RejectReason::from(e).into()
    }
}

/// Sender address from the signed envelope (`address.from_ed25519`).
pub fn sender_address(signed: &SignedTx) -> Result<Address, VerifyError> {
    let pk = public_key_from_bytes(&signed.public_key).map_err(|_| VerifyError::Signature)?;
    Ok(from_ed25519(&pk))
}

fn account_for_nonce_check(tx: &Tx, account: &Account) -> Result<Account, VerifyError> {
    if tx.nonce < account.nonce {
        nonce_check(tx, account)?;
        return Err(VerifyError::WrongNonce);
    }
    let mut a = account.clone();
    a.nonce = tx.nonce;
    Ok(a)
}

/// Run Tier 3 checks against the live account. Contract: `mempool.verify`.
pub fn verify(signed: &SignedTx, account: &Account) -> Result<(), VerifyError> {
    verify_ed25519(signed).map_err(|_| VerifyError::Signature)?;
    gas_meter(&signed.tx)?;
    if let Some(stake) = signed.tx.as_stake() {
        let debit = match stake.kind {
            StakeKind::Bond | StakeKind::Delegate => stake.amount,
            StakeKind::Unbond | StakeKind::Undelegate | StakeKind::Withdraw => Amount::ZERO,
        };
        value_balance_check(&signed.tx, debit, account)?;
    } else if signed.tx.as_deploy().is_some() || signed.tx.as_call().is_some() {
        value_balance_check(&signed.tx, Amount::ZERO, account)?;
    } else {
        let Some(transfer) = signed.tx.as_transfer() else {
            return Err(VerifyError::Gas);
        };
        balance_check(&signed.tx, transfer, account)?;
    }
    let nonce_acct = account_for_nonce_check(&signed.tx, account)?;
    nonce_check(&signed.tx, &nonce_acct)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::sig::ed25519::SecretKey;
    use crypto::tx::sign;
    use types::{Amount, ChainId, Hash, Nonce, GAS_TRANSFER};

    fn sk(b: u8) -> SecretKey {
        SecretKey::from_bytes(&[b; 32])
    }

    #[test]
    fn verify_happy_path() {
        let ska = sk(3);
        let from = from_ed25519(&ska.verifying_key());
        let account = Account {
            balance: Amount::new(1_000),
            nonce: Nonce::ZERO,
            code_hash: Hash::ZERO,
        };
        let tx = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            from,
            Amount::new(10),
        );
        let signed = sign(&ska, tx);
        verify(&signed, &account).unwrap();
        assert_eq!(sender_address(&signed).unwrap(), from);
    }

    #[test]
    fn verify_rejects_bad_signature() {
        let ska = sk(3);
        let account = Account::empty();
        let tx = Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            types::Address::ZERO,
            Amount::new(1),
        );
        let mut signed = sign(&ska, tx);
        signed.signature[0] ^= 1;
        assert_eq!(verify(&signed, &account), Err(VerifyError::Signature));
    }

    #[test]
    fn verify_rejects_stale_nonce() {
        let ska = sk(3);
        let account = Account {
            balance: Amount::new(1_000),
            nonce: Nonce(5),
            code_hash: Hash::ZERO,
        };
        let tx = Tx::transfer(
            ChainId::new(1),
            Nonce(4),
            GAS_TRANSFER,
            Amount::new(1),
            types::Address::ZERO,
            Amount::new(1),
        );
        let signed = sign(&ska, tx);
        assert_eq!(verify(&signed, &account), Err(VerifyError::WrongNonce));
    }
}
