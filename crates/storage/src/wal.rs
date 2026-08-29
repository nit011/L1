//! Write-ahead log around `exec.seq.apply_block` (architecture.md §4).
//!
//! Record the block **before** the state/chain commit. A crash after the WAL
//! write and before `store.block.put` is recovered by finishing the commit or
//! discarding a duplicate if the block is already stored — never a half-applied
//! index/header split (`kv.batch` on the chain write).
//!
//! Same crate-cycle rule as `replay`: invoke the frozen `apply_block` callback.

use crate::blocks::{get_block, put_block};
use crate::codec::{decode_block_body, encode_block_body, KEY_WAL};
use crate::kv::{BatchOp, Store};
use crate::replay::{replay_from_genesis, ReplayError};
use types::block::Block;
use types::genesis::Genesis;
use types::header::Header;
use types::Hash;
use types::TypesError;

/// WAL / recovery error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalError {
    /// Store.
    Store(TypesError),
    /// Replay while recovering.
    Replay(ReplayError),
    /// WAL bytes corrupt.
    Corrupt,
}

impl From<TypesError> for WalError {
    fn from(e: TypesError) -> Self {
        Self::Store(e)
    }
}

impl From<ReplayError> for WalError {
    fn from(e: ReplayError) -> Self {
        Self::Replay(e)
    }
}

fn encode_wal(
    header: &Header,
    block: &Block,
    receipt_encodings: &[Vec<u8>],
    app_hash: &Hash,
) -> Vec<u8> {
    let mut p = Vec::new();
    let pre = header.hash_preimage();
    p.extend_from_slice(&(pre.len() as u32).to_be_bytes());
    p.extend_from_slice(&pre);
    let body = encode_block_body(block);
    p.extend_from_slice(&(body.len() as u32).to_be_bytes());
    p.extend_from_slice(&body);
    p.extend_from_slice(app_hash.as_bytes());
    p.extend_from_slice(&(receipt_encodings.len() as u32).to_be_bytes());
    for r in receipt_encodings {
        p.extend_from_slice(&(r.len() as u32).to_be_bytes());
        p.extend_from_slice(r);
    }
    types::encode(&p)
}

struct WalPayload {
    header: Header,
    block: Block,
    receipts: Vec<Vec<u8>>,
    app_hash: Hash,
}

fn decode_wal(buf: &[u8]) -> Result<WalPayload, WalError> {
    let p = types::decode(buf).map_err(WalError::Store)?;
    if p.len() < 4 {
        return Err(WalError::Corrupt);
    }
    let mut i = 0usize;
    let take_u32 = |i: &mut usize| -> Result<u32, WalError> {
        if *i + 4 > p.len() {
            return Err(WalError::Corrupt);
        }
        let n = u32::from_be_bytes(p[*i..*i + 4].try_into().unwrap());
        *i += 4;
        Ok(n)
    };
    let plen = take_u32(&mut i)? as usize;
    if i + plen > p.len() {
        return Err(WalError::Corrupt);
    }
    let header = crate::codec::header_from_preimage(&p[i..i + plen]).map_err(WalError::Store)?;
    i += plen;
    let blen = take_u32(&mut i)? as usize;
    if i + blen > p.len() {
        return Err(WalError::Corrupt);
    }
    let block = decode_block_body(&p[i..i + blen]).map_err(WalError::Store)?;
    i += blen;
    if i + 32 > p.len() {
        return Err(WalError::Corrupt);
    }
    let mut app = [0u8; 32];
    app.copy_from_slice(&p[i..i + 32]);
    i += 32;
    let n = take_u32(&mut i)? as usize;
    let mut recs = Vec::with_capacity(n);
    for _ in 0..n {
        let rl = take_u32(&mut i)? as usize;
        if i + rl > p.len() {
            return Err(WalError::Corrupt);
        }
        recs.push(p[i..i + rl].to_vec());
        i += rl;
    }
    if i != p.len() {
        return Err(WalError::Corrupt);
    }
    Ok(WalPayload {
        header,
        block,
        receipts: recs,
        app_hash: Hash::from_bytes(app),
    })
}

/// Write the WAL record only (`kv.batch`). Used to simulate a crash before commit.
pub fn write_wal<S: Store>(
    store: &mut S,
    header: &Header,
    block: &Block,
    receipt_encodings: &[Vec<u8>],
    app_hash: &Hash,
) -> Result<(), TypesError> {
    store.apply_batch(&[BatchOp::Put {
        key: KEY_WAL.to_vec(),
        value: encode_wal(header, block, receipt_encodings, app_hash),
    }])
}

/// Clear the WAL (`kv.batch`).
pub fn clear_wal<S: Store>(store: &mut S) -> Result<(), TypesError> {
    store.apply_batch(&[BatchOp::Delete {
        key: KEY_WAL.to_vec(),
    }])
}

/// Commit path: WAL, then atomic `store.block.put`, then clear WAL.
///
/// If `crash_after_wal` is true, stop after the WAL write (no chain commit).
pub fn commit_with_wal<S: Store>(
    store: &mut S,
    header: &Header,
    block: &Block,
    receipt_encodings: &[Vec<u8>],
    app_hash: &Hash,
    crash_after_wal: bool,
) -> Result<(), TypesError> {
    write_wal(store, header, block, receipt_encodings, app_hash)?;
    if crash_after_wal {
        return Ok(());
    }
    put_block(store, header, block, receipt_encodings, app_hash)?;
    clear_wal(store)?;
    Ok(())
}

/// Recover: if WAL exists and the block is not stored, finish `put_block`.
/// If it is already stored, drop the WAL. Re-runs `apply_block` on replayed
/// state to confirm the WAL `app_hash` (never leaves a half-written batch).
pub fn recover<S, W, R, F>(
    store: &mut S,
    genesis: &Genesis,
    world: W,
    mut apply_block: F,
) -> Result<W, WalError>
where
    S: Store,
    F: FnMut(W, &Block) -> (W, Vec<R>, Hash),
{
    let wal = store.get(KEY_WAL)?;
    let Some(bytes) = wal else {
        let (w, _) = replay_from_genesis(store, genesis, world, apply_block)?;
        return Ok(w);
    };
    let WalPayload {
        header,
        block,
        receipts: recs,
        app_hash: wal_app,
    } = decode_wal(&bytes)?;
    let height = header.fields.height;
    if get_block(store, height)?.is_some() {
        clear_wal(store)?;
        let (w, _) = replay_from_genesis(store, genesis, world, apply_block)?;
        return Ok(w);
    }
    let (replayed, _) = replay_from_genesis(store, genesis, world, &mut apply_block)?;
    let (post, _r, app) = apply_block(replayed, &block);
    if app != wal_app {
        return Err(WalError::Corrupt);
    }
    put_block(store, &header, &block, &recs, &wal_app)?;
    clear_wal(store)?;
    Ok(post)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::{get_app_hash, put_genesis_hash, tip};
    use crate::memory::MemoryStore;
    use execution::seq::{apply_block, apply_block_with_roots, World};
    use types::header::{HeaderFields, DA_ROOT_PLACEHOLDER};
    use types::{ChainId, Hash, Height, Round, TestClock, ValidatorId};

    fn empty_at(
        g: &Genesis,
        height: Height,
        ts: u64,
    ) -> (Header, Block, Vec<Vec<u8>>, Hash, World) {
        let clock = TestClock::new(2_000_000);
        let fields =
            HeaderFields::new(&clock, height, Round::ZERO, ValidatorId::ZERO, 0, ts).unwrap();
        let block = Block {
            header_fields: fields.clone(),
            txs: vec![],
        };
        let world = World::from_genesis(g);
        let (post, recs, app, st, tx_r, rec_r) = apply_block_with_roots(world.clone(), &block);
        let header = Header {
            fields,
            tx_root: tx_r,
            state_root: st,
            receipts_root: rec_r,
            validators_hash: Hash::ZERO,
            da_root: DA_ROOT_PLACEHOLDER,
        };
        let enc: Vec<Vec<u8>> = recs.iter().map(|r| r.encode()).collect();
        (header, block, enc, app, post)
    }

    #[test]
    fn crash_after_wal_then_recover_commits() {
        let g = Genesis::new(ChainId::new(1));
        let mut store = MemoryStore::new();
        put_genesis_hash(&mut store, &g).unwrap();
        let (header, block, enc, app, _) = empty_at(&g, Height::GENESIS, 10);
        commit_with_wal(&mut store, &header, &block, &enc, &app, true).unwrap();
        assert!(crate::blocks::get_block(&store, Height::GENESIS)
            .unwrap()
            .is_none());
        assert!(store.get(KEY_WAL).unwrap().is_some());
        let world = World::from_genesis(&g);
        let recovered = recover(&mut store, &g, world, apply_block).unwrap();
        assert_eq!(get_app_hash(&store, Height::GENESIS).unwrap(), Some(app));
        assert_eq!(tip(&store).unwrap(), Some(Height::GENESIS));
        assert!(store.get(KEY_WAL).unwrap().is_none());
        let (_, _, app2) = apply_block(World::from_genesis(&g), &block);
        assert_eq!(app, app2);
        assert_eq!(recovered.commit_state_root(), header.state_root);
    }

    #[test]
    fn recover_idempotent_if_block_already_stored() {
        let g = Genesis::new(ChainId::new(1));
        let mut store = MemoryStore::new();
        put_genesis_hash(&mut store, &g).unwrap();
        let (header, block, enc, app, _) = empty_at(&g, Height::GENESIS, 11);
        commit_with_wal(&mut store, &header, &block, &enc, &app, false).unwrap();
        write_wal(&mut store, &header, &block, &enc, &app).unwrap();
        let world = World::from_genesis(&g);
        let _ = recover(&mut store, &g, world, apply_block).unwrap();
        assert!(store.get(KEY_WAL).unwrap().is_none());
        assert_eq!(get_app_hash(&store, Height::GENESIS).unwrap(), Some(app));
    }
}
