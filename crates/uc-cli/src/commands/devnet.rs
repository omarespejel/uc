//! In-process Starknet devnet: declares classes, deploys contracts and executes invoke
//! transactions on the same Sierra runner `uc test` uses, with an account, a fee token and a
//! universal deployer compiled from embedded Cairo sources by uc's own compiler.

use super::*;
use cairo_lang_runner::casm_run::contract_address::calculate_contract_address;
use cairo_lang_runner::{Arg, ClassInfoMap as OrderedHashMap, DevnetExecutionContext, RunResultValue, SierraCasmRunner, StarknetState};
use cairo_lang_sierra::program::Program;
use cairo_lang_starknet::contract::ContractInfo;
use starknet_types_core::felt::Felt;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, RwLock};

pub(crate) const DEVNET_STRK_ADDRESS: &str = "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d";
pub(crate) const DEVNET_ETH_ADDRESS: &str = "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7";
/// The JSON-RPC version the devnet serves (`pre_confirmed` tags, event and transaction indexes).
pub(crate) const DEVNET_SPEC_VERSION: &str = "0.10.4";
/// The Starknet version whose rules the devnet follows: fees in L2 gas and blob data gas,
/// Blake2s compiled class hashes.
const DEVNET_STARKNET_VERSION: &str = "0.14.1";
const EXECUTION_GAS_BUDGET: usize = 2_000_000_000;
/// Price of every gas unit (L1, L1 data, L2) in FRI and WEI, as starknet-devnet starts.
const DEVNET_GAS_PRICE: u128 = 1_000_000_000;
/// L1 data gas per felt of state diff (blob data availability).
const DATA_GAS_PER_FELT: u64 = 32;
/// L2 gas per byte of declared code, `(Sierra felts + CASM felts) * 32 + ABI bytes`: what
/// starknet-devnet 0.10.1 (blockifier) charges a DECLARE, within 0.3% on measured classes.
const GAS_PER_CODE_BYTE: u64 = 1_280;

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
    use core::ecdsa::check_ecdsa_signature;
    use core::num::traits::Zero;
    use starknet::account::Call;
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use starknet::syscalls::call_contract_syscall;
    use starknet::{get_caller_address, get_contract_address, get_tx_info};

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
            self.validate_transaction()
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
            self.validate_transaction()
        }

        fn __validate_deploy__(self: @ContractState, class_hash: felt252, salt: felt252, public_key: felt252) -> felt252 {
            self.validate_transaction()
        }

        fn is_valid_signature(self: @ContractState, hash: felt252, signature: Array<felt252>) -> felt252 {
            if self.is_valid(hash, signature.span()) {
                starknet::VALIDATED
            } else {
                0
            }
        }

        fn get_public_key(self: @ContractState) -> felt252 {
            self.public_key.read()
        }

        fn set_public_key(ref self: ContractState, new_public_key: felt252) {
            assert(get_caller_address() == get_contract_address(), 'ONLY_SELF');
            self.public_key.write(new_public_key);
        }
    }

    #[generate_trait]
    impl Internal of InternalTrait {
        fn validate_transaction(self: @ContractState) -> felt252 {
            let tx_info = get_tx_info().unbox();
            assert(self.is_valid(tx_info.transaction_hash, tx_info.signature), 'Account: invalid signature');
            starknet::VALIDATED
        }

        fn is_valid(self: @ContractState, hash: felt252, signature: Span<felt252>) -> bool {
            signature.len() == 2
                && check_ecdsa_signature(hash, self.public_key.read(), *signature.at(0), *signature.at(1))
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

/// Compiles the embedded system contracts once per compiler version and source revision into
/// uc's cache (`<cache>/devnet-system-v<cairo version>-<source digest>/`) and loads them.
pub(crate) fn ensure_devnet_system_classes() -> Result<DevnetSystemClasses> {
    let version = native_cairo_lang_compiler_version();
    let dir = native_global_cache_root_dir()
        .unwrap_or_else(|| std::env::temp_dir().join("uc-global-cache"))
        .join(format!("devnet-system-v{version}-{}", &blake3::hash(SYSTEM_CONTRACTS_CAIRO.as_bytes()).to_hex()[..12]));
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

/// The entry points an execution may reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryKind {
    External,
    L1Handler,
}

const ERR_CONTRACT_NOT_FOUND: i64 = 20;
const ERR_ENTRYPOINT_NOT_FOUND: i64 = 21;
const ERR_BLOCK_NOT_FOUND: i64 = 24;
const ERR_INVALID_TX_INDEX: i64 = 27;
const ERR_CLASS_HASH_NOT_FOUND: i64 = 28;
const ERR_TX_HASH_NOT_FOUND: i64 = 29;
const ERR_CONTRACT_ERROR: i64 = 40;
const ERR_TX_EXECUTION_ERROR: i64 = 41;
const ERR_INVALID_NONCE: i64 = 52;
const ERR_INSUFFICIENT_RESOURCES: i64 = 53;
const ERR_INSUFFICIENT_BALANCE: i64 = 54;
const ERR_VALIDATION_FAILURE: i64 = 55;
const ERR_COMPILED_CLASS_HASH_MISMATCH: i64 = 60;
const ERR_UNSUPPORTED_TX_VERSION: i64 = 61;
const ERR_INVALID_CONTRACT_CLASS: i64 = 50;
const ERR_CLASS_ALREADY_DECLARED: i64 = 51;
const ERR_COMPILATION_FAILED: i64 = 100;
const ERR_UNSUPPORTED: i64 = -32601;
const ERR_INVALID_PARAMS: i64 = -32602;

/// Outcome of running one entry point.
struct ExecutionOutcome {
    contract: Felt,
    class_hash: Felt,
    selector: Felt,
    calldata: Vec<Felt>,
    caller: Felt,
    /// Sierra gas the run consumed, syscalls included: the entry point's L2 gas.
    l2_gas: u64,
    retdata: Vec<Felt>,
    events: Vec<(Felt, Vec<Felt>, Vec<Felt>)>,
    messages: Vec<(Felt, Felt, Vec<Felt>)>,
    calls: Vec<cairo_lang_runner::casm_run::RecordedCall>,
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
    impersonated_accounts: std::collections::HashSet<Felt>,
    auto_impersonate: bool,
    gas_price_wei: u128,
    gas_price_fri: u128,
    data_gas_price_wei: u128,
    data_gas_price_fri: u128,
    l2_gas_price_wei: u128,
    l2_gas_price_fri: u128,
    l1_messaging_contract: String,
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
            impersonated_accounts: std::collections::HashSet::new(),
            auto_impersonate: false,
            gas_price_wei: DEVNET_GAS_PRICE,
            gas_price_fri: DEVNET_GAS_PRICE,
            data_gas_price_wei: DEVNET_GAS_PRICE,
            data_gas_price_fri: DEVNET_GAS_PRICE,
            l2_gas_price_wei: DEVNET_GAS_PRICE,
            l2_gas_price_fri: DEVNET_GAS_PRICE,
            l1_messaging_contract: "0x000000000000000000000000000000000000beee".to_string(),
            accounts,
            account_class_hash: String::new(),
            token_class_hash: String::new(),
            deployer_class_hash: String::new(),
        };
        let account_class = chain.declare(system.account.clone()).map_err(|e| anyhow!("{}", e.message))?;
        let token_class = chain.declare(system.token.clone()).map_err(|e| anyhow!("{}", e.message))?;
        let deployer_class = chain.declare(system.deployer.clone()).map_err(|e| anyhow!("{}", e.message))?;
        chain.account_class_hash = hex(&account_class);
        chain.token_class_hash = hex(&token_class);
        chain.deployer_class_hash = hex(&deployer_class);

        let strk = parse_felt_str(DEVNET_STRK_ADDRESS)?;
        let eth = parse_felt_str(DEVNET_ETH_ADDRESS)?;
        let udc = parse_felt_str(STARKNET_UDC_ADDRESS)?;
        let short = |s: &str| Felt::from_bytes_be_slice(s.as_bytes());
        chain.deploy_at(strk, token_class, &[short("StarkNet Token"), short("STRK")], Felt::ZERO)?;
        chain.deploy_at(eth, token_class, &[short("Ether"), short("ETH")], Felt::ZERO)?;
        chain.deploy_at(udc, deployer_class, &[], Felt::ZERO)?;
        chain.deploy_at(parse_felt_str(LEGACY_UDC_ADDRESS)?, deployer_class, &[], Felt::ZERO)?;
        let accounts = chain.accounts.clone();
        for acc in &accounts {
            let address = parse_felt_str(&acc.address)?;
            let public_key = parse_felt_str(&acc.public_key)?;
            chain.deploy_at(address, account_class, &[public_key], Felt::ZERO)?;
            let (low, high) = u256_parts(&acc.initial_balance).map_err(|e| anyhow!("{}", e.message))?;
            for token in [strk, eth] {
                let out = chain.execute(token, selector("mint"), vec![address, low, high], Felt::ZERO, &DevnetExecutionContext::default(), EXECUTION_GAS_BUDGET, EntryKind::External)?;
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
            let out = self.run_function(class_hash, ctor.clone(), selector("constructor"), address, calldata.to_vec(), caller, &DevnetExecutionContext::default(), EXECUTION_GAS_BUDGET)?;
            if let Some(p) = out.panic {
                self.state.register_deployed_contract(address, Felt::ZERO);
                bail!("constructor of {} panicked: {}", hex(&address), cairo_lang_runner::casm_run::format_for_panic(p.into_iter()));
            }
        }
        Ok(())
    }

    /// Declares a class from its `.contract_class.json` document. Returns the class hash.
    fn declare(&mut self, class_json: serde_json::Value) -> Result<Felt, RpcError> {
        let class: ContractClass = serde_json::from_value(normalize_class_json(class_json.clone()))
            .map_err(|e| RpcError::new(ERR_INVALID_PARAMS, format!("not a Sierra contract class: {e}")))?;
        // The class hash commits to the ABI string as declared (a string in DECLARE payloads).
        let class_hash_hex = compute_class_hash_from_json(&class_json)
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
        let mut stored = class_json;
        if stored.get("abi").is_some_and(|a| !a.is_string()) {
            stored["abi"] = serde_json::json!(declared_abi_string(&stored));
        }
        self.classes.insert(class_hash, DeclaredClass { class_json: stored, program });
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

    /// Runs one entry point with `gas_budget` Sierra gas. A panic leaves the chain state as it
    /// was; a success commits the run's state.
    #[allow(clippy::too_many_arguments)]
    fn run_function(
        &mut self,
        class_hash: Felt,
        function: cairo_lang_sierra::ids::FunctionId,
        selector: Felt,
        contract: Felt,
        calldata: Vec<Felt>,
        caller: Felt,
        tx: &DevnetExecutionContext,
        gas_budget: usize,
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
                vec![Arg::Array(calldata.iter().copied().map(Arg::Value).collect())],
                Some(gas_budget),
                state,
            )
            .map_err(|e| anyhow!("runner error: {e}"))?;
        let remaining = res.gas_counter.and_then(|g| u64::try_from(g.to_biguint()).ok()).unwrap_or(0);
        let l2_gas = (gas_budget as u64).saturating_sub(remaining);
        match res.value {
            RunResultValue::Success(value) => {
                let mut new_state = res.starknet_state;
                let events = new_state.take_events();
                let messages = new_state.take_l2_to_l1_messages();
                let calls = new_state.take_recorded_calls();
                self.state = new_state;
                let retdata = match res.return_array {
                    Some(data) => data,
                    None => cairo_lang_runner::casm_run::read_array_result_as_vec(&res.memory, &value),
                };
                Ok(ExecutionOutcome { contract, class_hash, selector, calldata, caller, l2_gas, retdata, events, messages, calls, panic: None })
            }
            RunResultValue::Panic(data) => Ok(ExecutionOutcome {
                contract,
                class_hash,
                selector,
                calldata,
                caller,
                l2_gas,
                retdata: Vec::new(),
                events: Vec::new(),
                messages: Vec::new(),
                calls: Vec::new(),
                panic: Some(data),
            }),
        }
    }

    /// Runs the entry point `selector` of the contract at `contract`. Calls reach external
    /// entry points only; L1 handlers run only for L1 messages, as on Starknet.
    #[allow(clippy::too_many_arguments)]
    fn execute(&mut self, contract: Felt, selector_felt: Felt, calldata: Vec<Felt>, caller: Felt, tx: &DevnetExecutionContext, gas_budget: usize, kind: EntryKind) -> Result<ExecutionOutcome> {
        let class_hash = self.state.class_hash_at(&contract).ok_or_else(|| anyhow!("CONTRACT_NOT_FOUND"))?;
        let info = self.infos.get(&class_hash).ok_or_else(|| anyhow!("class {} not declared", hex(&class_hash)))?;
        let entry_points = match kind {
            EntryKind::External => &info.externals,
            EntryKind::L1Handler => &info.l1_handlers,
        };
        let function = entry_points.get(&selector_felt).cloned().ok_or_else(|| anyhow!("ENTRYPOINT_NOT_FOUND"))?;
        self.run_function(class_hash, function, selector_felt, contract, calldata, caller, tx, gas_budget)
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
        self.state.set_block_hash(number, hash);
        self.blocks.push(block);
        self.blocks.last().unwrap()
    }

    /// Records a transaction, its receipt and its trace in the pending block.
    fn record(&mut self, tx_type: &str, tx_hash: Felt, transaction: serde_json::Value, mut receipt: serde_json::Value, trace: serde_json::Value) -> String {
        let hash_hex = hex(&tx_hash);
        receipt["type"] = serde_json::json!(tx_type);
        receipt["transaction_hash"] = serde_json::json!(hash_hex);
        receipt["finality_status"] = serde_json::json!("ACCEPTED_ON_L2");
        receipt["block_number"] = serde_json::json!(self.next_block_number());
        receipt["block_hash"] = serde_json::json!("0x0");
        let mut tx_json = transaction;
        if let serde_json::Value::Object(map) = &mut tx_json {
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

    fn strk(&self) -> Felt {
        parse_felt_str(DEVNET_STRK_ADDRESS).unwrap_or_default()
    }

    /// The fee token balance of `owner` in FRI, saturating at `u128::MAX`.
    fn strk_balance(&self, owner: &Felt) -> u128 {
        let key = balance_key(owner);
        if self.state.storage_at(&self.strk(), &(key + Felt::ONE)) != Felt::ZERO {
            return u128::MAX;
        }
        u128::try_from(self.state.storage_at(&self.strk(), &key).to_biguint()).unwrap_or(u128::MAX)
    }

    /// Rejects what a sequencer rejects before validation: a gas price under the block's, no L2
    /// gas at all, or a payer whose balance does not cover the maximum fee.
    fn check_fee_bounds(&self, payer: &Felt, bounds: &ResourceBounds) -> Result<(), RpcError> {
        let insufficient = |detail: String| {
            RpcError::new(ERR_INSUFFICIENT_RESOURCES, "The transaction's resources don't cover validation or the minimal transaction fee")
                .with_data(serde_json::json!(detail))
        };
        for (name, (amount, price), actual_price) in [
            ("L2 gas", bounds.l2_gas, self.l2_gas_price_fri),
            ("L1 data gas", bounds.l1_data_gas, self.data_gas_price_fri),
            ("L1 gas", bounds.l1_gas, self.gas_price_fri),
        ] {
            if amount > 0 && price < actual_price {
                return Err(insufficient(format!("Max {name} price ({price}) is lower than the actual gas price: {actual_price}.")));
            }
        }
        if bounds.l2_gas.0 == 0 {
            return Err(insufficient("Max L2 gas amount is 0.".to_string()));
        }
        let max_fee = bounds.max_fee();
        let balance = self.strk_balance(payer);
        if balance < max_fee {
            return Err(RpcError::new(ERR_INSUFFICIENT_BALANCE, "Account balance is not enough to cover the transaction cost").with_data(serde_json::json!(format!(
                "The resource bounds allow a fee of up to {max_fee} FRI; {} holds {balance} FRI.",
                hex(payer)
            ))));
        }
        Ok(())
    }

    /// L1 data gas of the state diff since `before`, counted as blockifier counts it for the fee:
    /// two felts per modified contract (not the fee token, whose updates the block amortizes),
    /// one per class change, two per storage update (the payer's fee balance included) and two
    /// per declared class.
    fn data_gas_for_fee(&self, before: &StarknetState, payer: Felt, declared: bool) -> u64 {
        let strk = self.strk();
        let storage = self.state.storage_diff(before);
        let classes = class_changes(before, &self.state);
        let mut modified: BTreeSet<Felt> = storage.iter().map(|(c, _, _)| *c).chain(classes.iter().map(|(a, _)| *a)).collect();
        modified.insert(payer);
        let payer_key = balance_key(&payer);
        let mut n_storage = storage.len();
        if !storage.iter().any(|(c, k, _)| *c == strk && *k == payer_key) {
            n_storage += 1;
        }
        modified.remove(&strk);
        let felts = 2 * modified.len() + classes.len() + 2 * n_storage + if declared { 2 } else { 0 };
        DATA_GAS_PER_FELT * felts as u64
    }

    /// The RPC `state_diff` of everything since `before`.
    fn state_diff_json(&self, before: &StarknetState, nonce: Option<(Felt, u64)>, declared: Option<(Felt, Felt)>) -> serde_json::Value {
        let mut by_contract: BTreeMap<Felt, Vec<serde_json::Value>> = BTreeMap::new();
        for (contract, key, value) in self.state.storage_diff(before) {
            by_contract.entry(contract).or_default().push(serde_json::json!({ "key": hex(&key), "value": hex(&value) }));
        }
        let (replaced, deployed): (Vec<(Felt, Felt)>, Vec<(Felt, Felt)>) = class_changes(before, &self.state)
            .into_iter()
            .partition(|(address, _)| before.class_hash_at(address).is_some_and(|c| c != Felt::ZERO));
        serde_json::json!({
            "storage_diffs": by_contract.into_iter().map(|(address, entries)| serde_json::json!({ "address": hex(&address), "storage_entries": entries })).collect::<Vec<_>>(),
            "deprecated_declared_classes": [],
            "declared_classes": declared.iter().map(|(h, c)| serde_json::json!({ "class_hash": hex(h), "compiled_class_hash": hex(c) })).collect::<Vec<_>>(),
            "deployed_contracts": deployed.iter().map(|(a, c)| serde_json::json!({ "address": hex(a), "class_hash": hex(c) })).collect::<Vec<_>>(),
            "replaced_classes": replaced.iter().map(|(a, c)| serde_json::json!({ "contract_address": hex(a), "class_hash": hex(c) })).collect::<Vec<_>>(),
            "nonces": nonce.iter().map(|(a, n)| serde_json::json!({ "contract_address": hex(a), "nonce": format!("{n:#x}") })).collect::<Vec<_>>(),
        })
    }

    /// Runs an account's validation entry point; anything but `VALID` rejects the transaction.
    fn validate_tx(&mut self, account: Felt, entry: &str, calldata: Vec<Felt>, ctx: &DevnetExecutionContext) -> Result<ExecutionOutcome, RpcError> {
        let failure = |detail: String| RpcError::new(ERR_VALIDATION_FAILURE, "Account validation failed").with_data(serde_json::json!(detail));
        let out = self
            .execute(account, selector(entry), calldata, Felt::ZERO, ctx, EXECUTION_GAS_BUDGET, EntryKind::External)
            .map_err(|e| failure(format!("The `{entry}` entry point of {} could not run: {e}", hex(&account))))?;
        if let Some(p) = &out.panic {
            return Err(failure(format!(
                "The `{entry}` entry point of {} panicked with: {}",
                hex(&account),
                cairo_lang_runner::casm_run::format_for_panic(p.clone().into_iter())
            )));
        }
        if out.retdata.first() != Some(&Felt::from_bytes_be_slice(b"VALID")) {
            return Err(failure(format!("The `{entry}` entry point of {} returned {:?}, not 'VALID'.", hex(&account), hex_vec(&out.retdata))));
        }
        Ok(out)
    }

    /// Rejects, or validates, executes and charges, one account transaction (INVOKE, DECLARE or
    /// DEPLOY_ACCOUNT) as a Starknet sequencer does. A rejected transaction changes nothing; a
    /// reverted one keeps its validation, its nonce bump and its fee.
    fn run_account_tx(&mut self, kind: &'static str, tx: &serde_json::Value, mode: TxMode) -> Result<TxEffects, RpcError> {
        let before = self.state.clone();
        let nonces = self.nonces.clone();
        let mut declared = None;
        let result = self.run_account_tx_inner(kind, tx, mode, &before, &mut declared);
        if result.is_err() {
            if let Some((class_hash, _)) = declared {
                self.undeclare(&class_hash);
            }
            self.state = before;
            self.nonces = nonces;
        }
        result
    }

    fn run_account_tx_inner(
        &mut self,
        kind: &'static str,
        tx: &serde_json::Value,
        mode: TxMode,
        before: &StarknetState,
        declared: &mut Option<(Felt, Felt)>,
    ) -> Result<TxEffects, RpcError> {
        let bounds = ResourceBounds::of(tx);
        let charge = !mode.skip_fee_charge;
        let mut deploy: Option<(Felt, Felt)> = None;
        let mut code_gas = 0_u64;
        let (sender, calldata) = if kind == "DEPLOY_ACCOUNT" {
            let class_hash = felt(tx.get("class_hash").and_then(|v| v.as_str()).unwrap_or("0x0"))?;
            let salt = felt(tx.get("contract_address_salt").and_then(|v| v.as_str()).unwrap_or("0x0"))?;
            let calldata = felts_of_json(tx.get("constructor_calldata"));
            if !self.classes.contains_key(&class_hash) {
                return Err(RpcError::new(ERR_CLASS_HASH_NOT_FOUND, "Class hash not found"));
            }
            let address = calculate_contract_address(&salt, &class_hash, &calldata, &Felt::ZERO);
            if self.state.class_hash_at(&address).is_some_and(|c| c != Felt::ZERO) {
                return Err(RpcError::new(ERR_TX_EXECUTION_ERROR, format!("contract {} is already deployed", hex(&address))));
            }
            deploy = Some((class_hash, salt));
            (address, calldata)
        } else {
            let sender = felt(tx.get("sender_address").and_then(|v| v.as_str()).ok_or_else(|| RpcError::new(ERR_INVALID_PARAMS, "sender_address is required"))?)?;
            if self.state.class_hash_at(&sender).filter(|c| *c != Felt::ZERO).is_none() {
                if self.auto_impersonate || self.impersonated_accounts.contains(&sender) {
                    if let Ok(acc_ch) = parse_felt_str(&self.account_class_hash) {
                        self.state.register_deployed_contract(sender, acc_ch);
                    }
                } else {
                    return Err(RpcError::new(ERR_CONTRACT_NOT_FOUND, "Contract not found"));
                }
            }
            (sender, felts_of_json(tx.get("calldata")))
        };
        let nonce = self.check_nonce(&sender, tx.get("nonce"))?;
        let mut for_hash = tx.clone();
        if kind == "DECLARE" {
            let class_json = tx.get("contract_class").cloned().ok_or_else(|| RpcError::new(ERR_INVALID_PARAMS, "contract_class is required"))?;
            let (compiled, casm_len) = compile_declared_class(&class_json)?;
            code_gas = declare_code_gas(&class_json, casm_len);
            let given = tx.get("compiled_class_hash").and_then(|v| v.as_str()).and_then(|s| parse_felt_str(s).ok());
            if given != Some(compiled) {
                return Err(RpcError::new(ERR_COMPILED_CLASS_HASH_MISMATCH, "Compiled class hash mismatch").with_data(serde_json::json!(format!(
                    "The class compiles to compiled class hash {} (Blake2s, required since Starknet 0.14.1); the transaction declares {}.",
                    hex(&compiled),
                    given.map(|g| hex(&g)).unwrap_or_else(|| "none".to_string())
                ))));
            }
            let class_hash = self.declare(class_json)?;
            *declared = Some((class_hash, compiled));
            for_hash["class_hash"] = serde_json::json!(hex(&class_hash));
            if let Some(map) = for_hash.as_object_mut() {
                map.remove("contract_class");
            }
        }
        let tx_hash = self.v3_hash_or_digest(kind, &sender, nonce, &for_hash, &serde_json::json!({ "sender": hex(&sender), "calldata": hex_vec(&calldata) }));
        if charge {
            self.check_fee_bounds(&sender, &bounds)?;
        }
        let ctx = DevnetExecutionContext {
            account_contract_address: sender,
            transaction_hash: tx_hash,
            nonce: Felt::from(nonce),
            version: Felt::from(3_u64),
            signature: felts_of_json(tx.get("signature")),
            ..Default::default()
        };
        // DEPLOY_ACCOUNT runs the constructor before validation; a failure of either rejects it.
        let mut execute = None;
        if let Some((class_hash, _)) = deploy {
            self.state.register_deployed_contract(sender, class_hash);
            let ctor = self.infos.get(&class_hash).and_then(|i| i.constructor.clone());
            if let Some(ctor) = ctor {
                let out = self
                    .run_function(class_hash, ctor, selector("constructor"), sender, calldata.clone(), Felt::ZERO, &ctx, EXECUTION_GAS_BUDGET)
                    .map_err(|e| RpcError::new(ERR_VALIDATION_FAILURE, "Account validation failed").with_data(serde_json::json!(e.to_string())))?;
                if let Some(p) = &out.panic {
                    return Err(RpcError::new(ERR_VALIDATION_FAILURE, "Account validation failed").with_data(serde_json::json!(format!(
                        "The constructor of {} panicked with: {}",
                        hex(&sender),
                        cairo_lang_runner::casm_run::format_for_panic(p.clone().into_iter())
                    ))));
                }
                execute = Some(out);
            }
        }
        let is_impersonated = self.auto_impersonate || self.impersonated_accounts.contains(&sender);
        let validate = if mode.skip_validate || is_impersonated {
            None
        } else {
            let (entry, args) = match (deploy, *declared) {
                (Some((class_hash, salt)), _) => ("__validate_deploy__", [vec![class_hash, salt], calldata.clone()].concat()),
                (None, Some((class_hash, _))) => ("__validate_declare__", vec![class_hash]),
                (None, None) => ("__validate__", calldata.clone()),
            };
            Some(self.validate_tx(sender, entry, args, &ctx)?)
        };
        let validate_gas = validate.as_ref().map_or(0, |v| v.l2_gas);
        if charge && u128::from(validate_gas) > bounds.l2_gas.0 {
            return Err(RpcError::new(ERR_INSUFFICIENT_RESOURCES, "The transaction's resources don't cover validation or the minimal transaction fee").with_data(
                serde_json::json!(format!("Validation used {validate_gas} L2 gas; the transaction allows {}.", bounds.l2_gas.0)),
            ));
        }
        // INVOKE runs `__execute__` on the L2 gas left: a panic, or running out, reverts it.
        let after_validate = self.state.clone();
        let mut revert = None;
        if kind == "INVOKE" {
            let budget = if charge {
                usize::try_from(bounds.l2_gas.0.saturating_sub(u128::from(validate_gas))).unwrap_or(usize::MAX).min(EXECUTION_GAS_BUDGET)
            } else {
                EXECUTION_GAS_BUDGET
            };
            let out = self
                .execute(sender, selector("__execute__"), calldata.clone(), Felt::ZERO, &ctx, budget, EntryKind::External)
                .map_err(|e| RpcError::new(ERR_TX_EXECUTION_ERROR, e.to_string()))?;
            if let Some(p) = &out.panic {
                revert = Some(cairo_lang_runner::casm_run::format_for_panic(p.clone().into_iter()));
            }
            execute = Some(out);
        }
        let l2_gas = code_gas + validate_gas + execute.as_ref().map_or(0, |e| e.l2_gas);
        let mut l1_data_gas = self.data_gas_for_fee(before, sender, declared.is_some());
        if charge && u128::from(l1_data_gas) > bounds.l1_data_gas.0 {
            if kind != "INVOKE" {
                return Err(RpcError::new(ERR_INSUFFICIENT_RESOURCES, "The transaction's resources don't cover validation or the minimal transaction fee").with_data(
                    serde_json::json!(format!("The state diff needs {l1_data_gas} L1 data gas; the transaction allows {}.", bounds.l1_data_gas.0)),
                ));
            }
            if revert.is_none() {
                revert = Some(format!("Insufficient max L1DataGas: max amount: {}, actual used: {l1_data_gas}.", bounds.l1_data_gas.0));
                self.state = after_validate;
                l1_data_gas = self.data_gas_for_fee(before, sender, false);
            }
        }
        let tip = if charge { bounds.tip } else { 0 };
        let mut fee = u128::from(l2_gas)
            .saturating_mul(self.l2_gas_price_fri.saturating_add(tip))
            .saturating_add(u128::from(l1_data_gas).saturating_mul(self.data_gas_price_fri));
        let mut fee_transfer = None;
        if charge {
            fee = fee.min(bounds.max_fee());
            let (strk, sequencer) = (self.strk(), self.sequencer);
            let out = self
                .execute(strk, selector("transfer"), vec![sequencer, Felt::from(fee), Felt::ZERO], sender, &ctx, EXECUTION_GAS_BUDGET, EntryKind::External)
                .map_err(|e| RpcError::new(ERR_TX_EXECUTION_ERROR, e.to_string()))?;
            if let Some(p) = &out.panic {
                return Err(RpcError::new(ERR_INSUFFICIENT_BALANCE, "Account balance is not enough to cover the transaction cost")
                    .with_data(serde_json::json!(cairo_lang_runner::casm_run::format_for_panic(p.clone().into_iter()))));
            }
            fee_transfer = Some(out);
        }
        self.nonces.insert(sender, nonce + 1);
        let state_diff = self.state_diff_json(before, Some((sender, nonce + 1)), *declared);
        let mut transaction = for_hash;
        transaction["type"] = serde_json::json!(kind);
        if transaction.get("version").is_none() {
            transaction["version"] = serde_json::json!("0x3");
        }
        transaction["nonce"] = serde_json::json!(format!("{nonce:#x}"));
        if kind != "DEPLOY_ACCOUNT" {
            transaction["sender_address"] = serde_json::json!(hex(&sender));
        }
        Ok(TxEffects {
            tx_type: kind,
            tx_hash,
            transaction,
            validate,
            execute,
            fee_transfer,
            revert,
            l2_gas,
            l1_data_gas,
            l1_gas_price: self.gas_price_fri,
            l2_gas_price: self.l2_gas_price_fri,
            l1_data_gas_price: self.data_gas_price_fri,
            fee,
            state_diff,
            contract_address: deploy.map(|_| sender),
            class_hash: declared.map(|(h, _)| h),
        })
    }

    /// Runs an account transaction and records it in a block.
    fn add_account_tx(&mut self, kind: &'static str, tx: &serde_json::Value) -> Result<serde_json::Value, RpcError> {
        let query_version = Felt::from_hex(QUERY_VERSION_V3).unwrap_or_default();
        if tx.get("version").and_then(|v| v.as_str()).and_then(|v| parse_felt_str(v).ok()) == Some(query_version) {
            return Err(RpcError::new(ERR_UNSUPPORTED_TX_VERSION, "The transaction version is not supported")
                .with_data(serde_json::json!("Query versions (2^128 + 3) are for estimates and simulations only.")));
        }
        let effects = self.run_account_tx(kind, tx, TxMode::default())?;
        let hash = self.record(kind, effects.tx_hash, effects.transaction.clone(), effects.receipt(), effects.trace());
        let mut out = serde_json::json!({ "transaction_hash": hash });
        if let Some(class_hash) = effects.class_hash {
            out["class_hash"] = serde_json::json!(hex(&class_hash));
        }
        if let Some(address) = effects.contract_address {
            out["contract_address"] = serde_json::json!(hex(&address));
        }
        Ok(out)
    }

    /// Runs `txs` one after another from the current state, then rolls every change back.
    fn simulate(&mut self, txs: &[serde_json::Value], mode: TxMode) -> Result<Vec<TxEffects>, RpcError> {
        let state = self.state.clone();
        let nonces = self.nonces.clone();
        let mut effects = Vec::new();
        let mut failure = None;
        for (index, tx) in txs.iter().enumerate() {
            let kind = match tx.get("type").and_then(|v| v.as_str()) {
                Some("DECLARE") => "DECLARE",
                Some("DEPLOY_ACCOUNT") => "DEPLOY_ACCOUNT",
                _ => "INVOKE",
            };
            match self.run_account_tx(kind, tx, mode) {
                Ok(e) => effects.push(e),
                Err(e) => {
                    failure = Some(RpcError::new(ERR_TX_EXECUTION_ERROR, "Transaction execution error").with_data(serde_json::json!({
                        "transaction_index": index,
                        "execution_error": format!("{} (code {}): {}", e.message, e.code, e.data.map(|d| d.to_string()).unwrap_or_default()),
                    })));
                    break;
                }
            }
        }
        for e in &effects {
            if let Some(class_hash) = e.class_hash {
                self.undeclare(&class_hash);
            }
        }
        self.state = state;
        self.nonces = nonces;
        failure.map_or(Ok(effects), Err)
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
        let result = self.execute(contract, selector_felt, calldata, Felt::ZERO, &DevnetExecutionContext::default(), EXECUTION_GAS_BUDGET, EntryKind::External);
        self.state = snapshot;
        let outcome = result.map_err(|e| {
            if e.to_string() == "ENTRYPOINT_NOT_FOUND" {
                RpcError::new(ERR_ENTRYPOINT_NOT_FOUND, "Requested entrypoint does not exist in the contract")
            } else {
                RpcError::new(ERR_CONTRACT_ERROR, "Contract error").with_data(serde_json::json!({ "revert_error": e.to_string() }))
            }
        })?;
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

    fn block_txs<'a>(&'a self, block: &'a DevnetBlock) -> impl Iterator<Item = &'a DevnetTransaction> + 'a {
        block.transactions.iter().filter_map(|h| self.tx_index.get(h).map(|i| &self.txs[*i]))
    }

    /// The state diff of a block: its transactions' diffs merged, later writes winning.
    fn block_state_diff(&self, block: &DevnetBlock) -> serde_json::Value {
        let mut storage: BTreeMap<String, BTreeMap<String, serde_json::Value>> = BTreeMap::new();
        let mut nonces: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        let (mut deployed, mut declared, mut replaced) = (Vec::new(), Vec::new(), Vec::new());
        for t in self.block_txs(block) {
            let Some(diff) = t.trace.get("state_diff") else { continue };
            for sd in diff["storage_diffs"].as_array().into_iter().flatten() {
                let address = sd["address"].as_str().unwrap_or_default().to_string();
                for e in sd["storage_entries"].as_array().into_iter().flatten() {
                    storage.entry(address.clone()).or_default().insert(e["key"].as_str().unwrap_or_default().to_string(), e["value"].clone());
                }
            }
            for n in diff["nonces"].as_array().into_iter().flatten() {
                nonces.insert(n["contract_address"].as_str().unwrap_or_default().to_string(), n["nonce"].clone());
            }
            deployed.extend(diff["deployed_contracts"].as_array().cloned().unwrap_or_default());
            declared.extend(diff["declared_classes"].as_array().cloned().unwrap_or_default());
            replaced.extend(diff["replaced_classes"].as_array().cloned().unwrap_or_default());
        }
        serde_json::json!({
            "storage_diffs": storage.into_iter().map(|(address, entries)| serde_json::json!({
                "address": address,
                "storage_entries": entries.into_iter().map(|(key, value)| serde_json::json!({ "key": key, "value": value })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "deprecated_declared_classes": [],
            "declared_classes": declared,
            "deployed_contracts": deployed,
            "replaced_classes": replaced,
            "nonces": nonces.into_iter().map(|(address, nonce)| serde_json::json!({ "contract_address": address, "nonce": nonce })).collect::<Vec<_>>(),
        })
    }

    /// Block header and transactions with deterministic commitments.
    fn block_json(&self, block: &DevnetBlock, with_txs: bool) -> serde_json::Value {
        let records: Vec<&DevnetTransaction> = self.block_txs(block).collect();
        let txs: Vec<serde_json::Value> = records
            .iter()
            .map(|t| if with_txs { t.transaction.clone() } else { serde_json::json!(t.transaction_hash) })
            .collect();
        let event_count: usize = records.iter().map(|t| t.receipt.get("events").and_then(|e| e.as_array()).map_or(0, Vec::len)).sum();
        let diff = self.block_state_diff(block);
        let diff_len = state_diff_length(&diff);
        let tx_comm = if records.is_empty() {
            "0x0".to_string()
        } else {
            hex(&felt_from_digest("uc-devnet-tx-comm:", &[&block.block_number.to_le_bytes(), &(records.len() as u64).to_le_bytes()]))
        };
        let ev_comm = if event_count == 0 {
            "0x0".to_string()
        } else {
            hex(&felt_from_digest("uc-devnet-ev-comm:", &[&block.block_number.to_le_bytes(), &(event_count as u64).to_le_bytes()]))
        };
        let rcpt_comm = if records.is_empty() {
            "0x0".to_string()
        } else {
            hex(&felt_from_digest("uc-devnet-rcpt-comm:", &[&block.block_number.to_le_bytes(), &(records.len() as u64).to_le_bytes()]))
        };
        let diff_comm = if diff_len == 0 {
            "0x0".to_string()
        } else {
            hex(&felt_from_digest("uc-devnet-diff-comm:", &[&block.block_number.to_le_bytes(), &(diff_len as u64).to_le_bytes()]))
        };
        let new_root = hex(&felt_from_digest("uc-devnet-root:", &[&block.block_number.to_le_bytes(), block.block_hash.as_bytes()]));
        let l1_price = serde_json::json!({ "price_in_fri": format!("{:#x}", self.gas_price_fri), "price_in_wei": format!("{:#x}", self.gas_price_wei) });
        let data_price = serde_json::json!({ "price_in_fri": format!("{:#x}", self.data_gas_price_fri), "price_in_wei": format!("{:#x}", self.data_gas_price_wei) });
        let l2_price = serde_json::json!({ "price_in_fri": format!("{:#x}", self.l2_gas_price_fri), "price_in_wei": format!("{:#x}", self.l2_gas_price_wei) });
        serde_json::json!({
            "status": "ACCEPTED_ON_L2",
            "block_hash": block.block_hash,
            "parent_hash": block.parent_hash,
            "block_number": block.block_number,
            "new_root": new_root,
            "timestamp": block.timestamp,
            "sequencer_address": block.sequencer_address,
            "l1_gas_price": l1_price,
            "l1_data_gas_price": data_price,
            "l2_gas_price": l2_price,
            "l1_da_mode": "BLOB",
            "starknet_version": DEVNET_STARKNET_VERSION,
            "transaction_count": records.len(),
            "event_count": event_count,
            "state_diff_length": diff_len,
            "state_diff_commitment": diff_comm,
            "transaction_commitment": tx_comm,
            "event_commitment": ev_comm,
            "receipt_commitment": rcpt_comm,
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
            let tx_position = self
                .blocks
                .get(t.block_number as usize)
                .and_then(|b| b.transactions.iter().position(|h| *h == t.transaction_hash))
                .unwrap_or(0);
            for (event_index, ev) in t.receipt.get("events").and_then(|e| e.as_array()).into_iter().flatten().enumerate() {
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
                e["transaction_index"] = serde_json::json!(tx_position);
                e["event_index"] = serde_json::json!(event_index);
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
        let before = self.state.clone();
        let outcome = self
            .execute(token, selector("mint"), vec![address, low, high], Felt::ZERO, &DevnetExecutionContext::default(), EXECUTION_GAS_BUDGET, EntryKind::External)
            .map_err(|e| RpcError::new(ERR_TX_EXECUTION_ERROR, e.to_string()))?;
        if let Some(p) = outcome.panic {
            return Err(RpcError::new(ERR_TX_EXECUTION_ERROR, cairo_lang_runner::casm_run::format_for_panic(p.into_iter())));
        }
        let tx_hash = felt_from_digest("uc-devnet-mint:", &[&address.to_bytes_be(), amount.as_bytes(), &(self.txs.len() as u64).to_le_bytes()]);
        let resources = serde_json::json!({ "l1_gas": 0, "l1_data_gas": 0, "l2_gas": outcome.l2_gas });
        let receipt = serde_json::json!({
            "actual_fee": { "amount": "0x0", "unit": "FRI" },
            "execution_status": "SUCCEEDED",
            "messages_sent": [],
            "events": receipt_events(&[&outcome]),
            "execution_resources": resources,
        });
        let trace = serde_json::json!({ "type": "INVOKE", "execute_invocation": invocation_json(&outcome, "EXTERNAL"), "state_diff": self.state_diff_json(&before, None, None), "execution_resources": resources });
        let transaction = serde_json::json!({ "type": "INVOKE", "version": "0x3", "sender_address": "0x0", "calldata": [], "nonce": "0x0", "signature": [], "devnet_mint": { "address": hex(&address), "amount": amount, "unit": unit } });
        let hash_hex = self.record("INVOKE", tx_hash, transaction, receipt, trace);
        let snapshot = self.state.clone();
        let balance = self.execute(token, selector("balanceOf"), vec![address], Felt::ZERO, &DevnetExecutionContext::default(), EXECUTION_GAS_BUDGET, EntryKind::External).ok();
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
            "starknet_getBlockWithReceipts" => {
                let b = self.block_by_id(p(0, "block_id").as_ref())?.clone();
                let mut json = self.block_json(&b, false);
                let txs: Vec<serde_json::Value> = self
                    .block_txs(&b)
                    .map(|t| serde_json::json!({ "transaction": t.transaction, "receipt": t.receipt }))
                    .collect();
                json["transactions"] = serde_json::json!(txs);
                Ok(json)
            }
            "starknet_getBlockTransactionCount" => Ok(serde_json::json!(self.block_by_id(p(0, "block_id").as_ref())?.transactions.len())),
            "starknet_getNonce" => {
                if let Some(bid) = p(0, "block_id") {
                    let _ = self.block_by_id(Some(&bid))?;
                }
                let address = felt(p(1, "contract_address").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?;
                if self.state.class_hash_at(&address).is_none() {
                    return Err(RpcError::new(ERR_CONTRACT_NOT_FOUND, "Contract not found"));
                }
                Ok(serde_json::json!(format!("{:#x}", self.nonces.get(&address).copied().unwrap_or(0))))
            }
            "starknet_getClassHashAt" => {
                if let Some(bid) = p(0, "block_id") {
                    let _ = self.block_by_id(Some(&bid))?;
                }
                let address = felt(p(1, "contract_address").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?;
                match self.state.class_hash_at(&address) {
                    Some(c) if c != Felt::ZERO => Ok(serde_json::json!(hex(&c))),
                    _ => Err(RpcError::new(ERR_CONTRACT_NOT_FOUND, "Contract not found")),
                }
            }
            "starknet_getClass" => {
                if let Some(bid) = p(0, "block_id") {
                    let _ = self.block_by_id(Some(&bid))?;
                }
                let class_hash = felt(p(1, "class_hash").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?;
                self.classes.get(&class_hash).map(|c| c.class_json.clone()).ok_or_else(|| RpcError::new(ERR_CLASS_HASH_NOT_FOUND, "Class hash not found"))
            }
            "starknet_getClassAt" => {
                if let Some(bid) = p(0, "block_id") {
                    let _ = self.block_by_id(Some(&bid))?;
                }
                let address = felt(p(1, "contract_address").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?;
                let class_hash = self.state.class_hash_at(&address).filter(|c| *c != Felt::ZERO).ok_or_else(|| RpcError::new(ERR_CONTRACT_NOT_FOUND, "Contract not found"))?;
                self.classes.get(&class_hash).map(|c| c.class_json.clone()).ok_or_else(|| RpcError::new(ERR_CLASS_HASH_NOT_FOUND, "Class hash not found"))
            }
            "starknet_getCompiledCasm" => {
                let ch_val = p(0, "class_hash").or_else(|| p(1, "class_hash"));
                let class_hash = felt(ch_val.as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?;
                let entry = self
                    .classes
                    .get(&class_hash)
                    .ok_or_else(|| RpcError::new(ERR_CLASS_HASH_NOT_FOUND, "Class hash not found"))?;
                let mut val = entry.class_json.clone();
                if let Some(abi_text) = val.get("abi").and_then(|a| a.as_str()) {
                    let parsed = serde_json::from_str::<serde_json::Value>(abi_text)
                        .unwrap_or(serde_json::Value::Array(Vec::new()));
                    val["abi"] = parsed;
                }
                let sierra_class = serde_json::from_value::<ContractClass>(val)
                    .map_err(|e| RpcError::new(ERR_INVALID_CONTRACT_CLASS, format!("invalid Sierra class: {e}")))?;
                let casm = compile_native_casm_contract(sierra_class, usize::MAX)
                    .map_err(|e| RpcError::new(ERR_COMPILATION_FAILED, e.to_string()))?;
                serde_json::to_value(&casm)
                    .map_err(|e| RpcError::new(ERR_COMPILATION_FAILED, e.to_string()))
            }
            "starknet_getStorageAt" => {
                if let Some(bid) = p(2, "block_id") {
                    let _ = self.block_by_id(Some(&bid))?;
                }
                let address = felt(p(0, "contract_address").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?;
                let key = felt(p(1, "key").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?;
                if self.state.class_hash_at(&address).is_none() {
                    return Err(RpcError::new(ERR_CONTRACT_NOT_FOUND, "Contract not found"));
                }
                Ok(serde_json::json!(hex(&self.state.storage_at(&address, &key))))
            }
            "starknet_call" => {
                if let Some(bid) = p(1, "block_id") {
                    let _ = self.block_by_id(Some(&bid))?;
                }
                let request = p(0, "request").unwrap_or_default();
                self.call(&request)
            }
            "starknet_estimateFee" => {
                if let Some(bid) = p(2, "block_id") {
                    let _ = self.block_by_id(Some(&bid))?;
                }
                let txs = p(0, "request").and_then(|v| v.as_array().cloned()).unwrap_or_default();
                let flags = simulation_flags(p(1, "simulation_flags"));
                let mode = TxMode { skip_validate: flags.iter().any(|f| f == "SKIP_VALIDATE"), skip_fee_charge: true };
                let effects = self.simulate(&txs, mode)?;
                if let Some((index, reason)) = effects.iter().enumerate().find_map(|(i, e)| e.revert.clone().map(|r| (i, r))) {
                    return Err(RpcError::new(ERR_TX_EXECUTION_ERROR, "Transaction execution error")
                        .with_data(serde_json::json!({ "transaction_index": index, "execution_error": reason })));
                }
                Ok(serde_json::json!(effects.iter().map(TxEffects::fee_estimate).collect::<Vec<_>>()))
            }
            "starknet_simulateTransactions" => {
                if let Some(bid) = p(0, "block_id") {
                    let _ = self.block_by_id(Some(&bid))?;
                }
                let txs = p(1, "transactions").and_then(|v| v.as_array().cloned()).unwrap_or_default();
                let flags = simulation_flags(p(2, "simulation_flags"));
                let mode = TxMode {
                    skip_validate: flags.iter().any(|f| f == "SKIP_VALIDATE"),
                    skip_fee_charge: flags.iter().any(|f| f == "SKIP_FEE_CHARGE"),
                };
                let effects = self.simulate(&txs, mode)?;
                Ok(serde_json::json!(effects
                    .iter()
                    .map(|e| serde_json::json!({ "transaction_trace": e.trace(), "fee_estimation": e.fee_estimate() }))
                    .collect::<Vec<_>>()))
            }
            "starknet_estimateMessageFee" => {
                if let Some(bid) = p(1, "block_id") {
                    let _ = self.block_by_id(Some(&bid))?;
                }
                Ok(serde_json::json!({
                    "l1_gas_consumed": "0x0",
                    "l1_gas_price": format!("{:#x}", self.gas_price_fri),
                    "l2_gas_consumed": "0x0",
                    "l2_gas_price": format!("{:#x}", self.l2_gas_price_fri),
                    "l1_data_gas_consumed": "0x0",
                    "l1_data_gas_price": format!("{:#x}", self.data_gas_price_fri),
                    "overall_fee": "0x0",
                    "unit": "FRI",
                }))
            }
            "starknet_addDeclareTransaction" => self.add_account_tx("DECLARE", &first_tx()),
            "starknet_addDeployAccountTransaction" => self.add_account_tx("DEPLOY_ACCOUNT", &first_tx()),
            "starknet_addInvokeTransaction" => self.add_account_tx("INVOKE", &first_tx()),
            "starknet_getTransactionByHash" => Ok(self.tx_by_hash(p(0, "transaction_hash").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?.transaction.clone()),
            "starknet_getTransactionReceipt" => Ok(self.tx_by_hash(p(0, "transaction_hash").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?.receipt.clone()),
            "starknet_getTransactionStatus" => {
                let t = self.tx_by_hash(p(0, "transaction_hash").as_ref().and_then(|v| v.as_str()).unwrap_or("0x0"))?;
                Ok(serde_json::json!({
                    "finality_status": "ACCEPTED_ON_L2",
                    "execution_status": t.receipt["execution_status"],
                    "failure_reason": t.receipt.get("revert_reason").cloned().unwrap_or(serde_json::Value::Null),
                }))
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
                let b = self.block_by_id(p(0, "block_id").as_ref())?.clone();
                let diff = self.block_state_diff(&b);
                let new_root = hex(&felt_from_digest(
                    "uc-devnet-root:",
                    &[b.block_hash.as_bytes(), diff.to_string().as_bytes()],
                ));
                let old_root = if b.block_number == 0 {
                    "0x0".to_string()
                } else if let Some(prev) = self.blocks.get((b.block_number - 1) as usize) {
                    let prev_diff = self.block_state_diff(prev);
                    hex(&felt_from_digest(
                        "uc-devnet-root:",
                        &[prev.block_hash.as_bytes(), prev_diff.to_string().as_bytes()],
                    ))
                } else {
                    "0x0".to_string()
                };
                Ok(serde_json::json!({ "block_hash": b.block_hash, "new_root": new_root, "old_root": old_root, "state_diff": diff }))
            }
            "starknet_traceBlockTransactions" => {
                let b = self.block_by_id(p(0, "block_id").as_ref())?.clone();
                let traces: Vec<serde_json::Value> = b
                    .transactions
                    .iter()
                    .filter_map(|th| {
                        self.tx_by_hash(th).ok().map(|t| {
                            serde_json::json!({
                                "transaction_hash": t.transaction_hash,
                                "trace_root": t.trace,
                            })
                        })
                    })
                    .collect();
                Ok(serde_json::json!(traces))
            }
            "devnet_getPredeployedAccounts" | "devnet_predeployedAccounts" => Ok(self.predeployed_accounts_json()),
            "devnet_getAccountBalance" => {
                let address_str = p(0, "address")
                    .and_then(|v| v.as_str().map(str::to_string))
                    .ok_or_else(|| RpcError::new(ERR_INVALID_PARAMS, "address is required"))?;
                let address = felt(&address_str)?;
                let unit = p(1, "unit")
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_else(|| "FRI".to_string())
                    .to_ascii_uppercase();
                let token = parse_felt_str(if unit == "WEI" { DEVNET_ETH_ADDRESS } else { DEVNET_STRK_ADDRESS }).unwrap();
                let snapshot = self.state.clone();
                let balance = self
                    .execute(
                        token,
                        selector("balanceOf"),
                        vec![address],
                        Felt::ZERO,
                        &DevnetExecutionContext::default(),
                        EXECUTION_GAS_BUDGET,
                        EntryKind::External,
                    )
                    .ok();
                self.state = snapshot;
                let amount = balance
                    .and_then(|o| (o.retdata.len() == 2).then(|| u256_to_decimal(&o.retdata[0], &o.retdata[1])))
                    .unwrap_or_else(|| "0".to_string());
                Ok(serde_json::json!({ "amount": amount, "unit": unit }))
            }
            "devnet_impersonateAccount" => {
                let addr_str = p(0, "account_address")
                    .or_else(|| params.get("address").cloned())
                    .and_then(|v| v.as_str().map(str::to_string))
                    .ok_or_else(|| RpcError::new(ERR_INVALID_PARAMS, "account_address is required"))?;
                let addr = felt(&addr_str)?;
                if self.state.class_hash_at(&addr).filter(|c| *c != Felt::ZERO).is_none() {
                    if let Ok(acc_ch) = parse_felt_str(&self.account_class_hash) {
                        self.state.register_deployed_contract(addr, acc_ch);
                    }
                }
                self.impersonated_accounts.insert(addr);
                Ok(serde_json::json!({}))
            }
            "devnet_stopImpersonateAccount" => {
                let addr_str = p(0, "account_address")
                    .or_else(|| params.get("address").cloned())
                    .and_then(|v| v.as_str().map(str::to_string))
                    .ok_or_else(|| RpcError::new(ERR_INVALID_PARAMS, "account_address is required"))?;
                let addr = felt(&addr_str)?;
                self.impersonated_accounts.remove(&addr);
                Ok(serde_json::json!({}))
            }
            "devnet_autoImpersonate" => {
                self.auto_impersonate = true;
                Ok(serde_json::json!({}))
            }
            "devnet_stopAutoImpersonate" => {
                self.auto_impersonate = false;
                Ok(serde_json::json!({}))
            }
            "devnet_setGasPrice" => {
                if let Some(v) = u64_from_json(params.get("gas_price_wei")) {
                    self.gas_price_wei = u128::from(v);
                }
                if let Some(v) = u64_from_json(params.get("gas_price_fri")) {
                    self.gas_price_fri = u128::from(v);
                }
                if let Some(v) = u64_from_json(params.get("data_gas_price_wei")) {
                    self.data_gas_price_wei = u128::from(v);
                }
                if let Some(v) = u64_from_json(params.get("data_gas_price_fri")) {
                    self.data_gas_price_fri = u128::from(v);
                }
                if let Some(v) = u64_from_json(params.get("l2_gas_price_wei")) {
                    self.l2_gas_price_wei = u128::from(v);
                }
                if let Some(v) = u64_from_json(params.get("l2_gas_price_fri")) {
                    self.l2_gas_price_fri = u128::from(v);
                }
                if params.get("generate_block").and_then(|v| v.as_bool()).unwrap_or(false) {
                    self.seal_block();
                }
                Ok(serde_json::json!({
                    "gas_price_wei": self.gas_price_wei,
                    "gas_price_fri": self.gas_price_fri,
                    "data_gas_price_wei": self.data_gas_price_wei,
                    "data_gas_price_fri": self.data_gas_price_fri,
                    "l2_gas_price_wei": self.l2_gas_price_wei,
                    "l2_gas_price_fri": self.l2_gas_price_fri,
                }))
            }
            "devnet_abortBlocks" => {
                let start_block = self
                    .block_by_id(p(0, "starting_block_id").as_ref())?
                    .block_number;
                let mut aborted = Vec::new();
                while self.blocks.len() > start_block as usize && self.blocks.len() > 1 {
                    if let Some(b) = self.blocks.pop() {
                        aborted.push(b.block_hash);
                    }
                }
                Ok(serde_json::json!({ "aborted": aborted }))
            }
            "devnet_restart" => {
                self.impersonated_accounts.clear();
                self.auto_impersonate = false;
                self.pending.clear();
                self.txs.clear();
                self.tx_index.clear();
                self.nonces.clear();
                if self.blocks.len() > 1 {
                    self.blocks.truncate(1);
                }
                Ok(serde_json::json!({}))
            }
            "devnet_dump" => {
                let dump_payload = serde_json::json!({
                    "chain_id": hex(&self.chain_id),
                    "timestamp": self.timestamp,
                    "blocks": self.blocks,
                    "transactions": self.txs,
                });
                if let Some(path_str) = p(0, "path").and_then(|v| v.as_str().map(str::to_string)) {
                    let p_buf = PathBuf::from(&path_str);
                    if let Some(parent) = p_buf.parent() {
                        if !parent.as_os_str().is_empty() {
                            let _ = fs::create_dir_all(parent);
                        }
                    }
                    let _ = fs::write(&p_buf, serde_json::to_string_pretty(&dump_payload).unwrap_or_default());
                    Ok(serde_json::Value::Null)
                } else {
                    Ok(dump_payload)
                }
            }
            "devnet_load" => {
                let path_str = p(0, "path")
                    .and_then(|v| v.as_str().map(str::to_string))
                    .ok_or_else(|| RpcError::new(ERR_INVALID_PARAMS, "path is required"))?;
                let raw = fs::read_to_string(&path_str)
                    .map_err(|e| RpcError::new(ERR_INVALID_PARAMS, format!("failed to read dump `{path_str}`: {e}")))?;
                let doc: serde_json::Value = serde_json::from_str(&raw)
                    .map_err(|e| RpcError::new(ERR_INVALID_PARAMS, format!("invalid dump JSON: {e}")))?;
                if let Some(ts) = doc.get("timestamp").and_then(|v| v.as_u64()) {
                    self.timestamp = ts;
                }
                Ok(serde_json::json!({}))
            }
            "devnet_postmanLoad" => {
                if let Some(addr) = params.get("address").and_then(|v| v.as_str()) {
                    self.l1_messaging_contract = addr.to_string();
                }
                Ok(serde_json::json!({ "messaging_contract_address": self.l1_messaging_contract }))
            }
            "devnet_postmanFlush" => Ok(serde_json::json!({
                "messages_to_l1": [],
                "messages_to_l2": [],
                "generated_l2_transactions": [],
                "l1_provider": "local",
            })),
            "devnet_postmanSendMessageToL2" => {
                let l2_contract = felt(
                    params
                        .get("l2_contract_address")
                        .or_else(|| params.get("contract_address"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("0x0"),
                )?;
                let l1_sender = felt(
                    params
                        .get("l1_contract_address")
                        .or_else(|| params.get("from_address"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("0x0"),
                )?;
                let ep_selector = felt(
                    params
                        .get("entry_point_selector")
                        .and_then(|v| v.as_str())
                        .unwrap_or("0x0"),
                )?;
                let mut calldata = vec![l1_sender];
                calldata.extend(felts_of_json(params.get("payload")));
                let before = self.state.clone();
                if self.state.class_hash_at(&l2_contract).is_some() {
                    let _ = self.execute(
                        l2_contract,
                        ep_selector,
                        calldata.clone(),
                        Felt::ZERO,
                        &DevnetExecutionContext::default(),
                        EXECUTION_GAS_BUDGET,
                        EntryKind::L1Handler,
                    );
                }
                let tx_hash = felt_from_digest(
                    "uc-devnet-l1-handler:",
                    &[
                        &l2_contract.to_bytes_be(),
                        &ep_selector.to_bytes_be(),
                        &(self.txs.len() as u64).to_le_bytes(),
                    ],
                );
                let receipt = serde_json::json!({
                    "actual_fee": { "amount": "0x0", "unit": "WEI" },
                    "execution_status": "SUCCEEDED",
                    "messages_sent": [],
                    "events": [],
                    "execution_resources": { "l1_gas": 0, "l1_data_gas": 0, "l2_gas": 0 },
                });
                let trace = serde_json::json!({
                    "type": "L1_HANDLER",
                    "state_diff": self.state_diff_json(&before, None, None),
                });
                let transaction = serde_json::json!({
                    "type": "L1_HANDLER",
                    "version": "0x0",
                    "nonce": params.get("nonce").and_then(|v| v.as_str()).unwrap_or("0x0"),
                    "contract_address": hex(&l2_contract),
                    "entry_point_selector": hex(&ep_selector),
                    "calldata": hex_vec(&calldata),
                });
                let hash_hex = self.record("L1_HANDLER", tx_hash, transaction, receipt, trace);
                Ok(serde_json::json!({ "transaction_hash": hash_hex }))
            }
            "devnet_postmanConsumeMessageFromL2" => {
                let from_addr = params.get("from_address").and_then(|v| v.as_str()).unwrap_or("0x0");
                let to_addr = params.get("to_address").and_then(|v| v.as_str()).unwrap_or("0x0");
                let msg_hash = felt_from_digest("uc-devnet-l2-to-l1-msg:", &[from_addr.as_bytes(), to_addr.as_bytes()]);
                Ok(serde_json::json!({ "message_hash": hex(&msg_hash) }))
            }
            "devnet_getConfig" => Ok(serde_json::json!({ "chain_id": hex(&self.chain_id), "block_generation_on": if self.block_on_demand { "demand" } else { "transaction" }, "account_class_hash": self.account_class_hash, "strk_fee_token_address": DEVNET_STRK_ADDRESS, "eth_fee_token_address": DEVNET_ETH_ADDRESS, "udc_address": STARKNET_UDC_ADDRESS, "gas_price_wei": self.gas_price_wei, "gas_price_fri": self.gas_price_fri, "data_gas_price_wei": self.data_gas_price_wei, "data_gas_price_fri": self.data_gas_price_fri })),
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
            "starknet_getMessagesStatus" => {
                let tx_hash = p(0, "transaction_hash")
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_else(|| "0x0".to_string());
                Ok(serde_json::json!([{
                    "transaction_hash": tx_hash,
                    "finality_status": "ACCEPTED_ON_L2",
                    "execution_status": "SUCCEEDED",
                }]))
            }
            "starknet_getStorageProof" => {
                let bid = p(0, "block_id");
                if bid.as_ref().and_then(|v| v.as_str()) == Some("pre_confirmed") {
                    return Err(RpcError::new(ERR_INVALID_PARAMS, "storage proof is not supported for pre_confirmed block"));
                }
                let b = self.block_by_id(bid.as_ref())?.clone();
                let root = hex(&felt_from_digest(
                    "uc-devnet-root:",
                    &[&b.block_number.to_le_bytes(), b.block_hash.as_bytes()],
                ));
                Ok(serde_json::json!({
                    "classes_proof": [],
                    "contracts_proof": {
                        "nodes": [],
                        "contract_leaves_data": [],
                    },
                    "contracts_storage_proofs": [],
                    "global_roots": {
                        "contracts_tree_root": root,
                        "classes_tree_root": root,
                        "block_hash": b.block_hash,
                    }
                }))
            }
            "starknet_subscribeNewHeads"
            | "starknet_subscribeEvents"
            | "starknet_subscribeTransactionStatus"
            | "starknet_subscribeNewTransactionReceipts"
            | "starknet_subscribeNewTransactions"
            | "starknet_subscribePendingTransactions" => {
                let sub_id = format!("0x{:x}", self.blocks.len() + self.txs.len() + 1);
                Ok(serde_json::json!(sub_id))
            }
            "starknet_unsubscribe" => Ok(serde_json::json!(true)),
            _ => Err(RpcError::new(ERR_UNSUPPORTED, format!("Method not found: {method}"))),
        }
    }
}

/// How `run_account_tx` treats a transaction: Starknet's simulation flags.
#[derive(Debug, Clone, Copy, Default)]
struct TxMode {
    skip_validate: bool,
    skip_fee_charge: bool,
}

/// V3 resource bounds, `(max_amount, max_price_per_unit)` per resource, and the tip.
#[derive(Debug, Clone, Copy, Default)]
struct ResourceBounds {
    l1_gas: (u128, u128),
    l2_gas: (u128, u128),
    l1_data_gas: (u128, u128),
    tip: u128,
}

impl ResourceBounds {
    fn of(tx: &serde_json::Value) -> Self {
        let num = |v: Option<&serde_json::Value>| -> u128 {
            v.and_then(|x| x.as_str())
                .and_then(|s| parse_felt_str(s).ok())
                .and_then(|f| u128::try_from(f.to_biguint()).ok())
                .unwrap_or(0)
        };
        let bound = |name: &str| {
            let b = tx.get("resource_bounds").and_then(|r| r.get(name));
            (num(b.and_then(|b| b.get("max_amount"))), num(b.and_then(|b| b.get("max_price_per_unit"))))
        };
        Self { l1_gas: bound("l1_gas"), l2_gas: bound("l2_gas"), l1_data_gas: bound("l1_data_gas"), tip: num(tx.get("tip")) }
    }

    /// The most the transaction can be charged.
    fn max_fee(&self) -> u128 {
        self.l1_gas
            .0
            .saturating_mul(self.l1_gas.1)
            .saturating_add(self.l2_gas.0.saturating_mul(self.l2_gas.1.saturating_add(self.tip)))
            .saturating_add(self.l1_data_gas.0.saturating_mul(self.l1_data_gas.1))
    }
}

/// Everything one account transaction did, before it is recorded or rolled back.
struct TxEffects {
    tx_type: &'static str,
    tx_hash: Felt,
    transaction: serde_json::Value,
    validate: Option<ExecutionOutcome>,
    /// `__execute__` for INVOKE, the constructor for DEPLOY_ACCOUNT.
    execute: Option<ExecutionOutcome>,
    fee_transfer: Option<ExecutionOutcome>,
    revert: Option<String>,
    l2_gas: u64,
    l1_data_gas: u64,
    l1_gas_price: u128,
    l2_gas_price: u128,
    l1_data_gas_price: u128,
    fee: u128,
    state_diff: serde_json::Value,
    contract_address: Option<Felt>,
    class_hash: Option<Felt>,
}

impl TxEffects {
    fn execution_resources(&self) -> serde_json::Value {
        serde_json::json!({ "l1_gas": 0, "l1_data_gas": self.l1_data_gas, "l2_gas": self.l2_gas })
    }

    fn fee_estimate(&self) -> serde_json::Value {
        serde_json::json!({
            "l1_gas_consumed": "0x0",
            "l1_gas_price": format!("{:#x}", self.l1_gas_price),
            "l2_gas_consumed": format!("{:#x}", self.l2_gas),
            "l2_gas_price": format!("{:#x}", self.l2_gas_price),
            "l1_data_gas_consumed": format!("{:#x}", self.l1_data_gas),
            "l1_data_gas_price": format!("{:#x}", self.l1_data_gas_price),
            "overall_fee": format!("{:#x}", self.fee),
            "unit": "FRI",
        })
    }

    /// The receipt body; `record` adds the hash, type, finality and block.
    fn receipt(&self) -> serde_json::Value {
        let mut outcomes: Vec<&ExecutionOutcome> = Vec::new();
        if self.revert.is_none() {
            outcomes.extend(self.execute.as_ref());
        }
        outcomes.extend(self.fee_transfer.as_ref());
        let mut receipt = serde_json::json!({
            "actual_fee": { "amount": format!("{:#x}", self.fee), "unit": "FRI" },
            "execution_status": if self.revert.is_some() { "REVERTED" } else { "SUCCEEDED" },
            "messages_sent": receipt_messages(&outcomes),
            "events": receipt_events(&outcomes),
            "execution_resources": self.execution_resources(),
        });
        if let Some(reason) = &self.revert {
            receipt["revert_reason"] = serde_json::json!(reason);
        }
        if let Some(address) = self.contract_address {
            receipt["contract_address"] = serde_json::json!(hex(&address));
        }
        receipt
    }

    fn trace(&self) -> serde_json::Value {
        let mut trace = serde_json::json!({ "type": self.tx_type, "state_diff": self.state_diff, "execution_resources": self.execution_resources() });
        if let Some(v) = &self.validate {
            trace["validate_invocation"] = invocation_json(v, "EXTERNAL");
        }
        if let Some(f) = &self.fee_transfer {
            trace["fee_transfer_invocation"] = invocation_json(f, "EXTERNAL");
        }
        match (self.tx_type, &self.execute, &self.revert) {
            ("INVOKE", _, Some(reason)) => trace["execute_invocation"] = serde_json::json!({ "revert_reason": reason }),
            ("INVOKE", Some(e), None) => trace["execute_invocation"] = invocation_json(e, "EXTERNAL"),
            ("DEPLOY_ACCOUNT", Some(c), _) => trace["constructor_invocation"] = invocation_json(c, "CONSTRUCTOR"),
            _ => {}
        }
        trace
    }
}

fn recorded_call_json(c: &cairo_lang_runner::casm_run::RecordedCall) -> serde_json::Value {
    serde_json::json!({
        "contract_address": hex(&c.contract_address),
        "entry_point_selector": hex(&c.entry_point_selector),
        "calldata": hex_vec(&c.calldata),
        "caller_address": hex(&c.caller_address),
        "class_hash": hex(&c.class_hash),
        "entry_point_type": c.entry_point_type,
        "call_type": c.call_type,
        "result": hex_vec(&c.result),
        "calls": c.calls.iter().map(recorded_call_json).collect::<Vec<_>>(),
        "events": c.events.iter().enumerate().map(|(order, (_, keys, data))| serde_json::json!({ "order": order, "keys": hex_vec(keys), "data": hex_vec(data) })).collect::<Vec<_>>(),
        "messages": c.messages.iter().enumerate().map(|(order, (from, to, payload))| serde_json::json!({ "order": order, "from_address": hex(from), "to_address": hex(to), "payload": hex_vec(payload) })).collect::<Vec<_>>(),
        "execution_resources": { "l1_gas": 0, "l2_gas": c.l2_gas },
        "is_reverted": false,
    })
}

/// An RPC `FUNCTION_INVOCATION` with recursive nested `calls`.
fn invocation_json(o: &ExecutionOutcome, entry_point_type: &str) -> serde_json::Value {
    serde_json::json!({
        "contract_address": hex(&o.contract),
        "entry_point_selector": hex(&o.selector),
        "calldata": hex_vec(&o.calldata),
        "caller_address": hex(&o.caller),
        "class_hash": hex(&o.class_hash),
        "entry_point_type": entry_point_type,
        "call_type": "CALL",
        "result": hex_vec(&o.retdata),
        "calls": o.calls.iter().map(recorded_call_json).collect::<Vec<_>>(),
        "events": o.events.iter().enumerate().map(|(order, (_, keys, data))| serde_json::json!({ "order": order, "keys": hex_vec(keys), "data": hex_vec(data) })).collect::<Vec<_>>(),
        "messages": o.messages.iter().enumerate().map(|(order, (from, to, payload))| serde_json::json!({ "order": order, "from_address": hex(from), "to_address": hex(to), "payload": hex_vec(payload) })).collect::<Vec<_>>(),
        "execution_resources": { "l1_gas": 0, "l2_gas": o.l2_gas },
        "is_reverted": o.panic.is_some(),
    })
}

fn receipt_events(outcomes: &[&ExecutionOutcome]) -> Vec<serde_json::Value> {
    outcomes
        .iter()
        .flat_map(|o| o.events.iter())
        .map(|(from, keys, data)| serde_json::json!({ "from_address": hex(from), "keys": hex_vec(keys), "data": hex_vec(data) }))
        .collect()
}

fn receipt_messages(outcomes: &[&ExecutionOutcome]) -> Vec<serde_json::Value> {
    outcomes
        .iter()
        .flat_map(|o| o.messages.iter())
        .map(|(from, to, payload)| serde_json::json!({ "from_address": hex(from), "to_address": hex(to), "payload": hex_vec(payload) }))
        .collect()
}

/// Contracts whose class changed between two states: deployments and replaced classes.
fn class_changes(before: &StarknetState, after: &StarknetState) -> Vec<(Felt, Felt)> {
    let mut changes: Vec<(Felt, Felt)> = after
        .deployed_contracts()
        .into_iter()
        .filter(|(address, class)| *class != Felt::ZERO && before.class_hash_at(address) != Some(*class))
        .collect();
    changes.sort();
    changes
}

/// The storage cell of the low word of `owner`'s balance in the devnet fee token.
fn balance_key(owner: &Felt) -> Felt {
    starknet_crypto::pedersen_hash(&selector("balances"), owner)
}

/// The compiled class hash a sequencer computes for a declared Sierra class (Blake2s), and the
/// length of its CASM bytecode.
fn compile_declared_class(class_json: &serde_json::Value) -> Result<(Felt, usize), RpcError> {
    let class: ContractClass = serde_json::from_value(normalize_class_json(class_json.clone()))
        .map_err(|e| RpcError::new(ERR_INVALID_PARAMS, format!("not a Sierra contract class: {e}")))?;
    let casm = compile_native_casm_contract(class, usize::MAX)
        .map_err(|e| RpcError::new(ERR_INVALID_PARAMS, format!("the class does not compile to CASM: {e:#}")))?;
    Ok((casm.compiled_class_hash(), casm.bytecode.len()))
}

/// L2 gas a DECLARE pays for its code.
fn declare_code_gas(class_json: &serde_json::Value, casm_len: usize) -> u64 {
    let sierra_len = class_json.get("sierra_program").and_then(|p| p.as_array()).map_or(0, Vec::len);
    let abi_len = match class_json.get("abi") {
        Some(serde_json::Value::String(abi)) => abi.len(),
        Some(abi) => declared_abi_string(&serde_json::json!({ "abi": abi })).len(),
        None => 0,
    };
    GAS_PER_CODE_BYTE * ((sierra_len + casm_len) * 32 + abi_len) as u64
}

fn simulation_flags(v: Option<serde_json::Value>) -> Vec<String> {
    v.and_then(|f| f.as_array().cloned()).unwrap_or_default().iter().filter_map(|f| f.as_str().map(str::to_string)).collect()
}

/// Number of entries in an RPC state diff.
fn state_diff_length(diff: &serde_json::Value) -> usize {
    let len = |k: &str| diff[k].as_array().map_or(0, Vec::len);
    let storage: usize = diff["storage_diffs"].as_array().into_iter().flatten().map(|sd| sd["storage_entries"].as_array().map_or(0, Vec::len)).sum();
    storage + len("nonces") + len("deployed_contracts") + len("declared_classes") + len("replaced_classes")
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

    fn u256_of(retdata: &serde_json::Value) -> String {
        let a = retdata.as_array().unwrap();
        let low = parse_felt_str(a[0].as_str().unwrap()).unwrap();
        let high = parse_felt_str(a[1].as_str().unwrap()).unwrap();
        u256_to_decimal(&low, &high)
    }

    const L2: u64 = 100_000_000;

    /// Resource bounds at the devnet's gas price.
    fn bounds(l2_amount: u64) -> serde_json::Value {
        let price = format!("{DEVNET_GAS_PRICE:#x}");
        serde_json::json!({
            "l1_gas": { "max_amount": "0x0", "max_price_per_unit": price },
            "l2_gas": { "max_amount": format!("{l2_amount:#x}"), "max_price_per_unit": price },
            "l1_data_gas": { "max_amount": "0x2710", "max_price_per_unit": price },
        })
    }

    /// `tx` completed with the V3 fields and signed with `key`.
    fn signed(chain_id: Felt, mut tx: serde_json::Value, nonce: u64, bounds: serde_json::Value, key: &Felt) -> serde_json::Value {
        tx["version"] = serde_json::json!("0x3");
        tx["nonce"] = serde_json::json!(format!("{nonce:#x}"));
        tx["resource_bounds"] = bounds;
        for (k, v) in [("tip", serde_json::json!("0x0")), ("paymaster_data", serde_json::json!([])), ("account_deployment_data", serde_json::json!([])), ("nonce_data_availability_mode", serde_json::json!("L1")), ("fee_data_availability_mode", serde_json::json!("L1"))] {
            tx[k] = v;
        }
        sign_v3_transaction(&mut tx, chain_id, Some(key)).unwrap();
        tx
    }

    fn signed_invoke(chain_id: Felt, acc: &DevnetPredeployedAccount, calldata: Vec<String>, nonce: u64, bounds: serde_json::Value, key: Option<Felt>) -> serde_json::Value {
        let key = key.unwrap_or_else(|| parse_felt_str(&acc.private_key).unwrap());
        signed(chain_id, serde_json::json!({ "type": "INVOKE", "sender_address": acc.address, "calldata": calldata }), nonce, bounds, &key)
    }

    /// Sends an INVOKE and returns its receipt.
    fn invoke(chain: &mut DevnetChain, tx: serde_json::Value) -> Result<serde_json::Value, RpcError> {
        let res = chain.handle_rpc("starknet_addInvokeTransaction", &serde_json::json!({ "invoke_transaction": tx }))?;
        Ok(chain.handle_rpc("starknet_getTransactionReceipt", &serde_json::json!({ "transaction_hash": res["transaction_hash"] })).unwrap())
    }

    fn strk_balance_of(chain: &mut DevnetChain, address: &str) -> u128 {
        let bal = chain.handle_rpc("starknet_call", &serde_json::json!({ "request": { "contract_address": DEVNET_STRK_ADDRESS, "entry_point_selector": hex(&selector("balanceOf")), "calldata": [address] }, "block_id": "latest" })).unwrap();
        u256_of(&bal).parse().unwrap()
    }

    fn hex_u128(v: &serde_json::Value) -> u128 {
        u128::from_str_radix(v.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
    }

    fn transfer(to: &str, low: &str, high: &str) -> Vec<String> {
        vec!["0x1".to_string(), DEVNET_STRK_ADDRESS.to_string(), hex(&selector("transfer")), "0x3".to_string(), to.to_string(), low.to_string(), high.to_string()]
    }

    #[test]
    #[cfg(feature = "native-test-runner")]
    fn devnet_validates_charges_fees_and_reverts_like_a_sequencer() {
        let _env = crate::test_env_shared();
        let mut chain = test_chain();
        let id = chain.chain_id;
        let acc = chain.accounts[0].clone();
        let other = chain.accounts[1].clone();
        let nonce_of = |chain: &mut DevnetChain| chain.handle_rpc("starknet_getNonce", &serde_json::json!({ "block_id": "latest", "contract_address": acc.address })).unwrap();
        let start = strk_balance_of(&mut chain, &acc.address);
        assert_eq!(start, 1_000_000_000_000_000_000_000);

        // A signed transfer of 5: the fee, in STRK, goes to the sequencer after the call's events.
        let receipt = invoke(&mut chain, signed_invoke(id, &acc, transfer(&other.address, "0x5", "0x0"), 0, bounds(L2), None)).unwrap();
        assert_eq!(receipt["execution_status"], "SUCCEEDED", "{receipt}");
        let l2 = receipt["execution_resources"]["l2_gas"].as_u64().unwrap();
        let data = receipt["execution_resources"]["l1_data_gas"].as_u64().unwrap();
        // Data gas: the account's nonce (2 felts) and the two balance cells (4 felts); the fee
        // token is not counted as a modified contract.
        assert_eq!(data, 6 * DATA_GAS_PER_FELT, "{receipt}");
        let fee = hex_u128(&receipt["actual_fee"]["amount"]);
        assert_eq!(fee, (u128::from(l2) + u128::from(data)) * DEVNET_GAS_PRICE);
        assert_eq!(strk_balance_of(&mut chain, &acc.address), start - 5 - fee);
        let events = receipt["events"].as_array().unwrap();
        assert_eq!(events.len(), 2, "the transfer, then the fee transfer: {events:?}");
        assert_eq!(parse_felt_str(events[1]["keys"][2].as_str().unwrap()).unwrap(), Felt::from(0x1000_u64), "the sequencer gets the fee");
        let trace = chain.handle_rpc("starknet_traceTransaction", &serde_json::json!({ "transaction_hash": receipt["transaction_hash"] })).unwrap();
        for key in ["validate_invocation", "execute_invocation", "fee_transfer_invocation", "state_diff"] {
            assert!(trace.get(key).is_some_and(|v| !v.is_null()), "{key} missing: {trace}");
        }
        assert_eq!(trace["validate_invocation"]["result"], serde_json::json!([hex(&Felt::from_bytes_be_slice(b"VALID"))]));
        let validate_gas = trace["validate_invocation"]["execution_resources"]["l2_gas"].as_u64().unwrap();
        let update = chain.handle_rpc("starknet_getStateUpdate", &serde_json::json!({ "block_id": "latest" })).unwrap();
        assert_eq!(update["state_diff"]["nonces"][0]["nonce"], "0x1");
        assert!(update["state_diff"]["storage_diffs"].as_array().unwrap().iter().any(|d| parse_felt_str(d["address"].as_str().unwrap()).unwrap() == parse_felt_str(DEVNET_STRK_ADDRESS).unwrap()));
        let block = chain.handle_rpc("starknet_getBlockWithTxHashes", &serde_json::json!({ "block_id": "latest" })).unwrap();
        assert_eq!((block["transaction_count"].clone(), block["event_count"].clone(), block["starknet_version"].clone()), (serde_json::json!(1), serde_json::json!(2), serde_json::json!(DEVNET_STARKNET_VERSION)));

        // Rejected, so no transaction and no nonce bump: a stale nonce, another key, no signature,
        // a gas price under the block's, a maximum fee above the balance.
        let pay_one = || transfer(&other.address, "0x1", "0x0");
        assert_eq!(invoke(&mut chain, signed_invoke(id, &acc, pay_one(), 0, bounds(L2), None)).unwrap_err().code, ERR_INVALID_NONCE);
        let forged = invoke(&mut chain, signed_invoke(id, &acc, pay_one(), 1, bounds(L2), Some(Felt::from(99_u64)))).unwrap_err();
        assert_eq!(forged.code, ERR_VALIDATION_FAILURE);
        assert!(forged.data.unwrap().to_string().contains("invalid signature"));
        let mut unsigned = signed_invoke(id, &acc, pay_one(), 1, bounds(L2), None);
        unsigned["signature"] = serde_json::json!([]);
        assert_eq!(invoke(&mut chain, unsigned).unwrap_err().code, ERR_VALIDATION_FAILURE);
        let mut cheap = bounds(L2);
        cheap["l2_gas"]["max_price_per_unit"] = serde_json::json!("0x1");
        assert_eq!(invoke(&mut chain, signed_invoke(id, &acc, pay_one(), 1, cheap, None)).unwrap_err().code, ERR_INSUFFICIENT_RESOURCES);
        assert_eq!(invoke(&mut chain, signed_invoke(id, &acc, pay_one(), 1, bounds(1_000_000_000_000_000), None)).unwrap_err().code, ERR_INSUFFICIENT_BALANCE);
        assert_eq!(nonce_of(&mut chain), serde_json::json!("0x1"));

        // A panicking call reverts: the nonce moves and the fee is charged, the transfer is undone.
        let other_before = strk_balance_of(&mut chain, &other.address);
        let receipt = invoke(&mut chain, signed_invoke(id, &acc, transfer(&other.address, "0x0", "0x1000"), 1, bounds(L2), None)).unwrap();
        assert_eq!(receipt["execution_status"], "REVERTED");
        assert!(receipt["revert_reason"].as_str().unwrap().contains("INSUFFICIENT_BALANCE"), "{receipt}");
        assert!(hex_u128(&receipt["actual_fee"]["amount"]) > 0);
        assert_eq!(receipt["events"].as_array().unwrap().len(), 1, "only the fee transfer: {receipt}");
        assert_eq!(strk_balance_of(&mut chain, &other.address), other_before);
        let status = chain.handle_rpc("starknet_getTransactionStatus", &serde_json::json!({ "transaction_hash": receipt["transaction_hash"] })).unwrap();
        assert!(status["failure_reason"].as_str().unwrap().contains("INSUFFICIENT_BALANCE"));

        // L2 gas for validation and a little more: execution runs out of gas and reverts.
        let receipt = invoke(&mut chain, signed_invoke(id, &acc, pay_one(), 2, bounds(validate_gas + 5_000), None)).unwrap();
        assert_eq!(receipt["execution_status"], "REVERTED", "{receipt}");
        assert!(receipt["revert_reason"].as_str().unwrap().to_ascii_lowercase().contains("out of gas"), "{receipt}");
        assert_eq!(nonce_of(&mut chain), serde_json::json!("0x3"));

        // estimateFee and simulateTransactions run on a copy of the state, and the estimate is
        // what the transaction is then charged.
        let probe = signed_invoke(id, &acc, pay_one(), 3, bounds(L2), None);
        let estimate = chain.handle_rpc("starknet_estimateFee", &serde_json::json!({ "request": [probe.clone()], "simulation_flags": [], "block_id": "latest" })).unwrap();
        let skip_validate = chain.handle_rpc("starknet_estimateFee", &serde_json::json!({ "request": [probe.clone()], "simulation_flags": ["SKIP_VALIDATE"], "block_id": "latest" })).unwrap();
        assert!(hex_u128(&skip_validate[0]["l2_gas_consumed"]) < hex_u128(&estimate[0]["l2_gas_consumed"]));
        let sim = chain.handle_rpc("starknet_simulateTransactions", &serde_json::json!({ "block_id": "latest", "transactions": [probe.clone()], "simulation_flags": [] })).unwrap();
        assert_eq!(sim[0]["fee_estimation"], estimate[0]);
        assert!(sim[0]["transaction_trace"]["fee_transfer_invocation"].is_object());
        assert_eq!(nonce_of(&mut chain), serde_json::json!("0x3"));
        let receipt = invoke(&mut chain, probe).unwrap();
        assert_eq!(u128::from(receipt["execution_resources"]["l2_gas"].as_u64().unwrap()), hex_u128(&estimate[0]["l2_gas_consumed"]));
        assert_eq!(receipt["actual_fee"]["amount"], estimate[0]["overall_fee"]);
        let reverting = signed_invoke(id, &acc, transfer(&other.address, "0x0", "0x1000"), 4, bounds(L2), None);
        let err = chain.handle_rpc("starknet_estimateFee", &serde_json::json!({ "request": [reverting], "simulation_flags": [], "block_id": "latest" })).unwrap_err();
        assert_eq!(err.code, ERR_TX_EXECUTION_ERROR);
    }

    #[test]
    #[cfg(feature = "native-test-runner")]
    fn devnet_declares_deploys_and_deploys_accounts() {
        let _env = crate::test_env_shared();
        let mut chain = test_chain();
        let id = chain.chain_id;
        let acc = chain.accounts[0].clone();
        let key = parse_felt_str(&acc.private_key).unwrap();
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
    #[l1_handler]
    fn bump(ref self: ContractState, from_address: felt252, by: u64) { self.value.write(self.value.read() + by); }
}
"#).unwrap();
        run_build(BuildArgs {
            common: BuildCommonArgs { manifest_path: Some(root.join("Scarb.toml")), package: None, workspace: false, features: Vec::new(), offline: true, release: false, profile: Some("dev".to_string()) },
            engine: EngineArg::Uc, daemon_mode: DaemonModeArg::Off, report_path: Some(root.join("build.json")), record_failure: None, json: false, plan_only: false,
        }).expect("build counter");
        let class_json: serde_json::Value = serde_json::from_str(&fs::read_to_string(root.join("target/dev/counter_Counter.contract_class.json")).unwrap()).unwrap();
        let _ = fs::remove_dir_all(&root);
        let class_hash = compute_class_hash_from_json(&class_json).unwrap();
        let compiled = hex(&compile_declared_class(&class_json).unwrap().0);
        let declare = |nonce: u64, compiled: &str| {
            let tx = serde_json::json!({ "type": "DECLARE", "sender_address": acc.address, "contract_class": class_json, "class_hash": class_hash, "compiled_class_hash": compiled });
            serde_json::json!({ "declare_transaction": signed(id, tx, nonce, bounds(L2), &key) })
        };

        // The compiled class hash must be the class's Blake2s hash.
        let err = chain.handle_rpc("starknet_addDeclareTransaction", &declare(0, "0x1234")).unwrap_err();
        assert_eq!(err.code, ERR_COMPILED_CLASS_HASH_MISMATCH);
        assert!(chain.handle_rpc("starknet_getClass", &serde_json::json!({ "block_id": "latest", "class_hash": class_hash })).is_err(), "a rejected declare declares nothing");
        let declared = chain.handle_rpc("starknet_addDeclareTransaction", &declare(0, &compiled)).unwrap();
        assert_eq!(parse_felt_str(declared["class_hash"].as_str().unwrap()).unwrap(), parse_felt_str(&class_hash).unwrap());
        let update = chain.handle_rpc("starknet_getStateUpdate", &serde_json::json!({ "block_id": "latest" })).unwrap();
        assert_eq!(update["state_diff"]["declared_classes"][0]["compiled_class_hash"], compiled);
        assert_eq!(chain.handle_rpc("starknet_addDeclareTransaction", &declare(1, &compiled)).unwrap_err().code, ERR_CLASS_ALREADY_DECLARED);

        // The UDC deploys it with constructor(start = 40); increase by 2 and read 42.
        let udc_calldata = vec!["0x1".to_string(), STARKNET_UDC_ADDRESS.to_string(), hex(&selector("deployContract")), "0x5".to_string(), class_hash.clone(), "0x7".to_string(), "0x0".to_string(), "0x1".to_string(), "0x28".to_string()];
        let receipt = invoke(&mut chain, signed_invoke(id, &acc, udc_calldata, 1, bounds(L2), None)).unwrap();
        assert_eq!(receipt["execution_status"], "SUCCEEDED", "{receipt}");
        let deployed = receipt["events"].as_array().unwrap().iter()
            .find(|e| e["from_address"].as_str().map(|s| parse_felt_str(s).unwrap()) == Some(parse_felt_str(STARKNET_UDC_ADDRESS).unwrap()))
            .and_then(|e| e["data"][0].as_str()).expect("ContractDeployed event").to_string();
        let expected = calculate_contract_address(&Felt::from(7_u64), &parse_felt_str(&class_hash).unwrap(), &[Felt::from(40_u64)], &Felt::ZERO);
        assert_eq!(parse_felt_str(&deployed).unwrap(), expected, "UDC address must follow the Starknet formula for unique = 0");
        let update = chain.handle_rpc("starknet_getStateUpdate", &serde_json::json!({ "block_id": "latest" })).unwrap();
        assert!(update["state_diff"]["deployed_contracts"].as_array().unwrap().iter().any(|d| parse_felt_str(d["address"].as_str().unwrap()).unwrap() == expected));
        let inc = vec!["0x1".to_string(), deployed.clone(), hex(&selector("increase")), "0x1".to_string(), "0x2".to_string()];
        invoke(&mut chain, signed_invoke(id, &acc, inc.clone(), 2, bounds(L2), None)).unwrap();
        let got = chain.handle_rpc("starknet_call", &serde_json::json!({ "request": { "contract_address": deployed, "entry_point_selector": hex(&selector("get")), "calldata": [] }, "block_id": "latest" })).unwrap();
        assert_eq!(got, serde_json::json!(["0x2a"]));
        // Calls reach external entry points only: a missing one and the L1 handler are both
        // ENTRYPOINT_NOT_FOUND (21), and the handler does not run.
        for name in ["no_such_entry_point", "bump"] {
            let call = serde_json::json!({ "request": { "contract_address": deployed, "entry_point_selector": hex(&selector(name)), "calldata": ["0x0", "0x5"] }, "block_id": "latest" });
            assert_eq!(chain.handle_rpc("starknet_call", &call).unwrap_err().code, ERR_ENTRYPOINT_NOT_FOUND, "{name}");
        }
        let got = chain.handle_rpc("starknet_call", &serde_json::json!({ "request": { "contract_address": deployed, "entry_point_selector": hex(&selector("get")), "calldata": [] }, "block_id": "latest" })).unwrap();
        assert_eq!(got, serde_json::json!(["0x2a"]));
        let events = chain.handle_rpc("starknet_getEvents", &serde_json::json!({ "filter": { "address": DEVNET_STRK_ADDRESS, "chunk_size": 10 } })).unwrap();
        let first = &events["events"][0];
        assert!(first.get("transaction_index").is_some() && first.get("event_index").is_some(), "{first}");

        // DEPLOY_ACCOUNT: the new account pays its own fee, from a balance minted beforehand.
        let new_key = Felt::from(0x1234_u64);
        let public_key = starknet_crypto::get_public_key(&new_key);
        let salt = Felt::from(0x123_u64);
        let address = calculate_contract_address(&salt, &parse_felt_str(&chain.account_class_hash).unwrap(), &[public_key], &Felt::ZERO);
        let account_class_hash = chain.account_class_hash.clone();
        let deploy_tx = |key: &Felt| {
            let tx = serde_json::json!({ "type": "DEPLOY_ACCOUNT", "class_hash": account_class_hash, "contract_address_salt": hex(&salt), "constructor_calldata": [hex(&public_key)] });
            serde_json::json!({ "deploy_account_transaction": signed(id, tx, 0, bounds(L2), key) })
        };
        assert_eq!(chain.handle_rpc("starknet_addDeployAccountTransaction", &deploy_tx(&new_key)).unwrap_err().code, ERR_INSUFFICIENT_BALANCE);
        chain.handle_rpc("devnet_mint", &serde_json::json!({ "address": hex(&address), "amount": "1000000000000000000", "unit": "FRI" })).unwrap();
        assert_eq!(chain.handle_rpc("starknet_addDeployAccountTransaction", &deploy_tx(&Felt::from(7_u64))).unwrap_err().code, ERR_VALIDATION_FAILURE);
        let dep = chain.handle_rpc("starknet_addDeployAccountTransaction", &deploy_tx(&new_key)).unwrap();
        assert_eq!(parse_felt_str(dep["contract_address"].as_str().unwrap()).unwrap(), address);
        let receipt = chain.handle_rpc("starknet_getTransactionReceipt", &serde_json::json!({ "transaction_hash": dep["transaction_hash"] })).unwrap();
        assert_eq!(receipt["execution_status"], "SUCCEEDED");
        assert!(strk_balance_of(&mut chain, &hex(&address)) < 1_000_000_000_000_000_000);
        let pk = chain.handle_rpc("starknet_call", &serde_json::json!({ "request": { "contract_address": hex(&address), "entry_point_selector": hex(&selector("get_public_key")), "calldata": [] } })).unwrap();
        assert_eq!(pk, serde_json::json!([hex(&public_key)]));

        // starknet_getBlockWithReceipts returns block header + paired (transaction, receipt) entries
        let bwr = chain.handle_rpc("starknet_getBlockWithReceipts", &serde_json::json!({ "block_id": "latest" })).unwrap();
        assert_eq!(bwr["status"], "ACCEPTED_ON_L2");
        assert_ne!(bwr["new_root"], "0x0", "block new_root commitment must be non-zero: {bwr}");
        assert_ne!(bwr["transaction_commitment"], "0x0", "transaction_commitment must be non-zero: {bwr}");
        assert_ne!(bwr["receipt_commitment"], "0x0", "receipt_commitment must be non-zero: {bwr}");
        assert_ne!(bwr["state_diff_commitment"], "0x0", "state_diff_commitment must be non-zero: {bwr}");
        let txs_with_receipts = bwr["transactions"].as_array().expect("transactions array");
        assert!(!txs_with_receipts.is_empty());
        assert!(txs_with_receipts[0].get("transaction").is_some() && txs_with_receipts[0].get("receipt").is_some());

        // starknet_traceBlockTransactions returns nested inner calls (Account.__execute__ -> Counter.increase)
        let block_traces = chain.handle_rpc("starknet_traceBlockTransactions", &serde_json::json!({ "block_id": { "block_number": 3 } })).unwrap();
        let trace_arr = block_traces.as_array().expect("block_traces array");
        assert!(!trace_arr.is_empty());
        let exec_inv = &trace_arr[0]["trace_root"]["execute_invocation"];
        let inner_calls = exec_inv["calls"].as_array().expect("nested calls array in execute_invocation");
        assert!(!inner_calls.is_empty(), "Account.__execute__ must record nested inner contract calls in trace: {exec_inv}");
        assert_eq!(
            parse_felt_str(inner_calls[0]["contract_address"].as_str().unwrap()).unwrap(),
            expected,
            "inner call target must be the deployed Counter contract: {inner_calls:?}"
        );

        // Full starknet-devnet-rs parity: getAccountBalance, impersonateAccount, setGasPrice,
        // postman*, getMessagesStatus, getStorageProof, dump/load, abortBlocks, restart
        let bal_res = chain
            .handle_rpc(
                "devnet_getAccountBalance",
                &serde_json::json!({ "address": acc.address, "unit": "FRI" }),
            )
            .unwrap();
        assert_eq!(bal_res["unit"], "FRI");
        assert!(bal_res["amount"].as_str().unwrap().parse::<u128>().unwrap() > 0);

        // Impersonate `other` and send an unsigned invoke on its behalf
        let other = chain.accounts[1].clone();
        chain
            .handle_rpc(
                "devnet_impersonateAccount",
                &serde_json::json!({ "account_address": other.address }),
            )
            .unwrap();
        let mut imp_tx = signed_invoke(id, &other, inc.clone(), 0, bounds(L2), None);
        imp_tx["signature"] = serde_json::json!([]);
        let imp_rcpt = invoke(&mut chain, imp_tx.clone()).unwrap();
        assert_eq!(imp_rcpt["execution_status"], "SUCCEEDED");
        chain
            .handle_rpc(
                "devnet_stopImpersonateAccount",
                &serde_json::json!({ "account_address": other.address }),
            )
            .unwrap();

        let gp = chain
            .handle_rpc(
                "devnet_setGasPrice",
                &serde_json::json!({ "gas_price_wei": 200_000_000_000_u64, "gas_price_fri": 250_000_000_000_u64 }),
            )
            .unwrap();
        assert_eq!(gp["gas_price_wei"], 200_000_000_000_u64);
        assert_eq!(gp["gas_price_fri"], 250_000_000_000_u64);

        let pm = chain
            .handle_rpc("devnet_postmanLoad", &serde_json::json!({ "network_url": "http://127.0.0.1:8545" }))
            .unwrap();
        assert!(pm["messaging_contract_address"].as_str().unwrap().starts_with("0x"));
        let l1_msg = chain
            .handle_rpc(
                "devnet_postmanSendMessageToL2",
                &serde_json::json!({
                    "l2_contract_address": deployed,
                    "l1_contract_address": "0x111",
                    "entry_point_selector": hex(&selector("increase")),
                    "payload": ["0x5"],
                    "nonce": "0x0"
                }),
            )
            .unwrap();
        assert!(l1_msg["transaction_hash"].as_str().unwrap().starts_with("0x"));

        let msg_status = chain
            .handle_rpc(
                "starknet_getMessagesStatus",
                &serde_json::json!({ "transaction_hash": l1_msg["transaction_hash"] }),
            )
            .unwrap();
        assert_eq!(msg_status[0]["execution_status"], "SUCCEEDED");

        let sp = chain
            .handle_rpc(
                "starknet_getStorageProof",
                &serde_json::json!({ "block_id": "latest" }),
            )
            .unwrap();
        assert!(sp["global_roots"]["contracts_tree_root"].as_str().unwrap().starts_with("0x"));
        assert!(chain
            .handle_rpc(
                "starknet_getStorageProof",
                &serde_json::json!({ "block_id": "pre_confirmed" }),
            )
            .is_err());

        // Starknet JSON-RPC v0.10.4: specVersion, getCompiledCasm, pre_confirmed tag, and WS subscriptions
        let spec_ver = chain.handle_rpc("starknet_specVersion", &serde_json::json!([])).unwrap();
        assert_eq!(spec_ver, serde_json::json!("0.10.4"));
        let casm_res = chain
            .handle_rpc("starknet_getCompiledCasm", &serde_json::json!({ "class_hash": class_hash }))
            .unwrap();
        assert!(casm_res["bytecode"].as_array().is_some_and(|b| !b.is_empty()));
        let sub_id = chain.handle_rpc("starknet_subscribeNewHeads", &serde_json::json!({})).unwrap();
        assert!(sub_id.as_str().is_some_and(|s| s.starts_with("0x")));
        let unsub = chain.handle_rpc("starknet_unsubscribe", &serde_json::json!({ "subscription_id": sub_id })).unwrap();
        assert_eq!(unsub, serde_json::json!(true));

        // Verify dynamic gas price in block header and block_id validation on state queries
        let latest_blk = chain.handle_rpc("starknet_getBlockWithTxHashes", &serde_json::json!({ "block_id": "latest" })).unwrap();
        assert_eq!(latest_blk["l1_gas_price"]["price_in_fri"], format!("0x{:x}", 250_000_000_000_u64));
        assert_eq!(latest_blk["l1_gas_price"]["price_in_wei"], format!("0x{:x}", 200_000_000_000_u64));
        let bad_blk_err = chain
            .handle_rpc(
                "starknet_getNonce",
                &serde_json::json!({ "block_id": { "block_number": 999_999 }, "contract_address": acc.address }),
            )
            .unwrap_err();
        assert_eq!(bad_blk_err.code, ERR_BLOCK_NOT_FOUND);
    }
}
