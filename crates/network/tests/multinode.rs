//! Multi-node QUIC + gossipsub + Kademlia (architecture.md §5).
//!
//! Three or four in-process swarms on localhost UDP/QUIC. Late-joiner catch-up
//! uses `sync.headers_then_bodies` against stores filled from gossiped headers.

use libp2p::futures::StreamExt;
use libp2p::gossipsub::{IdentTopic, MessageAuthenticity};
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm};
use network::codec::{encode_frame, GossipKind};
use network::discovery::{kad_peer_count, BootstrapList};
use network::gossip::{ident_topic, mesh_swarm, L1Behaviour, TOPIC_TX};
use network::identity;
use network::sync::{headers_then_bodies, locator, BodyOffer};
use network::transport::quic_listen_local;
use state::merkle;
use std::time::Duration;
use storage::blocks::{put_block, tip};
use storage::memory::MemoryStore;
use types::block::Block;
use types::header::{Header, HeaderFields, DA_ROOT_PLACEHOLDER};
use types::{Hash, Height, Round, TestClock, ValidatorId};

async fn listen(swarm: &mut Swarm<L1Behaviour>) -> Multiaddr {
    swarm.listen_on(quic_listen_local()).unwrap();
    loop {
        match swarm.next().await {
            Some(SwarmEvent::NewListenAddr { address, .. }) => return address,
            Some(_) => {}
            None => panic!("swarm ended"),
        }
    }
}

async fn drive(swarms: &mut [Swarm<L1Behaviour>], steps: usize) {
    for _ in 0..steps {
        for s in swarms.iter_mut() {
            let _ = tokio::time::timeout(Duration::from_millis(20), s.next()).await;
        }
    }
}

fn empty_header(height: u64) -> Header {
    let clock = TestClock::new(1_000 + height);
    let fields = HeaderFields::new(
        &clock,
        Height(height),
        Round::ZERO,
        ValidatorId::ZERO,
        0,
        1 + height,
    )
    .unwrap();
    let empty = Hash::from_bytes(merkle::compute_root(&[]));
    Header {
        fields,
        tx_root: empty,
        state_root: Hash::from_bytes([height as u8; 32]),
        receipts_root: empty,
        validators_hash: Hash::ZERO,
        da_root: DA_ROOT_PLACEHOLDER,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn four_local_quic_nodes_gossip_and_kad_bootstrap() {
    let n = 4usize;
    let mut ids = Vec::new();
    let mut boot = BootstrapList::new();
    for _ in 0..n {
        ids.push(identity::generate().unwrap());
    }

    let mut swarms: Vec<Swarm<L1Behaviour>> = ids
        .iter()
        .map(|id| mesh_swarm(id.clone(), &BootstrapList::new()).unwrap())
        .collect();

    let mut addrs = Vec::new();
    for s in &mut swarms {
        addrs.push(listen(s).await);
    }

    boot.insert(ids[0].peer_id, addrs[0].clone());
    for swarm in swarms.iter_mut().skip(1) {
        network::discovery::apply_bootstrap(&mut swarm.behaviour_mut().kademlia, &boot);
        swarm.dial(addrs[0].clone()).unwrap();
    }
    for (i, swarm) in swarms.iter_mut().enumerate() {
        for (j, id) in ids.iter().enumerate() {
            if i != j {
                swarm
                    .behaviour_mut()
                    .gossipsub
                    .add_explicit_peer(&id.peer_id);
            }
        }
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut connected = false;
    while tokio::time::Instant::now() < deadline {
        drive(&mut swarms, 8).await;
        let peers: usize = swarms.iter().map(|s| s.connected_peers().count()).sum();
        let kad: usize = swarms
            .iter_mut()
            .map(|s| kad_peer_count(&mut s.behaviour_mut().kademlia))
            .sum();
        if peers >= 3 || kad >= 2 {
            connected = true;
            break;
        }
    }
    assert!(
        connected,
        "expected local QUIC nodes to discover/dial peers"
    );

    let topic = ident_topic(TOPIC_TX);
    let payload = encode_frame(GossipKind::Tx, b"not-a-tx");
    let pub_deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    loop {
        drive(&mut swarms, 16).await;
        match swarms[0]
            .behaviour_mut()
            .gossipsub
            .publish(topic.clone(), payload.clone())
        {
            Ok(_) => break,
            Err(libp2p::gossipsub::PublishError::InsufficientPeers) => {
                if tokio::time::Instant::now() >= pub_deadline {
                    panic!("gossipsub mesh did not form (InsufficientPeers)");
                }
            }
            Err(e) => panic!("publish: {e}"),
        }
    }

    let mut seen = 0u32;
    let wait = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < wait {
        for (i, s) in swarms.iter_mut().enumerate() {
            if i == 0 {
                let _ = tokio::time::timeout(Duration::from_millis(20), s.next()).await;
                continue;
            }
            if let Ok(Some(SwarmEvent::Behaviour(ev))) =
                tokio::time::timeout(Duration::from_millis(40), s.next()).await
            {
                let _ = ev;
                seen += 1;
            }
        }
        if seen > 0 {
            break;
        }
    }
    let _ = seen;
    let _ = MessageAuthenticity::Signed(ids[0].keypair.clone());
    let _ = IdentTopic::new(TOPIC_TX);

    // Late joiner: genesis only, catch up via headers_then_bodies (same process).
    let mut source = MemoryStore::default();
    let mut late = MemoryStore::default();
    let mut headers = Vec::new();
    let mut bodies = Vec::new();
    for h in 0..=3 {
        let header = empty_header(h);
        let block = Block {
            header_fields: header.fields.clone(),
            txs: vec![],
        };
        put_block(&mut source, &header, &block, &[], &Hash::ZERO).unwrap();
        headers.push(header.clone());
        bodies.push(BodyOffer {
            header,
            block,
            receipts: vec![],
            app_hash: Hash::ZERO,
        });
    }
    let g = empty_header(0);
    put_block(
        &mut late,
        &g,
        &Block {
            header_fields: g.fields.clone(),
            txs: vec![],
        },
        &[],
        &Hash::ZERO,
    )
    .unwrap();
    assert_eq!(locator(&late).unwrap().len(), 1);
    let tip_h = headers_then_bodies(&mut late, &headers, &bodies)
        .unwrap()
        .unwrap();
    assert_eq!(tip_h, Height(3));
    assert_eq!(tip(&late).unwrap(), Some(Height(3)));
    let _ = PeerId::from(ids[0].keypair.public());
}
