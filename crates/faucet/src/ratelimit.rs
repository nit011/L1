//! Per-address / per-IP faucet rate limits (devnet/testnet).
//!
//! # Example
//!
//! ```no_run
//! use std::time::Duration;
//! use faucet::ratelimit::RateLimitedFaucet;
//! use faucet::service::Faucet;
//! # fn demo(inner: &mut rpc::server::RpcInner, faucet: Faucet, addr: types::Address) {
//! let mut gated = RateLimitedFaucet::new(faucet, 1, Duration::from_secs(60));
//! gated.drip(inner, addr, "127.0.0.1").unwrap();
//! # }
//! ```
//!
//! Calls [`crate::service::Faucet::drip`] (`faucet.service`) only when the
//! source is under the configured cap. Contract: `faucet.ratelimit`.

use crate::service::{Faucet, FaucetError};
use rpc::server::RpcInner;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use types::Address;
use types::Hash;

/// Rate-limit errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitError {
    /// Too many drips from this address or IP inside the window.
    Throttled,
    /// Underlying faucet / RPC.
    Faucet(FaucetError),
}

/// Windowed counter per key (address hex or IP string).
pub struct RateLimitedFaucet {
    faucet: Faucet,
    /// Max successful drips per key per `window`.
    pub max_per_window: u32,
    /// Sliding window.
    pub window: Duration,
    hits: BTreeMap<String, Vec<Instant>>,
}

impl RateLimitedFaucet {
    /// Wrap a faucet.
    pub fn new(faucet: Faucet, max_per_window: u32, window: Duration) -> Self {
        Self {
            faucet,
            max_per_window,
            window,
            hits: BTreeMap::new(),
        }
    }

    pub(crate) fn admit(&mut self, key: &str, now: Instant) -> bool {
        let win = self.window;
        let e = self.hits.entry(key.to_string()).or_default();
        e.retain(|t| now.duration_since(*t) < win);
        if e.len() as u32 >= self.max_per_window {
            return false;
        }
        e.push(now);
        true
    }

    /// Drip if neither `to` nor `ip` is over quota, then [`Faucet::drip`].
    pub fn drip(
        &mut self,
        inner: &mut RpcInner,
        to: Address,
        ip: &str,
    ) -> Result<Hash, RateLimitError> {
        let now = Instant::now();
        let addr_key = format!("a:{}", encode_addr(&to));
        let ip_key = format!("ip:{ip}");
        if !self.admit(&addr_key, now) || !self.admit(&ip_key, now) {
            // roll back the other key if we inserted ip after failing... admit is sequential.
            return Err(RateLimitError::Throttled);
        }
        self.faucet.drip(inner, to).map_err(RateLimitError::Faucet)
    }
}

fn encode_addr(a: &Address) -> String {
    a.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Faucet;
    use crypto::address::from_ed25519;
    use crypto::sig::ed25519::keygen;
    use network::discovery::BootstrapList;
    use network::identity;
    use node::config::NodeConfig;
    use types::genesis::{Genesis, GenesisAccount};
    use types::{Amount, ChainId, Nonce};

    fn inner_and_faucet() -> (rpc::server::RpcInner, RateLimitedFaucet, Address) {
        let faucet_sk = keygen();
        let dest = from_ed25519(&keygen().verifying_key());
        let mut g = Genesis::new(ChainId::new(1));
        g.insert_alloc(
            from_ed25519(&faucet_sk.verifying_key()),
            GenesisAccount {
                balance: Amount::new(1_000_000),
                nonce: Nonce::ZERO,
                code_hash: types::Hash::ZERO,
            },
        );
        let inner = rpc::server::RpcInner::from_config(NodeConfig::new(
            g,
            BootstrapList::new(),
            identity::generate().unwrap(),
            std::path::PathBuf::from("/tmp/faucet-rl"),
        ));
        let faucet = Faucet::new(faucet_sk, ChainId::new(1), Amount::new(1));
        let gated = RateLimitedFaucet::new(faucet, 1, Duration::from_millis(80));
        (inner, gated, dest)
    }

    #[test]
    fn second_rapid_request_is_throttled() {
        let (mut inner, mut gated, dest) = inner_and_faucet();
        gated.drip(&mut inner, dest, "10.0.0.1").unwrap();
        let err = gated.drip(&mut inner, dest, "10.0.0.1").unwrap_err();
        assert_eq!(err, RateLimitError::Throttled);
    }

    #[test]
    fn after_window_limiter_allows_again() {
        let (mut inner, mut gated, dest) = inner_and_faucet();
        gated.drip(&mut inner, dest, "10.0.0.2").unwrap();
        std::thread::sleep(Duration::from_millis(90));
        let now = Instant::now();
        assert!(gated.admit("ip:10.0.0.2", now));
        assert!(gated.admit(&format!("a:{}", encode_addr(&dest)), now));
    }
}
