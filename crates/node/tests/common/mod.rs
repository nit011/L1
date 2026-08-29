//! Shared simnet helpers (real OS processes + QUIC).
#![allow(dead_code)]

use crypto::from_bls;
use crypto::sig::bls;
use crypto::vrf::public_key_from_seed;
use network::discovery::BootstrapList;
use network::identity;
use node::config::{write_bootstrap, write_dir, write_vrf_pks, write_vrf_secrets, NodeConfig};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use types::collections::Map;
use types::genesis::Genesis;
use types::{ChainId, ValidatorId, VotingPower};

pub struct Cluster {
    pub children: Vec<Child>,
    pub dirs: Vec<PathBuf>,
}

impl Drop for Cluster {
    fn drop(&mut self) {
        for c in &mut self.children {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

pub fn read_tip(dir: &Path) -> Option<u64> {
    let s = std::fs::read_to_string(dir.join("tip")).ok()?;
    s.lines().next()?.parse().ok()
}

pub fn events(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("events.log")).unwrap_or_default()
}

pub fn wait_listen(dir: &Path, deadline: Instant) -> String {
    loop {
        if let Ok(s) = std::fs::read_to_string(dir.join("listen")) {
            let t = s.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
        if Instant::now() > deadline {
            panic!("no listen addr in {}", dir.display());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn wait_tip_at_least(dir: &Path, min_h: u64, deadline: Instant) -> u64 {
    loop {
        if let Some(h) = read_tip(dir) {
            if h >= min_h {
                return h;
            }
        }
        if Instant::now() > deadline {
            let mut dump = String::new();
            if let Some(parent) = dir.parent() {
                if let Ok(rd) = std::fs::read_dir(parent) {
                    for e in rd.flatten() {
                        let p = e.path();
                        dump.push_str(&format!("\n--- {} ---\n{}\n", p.display(), events(&p)));
                    }
                }
            }
            let err = std::fs::read_to_string(dir.join("stderr.log")).unwrap_or_default();
            panic!(
                "tip < {min_h} in {} events:\n{}\nstderr:\n{err}\ncluster:{dump}",
                dir.display(),
                events(dir)
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Four genesis validators + data dirs. Returns (cluster dirs not yet spawned, vrf pks).
pub fn prepare_validator_dirs(n: usize, tmp: &Path) -> (Vec<PathBuf>, Map<ValidatorId, [u8; 32]>) {
    let mut genesis = Genesis::new(ChainId::new(7));
    let mut vrf_pks = Map::new();
    let mut vrf_secrets = Map::new();
    let mut rows = Vec::new();
    for i in 0..n {
        let sk = bls::keygen().unwrap();
        let (id, _) = from_bls(&sk.sk_to_pk(), VotingPower(1));
        genesis.insert_validator(id, VotingPower(1));
        let mut vrf_sk = [0u8; 32];
        vrf_sk[0] = (i as u8).saturating_add(1);
        vrf_sk[1] = 0x5a;
        let vrf_pk = public_key_from_seed(&vrf_sk);
        vrf_pks.insert(id, vrf_pk);
        vrf_secrets.insert(id, vrf_sk);
        rows.push((sk.to_bytes(), vrf_sk, identity::generate().unwrap()));
        let _ = i;
    }
    let mut dirs = Vec::new();
    for (i, (bls_bytes, vrf_sk, id)) in rows.iter().enumerate() {
        let dir = tmp.join(format!("n{i}"));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = NodeConfig::new(
            genesis.clone(),
            BootstrapList::new(),
            id.clone(),
            dir.clone(),
        );
        write_dir(&dir, &cfg, bls_bytes, vrf_sk).unwrap();
        write_vrf_pks(&dir, &vrf_pks).unwrap();
        write_vrf_secrets(&dir, &vrf_secrets).unwrap();
        dirs.push(dir);
    }
    (dirs, vrf_pks)
}

pub fn spawn_node(dir: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_node"))
        .arg("--dir")
        .arg(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::from(
            std::fs::File::create(dir.join("stderr.log")).unwrap(),
        ))
        .spawn()
        .expect("spawn node")
}

pub fn wire_bootstrap(dirs: &[PathBuf]) {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut boot = BootstrapList::new();
    for d in dirs {
        let listen = wait_listen(d, deadline);
        let addr: libp2p::Multiaddr = listen.parse().expect("multiaddr");
        let cfg = node::config::load_dir(d).unwrap().0;
        boot.insert(cfg.identity.peer_id, addr);
    }
    for d in dirs {
        write_bootstrap(d, &boot).unwrap();
    }
}

pub fn start_validators(tmp: &Path, n: usize) -> Cluster {
    let (dirs, _) = prepare_validator_dirs(n, tmp);
    let mut children = Vec::new();
    for d in &dirs {
        children.push(spawn_node(d));
    }
    wire_bootstrap(&dirs);
    Cluster { children, dirs }
}

pub fn add_follower(
    tmp: &Path,
    genesis_from: &Path,
    name: &str,
    boot_from: &[PathBuf],
) -> (PathBuf, Child) {
    let dir = tmp.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let (mut cfg, _, _) = node::config::load_dir(genesis_from).unwrap();
    cfg.identity = identity::generate().unwrap();
    cfg.bootstrap = BootstrapList::new();
    cfg.data_dir = dir.clone();
    let bls = bls::keygen().unwrap().to_bytes();
    let vrf_sk = [9u8; 32];
    write_dir(&dir, &cfg, &bls, &vrf_sk).unwrap();
    std::fs::copy(genesis_from.join("vrf_pks.bin"), dir.join("vrf_pks.bin")).ok();
    std::fs::copy(
        genesis_from.join("vrf_secrets.bin"),
        dir.join("vrf_secrets.bin"),
    )
    .ok();
    let child = spawn_node(&dir);
    let deadline = Instant::now() + Duration::from_secs(10);
    let _ = wait_listen(&dir, deadline);
    let mut boot = BootstrapList::new();
    for d in boot_from {
        if let Ok(s) = std::fs::read_to_string(d.join("listen")) {
            if let Ok(addr) = s.trim().parse() {
                let peer = node::config::load_dir(d).unwrap().0.identity.peer_id;
                boot.insert(peer, addr);
            }
        }
    }
    write_bootstrap(&dir, &boot).unwrap();
    (dir, child)
}
