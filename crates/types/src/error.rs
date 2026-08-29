//! Core error type for the `types` crate.

use thiserror::Error;

/// Recoverable failures in encoding and newtype conversions. Contract: `error.core`.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TypesError {
    /// Buffer shorter than header.
    #[error("canonical codec: truncated")]
    CodecTruncated,
    /// Version byte is not [`crate::encoding::CODEC_VERSION`].
    #[error("canonical codec: unsupported version {0}")]
    CodecVersion(u8),
    /// Length prefix does not match remaining bytes.
    #[error("canonical codec: expected {expected} payload bytes, got {actual}")]
    CodecLength { expected: usize, actual: usize },
    /// Fixed-size type received the wrong number of bytes.
    #[error("expected {expected} bytes, got {actual}")]
    BadLength { expected: usize, actual: usize },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::{decode, encode};

    #[test]
    fn display_and_equality() {
        let e = TypesError::BadLength {
            expected: 32,
            actual: 1,
        };
        assert!(e.to_string().contains("32"));
        assert_eq!(
            e,
            TypesError::BadLength {
                expected: 32,
                actual: 1
            }
        );
        assert!(decode(&encode(b"ok")).is_ok());
    }
}
