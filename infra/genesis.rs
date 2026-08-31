//! Shared genesis for every compose node (`iac.genesis_config`).
//!
//! One `Genesis` object is encoded once via [`node::config::encode_genesis`]
//! (same preimage as Tier 3 [`Genesis::hash`]). Those **identical bytes** are
//! written to `shared/genesis.bin` and copied into every node data dir.
//! Compose mounts `shared/` read-only as `/genesis` so containers cannot
//! independently regenerate genesis (which would put them on different chains).
//!
//! Depends on `iac.docker_compose` (hostnames `node0`…`node3` and UDP ports
//! match `infra/docker-compose.yml`) and `genesis.hash`.
//!
//! Usage:
//! ```text
//! cargo run -p iac --bin l1-genesis -- infra/data
//! ```

use crypto::address::from_ed25519;
use crypto::from_bls;
use crypto::sig::ed25519::SecretKey;
use crypto::vrf::public_key_from_seed;
use libp2p::Multiaddr;
use network::discovery::BootstrapList;
use network::identity::{self, NodeIdentity};
use node::config::{
    encode_genesis, write_bootstrap, write_dir, write_vrf_pks, write_vrf_secrets, NodeConfig,
};
use std::env;
use std::path::Path;
use types::collections::Map;
use types::genesis::{Genesis, GenesisAccount};
use types::{Amount, ChainId, Hash, Nonce, ValidatorId, VotingPower};

/// Compose IPv4 addresses (must match `infra/docker-compose.yml` ipam).
pub const NODE_IPS: [&str; 4] = ["172.28.0.10", "172.28.0.11", "172.28.0.12", "172.28.0.13"];
/// QUIC UDP ports inside the compose network (must match compose).
pub const NODE_PORTS: [u16; 4] = [4001, 4002, 4003, 4004];
/// Devnet chain id for this IaC bundle (distinct from simnet's 7).
pub const DEVNET_CHAIN_ID: u64 = 18;
/// Validator count — same N as Tier 7 simnet / `mvp.finality_lan`.
pub const N_VALIDATORS: usize = 4;

/// Materialize shared genesis + per-node `node.config` directories under `root`.
///
/// Layout:
/// - `shared/genesis.bin` — single canonical encoding
/// - `shared/genesis.hash` — hex of [`Genesis::hash`]
/// - `shared/vrf_pks.bin` / `vrf_secrets.bin`
/// - `node{i}/` — identity + bootstrap (`p2p.bootstrap` addressing)
pub fn materialize(root: &Path) -> std::io::Result<Hash> {
    materialize_with_bank(root, 0).map(|(h, _)| h)
}

/// Same as [`materialize`], plus `n_bank` deterministically keyed allocs
/// (for load tests). Default IaC still uses `n_bank = 0` so the published
/// genesis.hash is unchanged.
pub fn materialize_with_bank(
    root: &Path,
    n_bank: usize,
) -> std::io::Result<(Hash, Vec<SecretKey>)> {
    let shared = root.join("shared");
    std::fs::create_dir_all(&shared)?;

    let mut genesis = Genesis::new(ChainId::new(DEVNET_CHAIN_ID));
    let mut bank = Vec::with_capacity(n_bank);
    for i in 0..n_bank {
        let sk = SecretKey::from_bytes(&bank_seed(i));
        let addr = from_ed25519(&sk.verifying_key());
        genesis.insert_alloc(
            addr,
            GenesisAccount {
                balance: Amount::new(1_000_000_000),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        bank.push(sk);
    }
    let mut vrf_pks = Map::new();
    let mut vrf_secrets = Map::new();
    let mut identities: Vec<(NodeIdentity, [u8; 32], [u8; 32], ValidatorId)> = Vec::new();

    for i in 0..N_VALIDATORS {
        let bls_ikm = bls_ikm(i);
        let bls_sk =
            blst::min_pk::SecretKey::key_gen(&bls_ikm, &[]).expect("deterministic bls key_gen");
        let bls_bytes = bls_sk.to_bytes();
        let (vid, _) = from_bls(&bls_sk.sk_to_pk(), VotingPower(1));
        genesis.insert_validator(vid, VotingPower(1));

        let mut vrf_sk = [0u8; 32];
        vrf_sk[0] = (i as u8).saturating_add(1);
        vrf_sk[1] = 0x5a;
        let vrf_pk = public_key_from_seed(&vrf_sk);
        vrf_pks.insert(vid, vrf_pk);
        vrf_secrets.insert(vid, vrf_sk);

        let ed = SecretKey::from_bytes(&ed25519_seed(i));
        let id = identity::from_ed25519_secret(ed).expect("p2p.identity");
        identities.push((id, bls_bytes, vrf_sk, vid));
    }

    let genesis_bytes = encode_genesis(&genesis);
    let gh = genesis.hash();
    std::fs::write(shared.join("genesis.bin"), &genesis_bytes)?;
    std::fs::write(
        shared.join("genesis.hash"),
        hex::encode(gh.as_bytes()) + "\n",
    )?;

    write_vrf_pks(&shared, &vrf_pks).map_err(io_cfg)?;
    write_vrf_secrets(&shared, &vrf_secrets).map_err(io_cfg)?;

    let mut boot = BootstrapList::new();
    for (i, (id, _, _, _)) in identities.iter().enumerate() {
        let ma: Multiaddr = format!("/ip4/{}/udp/{}/quic-v1", NODE_IPS[i], NODE_PORTS[i])
            .parse()
            .expect("ip4 multiaddr");
        boot.insert(id.peer_id, ma);
    }

    for (i, (id, bls_bytes, vrf_sk, _)) in identities.iter().enumerate() {
        let dir = root.join(format!("node{i}"));
        std::fs::create_dir_all(&dir)?;
        let cfg = NodeConfig::new(genesis.clone(), boot.clone(), id.clone(), dir.clone());
        write_dir(&dir, &cfg, bls_bytes, vrf_sk).map_err(io_cfg)?;
        // Force the shared encoding so no per-node re-encode can diverge.
        std::fs::write(dir.join("genesis.bin"), &genesis_bytes)?;
        write_bootstrap(&dir, &boot).map_err(io_cfg)?;
        std::fs::copy(shared.join("vrf_pks.bin"), dir.join("vrf_pks.bin"))?;
        std::fs::copy(shared.join("vrf_secrets.bin"), dir.join("vrf_secrets.bin"))?;
        std::fs::copy(shared.join("genesis.hash"), dir.join("genesis.hash"))?;
    }

    // Late-join follower: same genesis bytes, not in the validator set.
    let join = root.join("joiner");
    std::fs::create_dir_all(&join)?;
    let join_id = identity::from_ed25519_secret(SecretKey::from_bytes(&ed25519_seed(9)))
        .expect("joiner identity");
    let join_bls = blst::min_pk::SecretKey::key_gen(&bls_ikm(9), &[])
        .expect("joiner bls")
        .to_bytes();
    let join_vrf = [9u8; 32];
    let cfg = NodeConfig::new(genesis.clone(), boot.clone(), join_id, join.clone());
    write_dir(&join, &cfg, &join_bls, &join_vrf).map_err(io_cfg)?;
    std::fs::write(join.join("genesis.bin"), &genesis_bytes)?;
    write_bootstrap(&join, &boot).map_err(io_cfg)?;
    std::fs::copy(shared.join("vrf_pks.bin"), join.join("vrf_pks.bin"))?;
    std::fs::copy(shared.join("vrf_secrets.bin"), join.join("vrf_secrets.bin"))?;
    std::fs::copy(shared.join("genesis.hash"), join.join("genesis.hash"))?;

    Ok((gh, bank))
}

/// After processes write `listen` files (127.0.0.1 QUIC), rewrite `bootstrap.bin`
/// with those multiaddrs (`p2p.bootstrap`) so host-run nodes can dial.
pub fn rewire_from_listen(root: &Path, n: usize) -> std::io::Result<()> {
    let mut boot = BootstrapList::new();
    for i in 0..n {
        let dir = root.join(format!("node{i}"));
        let listen = std::fs::read_to_string(dir.join("listen"))?;
        let addr: Multiaddr = listen
            .trim()
            .parse()
            .map_err(|e| std::io::Error::other(format!("listen: {e}")))?;
        let cfg = node::config::load_dir(&dir).map_err(io_cfg)?.0;
        boot.insert(cfg.identity.peer_id, addr);
    }
    for i in 0..n {
        write_bootstrap(&root.join(format!("node{i}")), &boot).map_err(io_cfg)?;
    }
    if root.join("joiner").is_dir() {
        write_bootstrap(&root.join("joiner"), &boot).map_err(io_cfg)?;
    }
    Ok(())
}

fn bls_ikm(i: usize) -> [u8; 32] {
    let mut ikm = [0xA1u8; 32];
    ikm[0] = 0x18;
    ikm[1] = i as u8;
    ikm
}

fn ed25519_seed(i: usize) -> [u8; 32] {
    let mut s = [0xEDu8; 32];
    s[0] = 0x18;
    s[1] = i as u8;
    s
}

fn bank_seed(i: usize) -> [u8; 32] {
    let mut s = [0xBAu8; 32];
    s[0] = 0x18;
    s[1] = (i / 256) as u8;
    s[2] = (i % 256) as u8;
    s
}

fn io_cfg(e: node::config::ConfigError) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

#[allow(dead_code)] // compiled as both `lib` (stress harness) and `l1-genesis` bin
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.get(1).map(String::as_str) == Some("rewire") {
        let root = args.get(2).expect("l1-genesis rewire DIR");
        rewire_from_listen(Path::new(root), N_VALIDATORS).expect("rewire");
        println!("rewired p2p.bootstrap from listen files in {root}");
        return;
    }
    let out = args.get(1).cloned().unwrap_or_else(|| "infra/data".into());
    let (h, _) = materialize_with_bank(Path::new(&out), 0).expect("materialize genesis");
    println!("genesis.hash={}", hex::encode(h.as_bytes()));
    println!("wrote {out}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use node::config::decode_genesis;

    fn tmp() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "l1-iac-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn same_source_same_genesis_hash_two_dirs() {
        let a = tmp().join("a");
        let b = tmp().join("b");
        let ha = materialize(&a).unwrap();
        let hb = materialize(&b).unwrap();
        assert_eq!(ha, hb);
        let ba = std::fs::read(a.join("shared/genesis.bin")).unwrap();
        let bb = std::fs::read(b.join("shared/genesis.bin")).unwrap();
        assert_eq!(ba, bb, "canonical genesis.bin must be byte-identical");
        for i in 0..N_VALIDATORS {
            let ni = std::fs::read(a.join(format!("node{i}/genesis.bin"))).unwrap();
            assert_eq!(ni, ba, "node{i} genesis.bin diverged from shared");
        }
        let decoded = decode_genesis(&ba).unwrap();
        assert_eq!(decoded.hash(), ha);
        assert_eq!(decoded.chain_id, ChainId::new(DEVNET_CHAIN_ID));
        assert_eq!(decoded.validators.len(), N_VALIDATORS);
    }

    #[test]
    fn genesis_hash_changes_if_alloc_added() {
        let root = tmp();
        let h = materialize(&root).unwrap();
        let mut g =
            decode_genesis(&std::fs::read(root.join("shared/genesis.bin")).unwrap()).unwrap();
        g.insert_alloc(
            types::Address::from_bytes([7u8; 32]),
            types::genesis::GenesisAccount {
                balance: types::Amount::new(1),
                nonce: types::Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        assert_ne!(g.hash(), h);
    }

    #[test]
    fn dockerfile_pins_toolchain_channel() {
        let docker = include_str!("Dockerfile");
        let tool = include_str!("../rust-toolchain.toml");
        assert!(tool.contains("1.93.0"), "tooling.rust_toolchain");
        assert!(
            docker.contains("1.93.0"),
            "Dockerfile must pin tooling.rust_toolchain"
        );
        for line in docker.lines() {
            let t = line.trim();
            if t.starts_with("FROM ") {
                assert!(
                    !t.contains("latest"),
                    "FROM must not use a floating latest tag: {t}"
                );
            }
        }
    }

    #[test]
    fn compose_uses_dockerfile_and_four_nodes() {
        let yml = include_str!("docker-compose.yml");
        assert!(
            yml.contains("dockerfile: infra/Dockerfile") || yml.contains("dockerfile: Dockerfile")
        );
        for h in ["node0", "node1", "node2", "node3"] {
            assert!(yml.contains(h), "missing service {h}");
        }
        for ip in NODE_IPS {
            assert!(yml.contains(ip), "compose must pin {ip}");
        }
        assert!(yml.contains("node.config") || yml.contains("NodeConfig"));
    }

    #[test]
    fn bootstrap_script_loads_genesis_and_p2p_bootstrap() {
        let sh = include_str!("bootstrap.sh");
        assert!(sh.contains("genesis.bin"));
        assert!(sh.contains("bootstrap.bin"));
        assert!(sh.contains("p2p.bootstrap") || sh.contains("bootstrap"));
    }

    #[test]
    fn terraform_references_dockerfile_and_is_illustrative() {
        let tf = include_str!("terraform/main.tf");
        assert!(tf.contains("Dockerfile"));
        assert!(tf.contains("illustrative") || tf.contains("not a production"));
    }

    #[test]
    fn missing_genesis_is_a_bootstrap_failure_mode() {
        let sh = include_str!("bootstrap.sh");
        assert!(sh.contains("exit 1"));
    }
}
