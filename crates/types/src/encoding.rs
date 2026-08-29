//! Canonical binary codec: version byte plus length-prefixed payload.
//!
//! Frozen for hashing and replay (development-plan.md §1). JSON is RPC-only
//! and must not be used here.

use crate::error::TypesError;

/// Codec version byte. Changing this after Tier 2 forks the chain.
pub const CODEC_VERSION: u8 = 1;

/// Encode `payload` as `version || u32_be(len) || payload`.
///
/// Contract: `encoding.canonical.encode`.
pub fn encode(payload: &[u8]) -> Vec<u8> {
    let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(1 + 4 + payload.len());
    out.push(CODEC_VERSION);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Decode a buffer produced by [`encode`]. Rejects wrong version, truncated
/// buffers, and trailing bytes.
///
/// Contract: `encoding.canonical.decode`.
pub fn decode(buf: &[u8]) -> Result<Vec<u8>, TypesError> {
    if buf.len() < 5 {
        return Err(TypesError::CodecTruncated);
    }
    if buf[0] != CODEC_VERSION {
        return Err(TypesError::CodecVersion(buf[0]));
    }
    let len = u32::from_be_bytes(buf[1..5].try_into().expect("slice len 4")) as usize;
    let rest = &buf[5..];
    if rest.len() != len {
        return Err(TypesError::CodecLength {
            expected: len,
            actual: rest.len(),
        });
    }
    Ok(rest.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip() {
        let payload = b"hello-l1";
        let encoded = encode(payload);
        assert_eq!(encoded[0], CODEC_VERSION);
        assert_eq!(decode(&encoded).unwrap(), payload);
    }

    #[test]
    fn decode_rejects_wrong_version() {
        let mut encoded = encode(b"x");
        encoded[0] = 99;
        assert!(matches!(
            decode(&encoded),
            Err(TypesError::CodecVersion(99))
        ));
    }

    #[test]
    fn decode_rejects_truncated_and_trailing() {
        assert!(matches!(
            decode(&[1, 0, 0]),
            Err(TypesError::CodecTruncated)
        ));
        let mut encoded = encode(b"ab");
        encoded.push(0xff);
        assert!(matches!(
            decode(&encoded),
            Err(TypesError::CodecLength { .. })
        ));
    }
}
