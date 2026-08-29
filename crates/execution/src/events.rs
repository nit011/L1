//! Execution events (architecture.md §3).

use types::{Address, Amount};

/// Events emitted by `apply_tx`. Contract: `exec.events`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Native transfer.
    Transfer {
        /// Sender.
        from: Address,
        /// Recipient.
        to: Address,
        /// Amount.
        amount: Amount,
    },
    /// Staking mutation (`tx.stake.*`).
    Stake {
        /// Account that signed the envelope.
        from: Address,
        /// Amount moved in the staking ledger.
        amount: Amount,
    },
}
