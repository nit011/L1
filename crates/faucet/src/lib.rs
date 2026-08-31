//! Testnet faucet.

pub mod ratelimit;
pub mod service;

pub use ratelimit::RateLimitedFaucet;
pub use service::{Faucet, FaucetError};
