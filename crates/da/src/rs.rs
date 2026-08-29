//! Reed-Solomon erasure coding primitive (architecture.md §6).
//!
//! Chunk gossip and DAS are Tier 12. This crate only implements encode/decode.

use reed_solomon_erasure::galois_8::ReedSolomon;
use thiserror::Error;

/// Erasure-coding errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RsError {
    /// `k` or `m` was zero.
    #[error("reed-solomon: k and m must be >= 1")]
    Params,
    /// Reconstruction failed (too few shards).
    #[error("reed-solomon: reconstruction failed")]
    Reconstruct,
}

fn pad_len(data_len: usize, k: usize) -> usize {
    let body = 8 + data_len;
    let rem = body % k;
    if rem == 0 {
        body
    } else {
        body + (k - rem)
    }
}

/// Split `data` into `k` data shards + `m` parity shards (equal shard length).
/// Contract: `reed_solomon.encode`.
pub fn encode(data: &[u8], k: usize, m: usize) -> Result<Vec<Vec<u8>>, RsError> {
    if k == 0 || m == 0 {
        return Err(RsError::Params);
    }
    let total = pad_len(data.len(), k);
    let shard_len = total / k;
    let mut buf = vec![0u8; total];
    buf[..8].copy_from_slice(&(data.len() as u64).to_be_bytes());
    buf[8..8 + data.len()].copy_from_slice(data);

    let mut shards: Vec<Vec<u8>> = (0..k)
        .map(|i| buf[i * shard_len..(i + 1) * shard_len].to_vec())
        .collect();
    shards.extend((0..m).map(|_| vec![0u8; shard_len]));

    let r = ReedSolomon::new(k, m).map_err(|_| RsError::Params)?;
    r.encode(&mut shards).map_err(|_| RsError::Reconstruct)?;
    Ok(shards)
}

/// Reconstruct original data from any `k` of `k+m` shards (`None` = missing).
/// Contract: `reed_solomon.decode`.
pub fn decode(shards: &[Option<Vec<u8>>], k: usize, m: usize) -> Result<Vec<u8>, RsError> {
    if k == 0 || m == 0 {
        return Err(RsError::Params);
    }
    let r = ReedSolomon::new(k, m).map_err(|_| RsError::Params)?;
    let mut working = shards.to_vec();
    r.reconstruct(&mut working)
        .map_err(|_| RsError::Reconstruct)?;
    let present: Vec<Vec<u8>> = working
        .into_iter()
        .take(k)
        .map(|s| s.ok_or(RsError::Reconstruct))
        .collect::<Result<_, _>>()?;
    let mut buf = Vec::new();
    for s in present {
        buf.extend_from_slice(&s);
    }
    if buf.len() < 8 {
        return Err(RsError::Reconstruct);
    }
    let len = u64::from_be_bytes(buf[..8].try_into().unwrap()) as usize;
    let rest = &buf[8..];
    if len > rest.len() {
        return Err(RsError::Reconstruct);
    }
    Ok(rest[..len].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_all_shards() {
        let data = b"block-body-bytes-0123456789";
        let shards = encode(data, 4, 2).unwrap();
        assert_eq!(shards.len(), 6);
        let opts: Vec<_> = shards.into_iter().map(Some).collect();
        assert_eq!(decode(&opts, 4, 2).unwrap(), data);
    }

    #[test]
    fn decode_with_parity_only_dropped() {
        let data = b"keep-all-data-shards";
        let shards = encode(data, 3, 2).unwrap();
        let mut opts: Vec<_> = shards.into_iter().map(Some).collect();
        opts[3] = None;
        opts[4] = None;
        assert_eq!(decode(&opts, 3, 2).unwrap(), data);
    }

    #[test]
    fn decode_with_mixed_erasures() {
        let data = b"mix-data-and-parity-loss!!";
        let shards = encode(data, 4, 3).unwrap();
        let mut opts: Vec<_> = shards.into_iter().map(Some).collect();
        opts[0] = None;
        opts[2] = None;
        opts[5] = None;
        assert_eq!(decode(&opts, 4, 3).unwrap(), data);
    }

    #[test]
    fn too_few_shards_fails() {
        let shards = encode(b"xyz", 3, 2).unwrap();
        let mut opts: Vec<_> = shards.into_iter().map(Some).collect();
        opts[0] = None;
        opts[1] = None;
        opts[2] = None;
        assert!(decode(&opts, 3, 2).is_err());
    }

    #[test]
    fn rejects_zero_params() {
        assert!(encode(b"a", 0, 2).is_err());
        assert!(encode(b"a", 2, 0).is_err());
    }
}
