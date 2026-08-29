//! Global fee ordering on top of the nonce queue (architecture.md §5).
//!
//! Uses `tx.fee_priority` (Tier 3). Iteration is over `types.collections.Map`
//! so selection is deterministic across processes.

use crate::queue::Mempool;
use execution::builder::ReadyTxs;
use execution::fees::fee_priority;
use types::collections::{Map, Set};
use types::tx::SignedTx;
use types::{Address, Nonce};

impl Mempool {
    /// Highest `tx.fee_priority` among nonce-ready txs, ties by `Address`.
    pub fn peek_best_ready(
        &self,
        local_nonce: &Map<Address, Nonce>,
        skip: &Set<(Address, Nonce)>,
    ) -> Option<(Address, SignedTx)> {
        let mut best: Option<(u128, Address, SignedTx)> = None;
        for (addr, m) in &self.queued {
            let n = local_nonce.get(addr).copied().unwrap_or(Nonce::ZERO);
            let Some(tx) = m.get(&n) else { continue };
            if skip.contains(&(*addr, n)) {
                continue;
            }
            let Ok(p) = fee_priority(&tx.tx) else {
                continue;
            };
            let take = match &best {
                None => true,
                Some((bp, ba, _)) => p > *bp || (p == *bp && addr < ba),
            };
            if take {
                best = Some((p, *addr, tx.clone()));
            }
        }
        best.map(|(_, a, t)| (a, t))
    }
}

impl ReadyTxs for Mempool {
    fn take_ready(&mut self, max_gas: u64, max_bytes: u32) -> Vec<SignedTx> {
        let mut local = self.account_nonce.clone();
        let mut skip: Set<(Address, Nonce)> = Set::new();
        let mut out = Vec::new();
        let mut gas = 0u64;
        let mut bytes = 0u32;
        loop {
            let Some((addr, signed)) = self.peek_best_ready(&local, &skip) else {
                break;
            };
            let Ok(g) = execution::gas::gas_meter(&signed.tx) else {
                skip.insert((addr, signed.tx.nonce));
                continue;
            };
            let b = u32::try_from(signed.tx.encode().len()).unwrap_or(u32::MAX);
            if gas.saturating_add(g) > max_gas || bytes.saturating_add(b) > max_bytes {
                skip.insert((addr, signed.tx.nonce));
                continue;
            }
            let nonce = signed.tx.nonce;
            self.remove(&addr, nonce);
            local.insert(addr, nonce.checked_add(1).unwrap_or(nonce));
            gas = gas.saturating_add(g);
            bytes = bytes.saturating_add(b);
            out.push(signed);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::sender_address;
    use crypto::address::from_ed25519;
    use crypto::sig::ed25519::SecretKey;
    use crypto::tx::sign;
    use state::account::Account;
    use types::{Amount, ChainId, Hash, ParamsRegistry, GAS_TRANSFER};

    fn sk(b: u8) -> SecretKey {
        SecretKey::from_bytes(&[b; 32])
    }

    #[test]
    fn higher_fee_account_first_while_nonce_respected() {
        let ska = sk(5);
        let skb = sk(6);
        let a = from_ed25519(&ska.verifying_key());
        let b = from_ed25519(&skb.verifying_key());
        let acct = |bal| Account {
            balance: Amount::new(bal),
            nonce: Nonce::ZERO,
            code_hash: Hash::ZERO,
        };
        let mut pool = Mempool::new(&ParamsRegistry::new());
        let tx_a0 = sign(
            &ska,
            types::tx::Tx::transfer(
                ChainId::new(1),
                Nonce::ZERO,
                GAS_TRANSFER,
                Amount::new(1),
                types::Address::ZERO,
                Amount::new(1),
            ),
        );
        let tx_b0 = sign(
            &skb,
            types::tx::Tx::transfer(
                ChainId::new(1),
                Nonce::ZERO,
                GAS_TRANSFER,
                Amount::new(50),
                types::Address::ZERO,
                Amount::new(1),
            ),
        );
        let tx_a1 = sign(
            &ska,
            types::tx::Tx::transfer(
                ChainId::new(1),
                Nonce(1),
                GAS_TRANSFER,
                Amount::new(100),
                types::Address::ZERO,
                Amount::new(1),
            ),
        );
        pool.insert(tx_a0, &acct(10_000)).unwrap();
        pool.insert(tx_a1, &acct(10_000)).unwrap();
        pool.insert(tx_b0, &acct(10_000)).unwrap();
        let ready = pool.take_ready(u64::MAX, u32::MAX);
        assert_eq!(ready[0].tx.max_fee.0, 50);
        assert_eq!(sender_address(&ready[0]).unwrap(), b);
        assert_eq!(sender_address(&ready[1]).unwrap(), a);
        assert_eq!(ready[1].tx.nonce, Nonce::ZERO);
        assert_eq!(ready[2].tx.nonce, Nonce(1));
        assert_eq!(sender_address(&ready[2]).unwrap(), a);
    }
}
