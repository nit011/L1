//! Local plaintext mempool (architecture.md §5). No gossip (Tier 6), no
//! encrypted mempool (Tier 20), no EIP-1559 (development-plan.md).

pub mod fees;
pub mod limits;
pub mod order;
pub mod queue;
pub mod rbf;
pub mod verify;

pub use queue::Mempool;
pub use verify::{sender_address, verify, VerifyError};

use crate::rbf::rbf_allowed;
use state::account::Account;
use types::tx::SignedTx;

impl Mempool {
    /// Admit a tx: size, min fee, Tier 3 verify, RBF, occupancy. Contract glue
    /// for `mempool.verify` / `rbf` / `size_limits` / `min_fee` / `nonce_queue`.
    pub fn insert(&mut self, signed: SignedTx, account: &Account) -> Result<(), VerifyError> {
        crate::limits::check_tx_bytes(&signed)?;
        crate::fees::check_min_fee(&signed.tx, self.min_fee)?;
        crate::verify::verify(&signed, account)?;
        let addr = sender_address(&signed)?;
        self.observe_account(addr, account.nonce);
        if let Some(old) = self
            .queued
            .get(&addr)
            .and_then(|m| m.get(&signed.tx.nonce))
            .cloned()
        {
            rbf_allowed(&old, &signed)?;
            self.queued
                .get_mut(&addr)
                .expect("addr present")
                .insert(signed.tx.nonce, signed);
            return Ok(());
        }
        crate::limits::ensure_room(self, &signed)?;
        self.queued
            .entry(addr)
            .or_default()
            .insert(signed.tx.nonce, signed);
        Ok(())
    }

    /// Enqueue a tx that already passed `gossip.tx` / `mempool.verify`.
    ///
    /// Does **not** call `verify` again (`node.wire.mempool`). Still uses the
    /// same `queued` map that `mempool.fee_order` (`take_ready`) reads.
    pub fn admit_preverified(&mut self, signed: SignedTx, addr: types::Address) {
        self.queued
            .entry(addr)
            .or_default()
            .insert(signed.tx.nonce, signed);
    }
}
