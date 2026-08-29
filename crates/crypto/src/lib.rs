//! Cryptographic primitives (architecture.md §7). Independent of `types`.

pub mod domain;
pub mod hash;
pub mod kzg;
pub mod sig;
pub mod vrf;

pub use domain::{apply as apply_domain, DomainTag};
pub use hash::blake3::{hash as blake3_hash, hash_to_array};
pub use kzg::{setup as kzg_setup, KzgSetup};
pub use sig::bls;
pub use sig::ed25519;
pub use vrf::{prove as vrf_prove, verify as vrf_verify, Proof as VrfProof};
