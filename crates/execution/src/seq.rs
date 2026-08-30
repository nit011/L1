//! Sequential state transition (development-plan.md §0).
//!
//! # Frozen `apply_block` signature
//!
//! `apply_block(pre_state, block) -> (post_state, receipts, app_hash)`
//!
//! Tier 7 (Block-STM) and Tier 11 (WASM) **must reproduce this output exactly**
//! for the same inputs.
//!
//! Pipeline per tx: `tx.verify_ed25519` → `nonce_check` → `balance_check` →
//! `gas_meter` → mutate `state.account_trie`.
//!
//! # Frozen `exec.app_hash`
//!
//! `app_hash = blake3(state_root || tx_root || receipts_root)` (96-byte
//! concatenation, **no** domain tag). Each root is 32 bytes in that order.

use crate::checks::{balance_check, nonce_check, value_balance_check};
use crate::events::Event;
use crate::gas::gas_meter;
use crate::receipt::{Receipt, RejectReason};
use crate::staking::{self, StakingState};
use crypto::address::from_ed25519;
use crypto::hash::blake3::hash_to_array;
use crypto::sig::ed25519::public_key_from_bytes;
use crypto::tx::verify_ed25519;
use state::account::Account;
use state::root::commit_tries;
use state::tries::{AccountTrie, ContractStorageTrie};
use state::version::VersionedSlots;
use storage::memory::MemoryStore;
use types::block::{receipts_root, tx_root_signed, Block};
use types::collections::{Map, Set};
use types::genesis::Genesis;
use types::tx::SignedTx;
use types::{Address, Amount, Hash, ParamsRegistry};

/// In-memory world state (accounts + contract storage + WASM code).
#[derive(Clone, Debug, Default)]
pub struct World {
    /// Accounts MPT.
    pub accounts: AccountTrie,
    /// Contract storage (architecture.md §4.1).
    pub storage: ContractStorageTrie,
    /// Staking ledger (architecture.md §2.5). Not part of `state.commit_root` in this tier.
    pub staking: StakingState,
    /// `spec.params_registry` copy used by staking admission.
    pub params: ParamsRegistry,
    /// Installed bytecode keyed by contract address (`wasm.deploy`).
    pub code: Map<Address, Vec<u8>>,
    /// Per-contract execution guards (frozen no-reentrancy policy).
    pub executing: Set<Address>,
    /// Storage keys read by host `sload` during the current tx (STM RW-set).
    pub storage_reads: Set<Vec<u8>>,
    /// Storage keys written by host `sstore` during the current tx.
    pub storage_writes: Set<Vec<u8>>,
    /// Versioned overlay for host storage (`state.versioned_slot.*`).
    pub versioned: VersionedSlots<MemoryStore>,
    /// Remaining wasmtime fuel after the last WASM run (tests / determinism).
    pub wasm_fuel_left: u64,
}

impl World {
    /// Load genesis allocations through `state.account`.
    pub fn from_genesis(g: &Genesis) -> Self {
        let mut accounts = AccountTrie::new();
        for (addr, ga) in &g.alloc {
            accounts.put(addr, &Account::from_genesis(ga));
        }
        Self {
            accounts,
            storage: ContractStorageTrie::new(),
            staking: StakingState::default(),
            params: g.params.registry.clone(),
            code: Map::new(),
            executing: Set::new(),
            storage_reads: Set::new(),
            storage_writes: Set::new(),
            versioned: VersionedSlots::new(MemoryStore::new()),
            wasm_fuel_left: 0,
        }
    }

    /// Lookup an account (empty EOA if missing).
    pub fn account(&self, addr: &Address) -> Account {
        self.accounts.get(addr).unwrap_or_else(Account::empty)
    }

    /// `state.commit_root` (and the types `block.state_root` combination).
    pub fn commit_state_root(&self) -> Hash {
        let a = self.accounts.root();
        let c = self.storage.root();
        let via_commit = Hash::from_bytes(commit_tries(&self.accounts, &self.storage));
        let via_header = types::block::state_root(&a, &c);
        debug_assert_eq!(via_commit, via_header);
        via_commit
    }
}

/// `exec.app_hash` from the three roots.
pub fn app_hash(state_root: &Hash, tx_root: &Hash, receipts_root: &Hash) -> Hash {
    let mut buf = [0u8; 96];
    buf[0..32].copy_from_slice(state_root.as_bytes());
    buf[32..64].copy_from_slice(tx_root.as_bytes());
    buf[64..96].copy_from_slice(receipts_root.as_bytes());
    Hash::from_bytes(hash_to_array(&buf))
}

fn sender_address(signed: &SignedTx) -> Result<Address, RejectReason> {
    let pk = public_key_from_bytes(&signed.public_key).map_err(|_| RejectReason::Signature)?;
    Ok(from_ed25519(&pk))
}

fn fail(reason: RejectReason) -> Receipt {
    Receipt {
        success: false,
        gas_used: 0,
        events: vec![],
        reason: Some(reason),
    }
}

/// Per-tx transition. Contract: `exec.seq.apply_tx` / `exec.seq.apply_tx.wasm`.
///
/// WASM deploy/call ride the same signature → nonce → `gas_meter` →
/// balance path as transfers. They do not skip those checks.
pub fn apply_tx(world: &mut World, signed: &SignedTx) -> Receipt {
    world.storage_reads.clear();
    world.storage_writes.clear();
    world.executing.clear();
    if verify_ed25519(signed).is_err() {
        return fail(RejectReason::Signature);
    }
    let from = match sender_address(signed) {
        Ok(a) => a,
        Err(r) => return fail(r),
    };
    let tx = &signed.tx;
    let sender = world.accounts.get(&from).unwrap_or_else(Account::empty);
    if let Err(e) = nonce_check(tx, &sender) {
        return fail(e.into());
    }
    let gas = match gas_meter(tx) {
        Ok(g) => g,
        Err(e) => return fail(e.into()),
    };

    if let Some(stake) = tx.as_stake() {
        let debit = match stake.kind {
            types::staking::StakeKind::Bond | types::staking::StakeKind::Delegate => stake.amount,
            types::staking::StakeKind::Unbond
            | types::staking::StakeKind::Undelegate
            | types::staking::StakeKind::Withdraw => Amount::ZERO,
        };
        if let Err(e) = value_balance_check(tx, debit, &sender) {
            return fail(e.into());
        }
        match staking::apply_stake_tx(&mut world.staking, &world.params, &from, tx) {
            Ok(()) => {}
            Err(r) => return fail(r),
        }
        let mut sender = sender;
        let pay = debit.checked_add(tx.max_fee).expect("checked");
        sender.balance = sender.balance.checked_sub(pay).expect("checked");
        if stake.kind == types::staking::StakeKind::Withdraw {
            sender.balance = sender
                .balance
                .checked_add(stake.amount)
                .unwrap_or(sender.balance);
        }
        sender.nonce = sender.nonce.checked_add(1).unwrap_or(sender.nonce);
        world.accounts.put(&from, &sender);
        return Receipt {
            success: true,
            gas_used: gas,
            events: vec![Event::Stake {
                from,
                amount: stake.amount,
            }],
            reason: None,
        };
    }

    if let Some(deploy) = tx.as_deploy() {
        if let Err(e) = value_balance_check(tx, Amount::ZERO, &sender) {
            return fail(e.into());
        }
        if crate::wasm::deploy::prepare(tx, deploy).is_err() {
            return fail(RejectReason::WasmInvalid);
        }
        let snapshot = world.clone();
        let mut sender = sender;
        sender.balance = sender.balance.checked_sub(tx.max_fee).expect("checked");
        sender.nonce = sender.nonce.checked_add(1).unwrap_or(sender.nonce);
        world.accounts.put(&from, &sender);
        let addr = crate::wasm::deploy::create_address(&from, tx.nonce);
        match crate::wasm::deploy::install(world, addr, &deploy.code) {
            Ok(()) => Receipt {
                success: true,
                gas_used: gas,
                events: vec![Event::Wasm {
                    from,
                    contract: addr,
                }],
                reason: None,
            },
            Err(r) => {
                *world = snapshot;
                fail(r)
            }
        }
    } else if let Some(call) = tx.as_call() {
        if let Err(e) = value_balance_check(tx, Amount::ZERO, &sender) {
            return fail(e.into());
        }
        if !world.code.contains_key(&call.to) {
            return fail(RejectReason::WasmNoCode);
        }
        let snapshot = world.clone();
        let mut sender = sender;
        sender.balance = sender.balance.checked_sub(tx.max_fee).expect("checked");
        sender.nonce = sender.nonce.checked_add(1).unwrap_or(sender.nonce);
        world.accounts.put(&from, &sender);
        let fuel = crate::wasm::gas::fuel_budget(tx, gas);
        match crate::wasm::call::call(world, call, fuel) {
            Ok(()) => Receipt {
                success: true,
                gas_used: gas,
                events: vec![Event::Wasm {
                    from,
                    contract: call.to,
                }],
                reason: None,
            },
            Err(r) => {
                *world = snapshot;
                fail(r)
            }
        }
    } else {
        let Some(transfer) = tx.as_transfer() else {
            return fail(RejectReason::Gas);
        };
        if let Err(e) = balance_check(tx, transfer, &sender) {
            return fail(e.into());
        }

        let mut sender = sender;
        sender.balance = sender
            .balance
            .checked_sub(transfer.amount.checked_add(tx.max_fee).expect("checked"))
            .expect("checked");
        sender.nonce = sender.nonce.checked_add(1).unwrap_or(sender.nonce);
        world.accounts.put(&from, &sender);

        let mut recv = world
            .accounts
            .get(&transfer.to)
            .unwrap_or_else(Account::empty);
        recv.balance = recv
            .balance
            .checked_add(transfer.amount)
            .unwrap_or(recv.balance);
        world.accounts.put(&transfer.to, &recv);

        Receipt {
            success: true,
            gas_used: gas,
            events: vec![Event::Transfer {
                from,
                to: transfer.to,
                amount: transfer.amount,
            }],
            reason: None,
        }
    }
}

/// Canonical block application. Contract: `exec.seq.apply_block`.
pub fn apply_block(pre: World, block: &Block) -> (World, Vec<Receipt>, Hash) {
    let mut world = pre;
    let mut receipts = Vec::with_capacity(block.txs.len());
    for signed in &block.txs {
        receipts.push(apply_tx(&mut world, signed));
    }
    let encoded: Vec<Vec<u8>> = receipts.iter().map(|r| r.encode()).collect();
    let tx_r = tx_root_signed(&block.txs);
    let rec_root = receipts_root(&encoded);
    let st = world.commit_state_root();
    let app = app_hash(&st, &tx_r, &rec_root);
    (world, receipts, app)
}

/// Convenience: also return the three roots used in `app_hash`.
pub fn apply_block_with_roots(
    pre: World,
    block: &Block,
) -> (World, Vec<Receipt>, Hash, Hash, Hash, Hash) {
    let mut world = pre;
    let mut receipts = Vec::with_capacity(block.txs.len());
    for signed in &block.txs {
        receipts.push(apply_tx(&mut world, signed));
    }
    let encoded: Vec<Vec<u8>> = receipts.iter().map(|r| r.encode()).collect();
    let tx_r = tx_root_signed(&block.txs);
    let rec_r = receipts_root(&encoded);
    let st = world.commit_state_root();
    let app = app_hash(&st, &tx_r, &rec_r);
    (world, receipts, app, st, tx_r, rec_r)
}

/// Events from a receipt (contract `exec.events`).
pub fn events_of(receipt: &Receipt) -> &[Event] {
    &receipt.events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::address::from_ed25519;
    use crypto::sig::ed25519::SecretKey;
    use crypto::tx::sign;
    use types::genesis::GenesisAccount;
    use types::header::HeaderFields;
    use types::{Amount, ChainId, Height, Nonce, Round, TestClock, ValidatorId, GAS_TRANSFER};

    fn sk_from(b: u8) -> SecretKey {
        SecretKey::from_bytes(&[b; 32])
    }

    #[test]
    fn apply_tx_happy_and_bad_nonce() {
        let sk = sk_from(1);
        let from = from_ed25519(&sk.verifying_key());
        let to = Address::from_bytes([9u8; 32]);
        let mut g = Genesis::new(ChainId::new(1));
        g.insert_alloc(
            from,
            GenesisAccount {
                balance: Amount::new(1_000),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        let mut world = World::from_genesis(&g);
        let tx = types::tx::Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            to,
            Amount::new(10),
        );
        let signed = sign(&sk, tx);
        let r = apply_tx(&mut world, &signed);
        assert!(r.success);
        assert_eq!(events_of(&r).len(), 1);

        let tx2 = types::tx::Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::new(1),
            to,
            Amount::new(1),
        );
        let signed2 = sign(&sk, tx2);
        let r2 = apply_tx(&mut world, &signed2);
        assert!(!r2.success);
        assert_eq!(r2.reason, Some(RejectReason::WrongNonce));
    }

    #[test]
    fn apply_block_empty() {
        let g = Genesis::new(ChainId::new(1));
        let world = World::from_genesis(&g);
        let clock = TestClock::new(1_000);
        let fields = HeaderFields::new(
            &clock,
            Height::GENESIS,
            Round::ZERO,
            ValidatorId::ZERO,
            0,
            1,
        )
        .unwrap();
        let block = Block {
            header_fields: fields,
            txs: vec![],
        };
        let (w2, rec, app) = apply_block(world.clone(), &block);
        assert!(rec.is_empty());
        let app2 = apply_block(world, &block).2;
        assert_eq!(app, app2);
        let _ = w2;
    }

    #[test]
    fn genesis_from_bls_and_timeout_config() {
        let sk = crypto::sig::bls::keygen().unwrap();
        let (id, power) = crypto::from_bls(&sk.sk_to_pk(), types::VotingPower(3));
        let mut g = Genesis::new(ChainId::new(1));
        g.insert_validator(id, power);
        let cfg = consensus::timeout::TimeoutConfig::from_spec();
        assert_eq!(
            g.params.timeouts.propose_ms,
            cfg.duration_ms(consensus::timeout::TimeoutStep::Propose, Round::ZERO)
        );
        let ga = GenesisAccount {
            balance: Amount::new(1),
            nonce: Nonce::ZERO,
            code_hash: Hash::ZERO,
        };
        let acct = Account::from_genesis(&ga);
        assert_eq!(acct.balance, Amount::new(1));
        g.insert_alloc(Address::ZERO, ga);
        let _ = World::from_genesis(&g);
    }

    #[test]
    fn tx_root_matches_state_merkle() {
        let tx = types::tx::Tx::transfer(
            ChainId::new(1),
            Nonce::ZERO,
            GAS_TRANSFER,
            Amount::ZERO,
            Address::ZERO,
            Amount::ZERO,
        );
        let a = types::block::tx_root(std::slice::from_ref(&tx));
        let b = Hash::from_bytes(state::merkle::compute_root(&[tx.encode()]));
        assert_eq!(a, b);
    }

    #[test]
    fn header_hash_uses_header_domain() {
        let clock = TestClock::new(1_000);
        let fields = HeaderFields::new(
            &clock,
            Height::GENESIS,
            Round::ZERO,
            ValidatorId::ZERO,
            0,
            1,
        )
        .unwrap();
        let h = types::header::Header {
            fields,
            tx_root: Hash::ZERO,
            state_root: Hash::ZERO,
            receipts_root: Hash::ZERO,
            validators_hash: Hash::ZERO,
            da_root: types::header::DA_ROOT_PLACEHOLDER,
        };
        let tagged = crypto::apply_domain(crypto::DomainTag::Header, &h.hash_preimage());
        assert_eq!(h.hash(), Hash::from_bytes(hash_to_array(&tagged)));
    }

    fn store_wasm() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (import "host" "sload" (func $sload (param i32) (result i64)))
                (import "host" "sstore" (func $sstore (param i32 i64)))
                (import "host" "reenter" (func $reenter))
                (func (export "call")
                    (local $v i64)
                    (local.set $v (i64.add (call $sload (i32.const 0)) (i64.const 1)))
                    (call $sstore (i32.const 0) (local.get $v))
                )
            )"#,
        )
        .unwrap()
    }

    #[test]
    fn apply_tx_wasm_deploy_call_and_invalid() {
        let sk = sk_from(1);
        let from = from_ed25519(&sk.verifying_key());
        let mut g = Genesis::new(ChainId::new(1));
        g.insert_alloc(
            from,
            GenesisAccount {
                balance: Amount::new(1_000_000),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        let mut world = World::from_genesis(&g);
        let bad = types::tx::Tx::deploy(
            ChainId::new(1),
            Nonce::ZERO,
            types::GAS_DEPLOY,
            Amount::new(1),
            vec![0, 1, 2],
        );
        let r_bad = apply_tx(&mut world, &sign(&sk, bad));
        assert!(!r_bad.success);
        assert_eq!(r_bad.reason, Some(RejectReason::WasmInvalid));
        assert_eq!(world.account(&from).nonce, Nonce::ZERO);

        let tx = types::tx::Tx::deploy(
            ChainId::new(1),
            Nonce::ZERO,
            types::GAS_DEPLOY,
            Amount::new(1),
            store_wasm(),
        );
        let r = apply_tx(&mut world, &sign(&sk, tx));
        assert!(r.success);
        let addr = crate::wasm::deploy::create_address(&from, Nonce::ZERO);
        let call = types::tx::Tx::call(
            ChainId::new(1),
            Nonce(1),
            types::GAS_CALL + 50_000,
            Amount::new(1),
            addr,
            vec![],
        );
        let r2 = apply_tx(&mut world, &sign(&sk, call));
        assert!(r2.success);
        assert_eq!(crate::wasm::host::sload(&mut world, addr, 0), 1);
    }

    #[test]
    fn apply_tx_wasm_still_checks_nonce() {
        let sk = sk_from(1);
        let from = from_ed25519(&sk.verifying_key());
        let mut g = Genesis::new(ChainId::new(1));
        g.insert_alloc(
            from,
            GenesisAccount {
                balance: Amount::new(1_000_000),
                nonce: Nonce::ZERO,
                code_hash: Hash::ZERO,
            },
        );
        let mut world = World::from_genesis(&g);
        let tx = types::tx::Tx::deploy(
            ChainId::new(1),
            Nonce(9),
            types::GAS_DEPLOY,
            Amount::new(1),
            store_wasm(),
        );
        let r = apply_tx(&mut world, &sign(&sk, tx));
        assert!(!r.success);
        assert_eq!(r.reason, Some(RejectReason::WrongNonce));
    }
}
