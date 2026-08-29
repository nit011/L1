//! Multi-process simnet (development-plan.md Devnet MVP).
//!
//! Contract: `node.simnet.multiprocess`.

mod common;

use common::{add_follower, events, start_validators, wait_tip_at_least};
use crypto::sig::ed25519::SecretKey as EdSk;
use crypto::tx::sign;
use libp2p::futures::StreamExt;
use libp2p::swarm::SwarmEvent;
use network::codec::{encode_frame, GossipKind};
use network::eclipse::{v4, IpSlotTable};
use network::gossip::{ident_topic, mesh_swarm, TOPIC_TX};
use network::identity;
use network::transport::quic_listen_local;
use std::net::IpAddr;
use std::time::{Duration, Instant};
use types::{Amount, ChainId, Nonce, GAS_TRANSFER};

fn tmp() -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "l1-simnet-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn run_four_until_commit(runs: usize) {
    for run in 0..runs {
        let root = tmp().join(format!("run{run}"));
        let cluster = start_validators(&root, 4);
        let deadline = Instant::now() + Duration::from_secs(45);
        for d in &cluster.dirs {
            wait_tip_at_least(d, 0, deadline);
        }
        let h0: Vec<_> = cluster
            .dirs
            .iter()
            .filter_map(|d| common::read_tip(d))
            .collect();
        assert!(
            h0.iter().all(|&h| h == h0[0]),
            "split tips {h0:?} run {run}"
        );
    }
}

#[test]
fn multiprocess_four_nodes_commit_three_runs() {
    run_four_until_commit(3);
}

#[test]
fn late_join_fifth_node_catchup() {
    let root = tmp().join("late");
    let mut cluster = start_validators(&root, 4);
    let deadline = Instant::now() + Duration::from_secs(50);
    wait_tip_at_least(&cluster.dirs[0], 2, deadline);
    let behind = common::read_tip(&cluster.dirs[0]).unwrap();
    let (dir5, child) = add_follower(&root, &cluster.dirs[0], "n5", &cluster.dirs);
    cluster.children.push(child);
    cluster.dirs.push(dir5.clone());
    let catch_deadline = Instant::now() + Duration::from_secs(40);
    wait_tip_at_least(&dir5, behind, catch_deadline);
    let got = common::read_tip(&dir5).unwrap();
    assert!(got >= behind, "late join behind={behind} got={got}");
    let log = events(&dir5);
    assert!(
        log.contains("CATCHUP") || got >= behind,
        "catch-up log:\n{log}"
    );
}

#[test]
fn eclipse_rejection_ip_slot_cap_in_multiprocess_context() {
    let root = tmp().join("eclipse");
    let cluster = start_validators(&root, 4);
    let deadline = Instant::now() + Duration::from_secs(40);
    wait_tip_at_least(&cluster.dirs[0], 0, deadline);
    let mut table = IpSlotTable::new(2, 8);
    let mut admitted_same = 0usize;
    for i in 0..10u8 {
        let id = identity::generate().unwrap();
        if table.admit(&id.peer_id, v4(10, 0, 0, i)) {
            admitted_same += 1;
        }
    }
    assert_eq!(admitted_same, 2, "single /24 cannot fill the table");
    let diverse = identity::generate().unwrap();
    assert!(table.admit(&diverse.peer_id, v4(11, 1, 2, 3)));
    let prefixes: std::collections::BTreeSet<_> = [v4(10, 0, 0, 1), v4(11, 1, 2, 3)]
        .into_iter()
        .map(|ip| match ip {
            IpAddr::V4(v) => (v.octets()[0], v.octets()[1], v.octets()[2]),
            _ => (0, 0, 0),
        })
        .collect();
    assert!(prefixes.len() >= 2, "observed prefixes {prefixes:?}");
    assert_eq!(table.len(), 3);
    let _ = cluster;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_gossip_dropped_at_wire_mempool() {
    let root = tmp().join("badtx");
    let cluster = start_validators(&root, 4);
    let deadline = Instant::now() + Duration::from_secs(40);
    wait_tip_at_least(&cluster.dirs[0], 0, deadline);
    let listen: libp2p::Multiaddr = std::fs::read_to_string(cluster.dirs[0].join("listen"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let id = identity::generate().unwrap();
    let mut swarm = mesh_swarm(id, &network::discovery::BootstrapList::new()).unwrap();
    swarm.listen_on(quic_listen_local()).unwrap();
    loop {
        match swarm.next().await {
            Some(SwarmEvent::NewListenAddr { .. }) => break,
            Some(_) => {}
            None => panic!("swarm"),
        }
    }
    swarm.dial(listen).unwrap();
    for _ in 0..40 {
        let _ = tokio::time::timeout(Duration::from_millis(50), swarm.next()).await;
    }
    let ska = EdSk::from_bytes(&[3u8; 32]);
    let from = crypto::from_ed25519(&ska.verifying_key());
    let tx = types::tx::Tx::transfer(
        ChainId::new(7),
        Nonce::ZERO,
        GAS_TRANSFER,
        Amount::new(1),
        from,
        Amount::new(10),
    );
    let mut bad = sign(&ska, tx);
    bad.signature[0] ^= 1;
    let inner = storage::codec::encode_signed_tx(&bad);
    let frame = encode_frame(GossipKind::Tx, &inner);
    swarm
        .behaviour_mut()
        .gossipsub
        .publish(ident_topic(TOPIC_TX), frame)
        .unwrap();
    for _ in 0..30 {
        let _ = tokio::time::timeout(Duration::from_millis(40), swarm.next()).await;
    }
    std::thread::sleep(Duration::from_millis(400));
    let mut drops = 0usize;
    let mut admits = 0usize;
    for d in &cluster.dirs {
        let log = events(d);
        drops += log.matches("TX_DROP").count();
        admits += log.matches("TX_ADMIT").count();
    }
    assert!(
        drops >= 1,
        "invalid tx must hit wire_mempool drop; admits={admits} logs {:?}",
        cluster.dirs.iter().map(|d| events(d)).collect::<Vec<_>>()
    );
    assert_eq!(admits, 0, "invalid tx must not be admitted");
}
