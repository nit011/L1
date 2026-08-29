//! Tracing conventions for the node binary.
//!
//! Contract: `tracing.conventions` (development-plan.md Tier 0).
//!
//! - Span names use `layer::operation` (for example `consensus::prevote`).
//! - Log and trace output must **never** gate control flow on a hash-sensitive
//!   path (execution `apply_block`, vote signing, VRF seed). Hashing those
//!   paths must not depend on whether a span is enabled.
//! - Do not hash formatted log strings; hash canonical encodings only.

use tracing_subscriber::EnvFilter;

/// Span name for consensus rounds.
pub const SPAN_CONSENSUS: &str = "consensus";

/// Span name for execution.
pub const SPAN_EXECUTION: &str = "execution";

/// Initialize a default subscriber from `RUST_LOG`.
pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

/// Compose a span target `layer::op`.
pub fn span_name(layer: &str, op: &str) -> String {
    format!("{layer}::{op}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_names_are_layered() {
        assert_eq!(span_name(SPAN_CONSENSUS, "prevote"), "consensus::prevote");
        assert_eq!(span_name(SPAN_EXECUTION, "apply"), "execution::apply");
    }

    #[test]
    fn init_is_idempotent() {
        init();
        init();
    }
}
