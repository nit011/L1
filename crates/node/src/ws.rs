//! Checkpoint-based bootstrap (architecture.md §2.4).
//!
//! Trusts a [`Checkpoint`] from `ws.checkpoint`, then runs [`catchup`]
//! (`node.catchup`). A chain that does not include that height/hash is refused.

use crate::config::NodeConfig;
use crate::sync::catchup;
use crate::wire::{BlockBroadcast, WireError};
use consensus::checkpoint::Checkpoint;
use network::sync::BodyOffer;
use storage::kv::Store;
use types::header::Header;
use types::Hash;

/// Bootstrap from a weak-subjectivity checkpoint rather than genesis hash alone.
/// Contract: `ws.bootstrap`.
pub fn bootstrap<S: Store, B: BlockBroadcast>(
    cfg: &NodeConfig,
    store: &mut S,
    checkpoint: &Checkpoint,
    remote_headers: &[Header],
    bodies: &[BodyOffer],
    sink: &mut B,
) -> Result<Hash, WireError> {
    let Some(h) = remote_headers
        .iter()
        .find(|hdr| hdr.fields.height == checkpoint.height)
    else {
        return Err(WireError::Gossip);
    };
    if h.hash() != checkpoint.header_hash {
        return Err(WireError::Gossip);
    }
    catchup(cfg, store, remote_headers, bodies, sink)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{init_store, TraceSink};
    use consensus::checkpoint::{record_checkpoint, Checkpoint};
    use consensus::steps::Finalized;
    use execution::builder::build_local;
    use execution::seq::World;
    use mempool::Mempool;
    use network::discovery::BootstrapList;
    use network::identity;
    use storage::memory::MemoryStore;
    use types::genesis::Genesis;
    use types::header::HeaderFields;
    use types::{ChainId, Height, Round, TestClock, ValidatorId};

    fn cfg_and_built() -> (
        crate::config::NodeConfig,
        execution::builder::BuiltBlock,
        Vec<Vec<u8>>,
    ) {
        let genesis = Genesis::new(ChainId::new(1));
        let cfg = crate::config::NodeConfig::new(
            genesis,
            BootstrapList::new(),
            identity::generate().unwrap(),
            std::path::PathBuf::from("/tmp/ws-boot"),
        );
        let clock = TestClock::new(2_000);
        let fields = HeaderFields::new(
            &clock,
            Height::GENESIS,
            Round::ZERO,
            ValidatorId::ZERO,
            0,
            1,
        )
        .unwrap();
        let mut pool = Mempool::new(&cfg.genesis.params.registry);
        let built = build_local(
            &mut pool,
            &cfg.genesis,
            World::from_genesis(&cfg.genesis),
            fields,
        );
        let rec: Vec<_> = built.receipts.iter().map(|r| r.encode()).collect();
        (cfg, built, rec)
    }

    #[test]
    fn bootstrap_from_checkpoint_then_catchup() {
        let (cfg, built, rec) = cfg_and_built();
        let f = Finalized {
            height: built.header.fields.height,
            round: built.header.fields.round,
            block_hash: built.header.hash(),
            app_hash: built.app_hash,
        };
        let cp = record_checkpoint(&f, &built.header).expect("height 0 is on interval");
        let offer = BodyOffer {
            header: built.header.clone(),
            block: built.block.clone(),
            receipts: rec,
            app_hash: built.app_hash,
        };
        let mut late = MemoryStore::new();
        init_store(&mut late, &cfg).unwrap();
        let app = bootstrap(
            &cfg,
            &mut late,
            &cp,
            std::slice::from_ref(&built.header),
            &[offer],
            &mut TraceSink::default(),
        )
        .unwrap();
        assert_eq!(app, built.app_hash);
        assert_eq!(storage::blocks::tip(&late).unwrap(), Some(Height::GENESIS));
    }

    #[test]
    fn bootstrap_refuses_chain_missing_checkpoint() {
        let (cfg, built, rec) = cfg_and_built();
        let wrong = Checkpoint {
            height: built.header.fields.height,
            header_hash: types::Hash::from_bytes([0xab; 32]),
        };
        let offer = BodyOffer {
            header: built.header.clone(),
            block: built.block.clone(),
            receipts: rec,
            app_hash: built.app_hash,
        };
        let mut late = MemoryStore::new();
        init_store(&mut late, &cfg).unwrap();
        let r = bootstrap(
            &cfg,
            &mut late,
            &wrong,
            std::slice::from_ref(&built.header),
            &[offer],
            &mut TraceSink::default(),
        );
        assert!(r.is_err());
    }
}
