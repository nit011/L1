//! Cryptographic primitives (architecture.md §7).
//!
//! Address and validator-id derivation use `types` newtypes. Hashing and
//! signatures remain independent of state/execution.

pub mod address;
pub mod domain;
pub mod hash;
pub mod kzg;
pub mod sig;
pub mod tx;
pub mod validator;
pub mod vrf;

pub use address::from_ed25519;
pub use domain::{apply as apply_domain, DomainTag};
pub use hash::blake3::{hash as blake3_hash, hash_to_array};
pub use kzg::{
    commit as kzg_commit, open as kzg_open, setup as kzg_setup, verify as kzg_verify, KzgSetup,
};
pub use sig::bls;
pub use sig::ed25519;
pub use validator::from_bls;
pub use vrf::{prove as vrf_prove, verify as vrf_verify, Proof as VrfProof};
