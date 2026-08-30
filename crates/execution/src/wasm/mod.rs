//! Gas-metered WASM (architecture.md §3 execution, §4.1 contracts trie).
//!
//! Guest code never touches tries directly: all storage goes through
//! [`host::sload`] / [`host::sstore`].

pub mod call;
pub mod deploy;
pub mod gas;
pub mod host;
