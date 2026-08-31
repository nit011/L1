//! Shared load harness (`stress.load_harness`).
//!
//! Spins Tier 18 [`iac.docker_compose`] (`infra/docker-compose.yml`) and signs
//! transactions the same way as Tier 16 `sdk.e2e_integration_test` /
//! [`sdk::sign_tx`]. The multiprocess `node` binary has **no JSON-RPC** (rpc
//! already depends on `node`); live load is injected on `gossip` `TOPIC_TX`,
//! which is what `crates/node/src/main.rs` actually admits via `wire_mempool`.
//!
//! Validator count: compose is fixed at 4 (same as `mvp.finality_lan`).
//! `LoadConfig::n_validators > 4` is recorded but not launched — extra
//! validators need more compose services (finding, not a silent mock).
//!
//! Overhead: [`sign_burst_tps`] is the harness sign-only rate (no network).

use crypto::sig::ed25519::SecretKey;
use iac::{materialize_with_bank, DEVNET_CHAIN_ID};
use libp2p::futures::StreamExt;
use libp2p::swarm::SwarmEvent;
use network::codec::{encode_frame, GossipKind};
use network::gossip::{ident_topic, mesh_swarm, TOPIC_TX};
use network::identity;
use network::transport::quic_listen_local;
use sdk::sign_tx;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};
use storage::codec::encode_signed_tx;
use types::tx::Tx;
use types::{Amount, ChainId, Nonce, GAS_TRANSFER, MIN_TX_FEE};

/// How the harness is pointed at compose.
pub const COMPOSE_FILE: &str = "infra/docker-compose.yml";
/// Host UDP publish overlay.
pub const COMPOSE_OVERRIDE: &str = "tests/stress/compose.override.yml";
/// First published QUIC port (node0).
pub const HOST_QUIC_BASE: u16 = 14001;

/// Load knobs for the other five contracts.
#[derive(Clone, Debug)]
pub struct LoadConfig {
    /// Desired validators (compose implements 4).
    pub n_validators: usize,
    /// Genesis bank accounts (ed25519).
    pub bank_accounts: usize,
    /// How long to keep generating load.
    pub duration: Duration,
    /// Signed txs per burst.
    pub tx_burst: usize,
}

impl Default for LoadConfig {
    fn default() -> Self {
        Self {
            n_validators: 4,
            bank_accounts: 32,
            duration: Duration::from_secs(20),
            tx_burst: 64,
        }
    }
}

/// Repo root (workspace).
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// `true` when the Docker daemon answers.
pub fn docker_ok() -> bool {
    Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Compose invocation against the Tier 18 file + stress port overlay.
pub fn compose_cmd() -> Command {
    let root = repo_root();
    let mut c = Command::new("docker");
    c.current_dir(&root).args([
        "compose",
        "-p",
        "l1stress",
        "-f",
        COMPOSE_FILE,
        "-f",
        COMPOSE_OVERRIDE,
    ]);
    c
}

/// Materialize funded genesis into `infra/data` and `docker compose up`.
pub fn bring_up(cfg: &LoadConfig) -> std::io::Result<(types::Hash, Vec<SecretKey>)> {
    if cfg.n_validators != 4 {
        eprintln!(
            "stress.load_harness: n_validators={} but iac.docker_compose is 4 services",
            cfg.n_validators
        );
    }
    let root = repo_root();
    let data = root.join("infra/data");
    let (hash, bank) = materialize_with_bank(&data, cfg.bank_accounts)?;
    // Drop leftover runtime files so COMMIT/tip metrics are from this run only.
    for name in ["node0", "node1", "node2", "node3", "joiner"] {
        let dir = data.join(name);
        let _ = std::fs::remove_file(dir.join("events.log"));
        let _ = std::fs::remove_file(dir.join("tip"));
        let _ = std::fs::remove_file(dir.join("listen"));
    }
    let st = compose_cmd().args(["up", "-d", "--no-build"]).status()?;
    if !st.success() {
        return Err(std::io::Error::other("docker compose up failed"));
    }
    std::thread::sleep(Duration::from_secs(3));
    Ok((hash, bank))
}

/// `docker compose down`.
pub fn tear_down() {
    let _ = compose_cmd().args(["--profile", "join", "down"]).status();
}

/// Tip height from a bind-mounted node dir (`COMMIT` / `cons.commit`).
pub fn read_tip(node_i: usize) -> Option<u64> {
    let p = repo_root()
        .join("infra/data")
        .join(format!("node{node_i}"))
        .join("tip");
    let s = std::fs::read_to_string(p).ok()?;
    s.lines().next()?.parse().ok()
}

/// `events.log` for node i (includes `COMMIT n` from wire_commit / cons.commit).
pub fn events(node_i: usize) -> String {
    std::fs::read_to_string(
        repo_root()
            .join("infra/data")
            .join(format!("node{node_i}"))
            .join("events.log"),
    )
    .unwrap_or_default()
}

/// Sign-only throughput of the harness (no gossip, no consensus).
pub fn sign_burst_tps(n: usize) -> (f64, Duration) {
    let sk = SecretKey::from_bytes(&[0x11u8; 32]);
    let t0 = Instant::now();
    for i in 0..n {
        let tx = Tx::transfer(
            ChainId::new(DEVNET_CHAIN_ID),
            Nonce(i as u64),
            GAS_TRANSFER,
            Amount::new(MIN_TX_FEE),
            types::Address::ZERO,
            Amount::new(1),
        );
        let _ = sign_tx(&sk, tx);
    }
    let dt = t0.elapsed();
    let tps = n as f64 / dt.as_secs_f64().max(1e-9);
    (tps, dt)
}

fn bank_seed(i: usize) -> [u8; 32] {
    let mut s = [0xBAu8; 32];
    s[0] = 0x18;
    s[1] = (i / 256) as u8;
    s[2] = (i % 256) as u8;
    s
}

/// Deterministic bank key matching `iac::materialize_with_bank`.
pub fn bank_sk(i: usize) -> SecretKey {
    SecretKey::from_bytes(&bank_seed(i))
}

/// One transfer signed via [`sdk::sign_tx`] (e2e pattern).
pub fn signed_transfer(
    sk: &SecretKey,
    nonce: u64,
    to: types::Address,
    amount: u128,
) -> sdk::SignedFrom {
    let tx = Tx::transfer(
        ChainId::new(DEVNET_CHAIN_ID),
        Nonce(nonce),
        GAS_TRANSFER,
        Amount::new(MIN_TX_FEE),
        to,
        Amount::new(amount),
    );
    sign_tx(sk, tx)
}

/// Publish signed txs onto node0's gossip (live compose), same wire as the node.
pub async fn gossip_txs(signed: &[sdk::SignedFrom]) -> usize {
    let id = identity::generate().unwrap();
    let mut swarm = mesh_swarm(id, &network::discovery::BootstrapList::new()).unwrap();
    swarm.listen_on(quic_listen_local()).unwrap();
    loop {
        match swarm.next().await {
            Some(SwarmEvent::NewListenAddr { .. }) => break,
            Some(_) => {}
            None => return 0,
        }
    }
    let dial: libp2p::Multiaddr = format!("/ip4/127.0.0.1/udp/{HOST_QUIC_BASE}/quic-v1")
        .parse()
        .unwrap();
    let _ = swarm.dial(dial);
    for _ in 0..12 {
        let _ = tokio::time::timeout(Duration::from_millis(25), swarm.next()).await;
    }
    let mut ok = 0usize;
    for s in signed {
        let inner = encode_signed_tx(&s.signed);
        let frame = encode_frame(GossipKind::Tx, &inner);
        if swarm
            .behaviour_mut()
            .gossipsub
            .publish(ident_topic(TOPIC_TX), frame)
            .is_ok()
        {
            ok += 1;
        }
    }
    for _ in 0..8 {
        let _ = tokio::time::timeout(Duration::from_millis(25), swarm.next()).await;
    }
    ok
}

/// Percentile on a sorted slice (nearest-rank).
pub fn percentile(sorted: &[u128], p: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Poll bind-mounted tips until `min_h` or timeout.
pub fn wait_tip(node_i: usize, min_h: u64, timeout: Duration) -> Option<u64> {
    let t0 = Instant::now();
    loop {
        if let Some(h) = read_tip(node_i) {
            if h >= min_h {
                return Some(h);
            }
        }
        if t0.elapsed() >= timeout {
            return read_tip(node_i);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_files_exist_and_reference_dockerfile() {
        let root = repo_root();
        let yml = std::fs::read_to_string(root.join(COMPOSE_FILE)).unwrap();
        assert!(yml.contains("dockerfile: infra/Dockerfile") || yml.contains("NodeConfig"));
        assert!(root.join(COMPOSE_OVERRIDE).is_file());
        let _ = std::path::Path::new(COMPOSE_FILE);
    }

    #[test]
    fn sign_overhead_is_measurable() {
        let (tps, dt) = sign_burst_tps(2_000);
        eprintln!("stress.load_harness sign-only 2000 txs: {tps:.0} tps dt={dt:?}");
        assert!(
            tps > 100.0,
            "harness itself should sign well above consensus TPS"
        );
        assert!(dt < Duration::from_secs(5));
    }

    #[test]
    fn sdk_sign_matches_e2e_pattern() {
        let sk = bank_sk(0);
        let s = signed_transfer(&sk, 0, types::Address::ZERO, 1);
        crypto::tx::verify_ed25519(&s.signed).unwrap();
    }

    #[test]
    #[ignore]
    fn docker_compose_comes_up() {
        assert!(
            crate::harness::docker_ok(),
            "docker required for iac.docker_compose"
        );
        let cfg = LoadConfig {
            duration: Duration::from_secs(1),
            bank_accounts: 2,
            tx_burst: 1,
            ..LoadConfig::default()
        };
        let (hash, bank) = bring_up(&cfg).expect("compose up");
        let tip = wait_tip(0, 0, Duration::from_secs(40));
        eprintln!(
            "stress.load_harness genesis={} bank={} tip={tip:?}",
            hex::encode(hash.as_bytes()),
            bank.len()
        );
        tear_down();
        assert!(tip.is_some(), "compose node0 must write tip (cons.commit)");
    }
}
