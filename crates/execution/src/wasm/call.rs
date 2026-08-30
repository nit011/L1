//! Invoke a deployed contract (architecture.md §3 / §4.1).
//!
//! # Frozen reentrancy policy
//!
//! **No reentrancy.** While a contract address is executing, any attempt
//! to enter [`execute`] for that same address (directly or via the `host.reenter`
//! import, including a call chain that returns to this contract) is rejected
//! with [`RejectReason::WasmReentrancy`]. Enforcement is a per-contract
//! membership test on [`World::executing`], checked on every [`execute`] entry.
//! This is the frozen reentrancy policy for this chain; it must not change
//! silently (Tier 20 / external contract authors rely on it).

use crate::receipt::RejectReason;
use crate::seq::World;
use crate::wasm::deploy;
use crate::wasm::host::{self, Host};
use types::tx::Call;
use types::Address;
use wasmtime::{Engine, Linker, Module, Store};

fn engine() -> Engine {
    let mut c = wasmtime::Config::new();
    c.consume_fuel(true);
    Engine::new(&c).expect("wasmtime engine")
}

fn map_trap(err: wasmtime::Error) -> RejectReason {
    let s = format!("{err:#}").to_ascii_lowercase();
    if s.contains("reentrancy") {
        RejectReason::WasmReentrancy
    } else if s.contains("fuel") {
        RejectReason::WasmGas
    } else {
        RejectReason::WasmInvalid
    }
}

fn host_reenter(caller: wasmtime::Caller<'_, Host>) -> Result<(), wasmtime::Error> {
    let (ptr, contract) = {
        let h = caller.data();
        (h.world, h.contract)
    };
    let world = unsafe { &mut *ptr };
    match execute(world, contract, 0) {
        Err(RejectReason::WasmReentrancy) => Err(wasmtime::Error::msg("reentrancy")),
        Err(other) => Err(wasmtime::Error::msg(format!("{other:?}"))),
        Ok(()) => Ok(()),
    }
}

/// Run the `call` export of `contract`.
///
/// Loads bytecode previously stored by [`deploy::install`]. Wires
/// [`host::sstore`] (and `sload`) as the only storage ABI.
pub fn execute(world: &mut World, contract: Address, fuel: u64) -> Result<(), RejectReason> {
    if world.executing.contains(&contract) {
        return Err(RejectReason::WasmReentrancy);
    }
    if !world.code.contains_key(&contract) {
        return Err(RejectReason::WasmNoCode);
    }
    world.executing.insert(contract);
    let result = run_engine(world, contract, fuel);
    world.executing.remove(&contract);
    result
}

fn run_engine(world: &mut World, contract: Address, fuel: u64) -> Result<(), RejectReason> {
    let code = world
        .code
        .get(&contract)
        .cloned()
        .ok_or(RejectReason::WasmNoCode)?;
    let engine = engine();
    let module = Module::new(&engine, &code).map_err(|_| RejectReason::WasmInvalid)?;
    let mut store = Store::new(
        &engine,
        Host {
            world: world as *mut World,
            contract,
        },
    );
    store
        .set_fuel(fuel)
        .map_err(|_| RejectReason::WasmInvalid)?;
    let mut linker = Linker::new(&engine);
    linker
        .func_wrap("host", "sload", host::host_sload)
        .map_err(|_| RejectReason::WasmInvalid)?;
    linker
        .func_wrap("host", "sstore", host::host_sstore)
        .map_err(|_| RejectReason::WasmInvalid)?;
    linker
        .func_wrap("host", "reenter", host_reenter)
        .map_err(|_| RejectReason::WasmInvalid)?;
    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|_| RejectReason::WasmInvalid)?;
    let func = instance
        .get_typed_func::<(), ()>(&mut store, "call")
        .map_err(|_| RejectReason::WasmInvalid)?;
    match func.call(&mut store, ()) {
        Ok(()) => {
            world.wasm_fuel_left = store.get_fuel().unwrap_or(0);
            Ok(())
        }
        Err(e) => {
            world.wasm_fuel_left = store.get_fuel().unwrap_or(0);
            Err(map_trap(e))
        }
    }
}

/// Apply `tx.call` against a contract installed by `wasm.deploy`.
pub fn call(world: &mut World, payload: &Call, fuel: u64) -> Result<(), RejectReason> {
    deploy::validate_wasm(
        world
            .code
            .get(&payload.to)
            .ok_or(RejectReason::WasmNoCode)?,
    )?;
    execute(world, payload.to, fuel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm::deploy::{create_address, install};
    use types::genesis::Genesis;
    use types::{ChainId, Nonce};

    fn store_wat() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (import "host" "sload" (func $sload (param i32) (result i64)))
                (import "host" "sstore" (func $sstore (param i32 i64)))
                (import "host" "reenter" (func $reenter))
                (func (export "call")
                    (call $sstore (i32.const 0) (i64.const 42))
                )
            )"#,
        )
        .unwrap()
    }

    fn loop_wat() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (import "host" "sload" (func $sload (param i32) (result i64)))
                (import "host" "sstore" (func $sstore (param i32 i64)))
                (import "host" "reenter" (func $reenter))
                (func (export "call")
                    (loop $l (br $l))
                )
            )"#,
        )
        .unwrap()
    }

    fn reenter_wat() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (import "host" "sload" (func $sload (param i32) (result i64)))
                (import "host" "sstore" (func $sstore (param i32 i64)))
                (import "host" "reenter" (func $reenter))
                (func (export "call")
                    (call $reenter)
                )
            )"#,
        )
        .unwrap()
    }

    #[test]
    fn call_sstore_happy() {
        let mut world = World::from_genesis(&Genesis::new(ChainId::new(1)));
        let addr = create_address(&Address::ZERO, Nonce::ZERO);
        install(&mut world, addr, &store_wat()).unwrap();
        let payload = Call {
            to: addr,
            data: vec![],
        };
        call(&mut world, &payload, 100_000).unwrap();
        assert_eq!(host::sload(&mut world, addr, 0), 42);
    }

    #[test]
    fn missing_code() {
        let mut world = World::from_genesis(&Genesis::new(ChainId::new(1)));
        let payload = Call {
            to: Address::ZERO,
            data: vec![],
        };
        assert_eq!(
            call(&mut world, &payload, 1_000),
            Err(RejectReason::WasmNoCode)
        );
    }

    #[test]
    fn reentrancy_rejected() {
        let mut world = World::from_genesis(&Genesis::new(ChainId::new(1)));
        let addr = create_address(&Address::ZERO, Nonce::ZERO);
        install(&mut world, addr, &reenter_wat()).unwrap();
        let err = execute(&mut world, addr, 100_000).unwrap_err();
        assert_eq!(err, RejectReason::WasmReentrancy);
        for _ in 0..8 {
            assert_eq!(
                execute(&mut world, addr, 100_000).unwrap_err(),
                RejectReason::WasmReentrancy
            );
        }
    }

    #[test]
    fn gas_exhaustion_deterministic() {
        let mut world = World::from_genesis(&Genesis::new(ChainId::new(1)));
        let addr = create_address(&Address::ZERO, Nonce::ZERO);
        install(&mut world, addr, &loop_wat()).unwrap();
        let mut fuels = Vec::new();
        for _ in 0..8 {
            world.wasm_fuel_left = u64::MAX;
            let e = execute(&mut world, addr, 1_000).unwrap_err();
            assert_eq!(e, RejectReason::WasmGas);
            fuels.push(world.wasm_fuel_left);
        }
        assert!(fuels.iter().all(|f| *f == fuels[0]));
        assert_eq!(fuels[0], 0);
    }
}
