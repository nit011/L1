//! L1 node binary.

mod tracing;

fn main() {
    tracing::init();
    let _ = tracing::span_name(tracing::SPAN_CONSENSUS, "boot");
    let _ = tracing::span_name(tracing::SPAN_EXECUTION, "idle");
}
