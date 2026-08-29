//! BLAKE3-256 wrapper. Operates on raw bytes so this crate does not depend on `types`.
//!
//! See architecture.md §7 and development-plan.md §1.

/// 32-byte BLAKE3 digest.
pub type Digest = [u8; 32];

/// Hash `data` to 32 bytes. Contract: `hash.blake3`.
pub fn hash(data: &[u8]) -> blake3::Hash {
    blake3::hash(data)
}

/// Hash `data` to a raw array.
pub fn hash_to_array(data: &[u8]) -> Digest {
    *hash(data).as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Official BLAKE3 test vector for the empty input
    /// (BLAKE3 spec / test_vectors.json).
    const EMPTY: &str = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

    #[test]
    fn known_answer_empty() {
        let d = hash(b"");
        assert_eq!(hex::encode(d.as_bytes()), EMPTY);
        assert_eq!(hash_to_array(b""), *d.as_bytes());
    }

    #[test]
    fn different_inputs_differ() {
        assert_ne!(hash_to_array(b"a"), hash_to_array(b"b"));
    }

    #[test]
    fn empty_input_is_stable() {
        let a = hash_to_array(b"");
        let b = hash_to_array(b"");
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
    }
}
