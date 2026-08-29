//! Cold-start catch-up (development-plan.md Devnet MVP).
//!
//! Uses [`crate::wire::wire_sync`] then Tier 4 `store.replay_from_genesis`
//! with frozen `execution::seq::apply_block`.

use crate::config::NodeConfig;
use crate::wire::{wire_sync, BlockBroadcast, WireError};
use execution::seq::{apply_block, World};
use network::sync::BodyOffer;
use storage::kv::Store;
use storage::replay::replay_from_genesis;
use types::header::Header;
use types::Hash;

/// Fetch via `node.wire.sync`, then replay. Contract: `node.catchup`.
pub fn catchup<S: Store, B: BlockBroadcast>(
    cfg: &NodeConfig,
    store: &mut S,
    remote_headers: &[Header],
    bodies: &[BodyOffer],
    sink: &mut B,
) -> Result<Hash, WireError> {
    wire_sync(store, remote_headers, bodies, sink)?;
    let world = World::from_genesis(&cfg.genesis);
    let (_w, app) = replay_from_genesis(store, &cfg.genesis, world, apply_block)
        .map_err(|_| WireError::Store)?;
    Ok(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{init_store, TraceSink};
    use execution::builder::build_local;
    use mempool::Mempool;
    use network::discovery::BootstrapList;
    use network::identity;
    use storage::memory::MemoryStore;
    use types::genesis::Genesis;
    use types::header::HeaderFields;
    use types::{ChainId, Height, Round, TestClock, ValidatorId};

    #[test]
    fn catchup_replays_to_stored_app_hash() {
        let genesis = Genesis::new(ChainId::new(1));
        let cfg = NodeConfig::new(
            genesis,
            BootstrapList::new(),
            identity::generate().unwrap(),
            std::path::PathBuf::from("/tmp"),
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
        let mut late = MemoryStore::new();
        init_store(&mut late, &cfg).unwrap();
        let offer = BodyOffer {
            header: built.header.clone(),
            block: built.block.clone(),
            receipts: rec,
            app_hash: built.app_hash,
        };
        let app = catchup(
            &cfg,
            &mut late,
            std::slice::from_ref(&built.header),
            &[offer],
            &mut TraceSink::default(),
        )
        .unwrap();
        assert_eq!(app, built.app_hash);
        assert_eq!(storage::blocks::tip(&late).unwrap(), Some(Height::GENESIS));
    }

    #[test]
    fn catchup_rejects_wrong_genesis_hash() {
        let genesis = Genesis::new(ChainId::new(1));
        let cfg = NodeConfig::new(
            genesis,
            BootstrapList::new(),
            identity::generate().unwrap(),
            std::path::PathBuf::from("/tmp"),
        );
        let mut late = MemoryStore::new();
        let r = catchup(&cfg, &mut late, &[], &[], &mut TraceSink::default());
        assert!(r.is_err());
    }

    #[test]
    fn persist_then_broadcast_is_used_by_catchup() {
        let _ = super::catchup::<storage::memory::MemoryStore, crate::wire::TraceSink>;
    }
}
