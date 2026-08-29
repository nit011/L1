//! Data-availability primitives. Tier 0: Reed-Solomon only.

pub mod rs;

pub use rs::{decode, encode, RsError};
