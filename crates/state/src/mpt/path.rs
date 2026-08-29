//! Nibble-path / hex-prefix encoding for the hexary MPT (architecture.md §4.1).
//!
//! Packed HP bytes are wrapped with Tier 0 `encoding.canonical.encode`.

use types::encoding;
use types::TypesError;

/// Convert key bytes to a nibble sequence (high nibble first).
pub fn bytes_to_nibbles(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(b >> 4);
        out.push(b & 0x0f);
    }
    out
}

/// Pack nibbles with Ethereum-style hex-prefix flags (leaf vs extension).
pub fn pack_hex_prefix(nibbles: &[u8], is_leaf: bool) -> Vec<u8> {
    let odd = nibbles.len() % 2 == 1;
    let mut flags = if is_leaf { 0x20 } else { 0x00 };
    let mut packed = Vec::new();
    if odd {
        flags |= 0x10;
        packed.push(flags | nibbles[0]);
        for i in (1..nibbles.len()).step_by(2) {
            packed.push((nibbles[i] << 4) | nibbles[i + 1]);
        }
    } else {
        packed.push(flags);
        for i in (0..nibbles.len()).step_by(2) {
            packed.push((nibbles[i] << 4) | nibbles[i + 1]);
        }
    }
    packed
}

/// Decode hex-prefix packing.
pub fn unpack_hex_prefix(packed: &[u8]) -> Result<(Vec<u8>, bool), TypesError> {
    if packed.is_empty() {
        return Err(TypesError::BadLength {
            expected: 1,
            actual: 0,
        });
    }
    let flags = packed[0];
    let is_leaf = flags & 0x20 != 0;
    let odd = flags & 0x10 != 0;
    let mut nibbles = Vec::new();
    if odd {
        nibbles.push(flags & 0x0f);
        for b in &packed[1..] {
            nibbles.push(b >> 4);
            nibbles.push(b & 0x0f);
        }
    } else {
        for b in &packed[1..] {
            nibbles.push(b >> 4);
            nibbles.push(b & 0x0f);
        }
    }
    Ok((nibbles, is_leaf))
}

/// Canonical encoding of a nibble path. Contract: `mpt.pathencoding`.
pub fn encode_path(nibbles: &[u8], is_leaf: bool) -> Vec<u8> {
    encoding::encode(&pack_hex_prefix(nibbles, is_leaf))
}

/// Inverse of [`encode_path`].
pub fn decode_path(buf: &[u8]) -> Result<(Vec<u8>, bool), TypesError> {
    let packed = encoding::decode(buf)?;
    unpack_hex_prefix(&packed)
}

/// Length of the shared nibble prefix.
pub fn common_prefix_len(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_even_and_odd_leaf() {
        let even = vec![1, 2, 3, 4];
        let (n, leaf) = decode_path(&encode_path(&even, true)).unwrap();
        assert_eq!(n, even);
        assert!(leaf);
        let odd = vec![9, 8, 7];
        let (n, leaf) = decode_path(&encode_path(&odd, false)).unwrap();
        assert_eq!(n, odd);
        assert!(!leaf);
    }

    #[test]
    fn decode_rejects_truncated_codec() {
        assert!(decode_path(&[1, 0]).is_err());
    }

    #[test]
    fn common_prefix() {
        assert_eq!(common_prefix_len(&[1, 2, 3], &[1, 2, 9]), 2);
        assert_eq!(common_prefix_len(&[1], &[2]), 0);
    }

    #[test]
    fn bytes_to_nibbles_abc() {
        assert_eq!(bytes_to_nibbles(&[0xab, 0x0c]), vec![0xa, 0xb, 0x0, 0xc]);
    }
}
