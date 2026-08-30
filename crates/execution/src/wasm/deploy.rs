//! Contract deployment (architecture.md §3 / §4.1).
//!
//! Bytecode is parsed with the real wasmtime engine **before** the accounts
//! trie is updated. Invalid modules return [`RejectReason::WasmInvalid`]
//! with no nonce/balance mutation (callers must check this first).

use crate::receipt::RejectReason;
use crate::seq::World;
use crate::wasm::gas::meter;
use crypto::hash::blake3::hash_to_array;
use state::account::Account;
use types::tx::{Deploy, Tx};
use types::{Address, Hash, Nonce};

/// CREATE address = BLAKE3(`from || nonce`) as [`Address`].
pub fn create_address(from: &Address, nonce: Nonce) -> Address {
    let mut buf = Vec::with_capacity(40);
    buf.extend_from_slice(from.as_bytes());
    buf.extend_from_slice(&nonce.0.to_be_bytes());
    Address::from_bytes(hash_to_array(&buf))
}

fn engine() -> wasmtime::Engine {
    let mut c = wasmtime::Config::new();
    c.consume_fuel(true);
    wasmtime::Engine::new(&c).expect("wasmtime engine")
}

/// Parse/validate guest bytecode. Does not meter execution fuel.
pub fn validate_wasm(code: &[u8]) -> Result<(), RejectReason> {
    wasmtime::Module::new(&engine(), code)
        .map(|_| ())
        .map_err(|_| RejectReason::WasmInvalid)
}

/// Store validated code against `state.account_trie`. Contract: `wasm.deploy`.
pub fn install(world: &mut World, addr: Address, code: &[u8]) -> Result<(), RejectReason> {
    validate_wasm(code)?;
    let code_hash = Hash::from_bytes(hash_to_array(code));
    let mut acc = world.accounts.get(&addr).unwrap_or_else(Account::empty);
    acc.code_hash = code_hash;
    world.accounts.put(&addr, &acc);
    world.code.insert(addr, code.to_vec());
    Ok(())
}

/// Validate using `tx.deploy` + `wasm.meter` (intrinsic only).
pub fn prepare(tx: &Tx, deploy: &Deploy) -> Result<(), RejectReason> {
    let _ = meter(tx).map_err(RejectReason::from)?;
    validate_wasm(&deploy.code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::genesis::Genesis;
    use types::{Amount, ChainId, GAS_DEPLOY};

    fn trivial_wasm() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (import "host" "sload" (func $sload (param i32) (result i64)))
                (import "host" "sstore" (func $sstore (param i32 i64)))
                (import "host" "reenter" (func $reenter))
                (func (export "call"))
            )"#,
        )
        .unwrap()
    }

    #[test]
    fn deploy_happy_path() {
        let mut world = World::from_genesis(&Genesis::new(ChainId::new(1)));
        let from = Address::from_bytes([9u8; 32]);
        let code = trivial_wasm();
        let tx = Tx::deploy(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_DEPLOY,
            Amount::ZERO,
            code.clone(),
        );
        prepare(&tx, tx.as_deploy().unwrap()).unwrap();
        let addr = create_address(&from, Nonce::ZERO);
        install(&mut world, addr, &code).unwrap();
        assert_eq!(world.code.get(&addr).unwrap(), &code);
        assert_ne!(world.account(&addr).code_hash, Hash::ZERO);
    }

    #[test]
    fn invalid_bytecode_before_install() {
        let tx = Tx::deploy(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_DEPLOY,
            Amount::ZERO,
            vec![0xff, 0x00],
        );
        assert_eq!(
            prepare(&tx, tx.as_deploy().unwrap()),
            Err(RejectReason::WasmInvalid)
        );
        let mut world = World::from_genesis(&Genesis::new(ChainId::new(1)));
        let addr = create_address(&Address::ZERO, Nonce::ZERO);
        assert_eq!(
            install(&mut world, addr, &[0xff, 0x00]),
            Err(RejectReason::WasmInvalid)
        );
        assert!(world.code.is_empty());
    }
}
