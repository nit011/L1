//! Per-account nonce queues (`types.nonce`).
//!
//! A tx with nonce N+1 is not ready until nonce N is present or already
//! applied (architecture.md §5).

use crate::verify::{sender_address, verify, VerifyError};
use state::account::Account;
use types::collections::Map;
use types::tx::SignedTx;
use types::{Address, Nonce};

/// Pending txs grouped by sender then nonce. Contract: `mempool.nonce_queue`.
#[derive(Clone, Debug, Default)]
pub struct Mempool {
    /// `Address` → (`Nonce` → tx). Both maps are sorted (`types.collections.Map`).
    pub(crate) queued: Map<Address, Map<Nonce, SignedTx>>,
    /// Next nonce expected from chain/account state.
    pub(crate) account_nonce: Map<Address, Nonce>,
    pub(crate) min_fee: u128,
    pub(crate) max_txs: usize,
    pub(crate) max_bytes: usize,
}

impl Mempool {
    pub(crate) fn tx_count(&self) -> usize {
        self.queued.values().map(Map::len).sum()
    }

    pub(crate) fn byte_size(&self) -> usize {
        self.queued
            .values()
            .flat_map(|m| m.values())
            .map(|s| s.tx.encode().len())
            .sum()
    }

    /// Observe the account's committed nonce (`types.nonce`) and drop stale txs.
    pub fn observe_account(&mut self, addr: Address, nonce: Nonce) {
        self.account_nonce.insert(addr, nonce);
        if let Some(m) = self.queued.get_mut(&addr) {
            m.retain(|n, _| *n >= nonce);
            if m.is_empty() {
                self.queued.remove(&addr);
            }
        }
    }

    fn next_nonce(&self, addr: &Address) -> Nonce {
        self.account_nonce.get(addr).copied().unwrap_or(Nonce::ZERO)
    }

    /// Ready tx for an account: exact next `types.nonce`, if queued.
    pub fn ready_for(&self, addr: &Address) -> Option<&SignedTx> {
        let n = self.next_nonce(addr);
        self.queued.get(addr).and_then(|m| m.get(&n))
    }

    /// Insert after `mempool.verify`. Future nonces wait in the queue.
    pub fn queue_insert(
        &mut self,
        signed: SignedTx,
        account: &Account,
    ) -> Result<Address, VerifyError> {
        verify(&signed, account)?;
        let addr = sender_address(&signed)?;
        self.observe_account(addr, account.nonce);
        let nonce = signed.tx.nonce;
        self.queued.entry(addr).or_default().insert(nonce, signed);
        Ok(addr)
    }

    pub(crate) fn remove(&mut self, addr: &Address, nonce: Nonce) -> Option<SignedTx> {
        let tx = self.queued.get_mut(addr)?.remove(&nonce);
        if self.queued.get(addr).is_some_and(Map::is_empty) {
            self.queued.remove(addr);
        }
        tx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mempool as Pool;
    use crypto::address::from_ed25519;
    use crypto::sig::ed25519::SecretKey;
    use crypto::tx::sign;
    use types::{Amount, ChainId, Hash, ParamsRegistry, GAS_TRANSFER};

    fn sk(b: u8) -> SecretKey {
        SecretKey::from_bytes(&[b; 32])
    }

    #[test]
    fn out_of_order_nonce_ready_is_lower_first() {
        let ska = sk(4);
        let from = from_ed25519(&ska.verifying_key());
        let account = Account {
            balance: Amount::new(10_000),
            nonce: Nonce(4),
            code_hash: Hash::ZERO,
        };
        let mut pool = Pool::new(&ParamsRegistry::new());
        let mk = |n: u64| {
            sign(
                &ska,
                types::tx::Tx::transfer(
                    ChainId::new(1),
                    Nonce(n),
                    GAS_TRANSFER,
                    Amount::new(1),
                    types::Address::ZERO,
                    Amount::new(1),
                ),
            )
        };
        pool.queue_insert(mk(5), &account).unwrap();
        assert!(pool.ready_for(&from).is_none());
        pool.queue_insert(mk(4), &account).unwrap();
        let ready = pool.ready_for(&from).unwrap();
        assert_eq!(ready.tx.nonce, Nonce(4));
    }
}
