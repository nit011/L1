//! Node runtime configuration (architecture.md §5 / development-plan.md Devnet MVP).
//!
//! Assembled once from Tier 3 `genesis.params` and Tier 6 `p2p.bootstrap`.
//! Wire functions take [`NodeConfig`] rather than reading files themselves.

use network::discovery::BootstrapList;
use network::identity::{self, NodeIdentity};
use std::path::{Path, PathBuf};
use thiserror::Error;
use types::encoding::{decode, encode};
use types::genesis::{Genesis, GenesisAccount, GenesisParams, GenesisTimeouts};
use types::{
    Address, Amount, ChainId, Hash, Nonce, ParamId, ParamsRegistry, ValidatorId, VotingPower,
};

/// Full node configuration. Contract: `node.config`.
#[derive(Clone)]
pub struct NodeConfig {
    /// Chain genesis (`genesis.params` lives on [`Genesis::params`]).
    pub genesis: Genesis,
    /// Kademlia / dial list (`p2p.bootstrap`).
    pub bootstrap: BootstrapList,
    /// This process's libp2p identity (`p2p.identity`).
    pub identity: NodeIdentity,
    /// Data directory (listen addr, tip, event log).
    pub data_dir: PathBuf,
    /// Minimum milliseconds between committed blocks (architecture.md §10).
    pub min_block_time_ms: u64,
}

impl NodeConfig {
    /// Build from genesis + bootstrap. Contract: `node.config`.
    pub fn new(
        genesis: Genesis,
        bootstrap: BootstrapList,
        identity: NodeIdentity,
        data_dir: PathBuf,
    ) -> Self {
        let _params = &genesis.params;
        Self {
            genesis,
            bootstrap,
            identity,
            data_dir,
            min_block_time_ms: 1_000,
        }
    }
}

/// Config I/O errors.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Codec.
    #[error("codec")]
    Codec,
    /// Filesystem.
    #[error("io: {0}")]
    Io(String),
    /// Identity.
    #[error("identity")]
    Identity,
}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

fn param_from_byte(b: u8) -> Option<ParamId> {
    Some(match b {
        0 => ParamId::MaxBlockBytes,
        1 => ParamId::MaxTxBytes,
        2 => ParamId::MaxGas,
        3 => ParamId::EpochLength,
        4 => ParamId::UnbondingPeriod,
        5 => ParamId::TimeoutProposeMs,
        6 => ParamId::TimeoutPrevoteMs,
        7 => ParamId::TimeoutPrecommitMs,
        8 => ParamId::TimeoutDeltaMs,
        9 => ParamId::MaxTimestampDriftMs,
        10 => ParamId::MinSelfBond,
        11 => ParamId::DelegationCap,
        12 => ParamId::SlashPercent,
        _ => return None,
    })
}

/// Canonical genesis bytes (same field order as `genesis.hash` preimage).
pub fn encode_genesis(g: &Genesis) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&g.chain_id.0.to_be_bytes());
    p.extend_from_slice(&(g.alloc.len() as u32).to_be_bytes());
    for (addr, a) in &g.alloc {
        p.extend_from_slice(addr.as_bytes());
        p.extend_from_slice(&a.balance.0.to_be_bytes());
        p.extend_from_slice(&a.nonce.0.to_be_bytes());
        p.extend_from_slice(a.code_hash.as_bytes());
    }
    p.extend_from_slice(&(g.validators.len() as u32).to_be_bytes());
    for (id, power) in &g.validators {
        p.extend_from_slice(id.as_bytes());
        p.extend_from_slice(&power.0.to_be_bytes());
    }
    let params: Vec<_> = g.params.registry.iter().collect();
    p.extend_from_slice(&(params.len() as u32).to_be_bytes());
    for (id, v) in params {
        p.push(match id {
            ParamId::MaxBlockBytes => 0,
            ParamId::MaxTxBytes => 1,
            ParamId::MaxGas => 2,
            ParamId::EpochLength => 3,
            ParamId::UnbondingPeriod => 4,
            ParamId::TimeoutProposeMs => 5,
            ParamId::TimeoutPrevoteMs => 6,
            ParamId::TimeoutPrecommitMs => 7,
            ParamId::TimeoutDeltaMs => 8,
            ParamId::MaxTimestampDriftMs => 9,
            ParamId::MinSelfBond => 10,
            ParamId::DelegationCap => 11,
            ParamId::SlashPercent => 12,
        });
        p.extend_from_slice(&v.to_be_bytes());
    }
    p.extend_from_slice(&g.params.timeouts.propose_ms.to_be_bytes());
    p.extend_from_slice(&g.params.timeouts.prevote_ms.to_be_bytes());
    p.extend_from_slice(&g.params.timeouts.precommit_ms.to_be_bytes());
    p.extend_from_slice(&g.params.timeouts.delta_ms.to_be_bytes());
    encode(&p)
}

/// Inverse of [`encode_genesis`].
pub fn decode_genesis(buf: &[u8]) -> Result<Genesis, ConfigError> {
    let p = decode(buf).map_err(|_| ConfigError::Codec)?;
    if p.len() < 8 + 4 {
        return Err(ConfigError::Codec);
    }
    let mut i = 0usize;
    let take = |i: &mut usize, n: usize| -> Result<&[u8], ConfigError> {
        if *i + n > p.len() {
            return Err(ConfigError::Codec);
        }
        let s = &p[*i..*i + n];
        *i += n;
        Ok(s)
    };
    let chain_id = ChainId::new(u64::from_be_bytes(take(&mut i, 8)?.try_into().unwrap()));
    let n_alloc = u32::from_be_bytes(take(&mut i, 4)?.try_into().unwrap()) as usize;
    let mut g = Genesis::new(chain_id);
    for _ in 0..n_alloc {
        let addr = Address::from_bytes(take(&mut i, 32)?.try_into().unwrap());
        let balance = Amount(u128::from_be_bytes(take(&mut i, 16)?.try_into().unwrap()));
        let nonce = Nonce(u64::from_be_bytes(take(&mut i, 8)?.try_into().unwrap()));
        let code_hash = Hash::from_bytes(take(&mut i, 32)?.try_into().unwrap());
        g.insert_alloc(
            addr,
            GenesisAccount {
                balance,
                nonce,
                code_hash,
            },
        );
    }
    let n_val = u32::from_be_bytes(take(&mut i, 4)?.try_into().unwrap()) as usize;
    for _ in 0..n_val {
        let id = ValidatorId::from_bytes(take(&mut i, 48)?.try_into().unwrap());
        let power = VotingPower(u64::from_be_bytes(take(&mut i, 8)?.try_into().unwrap()));
        g.insert_validator(id, power);
    }
    let n_p = u32::from_be_bytes(take(&mut i, 4)?.try_into().unwrap()) as usize;
    let mut registry = ParamsRegistry::new();
    for _ in 0..n_p {
        let id = param_from_byte(take(&mut i, 1)?[0]).ok_or(ConfigError::Codec)?;
        let v = u64::from_be_bytes(take(&mut i, 8)?.try_into().unwrap());
        registry.set(id, v);
    }
    let propose_ms = u64::from_be_bytes(take(&mut i, 8)?.try_into().unwrap());
    let prevote_ms = u64::from_be_bytes(take(&mut i, 8)?.try_into().unwrap());
    let precommit_ms = u64::from_be_bytes(take(&mut i, 8)?.try_into().unwrap());
    let delta_ms = u64::from_be_bytes(take(&mut i, 8)?.try_into().unwrap());
    if i != p.len() {
        return Err(ConfigError::Codec);
    }
    g.params = GenesisParams {
        registry,
        timeouts: GenesisTimeouts {
            propose_ms,
            prevote_ms,
            precommit_ms,
            delta_ms,
        },
    };
    Ok(g)
}

/// Persist config files under `dir`.
pub fn write_dir(
    dir: &Path,
    cfg: &NodeConfig,
    bls_sk: &[u8; 32],
    vrf_sk: &[u8; 32],
) -> Result<(), ConfigError> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("genesis.bin"), encode_genesis(&cfg.genesis))?;
    std::fs::write(dir.join("ed25519"), cfg.identity.ed25519.to_bytes())?;
    std::fs::write(dir.join("bls_sk"), bls_sk)?;
    std::fs::write(dir.join("vrf_sk"), vrf_sk)?;
    let mut boot = Vec::new();
    boot.extend_from_slice(&(cfg.bootstrap.peers.len() as u32).to_be_bytes());
    for (peer, addr) in &cfg.bootstrap.peers {
        let pb = peer.to_bytes();
        boot.extend_from_slice(&(pb.len() as u32).to_be_bytes());
        boot.extend_from_slice(&pb);
        let a = addr.to_string();
        boot.extend_from_slice(&(a.len() as u32).to_be_bytes());
        boot.extend_from_slice(a.as_bytes());
    }
    std::fs::write(dir.join("bootstrap.bin"), boot)?;
    Ok(())
}

/// Rewrite `bootstrap.bin` so a running node can dial peers discovered after start.
pub fn write_bootstrap(dir: &Path, bootstrap: &BootstrapList) -> Result<(), ConfigError> {
    let mut boot = Vec::new();
    boot.extend_from_slice(&(bootstrap.peers.len() as u32).to_be_bytes());
    for (peer, addr) in &bootstrap.peers {
        let pb = peer.to_bytes();
        boot.extend_from_slice(&(pb.len() as u32).to_be_bytes());
        boot.extend_from_slice(&pb);
        let a = addr.to_string();
        boot.extend_from_slice(&(a.len() as u32).to_be_bytes());
        boot.extend_from_slice(a.as_bytes());
    }
    std::fs::write(dir.join("bootstrap.bin"), boot)?;
    Ok(())
}

/// Sorted VRF public keys for `cons.propose` / `cons.prevote_step`.
pub fn write_vrf_pks(
    dir: &Path,
    pks: &types::collections::Map<ValidatorId, [u8; 32]>,
) -> Result<(), ConfigError> {
    let mut b = Vec::new();
    for (id, pk) in pks {
        b.extend_from_slice(id.as_bytes());
        b.extend_from_slice(pk);
    }
    std::fs::write(dir.join("vrf_pks.bin"), b)?;
    Ok(())
}

/// All validators' VRF secrets (static genesis set; used so the round VRF
/// source's proof can be built on every process without a separate proof topic).
pub fn write_vrf_secrets(
    dir: &Path,
    secrets: &types::collections::Map<ValidatorId, [u8; 32]>,
) -> Result<(), ConfigError> {
    let mut b = Vec::new();
    for (id, sk) in secrets {
        b.extend_from_slice(id.as_bytes());
        b.extend_from_slice(sk);
    }
    std::fs::write(dir.join("vrf_secrets.bin"), b)?;
    Ok(())
}

/// Load [`NodeConfig`] from `dir` (written by [`write_dir`]).
pub fn load_dir(dir: &Path) -> Result<(NodeConfig, [u8; 32], [u8; 32]), ConfigError> {
    let genesis = decode_genesis(&std::fs::read(dir.join("genesis.bin"))?)?;
    let ed = std::fs::read(dir.join("ed25519"))?;
    if ed.len() != 32 {
        return Err(ConfigError::Identity);
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&ed);
    let sk = crypto::ed25519::SecretKey::from_bytes(&seed);
    let identity = identity::from_ed25519_secret(sk).map_err(|_| ConfigError::Identity)?;
    let mut bls = [0u8; 32];
    bls.copy_from_slice(&std::fs::read(dir.join("bls_sk"))?);
    let mut vrf = [0u8; 32];
    vrf.copy_from_slice(&std::fs::read(dir.join("vrf_sk"))?);
    let boot_bytes = std::fs::read(dir.join("bootstrap.bin")).unwrap_or_default();
    let mut bootstrap = BootstrapList::new();
    if boot_bytes.len() >= 4 {
        let n = u32::from_be_bytes(boot_bytes[0..4].try_into().unwrap()) as usize;
        let mut i = 4usize;
        for _ in 0..n {
            if i + 4 > boot_bytes.len() {
                break;
            }
            let plen = u32::from_be_bytes(boot_bytes[i..i + 4].try_into().unwrap()) as usize;
            i += 4;
            if i + plen > boot_bytes.len() {
                break;
            }
            let peer = libp2p::PeerId::from_bytes(&boot_bytes[i..i + plen])
                .map_err(|_| ConfigError::Codec)?;
            i += plen;
            if i + 4 > boot_bytes.len() {
                break;
            }
            let alen = u32::from_be_bytes(boot_bytes[i..i + 4].try_into().unwrap()) as usize;
            i += 4;
            if i + alen > boot_bytes.len() {
                break;
            }
            let addr_s =
                std::str::from_utf8(&boot_bytes[i..i + alen]).map_err(|_| ConfigError::Codec)?;
            i += alen;
            let addr: libp2p::Multiaddr = addr_s.parse().map_err(|_| ConfigError::Codec)?;
            bootstrap.insert(peer, addr);
        }
    }
    let cfg = NodeConfig::new(genesis, bootstrap, identity, dir.to_path_buf());
    Ok((cfg, bls, vrf))
}

/// Load only `bootstrap.bin` (running nodes re-dial).
pub fn load_bootstrap(dir: &Path) -> Result<BootstrapList, ConfigError> {
    let boot_bytes = std::fs::read(dir.join("bootstrap.bin")).unwrap_or_default();
    let mut bootstrap = BootstrapList::new();
    if boot_bytes.len() < 4 {
        return Ok(bootstrap);
    }
    let n = u32::from_be_bytes(boot_bytes[0..4].try_into().unwrap()) as usize;
    let mut i = 4usize;
    for _ in 0..n {
        if i + 4 > boot_bytes.len() {
            break;
        }
        let plen = u32::from_be_bytes(boot_bytes[i..i + 4].try_into().unwrap()) as usize;
        i += 4;
        if i + plen > boot_bytes.len() {
            break;
        }
        let peer =
            libp2p::PeerId::from_bytes(&boot_bytes[i..i + plen]).map_err(|_| ConfigError::Codec)?;
        i += plen;
        if i + 4 > boot_bytes.len() {
            break;
        }
        let alen = u32::from_be_bytes(boot_bytes[i..i + 4].try_into().unwrap()) as usize;
        i += 4;
        if i + alen > boot_bytes.len() {
            break;
        }
        let addr_s =
            std::str::from_utf8(&boot_bytes[i..i + alen]).map_err(|_| ConfigError::Codec)?;
        i += alen;
        let addr: libp2p::Multiaddr = addr_s.parse().map_err(|_| ConfigError::Codec)?;
        bootstrap.insert(peer, addr);
    }
    Ok(bootstrap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use network::transport::quic_listen_local;
    use types::{ChainId, VotingPower};

    #[test]
    fn genesis_params_round_trip() {
        let mut g = Genesis::new(ChainId::new(3));
        g.insert_validator(ValidatorId::from_bytes([1u8; 48]), VotingPower(1));
        g.params.timeouts.propose_ms = 1_000;
        let buf = encode_genesis(&g);
        let d = decode_genesis(&buf).unwrap();
        assert_eq!(g.hash(), d.hash());
        assert_eq!(d.params.timeouts.propose_ms, 1_000);
        let _ = g.params.registry.get(ParamId::MaxGas);
    }

    #[test]
    fn config_includes_bootstrap_and_params() {
        let g = Genesis::new(ChainId::new(1));
        let id = identity::generate().unwrap();
        let mut boot = BootstrapList::new();
        boot.insert(id.peer_id, quic_listen_local());
        let cfg = NodeConfig::new(g, boot, id, PathBuf::from("/tmp/l1-unused"));
        assert_eq!(cfg.bootstrap.peers.len(), 1);
        assert!(cfg.genesis.params.registry.get(ParamId::MaxGas).is_some());
    }

    #[test]
    fn decode_rejects_truncated() {
        assert!(decode_genesis(&[1, 2, 3]).is_err());
    }
}
