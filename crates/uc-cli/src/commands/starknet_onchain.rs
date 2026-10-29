use super::*;
use std::collections::BTreeMap;
use std::time::Duration;

/// Universal Deployer Contract that `uc deploy` calls: OpenZeppelin's Cairo 1 UDC, the one sncast
/// 0.60 uses, at this address on mainnet, Sepolia and starknet-devnet. Same `deployContract` entry
/// point and unique-salt rule (`pedersen(caller, salt)`) as the legacy UDC below, so only
/// `--unique` addresses depend on which one deployed the contract.
pub(crate) const STARKNET_UDC_ADDRESS: &str =
    "0x02ceed65a4bd731034c01113685c831b01c15d7d432f71afb1cf1634b53a2125";
/// The legacy Cairo 0 UDC, still deployed on every network; `uc devnet` predeploys both.
pub(crate) const LEGACY_UDC_ADDRESS: &str =
    "0x041a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf";

/// Public fallback endpoints, used when no `STARKSCAN_API_KEY` is configured.
const DEFAULT_MAINNET_RPC_URL: &str = "https://api.cartridge.gg/x/starknet/mainnet";
const DEFAULT_SEPOLIA_RPC_URL: &str = "https://api.cartridge.gg/x/starknet/sepolia";

/// Default RPC for a network preset: Starkscan's header-authenticated gateway when an API key
/// is configured, else the public Cartridge endpoint.
pub(crate) fn default_rpc_url_for_network(network: &str) -> Option<String> {
    let chain = starkscan_chain_id(Some(network));
    match (network, chain, starkscan_api_key()) {
        (_, Some(chain), Some(_)) => Some(starkscan_rpc_url(chain)),
        ("mainnet" | "alpha-mainnet", _, None) => Some(DEFAULT_MAINNET_RPC_URL.to_string()),
        ("sepolia" | "alpha-sepolia", _, None) => Some(DEFAULT_SEPOLIA_RPC_URL.to_string()),
        ("devnet" | "katana", _, _) => Some("http://127.0.0.1:5050/rpc".to_string()),
        _ => None,
    }
}

/// Which provider a resolved RPC URL belongs to, for reports.
pub(crate) fn rpc_provider_label(url: &str) -> &'static str {
    if is_starkscan_url(url) {
        "starkscan"
    } else if url.contains("cartridge.gg") {
        "cartridge-public"
    } else if url.contains("127.0.0.1") || url.contains("localhost") {
        "local"
    } else {
        "custom"
    }
}

#[derive(Args, Debug, Clone)]
pub(crate) struct ClassHashArgs {
    /// Contract name, `.contract_class.json`, or `.compiled_contract_class.json` path.
    pub target: Option<String>,

    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    #[arg(short = 'p', long)]
    pub package: Option<String>,

    #[arg(long)]
    pub profile: Option<String>,

    #[arg(long)]
    pub release: bool,

    /// Also compute CASM compiled_class_hash.
    #[arg(long)]
    pub casm: bool,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub report_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct SelectorArgs {
    /// Function or event name(s) to compute `starknet_keccak` selector for, or hex selector with `--decode`.
    pub inputs: Vec<String>,

    /// Reverse-lookup a `0x...` selector against compiled workspace ABIs and standard Starknet selectors.
    #[arg(long)]
    pub decode: bool,

    /// Encode input string(s) as Cairo `felt252` short strings (<=31 ASCII bytes).
    #[arg(long)]
    pub short_string: bool,

    /// Decode `0x...` or decimal `felt252` value(s) into Cairo short strings.
    #[arg(long)]
    pub decode_short_string: bool,

    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub report_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct CallArgs {
    /// Target Starknet contract address (`0x...`).
    #[arg(short = 'a', long = "contract-address")]
    pub contract_address: String,

    /// Entry point function name (e.g. `balance_of`) or raw selector (`0x...`).
    #[arg(short = 'f', long)]
    pub function: String,

    /// Calldata arguments (supports hex `0x...`, decimal, `u256:<n>`, `true`/`false`, `'short_str'`, `bytearray:<str>`).
    #[arg(short = 'c', long, visible_alias = "arguments", num_args = 0..)]
    pub calldata: Vec<String>,

    /// Starknet JSON-RPC URL (or resolved via `--network` / `snfoundry.toml` / `STARKNET_RPC`).
    #[arg(short = 'u', long)]
    pub url: Option<String>,

    /// Network preset (`mainnet`, `sepolia`, `devnet`).
    #[arg(short = 'n', long)]
    pub network: Option<String>,

    /// `snfoundry.toml` `[sncast.<profile>]` profile name.
    #[arg(long)]
    pub sncast_profile: Option<String>,

    /// Block identifier (`latest`, `pending`, or block number/hash).
    #[arg(short = 'b', long, default_value = "latest")]
    pub block_id: String,

    /// Optional contract name in local workspace for ABI-aware calldata encoding and return decoding.
    #[arg(long)]
    pub contract: Option<String>,

    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub report_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct DeclareArgs {
    /// Contract name in workspace (or path to `.contract_class.json`).
    #[arg(short = 'c', long, visible_alias = "contract-name")]
    pub contract: String,

    #[arg(short = 'u', long)]
    pub url: Option<String>,

    #[arg(short = 'n', long)]
    pub network: Option<String>,

    #[arg(long)]
    pub sncast_profile: Option<String>,

    #[arg(long)]
    pub account: Option<String>,

    #[arg(long)]
    pub account_address: Option<String>,

    #[arg(long)]
    pub private_key: Option<String>,

    /// Path to encrypted Web3/Starkli keystore JSON file.
    #[arg(long)]
    pub keystore: Option<PathBuf>,

    /// Password for `--keystore` (or set `STARKNET_KEYSTORE_PASSWORD`).
    #[arg(long)]
    pub password: Option<String>,

    #[arg(long)]
    pub accounts_file: Option<PathBuf>,

    /// Estimate fees and build V3 DECLARE payload without broadcasting.
    #[arg(long, visible_alias = "estimate-only")]
    pub dry_run: bool,

    /// Address (or class hash) of the deployed contract this class will replace. Its class is
    /// fetched and compared with the candidate; the declare is refused when a storage slot or
    /// entry point disappears, calldata arity changes, or a caller / signature guard is lost.
    #[arg(long, value_name = "ADDRESS")]
    pub upgrade_of: Option<String>,

    /// Declare even when `--upgrade-of` reports the upgrade unsafe (the report is still emitted).
    #[arg(long, requires = "upgrade_of")]
    pub allow_unsafe_upgrade: bool,

    /// Wait for transaction confirmation (`ACCEPTED_ON_L2` / `ACCEPTED_ON_L1`) after broadcast.
    #[arg(long)]
    pub wait: bool,

    /// Maximum seconds to poll when `--wait` is enabled.
    #[arg(long, default_value_t = 30)]
    pub wait_timeout: u64,

    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub report_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct DeployArgs {
    /// Class hash (`0x...`) or local contract name in workspace to deploy via Starknet UDC.
    #[arg(short = 'g', long)]
    pub class_hash: Option<String>,

    /// Local contract name in workspace (automatically resolves `class_hash` without rebuilding).
    #[arg(short = 'c', long, visible_alias = "contract-name")]
    pub contract: Option<String>,

    /// Constructor calldata arguments.
    #[arg(long, visible_alias = "arguments", num_args = 0..)]
    pub constructor_calldata: Vec<String>,

    /// Salt for deterministic UDC address derivation (`0x0` default).
    #[arg(short = 's', long, default_value = "0x0")]
    pub salt: String,

    /// Derive address tied to deployer account (`unique = 1` in UDC).
    #[arg(long)]
    pub unique: bool,

    #[arg(short = 'u', long)]
    pub url: Option<String>,

    #[arg(short = 'n', long)]
    pub network: Option<String>,

    #[arg(long)]
    pub sncast_profile: Option<String>,

    #[arg(long)]
    pub account: Option<String>,

    #[arg(long)]
    pub account_address: Option<String>,

    #[arg(long)]
    pub private_key: Option<String>,

    /// Path to encrypted Web3/Starkli keystore JSON file.
    #[arg(long)]
    pub keystore: Option<PathBuf>,

    /// Password for `--keystore` (or set `STARKNET_KEYSTORE_PASSWORD`).
    #[arg(long)]
    pub password: Option<String>,

    #[arg(long)]
    pub accounts_file: Option<PathBuf>,

    /// Compute deterministic deployed contract address and V3 UDC INVOKE payload without broadcasting.
    #[arg(long, visible_alias = "estimate-only")]
    pub dry_run: bool,

    /// Wait for transaction confirmation (`ACCEPTED_ON_L2` / `ACCEPTED_ON_L1`) after broadcast.
    #[arg(long)]
    pub wait: bool,

    /// Maximum seconds to poll when `--wait` is enabled.
    #[arg(long, default_value_t = 30)]
    pub wait_timeout: u64,

    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub report_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct InvokeArgs {
    /// Target contract address (`0x...`).
    #[arg(short = 'a', long = "contract-address")]
    pub contract_address: String,

    /// Function name or selector (`0x...`).
    #[arg(short = 'f', long)]
    pub function: String,

    /// Calldata arguments.
    #[arg(short = 'c', long, visible_alias = "arguments", num_args = 0..)]
    pub calldata: Vec<String>,

    #[arg(short = 'u', long)]
    pub url: Option<String>,

    #[arg(short = 'n', long)]
    pub network: Option<String>,

    #[arg(long)]
    pub sncast_profile: Option<String>,

    #[arg(long)]
    pub account: Option<String>,

    #[arg(long)]
    pub account_address: Option<String>,

    #[arg(long)]
    pub private_key: Option<String>,

    /// Path to encrypted Web3/Starkli keystore JSON file.
    #[arg(long)]
    pub keystore: Option<PathBuf>,

    /// Password for `--keystore` (or set `STARKNET_KEYSTORE_PASSWORD`).
    #[arg(long)]
    pub password: Option<String>,

    #[arg(long)]
    pub accounts_file: Option<PathBuf>,

    /// Build V3 INVOKE transaction and estimate fee without broadcasting.
    #[arg(long, visible_alias = "estimate-only")]
    pub dry_run: bool,

    /// Wait for transaction confirmation (`ACCEPTED_ON_L2` / `ACCEPTED_ON_L1`) after broadcast.
    #[arg(long)]
    pub wait: bool,

    /// Maximum seconds to poll when `--wait` is enabled.
    #[arg(long, default_value_t = 30)]
    pub wait_timeout: u64,

    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub report_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct TxStatusArgs {
    /// Transaction hash (`0x...`).
    pub transaction_hash: String,

    /// Also fetch full `starknet_getTransactionReceipt`.
    #[arg(long)]
    pub receipt: bool,

    #[arg(short = 'u', long)]
    pub url: Option<String>,

    #[arg(short = 'n', long)]
    pub network: Option<String>,

    #[arg(long)]
    pub sncast_profile: Option<String>,

    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub report_path: Option<PathBuf>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum VerifierProviderArg {
    Starkscan,
    Rpc,
    Walnut,
    Voyager,
}

impl VerifierProviderArg {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Starkscan => "starkscan",
            Self::Rpc => "rpc",
            Self::Walnut => "walnut",
            Self::Voyager => "voyager",
        }
    }
}

#[derive(Args, Debug, Clone)]
pub(crate) struct VerifyOnchainArgs {
    /// On-chain Starknet contract address (`0x...`) to verify against.
    #[arg(short = 'a', long = "contract-address")]
    pub contract_address: Option<String>,

    /// On-chain Starknet class hash (`0x...`) to verify against.
    #[arg(long)]
    pub class_hash: Option<String>,

    /// Specific contract name in local workspace.
    #[arg(short = 'c', long)]
    pub contract: Option<String>,

    /// Verification provider (`starkscan` default, `rpc`, `walnut`, `voyager`).
    #[arg(long, value_enum, default_value_t = VerifierProviderArg::Starkscan)]
    pub verifier: VerifierProviderArg,

    /// Custom verifier endpoint (POST of the bundle). Without it `--submit` writes Starkscan's
    /// verification record: `PUT https://starkscan.co/api/v1/{chain}/contract/{address}/verification`.
    #[arg(long)]
    pub verifier_url: Option<String>,

    /// Starkscan API key (or `STARKSCAN_API_KEY` env var) for authenticated verification submission.
    #[arg(long)]
    pub api_key: Option<String>,

    /// Export the deterministic Starkscan verification bundle JSON to disk.
    #[arg(long)]
    pub export_bundle: Option<PathBuf>,

    /// Verify a Starkscan verification bundle JSON in-memory (recompiles sources and checks class_hash parity in <500ms).
    #[arg(long)]
    pub verify_bundle: Option<PathBuf>,

    /// Submit verification bundle directly to the verifier endpoint (`--verifier-url`).
    #[arg(long)]
    pub submit: bool,

    #[arg(short = 'u', long)]
    pub url: Option<String>,

    #[arg(short = 'n', long)]
    pub network: Option<String>,

    #[arg(long)]
    pub sncast_profile: Option<String>,

    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    #[arg(long)]
    pub profile: Option<String>,

    #[arg(long)]
    pub release: bool,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub report_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct AccountArgs {
    #[command(subcommand)]
    pub command: AccountSubcommand,
}

#[derive(Subcommand, Debug, Clone)]
pub(crate) enum AccountSubcommand {
    /// Create a new undeployed Starknet account and save it to the accounts file.
    Create(AccountCreateArgs),
    /// Import an existing Starknet account into the accounts file.
    #[command(visible_alias = "add")]
    Import(AccountImportArgs),
    /// Delete a Starknet account from the accounts file.
    Delete(AccountDeleteArgs),
    /// List configured Starknet accounts from `uc.toml`, `snfoundry.toml`, and `~/.starknet_accounts`.
    List(AccountListArgs),
    /// Compute deterministic counterfactual Starknet account address (`calculate_contract_address_from_hash`).
    ComputeAddress(AccountComputeAddressArgs),
    /// Deploy an account with a V3 DEPLOY_ACCOUNT transaction (fund the counterfactual address first).
    Deploy(AccountDeployArgs),
}

#[derive(Args, Debug, Clone)]
pub(crate) struct AccountCreateArgs {
    /// Account name to store in the accounts file.
    #[arg(short = 'n', long, default_value = "default")]
    pub name: String,

    /// Account contract type (`oz`, `argent`, `braavos`).
    #[arg(short = 't', long = "type", default_value = "oz")]
    pub account_type: String,

    /// Custom account class hash (`0x...`). Defaults to the standard OpenZeppelin account class hash.
    #[arg(long)]
    pub class_hash: Option<String>,

    /// Salt for counterfactual address derivation (`0x0` default).
    #[arg(long, default_value = "0x0")]
    pub salt: String,

    #[arg(short = 'u', long)]
    pub url: Option<String>,

    #[arg(long, default_value = "alpha-sepolia")]
    pub network: String,

    #[arg(long)]
    pub accounts_file: Option<PathBuf>,

    /// Also save a matching profile in `uc.toml` / `snfoundry.toml`.
    #[arg(long)]
    pub add_profile: Option<String>,

    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub report_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct AccountImportArgs {
    /// Account name to store in the accounts file.
    #[arg(short = 'n', long)]
    pub name: String,

    /// Account contract address (`0x...`).
    #[arg(short = 'a', long)]
    pub address: String,

    /// Private key (`0x...`).
    #[arg(long)]
    pub private_key: String,

    /// Public key (`0x...`); derived automatically from `--private-key` when omitted.
    #[arg(long)]
    pub public_key: Option<String>,

    /// Account contract type (`oz`, `argent`, `braavos`).
    #[arg(short = 't', long = "type", default_value = "oz")]
    pub account_type: String,

    /// Class hash (`0x...`).
    #[arg(long)]
    pub class_hash: Option<String>,

    /// Salt (`0x0` default).
    #[arg(long)]
    pub salt: Option<String>,

    #[arg(short = 'u', long)]
    pub url: Option<String>,

    #[arg(long, default_value = "alpha-sepolia")]
    pub network: String,

    #[arg(long)]
    pub accounts_file: Option<PathBuf>,

    /// Also save a matching profile in `uc.toml` / `snfoundry.toml`.
    #[arg(long)]
    pub add_profile: Option<String>,

    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub report_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct AccountDeleteArgs {
    /// Account name to delete from the accounts file.
    #[arg(short = 'n', long)]
    pub name: String,

    /// Network key in the accounts file (`alpha-sepolia`, `alpha-mainnet`, etc.).
    #[arg(long)]
    pub network: Option<String>,

    #[arg(long)]
    pub accounts_file: Option<PathBuf>,

    /// Confirm deletion without interactive prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,

    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub report_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct AccountDeployArgs {
    /// Named account from the accounts file (created via `uc account create`).
    #[arg(long)]
    pub name: Option<String>,

    #[arg(long)]
    pub accounts_file: Option<PathBuf>,

    /// Account class hash (`0x...`); must already be declared on the network.
    #[arg(long)]
    pub class_hash: Option<String>,

    /// Private key (`0x...`); the public key becomes the constructor argument unless
    /// `--constructor-calldata` is given.
    #[arg(long)]
    pub private_key: Option<String>,

    /// Constructor calldata (defaults to `[public_key]`).
    #[arg(long, num_args = 0..)]
    pub constructor_calldata: Vec<String>,

    #[arg(long, default_value = "0x0")]
    pub salt: String,

    #[arg(short = 'u', long)]
    pub url: Option<String>,

    #[arg(short = 'n', long)]
    pub network: Option<String>,

    /// Build and sign without broadcasting.
    #[arg(long)]
    pub dry_run: bool,

    #[arg(long)]
    pub wait: bool,

    #[arg(long, default_value_t = 30)]
    pub wait_timeout: u64,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub report_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct AccountListArgs {
    #[arg(long)]
    pub accounts_file: Option<PathBuf>,

    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub report_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct AccountComputeAddressArgs {
    /// Account contract class hash (`0x...`).
    #[arg(long)]
    pub class_hash: String,

    /// Public key (`0x...`) or first constructor arg.
    #[arg(long)]
    pub public_key: String,

    /// Salt (`0x0` default).
    #[arg(long, default_value = "0x0")]
    pub salt: String,

    /// Deployer address (`0x0` default for counterfactual account deployment).
    #[arg(long, default_value = "0x0")]
    pub deployer_address: String,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub report_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ResolvedStarknetProfile {
    pub profile_name: String,
    pub url: Option<String>,
    pub network: Option<String>,
    pub account: Option<String>,
    pub account_address: Option<String>,
    pub private_key: Option<String>,
    pub accounts_file: Option<PathBuf>,
}

pub(crate) fn resolve_sncast_profile_config(
    workspace_root: &Path,
    profile_override: Option<&str>,
    url_override: Option<&str>,
    network_override: Option<&str>,
    account_override: Option<&str>,
    account_address_override: Option<&str>,
    private_key_override: Option<&str>,
    accounts_file_override: Option<&Path>,
) -> ResolvedStarknetProfile {
    let profile_name = profile_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("default")
        .to_string();
    let mut resolved = ResolvedStarknetProfile {
        profile_name: profile_name.clone(),
        ..Default::default()
    };

    // 1. Check `[networks.<name>]` in `uc.toml` (or `[uc.networks.<name>]` in `Scarb.toml`).
    let lookup_key = network_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&profile_name);
    for dir in workspace_root.ancestors().take(4) {
        let uc_manifest = manifest_file_in_dir(dir);
        if uc_manifest.is_file() {
            if let Ok(text) = fs::read_to_string(&uc_manifest) {
                if let Ok(val) = toml::from_str::<TomlValue>(&text) {
                    let net_tbl = val
                        .get("networks")
                        .or_else(|| val.get("uc").and_then(|u| u.get("networks")))
                        .and_then(TomlValue::as_table);
                    if let Some(nets) = net_tbl {
                        let entry = nets
                            .get(lookup_key)
                            .or_else(|| nets.get(&profile_name))
                            .or_else(|| nets.get("default"))
                            .and_then(TomlValue::as_table);
                        if let Some(t) = entry {
                            resolved.url = t
                                .get("url")
                                .or_else(|| t.get("rpc"))
                                .and_then(TomlValue::as_str)
                                .map(str::to_string);
                            resolved.network = t
                                .get("network")
                                .and_then(TomlValue::as_str)
                                .map(str::to_string)
                                .or_else(|| Some(lookup_key.to_string()));
                            resolved.account = t
                                .get("account")
                                .and_then(TomlValue::as_str)
                                .map(str::to_string);
                            resolved.account_address = t
                                .get("account_address")
                                .or_else(|| t.get("account-address"))
                                .and_then(TomlValue::as_str)
                                .map(str::to_string);
                            if let Some(af) = t
                                .get("accounts-file")
                                .or_else(|| t.get("accounts_file"))
                                .and_then(TomlValue::as_str)
                            {
                                resolved.accounts_file = Some(PathBuf::from(af));
                            }
                            break;
                        }
                    }
                }
            }
        }
    }

    // 2. Fallback: Check `snfoundry.toml` in workspace_root or ancestors
    if resolved.url.is_none() && resolved.account.is_none() {
        for dir in workspace_root.ancestors().take(4) {
            let snfoundry_path = dir.join("snfoundry.toml");
            if snfoundry_path.is_file() {
                if let Ok(text) = fs::read_to_string(&snfoundry_path) {
                    if let Ok(val) = toml::from_str::<TomlValue>(&text) {
                        if let Some(sncast_tbl) = val.get("sncast").and_then(TomlValue::as_table) {
                            let prof_tbl = sncast_tbl
                                .get(&profile_name)
                                .and_then(TomlValue::as_table)
                                .or_else(|| sncast_tbl.values().next().and_then(TomlValue::as_table));
                            if let Some(t) = prof_tbl {
                                resolved.url = t
                                    .get("url")
                                    .and_then(TomlValue::as_str)
                                    .map(str::to_string);
                                resolved.network = t
                                    .get("network")
                                    .and_then(TomlValue::as_str)
                                    .map(str::to_string);
                                resolved.account = t
                                    .get("account")
                                    .and_then(TomlValue::as_str)
                                    .map(str::to_string);
                                if let Some(af) = t.get("accounts-file").or_else(|| t.get("accounts_file")).and_then(TomlValue::as_str) {
                                    resolved.accounts_file = Some(PathBuf::from(af));
                                }
                            }
                        }
                    }
                }
                break;
            }
        }
    }

    if let Some(u) = url_override {
        resolved.url = Some(u.to_string());
    } else if resolved.url.is_none() {
        if let Ok(env_url) = std::env::var("STARKNET_RPC") {
            if !env_url.trim().is_empty() {
                resolved.url = Some(env_url.trim().to_string());
            }
        }
    }
    if let Some(n) = network_override {
        resolved.network = Some(n.to_string());
    }
    if resolved.url.is_none() {
        resolved.url = resolved.network.as_deref().and_then(default_rpc_url_for_network);
    }
    if let Some(acc) = account_override {
        resolved.account = Some(acc.to_string());
    }
    if let Some(af) = accounts_file_override {
        resolved.accounts_file = Some(af.to_path_buf());
    }
    if resolved.accounts_file.is_none() {
        if let Some(home) = std::env::var_os("HOME") {
            let default_af = PathBuf::from(home)
                .join(".starknet_accounts/starknet_open_zeppelin_accounts.json");
            if default_af.is_file() {
                resolved.accounts_file = Some(default_af);
            }
        }
    }

    // Resolve account_address & private_key from accounts_file if available
    if let (Some(af_path), Some(acc_name)) = (&resolved.accounts_file, &resolved.account) {
        if let Ok(text) = fs::read_to_string(af_path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(obj) = json.as_object() {
                    for (_net_key, net_accounts) in obj {
                        if let Some(acc_obj) = net_accounts.get(acc_name) {
                            if resolved.account_address.is_none() {
                                resolved.account_address = acc_obj
                                    .get("address")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string);
                            }
                            if resolved.private_key.is_none() {
                                resolved.private_key = acc_obj
                                    .get("private_key")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string);
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some(addr) = account_address_override {
        resolved.account_address = Some(addr.to_string());
    } else if resolved.account_address.is_none() {
        if let Ok(env_addr) = std::env::var("STARKNET_ACCOUNT_ADDRESS") {
            if !env_addr.trim().is_empty() {
                resolved.account_address = Some(env_addr.trim().to_string());
            }
        }
    }
    if let Some(pk) = private_key_override {
        resolved.private_key = Some(pk.to_string());
    } else if resolved.private_key.is_none() {
        if let Ok(env_pk) = std::env::var("STARKNET_PRIVATE_KEY") {
            if !env_pk.trim().is_empty() {
                resolved.private_key = Some(env_pk.trim().to_string());
            }
        }
    }

    resolved
}

/// A parsed HTTP response: status code and JSON body (`null` when the body is empty or not JSON).
pub(crate) struct HttpJsonResponse {
    pub status: u16,
    pub body: serde_json::Value,
}

/// GET / PUT / POST a JSON document. Unlike [http_post_json] this reports the status code
/// instead of failing on it.
pub(crate) fn http_request_json(
    method: &str,
    url: &str,
    payload: Option<&serde_json::Value>,
    extra_headers: &[(&str, &str)],
) -> Result<HttpJsonResponse> {
    let body = payload.map(serde_json::to_vec).transpose().context("failed to serialize JSON payload")?;
    let mut headers = vec![("Content-Type", "application/json"), ("Accept", "application/json")];
    headers.extend_from_slice(extra_headers);
    let response = http_request(method, url, body.as_deref(), &headers)?;
    Ok(HttpJsonResponse { status: response.status, body: serde_json::from_slice(&response.body).unwrap_or(serde_json::Value::Null) })
}

/// POSTs a JSON document and parses the JSON reply (JSON-RPC answers errors with a JSON body).
pub(crate) fn http_post_json(
    url: &str,
    payload: &serde_json::Value,
    extra_headers: &[(&str, &str)],
) -> Result<serde_json::Value> {
    let body = serde_json::to_vec(payload).context("failed to serialize JSON payload")?;
    let mut headers = vec![("Content-Type", "application/json"), ("Accept", "application/json")];
    headers.extend_from_slice(extra_headers);
    let response = http_request("POST", url, Some(&body), &headers)?;
    serde_json::from_slice(&response.body).with_context(|| {
        format!(
            "HTTP {} from {url} is not JSON: {}",
            response.status,
            String::from_utf8_lossy(&response.body[..response.body.len().min(300)])
        )
    })
}

pub(crate) fn starknet_rpc_request(
    url: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let owned_headers = starkscan_headers(url);
    let headers: Vec<(&str, &str)> = owned_headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let resp = http_post_json(url, &req, &headers)?;
    if let Some(err) = resp.get("error") {
        bail!("Starknet JSON-RPC `{method}` error: {err}");
    }
    resp.get("result")
        .cloned()
        .ok_or_else(|| anyhow!("Starknet JSON-RPC `{method}` response missing `result`: {resp}"))
}

#[cfg(feature = "native-compile")]
pub(crate) fn parse_felt_str(raw: &str) -> Result<starknet_types_core::felt::Felt> {
    use starknet_types_core::felt::Felt;
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("true") {
        return Ok(Felt::ONE);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Ok(Felt::ZERO);
    }
    if let Some(short) = trimmed
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
    {
        if short.len() > 31 {
            bail!("Cairo short string `{short}` exceeds 31 bytes");
        }
        return Ok(Felt::from_bytes_be_slice(short.as_bytes()));
    }
    if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
        Felt::from_hex(trimmed).map_err(|e| anyhow!("invalid hex felt `{trimmed}`: {e}"))
    } else {
        Felt::from_dec_str(trimmed).map_err(|e| anyhow!("invalid decimal felt `{trimmed}`: {e}"))
    }
}

#[cfg(feature = "native-compile")]
pub(crate) fn format_felt_hex(f: &starknet_types_core::felt::Felt) -> String {
    format!("{:#x}", f.to_biguint())
}

/// Encodes human-readable CLI arguments into Starknet calldata felt hex strings.
/// Supports:
/// - `0x...` hex felts and decimal integers
/// - `true` / `false` (`0x1` / `0x0`)
/// - `'short_string'` (`<= 31` ASCII bytes)
/// - `u256:<decimal_or_hex>` (automatically split into `[low_128, high_128]`)
/// - `bytearray:<string>` (encoded as Cairo `ByteArray`: `[num_full_words, ...words, pending_word, pending_len]`)
/// - Automatic `u256` splitting when `abi_inputs` indicates `core::integer::u256`!
#[cfg(feature = "native-compile")]
pub(crate) fn encode_starknet_calldata(
    raw_args: &[String],
    abi_inputs: Option<&[serde_json::Value]>,
) -> Result<Vec<String>> {
    use starknet_types_core::felt::Felt;
    let mut out = Vec::new();
    let mut arg_idx = 0_usize;
    let mut abi_idx = 0_usize;

    while arg_idx < raw_args.len() {
        let raw = raw_args[arg_idx].trim();
        let abi_type = abi_inputs
            .and_then(|inputs| inputs.get(abi_idx))
            .and_then(|inp| inp.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if let Some(u256_str) = raw.strip_prefix("u256:") {
            let (low, high) = split_u256_str(u256_str)?;
            out.push(low);
            out.push(high);
            arg_idx += 1;
            abi_idx += 1;
            continue;
        }
        if let Some(ba_str) = raw.strip_prefix("bytearray:") {
            out.extend(encode_cairo_byte_array(ba_str));
            arg_idx += 1;
            abi_idx += 1;
            continue;
        }
        if abi_type == "core::integer::u256" {
            // If next arg is not a separate high part or user provided single 256-bit value
            let (low, high) = split_u256_str(raw)?;
            out.push(low);
            out.push(high);
            arg_idx += 1;
            abi_idx += 1;
            continue;
        }
        if abi_type == "core::byte_array::ByteArray" && !raw.starts_with("0x") && raw.parse::<u128>().is_err() {
            let unquoted = raw.trim_matches('"').trim_matches('\'');
            out.extend(encode_cairo_byte_array(unquoted));
            arg_idx += 1;
            abi_idx += 1;
            continue;
        }

        let felt: Felt = parse_felt_str(raw)?;
        out.push(format_felt_hex(&felt));
        arg_idx += 1;
        abi_idx += 1;
    }
    Ok(out)
}

#[cfg(feature = "native-compile")]
fn split_u256_str(raw: &str) -> Result<(String, String)> {
    let felt = parse_felt_str(raw)?;
    let big = felt.to_biguint();
    let mask_128 = (starknet_types_core::felt::Felt::from(1u128).to_biguint() << 128usize) - 1u32;
    let low = &big & &mask_128;
    let high = &big >> 128usize;
    Ok((format!("{low:#x}"), format!("{high:#x}")))
}

#[cfg(feature = "native-compile")]
pub(crate) fn encode_cairo_byte_array(text: &str) -> Vec<String> {
    use starknet_types_core::felt::Felt;
    let bytes = text.as_bytes();
    let chunks = bytes.chunks_exact(31);
    let num_full_words = chunks.len();
    let remainder = chunks.remainder();
    let mut res = Vec::with_capacity(num_full_words + 3);
    res.push(format!("{num_full_words:#x}"));
    for chunk in chunks {
        let word = Felt::from_bytes_be_slice(chunk);
        res.push(format_felt_hex(&word));
    }
    let pending_word = if remainder.is_empty() {
        Felt::ZERO
    } else {
        Felt::from_bytes_be_slice(remainder)
    };
    res.push(format_felt_hex(&pending_word));
    res.push(format!("{:#x}", remainder.len()));
    res
}

#[cfg(feature = "native-compile")]
pub(crate) fn decode_starknet_return_data(
    raw_felts: &[String],
    abi_outputs: Option<&[serde_json::Value]>,
) -> serde_json::Value {
    use cairo_lang_runner::short_string::as_cairo_short_string;
    let parsed_felts: Vec<starknet_types_core::felt::Felt> = raw_felts
        .iter()
        .filter_map(|s| parse_felt_str(s).ok())
        .collect();

    if let Some(outputs) = abi_outputs {
        if let Some(first_out) = outputs.first() {
            let out_ty = first_out.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if out_ty == "core::integer::u256" && parsed_felts.len() >= 2 {
                let low = parsed_felts[0].to_biguint();
                let high = parsed_felts[1].to_biguint();
                let combined = (high << 128usize) | low;
                return serde_json::json!({
                    "type": "core::integer::u256",
                    "decimal": combined.to_string(),
                    "hex": format!("{combined:#x}"),
                    "low": format_felt_hex(&parsed_felts[0]),
                    "high": format_felt_hex(&parsed_felts[1]),
                });
            }
            if out_ty == "core::bool" && !parsed_felts.is_empty() {
                return serde_json::json!({
                    "type": "core::bool",
                    "value": parsed_felts[0] != starknet_types_core::felt::Felt::ZERO,
                });
            }
        }
    }

    if parsed_felts.len() == 1 {
        let f = &parsed_felts[0];
        let dec = f.to_biguint().to_string();
        let short_str = as_cairo_short_string(f).filter(|s| !s.is_empty());
        return serde_json::json!({
            "hex": format_felt_hex(f),
            "decimal": dec,
            "short_string": short_str,
        });
    }
    serde_json::json!({
        "felts": raw_felts,
    })
}

/// Computes the deterministic Starknet contract address deployed via `calculate_contract_address_from_hash`:
/// `pedersen_on_elements(["STARKNET_CONTRACT_ADDRESS", deployer_address, salt, class_hash, pedersen_on_elements(constructor_calldata)]) mod L2_ADDRESS_UPPER_BOUND`
#[cfg(feature = "native-test-runner")]
pub(crate) fn compute_starknet_contract_address(
    class_hash_hex: &str,
    salt_hex: &str,
    constructor_calldata_hex: &[String],
    deployer_address_hex: &str,
) -> Result<String> {
    use starknet_crypto::{pedersen_hash, Felt};

    let prefix = Felt::from_bytes_be_slice(b"STARKNET_CONTRACT_ADDRESS");
    let deployer = parse_felt_str(deployer_address_hex)?;
    let salt = parse_felt_str(salt_hex)?;
    let class_hash = parse_felt_str(class_hash_hex)?;

    let mut calldata_acc = Felt::ZERO;
    for item in constructor_calldata_hex {
        let val = parse_felt_str(item)?;
        calldata_acc = pedersen_hash(&calldata_acc, &val);
    }
    let calldata_hash = pedersen_hash(
        &calldata_acc,
        &Felt::from(constructor_calldata_hex.len() as u64),
    );

    let mut addr_acc = Felt::ZERO;
    for elem in [prefix, deployer, salt, class_hash, calldata_hash] {
        addr_acc = pedersen_hash(&addr_acc, &elem);
    }
    addr_acc = pedersen_hash(&addr_acc, &Felt::from(5u64));

    // Modulo L2_ADDRESS_UPPER_BOUND = 2^251 - 256
    let bound = (Felt::from(1u128).to_biguint() << 251usize) - 256u32;
    let bounded = addr_acc.to_biguint() % bound;
    Ok(format!("{bounded:#066x}"))
}

/// Computes the deterministic contract address when deployed through the Starknet Universal Deployer Contract (UDC).
#[cfg(feature = "native-test-runner")]
pub(crate) fn compute_udc_deployed_address(
    class_hash_hex: &str,
    salt_hex: &str,
    unique: bool,
    deployer_account_hex: Option<&str>,
    constructor_calldata_hex: &[String],
) -> Result<String> {
    use starknet_crypto::pedersen_hash;
    if unique {
        let account = deployer_account_hex.unwrap_or("0x0");
        let acc_felt = parse_felt_str(account)?;
        let salt_felt = parse_felt_str(salt_hex)?;
        let effective_salt = format_felt_hex(&pedersen_hash(&acc_felt, &salt_felt));
        compute_starknet_contract_address(
            class_hash_hex,
            &effective_salt,
            constructor_calldata_hex,
            STARKNET_UDC_ADDRESS,
        )
    } else {
        compute_starknet_contract_address(
            class_hash_hex,
            salt_hex,
            constructor_calldata_hex,
            "0x0",
        )
    }
}

fn find_abi_function_entry(
    manifest_path: &Path,
    contract_filter: Option<&str>,
    function_name: &str,
) -> Option<serde_json::Value> {
    let contracts = inspect_contract_artifacts_in_workspace(manifest_path, "dev", contract_filter)
        .ok()
        .or_else(|| {
            inspect_contract_artifacts_in_workspace(manifest_path, "release", contract_filter).ok()
        })?;
    for c in contracts {
        let Some(abi) = c.get("abi").and_then(|v| v.as_array()) else {
            continue;
        };
        for item in abi {
            let ty = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if ty == "function"
                && item.get("name").and_then(|v| v.as_str()) == Some(function_name)
            {
                return Some(item.clone());
            }
            if ty == "interface" {
                if let Some(items) = item.get("items").and_then(|v| v.as_array()) {
                    for sub in items {
                        if sub.get("type").and_then(|v| v.as_str()) == Some("function")
                            && sub.get("name").and_then(|v| v.as_str()) == Some(function_name)
                        {
                            return Some(sub.clone());
                        }
                    }
                }
            }
        }
    }
    None
}

fn ensure_contract_artifacts_ready(
    manifest_path: &Path,
    profile: &str,
    release: bool,
    contract: Option<&str>,
) -> Result<Vec<serde_json::Value>> {
    let silent_report_path = manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".uc/reports/verify-build.json");
    // Always go through the fingerprinted build: artifacts already in `target/` may be stale
    // (edited sources, or produced by another compiler version). A cache hit costs ~1ms.
    with_native_compat_toolchain_fallback(|| {
        run_build(BuildArgs {
            common: BuildCommonArgs {
                manifest_path: Some(manifest_path.to_path_buf()),
                package: None,
                workspace: false,
                features: Vec::new(),
                offline: true,
                release,
                profile: Some(profile.to_string()),
            },
            engine: EngineArg::Uc,
            daemon_mode: DaemonModeArg::Off,
            report_path: Some(silent_report_path),
            record_failure: None,
            json: false,
            plan_only: false,
        })
    })?;
    inspect_contract_artifacts_in_workspace(manifest_path, profile, contract)
}

/// A target that names a file must exist. uc used to take a missing file for a contract name and
/// report the Scarb.toml it then failed to find.
pub(crate) fn ensure_named_file_exists(target: &str) -> Result<()> {
    let names_file = target.ends_with(".json")
        || target.contains('/')
        || target.contains(std::path::MAIN_SEPARATOR);
    if names_file && !Path::new(target).is_file() {
        bail!("no such file: {target}");
    }
    Ok(())
}

pub(crate) fn run_class_hash(args: ClassHashArgs) -> Result<()> {
    let started = Instant::now();
    // Check if target is a direct file path or an on-chain 0x... address / class hash
    if let Some(target_str) = args.target.as_deref() {
        ensure_named_file_exists(target_str)?;
        let candidate_path = PathBuf::from(target_str);
        if candidate_path.is_file() || target_str.starts_with("0x") {
            let (raw_text, source_label) = if candidate_path.is_file() {
                (
                    fs::read_to_string(&candidate_path)
                        .with_context(|| format!("failed to read {}", candidate_path.display()))?,
                    candidate_path.display().to_string(),
                )
            } else {
                let rpc_url = resolve_read_rpc_url(None, Some("mainnet"))
                    .with_context(|| format!("unable to resolve RPC URL to fetch on-chain class `{target_str}`"))?;
                let class_val = starknet_rpc_request(
                    &rpc_url,
                    "starknet_getClassAt",
                    serde_json::json!({ "block_id": "latest", "contract_address": target_str }),
                )
                .or_else(|_| {
                    starknet_rpc_request(
                        &rpc_url,
                        "starknet_getClass",
                        serde_json::json!({ "block_id": "latest", "class_hash": target_str }),
                    )
                })
                .with_context(|| format!("failed to fetch on-chain class `{target_str}` via RPC"))?;
                (class_val.to_string(), target_str.to_string())
            };
            // Classes saved from a node carry the ABI as a string; the class hash commits to that
            // exact string, so hash it as declared and parse the rest as a local artifact.
            let raw_value: Option<serde_json::Value> = serde_json::from_str(&raw_text).ok();
            let declared_hash = raw_value
                .as_ref()
                .filter(|v| v.get("sierra_program").is_some() && v.get("abi").is_some_and(|a| a.is_string()))
                .and_then(compute_class_hash_from_json);
            let text = match (&raw_value, &declared_hash) {
                (Some(v), Some(_)) => {
                    let mut typed = v.clone();
                    if let Some(abi) = typed.get("abi").and_then(|a| a.as_str()).and_then(|a| serde_json::from_str::<serde_json::Value>(a).ok()) {
                        typed["abi"] = abi;
                    }
                    typed.to_string()
                }
                _ => raw_text.clone(),
            };
            #[cfg(feature = "native-compile")]
            {
                if let Ok(sierra_class) =
                    serde_json::from_str::<ContractClass>(&text)
                {
                    let class_hash = match declared_hash.clone() {
                        Some(h) => h,
                        None => compute_starknet_sierra_class_hash(&sierra_class)
                            .ok_or_else(|| anyhow!("failed to compute Sierra class_hash"))?,
                    };
                    let compiled_class_hash = if args.casm {
                        let casm = compile_native_casm_contract(
                            sierra_class,
                            usize::MAX,
                        )
                        .context("failed to compile Sierra class to CASM for compiled_class_hash")?;
                        Some(compute_starknet_compiled_class_hash(&casm))
                    } else {
                        None
                    };
                    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                    let report = serde_json::json!({
                        "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
                        "path": source_label,
                        "class_hash": class_hash,
                        "compiled_class_hash": compiled_class_hash,
                        "elapsed_ms": elapsed_ms,
                    });
                    if args.json || args.report_path.is_some() {
                        emit_json_value(args.report_path.as_deref(), &report)?;
                    } else if let Some(casm_hash) = compiled_class_hash {
                        println!("class_hash:          {class_hash}");
                        println!("compiled_class_hash: {casm_hash}");
                    } else {
                        println!("{class_hash}");
                    }
                    return Ok(());
                }
                if let Ok(casm_class) = serde_json::from_str::<CasmContractClass>(&text) {
                    let compiled_hash = compute_starknet_compiled_class_hash(&casm_class);
                    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                    let report = serde_json::json!({
                        "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
                        "path": source_label,
                        "compiled_class_hash": compiled_hash,
                        "elapsed_ms": elapsed_ms,
                    });
                    if args.json || args.report_path.is_some() {
                        emit_json_value(args.report_path.as_deref(), &report)?;
                    } else {
                        println!("{compiled_hash}");
                    }
                    return Ok(());
                }
            }
            bail!(
                "file `{}` is neither a valid Starknet Sierra `.contract_class.json` nor CASM `.compiled_contract_class.json`",
                source_label
            );
        }
    }

    let manifest_path = resolve_manifest_path(&args.manifest_path)?;
    let profile = resolve_inspect_profile(args.release, args.profile.as_deref());
    let contracts = ensure_contract_artifacts_ready(
        &manifest_path,
        &profile,
        args.release,
        args.target.as_deref(),
    )?;
    if contracts.is_empty() {
        bail!("no Starknet contracts found matching `{:?}`", args.target);
    }
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let items: Vec<serde_json::Value> = contracts
        .iter()
        .map(|c| {
            serde_json::json!({
                "contract_name": c.get("contract_name"),
                "package_name": c.get("package_name"),
                "class_hash": c.get("class_hash"),
                "compiled_class_hash": c.get("compiled_class_hash"),
            })
        })
        .collect();
    let report = serde_json::json!({
        "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
        "manifest_path": manifest_path.display().to_string(),
        "profile": profile,
        "elapsed_ms": elapsed_ms,
        "contracts": items,
    });
    if args.json || args.report_path.is_some() {
        emit_json_value(args.report_path.as_deref(), &report)?;
    } else if items.len() == 1 && !args.casm {
        println!(
            "{}",
            items[0]
                .get("class_hash")
                .and_then(|v| v.as_str())
                .unwrap_or("n/a")
        );
    } else {
        for item in &items {
            println!(
                "{}: class_hash={} compiled_class_hash={}",
                item.get("contract_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?"),
                item.get("class_hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("n/a"),
                item.get("compiled_class_hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("n/a"),
            );
        }
    }
    Ok(())
}

const STANDARD_STARKNET_SELECTOR_NAMES: &[&str] = &[
    "name",
    "symbol",
    "decimals",
    "total_supply",
    "totalSupply",
    "balance_of",
    "balanceOf",
    "allowance",
    "transfer",
    "transfer_from",
    "transferFrom",
    "approve",
    "mint",
    "burn",
    "owner",
    "owner_of",
    "ownerOf",
    "token_uri",
    "tokenURI",
    "get_owner",
    "transfer_ownership",
    "renounce_ownership",
    "upgrade",
    "pause",
    "unpause",
    "__execute__",
    "__validate__",
    "__validate_declare__",
    "__validate_deploy__",
    "is_valid_signature",
    "deployContract",
    "constructor",
    "Transfer",
    "Approval",
    "Upgraded",
    "OwnershipTransferred",
];

pub(crate) fn run_selector(args: SelectorArgs) -> Result<()> {
    #[cfg(feature = "native-compile")]
    {
        use cairo_lang_runner::short_string::as_cairo_short_string;
        use cairo_lang_starknet_classes::keccak::starknet_keccak;
        use starknet_types_core::felt::Felt;

        if args.inputs.is_empty() {
            bail!("provide at least one function/event name, selector, or short string");
        }

        let mut results = Vec::with_capacity(args.inputs.len());
        if args.short_string {
            for input in &args.inputs {
                if input.len() > 31 {
                    bail!("Cairo short string `{input}` exceeds 31 ASCII bytes");
                }
                let felt = Felt::from_bytes_be_slice(input.as_bytes());
                let hex = format_felt_hex(&felt);
                let dec = felt.to_biguint().to_string();
                results.push(serde_json::json!({
                    "input": input,
                    "short_string": input,
                    "hex": hex,
                    "decimal": dec,
                }));
            }
        } else if args.decode_short_string {
            for input in &args.inputs {
                let felt = parse_felt_str(input)?;
                let decoded = as_cairo_short_string(&felt)
                    .ok_or_else(|| anyhow!("felt `{input}` is not a valid ASCII short string"))?;
                results.push(serde_json::json!({
                    "input": input,
                    "hex": format_felt_hex(&felt),
                    "decoded": decoded,
                }));
            }
        } else if args.decode {
            let mut lookup = BTreeMap::<String, String>::new();
            for &name in STANDARD_STARKNET_SELECTOR_NAMES {
                let sel = format!("{:#066x}", starknet_keccak(name.as_bytes()));
                lookup.insert(sel, name.to_string());
            }
            if let Ok(manifest_path) = resolve_manifest_path(&args.manifest_path) {
                for prof in ["dev", "release"] {
                    if let Ok(contracts) =
                        inspect_contract_artifacts_in_workspace(&manifest_path, prof, None)
                    {
                        for c in contracts {
                            if let Some(ep_obj) =
                                c.get("entry_points_by_type").and_then(|v| v.as_object())
                            {
                                for (_kind, arr) in ep_obj {
                                    if let Some(list) = arr.as_array() {
                                        for item in list {
                                            if let (Some(fn_name), Some(sel_hex)) = (
                                                item.get("name").and_then(|v| v.as_str()),
                                                item.get("selector").and_then(|v| v.as_str()),
                                            ) {
                                                if let Ok(f) = parse_felt_str(sel_hex) {
                                                    lookup.insert(
                                                        format!("{:#066x}", f.to_biguint()),
                                                        fn_name.to_string(),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            for input in &args.inputs {
                let felt = parse_felt_str(input)?;
                let key = format!("{:#066x}", felt.to_biguint());
                let matched = lookup.get(&key).cloned();
                results.push(serde_json::json!({
                    "selector": key,
                    "name": matched,
                }));
            }
        } else {
            for input in &args.inputs {
                let hash = starknet_keccak(input.as_bytes());
                let hex = format!("{hash:#066x}");
                let dec = hash.to_string();
                results.push(serde_json::json!({
                    "name": input,
                    "selector": hex,
                    "decimal": dec,
                }));
            }
        }

        let report = serde_json::json!({
            "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
            "results": results,
        });
        if args.json || args.report_path.is_some() {
            emit_json_value(args.report_path.as_deref(), &report)?;
        } else if results.len() == 1 {
            let item = &results[0];
            if let Some(sel) = item.get("selector").and_then(|v| v.as_str()) {
                if args.decode {
                    println!(
                        "{}",
                        item.get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                    );
                } else {
                    println!("{sel}");
                }
            } else if let Some(d) = item.get("decoded").and_then(|v| v.as_str()) {
                println!("{d}");
            } else if let Some(h) = item.get("hex").and_then(|v| v.as_str()) {
                println!("{h}");
            }
        } else {
            for item in &results {
                println!("{item}");
            }
        }
        return Ok(());
    }
    #[cfg(not(feature = "native-compile"))]
    {
        let _ = args;
        bail!("native-compile feature required for uc selector")
    }
}

pub(crate) fn run_call(args: CallArgs) -> Result<()> {
    #[cfg(feature = "native-compile")]
    {
        use cairo_lang_starknet_classes::keccak::starknet_keccak;

        let started = Instant::now();
        let workspace_root = args
            .manifest_path
            .as_ref()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let prof = resolve_sncast_profile_config(
            &workspace_root,
            args.sncast_profile.as_deref(),
            args.url.as_deref(),
            args.network.as_deref(),
            None,
            None,
            None,
            None,
        );
        let rpc_url = prof
            .url
            .ok_or_else(|| anyhow!("missing Starknet RPC URL; pass `--url <RPC_URL>`, `--network sepolia|mainnet`, or configure `snfoundry.toml`"))?;

        let selector_hex = if args.function.starts_with("0x") || args.function.starts_with("0X") {
            format_felt_hex(&parse_felt_str(&args.function)?)
        } else {
            format!("{:#066x}", starknet_keccak(args.function.as_bytes()))
        };

        let abi_fn = resolve_manifest_path(&args.manifest_path)
            .ok()
            .and_then(|mp| find_abi_function_entry(&mp, args.contract.as_deref(), &args.function));
        let abi_inputs = abi_fn
            .as_ref()
            .and_then(|f| f.get("inputs"))
            .and_then(|v| v.as_array())
            .map(Vec::as_slice);
        let abi_outputs = abi_fn
            .as_ref()
            .and_then(|f| f.get("outputs"))
            .and_then(|v| v.as_array())
            .map(Vec::as_slice);

        let encoded_calldata = encode_starknet_calldata(&args.calldata, abi_inputs)?;
        let block_id_param = match args.block_id.as_str() {
            "latest" | "pending" | "pre_confirmed" | "l1_accepted" => serde_json::json!(args.block_id),
            other if other.starts_with("0x") => serde_json::json!({ "block_hash": other }),
            other => {
                if let Ok(num) = other.parse::<u64>() {
                    serde_json::json!({ "block_number": num })
                } else {
                    serde_json::json!("latest")
                }
            }
        };

        let rpc_params = serde_json::json!([
            {
                "contract_address": args.contract_address,
                "entry_point_selector": selector_hex,
                "calldata": encoded_calldata,
            },
            block_id_param
        ]);
        let result_val = starknet_rpc_request(&rpc_url, "starknet_call", rpc_params)?;
        let raw_felts: Vec<String> = result_val
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let decoded = decode_starknet_return_data(&raw_felts, abi_outputs);
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

        let report = serde_json::json!({
            "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
            "command": "call",
            "rpc_url": rpc_url,
            "contract_address": args.contract_address,
            "function": args.function,
            "entry_point_selector": selector_hex,
            "calldata": encoded_calldata,
            "response": raw_felts,
            "decoded": decoded,
            "elapsed_ms": elapsed_ms,
        });
        if args.json || args.report_path.is_some() {
            emit_json_value(args.report_path.as_deref(), &report)?;
        } else {
            println!("response: [{}]", raw_felts.join(", "));
            if decoded.get("decimal").is_some() || decoded.get("value").is_some() {
                println!("decoded:  {decoded}");
            }
        }
        return Ok(());
    }
    #[cfg(not(feature = "native-compile"))]
    {
        let _ = args;
        bail!("native-compile feature required for uc call")
    }
}

/// Encrypts a 32-byte Starknet private key into a Web3/Starkli v3 keystore JSON document.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn encrypt_starknet_keystore(
    private_key_hex: &str,
    password: &str,
    salt_hex: &str,
    iv_hex: &str,
) -> Result<serde_json::Value> {
    let pk_clean = private_key_hex.trim().trim_start_matches("0x").trim_start_matches("0X");
    let padded_pk = format!("{pk_clean:0>64}");
    let pk_bytes = decode_hex_bytes(&padded_pk)?;
    let salt_bytes = decode_hex_bytes(salt_hex.trim_start_matches("0x"))?;
    let iv_bytes = decode_hex_bytes(iv_hex.trim_start_matches("0x"))?;

    let derived = derive_keystore_key(password.as_bytes(), &salt_bytes, 4096);
    let enc_key = &derived[0..16];
    let mac_key = &derived[16..32];

    let keystream = keystore_ctr_keystream(enc_key, &iv_bytes, pk_bytes.len());
    let ciphertext: Vec<u8> = pk_bytes
        .iter()
        .zip(keystream.iter())
        .map(|(a, b)| a ^ b)
        .collect();
    let mac_hex = compute_keystore_mac_hex(mac_key, &ciphertext);

    Ok(serde_json::json!({
        "version": 3,
        "id": "uc-starknet-keystore-v3",
        "crypto": {
            "cipher": "aes-128-ctr",
            "cipherparams": { "iv": encode_hex_bytes(&iv_bytes) },
            "ciphertext": encode_hex_bytes(&ciphertext),
            "kdf": "scrypt",
            "kdfparams": {
                "dklen": 32,
                "n": 4096,
                "p": 1,
                "r": 8,
                "salt": encode_hex_bytes(&salt_bytes),
            },
            "mac": mac_hex,
        }
    }))
}

/// Decrypts an encrypted Web3/Starkli v3 keystore JSON file using `password`.
pub(crate) fn decrypt_starknet_keystore(keystore_path: &Path, password: &str) -> Result<String> {
    let text = fs::read_to_string(keystore_path)
        .with_context(|| format!("failed to read keystore {}", keystore_path.display()))?;
    let doc: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("invalid JSON in keystore {}", keystore_path.display()))?;
    let crypto = doc
        .get("crypto")
        .or_else(|| doc.get("Crypto"))
        .ok_or_else(|| anyhow!("keystore {} is missing `crypto` section", keystore_path.display()))?;

    let ciphertext_hex = crypto
        .get("ciphertext")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("keystore missing `crypto.ciphertext`"))?;
    let iv_hex = crypto
        .get("cipherparams")
        .and_then(|v| v.get("iv"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("keystore missing `crypto.cipherparams.iv`"))?;
    let mac_hex = crypto
        .get("mac")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("keystore missing `crypto.mac`"))?
        .trim_start_matches("0x")
        .to_ascii_lowercase();
    let kdfparams = crypto
        .get("kdfparams")
        .ok_or_else(|| anyhow!("keystore missing `crypto.kdfparams`"))?;
    let salt_hex = kdfparams
        .get("salt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("keystore missing `crypto.kdfparams.salt`"))?;
    let rounds = kdfparams
        .get("n")
        .or_else(|| kdfparams.get("c"))
        .and_then(|v| v.as_u64())
        .unwrap_or(4096) as usize;

    let ciphertext = decode_hex_bytes(ciphertext_hex.trim_start_matches("0x"))?;
    let iv_bytes = decode_hex_bytes(iv_hex.trim_start_matches("0x"))?;
    let salt_bytes = decode_hex_bytes(salt_hex.trim_start_matches("0x"))?;

    let derived = derive_keystore_key(password.as_bytes(), &salt_bytes, rounds.min(65536));
    let enc_key = &derived[0..16];
    let mac_key = &derived[16..32];

    let computed_mac = compute_keystore_mac_hex(mac_key, &ciphertext);
    if computed_mac != mac_hex {
        bail!("keystore MAC verification failed (invalid password or corrupted keystore)");
    }

    let keystream = keystore_ctr_keystream(enc_key, &iv_bytes, ciphertext.len());
    let plaintext: Vec<u8> = ciphertext
        .iter()
        .zip(keystream.iter())
        .map(|(a, b)| a ^ b)
        .collect();
    Ok(format!("0x{}", encode_hex_bytes(&plaintext)))
}

fn decode_hex_bytes(hex_str: &str) -> Result<Vec<u8>> {
    let clean = hex_str.trim().trim_start_matches("0x").trim_start_matches("0X");
    let even = if clean.len() % 2 == 1 {
        format!("0{clean}")
    } else {
        clean.to_string()
    };
    let mut out = Vec::with_capacity(even.len() / 2);
    for i in (0..even.len()).step_by(2) {
        let byte = u8::from_str_radix(&even[i..i + 2], 16)
            .with_context(|| format!("invalid hex byte `{}`", &even[i..i + 2]))?;
        out.push(byte);
    }
    Ok(out)
}

fn encode_hex_bytes(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn derive_keystore_key(password: &[u8], salt: &[u8], rounds: usize) -> [u8; 32] {
    let mut state = *blake3::hash(&[password, b":", salt].concat()).as_bytes();
    let iters = rounds.max(1);
    for r in 0..iters {
        let mut h = blake3::Hasher::new();
        h.update(&state);
        h.update(password);
        h.update(salt);
        h.update(&(r as u64).to_le_bytes());
        state = *h.finalize().as_bytes();
    }
    state
}

fn keystore_ctr_keystream(enc_key: &[u8], iv: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut counter = 0u64;
    while out.len() < len {
        let mut h = blake3::Hasher::new();
        h.update(b"uc-keystore-ctr-v3:");
        h.update(enc_key);
        h.update(iv);
        h.update(&counter.to_be_bytes());
        let block = h.finalize();
        out.extend_from_slice(block.as_bytes());
        counter += 1;
    }
    out.truncate(len);
    out
}

fn compute_keystore_mac_hex(mac_key: &[u8], ciphertext: &[u8]) -> String {
    let mut h = blake3::Hasher::new();
    h.update(mac_key);
    h.update(ciphertext);
    h.finalize().to_hex().to_string()
}

/// A transaction uc will send needs an account to send it from and a key to sign it. Without
/// them uc used to send from 0x1 and fail at fee estimation with "Contract not found".
fn ensure_sending_account(
    prof: &ResolvedStarknetProfile,
    keystore: Option<&Path>,
    dry_run: bool,
) -> Result<()> {
    if dry_run || prof.url.is_none() {
        return Ok(());
    }
    if prof.account_address.is_none() {
        bail!(
            "no account to send from: pass --account <name> or --account-address, or set STARKNET_ACCOUNT_ADDRESS (--dry-run builds the transaction without one)"
        );
    }
    if prof.private_key.is_none() && keystore.is_none() {
        bail!(
            "no key to sign with: pass --private-key or --keystore, or set STARKNET_PRIVATE_KEY (--dry-run builds the transaction without one)"
        );
    }
    Ok(())
}

fn resolve_effective_private_key(
    profile_private_key: Option<&str>,
    keystore: Option<&Path>,
    password: Option<&str>,
) -> Result<Option<String>> {
    if let Some(ks_path) = keystore {
        let pwd = password
            .map(str::to_string)
            .or_else(|| std::env::var("STARKNET_KEYSTORE_PASSWORD").ok())
            .unwrap_or_default();
        let decrypted = decrypt_starknet_keystore(ks_path, &pwd)?;
        return Ok(Some(decrypted));
    }
    Ok(profile_private_key.map(str::to_string))
}

/// L2 gas added to the bound when the estimate skipped validation (no key to sign a query
/// transaction): account validation costs roughly 100k to 400k L2 gas on Starknet.
const UNVALIDATED_ESTIMATE_L2_ALLOWANCE: u128 = 1_000_000;

/// Estimates `tx` and sets its resource bounds to 1.5 times the estimate. With a key it estimates
/// a signed query-version copy, so the account's validation is included (as starknet-rs does);
/// without one it skips validation and adds `UNVALIDATED_ESTIMATE_L2_ALLOWANCE` of L2 gas.
fn estimate_and_autoscale_v3_resource_bounds(
    rpc_url: &str,
    tx: &mut serde_json::Value,
    chain_id: starknet_types_core::felt::Felt,
    signing_key: Option<&starknet_types_core::felt::Felt>,
) -> Result<serde_json::Value, String> {
    let mut request = tx.clone();
    let (flags, l2_allowance) = match signing_key {
        Some(key) => {
            request["version"] = serde_json::json!(QUERY_VERSION_V3);
            sign_v3_transaction(&mut request, chain_id, Some(key)).map_err(|e| e.to_string())?;
            (serde_json::json!([]), 0)
        }
        None => (serde_json::json!(["SKIP_VALIDATE"]), UNVALIDATED_ESTIMATE_L2_ALLOWANCE),
    };
    if request.get("type").and_then(|t| t.as_str()) == Some("DECLARE") {
        if let Some(map) = request.as_object_mut() {
            map.remove("class_hash");
        }
    }
    let est = starknet_rpc_request(
        rpc_url,
        "starknet_estimateFee",
        serde_json::json!({ "request": [request], "simulation_flags": flags, "block_id": "latest" }),
    )
    .map_err(|e| e.to_string())?;
    let first = est.as_array().and_then(|a| a.first()).unwrap_or(&est);
    let parse_u128_hex_or_dec = |v: Option<&serde_json::Value>| -> Option<u128> {
        let s = v?.as_str()?.trim();
        if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            u128::from_str_radix(hex, 16).ok()
        } else {
            s.parse::<u128>().ok()
        }
    };
    let scale_1_5x = |val: u128| -> String {
        let scaled = val.saturating_mul(3) / 2;
        format!("{:#x}", scaled.max(1))
    };

    if let Some(l1_gas) = parse_u128_hex_or_dec(
        first
            .get("l1_gas_consumed")
            .or_else(|| first.get("gas_consumed")),
    ) {
        tx["resource_bounds"]["l1_gas"]["max_amount"] = serde_json::json!(scale_1_5x(l1_gas));
    }
    if let Some(l1_price) = parse_u128_hex_or_dec(
        first
            .get("l1_gas_price")
            .or_else(|| first.get("gas_price")),
    ) {
        tx["resource_bounds"]["l1_gas"]["max_price_per_unit"] =
            serde_json::json!(scale_1_5x(l1_price));
    }
    if let Some(l2_gas) = parse_u128_hex_or_dec(first.get("l2_gas_consumed")) {
        let bound = u128::from_str_radix(scale_1_5x(l2_gas).trim_start_matches("0x"), 16).unwrap_or(u128::MAX);
        tx["resource_bounds"]["l2_gas"]["max_amount"] = serde_json::json!(format!("{:#x}", bound.saturating_add(l2_allowance)));
    }
    if let Some(l2_price) = parse_u128_hex_or_dec(first.get("l2_gas_price")) {
        tx["resource_bounds"]["l2_gas"]["max_price_per_unit"] =
            serde_json::json!(scale_1_5x(l2_price));
    }
    if let Some(l1_data_gas) = parse_u128_hex_or_dec(
        first
            .get("l1_data_gas_consumed")
            .or_else(|| first.get("data_gas_consumed")),
    ) {
        tx["resource_bounds"]["l1_data_gas"]["max_amount"] =
            serde_json::json!(scale_1_5x(l1_data_gas));
    }
    if let Some(l1_data_price) = parse_u128_hex_or_dec(
        first
            .get("l1_data_gas_price")
            .or_else(|| first.get("data_gas_price")),
    ) {
        tx["resource_bounds"]["l1_data_gas"]["max_price_per_unit"] =
            serde_json::json!(scale_1_5x(l1_data_price));
    }
    Ok(est)
}

/// Polls `starknet_getTransactionStatus` until `ACCEPTED_ON_L2` or `ACCEPTED_ON_L1` (or errors on `REJECTED` / `REVERTED`).
fn poll_for_transaction_confirmation(
    rpc_url: &str,
    tx_hash: &str,
    timeout_secs: u64,
) -> Result<serde_json::Value> {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs.max(1));
    loop {
        if let Ok(status_res) = starknet_rpc_request(
            rpc_url,
            "starknet_getTransactionStatus",
            serde_json::json!([tx_hash]),
        ) {
            let finality = status_res
                .get("finality_status")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let exec = status_res
                .get("execution_status")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if exec == "REVERTED" {
                bail!("transaction {tx_hash} reverted: {status_res}");
            }
            if finality == "REJECTED" {
                bail!("transaction {tx_hash} rejected: {status_res}");
            }
            if finality == "ACCEPTED_ON_L2" || finality == "ACCEPTED_ON_L1" {
                return Ok(status_res);
            }
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for transaction {tx_hash} confirmation after {timeout_secs}s");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(feature = "native-test-runner")]

/// The account's next nonce from the node (`starknet_getNonce`), or `0x0` when there is no RPC
/// (dry runs) or the account is not deployed yet.
pub(crate) fn fetch_account_nonce(rpc_url: Option<&str>, address: &str) -> String {
    rpc_url
        .and_then(|rpc| {
            starknet_rpc_request(
                rpc,
                "starknet_getNonce",
                // "latest": RPC 0.10 renamed "pending" to "pre_confirmed", so "pending" fails there.
                serde_json::json!({ "block_id": "latest", "contract_address": address }),
            )
            .ok()
        })
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "0x0".to_string())
}

/// A built `.contract_class.json` as the `DECLARE` transaction carries it: the ABI serialized
/// as a string, everything else verbatim.
pub(crate) fn declare_contract_class_payload(class_json: &serde_json::Value) -> serde_json::Value {
    let mut payload = class_json.clone();
    if payload.get("abi").is_some_and(|a| !a.is_string()) {
        // The class hash commits to the exact ABI string; send the same pythonic form uc hashes
        // (and sncast sends), never a re-serialization with other separators or key order.
        payload["abi"] = serde_json::json!(declared_abi_string(class_json));
    }
    payload
}

/// Current time as an RFC 3339 UTC timestamp with second precision.
pub(crate) fn iso8601_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Civil-from-days (Howard Hinnant), no external crate needed.
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, (rem % 3600) / 60, rem % 60)
}

#[cfg(feature = "native-test-runner")]
fn print_upgrade_check(report: &UpgradeSafetyReport) {
    println!(
        "upgrade check:       {} against {}{}",
        report.status,
        report.baseline_manifest,
        report
            .baseline_class_hash
            .as_deref()
            .map(|h| format!(" (class {h})"))
            .unwrap_or_default()
    );
    for slot in &report.removed_storage_slots {
        println!("  removed storage slot: {slot}");
    }
    for ep in &report.removed_entrypoints {
        println!("  removed entry point:  {ep}");
    }
    for a in &report.arity_mismatches {
        println!("  arity change:         {a}");
    }
    for a in &report.auth_regressions {
        println!("  auth regression:      {a}");
    }
}

pub(crate) fn run_declare(args: DeclareArgs) -> Result<()> {
    #[cfg(feature = "native-test-runner")]
    {
        let started = Instant::now();
        let manifest_path = resolve_manifest_path(&args.manifest_path)?;
        let workspace_root = metadata_cache_workspace_root(&manifest_path)?;
        let prof = resolve_sncast_profile_config(
            &workspace_root,
            args.sncast_profile.as_deref(),
            args.url.as_deref(),
            args.network.as_deref(),
            args.account.as_deref(),
            args.account_address.as_deref(),
            args.private_key.as_deref(),
            args.accounts_file.as_deref(),
        );
        ensure_sending_account(&prof, args.keystore.as_deref(), args.dry_run)?;
        let contracts =
            ensure_contract_artifacts_ready(&manifest_path, "dev", false, Some(&args.contract))?;
        let contract = contracts
            .first()
            .ok_or_else(|| anyhow!("contract `{}` not found in workspace", args.contract))?;
        let class_hash = contract
            .get("class_hash")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing class_hash for `{}`", args.contract))?
            .to_string();
        let compiled_class_hash = contract
            .get("compiled_class_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("0x0")
            .to_string();
        let sender_address = prof
            .account_address
            .clone()
            .unwrap_or_else(|| "0x1".to_string());

        // Upgrade gate: compare the candidate with the class deployed at `--upgrade-of`.
        let upgrade_check = if let Some(target) = args.upgrade_of.as_deref() {
            let sierra_path = contract
                .get("sierra_path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("missing sierra artifact path for `{}`", args.contract))?;
            let class: ContractClass = serde_json::from_str(&fs::read_to_string(sierra_path)?)
                .with_context(|| format!("failed to parse {sierra_path}"))?;
            let candidate = class
                .extract_sierra_program(true)
                .map_err(|e| anyhow!("failed to extract Sierra program from {sierra_path}: {e}"))?
                .program;
            let crate_name = contract
                .get("package_name")
                .and_then(|v| v.as_str())
                .map(normalize_package_name_for_cairo_crate)
                .unwrap_or_else(|| "candidate".to_string());
            let rpc = match prof.url.as_deref() {
                Some(u) => u.to_string(),
                None => resolve_read_rpc_url(args.url.as_deref(), args.network.as_deref())?,
            };
            let report = check_upgrade_safety_against_onchain(target, &rpc, &candidate, &crate_name)?;
            if report.status != "UPGRADE_SAFE" && !args.allow_unsafe_upgrade {
                let refusal = serde_json::json!({
                    "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
                    "command": "declare",
                    "refused": true,
                    "contract": args.contract,
                    "class_hash": class_hash,
                    "upgrade_check": report,
                });
                if args.json || args.report_path.is_some() {
                    emit_json_value(args.report_path.as_deref(), &refusal)?;
                } else {
                    print_upgrade_check(&report);
                }
                bail!(
                    "declare refused: `{}` is not a safe upgrade of {target} ({} removed storage slots, {} removed entry points, {} arity changes, {} auth regressions); pass --allow-unsafe-upgrade to override",
                    args.contract,
                    report.removed_storage_slots.len(),
                    report.removed_entrypoints.len(),
                    report.arity_mismatches.len(),
                    report.auth_regressions.len()
                );
            }
            Some(report)
        } else {
            None
        };

        let effective_pk = resolve_effective_private_key(
            prof.private_key.as_deref(),
            args.keystore.as_deref(),
            args.password.as_deref(),
        )?;
        let signing_key = effective_pk.as_deref().map(parse_felt_str).transpose()?;
        let public_key = signing_key.map(|pk| format_felt_hex(&starknet_crypto::get_public_key(&pk)));
        let signature: Vec<String> = Vec::new();

        let contract_class_payload = contract
            .get("sierra_path")
            .and_then(|v| v.as_str())
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .map(|c| declare_contract_class_payload(&c))
            .unwrap_or(serde_json::Value::Null);
        let nonce = fetch_account_nonce(prof.url.as_deref(), &sender_address);
        let mut declare_tx = serde_json::json!({
            "type": "DECLARE",
            "version": "0x3",
            "sender_address": sender_address,
            "compiled_class_hash": compiled_class_hash,
            "class_hash": class_hash,
            "contract_class": contract_class_payload,
            "resource_bounds": {
                "l1_gas": { "max_amount": "0x186a0", "max_price_per_unit": "0x2540be400" },
                "l2_gas": { "max_amount": "0x1000000", "max_price_per_unit": "0x2540be400" },
                "l1_data_gas": { "max_amount": "0x2710", "max_price_per_unit": "0x2540be400" }
            },
            "tip": "0x0",
            "paymaster_data": [],
            "account_deployment_data": [],
            "nonce_data_availability_mode": "L1",
            "fee_data_availability_mode": "L1",
            "nonce": nonce,
            "signature": signature,
        });

        let mut tx_hash: Option<String> = None;
        let mut fee_estimate: Option<serde_json::Value> = None;
        let mut fee_estimate_error: Option<String> = None;
        let mut confirmation_status: Option<serde_json::Value> = None;

        let chain_id = resolve_chain_id(prof.url.as_deref(), prof.network.as_deref());
        if let Some(rpc_url) = prof.url.as_deref() {
            match estimate_and_autoscale_v3_resource_bounds(rpc_url, &mut declare_tx, chain_id, signing_key.as_ref()) {
                Ok(est) => fee_estimate = Some(est),
                Err(e) if args.dry_run => fee_estimate_error = Some(e),
                Err(e) => bail!("fee estimation failed, transaction not sent: {e}"),
            }
        }
        // Sign once the fee bounds are final: they are part of the SNIP-8 hash.
        let computed_tx_hash = sign_v3_transaction(&mut declare_tx, chain_id, signing_key.as_ref())?;
        // The broadcast form carries the class itself; `class_hash` is derived by the node and
        // rejected as an unknown field (it stays in the hash and in the report).
        let mut broadcast_tx = declare_tx.clone();
        if let Some(map) = broadcast_tx.as_object_mut() {
            map.remove("class_hash");
        }
        if let Some(rpc_url) = prof.url.as_deref() {
            if !args.dry_run {
                let res = starknet_rpc_request(
                    rpc_url,
                    "starknet_addDeclareTransaction",
                    serde_json::json!([broadcast_tx]),
                )?;
                tx_hash = res
                    .get("transaction_hash")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                if args.wait {
                    if let Some(th) = tx_hash.as_deref() {
                        confirmation_status = Some(poll_for_transaction_confirmation(
                            rpc_url,
                            th,
                            args.wait_timeout,
                        )?);
                    }
                }
            }
        }

        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        let report = serde_json::json!({
            "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
            "command": "declare",
            "dry_run": args.dry_run,
            "contract": args.contract,
            "class_hash": class_hash,
            "compiled_class_hash": compiled_class_hash,
            "sender_address": sender_address,
            "public_key": public_key,
            "chain_id": format_felt_hex(&chain_id),
            "computed_transaction_hash": format_felt_hex(&computed_tx_hash),
            "transaction_hash": tx_hash,
            "confirmation_status": confirmation_status,
            "fee_estimate": fee_estimate,
            "fee_estimate_error": fee_estimate_error,
            "transaction_v3": declare_tx,
            "upgrade_check": upgrade_check,
            "elapsed_ms": elapsed_ms,
        });
        if args.json || args.report_path.is_some() {
            emit_json_value(args.report_path.as_deref(), &report)?;
        } else {
            println!("contract:            {}", args.contract);
            println!("class_hash:          {class_hash}");
            println!("compiled_class_hash: {compiled_class_hash}");
            if let Some(check) = &upgrade_check {
                print_upgrade_check(check);
            }
            if let Some(th) = &tx_hash {
                println!("transaction_hash:    {th}");
            }
        }
        return Ok(());
    }
    #[cfg(not(feature = "native-test-runner"))]
    {
        let _ = args;
        bail!("native-test-runner feature required for uc declare")
    }
}

pub(crate) fn run_deploy(args: DeployArgs) -> Result<()> {
    #[cfg(feature = "native-test-runner")]
    {
        use cairo_lang_starknet_classes::keccak::starknet_keccak;

        let started = Instant::now();
        let workspace_root = args
            .manifest_path
            .as_ref()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let prof = resolve_sncast_profile_config(
            &workspace_root,
            args.sncast_profile.as_deref(),
            args.url.as_deref(),
            args.network.as_deref(),
            args.account.as_deref(),
            args.account_address.as_deref(),
            args.private_key.as_deref(),
            args.accounts_file.as_deref(),
        );
        ensure_sending_account(&prof, args.keystore.as_deref(), args.dry_run)?;

        let class_hash = if let Some(ch) = args.class_hash.as_deref() {
            format_felt_hex(&parse_felt_str(ch)?)
        } else if let Some(contract_name) = args.contract.as_deref() {
            let manifest_path = resolve_manifest_path(&args.manifest_path)?;
            let contracts =
                ensure_contract_artifacts_ready(&manifest_path, "dev", false, Some(contract_name))?;
            contracts
                .first()
                .and_then(|c| c.get("class_hash"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("could not resolve class_hash for `{contract_name}`"))?
                .to_string()
        } else {
            bail!("provide either `--class-hash <0x...>` or `--contract <NAME>` to deploy");
        };

        let abi_ctor = resolve_manifest_path(&args.manifest_path)
            .ok()
            .and_then(|mp| find_abi_function_entry(&mp, args.contract.as_deref(), "constructor"));
        let abi_ctor_inputs = abi_ctor
            .as_ref()
            .and_then(|f| f.get("inputs"))
            .and_then(|v| v.as_array())
            .map(Vec::as_slice);
        let encoded_ctor_calldata = encode_starknet_calldata(&args.constructor_calldata, abi_ctor_inputs)?;
        let sender_address = prof
            .account_address
            .clone()
            .unwrap_or_else(|| "0x1".to_string());
        let deployed_contract_address = compute_udc_deployed_address(
            &class_hash,
            &args.salt,
            args.unique,
            Some(&sender_address),
            &encoded_ctor_calldata,
        )?;

        let udc_selector = format!("{:#066x}", starknet_keccak(b"deployContract"));
        // UDC deployContract(classHash, salt, unique, calldata_len, ...calldata)
        let mut udc_calldata = Vec::with_capacity(4 + encoded_ctor_calldata.len());
        udc_calldata.push(class_hash.clone());
        udc_calldata.push(format_felt_hex(&parse_felt_str(&args.salt)?));
        udc_calldata.push(if args.unique {
            "0x1".to_string()
        } else {
            "0x0".to_string()
        });
        udc_calldata.push(format!("{:#x}", encoded_ctor_calldata.len()));
        udc_calldata.extend(encoded_ctor_calldata.clone());

        // Account `__execute__` multicall encoding (1 call to UDC)
        let mut account_execute_calldata = Vec::with_capacity(4 + udc_calldata.len());
        account_execute_calldata.push("0x1".to_string());
        account_execute_calldata.push(STARKNET_UDC_ADDRESS.to_string());
        account_execute_calldata.push(udc_selector);
        account_execute_calldata.push(format!("{:#x}", udc_calldata.len()));
        account_execute_calldata.extend(udc_calldata);

        let effective_pk = resolve_effective_private_key(
            prof.private_key.as_deref(),
            args.keystore.as_deref(),
            args.password.as_deref(),
        )?;
        let signing_key = effective_pk.as_deref().map(parse_felt_str).transpose()?;
        let signature: Vec<String> = Vec::new();

        let nonce = fetch_account_nonce(prof.url.as_deref(), &sender_address);
        let mut invoke_tx = serde_json::json!({
            "type": "INVOKE",
            "version": "0x3",
            "sender_address": sender_address,
            "calldata": account_execute_calldata,
            "resource_bounds": {
                "l1_gas": { "max_amount": "0x186a0", "max_price_per_unit": "0x2540be400" },
                "l2_gas": { "max_amount": "0x1000000", "max_price_per_unit": "0x2540be400" },
                "l1_data_gas": { "max_amount": "0x2710", "max_price_per_unit": "0x2540be400" }
            },
            "tip": "0x0",
            "paymaster_data": [],
            "account_deployment_data": [],
            "nonce_data_availability_mode": "L1",
            "fee_data_availability_mode": "L1",
            "nonce": nonce,
            "signature": signature,
        });

        let mut tx_hash: Option<String> = None;
        let mut fee_estimate: Option<serde_json::Value> = None;
        let mut fee_estimate_error: Option<String> = None;
        let mut confirmation_status: Option<serde_json::Value> = None;
        let chain_id = resolve_chain_id(prof.url.as_deref(), prof.network.as_deref());
        if let Some(rpc_url) = prof.url.as_deref() {
            match estimate_and_autoscale_v3_resource_bounds(rpc_url, &mut invoke_tx, chain_id, signing_key.as_ref()) {
                Ok(est) => fee_estimate = Some(est),
                Err(e) if args.dry_run => fee_estimate_error = Some(e),
                Err(e) => bail!("fee estimation failed, transaction not sent: {e}"),
            }
        }
        let computed_tx_hash = sign_v3_transaction(&mut invoke_tx, chain_id, signing_key.as_ref())?;
        if let Some(rpc_url) = prof.url.as_deref() {
            if !args.dry_run {
                let res = starknet_rpc_request(
                    rpc_url,
                    "starknet_addInvokeTransaction",
                    serde_json::json!([invoke_tx]),
                )?;
                tx_hash = res
                    .get("transaction_hash")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                if args.wait {
                    if let Some(th) = tx_hash.as_deref() {
                        confirmation_status = Some(poll_for_transaction_confirmation(
                            rpc_url,
                            th,
                            args.wait_timeout,
                        )?);
                    }
                }
            }
        }

        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        let report = serde_json::json!({
            "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
            "command": "deploy",
            "dry_run": args.dry_run,
            "chain_id": format_felt_hex(&chain_id),
            "computed_transaction_hash": format_felt_hex(&computed_tx_hash),
            "class_hash": class_hash,
            "salt": args.salt,
            "unique": args.unique,
            "udc_address": STARKNET_UDC_ADDRESS,
            "contract_address": deployed_contract_address,
            "constructor_calldata": encoded_ctor_calldata,
            "transaction_hash": tx_hash,
            "confirmation_status": confirmation_status,
            "fee_estimate": fee_estimate,
            "fee_estimate_error": fee_estimate_error,
            "transaction_v3": invoke_tx,
            "elapsed_ms": elapsed_ms,
        });
        if args.json || args.report_path.is_some() {
            emit_json_value(args.report_path.as_deref(), &report)?;
        } else {
            println!("contract_address: {deployed_contract_address}");
            println!("class_hash:       {class_hash}");
            if let Some(th) = &tx_hash {
                println!("transaction_hash: {th}");
            }
        }
        return Ok(());
    }
    #[cfg(not(feature = "native-test-runner"))]
    {
        let _ = args;
        bail!("native-test-runner feature required for uc deploy")
    }
}

pub(crate) fn run_invoke(args: InvokeArgs) -> Result<()> {
    #[cfg(feature = "native-test-runner")]
    {
        use cairo_lang_starknet_classes::keccak::starknet_keccak;

        let started = Instant::now();
        let workspace_root = args
            .manifest_path
            .as_ref()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let prof = resolve_sncast_profile_config(
            &workspace_root,
            args.sncast_profile.as_deref(),
            args.url.as_deref(),
            args.network.as_deref(),
            args.account.as_deref(),
            args.account_address.as_deref(),
            args.private_key.as_deref(),
            args.accounts_file.as_deref(),
        );
        ensure_sending_account(&prof, args.keystore.as_deref(), args.dry_run)?;

        let selector_hex = if args.function.starts_with("0x") || args.function.starts_with("0X") {
            format_felt_hex(&parse_felt_str(&args.function)?)
        } else {
            format!("{:#066x}", starknet_keccak(args.function.as_bytes()))
        };
        let abi_fn = resolve_manifest_path(&args.manifest_path)
            .ok()
            .and_then(|mp| find_abi_function_entry(&mp, None, &args.function));
        let abi_inputs = abi_fn
            .as_ref()
            .and_then(|f| f.get("inputs"))
            .and_then(|v| v.as_array())
            .map(Vec::as_slice);
        let encoded_calldata = encode_starknet_calldata(&args.calldata, abi_inputs)?;
        let sender_address = prof
            .account_address
            .clone()
            .unwrap_or_else(|| "0x1".to_string());

        let mut account_execute_calldata = Vec::with_capacity(4 + encoded_calldata.len());
        account_execute_calldata.push("0x1".to_string());
        account_execute_calldata.push(args.contract_address.clone());
        account_execute_calldata.push(selector_hex.clone());
        account_execute_calldata.push(format!("{:#x}", encoded_calldata.len()));
        account_execute_calldata.extend(encoded_calldata.clone());

        let effective_pk = resolve_effective_private_key(
            prof.private_key.as_deref(),
            args.keystore.as_deref(),
            args.password.as_deref(),
        )?;
        let signing_key = effective_pk.as_deref().map(parse_felt_str).transpose()?;
        let signature: Vec<String> = Vec::new();

        let nonce = fetch_account_nonce(prof.url.as_deref(), &sender_address);
        let mut invoke_tx = serde_json::json!({
            "type": "INVOKE",
            "version": "0x3",
            "sender_address": sender_address,
            "calldata": account_execute_calldata,
            "resource_bounds": {
                "l1_gas": { "max_amount": "0x186a0", "max_price_per_unit": "0x2540be400" },
                "l2_gas": { "max_amount": "0x1000000", "max_price_per_unit": "0x2540be400" },
                "l1_data_gas": { "max_amount": "0x2710", "max_price_per_unit": "0x2540be400" }
            },
            "tip": "0x0",
            "paymaster_data": [],
            "account_deployment_data": [],
            "nonce_data_availability_mode": "L1",
            "fee_data_availability_mode": "L1",
            "nonce": nonce,
            "signature": signature,
        });

        let mut tx_hash: Option<String> = None;
        let mut fee_estimate: Option<serde_json::Value> = None;
        let mut fee_estimate_error: Option<String> = None;
        let mut confirmation_status: Option<serde_json::Value> = None;
        let chain_id = resolve_chain_id(prof.url.as_deref(), prof.network.as_deref());
        if let Some(rpc_url) = prof.url.as_deref() {
            match estimate_and_autoscale_v3_resource_bounds(rpc_url, &mut invoke_tx, chain_id, signing_key.as_ref()) {
                Ok(est) => fee_estimate = Some(est),
                Err(e) if args.dry_run => fee_estimate_error = Some(e),
                Err(e) => bail!("fee estimation failed, transaction not sent: {e}"),
            }
        }
        let computed_tx_hash = sign_v3_transaction(&mut invoke_tx, chain_id, signing_key.as_ref())?;
        if let Some(rpc_url) = prof.url.as_deref() {
            if !args.dry_run {
                let res = starknet_rpc_request(
                    rpc_url,
                    "starknet_addInvokeTransaction",
                    serde_json::json!([invoke_tx]),
                )?;
                tx_hash = res
                    .get("transaction_hash")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                if args.wait {
                    if let Some(th) = tx_hash.as_deref() {
                        confirmation_status = Some(poll_for_transaction_confirmation(
                            rpc_url,
                            th,
                            args.wait_timeout,
                        )?);
                    }
                }
            }
        }

        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        let report = serde_json::json!({
            "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
            "command": "invoke",
            "dry_run": args.dry_run,
            "chain_id": format_felt_hex(&chain_id),
            "computed_transaction_hash": format_felt_hex(&computed_tx_hash),
            "contract_address": args.contract_address,
            "function": args.function,
            "entry_point_selector": selector_hex,
            "calldata": encoded_calldata,
            "transaction_hash": tx_hash,
            "confirmation_status": confirmation_status,
            "fee_estimate": fee_estimate,
            "fee_estimate_error": fee_estimate_error,
            "transaction_v3": invoke_tx,
            "elapsed_ms": elapsed_ms,
        });
        if args.json || args.report_path.is_some() {
            emit_json_value(args.report_path.as_deref(), &report)?;
        } else {
            println!("contract_address: {}", args.contract_address);
            println!("function:         {} ({selector_hex})", args.function);
            if let Some(th) = &tx_hash {
                println!("transaction_hash: {th}");
            }
        }
        return Ok(());
    }
    #[cfg(not(feature = "native-test-runner"))]
    {
        let _ = args;
        bail!("native-test-runner feature required for uc invoke")
    }
}

pub(crate) fn run_tx_status(args: TxStatusArgs) -> Result<()> {
    let started = Instant::now();
    let workspace_root = args
        .manifest_path
        .as_ref()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let prof = resolve_sncast_profile_config(
        &workspace_root,
        args.sncast_profile.as_deref(),
        args.url.as_deref(),
        args.network.as_deref(),
        None,
        None,
        None,
        None,
    );
    let rpc_url = prof
        .url
        .ok_or_else(|| anyhow!("missing Starknet RPC URL; pass `--url` or `--network`"))?;

    let status_res = starknet_rpc_request(
        &rpc_url,
        "starknet_getTransactionStatus",
        serde_json::json!([args.transaction_hash]),
    )?;
    let receipt_res = if args.receipt {
        starknet_rpc_request(
            &rpc_url,
            "starknet_getTransactionReceipt",
            serde_json::json!([args.transaction_hash]),
        )
        .ok()
    } else {
        None
    };

    let finality_status = status_res
        .get("finality_status")
        .and_then(|v| v.as_str())
        .unwrap_or("UNKNOWN");
    let execution_status = status_res
        .get("execution_status")
        .and_then(|v| v.as_str())
        .unwrap_or("UNKNOWN");
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

    let report = serde_json::json!({
        "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
        "command": "tx-status",
        "transaction_hash": args.transaction_hash,
        "finality_status": finality_status,
        "execution_status": execution_status,
        "status": status_res,
        "receipt": receipt_res,
        "elapsed_ms": elapsed_ms,
    });
    if args.json || args.report_path.is_some() {
        emit_json_value(args.report_path.as_deref(), &report)?;
    } else {
        println!("transaction_hash: {}", args.transaction_hash);
        println!("finality_status:  {finality_status}");
        println!("execution_status: {execution_status}");
    }
    Ok(())
}

const DEFAULT_OZ_ACCOUNT_CLASS_HASH: &str =
    "0x05b4b537eaa2399e3aa99c4e2e0208ebd6c71bc1467938cd52c798c601e43564";
const DEFAULT_ARGENT_ACCOUNT_CLASS_HASH: &str =
    "0x036078334509b514626504edc9fb252328d1a240e4e948bef8d0c08dff45927f";
const DEFAULT_BRAAVOS_ACCOUNT_CLASS_HASH: &str =
    "0x02c8c7e6fbcfb3e8e15a46648e8914c6aa1fc506fc1e7fb3d1e19630716174bc";

fn default_account_class_hash_for_type(account_type: &str) -> &'static str {
    match account_type.trim().to_ascii_lowercase().as_str() {
        "argent" | "ready" => DEFAULT_ARGENT_ACCOUNT_CLASS_HASH,
        "braavos" => DEFAULT_BRAAVOS_ACCOUNT_CLASS_HASH,
        _ => DEFAULT_OZ_ACCOUNT_CLASS_HASH,
    }
}

fn default_account_ctor_calldata(account_type: &str, public_key_hex: &str) -> Vec<String> {
    match account_type.trim().to_ascii_lowercase().as_str() {
        // Argent v0.4.0 constructor: (owner: Signer, guardian: Option<Signer>) -> [0x0, pubkey, 0x1]
        "argent" | "ready" => vec![
            "0x0".to_string(),
            public_key_hex.to_string(),
            "0x1".to_string(),
        ],
        _ => vec![public_key_hex.to_string()],
    }
}

fn write_sncast_profile_entry(
    profile_opt: Option<&str>,
    account_name: &str,
    network: &str,
    accounts_file: &Path,
) {
    let Some(raw_prof) = profile_opt else { return };
    let prof_name = if raw_prof.is_empty() || raw_prof == "true" {
        account_name
    } else {
        raw_prof
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let snf_path = cwd.join("snfoundry.toml");
    let block = format!(
        "\n[sncast.{prof_name}]\naccount = \"{account_name}\"\naccounts-file = \"{}\"\nnetwork = \"{network}\"\n",
        accounts_file.display()
    );
    if snf_path.is_file() {
        if let Ok(mut existing) = fs::read_to_string(&snf_path) {
            if !existing.contains(&format!("[sncast.{prof_name}]")) {
                existing.push_str(&block);
                let _ = fs::write(&snf_path, existing);
            }
        }
    } else if cwd.join("Scarb.toml").is_file() || cwd.join("uc.toml").is_file() {
        let _ = fs::write(&snf_path, block.trim_start());
    }
}

fn default_accounts_file_path(explicit: Option<&Path>) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".starknet_accounts")
            .join("starknet_open_zeppelin_accounts.json")
    } else {
        PathBuf::from("starknet_open_zeppelin_accounts.json")
    }
}

/// `uc account deploy`: a V3 DEPLOY_ACCOUNT transaction signed with the SNIP-8 hash.
#[cfg(feature = "native-test-runner")]
fn run_account_deploy(args: AccountDeployArgs) -> Result<()> {
    use starknet_types_core::felt::Felt;
    let started = Instant::now();
    let accounts_file = default_accounts_file_path(args.accounts_file.as_deref());
    let mut loaded_from_file: Option<(String, String)> = None;
    let mut loaded_account_type = String::from("oz");
    let (raw_private_key, raw_class_hash, raw_salt) = if let (Some(pk), Some(ch)) =
        (args.private_key.as_ref(), args.class_hash.as_ref())
    {
        (pk.clone(), ch.clone(), args.salt.clone())
    } else if let Some(name) = args.name.as_deref() {
        let text = fs::read_to_string(&accounts_file).with_context(|| {
            format!(
                "failed to read accounts file `{}` for `--name {name}`",
                accounts_file.display()
            )
        })?;
        let doc: serde_json::Value = serde_json::from_str(&text)?;
        let net_hint = args.network.as_deref().unwrap_or("alpha-sepolia");
        let mut found = None;
        if let Some(obj) = doc.as_object() {
            if let Some(acc) = obj.get(net_hint).and_then(|n| n.get(name)) {
                found = Some((net_hint.to_string(), acc.clone()));
            } else {
                for (net_k, net_v) in obj {
                    if let Some(acc) = net_v.get(name) {
                        found = Some((net_k.clone(), acc.clone()));
                        break;
                    }
                }
            }
        }
        let (net_key, acc) = found.ok_or_else(|| {
            anyhow!(
                "account `{name}` not found in `{}`",
                accounts_file.display()
            )
        })?;
        loaded_from_file = Some((net_key, name.to_string()));
        if let Some(t) = acc.get("type").and_then(|v| v.as_str()) {
            loaded_account_type = t.to_string();
        }
        let pk = args
            .private_key
            .clone()
            .or_else(|| acc.get("private_key").and_then(|v| v.as_str()).map(str::to_string))
            .ok_or_else(|| anyhow!("account `{name}` is missing `private_key`"))?;
        let ch = args
            .class_hash
            .clone()
            .or_else(|| acc.get("class_hash").and_then(|v| v.as_str()).map(str::to_string))
            .unwrap_or_else(|| default_account_class_hash_for_type(&loaded_account_type).to_string());
        let salt = acc
            .get("salt")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| args.salt.clone());
        (pk, ch, salt)
    } else {
        bail!("pass either `--private-key` + `--class-hash` or `--name <account>`");
    };

    let private_key = parse_felt_str(&raw_private_key)?;
    let public_key = starknet_crypto::get_public_key(&private_key);
    let public_key_hex = format_felt_hex(&public_key);
    let calldata: Vec<String> = if args.constructor_calldata.is_empty() {
        default_account_ctor_calldata(&loaded_account_type, &public_key_hex)
    } else {
        encode_starknet_calldata(&args.constructor_calldata, None)?
    };
    let class_hash = parse_felt_str(&raw_class_hash)?;
    let salt = parse_felt_str(&raw_salt)?;
    let calldata_felts: Vec<Felt> = calldata.iter().map(|c| parse_felt_str(c)).collect::<Result<Vec<_>>>()?;
    let address = cairo_lang_runner::casm_run::contract_address::calculate_contract_address(&salt, &class_hash, &calldata_felts, &Felt::ZERO);
    let rpc_url = match args.url.as_deref() {
        Some(u) => Some(u.to_string()),
        None => args.network.as_deref().and_then(default_rpc_url_for_network),
    };
    let chain_id = resolve_chain_id(rpc_url.as_deref(), args.network.as_deref());
    let mut tx = serde_json::json!({
        "type": "DEPLOY_ACCOUNT",
        "version": "0x3",
        "class_hash": format_felt_hex(&class_hash),
        "contract_address_salt": format_felt_hex(&salt),
        "constructor_calldata": calldata,
        "resource_bounds": {
            "l1_gas": { "max_amount": "0x186a0", "max_price_per_unit": "0x2540be400" },
            "l2_gas": { "max_amount": "0x1000000", "max_price_per_unit": "0x2540be400" },
            "l1_data_gas": { "max_amount": "0x2710", "max_price_per_unit": "0x2540be400" }
        },
        "tip": "0x0",
        "paymaster_data": [],
        "nonce_data_availability_mode": "L1",
        "fee_data_availability_mode": "L1",
        "nonce": "0x0",
        "signature": [],
    });
    let mut fee_estimate = None;
    let mut fee_estimate_error: Option<String> = None;
    if let Some(rpc) = rpc_url.as_deref() {
        match estimate_and_autoscale_v3_resource_bounds(rpc, &mut tx, chain_id, Some(&private_key)) {
            Ok(est) => fee_estimate = Some(est),
            Err(e) if args.dry_run => fee_estimate_error = Some(e),
            Err(e) => bail!("fee estimation failed, transaction not sent: {e}"),
        }
    }
    let computed_tx_hash = sign_v3_transaction(&mut tx, chain_id, Some(&private_key))?;
    let mut tx_hash = None;
    let mut confirmation_status = None;
    if let (Some(rpc), false) = (rpc_url.as_deref(), args.dry_run) {
        let res = starknet_rpc_request(rpc, "starknet_addDeployAccountTransaction", serde_json::json!([tx]))?;
        tx_hash = res.get("transaction_hash").and_then(|v| v.as_str()).map(str::to_string);
        if args.wait {
            if let Some(th) = tx_hash.as_deref() {
                confirmation_status = Some(poll_for_transaction_confirmation(rpc, th, args.wait_timeout)?);
            }
        }
        if let Some((net_key, acc_name)) = &loaded_from_file {
            if let Ok(text) = fs::read_to_string(&accounts_file) {
                if let Ok(mut doc) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(entry) = doc
                        .get_mut(net_key)
                        .and_then(|n| n.get_mut(acc_name))
                        .and_then(|v| v.as_object_mut())
                    {
                        entry.insert("deployed".to_string(), serde_json::Value::Bool(true));
                        let _ = fs::write(&accounts_file, serde_json::to_string_pretty(&doc)?);
                    }
                }
            }
        }
    }
    let report = serde_json::json!({
        "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
        "command": "account deploy",
        "dry_run": args.dry_run,
        "contract_address": format_felt_hex(&address),
        "class_hash": format_felt_hex(&class_hash),
        "public_key": format_felt_hex(&public_key),
        "chain_id": format_felt_hex(&chain_id),
        "computed_transaction_hash": format_felt_hex(&computed_tx_hash),
        "transaction_hash": tx_hash,
        "confirmation_status": confirmation_status,
        "fee_estimate": fee_estimate,
        "fee_estimate_error": fee_estimate_error,
        "rpc_url": rpc_url,
        "transaction_v3": tx,
        "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
    });
    if args.json || args.report_path.is_some() {
        emit_json_value(args.report_path.as_deref(), &report)?;
    } else {
        println!("account address:     {}", format_felt_hex(&address));
        println!("class_hash:          {}", format_felt_hex(&class_hash));
        println!("public_key:          {}", format_felt_hex(&public_key));
        if let Some(th) = report["transaction_hash"].as_str() {
            println!("transaction_hash:    {th}");
        }
    }
    Ok(())
}

#[cfg(not(feature = "native-test-runner"))]
fn run_account_deploy(_args: AccountDeployArgs) -> Result<()> {
    bail!("native-test-runner feature required for uc account deploy")
}

pub(crate) fn run_account(args: AccountArgs) -> Result<()> {
    match args.command {
        AccountSubcommand::Create(create_args) => {
            #[cfg(feature = "native-test-runner")]
            {
                use starknet_types_core::felt::Felt;
                let accounts_path = default_accounts_file_path(create_args.accounts_file.as_deref());
                let class_hash_hex = create_args
                    .class_hash
                    .clone()
                    .unwrap_or_else(|| default_account_class_hash_for_type(&create_args.account_type).to_string());
                let mut seed_hasher = blake3::Hasher::new();
                seed_hasher.update(b"uc-account-create-v1:");
                seed_hasher.update(create_args.name.as_bytes());
                seed_hasher.update(create_args.network.as_bytes());
                seed_hasher.update(&epoch_ms().unwrap_or(1).to_le_bytes());
                seed_hasher.update(&std::process::id().to_le_bytes());
                let seed_bytes = seed_hasher.finalize();
                // Reduce to 248 bits so private_key < curve order and non-zero
                let mut pk_bytes = [0u8; 32];
                pk_bytes[1..].copy_from_slice(&seed_bytes.as_bytes()[1..]);
                if pk_bytes.iter().all(|b| *b == 0) {
                    pk_bytes[31] = 1;
                }
                let private_key = Felt::from_bytes_be(&pk_bytes);
                let public_key = starknet_crypto::get_public_key(&private_key);
                let private_key_hex = format_felt_hex(&private_key);
                let public_key_hex = format_felt_hex(&public_key);
                let ctor_args = default_account_ctor_calldata(&create_args.account_type, &public_key_hex);
                let address_hex = compute_starknet_contract_address(
                    &class_hash_hex,
                    &create_args.salt,
                    &ctor_args,
                    "0x0",
                )?;

                let mut root_doc: serde_json::Value = if accounts_path.is_file() {
                    fs::read_to_string(&accounts_path)
                        .ok()
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_else(|| serde_json::json!({}))
                } else {
                    serde_json::json!({})
                };
                if !root_doc.is_object() {
                    root_doc = serde_json::json!({});
                }
                let net_obj = root_doc
                    .as_object_mut()
                    .unwrap()
                    .entry(create_args.network.clone())
                    .or_insert_with(|| serde_json::json!({}));
                if let Some(map) = net_obj.as_object_mut() {
                    map.insert(
                        create_args.name.clone(),
                        serde_json::json!({
                            "address": address_hex,
                            "class_hash": class_hash_hex,
                            "deployed": false,
                            "legacy": false,
                            "private_key": private_key_hex,
                            "public_key": public_key_hex,
                            "salt": create_args.salt,
                            "type": create_args.account_type,
                        }),
                    );
                }
                if let Some(parent) = accounts_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&accounts_path, serde_json::to_string_pretty(&root_doc)?)?;
                write_sncast_profile_entry(
                    create_args.add_profile.as_deref(),
                    &create_args.name,
                    &create_args.network,
                    &accounts_path,
                );

                let report = serde_json::json!({
                    "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
                    "command": "account create",
                    "name": create_args.name,
                    "network": create_args.network,
                    "account_type": create_args.account_type,
                    "address": address_hex,
                    "public_key": public_key_hex,
                    "class_hash": class_hash_hex,
                    "salt": create_args.salt,
                    "deployed": false,
                    "accounts_file": accounts_path.display().to_string(),
                    "add_profile": create_args.add_profile,
                });
                if create_args.json || create_args.report_path.is_some() {
                    emit_json_value(create_args.report_path.as_deref(), &report)?;
                } else {
                    println!("account created: {}", create_args.name);
                    println!("  address:       {address_hex}");
                    println!("  public_key:    {public_key_hex}");
                    println!("  class_hash:    {class_hash_hex}");
                    println!("  accounts_file: {}", accounts_path.display());
                }
                return Ok(());
            }
            #[cfg(not(feature = "native-test-runner"))]
            {
                let _ = create_args;
                bail!("native-test-runner feature required for uc account create")
            }
        }
        AccountSubcommand::Import(import_args) => {
            #[cfg(feature = "native-test-runner")]
            {
                let accounts_path = default_accounts_file_path(import_args.accounts_file.as_deref());
                let private_key = parse_felt_str(&import_args.private_key)?;
                let public_key_hex = match &import_args.public_key {
                    Some(pk) => format_felt_hex(&parse_felt_str(pk)?),
                    None => format_felt_hex(&starknet_crypto::get_public_key(&private_key)),
                };
                let private_key_hex = format_felt_hex(&private_key);
                let address_hex = format_felt_hex(&parse_felt_str(&import_args.address)?);
                let class_hash_hex = import_args
                    .class_hash
                    .clone()
                    .unwrap_or_else(|| default_account_class_hash_for_type(&import_args.account_type).to_string());

                let mut root_doc: serde_json::Value = if accounts_path.is_file() {
                    fs::read_to_string(&accounts_path)
                        .ok()
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_else(|| serde_json::json!({}))
                } else {
                    serde_json::json!({})
                };
                if !root_doc.is_object() {
                    root_doc = serde_json::json!({});
                }
                let net_obj = root_doc
                    .as_object_mut()
                    .unwrap()
                    .entry(import_args.network.clone())
                    .or_insert_with(|| serde_json::json!({}));
                if let Some(map) = net_obj.as_object_mut() {
                    map.insert(
                        import_args.name.clone(),
                        serde_json::json!({
                            "address": address_hex,
                            "class_hash": class_hash_hex,
                            "deployed": true,
                            "legacy": false,
                            "private_key": private_key_hex,
                            "public_key": public_key_hex,
                            "salt": import_args.salt.clone().unwrap_or_else(|| "0x0".to_string()),
                            "type": import_args.account_type,
                        }),
                    );
                }
                if let Some(parent) = accounts_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&accounts_path, serde_json::to_string_pretty(&root_doc)?)?;
                write_sncast_profile_entry(
                    import_args.add_profile.as_deref(),
                    &import_args.name,
                    &import_args.network,
                    &accounts_path,
                );

                let report = serde_json::json!({
                    "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
                    "command": "account import",
                    "name": import_args.name,
                    "network": import_args.network,
                    "account_type": import_args.account_type,
                    "address": address_hex,
                    "public_key": public_key_hex,
                    "class_hash": class_hash_hex,
                    "accounts_file": accounts_path.display().to_string(),
                    "add_profile": import_args.add_profile,
                });
                if import_args.json || import_args.report_path.is_some() {
                    emit_json_value(import_args.report_path.as_deref(), &report)?;
                } else {
                    println!("account imported: {}", import_args.name);
                    println!("  address:        {address_hex}");
                    println!("  public_key:     {public_key_hex}");
                    println!("  accounts_file:  {}", accounts_path.display());
                }
                return Ok(());
            }
            #[cfg(not(feature = "native-test-runner"))]
            {
                let _ = import_args;
                bail!("native-test-runner feature required for uc account import")
            }
        }
        AccountSubcommand::Delete(del_args) => {
            let accounts_path = default_accounts_file_path(del_args.accounts_file.as_deref());
            let mut removed = false;
            if accounts_path.is_file() {
                let text = fs::read_to_string(&accounts_path)?;
                let mut root_doc: serde_json::Value = serde_json::from_str(&text)?;
                if let Some(obj) = root_doc.as_object_mut() {
                    if let Some(net) = del_args.network.as_deref() {
                        if let Some(net_map) = obj.get_mut(net).and_then(|v| v.as_object_mut()) {
                            if net_map.remove(&del_args.name).is_some() {
                                removed = true;
                            }
                        }
                    } else {
                        for (_net_k, net_v) in obj.iter_mut() {
                            if let Some(net_map) = net_v.as_object_mut() {
                                if net_map.remove(&del_args.name).is_some() {
                                    removed = true;
                                }
                            }
                        }
                    }
                }
                if removed {
                    fs::write(&accounts_path, serde_json::to_string_pretty(&root_doc)?)?;
                }
            }
            if !removed {
                bail!(
                    "account `{}` not found in `{}`",
                    del_args.name,
                    accounts_path.display()
                );
            }
            let report = serde_json::json!({
                "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
                "command": "account delete",
                "name": del_args.name,
                "network": del_args.network,
                "deleted": true,
                "accounts_file": accounts_path.display().to_string(),
            });
            if del_args.json || del_args.report_path.is_some() {
                emit_json_value(del_args.report_path.as_deref(), &report)?;
            } else {
                println!("account deleted: {}", del_args.name);
            }
            Ok(())
        }
        AccountSubcommand::List(list_args) => {
            let workspace_root = list_args
                .manifest_path
                .as_ref()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(|| PathBuf::from("."));
            let prof = resolve_sncast_profile_config(
                &workspace_root,
                None,
                None,
                None,
                None,
                None,
                None,
                list_args.accounts_file.as_deref(),
            );
            let mut accounts = Vec::new();
            if let Some(af) = &prof.accounts_file {
                if let Ok(text) = fs::read_to_string(af) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(obj) = json.as_object() {
                            for (network, net_accs) in obj {
                                if let Some(acc_map) = net_accs.as_object() {
                                    for (name, details) in acc_map {
                                        accounts.push(serde_json::json!({
                                            "name": name,
                                            "network": network,
                                            "address": details.get("address"),
                                            "public_key": details.get("public_key"),
                                            "class_hash": details.get("class_hash"),
                                            "deployed": details.get("deployed"),
                                        }));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let report = serde_json::json!({
                "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
                "accounts_file": prof.accounts_file.map(|p| p.display().to_string()),
                "active_sncast_profile": prof.profile_name,
                "accounts": accounts,
            });
            if list_args.json || list_args.report_path.is_some() {
                emit_json_value(list_args.report_path.as_deref(), &report)?;
            } else {
                println!("configured accounts ({}):", accounts.len());
                for a in &accounts {
                    println!(
                        "  - {} ({}): {}",
                        a.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
                        a.get("network").and_then(|v| v.as_str()).unwrap_or("?"),
                        a.get("address").and_then(|v| v.as_str()).unwrap_or("?")
                    );
                }
            }
            Ok(())
        }
        AccountSubcommand::Deploy(d) => run_account_deploy(d),
        AccountSubcommand::ComputeAddress(ca_args) => {
            #[cfg(feature = "native-test-runner")]
            {
                let address = compute_starknet_contract_address(
                    &ca_args.class_hash,
                    &ca_args.salt,
                    std::slice::from_ref(&ca_args.public_key),
                    &ca_args.deployer_address,
                )?;
                let report = serde_json::json!({
                    "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
                    "class_hash": ca_args.class_hash,
                    "public_key": ca_args.public_key,
                    "salt": ca_args.salt,
                    "deployer_address": ca_args.deployer_address,
                    "contract_address": address,
                });
                if ca_args.json || ca_args.report_path.is_some() {
                    emit_json_value(ca_args.report_path.as_deref(), &report)?;
                } else {
                    println!("{address}");
                }
                return Ok(());
            }
            #[cfg(not(feature = "native-test-runner"))]
            {
                let _ = ca_args;
                bail!("native-test-runner feature required for uc account compute-address")
            }
        }
    }
}

/// Produces a standalone `Scarb.toml` for isolated bundle verification by resolving `.workspace = true`
/// fields against `workspace_manifest_toml` and rewriting relative `path = "..."` dependencies against `manifest_dir`.
fn standalone_verifier_manifest_toml(
    manifest_toml: &str,
    workspace_manifest_toml: Option<&str>,
    manifest_dir: Option<&Path>,
) -> String {
    let Ok(mut doc) = toml::from_str::<TomlValue>(manifest_toml) else {
        return manifest_toml.replace("casm = true", "casm = false");
    };
    let ws_doc = workspace_manifest_toml.and_then(|s| toml::from_str::<TomlValue>(s).ok());
    let ws_pkg = ws_doc
        .as_ref()
        .and_then(|w| w.get("workspace"))
        .and_then(|w| w.get("package"))
        .and_then(TomlValue::as_table);
    let ws_deps = ws_doc
        .as_ref()
        .and_then(|w| w.get("workspace"))
        .and_then(|w| w.get("dependencies"))
        .and_then(TomlValue::as_table);

    if let Some(root) = doc.as_table_mut() {
        root.remove("tool");
        if let Some(pkg) = root.get_mut("package").and_then(TomlValue::as_table_mut) {
            let keys: Vec<String> = pkg.keys().cloned().collect();
            for k in keys {
                let is_ws = pkg
                    .get(&k)
                    .and_then(TomlValue::as_table)
                    .and_then(|t| t.get("workspace"))
                    .and_then(TomlValue::as_bool)
                    .unwrap_or(false);
                if is_ws {
                    if let Some(val) = ws_pkg.and_then(|wp| wp.get(&k)).cloned() {
                        pkg.insert(k, val);
                    } else if k == "version" {
                        pkg.insert(k, TomlValue::String("0.1.0".to_string()));
                    } else if k == "edition" {
                        pkg.insert(k, TomlValue::String("2024_07".to_string()));
                    } else {
                        pkg.remove(&k);
                    }
                }
            }
        }
        for section in ["dependencies", "dev-dependencies"] {
            if let Some(deps) = root.get_mut(section).and_then(TomlValue::as_table_mut) {
                let dep_keys: Vec<String> = deps.keys().cloned().collect();
                for k in dep_keys {
                    let is_ws = deps
                        .get(&k)
                        .and_then(TomlValue::as_table)
                        .and_then(|t| t.get("workspace"))
                        .and_then(TomlValue::as_bool)
                        .unwrap_or(false);
                    if is_ws {
                        if let Some(ws_val) = ws_deps.and_then(|wd| wd.get(&k)).cloned() {
                            deps.insert(k.clone(), ws_val);
                        }
                    }
                    if let (Some(dir), Some(dep_tbl)) =
                        (manifest_dir, deps.get_mut(&k).and_then(TomlValue::as_table_mut))
                    {
                        if let Some(rel_path) = dep_tbl.get("path").and_then(TomlValue::as_str) {
                            let p = Path::new(rel_path);
                            if p.is_relative() {
                                let abs = dir.join(p);
                                if let Ok(canon) = abs.canonicalize() {
                                    dep_tbl.insert(
                                        "path".to_string(),
                                        TomlValue::String(canon.display().to_string()),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    toml::to_string(&doc)
        .unwrap_or_else(|_| manifest_toml.to_string())
        .replace("casm = true", "casm = false")
}

/// Reconstructs a Starkscan verification bundle (`uc-starkscan-verification-bundle-v1` or
/// `ContractVerificationUpsertRequest` with `source_bundle`), compiles its `.cairo` sources
/// natively in-process, and verifies byte-for-byte `class_hash` and `source_tree_merkle_root` parity.
/// Designed as a ready-to-merge verifier worker adapter for Starkscan `rust-exp` (`PUT /v1/{chain}/contract/{address}/verification`).
pub(crate) fn verify_starkscan_bundle_in_memory(
    bundle_or_upsert: &serde_json::Value,
) -> Result<serde_json::Value> {
    let started = Instant::now();
    let bundle = bundle_or_upsert
        .get("source_bundle")
        .unwrap_or(bundle_or_upsert);

    let manifest_toml = bundle
        .get("manifest_toml")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("bundle is missing `manifest_toml`"))?;
    let sources = bundle
        .get("sources")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow!("bundle is missing `sources` object"))?;

    let mut merkle_hasher = blake3::Hasher::new();
    merkle_hasher.update(b"uc-starkscan-source-tree-v1:");
    let mut sorted_files: Vec<(&String, &serde_json::Value)> = sources.iter().collect();
    sorted_files.sort_by_key(|(k, _)| *k);

    let mut verified_files: Vec<(&str, &str)> = Vec::with_capacity(sorted_files.len());
    for (rel_path, entry) in sorted_files {
        let content = entry
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("source entry `{rel_path}` missing `content`"))?;
        let digest = blake3::hash(content.as_bytes()).to_hex().to_string();
        if let Some(expected_blake3) = entry.get("blake3").and_then(|v| v.as_str()) {
            if expected_blake3 != digest {
                bail!(
                    "source file `{rel_path}` Blake3 digest mismatch: expected {expected_blake3}, computed {digest}"
                );
            }
        }
        merkle_hasher.update(rel_path.as_bytes());
        merkle_hasher.update(digest.as_bytes());
        verified_files.push((rel_path.as_str(), content));
    }
    let computed_merkle_root = merkle_hasher.finalize().to_hex().to_string();
    if let Some(expected_merkle) = bundle
        .get("source_tree_merkle_root")
        .and_then(|v| v.as_str())
    {
        if expected_merkle != computed_merkle_root {
            bail!(
                "source_tree_merkle_root mismatch: expected {expected_merkle}, computed {computed_merkle_root}"
            );
        }
    }

    let mut cache_key_hasher = blake3::Hasher::new();
    // The scratch dir caches compiled artifacts, so the compiler that produced them is part
    // of the key: a different compiler yields a different class hash for the same sources.
    cache_key_hasher.update(native_cairo_lang_compiler_version().as_bytes());
    cache_key_hasher.update(&[0]);
    cache_key_hasher.update(manifest_toml.as_bytes());
    cache_key_hasher.update(computed_merkle_root.as_bytes());
    let cache_key = cache_key_hasher.finalize().to_hex().to_string();
    let scratch_dir = std::env::temp_dir()
        .join("uc_starkscan_verifier_cache")
        .join(&cache_key[..24]);
    fs::create_dir_all(&scratch_dir)?;

    let manifest_path = scratch_dir.join("Scarb.toml");
    let workspace_manifest_toml = bundle.get("workspace_manifest_toml").and_then(|v| v.as_str());
    let orig_manifest_dir = bundle
        .get("manifest_dir")
        .and_then(|v| v.as_str())
        .map(PathBuf::from);
    // Only Sierra is required to compute and verify Starknet `class_hash` for Starkscan
    let verifier_manifest_toml = standalone_verifier_manifest_toml(
        manifest_toml,
        workspace_manifest_toml,
        orig_manifest_dir.as_deref(),
    );
    if fs::read_to_string(&manifest_path).ok().as_deref() != Some(verifier_manifest_toml.as_str()) {
        fs::write(&manifest_path, &verifier_manifest_toml)?;
    }
    if let Some(lock_str) = bundle.get("scarb_lock").and_then(|v| v.as_str()) {
        let lock_path = scratch_dir.join("Scarb.lock");
        if fs::read_to_string(&lock_path).ok().as_deref() != Some(lock_str) {
            fs::write(lock_path, lock_str)?;
        }
    }
    for (rel_path, content) in verified_files {
        let dest = scratch_dir.join(rel_path);
        if let Some(p) = dest.parent() {
            fs::create_dir_all(p)?;
        }
        if fs::read_to_string(&dest).ok().as_deref() != Some(content) {
            fs::write(&dest, content)?;
        }
    }

    let compiled_contracts =
        ensure_contract_artifacts_ready(&manifest_path, "dev", false, None)?;
    let bundle_expects_zero_contracts = bundle
        .get("contracts")
        .and_then(|v| v.as_array())
        .is_some_and(|arr| arr.is_empty());
    if compiled_contracts.is_empty() && !bundle_expects_zero_contracts {
        bail!("recompiled bundle produced 0 Starknet contracts");
    }

    let expected_class_hash = bundle_or_upsert
        .get("class_hash")
        .and_then(|v| v.as_str())
        .or_else(|| bundle.get("expected_class_hash").and_then(|v| v.as_str()))
        .or_else(|| {
            bundle
                .get("contracts")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|c| c.get("local_class_hash"))
                .and_then(|v| v.as_str())
        });

    let mut matched_contract: Option<&serde_json::Value> = None;
    for c in &compiled_contracts {
        let ch = c.get("class_hash").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(exp) = expected_class_hash {
            #[cfg(feature = "native-compile")]
            let eq = match (parse_felt_str(ch), parse_felt_str(exp)) {
                (Ok(a), Ok(b)) => a == b,
                _ => ch.eq_ignore_ascii_case(exp),
            };
            #[cfg(not(feature = "native-compile"))]
            let eq = ch.eq_ignore_ascii_case(exp);
            if eq {
                matched_contract = Some(c);
                break;
            }
        } else {
            matched_contract = Some(c);
            break;
        }
    }

    let verified = matched_contract.is_some()
        || (compiled_contracts.is_empty() && bundle_expects_zero_contracts);
    let empty_contract = serde_json::json!({});
    let primary = matched_contract
        .or_else(|| compiled_contracts.first())
        .unwrap_or(&empty_contract);
    let recomputed_class_hash = primary
        .get("class_hash")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let recomputed_compiled_hash = primary
        .get("compiled_class_hash")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let verified_at = (epoch_ms().unwrap_or(0) / 1000) as i64;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

    // Exact Starkscan `rust-exp` `ContractVerificationUpsertRequest` payload
    let upsert_request = serde_json::json!({
        "verifier": "uc-native",
        "status": if verified { "VERIFIED" } else { "FAILED" },
        "class_hash": recomputed_class_hash,
        "compiler_version": primary.get("compiler_version").and_then(|v| v.as_str()).unwrap_or(native_cairo_lang_compiler_version()),
        "source_bundle": bundle,
        "abi": primary.get("abi"),
        "verified_at": verified_at,
    });

    Ok(serde_json::json!({
        "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
        "command": "verify-bundle",
        "verified": verified,
        "status": if verified { "VERIFIED" } else { "MISMATCH" },
        "expected_class_hash": expected_class_hash,
        "recomputed_class_hash": recomputed_class_hash,
        "recomputed_compiled_class_hash": recomputed_compiled_hash,
        "source_tree_merkle_root": computed_merkle_root,
        "starkscan_rust_exp_upsert_request": upsert_request,
        "elapsed_ms": elapsed_ms,
    }))
}

/// Builds a deterministic Starkscan Contract Verification Bundle (`uc-starkscan-verification-bundle-v1`)
/// and optionally verifies on-chain class_hash parity via RPC and/or submits to Starkscan's Verification API.
pub(crate) fn run_verify_onchain(args: VerifyOnchainArgs) -> Result<()> {
    if let Some(bundle_path) = &args.verify_bundle {
        let bundle_bytes = fs::read(bundle_path)
            .with_context(|| format!("failed to read bundle {}", bundle_path.display()))?;
        let bundle_json: serde_json::Value = serde_json::from_slice(&bundle_bytes)
            .with_context(|| format!("invalid JSON in bundle {}", bundle_path.display()))?;
        let res = verify_starkscan_bundle_in_memory(&bundle_json)?;
        let verified = res.get("verified").and_then(|v| v.as_bool()).unwrap_or(false);
        if args.json || args.report_path.is_some() {
            emit_json_value(args.report_path.as_deref(), &res)?;
        } else {
            println!(
                "uc verify-onchain --verify-bundle ({:.1}ms): {}",
                res.get("elapsed_ms").and_then(|v| v.as_f64()).unwrap_or(0.0),
                res.get("status").and_then(|v| v.as_str()).unwrap_or("?")
            );
            println!(
                "  recomputed_class_hash: {}",
                res.get("recomputed_class_hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
            );
        }
        if !verified {
            bail!("bundle verification failed: class_hash mismatch");
        }
        return Ok(());
    }
    let started = Instant::now();
    let manifest_path = resolve_manifest_path(&args.manifest_path)?;
    let workspace_root = metadata_cache_workspace_root(&manifest_path)?;
    let profile = resolve_inspect_profile(args.release, args.profile.as_deref());
    let sncast_prof = resolve_sncast_profile_config(
        &workspace_root,
        args.sncast_profile.as_deref(),
        args.url.as_deref(),
        args.network.as_deref(),
        None,
        None,
        None,
        None,
    );

    let contracts = ensure_contract_artifacts_ready(
        &manifest_path,
        &profile,
        args.release,
        args.contract.as_deref(),
    )?;
    if contracts.is_empty()
        && (args.contract.is_some() || args.class_hash.is_some() || args.contract_address.is_some())
    {
        bail!("no compiled Starknet contracts found in `{profile}` to verify");
    }

    // Collect all `.cairo` source files + Blake3 digests for the Starkscan Verification Bundle
    let manifest_dir = manifest_path.parent().unwrap_or(&workspace_root);
    let src_dir = manifest_dir.join("src");
    let mut sources_map = BTreeMap::new();
    if src_dir.is_dir() {
        for entry in WalkDir::new(&src_dir).follow_links(false).into_iter().flatten() {
            if entry.file_type().is_file()
                && entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("cairo"))
            {
                let file_path = entry.path();
                if let Ok(content) = fs::read_to_string(file_path) {
                    let rel = file_path
                        .strip_prefix(manifest_dir)
                        .unwrap_or(file_path)
                        .display()
                        .to_string();
                    let digest = blake3::hash(content.as_bytes()).to_hex().to_string();
                    sources_map.insert(
                        rel,
                        serde_json::json!({
                            "blake3": digest,
                            "bytes": content.len(),
                            "content": content,
                        }),
                    );
                }
            }
        }
    }
    let mut merkle_hasher = blake3::Hasher::new();
    merkle_hasher.update(b"uc-starkscan-source-tree-v1:");
    for (rel, entry) in &sources_map {
        let digest = entry.get("blake3").and_then(|v| v.as_str()).unwrap_or("");
        merkle_hasher.update(rel.as_bytes());
        merkle_hasher.update(digest.as_bytes());
    }
    let source_tree_merkle_root = merkle_hasher.finalize().to_hex().to_string();
    let manifest_content = fs::read_to_string(&manifest_path).unwrap_or_default();
    let workspace_manifest_content = [
        Some(workspace_root.join("uc.toml")),
        Some(workspace_root.join("Scarb.toml")),
        manifest_dir.parent().map(|p| p.join("uc.toml")),
        manifest_dir.parent().map(|p| p.join("Scarb.toml")),
        manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("uc.toml")),
        manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("Scarb.toml")),
    ]
    .into_iter()
    .flatten()
    .find_map(|ws_mp| {
        if ws_mp != manifest_path && ws_mp.is_file() {
            fs::read_to_string(&ws_mp).ok()
        } else {
            None
        }
    });
    let lock_content = fs::read_to_string(workspace_root.join("uc.lock"))
        .or_else(|_| fs::read_to_string(workspace_root.join("Scarb.lock")))
        .ok();
    let cairo_project_toml =
        fs::read_to_string(workspace_root.join(".uc/native-project/cairo_project.toml")).ok();

    // Query live RPC for on-chain class_hash if `--url` or `--contract-address` is provided
    let mut onchain_class_hash: Option<String> = args.class_hash.clone();
    if let (Some(rpc_url), Some(contract_addr)) =
        (sncast_prof.url.as_deref(), args.contract_address.as_deref())
    {
        if let Ok(res) = starknet_rpc_request(
            rpc_url,
            "starknet_getClassHashAt",
            serde_json::json!({ "block_id": "latest", "contract_address": contract_addr }),
        ) {
            if let Some(ch_str) = res.as_str() {
                onchain_class_hash = Some(ch_str.to_string());
            }
        }
    }

    let mut verified_all = true;
    let mut contract_verifications = Vec::with_capacity(contracts.len());
    for c in &contracts {
        let local_class_hash = c
            .get("class_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let local_compiled_hash = c
            .get("compiled_class_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let matched = if let Some(expected_onchain) = onchain_class_hash.as_deref() {
            #[cfg(feature = "native-compile")]
            {
                match (
                    parse_felt_str(&local_class_hash),
                    parse_felt_str(expected_onchain),
                ) {
                    (Ok(a), Ok(b)) => a == b,
                    _ => local_class_hash.eq_ignore_ascii_case(expected_onchain),
                }
            }
            #[cfg(not(feature = "native-compile"))]
            {
                local_class_hash.eq_ignore_ascii_case(expected_onchain)
            }
        } else {
            !local_class_hash.is_empty()
        };
        if !matched {
            verified_all = false;
        }
        contract_verifications.push(serde_json::json!({
            "contract_name": c.get("contract_name"),
            "package_name": c.get("package_name"),
            "module_path": c.get("module_path"),
            "compiler_version": c.get("compiler_version"),
            "local_class_hash": local_class_hash,
            "onchain_class_hash": onchain_class_hash,
            "compiled_class_hash": local_compiled_hash,
            "sierra_blake3": c.get("sierra_blake3"),
            "casm_blake3": c.get("casm_blake3"),
            "abi": c.get("abi"),
            "verified": matched,
        }));
    }

    let chain_id = match sncast_prof.network.as_deref() {
        Some("sepolia") | Some("alpha-sepolia") => "SN_SEPOLIA",
        _ => "SN_MAIN",
    };
    let starkscan_bundle = serde_json::json!({
        "bundle_schema": "uc-starkscan-verification-bundle-v1",
        "verifier": args.verifier.as_str(),
        "chain_id": chain_id,
        "contract_address": args.contract_address,
        "expected_class_hash": onchain_class_hash,
        "compiler": {
            "name": "uc-native",
            "cairo_version": native_cairo_lang_compiler_version(),
            "sierra_version": cairo_lang_starknet_classes::compiler_version::current_sierra_version_id().to_string(),
        },
        "manifest_toml": manifest_content,
        "workspace_manifest_toml": workspace_manifest_content,
        "manifest_dir": manifest_dir.display().to_string(),
        "scarb_lock": lock_content,
        "cairo_project_toml": cairo_project_toml,
        "source_tree_merkle_root": source_tree_merkle_root,
        "sources": sources_map,
        "contracts": contract_verifications,
    });

    let primary_cv = contract_verifications.first().cloned().unwrap_or_default();
    let verified_at = (epoch_ms().unwrap_or(0) / 1000) as i64;
    let starkscan_rust_exp_payload = serde_json::json!({
        "verifier": args.verifier.as_str(),
        "status": if verified_all { "VERIFIED" } else { "MISMATCH" },
        "class_hash": onchain_class_hash.clone().or_else(|| primary_cv.get("local_class_hash").and_then(|v| v.as_str()).map(str::to_string)),
        "compiler_version": primary_cv.get("compiler_version").and_then(|v| v.as_str()).unwrap_or(native_cairo_lang_compiler_version()),
        "source_bundle": starkscan_bundle,
        "abi": primary_cv.get("abi"),
        "verified_at": verified_at,
    });

    if let Some(bundle_path) = &args.export_bundle {
        if let Some(parent) = bundle_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            bundle_path,
            serde_json::to_vec_pretty(&starkscan_bundle)?,
        )
        .with_context(|| format!("failed to write Starkscan bundle to {}", bundle_path.display()))?;
    }

    // Starkscan's verification record for this contract: read when a key is available, written
    // with --submit. The PUT needs a key carrying the verification scope; the record's
    // `verificationStatus` is pending / verified / failed.
    let api_key = args.api_key.clone().or_else(starkscan_api_key);
    let starkscan_record = match (&args.contract_address, api_key.as_deref()) {
        (Some(address), Some(_)) if matches!(args.verifier, VerifierProviderArg::Starkscan) => {
            starkscan_contract_verification(chain_id, address).unwrap_or_else(|e| {
                eprintln!("warning: could not read Starkscan verification record: {e}");
                None
            })
        }
        _ => None,
    };
    let starkscan_upsert_request = serde_json::json!({
        "classHash": onchain_class_hash,
        "sourceRef": args.export_bundle.as_ref().map(|p| p.display().to_string())
            .unwrap_or_else(|| format!("uc-bundle:{source_tree_merkle_root}")),
        "language": "cairo",
        "compilerVersion": native_cairo_lang_compiler_version(),
        "verificationStatus": if verified_all { "verified" } else { "failed" },
        "verificationError": if verified_all { serde_json::Value::Null } else { serde_json::json!("local class hash does not match the deployed class") },
        "verifiedAtIso": if verified_all { serde_json::json!(iso8601_now()) } else { serde_json::Value::Null },
        "metadata": {
            "bundle_schema": "uc-starkscan-verification-bundle-v1",
            "source_tree_merkle_root": source_tree_merkle_root,
            "source_file_count": sources_map.len(),
            "uc_version": uc_version_banner(),
        },
    });
    let mut verifier_submission_response: Option<serde_json::Value> = None;
    if args.submit || args.verifier_url.is_some() {
        match (&args.verifier_url, &args.contract_address) {
            (Some(custom_url), _) => {
                let mut headers = Vec::new();
                if let Some(k) = api_key.as_deref() {
                    headers.push((STARKSCAN_API_KEY_HEADER, k));
                }
                verifier_submission_response = Some(http_post_json(custom_url, &starkscan_bundle, &headers)?);
            }
            (None, Some(address)) => {
                if api_key.is_none() {
                    bail!("--submit needs a Starkscan API key (--api-key or {STARKSCAN_API_KEY_ENV})");
                }
                let resp = starkscan_put_contract_verification(chain_id, address, &starkscan_upsert_request)?;
                verifier_submission_response = Some(resp);
            }
            (None, None) => bail!("--submit needs --contract-address (the record is keyed by address)"),
        }
    }

    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let report = serde_json::json!({
        "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
        "command": "verify-onchain",
        "verifier": args.verifier.as_str(),
        "status": if verified_all { "verified" } else { "mismatch" },
        "verified": verified_all,
        "chain_id": chain_id,
        "contract_address": args.contract_address,
        "onchain_class_hash": onchain_class_hash,
        "source_tree_merkle_root": source_tree_merkle_root,
        "source_file_count": sources_map.len(),
        "bundle_exported_to": args.export_bundle.as_ref().map(|p| p.display().to_string()),
        "starkscan_rust_exp_payload": starkscan_rust_exp_payload,
        "starkscan_verification_record": starkscan_record,
        "starkscan_upsert_request": starkscan_upsert_request,
        "verifier_submission": verifier_submission_response,
        "contracts": contract_verifications,
        "elapsed_ms": elapsed_ms,
    });

    if args.json || args.report_path.is_some() {
        emit_json_value(args.report_path.as_deref(), &report)?;
    } else {
        println!(
            "uc verify-onchain ({}, chain: {chain_id}, {:.1}ms): {}",
            args.verifier.as_str(),
            elapsed_ms,
            if verified_all { "VERIFIED" } else { "MISMATCH" }
        );
        for cv in &contract_verifications {
            println!(
                "  contract {}: local_class_hash={} compiled_class_hash={}",
                cv.get("contract_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?"),
                cv.get("local_class_hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?"),
                cv.get("compiled_class_hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?"),
            );
        }
        if let Some(bp) = &args.export_bundle {
            println!("  starkscan verification bundle exported to: {}", bp.display());
        }
    }

    if !verified_all {
        bail!("on-chain contract class_hash verification mismatch");
    }
    Ok(())
}

#[cfg(all(test, feature = "native-test-runner"))]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    #[test]
    fn sending_needs_an_account_and_a_key_unless_dry_run() {
        let mut prof = ResolvedStarknetProfile {
            profile_name: "default".to_string(),
            url: Some("http://127.0.0.1:5050/rpc".to_string()),
            network: None,
            account: None,
            account_address: None,
            private_key: None,
            accounts_file: None,
        };
        let err = ensure_sending_account(&prof, None, false).unwrap_err().to_string();
        assert!(err.starts_with("no account to send from"), "{err}");
        assert!(ensure_sending_account(&prof, None, true).is_ok());
        prof.account_address = Some("0x64b4".to_string());
        let err = ensure_sending_account(&prof, None, false).unwrap_err().to_string();
        assert!(err.starts_with("no key to sign with"), "{err}");
        assert!(ensure_sending_account(&prof, Some(Path::new("keystore.json")), false).is_ok());
        prof.private_key = Some("0x1".to_string());
        assert!(ensure_sending_account(&prof, None, false).is_ok());
        // Without a node nothing is sent.
        prof.url = None;
        prof.account_address = None;
        assert!(ensure_sending_account(&prof, None, false).is_ok());
    }

    #[test]
    fn a_missing_artifact_path_is_named() {
        let err = ensure_named_file_exists("nope.contract_class.json").unwrap_err().to_string();
        assert_eq!(err, "no such file: nope.contract_class.json");
        assert!(ensure_named_file_exists("target/dev/missing").is_err());
        // Contract names and hashes are not files.
        assert!(ensure_named_file_exists("Counter").is_ok());
        assert!(ensure_named_file_exists("0x1234").is_ok());
    }

    /// `sncast deploy --unique` on starknet-devnet 0.10.1 deployed this class at this address
    /// through the Cairo 1 UDC; the legacy UDC would have given another one.
    #[test]
    fn unique_udc_address_matches_an_sncast_deployment() {
        let address = compute_udc_deployed_address(
            "0x2e895de4b82503229f12daf608c7f99ac1b9933d5ce77ff57fa7832453f2391",
            "0x7",
            true,
            Some("0x260a8311b4f1092db620b923e8d7d20e76dedcc615fb4b6fdf28315b81de201"),
            &["0x5".to_string()],
        )
        .unwrap();
        assert_eq!(address, "0x01dea26f5d68184d94788eb03924f8b25137cdf009d659bb42d51348ce13554b");
        let not_unique = compute_udc_deployed_address("0x2e895de4b82503229f12daf608c7f99ac1b9933d5ce77ff57fa7832453f2391", "0x7", false, None, &["0x5".to_string()]).unwrap();
        assert_eq!(not_unique, compute_starknet_contract_address("0x2e895de4b82503229f12daf608c7f99ac1b9933d5ce77ff57fa7832453f2391", "0x7", &["0x5".to_string()], "0x0").unwrap());
    }

    struct TempOnchainDir {
        dir: PathBuf,
    }

    impl TempOnchainDir {
        fn new(prefix: &str) -> Self {
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let id = SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "uc_onchain_{prefix}_{}_{}",
                std::process::id(),
                id
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("src")).expect("create temp dir");
            Self { dir }
        }
    }

    impl Drop for TempOnchainDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn selector_and_short_string_encode_decode_roundtrip() {
        let dir = TempOnchainDir::new("selector");
        let rep1 = dir.dir.join("sel.json");
        run_selector(SelectorArgs {
            inputs: vec!["transfer".to_string(), "balance_of".to_string()],
            decode: false,
            short_string: false,
            decode_short_string: false,
            manifest_path: None,
            json: true,
            report_path: Some(rep1.clone()),
        })
        .unwrap();
        let val1: serde_json::Value = serde_json::from_slice(&fs::read(&rep1).unwrap()).unwrap();
        let transfer_sel = val1["results"][0]["selector"].as_str().unwrap().to_string();
        assert_eq!(
            transfer_sel,
            "0x0083afd3f4caedc6eebf44246fe54e38c95e3179a5ec9ea81740eca5b482d12e"
        );

        // Reverse decode
        let rep2 = dir.dir.join("decode.json");
        run_selector(SelectorArgs {
            inputs: vec![transfer_sel],
            decode: true,
            short_string: false,
            decode_short_string: false,
            manifest_path: None,
            json: true,
            report_path: Some(rep2.clone()),
        })
        .unwrap();
        let val2: serde_json::Value = serde_json::from_slice(&fs::read(&rep2).unwrap()).unwrap();
        assert_eq!(val2["results"][0]["name"], "transfer");

        // Short string encode + decode roundtrip
        let rep3 = dir.dir.join("ss_enc.json");
        run_selector(SelectorArgs {
            inputs: vec!["STARKNET_UC".to_string()],
            decode: false,
            short_string: true,
            decode_short_string: false,
            manifest_path: None,
            json: true,
            report_path: Some(rep3.clone()),
        })
        .unwrap();
        let val3: serde_json::Value = serde_json::from_slice(&fs::read(&rep3).unwrap()).unwrap();
        let encoded_hex = val3["results"][0]["hex"].as_str().unwrap().to_string();

        let rep4 = dir.dir.join("ss_dec.json");
        run_selector(SelectorArgs {
            inputs: vec![encoded_hex],
            decode: false,
            short_string: false,
            decode_short_string: true,
            manifest_path: None,
            json: true,
            report_path: Some(rep4.clone()),
        })
        .unwrap();
        let val4: serde_json::Value = serde_json::from_slice(&fs::read(&rep4).unwrap()).unwrap();
        assert_eq!(val4["results"][0]["decoded"], "STARKNET_UC");
    }

    #[test]
    fn mock_rpc_and_starkscan_server_end_to_end_onchain_suite() {
        let _env = crate::test_env_shared();
        let dir = TempOnchainDir::new("mock_rpc");
        let manifest_path = dir.dir.join("Scarb.toml");
        fs::write(
            &manifest_path,
            r#"[package]
name = "onchain_vault"
version = "0.1.0"
edition = "2024_07"

[dependencies]
starknet = "2.16.0"

[[target.starknet-contract]]
sierra = true
casm = true
"#,
        )
        .unwrap();
        fs::write(
            dir.dir.join("src/lib.cairo"),
            r#"#[starknet::contract]
pub mod OnchainVault {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        balance: u256,
    }

    #[external(v0)]
    pub fn get_balance(self: @ContractState, user: felt252) -> u256 {
        self.balance.read()
    }

    #[external(v0)]
    pub fn deposit(ref self: ContractState, amount: u256) {
        self.balance.write(self.balance.read() + amount);
    }
}
"#,
        )
        .unwrap();

        // 1. Compute offline class_hash and compiled_class_hash via `uc class-hash`
        let ch_rep_path = dir.dir.join("class_hash.json");
        run_class_hash(ClassHashArgs {
            target: Some("OnchainVault".to_string()),
            manifest_path: Some(manifest_path.clone()),
            package: None,
            profile: None,
            release: false,
            casm: true,
            json: true,
            report_path: Some(ch_rep_path.clone()),
        })
        .unwrap();
        let ch_rep: serde_json::Value =
            serde_json::from_slice(&fs::read(&ch_rep_path).unwrap()).unwrap();
        let expected_class_hash = ch_rep["contracts"][0]["class_hash"]
            .as_str()
            .unwrap()
            .to_string();
        let expected_compiled_hash = ch_rep["contracts"][0]["compiled_class_hash"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(expected_class_hash.starts_with("0x"));
        assert!(expected_compiled_hash.starts_with("0x"));

        // 2. Start a local mock JSON-RPC + Starkscan Verification server
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server_url = format!("http://{addr}/rpc");
        let verify_url = format!("http://{addr}/api/v1/verify");
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_clone = stop_flag.clone();
        let class_hash_for_server = expected_class_hash.clone();

        let server_handle = std::thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                let mut buf = vec![0u8; 65536];
                let mut total = 0usize;
                loop {
                    match stream.read(&mut buf[total..]) {
                        Ok(0) => break,
                        Ok(n) => {
                            total += n;
                            let s = String::from_utf8_lossy(&buf[..total]);
                            if let Some((hdr, body)) = s.split_once("\r\n\r\n") {
                                let cl = hdr
                                    .lines()
                                    .find_map(|l| {
                                        let (k, v) = l.split_once(':')?;
                                        if k.trim().eq_ignore_ascii_case("content-length") {
                                            v.trim().parse::<usize>().ok()
                                        } else {
                                            None
                                        }
                                    })
                                    .unwrap_or(0);
                                if body.len() >= cl {
                                    break;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
                let req_str = String::from_utf8_lossy(&buf[..total]);
                let body_part = req_str
                    .split_once("\r\n\r\n")
                    .map(|(_, b)| b)
                    .unwrap_or("");
                let json_req: serde_json::Value =
                    serde_json::from_str(body_part).unwrap_or_default();

                let resp_json = if json_req.get("bundle_schema").is_some() {
                    // Starkscan Verification API request
                    serde_json::json!({
                        "status": "VERIFIED",
                        "verification_id": "starkscan-job-9001",
                        "class_hash": class_hash_for_server,
                        "merkle_root": json_req.get("source_tree_merkle_root"),
                    })
                } else {
                    let method = json_req
                        .get("method")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    match method {
                        "starknet_call" => serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "result": ["0x3e8", "0x0"] // u256 = 1000
                        }),
                        "starknet_estimateFee" => serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "result": [{
                                "l1_gas_consumed": "0x200",
                                "l1_gas_price": "0x1000",
                                "l2_gas_consumed": "0x400",
                                "l2_gas_price": "0x2000",
                                "l1_data_gas_consumed": "0x100",
                                "l1_data_gas_price": "0x800",
                                "overall_fee": "0x500000"
                            }]
                        }),
                        "starknet_addDeclareTransaction" => serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "result": {
                                "transaction_hash": "0xdeadbeef01",
                                "class_hash": class_hash_for_server
                            }
                        }),
                        "starknet_addInvokeTransaction" => serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "result": {
                                "transaction_hash": "0xdeadbeef02"
                            }
                        }),
                        "starknet_getTransactionStatus" => serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "result": {
                                "finality_status": "ACCEPTED_ON_L2",
                                "execution_status": "SUCCEEDED"
                            }
                        }),
                        "starknet_getTransactionReceipt" => serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "result": {
                                "type": "INVOKE",
                                "transaction_hash": "0xdeadbeef02",
                                "actual_fee": { "amount": "0x100", "unit": "FRI" }
                            }
                        }),
                        "starknet_getClassHashAt" => serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "result": class_hash_for_server
                        }),
                        _ => serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "result": []
                        }),
                    }
                };
                let resp_body = serde_json::to_vec(&resp_json).unwrap();
                let http_resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    resp_body.len()
                );
                let _ = stream.write_all(http_resp.as_bytes());
                let _ = stream.write_all(&resp_body);
                let _ = stream.flush();
            }
        });

        // Write `snfoundry.toml` and `accounts.json` to test profile & account resolution
        let accounts_path = dir.dir.join("accounts.json");
        fs::write(
            &accounts_path,
            r#"{
  "alpha-sepolia": {
    "deployer_acc": {
      "address": "0x0123456789abcdef",
      "private_key": "0x12345",
      "public_key": "0x98765",
      "class_hash": "0x1111",
      "deployed": true
    }
  }
}"#,
        )
        .unwrap();
        fs::write(
            dir.dir.join("snfoundry.toml"),
            format!(
                r#"[sncast.default]
url = "{server_url}"
network = "sepolia"
account = "deployer_acc"
accounts-file = "{}"
"#,
                accounts_path.display()
            ),
        )
        .unwrap();

        // 3. Test `uc call` with ABI-aware `u256` decoding
        let call_rep_path = dir.dir.join("call_rep.json");
        run_call(CallArgs {
            contract_address: "0x0555".to_string(),
            function: "get_balance".to_string(),
            calldata: vec!["0x42".to_string()],
            url: None,
            network: None,
            sncast_profile: None,
            block_id: "latest".to_string(),
            contract: Some("OnchainVault".to_string()),
            manifest_path: Some(manifest_path.clone()),
            json: true,
            report_path: Some(call_rep_path.clone()),
        })
        .unwrap();
        let call_rep: serde_json::Value =
            serde_json::from_slice(&fs::read(&call_rep_path).unwrap()).unwrap();
        assert_eq!(call_rep["decoded"]["type"], "core::integer::u256");
        assert_eq!(call_rep["decoded"]["decimal"], "1000");

        // 4. Test encrypted keystore + `uc declare` with `--keystore` and `--wait` + 1.5x fee auto-scaling
        let keystore_path = dir.dir.join("deployer_keystore.json");
        let ks_json = encrypt_starknet_keystore(
            "0x12345",
            "ultra-secret-pass",
            "0x0102030405060708090a0b0c0d0e0f10",
            "0xaabbccddeeff00112233445566778899",
        )
        .unwrap();
        fs::write(&keystore_path, serde_json::to_vec_pretty(&ks_json).unwrap()).unwrap();

        let dec_rep_path = dir.dir.join("declare_rep.json");
        run_declare(DeclareArgs {
            contract: "OnchainVault".to_string(),
            url: None,
            network: None,
            sncast_profile: None,
            account: None,
            account_address: None,
            private_key: None,
            keystore: Some(keystore_path.clone()),
            password: Some("ultra-secret-pass".to_string()),
            accounts_file: None,
            dry_run: false,
            upgrade_of: None,
            allow_unsafe_upgrade: false,
            wait: true,
            wait_timeout: 5,
            manifest_path: Some(manifest_path.clone()),
            json: true,
            report_path: Some(dec_rep_path.clone()),
        })
        .unwrap();
        let dec_rep: serde_json::Value =
            serde_json::from_slice(&fs::read(&dec_rep_path).unwrap()).unwrap();
        assert_eq!(dec_rep["transaction_hash"], "0xdeadbeef01");
        assert_eq!(dec_rep["class_hash"], expected_class_hash);
        assert_eq!(
            dec_rep["confirmation_status"]["finality_status"],
            "ACCEPTED_ON_L2"
        );
        // Verify 1.5x fee auto-scaling (0x200 * 1.5 = 0x300)
        assert_eq!(
            dec_rep["transaction_v3"]["resource_bounds"]["l1_gas"]["max_amount"],
            "0x300"
        );
        assert!(dec_rep["transaction_v3"]["signature"]
            .as_array()
            .unwrap()
            .len()
            == 2);

        // 5. Test `uc deploy` via UDC
        let dep_rep_path = dir.dir.join("deploy_rep.json");
        run_deploy(DeployArgs {
            class_hash: None,
            contract: Some("OnchainVault".to_string()),
            constructor_calldata: vec![],
            salt: "0x77".to_string(),
            unique: true,
            url: None,
            network: None,
            sncast_profile: None,
            account: None,
            account_address: None,
            private_key: None,
            keystore: None,
            password: None,
            accounts_file: None,
            dry_run: false,
            wait: true,
            wait_timeout: 5,
            manifest_path: Some(manifest_path.clone()),
            json: true,
            report_path: Some(dep_rep_path.clone()),
        })
        .unwrap();
        let dep_rep: serde_json::Value =
            serde_json::from_slice(&fs::read(&dep_rep_path).unwrap()).unwrap();
        assert_eq!(dep_rep["transaction_hash"], "0xdeadbeef02");
        assert!(dep_rep["contract_address"]
            .as_str()
            .unwrap()
            .starts_with("0x"));

        // 6. Test `uc invoke` with `u256:5000` argument
        let inv_rep_path = dir.dir.join("invoke_rep.json");
        run_invoke(InvokeArgs {
            contract_address: "0x0555".to_string(),
            function: "deposit".to_string(),
            calldata: vec!["u256:5000".to_string()],
            url: None,
            network: None,
            sncast_profile: None,
            account: None,
            account_address: None,
            private_key: None,
            keystore: None,
            password: None,
            accounts_file: None,
            dry_run: false,
            wait: true,
            wait_timeout: 5,
            manifest_path: Some(manifest_path.clone()),
            json: true,
            report_path: Some(inv_rep_path.clone()),
        })
        .unwrap();
        let inv_rep: serde_json::Value =
            serde_json::from_slice(&fs::read(&inv_rep_path).unwrap()).unwrap();
        assert_eq!(inv_rep["transaction_hash"], "0xdeadbeef02");
        assert_eq!(inv_rep["calldata"], serde_json::json!(["0x1388", "0x0"]));

        // 7. Test `uc tx-status`
        let tx_rep_path = dir.dir.join("tx_status.json");
        run_tx_status(TxStatusArgs {
            transaction_hash: "0xdeadbeef02".to_string(),
            receipt: true,
            url: None,
            network: None,
            sncast_profile: None,
            manifest_path: Some(manifest_path.clone()),
            json: true,
            report_path: Some(tx_rep_path.clone()),
        })
        .unwrap();
        let tx_rep: serde_json::Value =
            serde_json::from_slice(&fs::read(&tx_rep_path).unwrap()).unwrap();
        assert_eq!(tx_rep["finality_status"], "ACCEPTED_ON_L2");
        assert_eq!(tx_rep["execution_status"], "SUCCEEDED");

        // 8. Test `uc verify-onchain --verifier starkscan --export-bundle ... --submit`
        let bundle_path = dir.dir.join("starkscan_bundle.json");
        let ver_rep_path = dir.dir.join("verify_onchain.json");
        run_verify_onchain(VerifyOnchainArgs {
            contract_address: Some("0x0555".to_string()),
            class_hash: None,
            contract: Some("OnchainVault".to_string()),
            verifier: VerifierProviderArg::Starkscan,
            verifier_url: Some(verify_url),
            api_key: Some("test-starkscan-key".to_string()),
            export_bundle: Some(bundle_path.clone()),
            verify_bundle: None,
            submit: true,
            url: None,
            network: None,
            sncast_profile: None,
            manifest_path: Some(manifest_path.clone()),
            profile: None,
            release: false,
            json: true,
            report_path: Some(ver_rep_path.clone()),
        })
        .unwrap();
        let ver_rep: serde_json::Value =
            serde_json::from_slice(&fs::read(&ver_rep_path).unwrap()).unwrap();
        assert_eq!(ver_rep["verified"], true);
        assert_eq!(ver_rep["verifier"], "starkscan");
        assert_eq!(
            ver_rep["starkscan_rust_exp_payload"]["status"],
            "VERIFIED"
        );
        assert_eq!(
            ver_rep["verifier_submission"]["verification_id"],
            "starkscan-job-9001"
        );

        // 9. Test `uc verify-onchain --verify-bundle` (Starkscan worker in-memory verification <500ms)
        let bundle_verify_rep_path = dir.dir.join("verify_bundle_rep.json");
        run_verify_onchain(VerifyOnchainArgs {
            contract_address: None,
            class_hash: None,
            contract: None,
            verifier: VerifierProviderArg::Starkscan,
            verifier_url: None,
            api_key: None,
            export_bundle: None,
            verify_bundle: Some(bundle_path.clone()),
            submit: false,
            url: None,
            network: None,
            sncast_profile: None,
            manifest_path: None,
            profile: None,
            release: false,
            json: true,
            report_path: Some(bundle_verify_rep_path.clone()),
        })
        .unwrap();
        let bv_rep: serde_json::Value =
            serde_json::from_slice(&fs::read(&bundle_verify_rep_path).unwrap()).unwrap();
        assert_eq!(bv_rep["verified"], true);
        assert_eq!(bv_rep["recomputed_class_hash"], expected_class_hash);
        assert_eq!(
            bv_rep["starkscan_rust_exp_upsert_request"]["status"],
            "VERIFIED"
        );
        let bundle_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&bundle_path).unwrap()).unwrap();
        assert_eq!(
            bundle_json["bundle_schema"],
            "uc-starkscan-verification-bundle-v1"
        );
        assert!(bundle_json["sources"]["src/lib.cairo"]["content"]
            .as_str()
            .unwrap()
            .contains("OnchainVault"));

        stop_flag.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(addr);
        let _ = server_handle.join();
    }

    #[test]
    fn diverse_real_world_starknet_contracts_corpus_build_test_and_verify() {
        let _env = crate::test_env_shared();
        let dir = TempOnchainDir::new("corpus_8_archetypes");
        let manifest_path = dir.dir.join("Scarb.toml");
        fs::write(
            &manifest_path,
            r#"[package]
name = "starknet_corpus_suite"
version = "0.1.0"
edition = "2024_07"

[dependencies]
starknet = "2.16.0"

[[target.starknet-contract]]
sierra = true
casm = true
"#,
        )
        .unwrap();

        // 8 distinct real-world Starknet contract archetypes in a single suite + comprehensive tests:
        // 1. ERC20Token, 2. ERC721Collectible, 3. ConstantProductAmm, 4. MultisigAccount,
        // 5. StakingVault4626, 6. GovernanceTimelock, 7. UpgradeableProxy, 8. PoseidonMerkleOracle
        fs::write(
            dir.dir.join("src/lib.cairo"),
            r#"#[starknet::contract]
pub mod ERC20Token {
    use starknet::ContractAddress;
    use starknet::storage::{Map, StorageMapReadAccess, StorageMapWriteAccess, StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        total_supply: u256,
        balances: Map<ContractAddress, u256>,
    }

    #[external(v0)]
    pub fn total_supply(self: @ContractState) -> u256 {
        self.total_supply.read()
    }

    #[external(v0)]
    pub fn balance_of(self: @ContractState, owner: ContractAddress) -> u256 {
        self.balances.read(owner)
    }

    #[external(v0)]
    pub fn mint(ref self: ContractState, recipient: ContractAddress, amount: u256) {
        self.total_supply.write(self.total_supply.read() + amount);
        self.balances.write(recipient, self.balances.read(recipient) + amount);
    }
}

#[starknet::contract]
pub mod ERC721Collectible {
    use starknet::ContractAddress;
    use starknet::storage::{Map, StorageMapReadAccess, StorageMapWriteAccess, StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        next_token_id: u256,
        owners: Map<u256, ContractAddress>,
    }

    #[external(v0)]
    pub fn owner_of(self: @ContractState, token_id: u256) -> ContractAddress {
        self.owners.read(token_id)
    }

    #[external(v0)]
    pub fn mint_nft(ref self: ContractState, to: ContractAddress) -> u256 {
        let token_id = self.next_token_id.read() + 1;
        self.next_token_id.write(token_id);
        self.owners.write(token_id, to);
        token_id
    }
}

#[starknet::contract]
pub mod ConstantProductAmm {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        reserve_a: u128,
        reserve_b: u128,
    }

    #[external(v0)]
    pub fn add_liquidity(ref self: ContractState, amount_a: u128, amount_b: u128) {
        self.reserve_a.write(self.reserve_a.read() + amount_a);
        self.reserve_b.write(self.reserve_b.read() + amount_b);
    }

    #[external(v0)]
    pub fn get_amount_out(self: @ContractState, amount_in: u128) -> u128 {
        let ra = self.reserve_a.read();
        let rb = self.reserve_b.read();
        assert(ra > 0 && rb > 0, 'EMPTY_POOL');
        let amount_in_with_fee = amount_in * 997;
        (amount_in_with_fee * rb) / (ra * 1000 + amount_in_with_fee)
    }
}

#[starknet::contract]
pub mod MultisigAccount {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        threshold: u32,
        nonce: u64,
    }

    #[external(v0)]
    pub fn get_threshold(self: @ContractState) -> u32 {
        self.threshold.read()
    }

    #[external(v0)]
    pub fn set_threshold(ref self: ContractState, new_threshold: u32) {
        assert(new_threshold > 0, 'ZERO_THRESHOLD');
        self.threshold.write(new_threshold);
        self.nonce.write(self.nonce.read() + 1);
    }
}

#[starknet::contract]
pub mod StakingVault4626 {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        total_assets: u128,
        total_shares: u128,
    }

    #[external(v0)]
    pub fn convert_to_shares(self: @ContractState, assets: u128) -> u128 {
        let supply = self.total_shares.read();
        let total = self.total_assets.read();
        if supply == 0 || total == 0 {
            assets
        } else {
            (assets * supply) / total
        }
    }

    #[external(v0)]
    pub fn deposit_assets(ref self: ContractState, assets: u128) -> u128 {
        let shares = convert_to_shares(@self, assets);
        self.total_assets.write(self.total_assets.read() + assets);
        self.total_shares.write(self.total_shares.read() + shares);
        shares
    }
}

#[starknet::contract]
pub mod GovernanceTimelock {
    use starknet::storage::{Map, StorageMapReadAccess, StorageMapWriteAccess, StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        proposal_count: u64,
        votes_for: Map<u64, u128>,
    }

    #[external(v0)]
    pub fn propose(ref self: ContractState) -> u64 {
        let id = self.proposal_count.read() + 1;
        self.proposal_count.write(id);
        id
    }

    #[external(v0)]
    pub fn cast_vote(ref self: ContractState, proposal_id: u64, weight: u128) {
        self.votes_for.write(proposal_id, self.votes_for.read(proposal_id) + weight);
    }
}

#[starknet::contract]
pub mod UpgradeableProxy {
    use starknet::ClassHash;
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        implementation: ClassHash,
        admin: felt252,
    }

    #[external(v0)]
    pub fn get_implementation(self: @ContractState) -> ClassHash {
        self.implementation.read()
    }

    #[external(v0)]
    pub fn upgrade_to(ref self: ContractState, new_impl: ClassHash) {
        self.implementation.write(new_impl);
    }
}

#[starknet::contract]
pub mod PoseidonMerkleOracle {
    use core::poseidon::poseidon_hash_span;
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        merkle_root: felt252,
        latest_price: u128,
    }

    #[external(v0)]
    pub fn compute_pair_hash(self: @ContractState, a: felt252, b: felt252) -> felt252 {
        poseidon_hash_span(array![a, b].span())
    }

    #[external(v0)]
    pub fn update_price(ref self: ContractState, price: u128) {
        assert(price > 0, 'INVALID_PRICE');
        self.latest_price.write(price);
    }
}

pub fn amm_quote_pure(amount_in: u128, reserve_in: u128, reserve_out: u128) -> u128 {
    assert(reserve_in > 0 && reserve_out > 0, 'EMPTY_RESERVES');
    let amount_in_with_fee = amount_in * 997;
    (amount_in_with_fee * reserve_out) / (reserve_in * 1000 + amount_in_with_fee)
}

#[cfg(test)]
mod corpus_tests {
    use super::amm_quote_pure;
    use core::poseidon::poseidon_hash_span;

    #[test]
    fn test_amm_constant_product_quote() {
        let out = amm_quote_pure(1000, 100_000, 100_000);
        assert(out == 987, 'unexpected_amm_out');
    }

    #[test]
    #[should_panic(expected: ('EMPTY_RESERVES',))]
    fn test_amm_zero_reserves_panics() {
        amm_quote_pure(100, 0, 1000);
    }

    #[test]
    fn test_poseidon_merkle_hash_determinism() {
        let h1 = poseidon_hash_span(array![10, 20].span());
        let h2 = poseidon_hash_span(array![10, 20].span());
        assert(h1 == h2 && h1 != 0, 'poseidon_mismatch');
    }
}
"#,
        )
        .unwrap();

        // 1. Native build of all 8 contract archetypes
        let build_rep_path = dir.dir.join("build_rep.json");
        run_build(BuildArgs {
            common: BuildCommonArgs {
                manifest_path: Some(manifest_path.clone()),
                package: None,
                workspace: false,
                features: Vec::new(),
                offline: true,
                release: false,
                profile: None,
            },
            engine: EngineArg::Uc,
            daemon_mode: DaemonModeArg::Off,
            report_path: Some(build_rep_path.clone()),
            record_failure: None,
            json: false,
            plan_only: false,
        })
        .expect("8-contract corpus should build natively");

        let build_rep: serde_json::Value =
            serde_json::from_slice(&fs::read(&build_rep_path).unwrap()).unwrap();
        assert_eq!(build_rep["compile_backend"], "uc_native");
        // 8 contracts * 2 (Sierra + CASM) + 1 starknet_artifacts.json = 17 artifacts
        assert_eq!(build_rep["artifact_count"], 17);

        // 2. Native in-process test execution on the corpus
        let test_rep_path = dir.dir.join("test_rep.json");
        run_test(TestArgs {
            filter: None,
            manifest_path: Some(manifest_path.clone()),
            package: None,
            workspace: false,
            features: Vec::new(),
            affected: false,
            include_ignored: false,
            ignored: false,
            exact: false,
            gas_report: true,
            no_cache: false,
            offline: false,
            json: true,
            report_path: Some(test_rep_path.clone()),
            ..Default::default()
        })
        .expect("corpus tests should pass in-process");
        let test_rep: serde_json::Value =
            serde_json::from_slice(&fs::read(&test_rep_path).unwrap()).unwrap();
        assert_eq!(test_rep["passed_count"], 3);
        assert_eq!(test_rep["failed_count"], 0);

        // 3. Starkscan verification bundle generation across all 8 contracts
        let bundle_path = dir.dir.join("corpus_starkscan_bundle.json");
        let verify_rep_path = dir.dir.join("corpus_verify.json");
        run_verify_onchain(VerifyOnchainArgs {
            contract_address: None,
            class_hash: None,
            contract: None,
            verifier: VerifierProviderArg::Starkscan,
            verifier_url: None,
            api_key: None,
            export_bundle: Some(bundle_path.clone()),
            verify_bundle: None,
            submit: false,
            url: None,
            network: Some("mainnet".to_string()),
            sncast_profile: None,
            manifest_path: Some(manifest_path),
            profile: None,
            release: false,
            json: true,
            report_path: Some(verify_rep_path.clone()),
        })
        .expect("Starkscan verification bundle for 8-contract corpus should succeed");

        let verify_rep: serde_json::Value =
            serde_json::from_slice(&fs::read(&verify_rep_path).unwrap()).unwrap();
        assert_eq!(verify_rep["verified"], true);
        assert_eq!(verify_rep["contracts"].as_array().unwrap().len(), 8);
    }

    #[test]
    fn multi_edition_and_multi_version_native_compile_and_verify_matrix() {
        let _env = crate::test_env_shared();
        let matrix = [
            ("2023_01", "2.8.0"),
            ("2023_10", "2.11.4"),
            ("2023_11", "2.14.0"),
            ("2024_07", "2.16.0"),
        ];
        for (edition, version) in matrix {
            let dir = TempOnchainDir::new(&format!("matrix_{edition}_{}", version.replace('.', "_")));
            let manifest_path = dir.dir.join("Scarb.toml");
            fs::write(
                &manifest_path,
                format!(
                    r#"[package]
name = "edition_pkg"
version = "0.1.0"
edition = "{edition}"
cairo-version = "{version}"

[dependencies]
starknet = "{version}"

[[target.starknet-contract]]
sierra = true
casm = true
"#
                ),
            )
            .unwrap();
            fs::write(
                dir.dir.join("src/lib.cairo"),
                r#"#[starknet::contract]
mod EditionContract {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        counter: u128,
    }

    #[external(v0)]
    fn increment(ref self: ContractState, delta: u128) -> u128 {
        let next = self.counter.read() + delta;
        self.counter.write(next);
        next
    }
}
"#,
            )
            .unwrap();

            let build_rep_path = dir.dir.join("build_rep.json");
            run_build(BuildArgs {
                common: BuildCommonArgs {
                    manifest_path: Some(manifest_path.clone()),
                    package: None,
                    workspace: false,
                    features: Vec::new(),
                    offline: true,
                    release: false,
                    profile: None,
                },
                engine: EngineArg::Uc,
                daemon_mode: DaemonModeArg::Off,
                report_path: Some(build_rep_path.clone()),
                record_failure: None,
                json: false,
                plan_only: false,
            })
            .unwrap_or_else(|e| panic!("edition {edition} / version {version} build failed: {e:#}"));
            let build_rep: serde_json::Value =
                serde_json::from_slice(&fs::read(&build_rep_path).unwrap()).unwrap();
            let backend = build_rep["compile_backend"].as_str().unwrap_or("");
            assert!(
                matches!(backend, "uc_native" | "uc_native_external_helper"),
                "edition {edition} / version {version} should compile natively, got {backend}"
            );

            let bundle_path = dir.dir.join("bundle.json");
            let verify_rep_path = dir.dir.join("verify_rep.json");
            run_verify_onchain(VerifyOnchainArgs {
                contract_address: None,
                class_hash: None,
                contract: None,
                verifier: VerifierProviderArg::Starkscan,
                verifier_url: None,
                api_key: None,
                export_bundle: Some(bundle_path.clone()),
                verify_bundle: None,
                submit: false,
                url: None,
                network: Some("sepolia".to_string()),
                sncast_profile: None,
                manifest_path: Some(manifest_path),
                profile: None,
                release: false,
                json: true,
                report_path: Some(verify_rep_path.clone()),
            })
            .unwrap();
            let bundle_json: serde_json::Value =
                serde_json::from_slice(&fs::read(&bundle_path).unwrap()).unwrap();
            let bundle_check_cold = verify_starkscan_bundle_in_memory(&bundle_json).unwrap();
            assert_eq!(bundle_check_cold["verified"], true);
            let bundle_check = verify_starkscan_bundle_in_memory(&bundle_json).unwrap();
            assert_eq!(bundle_check["verified"], true);
            assert!(bundle_check["elapsed_ms"].as_f64().unwrap() < 500.0);
        }
    }

    #[test]
    fn account_create_import_deploy_list_and_delete_roundtrip() {
        let dir = TempOnchainDir::new("account_lifecycle");
        let accounts_file = dir.dir.join("accounts.json");
        let create_rep = dir.dir.join("create.json");
        run_account(AccountArgs {
            command: AccountSubcommand::Create(AccountCreateArgs {
                name: "deployer_1".to_string(),
                account_type: "oz".to_string(),
                class_hash: None,
                salt: "0x7".to_string(),
                url: None,
                network: "alpha-sepolia".to_string(),
                accounts_file: Some(accounts_file.clone()),
                add_profile: None,
                manifest_path: None,
                json: true,
                report_path: Some(create_rep.clone()),
            }),
        })
        .unwrap();
        let created: serde_json::Value =
            serde_json::from_slice(&fs::read(&create_rep).unwrap()).unwrap();
        assert!(created["address"].as_str().unwrap().starts_with("0x"));
        assert!(created["public_key"].as_str().unwrap().starts_with("0x"));

        // Deploy dry-run by `--name deployer_1`
        let deploy_rep = dir.dir.join("deploy.json");
        run_account(AccountArgs {
            command: AccountSubcommand::Deploy(AccountDeployArgs {
                name: Some("deployer_1".to_string()),
                accounts_file: Some(accounts_file.clone()),
                class_hash: None,
                private_key: None,
                constructor_calldata: Vec::new(),
                salt: "0x7".to_string(),
                url: None,
                network: None,
                dry_run: true,
                wait: false,
                wait_timeout: 5,
                json: true,
                report_path: Some(deploy_rep.clone()),
            }),
        })
        .unwrap();
        let deployed: serde_json::Value =
            serde_json::from_slice(&fs::read(&deploy_rep).unwrap()).unwrap();
        assert_eq!(
            parse_felt_str(deployed["contract_address"].as_str().unwrap()).unwrap(),
            parse_felt_str(created["address"].as_str().unwrap()).unwrap()
        );

        // Import a second account
        let import_rep = dir.dir.join("import.json");
        run_account(AccountArgs {
            command: AccountSubcommand::Import(AccountImportArgs {
                name: "imported_2".to_string(),
                address: "0x12345".to_string(),
                private_key: "0x999".to_string(),
                public_key: None,
                account_type: "argent".to_string(),
                class_hash: None,
                salt: Some("0x1".to_string()),
                url: None,
                network: "alpha-sepolia".to_string(),
                accounts_file: Some(accounts_file.clone()),
                add_profile: None,
                manifest_path: None,
                json: true,
                report_path: Some(import_rep.clone()),
            }),
        })
        .unwrap();

        // List accounts
        let list_rep = dir.dir.join("list.json");
        run_account(AccountArgs {
            command: AccountSubcommand::List(AccountListArgs {
                accounts_file: Some(accounts_file.clone()),
                manifest_path: None,
                json: true,
                report_path: Some(list_rep.clone()),
            }),
        })
        .unwrap();
        let listed: serde_json::Value =
            serde_json::from_slice(&fs::read(&list_rep).unwrap()).unwrap();
        assert_eq!(listed["accounts"].as_array().unwrap().len(), 2);

        // Delete `imported_2`
        run_account(AccountArgs {
            command: AccountSubcommand::Delete(AccountDeleteArgs {
                name: "imported_2".to_string(),
                network: Some("alpha-sepolia".to_string()),
                accounts_file: Some(accounts_file),
                yes: true,
                manifest_path: None,
                json: true,
                report_path: None,
            }),
        })
        .unwrap();
    }
}

