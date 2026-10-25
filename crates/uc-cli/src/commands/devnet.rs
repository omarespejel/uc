//! In-process Starknet devnet: declares classes, deploys contracts and executes invoke
//! transactions on the same Sierra runner `uc test` uses, with an account, a fee token and a
//! universal deployer compiled from embedded Cairo sources by uc's own compiler.

use super::*;
use cairo_lang_runner::casm_run::contract_address::calculate_contract_address;
use cairo_lang_runner::{Arg, ClassInfoMap as OrderedHashMap, DevnetExecutionContext, RunResultValue, SierraCasmRunner, StarknetState};
use cairo_lang_sierra::program::Program;
use cairo_lang_starknet::contract::ContractInfo;
use starknet_types_core::felt::Felt;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

pub(crate) const DEVNET_STRK_ADDRESS: &str = "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d";
pub(crate) const DEVNET_ETH_ADDRESS: &str = "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7";
pub(crate) const DEVNET_UDC_ADDRESS: &str = "0x041a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf";
const DEVNET_SPEC_VERSION: &str = "0.8.1";
const EXECUTION_GAS_BUDGET: usize = 2_000_000_000;

/// The three system contracts every devnet starts with, as Cairo 2.18 sources.
const SYSTEM_CONTRACTS_CAIRO: &str = r#"
use starknet::ContractAddress;

#[starknet::interface]
pub trait IDevnetAccount<T> {
    fn __validate__(self: @T, calls: Array<starknet::account::Call>) -> felt252;
    fn __execute__(ref self: T, calls: Array<starknet::account::Call>) -> Array<Span<felt252>>;
    fn __validate_declare__(self: @T, class_hash: felt252) -> felt252;
    fn __validate_deploy__(self: @T, class_hash: felt252, salt: felt252, public_key: felt252) -> felt252;
    fn is_valid_signature(self: @T, hash: felt252, signature: Array<felt252>) -> felt252;
    fn get_public_key(self: @T) -> felt252;
    fn set_public_key(ref self: T, new_public_key: felt252);
}

#[starknet::contract(account)]
pub mod DevnetAccount {
    use core::num::traits::Zero;
    use starknet::account::Call;
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use starknet::syscalls::call_contract_syscall;
    use starknet::{get_caller_address, get_contract_address};

    #[storage]
    struct Storage {
        public_key: felt252,
    }

    #[constructor]
    fn constructor(ref self: ContractState, public_key: felt252) {
        self.public_key.write(public_key);
    }

    #[abi(embed_v0)]
    impl AccountImpl of super::IDevnetAccount<ContractState> {
        fn __validate__(self: @ContractState, calls: Array<Call>) -> felt252 {
            starknet::VALIDATED
        }

        fn __execute__(ref self: ContractState, calls: Array<Call>) -> Array<Span<felt252>> {
            assert(get_caller_address().is_zero(), 'INVALID_CALLER');
            let mut results = array![];
            for call in calls {
                match call_contract_syscall(call.to, call.selector, call.calldata) {
                    Result::Ok(result) => results.append(result),
                    Result::Err(revert) => core::panics::panic(revert),
                }
            }
            results
        }

        fn __validate_declare__(self: @ContractState, class_hash: felt252) -> felt252 {
            starknet::VALIDATED
        }

        fn __validate_deploy__(self: @ContractState, class_hash: felt252, salt: felt252, public_key: felt252) -> felt252 {
            starknet::VALIDATED
        }

        fn is_valid_signature(self: @ContractState, hash: felt252, signature: Array<felt252>) -> felt252 {
            starknet::VALIDATED
        }

        fn get_public_key(self: @ContractState) -> felt252 {
            self.public_key.read()
        }

        fn set_public_key(ref self: ContractState, new_public_key: felt252) {
            assert(get_caller_address() == get_contract_address(), 'ONLY_SELF');
            self.public_key.write(new_public_key);
        }
    }
}

#[starknet::interface]
pub trait IDevnetToken<T> {
    fn name(self: @T) -> felt252;
    fn symbol(self: @T) -> felt252;
    fn decimals(self: @T) -> u8;
    fn total_supply(self: @T) -> u256;
    fn totalSupply(self: @T) -> u256;
    fn balance_of(self: @T, account: ContractAddress) -> u256;
    fn balanceOf(self: @T, account: ContractAddress) -> u256;
    fn allowance(self: @T, owner: ContractAddress, spender: ContractAddress) -> u256;
    fn transfer(ref self: T, recipient: ContractAddress, amount: u256) -> bool;
    fn transfer_from(ref self: T, sender: ContractAddress, recipient: ContractAddress, amount: u256) -> bool;
    fn transferFrom(ref self: T, sender: ContractAddress, recipient: ContractAddress, amount: u256) -> bool;
    fn approve(ref self: T, spender: ContractAddress, amount: u256) -> bool;
    fn mint(ref self: T, recipient: ContractAddress, amount: u256);
}

#[starknet::contract]
pub mod DevnetToken {
    use starknet::ContractAddress;
    use starknet::get_caller_address;
    use starknet::storage::{
        Map, StorageMapReadAccess, StorageMapWriteAccess, StoragePointerReadAccess,
        StoragePointerWriteAccess,
    };

    #[storage]
    struct Storage {
        name: felt252,
        symbol: felt252,
        total_supply: u256,
        balances: Map<ContractAddress, u256>,
        allowances: Map<(ContractAddress, ContractAddress), u256>,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    pub enum Event {
        Transfer: Transfer,
        Approval: Approval,
    }

    #[derive(Drop, starknet::Event)]
    pub struct Transfer {
        #[key]
        pub from: ContractAddress,
        #[key]
        pub to: ContractAddress,
        pub value: u256,
    }

    #[derive(Drop, starknet::Event)]
    pub struct Approval {
        #[key]
        pub owner: ContractAddress,
        #[key]
        pub spender: ContractAddress,
        pub value: u256,
    }

    #[constructor]
    fn constructor(ref self: ContractState, name: felt252, symbol: felt252) {
        self.name.write(name);
        self.symbol.write(symbol);
    }

    #[abi(embed_v0)]
    impl TokenImpl of super::IDevnetToken<ContractState> {
        fn name(self: @ContractState) -> felt252 {
            self.name.read()
        }
        fn symbol(self: @ContractState) -> felt252 {
            self.symbol.read()
        }
        fn decimals(self: @ContractState) -> u8 {
            18
        }
        fn total_supply(self: @ContractState) -> u256 {
            self.total_supply.read()
        }
        fn totalSupply(self: @ContractState) -> u256 {
            self.total_supply.read()
        }
        fn balance_of(self: @ContractState, account: ContractAddress) -> u256 {
            self.balances.read(account)
        }
        fn balanceOf(self: @ContractState, account: ContractAddress) -> u256 {
            self.balances.read(account)
        }
        fn allowance(self: @ContractState, owner: ContractAddress, spender: ContractAddress) -> u256 {
            self.allowances.read((owner, spender))
        }
        fn transfer(ref self: ContractState, recipient: ContractAddress, amount: u256) -> bool {
            let sender = get_caller_address();
            self.move_tokens(sender, recipient, amount);
            true
        }
        fn transfer_from(
            ref self: ContractState, sender: ContractAddress, recipient: ContractAddress, amount: u256,
        ) -> bool {
            self.spend_allowance(sender, get_caller_address(), amount);
            self.move_tokens(sender, recipient, amount);
            true
        }
        fn transferFrom(
            ref self: ContractState, sender: ContractAddress, recipient: ContractAddress, amount: u256,
        ) -> bool {
            self.spend_allowance(sender, get_caller_address(), amount);
            self.move_tokens(sender, recipient, amount);
            true
        }
        fn approve(ref self: ContractState, spender: ContractAddress, amount: u256) -> bool {
            let owner = get_caller_address();
            self.allowances.write((owner, spender), amount);
            self.emit(Approval { owner, spender, value: amount });
            true
        }
        fn mint(ref self: ContractState, recipient: ContractAddress, amount: u256) {
            self.total_supply.write(self.total_supply.read() + amount);
            self.balances.write(recipient, self.balances.read(recipient) + amount);
            self.emit(Transfer { from: 0.try_into().unwrap(), to: recipient, value: amount });
        }
    }

    #[generate_trait]
    impl Internal of InternalTrait {
        fn move_tokens(ref self: ContractState, from: ContractAddress, to: ContractAddress, amount: u256) {
            let from_balance = self.balances.read(from);
            assert(from_balance >= amount, 'INSUFFICIENT_BALANCE');
            self.balances.write(from, from_balance - amount);
            self.balances.write(to, self.balances.read(to) + amount);
            self.emit(Transfer { from, to, value: amount });
        }
        fn spend_allowance(ref self: ContractState, owner: ContractAddress, spender: ContractAddress, amount: u256) {
            let current = self.allowances.read((owner, spender));
            assert(current >= amount, 'INSUFFICIENT_ALLOWANCE');
            self.allowances.write((owner, spender), current - amount);
        }
    }
}

#[starknet::interface]
pub trait IDevnetDeployer<T> {
    fn deployContract(
        ref self: T, class_hash: starknet::ClassHash, salt: felt252, unique: felt252, calldata: Span<felt252>,
    ) -> ContractAddress;
    fn deploy_contract(
        ref self: T, class_hash: starknet::ClassHash, salt: felt252, unique: felt252, calldata: Span<felt252>,
    ) -> ContractAddress;
}

#[starknet::contract]
pub mod DevnetDeployer {
    use core::pedersen::pedersen;
    use starknet::syscalls::deploy_syscall;
    use starknet::{ClassHash, ContractAddress, get_caller_address};

    #[storage]
    struct Storage {}

    #[event]
    #[derive(Drop, starknet::Event)]
    pub enum Event {
        ContractDeployed: ContractDeployed,
    }

    #[derive(Drop, starknet::Event)]
    pub struct ContractDeployed {
        pub address: ContractAddress,
        pub deployer: ContractAddress,
        pub unique: felt252,
        pub class_hash: ClassHash,
        pub salt: felt252,
    }

    #[abi(embed_v0)]
    impl DeployerImpl of super::IDevnetDeployer<ContractState> {
        fn deployContract(
            ref self: ContractState, class_hash: ClassHash, salt: felt252, unique: felt252, calldata: Span<felt252>,
        ) -> ContractAddress {
            self.deploy_impl(class_hash, salt, unique, calldata)
        }
        fn deploy_contract(
            ref self: ContractState, class_hash: ClassHash, salt: felt252, unique: felt252, calldata: Span<felt252>,
        ) -> ContractAddress {
            self.deploy_impl(class_hash, salt, unique, calldata)
        }
    }

    #[generate_trait]
    impl Internal of InternalTrait {
        fn deploy_impl(
            ref self: ContractState, class_hash: ClassHash, salt: felt252, unique: felt252, calldata: Span<felt252>,
        ) -> ContractAddress {
            let deployer = get_caller_address();
            let (effective_salt, from_zero) = if unique == 0 {
                (salt, true)
            } else {
                (pedersen(deployer.into(), salt), false)
            };
            let (address, _) = deploy_syscall(class_hash, effective_salt, calldata, from_zero).unwrap();
            self.emit(ContractDeployed { address, deployer, unique, class_hash, salt });
            address
        }
    }
}
"#;

/// The compiled system classes.
pub(crate) struct DevnetSystemClasses {
    pub account: serde_json::Value,
    pub token: serde_json::Value,
    pub deployer: serde_json::Value,
}

/// Compiles the embedded system contracts once per compiler version into uc's cache
/// (`<cache>/devnet-system-v<cairo version>/`) and loads their class files.
pub(crate) fn ensure_devnet_system_classes() -> Result<DevnetSystemClasses> {
    let version = native_cairo_lang_compiler_version();
    let dir = native_global_cache_root_dir()
        .unwrap_or_else(|| std::env::temp_dir().join("uc-global-cache"))
        .join(format!("devnet-system-v{version}"));
    fs::create_dir_all(dir.join("src"))?;
    let manifest = format!(
        "[package]\nname = \"uc_devnet_system\"\nversion = \"0.1.0\"\nedition = \"2024_07\"\n\n[dependencies]\nstarknet = \"{version}\"\n\n[[target.starknet-contract]]\nsierra = true\ncasm = false\n"
    );
    let write_if_changed = |path: PathBuf, content: &str| -> Result<()> {
        if fs::read_to_string(&path).ok().as_deref() != Some(content) {
            fs::write(&path, content)?;
        }
        Ok(())
    };
    write_if_changed(dir.join("Scarb.toml"), &manifest)?;
    write_if_changed(dir.join("src/lib.cairo"), SYSTEM_CONTRACTS_CAIRO)?;
    let load = |name: &str| -> Result<serde_json::Value> {
        let path = dir.join(format!("target/dev/uc_devnet_system_{name}.contract_class.json"));
        let text = fs::read_to_string(&path).with_context(|| format!("missing devnet system class {}", path.display()))?;
        Ok(serde_json::from_str(&text)?)
    };
    let all_present = ["DevnetAccount", "DevnetToken", "DevnetDeployer"]
        .iter()
        .all(|n| dir.join(format!("target/dev/uc_devnet_system_{n}.contract_class.json")).is_file());
    if !all_present || fs::metadata(dir.join("src/lib.cairo")).and_then(|m| m.modified()).ok()
        > fs::metadata(dir.join("target/dev/uc_devnet_system_DevnetAccount.contract_class.json")).and_then(|m| m.modified()).ok()
    {
        run_build(BuildArgs {
            common: BuildCommonArgs {
                manifest_path: Some(dir.join("Scarb.toml")),
                package: None,
                workspace: false,
                features: Vec::new(),
                offline: true,
                release: false,
                profile: Some("dev".to_string()),
            },
            engine: EngineArg::Uc,
            daemon_mode: DaemonModeArg::Off,
            report_path: Some(dir.join("build.json")),
            record_failure: None,
            json: false,
            plan_only: false,
        })
        .context("failed to compile the devnet system contracts")?;
    }
    Ok(DevnetSystemClasses { account: load("DevnetAccount")?, token: load("DevnetToken")?, deployer: load("DevnetDeployer")? })
}

/// A declared Sierra class with everything execution needs.
struct DeclaredClass {
    class_json: serde_json::Value,
    program: Program,
    declared_in_block: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DevnetBlock {
    pub block_number: u64,
    pub block_hash: String,
    pub parent_hash: String,
    pub timestamp: u64,
    pub sequencer_address: String,
    pub transactions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DevnetTransaction {
    pub transaction_hash: String,
    pub tx_type: String,
    pub block_number: u64,
    pub block_hash: String,
    pub transaction: serde_json::Value,
    pub receipt: serde_json::Value,
    pub trace: serde_json::Value,
}

/// A JSON-RPC error as the Starknet spec numbers them.
#[derive(Debug)]
pub(crate) struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), data: None }
    }
    fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }
}

const ERR_CONTRACT_NOT_FOUND: i64 = 20;
const ERR_BLOCK_NOT_FOUND: i64 = 24;
const ERR_INVALID_TX_INDEX: i64 = 27;
const ERR_CLASS_HASH_NOT_FOUND: i64 = 28;
const ERR_TX_HASH_NOT_FOUND: i64 = 29;
const ERR_CONTRACT_ERROR: i64 = 40;
const ERR_TX_EXECUTION_ERROR: i64 = 41;
const ERR_INVALID_NONCE: i64 = 52;
const ERR_CLASS_ALREADY_DECLARED: i64 = 51;
const ERR_UNSUPPORTED: i64 = -32601;
const ERR_INVALID_PARAMS: i64 = -32602;

/// Outcome of running one entry point.
struct ExecutionOutcome {
    retdata: Vec<Felt>,
    events: Vec<(Felt, Vec<Felt>, Vec<Felt>)>,
    messages: Vec<(Felt, Felt, Vec<Felt>)>,
    steps: usize,
    builtins: BTreeMap<String, usize>,
    panic: Option<Vec<Felt>>,
}

/// The chain: classes, state, blocks and transactions.
pub(crate) struct DevnetChain {
    classes: HashMap<Felt, DeclaredClass>,
    infos: OrderedHashMap<Felt, ContractInfo>,
    runners: Arc<RwLock<HashMap<Felt, Arc<SierraCasmRunner>>>>,
    state: StarknetState,
    nonces: HashMap<Felt, u64>,
    blocks: Vec<DevnetBlock>,
    txs: Vec<DevnetTransaction>,
    tx_index: HashMap<String, usize>,
    pending: Vec<String>,
    chain_id: Felt,
    timestamp: u64,
    sequencer: Felt,
    block_on_demand: bool,
    pub accounts: Vec<DevnetPredeployedAccount>,
    pub account_class_hash: String,
    pub token_class_hash: String,
    pub deployer_class_hash: String,
}

fn felt(s: &str) -> Result<Felt, RpcError> {
    parse_felt_str(s).map_err(|e| RpcError::new(ERR_INVALID_PARAMS, format!("invalid felt `{s}`: {e}")))
}

fn hex(f: &Felt) -> String {
    format_felt_hex(f)
}

fn hex_vec(v: &[Felt]) -> Vec<String> {
    v.iter().map(hex).collect()
}

fn felt_from_digest(domain: &str, parts: &[&[u8]]) -> Felt {
    let mut h = blake3::Hasher::new();
    h.update(domain.as_bytes());
    for p in parts {
        h.update(p);
    }
    let digest = h.finalize();
    let mut bytes = [0u8; 32];
    bytes[1..32].copy_from_slice(&digest.as_bytes()[0..31]);
    Felt::from_bytes_be(&bytes)
}

fn u64_from_json(v: Option<&serde_json::Value>) -> Option<u64> {
    v.and_then(|x| x.as_u64().or_else(|| x.as_str().and_then(|s| parse_felt_str(s).ok()).and_then(|f| u64::try_from(f.to_biguint()).ok())))
}

/// Encodes a `u256` amount (decimal or hex string, at most 2^128 - 1 on a devnet) as
/// `(low, high)` felts.
fn u256_parts(raw: &str) -> Result<(Felt, Felt), RpcError> {
    let digits = raw.trim();
    let value = if let Some(h) = digits.strip_prefix("0x") {
        u128::from_str_radix(h, 16).ok()
    } else {
        digits.parse::<u128>().ok()
    }
    .ok_or_else(|| RpcError::new(ERR_INVALID_PARAMS, format!("invalid amount `{raw}` (devnet amounts must fit u128)")))?;
    Ok((Felt::from(value), Felt::ZERO))
}

/// Decimal string of a `u256` returned as `(low, high)` felts.
fn u256_to_decimal(low: &Felt, high: &Felt) -> String {
    (low.to_biguint() + (high.to_biguint() << 128usize)).to_string()
}

impl DevnetChain {
    /// Genesis: system classes declared, fee tokens at their mainnet addresses, the universal
    /// deployer at its mainnet address, and `accounts` funded with `initial_balance` of each.
    pub(crate) fn genesis(
        system: &DevnetSystemClasses,
        accounts: Vec<DevnetPredeployedAccount>,
        chain_id: Felt,
        start_time: u64,
        block_on_demand: bool,
    ) -> Result<Self> {
        let mut chain = Self {
            classes: HashMap::new(),
            infos: OrderedHashMap::default(),
            runners: Arc::new(RwLock::new(HashMap::new())),
            state: StarknetState::default(),
            nonces: HashMap::new(),
            blocks: Vec::new(),
            txs: Vec::new(),
            tx_index: HashMap::new(),
            pending: Vec::new(),
            chain_id,
            timestamp: start_time,
            sequencer: Felt::from(0x1000_u64),
            block_on_demand,
            accounts,
            account_class_hash: String::new(),
            token_class_hash: String::new(),
            deployer_class_hash: String::new(),
        };
        let account_class = chain.declare(system.account.clone(), 0).map_err(|e| anyhow!("{}", e.message))?;
        let token_class = chain.declare(system.token.clone(), 0).map_err(|e| anyhow!("{}", e.message))?;
        let deployer_class = chain.declare(system.deployer.clone(), 0).map_err(|e| anyhow!("{}", e.message))?;
        chain.account_class_hash = hex(&account_class);
        chain.token_class_hash = hex(&token_class);
        chain.deployer_class_hash = hex(&deployer_class);

        let strk = parse_felt_str(DEVNET_STRK_ADDRESS)?;
        let eth = parse_felt_str(DEVNET_ETH_ADDRESS)?;
        let udc = parse_felt_str(DEVNET_UDC_ADDRESS)?;
        let short = |s: &str| Felt::from_bytes_be_slice(s.as_bytes());
        chain.deploy_at(strk, token_class, &[short("StarkNet Token"), short("STRK")], Felt::ZERO)?;
        chain.deploy_at(eth, token_class, &[short("Ether"), short("ETH")], Felt::ZERO)?;
        chain.deploy_at(udc, deployer_class, &[], Felt::ZERO)?;
        let accounts = chain.accounts.clone();
        for acc in &accounts {
            let address = parse_felt_str(&acc.address)?;
            let public_key = parse_felt_str(&acc.public_key)?;
            chain.deploy_at(address, account_class, &[public_key], Felt::ZERO)?;
            let (low, high) = u256_parts(&acc.initial_balance).map_err(|e| anyhow!("{}", e.message))?;
            for token in [strk, eth] {
                let out = chain.execute(token, selector("mint"), vec![address, low, high], Felt::ZERO, &DevnetExecutionContext::default())?;
                if let Some(p) = out.panic {
                    bail!("genesis mint failed: {}", cairo_lang_runner::casm_run::format_for_panic(p.into_iter()));
                }
            }
        }
        chain.state.take_events();
        chain.seal_block();
        Ok(chain)
    }

    fn deploy_at(&mut self, address: Felt, class_hash: Felt, calldata: &[Felt], caller: Felt) -> Result<()> {
        self.state.register_deployed_contract(address, class_hash);
        let info = self.infos.get(&class_hash).ok_or_else(|| anyhow!("class {} not declared", hex(&class_hash)))?.clone();
        if let Some(ctor) = &info.constructor {
            let out = self.run_function(class_hash, ctor.clone(), address, calldata.to_vec(), caller, &DevnetExecutionContext::default())?;
            if let Some(p) = out.panic {
                self.state.register_deployed_contract(address, Felt::ZERO);
                bail!("constructor of {} panicked: {}", hex(&address), cairo_lang_runner::casm_run::format_for_panic(p.into_iter()));
            }
        }
        Ok(())
    }

    /// Declares a class from its `.contract_class.json` document. Returns the class hash.
    fn declare(&mut self, class_json: serde_json::Value, block: u64) -> Result<Felt, RpcError> {
        let class: ContractClass = serde_json::from_value(normalize_class_json(class_json.clone()))
            .map_err(|e| RpcError::new(ERR_INVALID_PARAMS, format!("not a Sierra contract class: {e}")))?;
        let class_hash_hex = compute_starknet_sierra_class_hash(&class)
            .ok_or_else(|| RpcError::new(ERR_INVALID_PARAMS, "could not hash the class"))?;
        let class_hash = felt(&class_hash_hex)?;
        if self.classes.contains_key(&class_hash) {
            return Err(RpcError::new(ERR_CLASS_ALREADY_DECLARED, "Class already declared"));
        }
        let program = class
            .extract_sierra_program(true)
            .map_err(|e| RpcError::new(ERR_INVALID_PARAMS, format!("failed to extract the Sierra program: {e}")))?
            .program;
        let fn_id = |idx: usize| program.funcs.get(idx).map(|f| f.id.clone());
        let mut info = ContractInfo { constructor: None, externals: Default::default(), l1_handlers: Default::default() };
        for ep in &class.entry_points_by_type.external {
            if let Some(id) = fn_id(ep.function_idx) {
                info.externals.insert(Felt::from(&ep.selector), id);
            }
        }
        for ep in &class.entry_points_by_type.l1_handler {
            if let Some(id) = fn_id(ep.function_idx) {
                info.l1_handlers.insert(Felt::from(&ep.selector), id);
            }
        }
        if let Some(ep) = class.entry_points_by_type.constructor.first() {
            info.constructor = fn_id(ep.function_idx);
        }
        self.infos.insert(class_hash, info);
        self.classes.insert(class_hash, DeclaredClass { class_json, program, declared_in_block: block });
        self.rebuild_runners().map_err(|e| RpcError::new(ERR_CONTRACT_ERROR, format!("failed to prepare class runners: {e}")))?;
        Ok(class_hash)
    }

    /// Every class gets a runner that knows every class's entry points, so calls cross classes.
    fn rebuild_runners(&mut self) -> Result<()> {
        let mut fresh = HashMap::new();
        for (hash, declared) in &self.classes {
            let runner = SierraCasmRunner::new_with_shared_classes(
                declared.program.clone(),
                None,
                self.infos.clone(),
                Arc::clone(&self.runners),
            )
            .map_err(|e| anyhow!("class {}: {e}", hex(hash)))?;
            fresh.insert(*hash, Arc::new(runner));
        }
        *self.runners.write().unwrap_or_else(|e| e.into_inner()) = fresh;
        Ok(())
    }

    fn runner_for(&self, class_hash: &Felt) -> Option<Arc<SierraCasmRunner>> {
        self.runners.read().ok()?.get(class_hash).cloned()
    }

    fn execution_context(&self, contract: Felt, caller: Felt, tx: &DevnetExecutionContext) -> DevnetExecutionContext {
        DevnetExecutionContext {
            contract_address: contract,
            caller_address: caller,
            account_contract_address: tx.account_contract_address,
            transaction_hash: tx.transaction_hash,
            nonce: tx.nonce,
            version: tx.version,
            chain_id: self.chain_id,
            signature: tx.signature.clone(),
            block_number: Felt::from(self.next_block_number()),
            block_timestamp: Felt::from(self.timestamp),
            sequencer_address: self.sequencer,
        }
    }

    fn run_function(
        &mut self,
        class_hash: Felt,
        function: cairo_lang_sierra::ids::FunctionId,
        contract: Felt,
        calldata: Vec<Felt>,
        caller: Felt,
        tx: &DevnetExecutionContext,
    ) -> Result<ExecutionOutcome> {
        let runner = self.runner_for(&class_hash).ok_or_else(|| anyhow!("no runner for class {}", hex(&class_hash)))?;
        let func = runner
            .function_by_id(&function)
            .map_err(|e| anyhow!("entry point not in class program: {e}"))?;
        let mut state = self.state.clone();
        state.set_execution_context(&self.execution_context(contract, caller, tx));
        let res = runner
            .run_function_with_starknet_context(
                func,
                vec![Arg::Array(calldata.into_iter().map(Arg::Value).collect())],
                Some(EXECUTION_GAS_BUDGET),
                state,
            )
            .map_err(|e| anyhow!("runner error: {e}"))?;
        let steps = res.used_resources.basic_resources.n_steps;
        let builtins: BTreeMap<String, usize> = res
            .used_resources
            .basic_resources
            .builtin_instance_counter
            .iter()
            .filter(|(_, n)| **n > 0)
            .map(|(k, v)| (k.to_str().to_string(), *v))
            .collect();
        match res.value {
            RunResultValue::Success(value) => {
                let mut new_state = res.starknet_state;
                let events = new_state.take_events();
                let messages = new_state.take_l2_to_l1_messages();
                self.state = new_state;
                let retdata = match res.return_array {
                    Some(data) => data,
                    None => cairo_lang_runner::casm_run::read_array_result_as_vec(&res.memory, &value),
                };
                Ok(ExecutionOutcome { retdata, events, messages, steps, builtins, panic: None })
            }
            RunResultValue::Panic(data) => Ok(ExecutionOutcome {
                retdata: Vec::new(),
                events: Vec::new(),
                messages: Vec::new(),
                steps,
                builtins,
                panic: Some(data),
            }),
        }
    }

    /// Runs the external entry point `selector` of the contract at `contract`.
    fn execute(&mut self, contract: Felt, selector_felt: Felt, calldata: Vec<Felt>, caller: Felt, tx: &DevnetExecutionContext) -> Result<ExecutionOutcome> {
        let class_hash = self.state.class_hash_at(&contract).ok_or_else(|| anyhow!("CONTRACT_NOT_FOUND"))?;
        let info = self.infos.get(&class_hash).ok_or_else(|| anyhow!("class {} not declared", hex(&class_hash)))?;
        let function = info
            .externals
            .get(&selector_felt)
            .or_else(|| info.l1_handlers.get(&selector_felt))
            .cloned()
            .ok_or_else(|| anyhow!("ENTRYPOINT_NOT_FOUND"))?;
        self.run_function(class_hash, function, contract, calldata, caller, tx)
    }

    fn next_block_number(&self) -> u64 {
        self.blocks.len() as u64
    }

    fn seal_block(&mut self) -> &DevnetBlock {
        let number = self.next_block_number();
        let parent_hash = self.blocks.last().map(|b| b.block_hash.clone()).unwrap_or_else(|| "0x0".to_string());
        let hash = felt_from_digest("uc-devnet-block:", &[&number.to_le_bytes(), parent_hash.as_bytes(), &self.timestamp.to_le_bytes()]);
        let txs = std::mem::take(&mut self.pending);
        let block = DevnetBlock {
            block_number: number,
            block_hash: hex(&hash),
            parent_hash,
            timestamp: self.timestamp,
            sequencer_address: hex(&self.sequencer),
            transactions: txs.clone(),
        };
        for t in &txs {
            if let Some(i) = self.tx_index.get(t).copied() {
                self.txs[i].block_number = number;
                self.txs[i].block_hash = block.block_hash.clone();
                self.txs[i].receipt["block_number"] = serde_json::json!(number);
                self.txs[i].receipt["block_hash"] = serde_json::json!(block.block_hash);
            }
        }
        self.blocks.push(block);
        self.blocks.last().unwrap()
    }

    fn record_tx(&mut self, tx_type: &str, tx_hash: Felt, transaction: serde_json::Value, outcome: Option<&ExecutionOutcome>, revert: Option<String>, extra: serde_json::Value) -> String {
        let hash_hex = hex(&tx_hash);
        let (events, messages, steps, builtins) = match outcome {
            Some(o) => (
                o.events.iter().map(|(from, keys, data)| serde_json::json!({ "from_address": hex(from), "keys": hex_vec(keys), "data": hex_vec(data) })).collect::<Vec<_>>(),
                o.messages.iter().map(|(from, to, payload)| serde_json::json!({ "from_address": hex(from), "to_address": hex(to), "payload": hex_vec(payload) })).collect::<Vec<_>>(),
                o.steps,
                o.builtins.clone(),
            ),
            None => (Vec::new(), Vec::new(), 0, BTreeMap::new()),
        };
        let mut receipt = serde_json::json!({
            "type": tx_type,
            "transaction_hash": hash_hex,
            "actual_fee": { "amount": "0x0", "unit": "FRI" },
            "execution_status": if revert.is_some() { "REVERTED" } else { "SUCCEEDED" },
            "finality_status": "ACCEPTED_ON_L2",
            "block_number": self.next_block_number(),
            "block_hash": "0x0",
            "messages_sent": messages,
            "events": events,
            "execution_resources": {
                "steps": steps,
                "l1_gas": 0, "l1_data_gas": 0, "l2_gas": steps * 100,
                "builtins": builtins,
            },
        });
        if let Some(r) = &revert {
            receipt["revert_reason"] = serde_json::json!(r);
        }
        if let serde_json::Value::Object(map) = extra {
            for (k, v) in map {
                receipt[k] = v;
            }
        }
        let trace = serde_json::json!({
            "type": tx_type,
            "execute_invocation": transaction.get("execute_invocation").cloned().unwrap_or(serde_json::Value::Null),
            "state_diff": serde_json::Value::Null,
        });
        let mut tx_json = transaction;
        if let serde_json::Value::Object(map) = &mut tx_json {
            map.remove("execute_invocation");
            map.insert("transaction_hash".into(), serde_json::json!(hash_hex));
        }
        let record = DevnetTransaction {
            transaction_hash: hash_hex.clone(),
            tx_type: tx_type.to_string(),
            block_number: self.next_block_number(),
            block_hash: "0x0".to_string(),
            transaction: tx_json,
            receipt,
            trace,
        };
        self.tx_index.insert(hash_hex.clone(), self.txs.len());
        self.txs.push(record);
        self.pending.push(hash_hex.clone());
        if !self.block_on_demand {
            self.seal_block();
        }
        hash_hex
    }

    fn check_nonce(&self, sender: &Felt, given: Option<&serde_json::Value>) -> Result<u64, RpcError> {
        let expected = self.nonces.get(sender).copied().unwrap_or(0);
        let given = u64_from_json(given).unwrap_or(expected);
        if given != expected {
            return Err(RpcError::new(ERR_INVALID_NONCE, format!("Invalid transaction nonce: expected {expected}, got {given}")));
        }
        Ok(expected)
    }

    fn tx_hash_for(&self, kind: &str, sender: &Felt, nonce: u64, body: &serde_json::Value) -> Felt {
        felt_from_digest("uc-devnet-tx:", &[kind.as_bytes(), &sender.to_bytes_be(), &nonce.to_le_bytes(), body.to_string().as_bytes(), &self.chain_id.to_bytes_be()])
    }

    /// The SNIP-8 hash of a well-formed V3 transaction, else a digest of its fields.
    fn v3_hash_or_digest(&self, kind: &str, sender: &Felt, nonce: u64, tx: &serde_json::Value, fallback: &serde_json::Value) -> Felt {
        let mut normalized = tx.clone();
        normalized["type"] = serde_json::json!(kind);
        normalized["version"] = tx.get("version").cloned().unwrap_or(serde_json::json!("0x3"));
        normalized["nonce"] = serde_json::json!(format!("{nonce:#x}"));
        if kind != "DEPLOY_ACCOUNT" {
            normalized["sender_address"] = serde_json::json!(hex(sender));
        }
        v3_transaction_hash(&normalized, self.chain_id).unwrap_or_else(|_| self.tx_hash_for(kind, sender, nonce, fallback))
    }

    /// Checks a transaction's signature against the account's public key when both exist.
    /// Empty signatures are accepted (a devnet convenience); a wrong one is refused.
    fn check_signature(&mut self, tx_hash: &Felt, signature: &[Felt], public_key: Option<Felt>) -> Result<(), RpcError> {
        if signature.is_empty() {
            return Ok(());
        }
        let Some(public_key) = public_key else { return Ok(()) };
        if signature.len() != 2 || !verify_transaction_signature(&public_key, tx_hash, &signature[0], &signature[1]) {
            return Err(RpcError::new(ERR_TX_EXECUTION_ERROR, "invalid signature").with_data(serde_json::json!({
                "transaction_hash": hex(tx_hash),
                "public_key": hex(&public_key),
            })));
        }
        Ok(())
    }

    /// The public key the embedded account at `address` reports, if it is one.
    fn account_public_key(&mut self, address: Felt) -> Option<Felt> {
        let snapshot = self.state.clone();
        let out = self.execute(address, selector("get_public_key"), Vec::new(), Felt::ZERO, &DevnetExecutionContext::default()).ok();
        self.state = snapshot;
        out.filter(|o| o.panic.is_none()).and_then(|o| o.retdata.first().copied())
    }

    fn add_declare(&mut self, tx: &serde_json::Value) -> Result<serde_json::Value, RpcError> {
        let sender = felt(tx.get("sender_address").and_then(|v| v.as_str()).unwrap_or("0x1"))?;
        let nonce = self.check_nonce(&sender, tx.get("nonce"))?;
        let class_json = tx.get("contract_class").cloned().ok_or_else(|| RpcError::new(ERR_INVALID_PARAMS, "contract_class is required"))?;
        let block = self.next_block_number();
        let signature = felts_of_json(tx.get("signature"));
        let class_hash = self.declare(class_json, block)?;
        let mut for_hash = tx.clone();
        for_hash["class_hash"] = serde_json::json!(hex(&class_hash));
        for_hash.as_object_mut().map(|m| m.remove("contract_class"));
        let tx_hash = self.v3_hash_or_digest("DECLARE", &sender, nonce, &for_hash, &serde_json::json!({ "class_hash": hex(&class_hash) }));
        let public_key = self.account_public_key(sender);
        if let Err(e) = self.check_signature(&tx_hash, &signature, public_key) {
            self.undeclare(&class_hash);
            return Err(e);
        }
        self.nonces.insert(sender, nonce + 1);
        let record = serde_json::json!({
            "type": "DECLARE",
            "version": tx.get("version").cloned().unwrap_or(serde_json::json!("0x3")),
            "sender_address": hex(&sender),
            "class_hash": hex(&class_hash),
            "compiled_class_hash": tx.get("compiled_class_hash").cloned().unwrap_or(serde_json::json!("0x0")),
            "nonce": format!("{nonce:#x}"),
            "signature": tx.get("signature").cloned().unwrap_or(serde_json::json!([])),
        });
        let hash_hex = self.record_tx("DECLARE", tx_hash, record, None, None, serde_json::json!({}));
        Ok(serde_json::json!({ "transaction_hash": hash_hex, "class_hash": hex(&class_hash) }))
    }

    fn add_deploy_account(&mut self, tx: &serde_json::Value) -> Result<serde_json::Value, RpcError> {
        let class_hash = felt(tx.get("class_hash").and_then(|v| v.as_str()).unwrap_or("0x0"))?;
        let salt = felt(tx.get("contract_address_salt").and_then(|v| v.as_str()).unwrap_or("0x0"))?;
        let calldata: Vec<Felt> = tx
            .get("constructor_calldata")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().and_then(|s| parse_felt_str(s).ok())).collect())
            .unwrap_or_default();
        if !self.classes.contains_key(&class_hash) {
            return Err(RpcError::new(ERR_CLASS_HASH_NOT_FOUND, "Class hash not found"));
        }
        let address = calculate_contract_address(&salt, &class_hash, &calldata, &Felt::ZERO);
        if self.state.class_hash_at(&address).is_some_and(|c| c != Felt::ZERO) {
            return Err(RpcError::new(ERR_TX_EXECUTION_ERROR, format!("contract {} is already deployed", hex(&address))));
        }
        let nonce = self.check_nonce(&address, tx.get("nonce"))?;
        let tx_hash = self.v3_hash_or_digest("DEPLOY_ACCOUNT", &address, nonce, tx, &serde_json::json!({ "class_hash": hex(&class_hash), "salt": hex(&salt) }));
        // The embedded account takes its public key as the first constructor argument.
        let signature = felts_of_json(tx.get("signature"));
        let public_key = (class_hash == parse_felt_str(&self.account_class_hash).unwrap_or_default()).then(|| calldata.first().copied()).flatten();
        self.check_signature(&tx_hash, &signature, public_key)?;
        let ctx = DevnetExecutionContext { account_contract_address: address, transaction_hash: tx_hash, nonce: Felt::from(nonce), version: Felt::from(3_u64), ..Default::default() };
        self.state.register_deployed_contract(address, class_hash);
        let info = self.infos.get(&class_hash).cloned().unwrap();
        let outcome = match &info.constructor {
            Some(ctor) => Some(self.run_function(class_hash, ctor.clone(), address, calldata.clone(), Felt::ZERO, &ctx).map_err(|e| RpcError::new(ERR_TX_EXECUTION_ERROR, e.to_string()))?),
            None => None,
        };
        let revert = outcome.as_ref().and_then(|o| o.panic.as_ref()).map(|p| cairo_lang_runner::casm_run::format_for_panic(p.clone().into_iter()));
        if revert.is_some() {
            self.state.register_deployed_contract(address, Felt::ZERO);
        }
        self.nonces.insert(address, nonce + 1);
        let record = serde_json::json!({
            "type": "DEPLOY_ACCOUNT",
            "version": tx.get("version").cloned().unwrap_or(serde_json::json!("0x3")),
            "class_hash": hex(&class_hash),
            "contract_address_salt": hex(&salt),
            "constructor_calldata": hex_vec(&calldata),
            "nonce": format!("{nonce:#x}"),
            "signature": tx.get("signature").cloned().unwrap_or(serde_json::json!([])),
            "execute_invocation": { "contract_address": hex(&address), "entry_point_selector": hex(&selector("constructor")), "calldata": hex_vec(&calldata), "result": [], "calls": [] },
        });
        let hash_hex = self.record_tx("DEPLOY_ACCOUNT", tx_hash, record, outcome.as_ref(), revert, serde_json::json!({ "contract_address": hex(&address) }));
        Ok(serde_json::json!({ "transaction_hash": hash_hex, "contract_address": hex(&address) }))
    }

    fn add_invoke(&mut self, tx: &serde_json::Value) -> Result<serde_json::Value, RpcError> {
        let sender = felt(tx.get("sender_address").and_then(|v| v.as_str()).ok_or_else(|| RpcError::new(ERR_INVALID_PARAMS, "sender_address is required"))?)?;
        if self.state.class_hash_at(&sender).is_none() {
            return Err(RpcError::new(ERR_CONTRACT_NOT_FOUND, "Contract not found"));
        }
        let nonce = self.check_nonce(&sender, tx.get("nonce"))?;
        let calldata: Vec<Felt> = tx
            .get("calldata")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().and_then(|s| parse_felt_str(s).ok())).collect())
            .unwrap_or_default();
        let signature: Vec<Felt> = tx
            .get("signature")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().and_then(|s| parse_felt_str(s).ok())).collect())
            .unwrap_or_default();
        let tx_hash = self.v3_hash_or_digest("INVOKE", &sender, nonce, tx, &serde_json::json!({ "calldata": hex_vec(&calldata) }));
        let public_key = self.account_public_key(sender);
        self.check_signature(&tx_hash, &signature, public_key)?;
        let ctx = DevnetExecutionContext { account_contract_address: sender, transaction_hash: tx_hash, nonce: Felt::from(nonce), version: Felt::from(3_u64), signature, ..Default::default() };
        let outcome = self
            .execute(sender, selector("__execute__"), calldata.clone(), Felt::ZERO, &ctx)
            .map_err(|e| RpcError::new(ERR_TX_EXECUTION_ERROR, e.to_string()))?;
        let revert = outcome.panic.as_ref().map(|p| cairo_lang_runner::casm_run::format_for_panic(p.clone().into_iter()));
        self.nonces.insert(sender, nonce + 1);
        let record = serde_json::json!({
            "type": "INVOKE",
            "version": tx.get("version").cloned().unwrap_or(serde_json::json!("0x3")),
            "sender_address": hex(&sender),
            "calldata": hex_vec(&calldata),
            "nonce": format!("{nonce:#x}"),
            "signature": tx.get("signature").cloned().unwrap_or(serde_json::json!([])),
            "execute_invocation": {
                "contract_address": hex(&sender),
                "entry_point_selector": hex(&selector("__execute__")),
                "calldata": hex_vec(&calldata),
                "result": hex_vec(&outcome.retdata),
                "revert_reason": revert,
                "calls": [],
            },
        });
        let hash_hex = self.record_tx("INVOKE", tx_hash, record, Some(&outcome), revert, serde_json::json!({}));
        Ok(serde_json::json!({ "transaction_hash": hash_hex }))
    }

    fn call(&mut self, request: &serde_json::Value) -> Result<serde_json::Value, RpcError> {
        let contract = felt(request.get("contract_address").and_then(|v| v.as_str()).ok_or_else(|| RpcError::new(ERR_INVALID_PARAMS, "contract_address is required"))?)?;
        let selector_felt = felt(request.get("entry_point_selector").and_then(|v| v.as_str()).ok_or_else(|| RpcError::new(ERR_INVALID_PARAMS, "entry_point_selector is required"))?)?;
        let calldata: Vec<Felt> = request
            .get("calldata")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().and_then(|s| parse_felt_str(s).ok())).collect())
            .unwrap_or_default();
        if self.state.class_hash_at(&contract).is_none() {
            return Err(RpcError::new(ERR_CONTRACT_NOT_FOUND, "Contract not found"));
        }
        let snapshot = self.state.clone();
        let result = self.execute(contract, selector_felt, calldata, Felt::ZERO, &DevnetExecutionContext::default());
        self.state = snapshot;
        let outcome = result.map_err(|e| RpcError::new(ERR_CONTRACT_ERROR, "Contract error").with_data(serde_json::json!({ "revert_error": e.to_string() })))?;
        if let Some(p) = outcome.panic {
            let reason = cairo_lang_runner::casm_run::format_for_panic(p.into_iter());
            return Err(RpcError::new(ERR_CONTRACT_ERROR, "Contract error").with_data(serde_json::json!({ "revert_error": reason })));
        }
        Ok(serde_json::json!(hex_vec(&outcome.retdata)))
    }

    fn block_by_id(&self, block_id: Option<&serde_json::Value>) -> Result<&DevnetBlock, RpcError> {
        let latest = self.blocks.last().ok_or_else(|| RpcError::new(ERR_BLOCK_NOT_FOUND, "Block not found"))?;
        let Some(id) = block_id else { return Ok(latest) };
        if let Some(tag) = id.as_str() {
            return match tag {
                "latest" | "pending" | "pre_confirmed" | "l1_accepted" => Ok(latest),
                _ => Err(RpcError::new(ERR_BLOCK_NOT_FOUND, "Block not found")),
            };
        }
        if let Some(n) = id.get("block_number").and_then(|v| v.as_u64()) {
            return self.blocks.get(n as usize).ok_or_else(|| RpcError::new(ERR_BLOCK_NOT_FOUND, "Block not found"));
        }
        if let Some(h) = id.get("block_hash").and_then(|v| v.as_str()) {
            let wanted = parse_felt_str(h).ok();
            return self
                .blocks
                .iter()
                .find(|b| parse_felt_str(&b.block_hash).ok() == wanted)
                .ok_or_else(|| RpcError::new(ERR_BLOCK_NOT_FOUND, "Block not found"));
        }
        Ok(latest)
    }

    fn block_json(&self, block: &DevnetBlock, with_txs: bool) -> serde_json::Value {
        let txs: Vec<serde_json::Value> = block
            .transactions
            .iter()
            .filter_map(|h| self.tx_index.get(h).map(|i| &self.txs[*i]))
            .map(|t| if with_txs { t.transaction.clone() } else { serde_json::json!(t.transaction_hash) })
            .collect();
        serde_json::json!({
            "status": "ACCEPTED_ON_L2",
            "block_hash": block.block_hash,
            "parent_hash": block.parent_hash,
            "block_number": block.block_number,
            "new_root": "0x0",
            "timestamp": block.timestamp,
            "sequencer_address": block.sequencer_address,
            "l1_gas_price": { "price_in_fri": "0x1", "price_in_wei": "0x1" },
            "l1_data_gas_price": { "price_in_fri": "0x1", "price_in_wei": "0x1" },
            "l2_gas_price": { "price_in_fri": "0x1", "price_in_wei": "0x1" },
            "l1_da_mode": "BLOB",
            "starknet_version": DEVNET_SPEC_VERSION,
            "transactions": txs,
        })
    }

    fn tx_by_hash(&self, hash: &str) -> Result<&DevnetTransaction, RpcError> {
        let wanted = parse_felt_str(hash).ok();
        self.txs
            .iter()
            .find(|t| parse_felt_str(&t.transaction_hash).ok() == wanted)
            .ok_or_else(|| RpcError::new(ERR_TX_HASH_NOT_FOUND, "Transaction hash not found"))
    }

    fn get_events(&self, filter: &serde_json::Value) -> Result<serde_json::Value, RpcError> {
        let from = filter.get("from_block").map(|b| self.block_by_id(Some(b)).map(|b| b.block_number)).transpose()?.unwrap_or(0);
        let to = filter.get("to_block").map(|b| self.block_by_id(Some(b)).map(|b| b.block_number)).transpose()?.unwrap_or(u64::MAX);
        let address = filter.get("address").and_then(|v| v.as_str()).and_then(|s| parse_felt_str(s).ok());
        let keys: Vec<Vec<Felt>> = filter
            .get("keys")
            .and_then(|v| v.as_array())
            .map(|outer| outer.iter().map(|inner| inner.as_array().map(|a| a.iter().filter_map(|k| k.as_str().and_then(|s| parse_felt_str(s).ok())).collect()).unwrap_or_default()).collect())
            .unwrap_or_default();
        let chunk = filter.get("chunk_size").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
        let skip: usize = filter.get("continuation_token").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0);
        let mut all = Vec::new();
        for t in &self.txs {
            if t.block_number < from || t.block_number > to {
                continue;
            }
            for ev in t.receipt.get("events").and_then(|e| e.as_array()).into_iter().flatten() {
                let from_addr = ev.get("from_address").and_then(|v| v.as_str()).and_then(|s| parse_felt_str(s).ok());
                if let Some(a) = address {
                    if from_addr != Some(a) {
                        continue;
                    }
                }
                let ev_keys: Vec<Felt> = ev.get("keys").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|k| k.as_str().and_then(|s| parse_felt_str(s).ok())).collect()).unwrap_or_default();
                let keys_match = keys.iter().enumerate().all(|(i, allowed)| allowed.is_empty() || ev_keys.get(i).is_some_and(|k| allowed.contains(k)));
                if !keys_match {
                    continue;
                }
                let mut e = ev.clone();
                e["block_hash"] = serde_json::json!(t.block_hash);
                e["block_number"] = serde_json::json!(t.block_number);
                e["transaction_hash"] = serde_json::json!(t.transaction_hash);
                all.push(e);
            }
        }
        let page: Vec<serde_json::Value> = all.iter().skip(skip).take(chunk).cloned().collect();
        let mut out = serde_json::json!({ "events": page });
        if skip + chunk < all.len() {
            out["continuation_token"] = serde_json::json!((skip + chunk).to_string());
        }
        Ok(out)
    }

    fn mint(&mut self, params: &serde_json::Value) -> Result<serde_json::Value, RpcError> {
        let address = felt(params.get("address").and_then(|v| v.as_str()).ok_or_else(|| RpcError::new(ERR_INVALID_PARAMS, "address is required"))?)?;
        let amount = params.get("amount").map(|v| v.as_u64().map(|n| n.to_string()).unwrap_or_else(|| v.as_str().unwrap_or("0").to_string())).unwrap_or_else(|| "0".to_string());
        let unit = params.get("unit").and_then(|v| v.as_str()).unwrap_or("FRI").to_ascii_uppercase();
        let token = parse_felt_str(if unit == "WEI" { DEVNET_ETH_ADDRESS } else { DEVNET_STRK_ADDRESS }).unwrap();
        let (low, high) = u256_parts(&amount)?;
        let outcome = self
            .execute(token, selector("mint"), vec![address, low, high], Felt::ZERO, &DevnetExecutionContext::default())
            .map_err(|e| RpcError::new(ERR_TX_EXECUTION_ERROR, e.to_string()))?;
        if let Some(p) = outcome.panic {
            return Err(RpcError::new(ERR_TX_EXECUTION_ERROR, cairo_lang_runner::casm_run::format_for_panic(p.into_iter())));
        }
        let tx_hash = felt_from_digest("uc-devnet-mint:", &[&address.to_bytes_be(), amount.as_bytes(), &(self.txs.len() as u64).to_le_bytes()]);
        let hash_hex = self.record_tx("INVOKE", tx_hash, serde_json::json!({ "type": "INVOKE", "version": "0x3", "sender_address": "0x0", "calldata": [], "nonce": "0x0", "signature": [], "devnet_mint": { "address": hex(&address), "amount": amount, "unit": unit } }), Some(&outcome), None, serde_json::json!({}));
        let snapshot = self.state.clone();
        let balance = self.execute(token, selector("balanceOf"), vec![address], Felt::ZERO, &DevnetExecutionContext::default()).ok();
        self.state = snapshot;
        let new_balance = balance
            .and_then(|o| (o.retdata.len() == 2).then(|| u256_to_decimal(&o.retdata[0], &o.retdata[1])))
            .unwrap_or_else(|| "0".to_string());
        Ok(serde_json::json!({ "new_balance": new_balance, "unit": unit, "tx_hash": hash_hex }))
    }

    fn predeployed_accounts_json(&self) -> serde_json::Value {
        serde_json::json!(self.accounts.iter().map(|a| serde_json::json!({
            "address": a.address,
            "public_key": a.public_key,
            "private_key": a.private_key,
            "initial_balance": a.initial_balance,
            "class_hash": self.account_class_hash,
        })).collect::<Vec<_>>())
    }

    /// Dispatches one JSON-RPC method.
    pub(crate) fn handle_rpc(&mut self, method: &str, params: &serde_json::Value) -> Result<serde_json::Value, RpcError> {
        let p = |i: usize, name: &str| -> Option<serde_json::Value> {
            params.get(name).cloned().or_else(|| params.as_array().and_then(|a| a.get(i)).cloned())
        };
        let first_tx = || params.get("declare_transaction").or_else(|| params.get("deploy_account_transaction")).or_else(|| params.get("invoke_transaction")).cloned().or_else(|| params.as_array().and_then(|a| a.first()).cloned()).unwrap_or_default();
        match method {
            "starknet_specVersion" => Ok(serde_json::json!(DEVNET_SPEC_VERSION)),
            "starknet_chainId" => Ok(serde_json::json!(hex(&self.chain_id))),
            "starknet_syncing" => Ok(serde_json::json!(false)),
            "starknet_blockNumber" => Ok(serde_json::json!(self.blocks.len().saturating_sub(1))),
            "starknet_blockHashAndNumber" => {
                let b = self.block_by_id(None)?;
                Ok(serde_json::json!({ "block_hash": b.block_hash, "block_number": b.block_number }))
            }
            "starknet_getBlockWithTxHashes" => {
                let b = self.block_by_id(p(0, "block_id").as_ref())?.clone();
                Ok(self.block_json(&b, false))
            }
            "starknet_getBlockWithTxs" => {
                let b = self.block_by_id(p(0, "block_id").as_ref())?.clone();
                Ok(self.block_json(&b, true))
            }
            "starknet_getBlockTransactionCount" => Ok(serde_json::json!(self.block_by_id(p(0, "block_id").as_ref())?.transactions.len())),
            "starknet_getNonce" => {
                let address = felt(p(1, "contract_address").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?;
                if self.state.class_hash_at(&address).is_none() {
                    return Err(RpcError::new(ERR_CONTRACT_NOT_FOUND, "Contract not found"));
                }
                Ok(serde_json::json!(format!("{:#x}", self.nonces.get(&address).copied().unwrap_or(0))))
            }
            "starknet_getClassHashAt" => {
                let address = felt(p(1, "contract_address").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?;
                match self.state.class_hash_at(&address) {
                    Some(c) if c != Felt::ZERO => Ok(serde_json::json!(hex(&c))),
                    _ => Err(RpcError::new(ERR_CONTRACT_NOT_FOUND, "Contract not found")),
                }
            }
            "starknet_getClass" => {
                let class_hash = felt(p(1, "class_hash").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?;
                self.classes.get(&class_hash).map(|c| c.class_json.clone()).ok_or_else(|| RpcError::new(ERR_CLASS_HASH_NOT_FOUND, "Class hash not found"))
            }
            "starknet_getClassAt" => {
                let address = felt(p(1, "contract_address").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?;
                let class_hash = self.state.class_hash_at(&address).filter(|c| *c != Felt::ZERO).ok_or_else(|| RpcError::new(ERR_CONTRACT_NOT_FOUND, "Contract not found"))?;
                self.classes.get(&class_hash).map(|c| c.class_json.clone()).ok_or_else(|| RpcError::new(ERR_CLASS_HASH_NOT_FOUND, "Class hash not found"))
            }
            "starknet_getStorageAt" => {
                let address = felt(p(0, "contract_address").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?;
                let key = felt(p(1, "key").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?;
                if self.state.class_hash_at(&address).is_none() {
                    return Err(RpcError::new(ERR_CONTRACT_NOT_FOUND, "Contract not found"));
                }
                Ok(serde_json::json!(hex(&self.state.storage_at(&address, &key))))
            }
            "starknet_call" => {
                let request = p(0, "request").unwrap_or_default();
                self.call(&request)
            }
            "starknet_estimateFee" => {
                let txs = p(0, "request").and_then(|v| v.as_array().cloned()).unwrap_or_default();
                Ok(serde_json::json!(txs.iter().map(|_| serde_json::json!({
                    "l1_gas_consumed": "0x0", "l1_gas_price": "0x1",
                    "l2_gas_consumed": "0x186a0", "l2_gas_price": "0x1",
                    "l1_data_gas_consumed": "0x0", "l1_data_gas_price": "0x1",
                    "overall_fee": "0x186a0", "unit": "FRI"
                })).collect::<Vec<_>>()))
            }
            "starknet_estimateMessageFee" => Ok(serde_json::json!({ "l1_gas_consumed": "0x0", "l1_gas_price": "0x1", "l2_gas_consumed": "0x0", "l2_gas_price": "0x1", "l1_data_gas_consumed": "0x0", "l1_data_gas_price": "0x1", "overall_fee": "0x0", "unit": "FRI" })),
            "starknet_addDeclareTransaction" => {
                let tx = first_tx();
                self.add_declare(&tx)
            }
            "starknet_addDeployAccountTransaction" => {
                let tx = first_tx();
                self.add_deploy_account(&tx)
            }
            "starknet_addInvokeTransaction" => {
                let tx = first_tx();
                self.add_invoke(&tx)
            }
            "starknet_getTransactionByHash" => Ok(self.tx_by_hash(p(0, "transaction_hash").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?.transaction.clone()),
            "starknet_getTransactionReceipt" => Ok(self.tx_by_hash(p(0, "transaction_hash").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?.receipt.clone()),
            "starknet_getTransactionStatus" => {
                let t = self.tx_by_hash(p(0, "transaction_hash").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?;
                Ok(serde_json::json!({ "finality_status": "ACCEPTED_ON_L2", "execution_status": t.receipt["execution_status"] }))
            }
            "starknet_getTransactionByBlockIdAndIndex" => {
                let b = self.block_by_id(p(0, "block_id").as_ref())?;
                let idx = p(1, "index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let hash = b.transactions.get(idx).cloned().ok_or_else(|| RpcError::new(ERR_INVALID_TX_INDEX, "Invalid transaction index in a block"))?;
                Ok(self.tx_by_hash(&hash)?.transaction.clone())
            }
            "starknet_traceTransaction" => Ok(self.tx_by_hash(p(0, "transaction_hash").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?.trace.clone()),
            "starknet_getEvents" => {
                let filter = p(0, "filter").unwrap_or_default();
                self.get_events(&filter)
            }
            "starknet_getStateUpdate" => {
                let b = self.block_by_id(p(0, "block_id").as_ref())?;
                let declared: Vec<serde_json::Value> = self
                    .classes
                    .iter()
                    .filter(|(_, c)| c.declared_in_block == b.block_number && b.block_number > 0)
                    .map(|(h, _)| serde_json::json!({ "class_hash": hex(h), "compiled_class_hash": "0x0" }))
                    .collect();
                Ok(serde_json::json!({ "block_hash": b.block_hash, "new_root": "0x0", "old_root": "0x0", "state_diff": { "storage_diffs": [], "deprecated_declared_classes": [], "declared_classes": declared, "deployed_contracts": [], "replaced_classes": [], "nonces": [] } }))
            }
            "devnet_getPredeployedAccounts" | "devnet_predeployedAccounts" => Ok(self.predeployed_accounts_json()),
            "devnet_getConfig" => Ok(serde_json::json!({ "chain_id": hex(&self.chain_id), "block_generation_on": if self.block_on_demand { "demand" } else { "transaction" }, "account_class_hash": self.account_class_hash, "strk_fee_token_address": DEVNET_STRK_ADDRESS, "eth_fee_token_address": DEVNET_ETH_ADDRESS, "udc_address": DEVNET_UDC_ADDRESS })),
            "devnet_mint" => self.mint(params),
            "devnet_createBlock" => {
                let b = self.seal_block().clone();
                Ok(serde_json::json!({ "block_hash": b.block_hash, "block_number": b.block_number }))
            }
            "devnet_setTime" => {
                self.timestamp = params.get("time").and_then(|v| v.as_u64()).unwrap_or(self.timestamp);
                let generate = params.get("generate_block").and_then(|v| v.as_bool()).unwrap_or(true);
                let hash = if generate { Some(self.seal_block().block_hash.clone()) } else { None };
                Ok(serde_json::json!({ "block_timestamp": self.timestamp, "block_hash": hash }))
            }
            "devnet_increaseTime" => {
                self.timestamp += params.get("time").and_then(|v| v.as_u64()).unwrap_or(0);
                let hash = self.seal_block().block_hash.clone();
                Ok(serde_json::json!({ "timestamp": self.timestamp, "block_hash": hash }))
            }
            "starknet_simulateTransactions" | "starknet_traceBlockTransactions" | "starknet_getMessagesStatus" | "starknet_getStorageProof" => {
                Err(RpcError::new(ERR_UNSUPPORTED, format!("{method} is not implemented by uc devnet")))
            }
            _ => Err(RpcError::new(ERR_UNSUPPORTED, format!("Method not found: {method}"))),
        }
    }
}

fn felts_of_json(v: Option<&serde_json::Value>) -> Vec<Felt> {
    v.and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().and_then(|s| parse_felt_str(s).ok())).collect())
        .unwrap_or_default()
}

impl DevnetChain {
    fn undeclare(&mut self, class_hash: &Felt) {
        self.classes.remove(class_hash);
        self.infos.swap_remove(class_hash);
        let _ = self.rebuild_runners();
    }
}

/// `starknet_keccak(name)` as a felt.
pub(crate) fn selector(name: &str) -> Felt {
    Felt::from(&cairo_lang_starknet_classes::keccak::starknet_keccak(name.as_bytes()))
}

/// The RPC ships `abi` as a string; the class type wants the array.
fn normalize_class_json(mut v: serde_json::Value) -> serde_json::Value {
    if let Some(text) = v.get("abi").and_then(|a| a.as_str()) {
        let parsed = serde_json::from_str::<serde_json::Value>(text).unwrap_or(serde_json::Value::Array(Vec::new()));
        v["abi"] = parsed;
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_chain() -> DevnetChain {
        let system = ensure_devnet_system_classes().expect("system classes");
        let accounts = generate_devnet_predeployed_accounts(2, 7, "1000000000000000000000");
        DevnetChain::genesis(&system, accounts, Felt::from_bytes_be_slice(b"SN_SEPOLIA"), 1_700_000_000, false).expect("genesis")
    }

    fn inc_again(deployed: &str) -> Vec<String> {
        vec!["0x1".to_string(), deployed.to_string(), hex(&selector("increase")), "0x1".to_string(), "0x1".to_string()]
    }

    fn u256_of(retdata: &serde_json::Value) -> String {
        let a = retdata.as_array().unwrap();
        let low = parse_felt_str(a[0].as_str().unwrap()).unwrap();
        let high = parse_felt_str(a[1].as_str().unwrap()).unwrap();
        u256_to_decimal(&low, &high)
    }

    #[test]
    #[cfg(feature = "native-test-runner")]
    fn devnet_executes_declare_deploy_invoke_and_call_end_to_end() {
        let mut chain = test_chain();
        let acc = chain.accounts[0].clone();
        let other = chain.accounts[1].clone();
        let strk = DEVNET_STRK_ADDRESS;

        // Genesis funded the accounts.
        let bal = chain.handle_rpc("starknet_call", &serde_json::json!({ "request": { "contract_address": strk, "entry_point_selector": hex(&selector("balanceOf")), "calldata": [acc.address] }, "block_id": "latest" })).unwrap();
        assert_eq!(u256_of(&bal), "1000000000000000000000");
        assert_eq!(chain.handle_rpc("starknet_getNonce", &serde_json::json!({ "block_id": "latest", "contract_address": acc.address })).unwrap(), serde_json::json!("0x0"));

        // Invoke: account __execute__ -> STRK transfer of 5 to the other account.
        let transfer_calldata = vec![
            "0x1".to_string(), strk.to_string(), hex(&selector("transfer")), "0x3".to_string(),
            other.address.clone(), "0x5".to_string(), "0x0".to_string(),
        ];
        let res = chain.handle_rpc("starknet_addInvokeTransaction", &serde_json::json!({ "invoke_transaction": { "type": "INVOKE", "version": "0x3", "sender_address": acc.address, "calldata": transfer_calldata, "nonce": "0x0", "signature": [] } })).unwrap();
        let tx_hash = res["transaction_hash"].as_str().unwrap().to_string();
        let receipt = chain.handle_rpc("starknet_getTransactionReceipt", &serde_json::json!({ "transaction_hash": tx_hash })).unwrap();
        assert_eq!(receipt["execution_status"], "SUCCEEDED", "{receipt}");
        assert_eq!(receipt["block_number"], 1);
        let events = receipt["events"].as_array().unwrap();
        assert!(events.iter().any(|e| e["from_address"].as_str().map(|s| parse_felt_str(s).unwrap()) == Some(parse_felt_str(strk).unwrap())), "Transfer event expected: {events:?}");
        let bal_other = chain.handle_rpc("starknet_call", &serde_json::json!({ "request": { "contract_address": strk, "entry_point_selector": hex(&selector("balanceOf")), "calldata": [other.address] }, "block_id": "latest" })).unwrap();
        assert_eq!(u256_of(&bal_other), "1000000000000000000005");
        assert_eq!(chain.handle_rpc("starknet_getNonce", &serde_json::json!({ "block_id": "latest", "contract_address": acc.address })).unwrap(), serde_json::json!("0x1"));

        // A stale nonce is refused; a reverting call is reported with its reason.
        let stale = chain.handle_rpc("starknet_addInvokeTransaction", &serde_json::json!({ "invoke_transaction": { "sender_address": acc.address, "calldata": [], "nonce": "0x0" } }));
        assert_eq!(stale.unwrap_err().code, ERR_INVALID_NONCE);
        let too_much = vec!["0x1".to_string(), strk.to_string(), hex(&selector("transfer")), "0x3".to_string(), other.address.clone(), "0x0".to_string(), "0x1000".to_string()];
        let res = chain.handle_rpc("starknet_addInvokeTransaction", &serde_json::json!({ "invoke_transaction": { "sender_address": acc.address, "calldata": too_much, "nonce": "0x1" } })).unwrap();
        let receipt = chain.handle_rpc("starknet_getTransactionReceipt", &serde_json::json!({ "transaction_hash": res["transaction_hash"] })).unwrap();
        assert_eq!(receipt["execution_status"], "REVERTED");
        assert!(receipt["revert_reason"].as_str().unwrap().contains("INSUFFICIENT_BALANCE"), "{receipt}");

        // Declare the token class again under a new name is refused; declaring a fresh class works
        // and the UDC deploys it with a constructor.
        let dup = chain.handle_rpc("starknet_addDeclareTransaction", &serde_json::json!({ "declare_transaction": { "sender_address": acc.address, "nonce": "0x2", "contract_class": chain.classes[&parse_felt_str(&chain.token_class_hash).unwrap()].class_json.clone() } }));
        assert_eq!(dup.unwrap_err().code, ERR_CLASS_ALREADY_DECLARED);

        let root = std::env::temp_dir().join(format!("uc-devnet-counter-{}-{}", std::process::id(), epoch_ms_u64().unwrap_or(1)));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("Scarb.toml"), "[package]\nname = \"counter\"\nversion = \"0.1.0\"\nedition = \"2024_07\"\n\n[dependencies]\nstarknet = \"2.18.0\"\n\n[[target.starknet-contract]]\nsierra = true\ncasm = false\n").unwrap();
        fs::write(root.join("src/lib.cairo"), r#"
#[starknet::interface]
pub trait ICounter<T> { fn increase(ref self: T, by: u64); fn get(self: @T) -> u64; }
#[starknet::contract]
pub mod Counter {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    #[storage]
    struct Storage { value: u64 }
    #[constructor]
    fn constructor(ref self: ContractState, start: u64) { self.value.write(start); }
    #[abi(embed_v0)]
    impl CounterImpl of super::ICounter<ContractState> {
        fn increase(ref self: ContractState, by: u64) { self.value.write(self.value.read() + by); }
        fn get(self: @ContractState) -> u64 { self.value.read() }
    }
}
"#).unwrap();
        run_build(BuildArgs {
            common: BuildCommonArgs { manifest_path: Some(root.join("Scarb.toml")), package: None, workspace: false, features: Vec::new(), offline: true, release: false, profile: Some("dev".to_string()) },
            engine: EngineArg::Uc, daemon_mode: DaemonModeArg::Off, report_path: Some(root.join("build.json")), record_failure: None, json: false, plan_only: false,
        }).expect("build counter");
        let class_json: serde_json::Value = serde_json::from_str(&fs::read_to_string(root.join("target/dev/counter_Counter.contract_class.json")).unwrap()).unwrap();
        let _ = fs::remove_dir_all(&root);
        let declared = chain.handle_rpc("starknet_addDeclareTransaction", &serde_json::json!({ "declare_transaction": { "sender_address": acc.address, "nonce": "0x2", "contract_class": class_json, "compiled_class_hash": "0x0" } })).unwrap();
        let class_hash = declared["class_hash"].as_str().unwrap().to_string();
        let fetched = chain.handle_rpc("starknet_getClass", &serde_json::json!({ "block_id": "latest", "class_hash": class_hash })).unwrap();
        assert!(fetched.get("sierra_program").is_some());

        // Deploy through the UDC with constructor(start = 40), then increase by 2 and read 42.
        let udc_calldata = vec!["0x1".to_string(), DEVNET_UDC_ADDRESS.to_string(), hex(&selector("deployContract")), "0x5".to_string(), class_hash.clone(), "0x7".to_string(), "0x0".to_string(), "0x1".to_string(), "0x28".to_string()];
        let res = chain.handle_rpc("starknet_addInvokeTransaction", &serde_json::json!({ "invoke_transaction": { "sender_address": acc.address, "calldata": udc_calldata, "nonce": "0x3" } })).unwrap();
        let receipt = chain.handle_rpc("starknet_getTransactionReceipt", &serde_json::json!({ "transaction_hash": res["transaction_hash"] })).unwrap();
        assert_eq!(receipt["execution_status"], "SUCCEEDED", "{receipt}");
        let deployed = receipt["events"].as_array().unwrap().iter()
            .find(|e| e["from_address"].as_str().map(|s| parse_felt_str(s).unwrap()) == Some(parse_felt_str(DEVNET_UDC_ADDRESS).unwrap()))
            .and_then(|e| e["data"][0].as_str()).expect("ContractDeployed event").to_string();
        let expected = calculate_contract_address(&Felt::from(7_u64), &parse_felt_str(&class_hash).unwrap(), &[Felt::from(40_u64)], &Felt::ZERO);
        assert_eq!(parse_felt_str(&deployed).unwrap(), expected, "UDC address must follow the Starknet formula for unique = 0");
        assert_eq!(chain.handle_rpc("starknet_getClassHashAt", &serde_json::json!({ "block_id": "latest", "contract_address": deployed })).unwrap().as_str().map(|s| parse_felt_str(s).unwrap()), parse_felt_str(&class_hash).ok());

        let inc = vec!["0x1".to_string(), deployed.clone(), hex(&selector("increase")), "0x1".to_string(), "0x2".to_string()];
        chain.handle_rpc("starknet_addInvokeTransaction", &serde_json::json!({ "invoke_transaction": { "sender_address": acc.address, "calldata": inc, "nonce": "0x4" } })).unwrap();
        let got = chain.handle_rpc("starknet_call", &serde_json::json!({ "request": { "contract_address": deployed, "entry_point_selector": hex(&selector("get")), "calldata": [] }, "block_id": "latest" })).unwrap();
        assert_eq!(got, serde_json::json!(["0x2a"]));
        let slot = hex(&selector("value"));
        assert_eq!(chain.handle_rpc("starknet_getStorageAt", &serde_json::json!({ "contract_address": deployed, "key": slot, "block_id": "latest" })).unwrap(), serde_json::json!("0x2a"));

        // Blocks: genesis + 4 transactions (the reverted one included).
        assert_eq!(chain.handle_rpc("starknet_blockNumber", &serde_json::json!([])).unwrap(), serde_json::json!(5));
        let block = chain.handle_rpc("starknet_getBlockWithTxs", &serde_json::json!({ "block_id": { "block_number": 1 } })).unwrap();
        assert_eq!(block["transactions"].as_array().unwrap().len(), 1);
        let events = chain.handle_rpc("starknet_getEvents", &serde_json::json!({ "filter": { "address": strk, "chunk_size": 10 } })).unwrap();
        assert!(events["events"].as_array().unwrap().len() >= 1);

        // Signatures: a transaction signed with the account's key over its SNIP-8 hash is accepted;
        // one signed with another key is refused; the node's hash is the SNIP-8 hash.
        let pk = parse_felt_str(&acc.private_key).unwrap();
        let mut signed = serde_json::json!({ "type": "INVOKE", "version": "0x3", "sender_address": acc.address, "calldata": inc_again(&deployed), "nonce": "0x5",
            "resource_bounds": { "l1_gas": { "max_amount": "0x1", "max_price_per_unit": "0x1" }, "l2_gas": { "max_amount": "0x1", "max_price_per_unit": "0x1" }, "l1_data_gas": { "max_amount": "0x1", "max_price_per_unit": "0x1" } },
            "tip": "0x0", "paymaster_data": [], "account_deployment_data": [], "nonce_data_availability_mode": "L1", "fee_data_availability_mode": "L1" });
        let expected_hash = sign_v3_transaction(&mut signed, chain.chain_id, Some(&pk)).unwrap();
        let res = chain.handle_rpc("starknet_addInvokeTransaction", &serde_json::json!({ "invoke_transaction": signed.clone() })).unwrap();
        assert_eq!(parse_felt_str(res["transaction_hash"].as_str().unwrap()).unwrap(), expected_hash, "the devnet reports the SNIP-8 hash");
        let mut forged = signed.clone();
        forged["nonce"] = serde_json::json!("0x6");
        sign_v3_transaction(&mut forged, chain.chain_id, Some(&Felt::from(99_u64))).unwrap();
        let err = chain.handle_rpc("starknet_addInvokeTransaction", &serde_json::json!({ "invoke_transaction": forged })).unwrap_err();
        assert_eq!(err.message, "invalid signature");
        assert_eq!(chain.handle_rpc("starknet_getNonce", &serde_json::json!({ "block_id": "latest", "contract_address": acc.address })).unwrap(), serde_json::json!("0x6"));

        // Mint and deploy_account.
        let minted = chain.handle_rpc("devnet_mint", &serde_json::json!({ "address": other.address, "amount": 7, "unit": "FRI" })).unwrap();
        assert_eq!(minted["new_balance"], "1000000000000000000012");
        let salt = "0x123";
        let addr = calculate_contract_address(&parse_felt_str(salt).unwrap(), &parse_felt_str(&chain.account_class_hash).unwrap(), &[Felt::from(9_u64)], &Felt::ZERO);
        let dep = chain.handle_rpc("starknet_addDeployAccountTransaction", &serde_json::json!({ "deploy_account_transaction": { "class_hash": chain.account_class_hash, "contract_address_salt": salt, "constructor_calldata": ["0x9"], "nonce": "0x0" } })).unwrap();
        assert_eq!(parse_felt_str(dep["contract_address"].as_str().unwrap()).unwrap(), addr);
        let pk = chain.handle_rpc("starknet_call", &serde_json::json!({ "request": { "contract_address": hex(&addr), "entry_point_selector": hex(&selector("get_public_key")), "calldata": [] } })).unwrap();
        assert_eq!(pk, serde_json::json!(["0x9"]));
    }
}
