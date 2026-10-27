use super::*;
#[cfg(feature = "native-compile")]
use cairo_lang_formatter::{
    BreakingBehaviorConfig, CairoFormatter, CollectionsBreakingBehavior, FormatOutcome,
    FormatterConfig,
};
#[cfg(feature = "native-compile")]
use cairo_lang_lowering::db::LoweringGroup;
#[cfg(feature = "native-compile")]
use cairo_lang_parser::db::ParserGroup;
#[cfg(feature = "native-compile")]
use cairo_lang_semantic::db::SemanticGroup;
#[cfg(feature = "native-compile")]
use rayon::prelude::*;

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FmtEmitMode {
    Files,
    Stdout,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct FmtArgs {
    /// Check if files are formatted without modifying them.
    #[arg(short = 'c', long)]
    pub(crate) check: bool,

    /// Emit formatted output to files (default) or stdout.
    #[arg(long, value_enum, default_value_t = FmtEmitMode::Files)]
    pub(crate) emit: FmtEmitMode,

    /// Disable colored diff output.
    #[arg(long)]
    pub(crate) no_color: bool,

    #[arg(long)]
    pub(crate) manifest_path: Option<PathBuf>,

    #[arg(short = 'p', long)]
    pub(crate) package: Option<String>,

    #[arg(short = 'w', long)]
    pub(crate) workspace: bool,

    #[arg(long)]
    pub(crate) sort_module_level_items: Option<bool>,

    #[arg(long)]
    pub(crate) json: bool,

    #[arg(long)]
    pub(crate) report_path: Option<PathBuf>,

    /// Explicit Cairo files or directories to format.
    pub(crate) paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FmtFileResult {
    pub(crate) path: String,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) diff: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) formatted_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FmtReport {
    pub(crate) schema_version: u32,
    pub(crate) generated_at_epoch_ms: u64,
    pub(crate) check_mode: bool,
    pub(crate) emit_mode: FmtEmitMode,
    pub(crate) total_files: usize,
    pub(crate) formatted_files: usize,
    pub(crate) unchanged_files: usize,
    pub(crate) error_files: usize,
    pub(crate) elapsed_ms: f64,
    pub(crate) passed: bool,
    pub(crate) files: Vec<FmtFileResult>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct CheckArgs {
    #[arg(long)]
    pub(crate) manifest_path: Option<PathBuf>,

    #[arg(short = 'p', long)]
    pub(crate) package: Option<String>,

    #[arg(short = 'w', long)]
    pub(crate) workspace: bool,

    #[arg(long)]
    pub(crate) offline: bool,

    #[arg(short = 'P', long)]
    pub(crate) profile: Option<String>,

    #[arg(long)]
    pub(crate) release: bool,

    #[arg(long)]
    pub(crate) dev: bool,

    #[arg(long)]
    pub(crate) json: bool,

    #[arg(long)]
    pub(crate) report_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CheckPackageResult {
    pub(crate) package_name: String,
    pub(crate) manifest_path: String,
    pub(crate) checked_modules: usize,
    pub(crate) checked_contracts: usize,
    pub(crate) passed: bool,
    pub(crate) elapsed_ms: f64,
    pub(crate) diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CheckReport {
    pub(crate) schema_version: u32,
    pub(crate) generated_at_epoch_ms: u64,
    pub(crate) passed: bool,
    pub(crate) total_packages: usize,
    pub(crate) total_modules: usize,
    pub(crate) total_contracts: usize,
    pub(crate) elapsed_ms: f64,
    pub(crate) packages: Vec<CheckPackageResult>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct CleanArgs {
    #[arg(long)]
    pub(crate) manifest_path: Option<PathBuf>,

    #[arg(long)]
    pub(crate) target_dir: Option<PathBuf>,

    /// Also clean `.uc/cache`, `.uc/receipts`, and `.uc/native-project`.
    #[arg(long)]
    pub(crate) uc_cache: bool,

    /// Clean both `target/` and `.uc/`.
    #[arg(long)]
    pub(crate) all: bool,

    #[arg(long)]
    pub(crate) dry_run: bool,

    #[arg(long)]
    pub(crate) json: bool,

    #[arg(long)]
    pub(crate) report_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CleanReport {
    pub(crate) schema_version: u32,
    pub(crate) generated_at_epoch_ms: u64,
    pub(crate) workspace_root: String,
    pub(crate) dry_run: bool,
    pub(crate) removed_paths: Vec<String>,
    pub(crate) removed_files: u64,
    pub(crate) removed_bytes: u64,
    pub(crate) elapsed_ms: f64,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VcsKindArg {
    Git,
    None,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct NewArgs {
    /// Directory path where the new Cairo package will be created.
    pub(crate) path: PathBuf,

    /// Set the resulting package name (defaults to the directory name).
    #[arg(long)]
    pub(crate) name: Option<String>,

    /// Specify the Cairo edition to use.
    #[arg(long, default_value = "2024_07")]
    pub(crate) edition: String,

    /// Scaffold a Starknet smart contract template (`#[starknet::contract]`).
    #[arg(long)]
    pub(crate) starknet: bool,

    /// Emit a legacy `Scarb.toml` instead of `uc.toml`.
    #[arg(long)]
    pub(crate) legacy_scarb: bool,

    /// Version control system to initialize.
    #[arg(long, value_enum, default_value_t = VcsKindArg::Git)]
    pub(crate) vcs: VcsKindArg,

    /// Do not initialize any version control system.
    #[arg(long)]
    pub(crate) no_vcs: bool,

    #[arg(long)]
    pub(crate) json: bool,

    #[arg(long)]
    pub(crate) report_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct InitArgs {
    /// Directory path to initialize (defaults to current working directory).
    #[arg(long)]
    pub(crate) path: Option<PathBuf>,

    /// Set the resulting package name (defaults to the directory name).
    #[arg(long)]
    pub(crate) name: Option<String>,

    /// Specify the Cairo edition to use.
    #[arg(long, default_value = "2024_07")]
    pub(crate) edition: String,

    /// Scaffold a Starknet smart contract template (`#[starknet::contract]`).
    #[arg(long)]
    pub(crate) starknet: bool,

    /// Emit a legacy `Scarb.toml` instead of `uc.toml`.
    #[arg(long)]
    pub(crate) legacy_scarb: bool,

    /// Version control system to initialize.
    #[arg(long, value_enum, default_value_t = VcsKindArg::Git)]
    pub(crate) vcs: VcsKindArg,

    /// Do not initialize any version control system.
    #[arg(long)]
    pub(crate) no_vcs: bool,

    #[arg(long)]
    pub(crate) json: bool,

    #[arg(long)]
    pub(crate) report_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ScaffoldReport {
    pub(crate) schema_version: u32,
    pub(crate) generated_at_epoch_ms: u64,
    pub(crate) package_name: String,
    pub(crate) edition: String,
    pub(crate) starknet: bool,
    pub(crate) root_dir: String,
    pub(crate) manifest_path: String,
    pub(crate) created_files: Vec<String>,
    pub(crate) vcs_initialized: bool,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct AddArgs {
    /// Dependencies to add (e.g. `starknet`, `openzeppelin@1.0.0`, `alexandria_math@>=0.1.0`).
    #[arg(required = true)]
    pub(crate) packages: Vec<String>,

    /// Filesystem path to local dependency package.
    #[arg(long)]
    pub(crate) path: Option<String>,

    /// Git repository URL for dependency.
    #[arg(long)]
    pub(crate) git: Option<String>,

    /// Git branch for `--git` dependency.
    #[arg(long)]
    pub(crate) branch: Option<String>,

    /// Git tag for `--git` dependency.
    #[arg(long)]
    pub(crate) tag: Option<String>,

    /// Git revision hash for `--git` dependency.
    #[arg(long)]
    pub(crate) rev: Option<String>,

    /// Add to `[dev-dependencies]` instead of `[dependencies]`.
    #[arg(long)]
    pub(crate) dev: bool,

    /// Use workspace inheritance (`{ workspace = true }`).
    #[arg(long)]
    pub(crate) workspace: bool,

    #[arg(long)]
    pub(crate) manifest_path: Option<PathBuf>,

    #[arg(short = 'p', long)]
    pub(crate) package: Option<String>,

    #[arg(long)]
    pub(crate) dry_run: bool,

    #[arg(long)]
    pub(crate) json: bool,

    #[arg(long)]
    pub(crate) report_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct RemoveArgs {
    /// Dependencies to remove.
    #[arg(required = true)]
    pub(crate) packages: Vec<String>,

    /// Remove from `[dev-dependencies]` instead of `[dependencies]`.
    #[arg(long)]
    pub(crate) dev: bool,

    #[arg(long)]
    pub(crate) manifest_path: Option<PathBuf>,

    #[arg(short = 'p', long)]
    pub(crate) package: Option<String>,

    #[arg(long)]
    pub(crate) dry_run: bool,

    #[arg(long)]
    pub(crate) json: bool,

    #[arg(long)]
    pub(crate) report_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ManifestMutationReport {
    pub(crate) schema_version: u32,
    pub(crate) generated_at_epoch_ms: u64,
    pub(crate) operation: String,
    pub(crate) manifest_path: String,
    pub(crate) section: String,
    pub(crate) dry_run: bool,
    pub(crate) mutated_dependencies: Vec<String>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TreeCharsetArg {
    Utf8,
    Ascii,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct TreeArgs {
    #[arg(long)]
    pub(crate) manifest_path: Option<PathBuf>,

    #[arg(short = 'p', long)]
    pub(crate) package: Option<String>,

    /// Maximum display depth of the dependency tree.
    #[arg(long)]
    pub(crate) depth: Option<usize>,

    /// Do not de-duplicate repeated dependencies (`(*)`).
    #[arg(long)]
    pub(crate) no_dedupe: bool,

    /// Exclude `[dev-dependencies]`.
    #[arg(long)]
    pub(crate) no_dev: bool,

    /// Prune the given package from the dependency tree.
    #[arg(long)]
    pub(crate) prune: Vec<String>,

    #[arg(long, value_enum, default_value_t = TreeCharsetArg::Utf8)]
    pub(crate) charset: TreeCharsetArg,

    #[arg(long)]
    pub(crate) json: bool,

    #[arg(long)]
    pub(crate) report_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TreeNodeReport {
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<String>,
    pub(crate) kind: String,
    pub(crate) deduped: bool,
    pub(crate) dependencies: Vec<TreeNodeReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TreeReport {
    pub(crate) schema_version: u32,
    pub(crate) generated_at_epoch_ms: u64,
    pub(crate) manifest_path: String,
    pub(crate) roots: Vec<TreeNodeReport>,
    pub(crate) rendered_tree: String,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct ExpandArgs {
    #[arg(long)]
    pub(crate) manifest_path: Option<PathBuf>,

    /// Filter by module path (e.g. `my_pkg::my_mod`).
    #[arg(long)]
    pub(crate) module: Option<String>,

    /// Filter by Starknet contract name (e.g. `token`).
    #[arg(long)]
    pub(crate) contract: Option<String>,

    /// Emit raw expanded source without running `cairo-lang-formatter`.
    #[arg(long)]
    pub(crate) raw: bool,

    #[arg(long)]
    pub(crate) json: bool,

    #[arg(long)]
    pub(crate) report_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ExpandedModuleReport {
    pub(crate) module_path: String,
    pub(crate) virtual_files_count: usize,
    pub(crate) expanded_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ExpandReport {
    pub(crate) schema_version: u32,
    pub(crate) generated_at_epoch_ms: u64,
    pub(crate) package_name: String,
    pub(crate) manifest_path: String,
    pub(crate) elapsed_ms: f64,
    pub(crate) modules: Vec<ExpandedModuleReport>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct LintArgs {
    #[arg(long)]
    pub(crate) manifest_path: Option<PathBuf>,

    #[arg(short = 'p', long)]
    pub(crate) package: Option<String>,

    #[arg(short = 'w', long)]
    pub(crate) workspace: bool,

    /// Treat warnings as errors (exit code 1).
    #[arg(short = 'D', long)]
    pub(crate) deny_warnings: bool,

    /// Emit GitHub Code Scanning SARIF v2.1.0 JSON format.
    #[arg(long)]
    pub(crate) sarif: bool,

    /// Automatically apply safe source-level remediations (`LegacyMap` -> `Map`, boolean simplifications, `#[key]` hints).
    #[arg(long)]
    pub(crate) fix: bool,

    /// Preview source-level remediations as unified diffs without modifying files.
    #[arg(long)]
    pub(crate) diff: bool,

    #[arg(long)]
    pub(crate) json: bool,

    #[arg(long)]
    pub(crate) report_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LintFinding {
    pub(crate) rule_id: String,
    pub(crate) severity: String,
    pub(crate) category: String,
    pub(crate) file: String,
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) message: String,
    pub(crate) suggestion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LintReport {
    pub(crate) schema_version: u32,
    pub(crate) generated_at_epoch_ms: u64,
    pub(crate) manifest_path: String,
    pub(crate) passed: bool,
    pub(crate) error_count: usize,
    pub(crate) warning_count: usize,
    pub(crate) info_count: usize,
    #[serde(default)]
    pub(crate) fixes_applied: usize,
    pub(crate) elapsed_ms: f64,
    pub(crate) findings: Vec<LintFinding>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct ProfileArgs {
    /// Optional contract name filter.
    pub(crate) target: Option<String>,

    #[arg(long)]
    pub(crate) manifest_path: Option<PathBuf>,

    #[arg(short = 'P', long)]
    pub(crate) profile: Option<String>,

    #[arg(long)]
    pub(crate) release: bool,

    /// Maximum number of functions/entrypoints to display per contract.
    #[arg(long, default_value_t = 20)]
    pub(crate) top: usize,

    /// Compute Stwo Circle STARK prover AIR builtin trace multiplicities and proving weights.
    #[arg(long)]
    pub(crate) prover_air: bool,

    #[arg(long)]
    pub(crate) json: bool,

    #[arg(long)]
    pub(crate) report_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ProverAirMultiplicitySummary {
    pub(crate) range_check_builtin: usize,
    pub(crate) pedersen_builtin: usize,
    pub(crate) poseidon_builtin: usize,
    pub(crate) bitwise_builtin: usize,
    pub(crate) ec_op_builtin: usize,
    pub(crate) keccak_builtin: usize,
    pub(crate) stwo_trace_cells_estimate: u64,
    pub(crate) bottleneck_builtin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EntrypointProfileSummary {
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) selector: String,
    pub(crate) function_idx: usize,
    pub(crate) reachable_functions: usize,
    pub(crate) reachable_sierra_statements: usize,
    pub(crate) storage_reads: usize,
    pub(crate) storage_writes: usize,
    pub(crate) contract_calls: usize,
    pub(crate) events_emitted: usize,
    pub(crate) crypto_ops: usize,
    pub(crate) estimated_l2_gas_weight: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ContractProfileSummary {
    pub(crate) package_name: String,
    pub(crate) contract_name: String,
    pub(crate) module_path: String,
    pub(crate) class_hash: String,
    pub(crate) compiled_class_hash: String,
    pub(crate) sierra_statements: usize,
    pub(crate) sierra_functions: usize,
    pub(crate) sierra_libfuncs: usize,
    pub(crate) sierra_types: usize,
    pub(crate) casm_bytecode_felts: usize,
    pub(crate) casm_hints: usize,
    pub(crate) max_casm_bytecode_felts: usize,
    pub(crate) casm_limit_utilization_pct: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) prover_air: Option<ProverAirMultiplicitySummary>,
    pub(crate) entrypoints: Vec<EntrypointProfileSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProfileReport {
    pub(crate) schema_version: u32,
    pub(crate) generated_at_epoch_ms: u64,
    pub(crate) manifest_path: String,
    pub(crate) profile: String,
    pub(crate) elapsed_ms: f64,
    pub(crate) contracts: Vec<ContractProfileSummary>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct DiffArgs {
    /// Baseline `.contract_class.json` file or `target/<profile>` directory.
    pub(crate) baseline: PathBuf,

    /// Candidate `.contract_class.json` file or `target/<profile>` directory (defaults to current workspace target).
    pub(crate) candidate: Option<PathBuf>,

    #[arg(long)]
    pub(crate) manifest_path: Option<PathBuf>,

    /// Exit with code 1 if breaking ABI or selector changes are detected.
    #[arg(long)]
    pub(crate) fail_on_breaking: bool,

    #[arg(long)]
    pub(crate) json: bool,

    #[arg(long)]
    pub(crate) report_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ContractDiffEntry {
    pub(crate) contract_name: String,
    pub(crate) status: String,
    pub(crate) baseline_class_hash: Option<String>,
    pub(crate) candidate_class_hash: Option<String>,
    pub(crate) baseline_sierra_statements: Option<usize>,
    pub(crate) candidate_sierra_statements: Option<usize>,
    pub(crate) sierra_statements_delta: isize,
    pub(crate) baseline_casm_felts: Option<usize>,
    pub(crate) candidate_casm_felts: Option<usize>,
    pub(crate) casm_felts_delta: isize,
    pub(crate) added_selectors: Vec<String>,
    pub(crate) removed_selectors: Vec<String>,
    pub(crate) breaking_changes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DiffReport {
    pub(crate) schema_version: u32,
    pub(crate) generated_at_epoch_ms: u64,
    pub(crate) baseline_path: String,
    pub(crate) candidate_path: String,
    pub(crate) has_changes: bool,
    pub(crate) has_breaking_changes: bool,
    pub(crate) contracts: Vec<ContractDiffEntry>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AbiOutputFormat {
    Json,
    Ts,
    Cairo,
    Rust,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct AbiArgs {
    /// Optional contract name or `.contract_class.json` path filter.
    pub(crate) target: Option<String>,

    #[arg(long)]
    pub(crate) manifest_path: Option<PathBuf>,

    #[arg(short = 'P', long)]
    pub(crate) profile: Option<String>,

    #[arg(long)]
    pub(crate) release: bool,

    /// Output format: `json`, `ts` (TypeScript `as const`), `cairo` (`#[starknet::interface]`), or `rust` (`starknet-rs` selectors).
    #[arg(long, value_enum, default_value_t = AbiOutputFormat::Json)]
    pub(crate) format: AbiOutputFormat,

    /// Optional file path to write the generated bindings/ABI.
    #[arg(short = 'o', long)]
    pub(crate) output: Option<PathBuf>,

    #[arg(long)]
    pub(crate) json: bool,

    #[arg(long)]
    pub(crate) report_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct ManifestPathArgs {
    #[arg(long)]
    pub(crate) manifest_path: Option<PathBuf>,

    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct RunScriptArgs {
    /// Script name defined in `[scripts]` in `Scarb.toml`.
    pub(crate) script: Option<String>,

    /// List available scripts in `Scarb.toml`.
    #[arg(long)]
    pub(crate) list: bool,

    #[arg(long)]
    pub(crate) manifest_path: Option<PathBuf>,

    #[arg(short = 'p', long)]
    pub(crate) package: Option<String>,

    #[arg(long)]
    pub(crate) json: bool,

    /// Extra arguments forwarded to the script command.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub(crate) args: Vec<String>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum ShellKindArg {
    Bash,
    Zsh,
    Fish,
    Powershell,
    Elvish,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct CompletionsArgs {
    pub(crate) shell: ShellKindArg,
}

// ============================================================================
// 1. `uc fmt` Implementation
// ============================================================================

#[cfg(feature = "native-compile")]
fn resolve_formatter_config_from_manifest(
    manifest_opt: Option<&TomlValue>,
    sort_override: Option<bool>,
) -> FormatterConfig {
    let mut config = FormatterConfig::default();
    if let Some(manifest) = manifest_opt {
        if let Some(fmt_table) = manifest
            .get("tool")
            .and_then(TomlValue::as_table)
            .and_then(|tool| tool.get("fmt"))
            .and_then(TomlValue::as_table)
        {
            if let Some(tab_size) = fmt_table.get("tab-size").and_then(TomlValue::as_integer) {
                if tab_size > 0 {
                    config.tab_size = tab_size as usize;
                }
            }
            if let Some(max_len) = fmt_table
                .get("max-line-length")
                .and_then(TomlValue::as_integer)
            {
                if max_len > 0 {
                    config.max_line_length = max_len as usize;
                }
            }
            if let Some(sort) = fmt_table
                .get("sort-module-level-items")
                .and_then(TomlValue::as_bool)
            {
                config.sort_module_level_items = sort;
            }
            if let Some(merge) = fmt_table
                .get("merge-use-items")
                .or_else(|| fmt_table.get("use-merge-strategy"))
                .and_then(TomlValue::as_bool)
            {
                config.merge_use_items = merge;
            }
            if let Some(dup) = fmt_table
                .get("allow-duplicate-uses")
                .and_then(TomlValue::as_bool)
            {
                config.allow_duplicate_uses = dup;
            }
            let parse_breaking = |val: &TomlValue| -> Option<CollectionsBreakingBehavior> {
                if let Some(b) = val.as_bool() {
                    return Some(CollectionsBreakingBehavior::from(b));
                }
                match val.as_str()?.trim() {
                    "line-by-line" | "LineByLine" => Some(CollectionsBreakingBehavior::LineByLine),
                    "single-break-point" | "SingleBreakPoint" => {
                        Some(CollectionsBreakingBehavior::SingleBreakPoint)
                    }
                    _ => None,
                }
            };
            let mut breaking = BreakingBehaviorConfig {
                tuple: config.breaking_behavior.tuple,
                fixed_array: config.breaking_behavior.fixed_array,
                macro_call: config.breaking_behavior.macro_call,
            };
            if let Some(v) = fmt_table
                .get("tuple-line-breaking")
                .and_then(parse_breaking)
            {
                breaking.tuple = v;
            }
            if let Some(v) = fmt_table
                .get("fixed-array-line-breaking")
                .and_then(parse_breaking)
            {
                breaking.fixed_array = v;
            }
            if let Some(v) = fmt_table
                .get("macro-call-line-breaking")
                .and_then(parse_breaking)
            {
                breaking.macro_call = v;
            }
            config.breaking_behavior = breaking;
        }
    }
    if let Some(sort) = sort_override {
        config.sort_module_level_items = sort;
    }
    config
}

fn collect_cairo_files_under(root: &Path, out: &mut BTreeSet<PathBuf>) {
    if root.is_file() {
        if root.extension().and_then(|e| e.to_str()) == Some("cairo") {
            out.insert(root.to_path_buf());
        }
        return;
    }
    if !root.is_dir() {
        return;
    }
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_str().unwrap_or("");
            name != "target" && name != ".uc" && name != ".git" && name != "node_modules"
        })
        .flatten()
    {
        if entry.file_type().is_file()
            && entry.path().extension().and_then(|e| e.to_str()) == Some("cairo")
        {
            out.insert(entry.path().to_path_buf());
        }
    }
}

pub(crate) fn run_fmt(args: FmtArgs) -> Result<()> {
    #[cfg(not(feature = "native-compile"))]
    {
        let _ = args;
        bail!("`uc fmt` requires a binary built with the `native-compile` feature");
    }
    #[cfg(feature = "native-compile")]
    {
        let started = Instant::now();
        let manifest_opt = resolve_manifest_path(&args.manifest_path).ok();
        let manifest_val = manifest_opt
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|txt| toml::from_str::<TomlValue>(&txt).ok());
        let config =
            resolve_formatter_config_from_manifest(manifest_val.as_ref(), args.sort_module_level_items);

        let mut files = BTreeSet::new();
        if !args.paths.is_empty() {
            for p in &args.paths {
                if !p.exists() {
                    bail!("path does not exist: {}", p.display());
                }
                collect_cairo_files_under(p, &mut files);
            }
        } else if let Some(manifest_path) = &manifest_opt {
            let root = manifest_path.parent().unwrap_or_else(|| Path::new("."));
            if let Some(pkg) = args.package.as_deref() {
                if let Ok(eff) = resolve_native_effective_manifest_path(manifest_path, root, Some(pkg)) {
                    let pkg_root = eff.parent().unwrap_or(root);
                    collect_cairo_files_under(pkg_root, &mut files);
                } else {
                    collect_cairo_files_under(root, &mut files);
                }
            } else {
                collect_cairo_files_under(root, &mut files);
            }
        } else {
            let cwd = std::env::current_dir()?;
            collect_cairo_files_under(&cwd, &mut files);
        }

        let file_vec: Vec<PathBuf> = files.into_iter().collect();
        let check_mode = args.check;
        let emit_mode = args.emit;
        let no_color = args.no_color || args.json || args.report_path.is_some();

        let file_results: Vec<FmtFileResult> = file_vec
            .par_iter()
            .map(|path| {
                let formatter = CairoFormatter::new(config.clone());
                let path_str = path.display().to_string();
                let outcome_res = if check_mode || emit_mode == FmtEmitMode::Stdout {
                    formatter.format_to_string(&path.as_path())
                } else {
                    formatter.format_in_place(&path.as_path())
                };
                match outcome_res {
                    Ok(FormatOutcome::Identical(text)) => FmtFileResult {
                        path: path_str,
                        status: "unchanged".to_string(),
                        diff: None,
                        formatted_output: (emit_mode == FmtEmitMode::Stdout).then_some(text),
                        error: None,
                    },
                    Ok(FormatOutcome::DiffFound(diff)) => {
                        let diff_text = if no_color {
                            diff.to_string()
                        } else {
                            diff.display_colored().to_string()
                        };
                        FmtFileResult {
                            path: path_str,
                            status: if check_mode {
                                "would_reformat".to_string()
                            } else {
                                "reformatted".to_string()
                            },
                            diff: Some(diff_text),
                            formatted_output: (emit_mode == FmtEmitMode::Stdout)
                                .then_some(diff.formatted),
                            error: None,
                        }
                    }
                    Err(err) => FmtFileResult {
                        path: path_str,
                        status: "error".to_string(),
                        diff: None,
                        formatted_output: None,
                        error: Some(err.to_string()),
                    },
                }
            })
            .collect();

        let formatted_files = file_results
            .iter()
            .filter(|r| r.status == "reformatted" || r.status == "would_reformat")
            .count();
        let unchanged_files = file_results
            .iter()
            .filter(|r| r.status == "unchanged")
            .count();
        let error_files = file_results.iter().filter(|r| r.status == "error").count();
        let passed = error_files == 0 && (!check_mode || formatted_files == 0);
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

        let report = FmtReport {
            schema_version: UC_AGENT_JSON_SCHEMA_VERSION,
            generated_at_epoch_ms: epoch_ms_u64()?,
            check_mode,
            emit_mode,
            total_files: file_results.len(),
            formatted_files,
            unchanged_files,
            error_files,
            elapsed_ms,
            passed,
            files: file_results,
        };

        if let Some(report_path) = &args.report_path {
            write_json_report(report_path, &report)?;
        }

        if args.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else if emit_mode == FmtEmitMode::Stdout {
            for f in &report.files {
                if let Some(out) = &f.formatted_output {
                    print!("{out}");
                }
                if let Some(err) = &f.error {
                    eprintln!("error formatting {}: {err}", f.path);
                }
            }
        } else {
            for f in &report.files {
                if check_mode {
                    if let Some(diff) = &f.diff {
                        println!("Diff in file {}:\n{}", f.path, diff);
                    }
                }
                if let Some(err) = &f.error {
                    eprintln!("error formatting {}: {err}", f.path);
                }
            }
            if !check_mode && error_files == 0 {
                println!(
                    "uc fmt: formatted {} file(s) ({} unchanged) in {:.1}ms",
                    report.formatted_files, report.unchanged_files, report.elapsed_ms
                );
            }
        }

        if !passed {
            std::process::exit(1);
        }
        Ok(())
    }
}

// ============================================================================
// 2. `uc check` Implementation
// ============================================================================

#[cfg(feature = "native-compile")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FastCheckCacheEntry {
    source_digest: String,
    package_name: String,
    checked_modules: usize,
    checked_contracts: usize,
}

#[cfg(feature = "native-compile")]
fn hash_cairo_tree(hasher: &mut blake3::Hasher, root: &Path, src_dir: &Path) -> Result<()> {
    let mut files = BTreeSet::new();
    if src_dir.is_dir() {
        collect_cairo_files_under(src_dir, &mut files);
    }
    for file in files {
        let rel = file.strip_prefix(root).unwrap_or(&file);
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update(&[0]);
        let bytes = fs::read(&file)?;
        hasher.update(&bytes);
        hasher.update(&[0xff]);
    }
    Ok(())
}

/// Digest of everything that can change the outcome of `uc check`: the manifest, lockfile,
/// package sources, every resolved dependency's sources (or a marker when it is missing on
/// disk), and the in-process compiler version. A hit on this digest is a real "nothing changed".
fn compute_fast_package_source_digest(manifest_path: &Path) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"uc-fast-check-digest-v2");
    hasher.update(native_cairo_lang_compiler_version().as_bytes());
    hasher.update(&[0]);
    let manifest_bytes = fs::read(manifest_path)?;
    hasher.update(&manifest_bytes);
    let pkg_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    for lock_name in ["Scarb.lock", "uc.lock"] {
        if let Ok(lock) = fs::read(pkg_dir.join(lock_name)) {
            hasher.update(b"lock");
            hasher.update(&lock);
        }
    }
    hash_cairo_tree(&mut hasher, pkg_dir, &pkg_dir.join("src"))?;
    hash_cairo_tree(&mut hasher, pkg_dir, &pkg_dir.join("tests"))?;

    if let Ok(manifest) = toml::from_str::<TomlValue>(&String::from_utf8_lossy(&manifest_bytes)) {
        for section in ["dependencies", "dev-dependencies"] {
            let Some(deps) = manifest.get(section).and_then(TomlValue::as_table) else {
                continue;
            };
            for (name, value) in deps {
                if name == "starknet" {
                    continue;
                }
                hasher.update(b"dep:");
                hasher.update(name.as_bytes());
                let path_root = value
                    .as_table()
                    .and_then(|t| t.get("path"))
                    .and_then(TomlValue::as_str)
                    .map(|p| pkg_dir.join(p));
                let src_dir = match path_root {
                    Some(root) => Some(root.join("src")),
                    None => resolve_cached_scarb_registry_dependency_source_root(name, value),
                };
                match src_dir {
                    Some(src_dir) if src_dir.is_dir() => {
                        hasher.update(src_dir.to_string_lossy().as_bytes());
                        let root = src_dir.parent().unwrap_or(&src_dir).to_path_buf();
                        if let Ok(dep_manifest) = fs::read(root.join("Scarb.toml"))
                            .or_else(|_| fs::read(root.join("uc.toml")))
                        {
                            hasher.update(&dep_manifest);
                        }
                        hash_cairo_tree(&mut hasher, &root, &src_dir)?;
                    }
                    _ => {
                        hasher.update(b"missing");
                    }
                }
                hasher.update(&[0xfe]);
            }
        }
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(feature = "native-compile")]
pub(crate) fn check_single_manifest_native(
    manifest_path: &Path,
    package: Option<&str>,
    offline: bool,
    release: bool,
    profile_override: Option<&str>,
) -> Result<CheckPackageResult> {
    let started = Instant::now();
    let workspace_root = metadata_cache_workspace_root(manifest_path)?;
    let profile = if let Some(p) = profile_override {
        p.to_string()
    } else if release {
        "release".to_string()
    } else {
        "dev".to_string()
    };
    let effective_manifest = resolve_native_effective_manifest_path(
        manifest_path,
        &workspace_root,
        package,
    )
    .unwrap_or_else(|_| manifest_path.to_path_buf());
    let fast_digest = compute_fast_package_source_digest(&effective_manifest).ok();
    let pkg_cache_id = short_hash(&effective_manifest.display().to_string());
    let check_cache_path = workspace_root
        .join(".uc")
        .join(format!("check-cache-v1-{profile}-{pkg_cache_id}.json"));

    if let Some(ref digest) = fast_digest {
        if let Ok(raw) = fs::read(&check_cache_path) {
            if let Ok(cached) = serde_json::from_slice::<FastCheckCacheEntry>(&raw) {
                if &cached.source_digest == digest {
                    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                    return Ok(CheckPackageResult {
                        package_name: cached.package_name,
                        manifest_path: effective_manifest.display().to_string(),
                        passed: true,
                        checked_modules: cached.checked_modules,
                        checked_contracts: cached.checked_contracts,
                        diagnostics: Vec::new(),
                        elapsed_ms,
                    });
                }
            }
        }
    }

    let common = BuildCommonArgs {
        manifest_path: Some(manifest_path.to_path_buf()),
        package: package.map(str::to_string),
        workspace: false,
        features: Vec::new(),
        offline,
        release,
        profile: Some(profile),
    };
    let context = with_native_compat_toolchain_fallback(|| {
        build_native_compile_context(&common, manifest_path, &workspace_root)
    })?;
    let signature = native_compile_session_signature(manifest_path, &context);
    let mut diagnostics_vec = Vec::new();
    let mut checked_modules = 0usize;
    let mut checked_contracts = 0usize;

    let check_res = with_native_compile_session(
        &workspace_root,
        &signature,
        false,
        true,
        |session| -> Result<bool> {
            let db = &session.db;
            let root_crate_inputs: Vec<CrateInput> = session
                .main_crate_inputs
                .iter()
                .filter(|input| match input {
                    CrateInput::Real { name, .. } => name == &context.crate_name,
                    _ => true,
                })
                .cloned()
                .collect();
            let crate_ids = CrateInput::into_crate_ids(
                db,
                if root_crate_inputs.is_empty() {
                    session.main_crate_inputs.clone()
                } else {
                    root_crate_inputs
                },
            );
            let contracts = find_contracts(db, &crate_ids);
            checked_contracts = contracts.len();
            let mut modules: Vec<ModuleId<'_>> = crate_ids
                .iter()
                .flat_map(|cid| db.crate_modules(*cid).iter().copied())
                .collect();
            modules.dedup();
            checked_modules = modules.len();

            if modules.len() > 1 {
                modules
                    .par_iter()
                    .for_each_with(db.snapshot(), |snap, module_id| {
                        if let Ok(files) = snap.module_files(*module_id) {
                            for file_id in files.iter().copied() {
                                let _ = snap.file_syntax_diagnostics(file_id);
                            }
                        }
                        let _ = snap.module_semantic_diagnostics(*module_id);
                        let _ = snap.module_lowering_diagnostics(*module_id);
                    });
            }

            let mut has_error = false;
            for module_id in &modules {
                if let Ok(files) = db.module_files(*module_id) {
                    for file_id in files.iter().copied() {
                        let syn = db.file_syntax_diagnostics(file_id);
                        if syn.check_error_free().is_err() {
                            has_error = true;
                            diagnostics_vec.push(syn.format(db));
                        }
                    }
                }
                if let Ok(sem) = db.module_semantic_diagnostics(*module_id) {
                    if sem.check_error_free().is_err() {
                        has_error = true;
                        diagnostics_vec.push(sem.format(db));
                    }
                } else {
                    has_error = true;
                }
                if let Ok(low) = db.module_lowering_diagnostics(*module_id) {
                    if low.check_error_free().is_err() {
                        has_error = true;
                        diagnostics_vec.push(low.format(db));
                    }
                } else {
                    has_error = true;
                }
            }
            Ok(!has_error)
        },
    )?;

    if check_res {
        if let Some(source_digest) = fast_digest {
            if let Some(parent) = check_cache_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let entry = FastCheckCacheEntry {
                source_digest,
                package_name: context.package_name.clone(),
                checked_modules,
                checked_contracts,
            };
            if let Ok(bytes) = serde_json::to_vec(&entry) {
                let _ = fs::write(&check_cache_path, bytes);
            }
        }
    }

    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(CheckPackageResult {
        package_name: context.package_name,
        manifest_path: manifest_path.display().to_string(),
        checked_modules,
        checked_contracts,
        passed: check_res,
        elapsed_ms,
        diagnostics: diagnostics_vec,
    })
}

pub(crate) fn run_check(args: CheckArgs) -> Result<()> {
    #[cfg(not(feature = "native-compile"))]
    {
        let _ = args;
        bail!("`uc check` requires a binary built with the `native-compile` feature");
    }
    #[cfg(feature = "native-compile")]
    {
        let started = Instant::now();
        let manifest_path = resolve_manifest_path(&args.manifest_path)?;
        let workspace_root = metadata_cache_workspace_root(&manifest_path)?;
        let manifests = if args.workspace {
            resolve_all_workspace_member_manifests(&manifest_path, &workspace_root)?
        } else {
            vec![resolve_native_effective_manifest_path(
                &manifest_path,
                &workspace_root,
                args.package.as_deref(),
            )?]
        };

        let mut package_results = Vec::with_capacity(manifests.len());
        for mpath in &manifests {
            let pkg_filter = if manifests.len() == 1 {
                args.package.as_deref()
            } else {
                None
            };
            package_results.push(check_single_manifest_native(
                mpath,
                pkg_filter,
                args.offline,
                args.release,
                args.profile.as_deref(),
            )?);
        }

        let passed = package_results.iter().all(|p| p.passed);
        let total_modules = package_results.iter().map(|p| p.checked_modules).sum();
        let total_contracts = package_results.iter().map(|p| p.checked_contracts).sum();
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

        let report = CheckReport {
            schema_version: UC_AGENT_JSON_SCHEMA_VERSION,
            generated_at_epoch_ms: epoch_ms_u64()?,
            passed,
            total_packages: package_results.len(),
            total_modules,
            total_contracts,
            elapsed_ms,
            packages: package_results,
        };

        if let Some(report_path) = &args.report_path {
            write_json_report(report_path, &report)?;
        }

        if args.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            for pkg in &report.packages {
                for diag in &pkg.diagnostics {
                    eprint!("{diag}");
                }
                println!(
                    "uc check: {} ({} modules, {} contracts) -> {} in {:.1}ms",
                    pkg.package_name,
                    pkg.checked_modules,
                    pkg.checked_contracts,
                    if pkg.passed { "ok" } else { "FAILED" },
                    pkg.elapsed_ms
                );
            }
        }

        if !passed {
            std::process::exit(1);
        }
        Ok(())
    }
}

// ============================================================================
// 3. `uc clean` Implementation
// ============================================================================

fn dir_stats_files_and_bytes(path: &Path) -> (u64, u64) {
    if !path.exists() {
        return (0, 0);
    }
    if path.is_file() {
        return (1, fs::metadata(path).map(|m| m.len()).unwrap_or(0));
    }
    let mut files = 0u64;
    let mut bytes = 0u64;
    for entry in WalkDir::new(path).follow_links(false).into_iter().flatten() {
        if entry.file_type().is_file() {
            files += 1;
            if let Ok(meta) = entry.metadata() {
                bytes = bytes.saturating_add(meta.len());
            }
        }
    }
    (files, bytes)
}

pub(crate) fn run_clean(args: CleanArgs) -> Result<()> {
    let started = Instant::now();
    let manifest_path = resolve_manifest_path(&args.manifest_path)?;
    let workspace_root = metadata_cache_workspace_root(&manifest_path)?;
    let target_dir = args
        .target_dir
        .clone()
        .unwrap_or_else(|| workspace_root.join("target"));
    ensure_path_within_root(&workspace_root, &target_dir, "clean target directory")?;

    let mut candidate_dirs = vec![target_dir];
    if args.all {
        candidate_dirs.push(workspace_root.join(".uc"));
    } else if args.uc_cache {
        candidate_dirs.push(workspace_root.join(".uc/cache"));
        candidate_dirs.push(workspace_root.join(".uc/receipts"));
        candidate_dirs.push(workspace_root.join(".uc/native-project"));
        // `uc check` result cache lives next to the other `.uc` caches.
        if let Ok(entries) = fs::read_dir(workspace_root.join(".uc")) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("check-cache-v") && name.ends_with(".json") {
                    candidate_dirs.push(entry.path());
                }
            }
        }
    }

    let mut removed_paths = Vec::new();
    let mut removed_files = 0u64;
    let mut removed_bytes = 0u64;

    for dir in candidate_dirs {
        if dir.exists() {
            ensure_path_within_root(&workspace_root, &dir, "clean target")?;
            let (f, b) = dir_stats_files_and_bytes(&dir);
            removed_files = removed_files.saturating_add(f);
            removed_bytes = removed_bytes.saturating_add(b);
            removed_paths.push(dir.display().to_string());
            if !args.dry_run {
                if dir.is_file() {
                    fs::remove_file(&dir)
                        .with_context(|| format!("failed to remove {}", dir.display()))?;
                } else {
                    fs::remove_dir_all(&dir)
                        .with_context(|| format!("failed to remove {}", dir.display()))?;
                }
            }
        }
    }

    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let report = CleanReport {
        schema_version: UC_AGENT_JSON_SCHEMA_VERSION,
        generated_at_epoch_ms: epoch_ms_u64()?,
        workspace_root: workspace_root.display().to_string(),
        dry_run: args.dry_run,
        removed_paths,
        removed_files,
        removed_bytes,
        elapsed_ms,
    };

    if let Some(report_path) = &args.report_path {
        write_json_report(report_path, &report)?;
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "uc clean: {} {} file(s) ({:.2} MB) in {:.1}ms",
            if args.dry_run { "would remove" } else { "removed" },
            report.removed_files,
            report.removed_bytes as f64 / (1024.0 * 1024.0),
            report.elapsed_ms
        );
    }
    Ok(())
}

// ============================================================================
// 4. `uc new` & `uc init` Implementation
// ============================================================================

fn validate_cairo_package_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        bail!("package name cannot be empty");
    }
    if trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        bail!("package name `{trimmed}` cannot start with a digit");
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        bail!(
            "package name `{trimmed}` must contain only lowercase ASCII alphanumeric characters or `_`"
        );
    }
    Ok(())
}

fn scaffold_package_at(
    root_dir: &Path,
    name_override: Option<&str>,
    edition: &str,
    starknet: bool,
    legacy_scarb: bool,
    vcs: VcsKindArg,
    no_vcs: bool,
    allow_existing_dir: bool,
) -> Result<ScaffoldReport> {
    if root_dir.exists() && !allow_existing_dir {
        bail!(
            "destination `{}` already exists; use `uc init` to initialize an existing directory",
            root_dir.display()
        );
    }
    fs::create_dir_all(root_dir)
        .with_context(|| format!("failed to create directory {}", root_dir.display()))?;
    let canonical_root = root_dir
        .canonicalize()
        .unwrap_or_else(|_| root_dir.to_path_buf());
    let manifest_filename = if legacy_scarb { "Scarb.toml" } else { "uc.toml" };
    let manifest_path = canonical_root.join(manifest_filename);
    if manifest_path.exists()
        || canonical_root.join("Scarb.toml").exists()
        || canonical_root.join("uc.toml").exists()
    {
        bail!(
            "a Cairo manifest (`Scarb.toml` or `uc.toml`) already exists in `{}`",
            canonical_root.display()
        );
    }

    let raw_name = match name_override {
        Some(n) => n.to_string(),
        None => canonical_root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("cairo_pkg")
            .replace('-', "_")
            .to_ascii_lowercase(),
    };
    validate_cairo_package_name(&raw_name)?;

    let mut created_files = Vec::new();
    let manifest_content = if !legacy_scarb {
        format!(
            r#"[project]
name = "{raw_name}"
version = "0.1.0"
edition = "{edition}"

[dependencies]

[prove]
fail-on = "counterexample"

[test]
fuzzer-runs = 256
"#
        )
    } else if starknet {
        format!(
            r#"[package]
name = "{raw_name}"
version = "0.1.0"
edition = "{edition}"

[dependencies]
starknet = ">=2.14.0"

[[target.starknet-contract]]
sierra = true
casm = true
"#
        )
    } else {
        format!(
            r#"[package]
name = "{raw_name}"
version = "0.1.0"
edition = "{edition}"

[dependencies]
"#
        )
    };
    fs::write(&manifest_path, manifest_content)?;
    created_files.push(manifest_path.display().to_string());

    let gitignore_path = canonical_root.join(".gitignore");
    if !gitignore_path.exists() {
        fs::write(&gitignore_path, "target/\n.uc/\n")?;
        created_files.push(gitignore_path.display().to_string());
    }

    let src_dir = canonical_root.join("src");
    fs::create_dir_all(&src_dir)?;
    let lib_path = src_dir.join("lib.cairo");
    if !lib_path.exists() {
        let lib_content = if starknet {
            r#"#[starknet::interface]
pub trait ICounter<TContractState> {
    fn get_count(self: @TContractState) -> u128;
    fn increment(ref self: TContractState, step: u128);
}

#[starknet::contract]
pub mod Counter {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        count: u128,
    }

    #[abi(embed_v0)]
    impl CounterImpl of super::ICounter<ContractState> {
        fn get_count(self: @ContractState) -> u128 {
            self.count.read()
        }

        fn increment(ref self: ContractState, step: u128) {
            self.count.write(self.count.read() + step);
        }
    }
}
"#
        } else {
            r#"pub fn add(a: u128, b: u128) -> u128 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::add;

    #[test]
    fn test_add() {
        assert(add(2, 3) == 5, 'add failed');
    }
}
"#
        };
        fs::write(&lib_path, lib_content)?;
        created_files.push(lib_path.display().to_string());
    }

    let mut vcs_initialized = false;
    if !no_vcs && vcs == VcsKindArg::Git && !canonical_root.join(".git").exists() {
        if let Ok(status) = Command::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(&canonical_root)
            .status()
        {
            vcs_initialized = status.success();
        }
    }

    Ok(ScaffoldReport {
        schema_version: UC_AGENT_JSON_SCHEMA_VERSION,
        generated_at_epoch_ms: epoch_ms_u64()?,
        package_name: raw_name,
        edition: edition.to_string(),
        starknet,
        root_dir: canonical_root.display().to_string(),
        manifest_path: manifest_path.display().to_string(),
        created_files,
        vcs_initialized,
    })
}

pub(crate) fn run_new(args: NewArgs) -> Result<()> {
    let report = scaffold_package_at(
        &args.path,
        args.name.as_deref(),
        &args.edition,
        args.starknet,
        args.legacy_scarb,
        args.vcs,
        args.no_vcs,
        false,
    )?;
    if let Some(report_path) = &args.report_path {
        write_json_report(report_path, &report)?;
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Created `{}` package ({}) at {}",
            report.package_name,
            if report.starknet {
                "starknet-contract"
            } else {
                "lib"
            },
            report.root_dir
        );
    }
    Ok(())
}

pub(crate) fn run_init(args: InitArgs) -> Result<()> {
    let target_dir = match &args.path {
        Some(p) => p.clone(),
        None => std::env::current_dir()?,
    };
    let report = scaffold_package_at(
        &target_dir,
        args.name.as_deref(),
        &args.edition,
        args.starknet,
        args.legacy_scarb,
        args.vcs,
        args.no_vcs,
        true,
    )?;
    if let Some(report_path) = &args.report_path {
        write_json_report(report_path, &report)?;
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Initialized `{}` package ({}) at {}",
            report.package_name,
            if report.starknet {
                "starknet-contract"
            } else {
                "lib"
            },
            report.root_dir
        );
    }
    Ok(())
}

// ============================================================================
// 5. `uc add` & `uc remove` Implementation
// ============================================================================

fn parse_package_spec(raw: &str) -> Result<(String, Option<String>)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("dependency specification cannot be empty");
    }
    if let Some((name, ver)) = trimmed.split_once('@') {
        let name = name.trim();
        let ver = ver.trim();
        validate_cairo_package_name(name)?;
        if ver.is_empty() {
            bail!("empty version after `@` in `{trimmed}`");
        }
        Ok((name.to_string(), Some(ver.to_string())))
    } else {
        validate_cairo_package_name(trimmed)?;
        Ok((trimmed.to_string(), None))
    }
}

pub(crate) fn run_add(args: AddArgs) -> Result<()> {
    let base_manifest_path = resolve_manifest_path(&args.manifest_path)?;
    let workspace_root = metadata_cache_workspace_root(&base_manifest_path)?;
    #[cfg(feature = "native-compile")]
    let manifest_path = resolve_native_effective_manifest_path(
        &base_manifest_path,
        &workspace_root,
        args.package.as_deref(),
    )
    .unwrap_or(base_manifest_path);
    #[cfg(not(feature = "native-compile"))]
    let manifest_path = base_manifest_path;

    let text = read_text_file_with_limit(&manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
    let mut doc: TomlValue = text
        .parse::<TomlValue>()
        .with_context(|| format!("failed to parse manifest for `uc add` {}", manifest_path.display()))?;
    let root_table = doc
        .as_table_mut()
        .context("Scarb.toml root must be a TOML table")?;
    let section_key = if args.dev {
        "dev-dependencies"
    } else {
        "dependencies"
    };
    let deps_table = root_table
        .entry(section_key.to_string())
        .or_insert_with(|| TomlValue::Table(toml::map::Map::new()))
        .as_table_mut()
        .with_context(|| format!("`[{section_key}]` in Scarb.toml must be a table"))?;

    let mut mutated = Vec::new();
    for spec in &args.packages {
        let (dep_name, explicit_ver) = parse_package_spec(spec)?;
        let dep_val = if args.workspace {
            let mut t = toml::map::Map::new();
            t.insert("workspace".to_string(), TomlValue::Boolean(true));
            TomlValue::Table(t)
        } else if args.path.is_some() || args.git.is_some() {
            let mut t = toml::map::Map::new();
            if let Some(ver) = explicit_ver {
                t.insert("version".to_string(), TomlValue::String(ver));
            }
            if let Some(p) = &args.path {
                t.insert("path".to_string(), TomlValue::String(p.clone()));
            }
            if let Some(git) = &args.git {
                t.insert("git".to_string(), TomlValue::String(git.clone()));
            }
            if let Some(branch) = &args.branch {
                t.insert("branch".to_string(), TomlValue::String(branch.clone()));
            }
            if let Some(tag) = &args.tag {
                t.insert("tag".to_string(), TomlValue::String(tag.clone()));
            }
            if let Some(rev) = &args.rev {
                t.insert("rev".to_string(), TomlValue::String(rev.clone()));
            }
            TomlValue::Table(t)
        } else {
            let default_ver = if dep_name == "starknet" {
                format!(">={}", native_cairo_lang_compiler_version())
            } else {
                "*".to_string()
            };
            TomlValue::String(explicit_ver.unwrap_or(default_ver))
        };
        deps_table.insert(dep_name.clone(), dep_val);
        mutated.push(dep_name);
    }

    if !args.dry_run {
        let mut rendered = toml::to_string_pretty(&doc)?;
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        fs::write(&manifest_path, rendered)?;
    }

    let report = ManifestMutationReport {
        schema_version: UC_AGENT_JSON_SCHEMA_VERSION,
        generated_at_epoch_ms: epoch_ms_u64()?,
        operation: "add".to_string(),
        manifest_path: manifest_path.display().to_string(),
        section: section_key.to_string(),
        dry_run: args.dry_run,
        mutated_dependencies: mutated,
    };

    if let Some(report_path) = &args.report_path {
        write_json_report(report_path, &report)?;
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        for dep in &report.mutated_dependencies {
            println!(
                "uc add: {} `{}` to [{}] in {}",
                if args.dry_run { "would add" } else { "added" },
                dep,
                report.section,
                report.manifest_path
            );
        }
    }
    Ok(())
}

pub(crate) fn run_remove(args: RemoveArgs) -> Result<()> {
    let base_manifest_path = resolve_manifest_path(&args.manifest_path)?;
    let workspace_root = metadata_cache_workspace_root(&base_manifest_path)?;
    #[cfg(feature = "native-compile")]
    let manifest_path = resolve_native_effective_manifest_path(
        &base_manifest_path,
        &workspace_root,
        args.package.as_deref(),
    )
    .unwrap_or(base_manifest_path);
    #[cfg(not(feature = "native-compile"))]
    let manifest_path = base_manifest_path;

    let text = read_text_file_with_limit(&manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
    let mut doc: TomlValue = text
        .parse::<TomlValue>()
        .with_context(|| format!("failed to parse manifest for `uc remove` {}", manifest_path.display()))?;
    let root_table = doc
        .as_table_mut()
        .context("Scarb.toml root must be a TOML table")?;
    let section_key = if args.dev {
        "dev-dependencies"
    } else {
        "dependencies"
    };
    let deps_table = root_table
        .get_mut(section_key)
        .and_then(TomlValue::as_table_mut)
        .with_context(|| format!("section `[{section_key}]` not found in {}", manifest_path.display()))?;

    let mut mutated = Vec::new();
    for dep_name in &args.packages {
        let trimmed = dep_name.trim();
        if deps_table.remove(trimmed).is_none() {
            bail!(
                "dependency `{trimmed}` does not exist in `[{section_key}]` of {}",
                manifest_path.display()
            );
        }
        mutated.push(trimmed.to_string());
    }

    if !args.dry_run {
        let mut rendered = toml::to_string_pretty(&doc)?;
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        fs::write(&manifest_path, rendered)?;
    }

    let report = ManifestMutationReport {
        schema_version: UC_AGENT_JSON_SCHEMA_VERSION,
        generated_at_epoch_ms: epoch_ms_u64()?,
        operation: "remove".to_string(),
        manifest_path: manifest_path.display().to_string(),
        section: section_key.to_string(),
        dry_run: args.dry_run,
        mutated_dependencies: mutated,
    };

    if let Some(report_path) = &args.report_path {
        write_json_report(report_path, &report)?;
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        for dep in &report.mutated_dependencies {
            println!(
                "uc remove: {} `{}` from [{}] in {}",
                if args.dry_run {
                    "would remove"
                } else {
                    "removed"
                },
                dep,
                report.section,
                report.manifest_path
            );
        }
    }
    Ok(())
}

// ============================================================================
// 6. `uc tree` Implementation
// ============================================================================

fn build_tree_node_for_manifest(
    manifest_path: &Path,
    workspace_root: &Path,
    lockfile_versions: &HashMap<String, (Option<String>, Option<String>)>,
    no_dev: bool,
    no_dedupe: bool,
    max_depth: Option<usize>,
    prune_set: &HashSet<String>,
    current_depth: usize,
    seen_packages: &mut HashSet<String>,
) -> Result<TreeNodeReport> {
    let text = read_text_file_with_limit(manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
    let manifest: TomlValue = parse_manifest_toml(&text, manifest_path, "failed to parse manifest")?;
    let pkg_table = manifest
        .get("package")
        .or_else(|| manifest.get("project"))
        .and_then(TomlValue::as_table);
    let pkg_name = pkg_table
        .and_then(|t| t.get("name"))
        .and_then(TomlValue::as_str)
        .unwrap_or("workspace")
        .to_string();
    let pkg_version = pkg_table
        .and_then(|t| t.get("version"))
        .and_then(TomlValue::as_str)
        .map(str::to_string);
    let manifest_dir = manifest_path.parent().unwrap_or(workspace_root);

    let mut children = Vec::new();
    if max_depth.is_some_and(|limit| current_depth >= limit) {
        return Ok(TreeNodeReport {
            name: pkg_name,
            version: pkg_version,
            source: Some(manifest_dir.display().to_string()),
            kind: "package".to_string(),
            deduped: false,
            dependencies: children,
        });
    }

    let mut sections = vec![("dependencies", "normal")];
    if !no_dev {
        sections.push(("dev-dependencies", "dev"));
    }

    for (section, kind_label) in sections {
        let Some(table) = manifest.get(section).and_then(TomlValue::as_table) else {
            continue;
        };
        for (dep_name, dep_val) in table {
            if prune_set.contains(dep_name) {
                continue;
            }
            let already_seen = !seen_packages.insert(dep_name.clone());
            let deduped = !no_dedupe && already_seen;

            let dep_path_opt = dep_val
                .as_table()
                .and_then(|t| t.get("path"))
                .and_then(TomlValue::as_str)
                .map(|p| {
                    let base = manifest_dir.join(p);
                    let scarb = base.join("Scarb.toml");
                    if scarb.is_file() {
                        scarb
                    } else {
                        base.join("uc.toml")
                    }
                });

            if let Some(child_manifest) = dep_path_opt.filter(|p| p.is_file()) {
                if deduped {
                    let child_text = fs::read_to_string(&child_manifest).unwrap_or_default();
                    let child_ver = toml::from_str::<TomlValue>(&child_text)
                        .ok()
                        .and_then(|v| {
                            v.get("package")
                                .or_else(|| v.get("project"))?
                                .get("version")?
                                .as_str()
                                .map(str::to_string)
                        });
                    children.push(TreeNodeReport {
                        name: dep_name.clone(),
                        version: child_ver,
                        source: Some(
                            child_manifest
                                .parent()
                                .unwrap_or(workspace_root)
                                .display()
                                .to_string(),
                        ),
                        kind: kind_label.to_string(),
                        deduped: true,
                        dependencies: Vec::new(),
                    });
                } else if let Ok(mut child_node) = build_tree_node_for_manifest(
                    &child_manifest,
                    workspace_root,
                    lockfile_versions,
                    no_dev,
                    no_dedupe,
                    max_depth,
                    prune_set,
                    current_depth + 1,
                    seen_packages,
                ) {
                    child_node.kind = kind_label.to_string();
                    children.push(child_node);
                }
                continue;
            }

            let (lock_ver, lock_src) = lockfile_versions
                .get(dep_name)
                .cloned()
                .unwrap_or((None, None));
            let declared_ver = match dep_val {
                TomlValue::String(s) => Some(s.clone()),
                TomlValue::Table(t) => t
                    .get("version")
                    .and_then(TomlValue::as_str)
                    .map(str::to_string),
                _ => None,
            };
            let effective_ver = lock_ver.or(declared_ver).or_else(|| {
                (dep_name == "starknet")
                    .then(|| native_cairo_lang_compiler_version().to_string())
            });
            children.push(TreeNodeReport {
                name: dep_name.clone(),
                version: effective_ver,
                source: lock_src,
                kind: kind_label.to_string(),
                deduped,
                dependencies: Vec::new(),
            });
        }
    }

    Ok(TreeNodeReport {
        name: pkg_name,
        version: pkg_version,
        source: Some(manifest_dir.display().to_string()),
        kind: "package".to_string(),
        deduped: false,
        dependencies: children,
    })
}

fn render_tree_node(
    node: &TreeNodeReport,
    prefix: &str,
    is_last: bool,
    is_root: bool,
    charset: TreeCharsetArg,
    out: &mut String,
) {
    let ver_str = node
        .version
        .as_deref()
        .map(|v| format!(" v{v}"))
        .unwrap_or_default();
    let src_str = if is_root {
        node.source
            .as_deref()
            .map(|s| format!(" ({s})"))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let dedupe_str = if node.deduped { " (*)" } else { "" };
    let dev_str = if node.kind == "dev" { " [dev]" } else { "" };

    if is_root {
        out.push_str(&format!("{}{}{}{}\n", node.name, ver_str, src_str, dedupe_str));
    } else {
        let connector = match (charset, is_last) {
            (TreeCharsetArg::Utf8, true) => "└── ",
            (TreeCharsetArg::Utf8, false) => "├── ",
            (TreeCharsetArg::Ascii, true) => "`-- ",
            (TreeCharsetArg::Ascii, false) => "|-- ",
        };
        out.push_str(&format!(
            "{}{}{}{}{}{}\n",
            prefix, connector, node.name, ver_str, dev_str, dedupe_str
        ));
    }

    let next_prefix = if is_root {
        String::new()
    } else {
        let bar = match (charset, is_last) {
            (_, true) => "    ",
            (TreeCharsetArg::Utf8, false) => "│   ",
            (TreeCharsetArg::Ascii, false) => "|   ",
        };
        format!("{prefix}{bar}")
    };

    for (idx, child) in node.dependencies.iter().enumerate() {
        let child_last = idx + 1 == node.dependencies.len();
        render_tree_node(child, &next_prefix, child_last, false, charset, out);
    }
}

pub(crate) fn run_tree(args: TreeArgs) -> Result<()> {
    let manifest_path = resolve_manifest_path(&args.manifest_path)?;
    let workspace_root = metadata_cache_workspace_root(&manifest_path)?;
    let inspect =
        project_inspect_report_from_manifest_path_with_native_support(&manifest_path, false)?;
    let mut lockfile_versions = HashMap::new();
    for pkg in &inspect.lockfile.packages {
        lockfile_versions.insert(
            pkg.name.clone(),
            (pkg.version.clone(), pkg.source.clone()),
        );
    }

    #[cfg(feature = "native-compile")]
    let target_manifests = if let Some(pkg) = args.package.as_deref() {
        vec![resolve_native_effective_manifest_path(
            &manifest_path,
            &workspace_root,
            Some(pkg),
        )?]
    } else {
        resolve_all_workspace_member_manifests(&manifest_path, &workspace_root)?
    };
    #[cfg(not(feature = "native-compile"))]
    let target_manifests = vec![manifest_path.clone()];

    let prune_set: HashSet<String> = args.prune.iter().cloned().collect();
    let mut roots = Vec::new();
    let mut rendered_tree = String::new();
    for mpath in &target_manifests {
        let mut seen = HashSet::new();
        let node = build_tree_node_for_manifest(
            mpath,
            &workspace_root,
            &lockfile_versions,
            args.no_dev,
            args.no_dedupe,
            args.depth,
            &prune_set,
            0,
            &mut seen,
        )?;
        render_tree_node(&node, "", true, true, args.charset, &mut rendered_tree);
        roots.push(node);
    }

    let report = TreeReport {
        schema_version: UC_AGENT_JSON_SCHEMA_VERSION,
        generated_at_epoch_ms: epoch_ms_u64()?,
        manifest_path: manifest_path.display().to_string(),
        roots,
        rendered_tree: rendered_tree.clone(),
    };

    if let Some(report_path) = &args.report_path {
        write_json_report(report_path, &report)?;
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{rendered_tree}");
    }
    Ok(())
}

// ============================================================================
// 7. `uc expand` Implementation
// ============================================================================

pub(crate) fn run_expand(args: ExpandArgs) -> Result<()> {
    #[cfg(not(feature = "native-compile"))]
    {
        let _ = args;
        bail!("`uc expand` requires a binary built with the `native-compile` feature");
    }
    #[cfg(feature = "native-compile")]
    {
        let started = Instant::now();
        let manifest_path = resolve_manifest_path(&args.manifest_path)?;
        let workspace_root = metadata_cache_workspace_root(&manifest_path)?;
        let common = BuildCommonArgs {
            manifest_path: Some(manifest_path.clone()),
            package: None,
            workspace: false,
            features: Vec::new(),
            offline: false,
            release: false,
            profile: Some("dev".to_string()),
        };
        let context = build_native_compile_context(&common, &manifest_path, &workspace_root)?;
        let signature = native_compile_session_signature(&manifest_path, &context);

        let modules = with_native_compile_session(
            &workspace_root,
            &signature,
            false,
            false,
            |session| -> Result<Vec<ExpandedModuleReport>> {
                let db = &session.db;
                let root_crate_inputs: Vec<CrateInput> = if args.module.is_none() {
                    session
                        .main_crate_inputs
                        .iter()
                        .filter(|input| match input {
                            CrateInput::Real { name, .. } => name == &context.crate_name,
                            _ => false,
                        })
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };
                let crate_ids = CrateInput::into_crate_ids(
                    db,
                    if root_crate_inputs.is_empty() {
                        session.main_crate_inputs.clone()
                    } else {
                        root_crate_inputs
                    },
                );
                let all_modules: Vec<ModuleId<'_>> = crate_ids
                    .iter()
                    .flat_map(|cid| db.crate_modules(*cid).iter().copied())
                    .collect();

                let mut expanded_modules = Vec::new();
                for module_id in all_modules {
                    let full_path = module_id.full_path(db);
                    if let Some(mod_filter) = &args.module {
                        if !full_path.contains(mod_filter) {
                            continue;
                        }
                    }
                    if let Some(contract_filter) = &args.contract {
                        if !full_path.ends_with(&format!("::{contract_filter}"))
                            && !full_path.contains(&format!("::{contract_filter}::"))
                            && full_path != *contract_filter
                        {
                            continue;
                        }
                    }
                    let Ok(files) = db.module_files(module_id) else {
                        continue;
                    };
                    let mut combined = String::new();
                    let mut virtual_files_count = 0usize;
                    for (idx, file_id) in files.iter().copied().enumerate() {
                        if idx > 0 {
                            virtual_files_count += 1;
                            combined.push_str(&format!(
                                "\n// --- expanded plugin file: {} ---\n",
                                file_id.file_name(db).to_string(db)
                            ));
                        }
                        if let Some(content) = db.file_content(file_id) {
                            combined.push_str(content);
                            if !combined.ends_with('\n') {
                                combined.push('\n');
                            }
                        }
                    }
                    let expanded_code = if args.raw {
                        combined
                    } else {
                        cairo_lang_formatter::format_string(db, combined)
                    };
                    expanded_modules.push(ExpandedModuleReport {
                        module_path: full_path,
                        virtual_files_count,
                        expanded_code,
                    });
                }
                Ok(expanded_modules)
            },
        )?;

        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        let report = ExpandReport {
            schema_version: UC_AGENT_JSON_SCHEMA_VERSION,
            generated_at_epoch_ms: epoch_ms_u64()?,
            package_name: context.package_name,
            manifest_path: manifest_path.display().to_string(),
            elapsed_ms,
            modules,
        };

        if let Some(report_path) = &args.report_path {
            write_json_report(report_path, &report)?;
        }
        if args.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            for m in &report.modules {
                println!(
                    "// ============================================================================"
                );
                println!(
                    "// Module: {} ({} macro/plugin expansion(s))",
                    m.module_path, m.virtual_files_count
                );
                println!(
                    "// ============================================================================"
                );
                print!("{}", m.expanded_code);
            }
        }
        Ok(())
    }
}

// ============================================================================
// 8. `uc lint` Implementation (Cairo + Starknet Security/Gas Static Analyzer)
// ============================================================================

fn scan_cairo_file_for_starknet_lints(
    relative_path: &str,
    source_text: &str,
    findings: &mut Vec<LintFinding>,
) {
    let lines: Vec<&str> = source_text.lines().collect();
    let mut in_loop_depth = 0usize;
    let mut loop_brace_stack: Vec<usize> = Vec::new();
    let mut brace_depth = 0usize;

    let mut current_fn_is_external_or_ctor = false;
    let mut current_fn_has_address_param = false;
    let mut current_fn_checks_zero = false;
    let mut current_fn_writes_storage = false;
    let mut current_fn_checks_caller = false;
    let mut current_fn_line = 0usize;
    let mut current_fn_name = String::new();
    let mut current_fn_brace_start: Option<usize> = None;
    let mut pending_external_or_ctor_attr = false;

    let mut pending_event_derive = false;
    let mut in_event_struct = false;
    let mut event_struct_line = 0usize;
    let mut event_struct_name = String::new();
    let mut event_struct_has_key = false;
    let mut event_struct_field_count = 0usize;

    for (idx, raw_line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw_line.trim();
        if trimmed.starts_with("//") {
            continue;
        }

        if trimmed.starts_with("#[external")
            || trimmed.starts_with("#[constructor]")
            || trimmed.starts_with("#[abi(embed_v0)]")
        {
            pending_external_or_ctor_attr = true;
        }
        if trimmed.contains("starknet::Event") && trimmed.starts_with("#[derive") {
            pending_event_derive = true;
        }

        // Check function declarations (ignoring inline comments and string literals)
        let code_no_comment = trimmed.split("//").next().unwrap_or(trimmed).trim();
        let after_vis = code_no_comment
            .strip_prefix("pub(crate) ")
            .or_else(|| code_no_comment.strip_prefix("pub "))
            .or_else(|| code_no_comment.strip_prefix("extern "))
            .or_else(|| code_no_comment.strip_prefix("const "))
            .unwrap_or(code_no_comment)
            .trim_start();
        if let Some(after_fn) = after_vis.strip_prefix("fn ") {
            let fn_pos = raw_line.find("fn ").unwrap_or(0);
            let fn_name: String = after_fn
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !fn_name.is_empty() && fn_name.chars().any(|c| c.is_ascii_uppercase()) {
                let mut snake = String::with_capacity(fn_name.len() + 4);
                for (ch_idx, ch) in fn_name.chars().enumerate() {
                    if ch.is_ascii_uppercase() {
                        if ch_idx > 0 && !snake.ends_with('_') {
                            snake.push('_');
                        }
                        snake.push(ch.to_ascii_lowercase());
                    } else {
                        snake.push(ch);
                    }
                }
                findings.push(LintFinding {
                    rule_id: "CAIRO_NAMING_CONVENTION".to_string(),
                    severity: "warning".to_string(),
                    category: "style".to_string(),
                    file: relative_path.to_string(),
                    line: line_no,
                    column: fn_pos + 1,
                    message: format!("Function `{fn_name}` should use `snake_case` naming."),
                    suggestion: format!("Rename `{fn_name}` to `{snake}`."),
                });
            }
            if !trimmed.ends_with(';') && current_fn_brace_start.is_none() {
                current_fn_name = fn_name;
                current_fn_line = line_no;
                current_fn_is_external_or_ctor =
                    pending_external_or_ctor_attr || trimmed.contains("ref self: ContractState");
                current_fn_has_address_param = trimmed.contains("ContractAddress");
                current_fn_checks_zero = false;
                current_fn_writes_storage = false;
                current_fn_checks_caller = false;
                current_fn_brace_start = Some(brace_depth);
                pending_external_or_ctor_attr = false;
            }
        }

        if pending_event_derive && trimmed.starts_with("struct ") {
            let struct_name: String = trimmed["struct ".len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            in_event_struct = true;
            event_struct_line = line_no;
            event_struct_name = struct_name;
            event_struct_has_key = false;
            event_struct_field_count = 0;
            pending_event_derive = false;
        } else if in_event_struct {
            if trimmed.starts_with("#[key]") {
                event_struct_has_key = true;
            } else if trimmed.contains(':') && !trimmed.starts_with('#') {
                event_struct_field_count += 1;
            }
        }

        // Loop detection
        if trimmed.starts_with("while ")
            || trimmed.starts_with("loop ")
            || trimmed == "loop {"
            || trimmed.starts_with("for ")
        {
            loop_brace_stack.push(brace_depth);
            in_loop_depth += 1;
        }

        if in_loop_depth > 0
            && (trimmed.contains("self.") && (trimmed.contains(".read(") || trimmed.contains(".write(")))
        {
            findings.push(LintFinding {
                rule_id: "SN_STORAGE_IN_LOOP".to_string(),
                severity: "warning".to_string(),
                category: "gas_optimization".to_string(),
                file: relative_path.to_string(),
                line: line_no,
                column: 1,
                message: "Contract storage `.read()` / `.write()` inside a loop triggers repeated Starknet storage syscalls.".to_string(),
                suggestion: "Cache the storage value in a local variable before the loop and write back once after the loop completes.".to_string(),
            });
        }

        if current_fn_brace_start.is_some() {
            if trimmed.contains("is_zero()") || trimmed.contains("!= 0") {
                current_fn_checks_zero = true;
            }
            if trimmed.contains("get_caller_address()") || trimmed.contains("assert_only_") {
                current_fn_checks_caller = true;
            }
            if trimmed.contains("self.") && trimmed.contains(".write(") {
                current_fn_writes_storage = true;
            }
        }

        for ch in trimmed.chars() {
            if ch == '{' {
                brace_depth += 1;
            } else if ch == '}' {
                brace_depth = brace_depth.saturating_sub(1);
                if loop_brace_stack.last().copied() == Some(brace_depth) {
                    loop_brace_stack.pop();
                    in_loop_depth = in_loop_depth.saturating_sub(1);
                }
                if in_event_struct && brace_depth <= 1 {
                    if event_struct_field_count >= 2 && !event_struct_has_key {
                        findings.push(LintFinding {
                            rule_id: "SN_EVENT_UNINDEXED".to_string(),
                            severity: "info".to_string(),
                            category: "starknet_best_practice".to_string(),
                            file: relative_path.to_string(),
                            line: event_struct_line,
                            column: 1,
                            message: format!(
                                "Starknet event struct `{event_struct_name}` has {event_struct_field_count} fields but no `#[key]` indexed field."
                            ),
                            suggestion: "Annotate primary identifier fields (such as `owner`, `from`, `to`, or `caller`) with `#[key]` for efficient Starknet event indexing.".to_string(),
                        });
                    }
                    in_event_struct = false;
                }
                if let Some(start_depth) = current_fn_brace_start {
                    if brace_depth <= start_depth {
                        if current_fn_is_external_or_ctor
                            && current_fn_writes_storage
                            && current_fn_has_address_param
                            && !current_fn_checks_zero
                        {
                            findings.push(LintFinding {
                                rule_id: "SN_UNCHECKED_ZERO_ADDRESS".to_string(),
                                severity: "warning".to_string(),
                                category: "security".to_string(),
                                file: relative_path.to_string(),
                                line: current_fn_line,
                                column: 1,
                                message: format!(
                                    "Entrypoint `{current_fn_name}` accepts a `ContractAddress` and mutates storage without checking `.is_zero()`."
                                ),
                                suggestion: "Validate that incoming `ContractAddress` parameters are non-zero (`assert(!addr.is_zero(), 'zero address')`) before persisting state.".to_string(),
                            });
                        }
                        if current_fn_is_external_or_ctor
                            && current_fn_name != "constructor"
                            && current_fn_writes_storage
                            && !current_fn_checks_caller
                        {
                            findings.push(LintFinding {
                                rule_id: "SN_UNPROTECTED_STATE_MUTATION".to_string(),
                                severity: "info".to_string(),
                                category: "security".to_string(),
                                file: relative_path.to_string(),
                                line: current_fn_line,
                                column: 1,
                                message: format!(
                                    "External function `{current_fn_name}` writes to contract storage without reading `get_caller_address()`."
                                ),
                                suggestion: "Verify whether this state mutation is intentionally permissionless or requires caller access control.".to_string(),
                            });
                        }
                        current_fn_brace_start = None;
                    }
                }
            }
        }
    }
}

pub(crate) fn run_lint(args: LintArgs) -> Result<()> {
    #[cfg(not(feature = "native-compile"))]
    {
        let _ = args;
        bail!("`uc lint` requires a binary built with the `native-compile` feature");
    }
    #[cfg(feature = "native-compile")]
    {
        let started = Instant::now();
        let manifest_path = resolve_manifest_path(&args.manifest_path)?;
        let workspace_root = metadata_cache_workspace_root(&manifest_path)?;
        let common = BuildCommonArgs {
            manifest_path: Some(manifest_path.clone()),
            package: args.package.clone(),
            workspace: args.workspace,
            features: Vec::new(),
            offline: false,
            release: false,
            profile: Some("dev".to_string()),
        };
        let context = build_native_compile_context(&common, &manifest_path, &workspace_root)?;
        let mut findings = Vec::new();

        // 1. Run full compiler diagnostics with unused-import warnings enabled!
        let mut db = native_seeded_root_database(&context.corelib_src, false)?;
        let main_crate_inputs = native_setup_project(&mut db, &context.cairo_project_dir)?;
        let crate_ids = CrateInput::into_crate_ids(&db, main_crate_inputs);
        for cid in &crate_ids {
            for module_id in db.crate_modules(*cid).iter().copied() {
                if let Ok(files) = db.module_files(module_id) {
                    for file_id in files.iter().copied() {
                        let syn = db.file_syntax_diagnostics(file_id);
                        for entry in syn.format_with_severity(&db, &Default::default()) {
                            findings.push(LintFinding {
                                rule_id: "CAIRO_SYNTAX".to_string(),
                                severity: if syn.check_error_free().is_err() {
                                    "error".to_string()
                                } else {
                                    "warning".to_string()
                                },
                                category: "compiler".to_string(),
                                file: file_id.full_path(&db),
                                line: 1,
                                column: 1,
                                message: entry.message().trim().to_string(),
                                suggestion: "Fix the syntax issue reported by the Cairo parser."
                                    .to_string(),
                            });
                        }
                    }
                }
                if let Ok(sem) = db.module_semantic_diagnostics(module_id) {
                    let is_err = sem.check_error_free().is_err();
                    for entry in sem.format_with_severity(&db, &Default::default()) {
                        findings.push(LintFinding {
                            rule_id: "CAIRO_SEMANTIC".to_string(),
                            severity: if is_err {
                                "error".to_string()
                            } else {
                                "warning".to_string()
                            },
                            category: "compiler".to_string(),
                            file: module_id.full_path(&db),
                            line: 1,
                            column: 1,
                            message: entry.message().trim().to_string(),
                            suggestion:
                                "Resolve the semantic diagnostic reported by the Cairo compiler."
                                    .to_string(),
                        });
                    }
                }
                if let Ok(low) = db.module_lowering_diagnostics(module_id) {
                    let is_err = low.check_error_free().is_err();
                    for entry in low.format_with_severity(&db, &Default::default()) {
                        findings.push(LintFinding {
                            rule_id: "CAIRO_LOWERING".to_string(),
                            severity: if is_err {
                                "error".to_string()
                            } else {
                                "warning".to_string()
                            },
                            category: "compiler".to_string(),
                            file: module_id.full_path(&db),
                            line: 1,
                            column: 1,
                            message: entry.message().trim().to_string(),
                            suggestion:
                                "Resolve the lowering diagnostic reported by the Cairo compiler."
                                    .to_string(),
                        });
                    }
                }
            }
        }

        // 2. Run Starknet & Cairo static analysis rules across all tracked `.cairo` files
        let source_roots = native_compile_source_roots(&context);
        let (tracked_sources, _) = native_collect_tracked_sources(&workspace_root, &source_roots)?;
        let mut fixes_applied = 0usize;
        for rel_path in tracked_sources.keys() {
            let abs_path = workspace_root.join(rel_path);
            if let Ok(source_text) = fs::read_to_string(&abs_path) {
                scan_cairo_file_for_starknet_lints(rel_path, &source_text, &mut findings);
                if args.fix || args.diff {
                    let mut fixed_text = source_text.clone();
                    if fixed_text.contains("LegacyMap<") {
                        fixed_text = fixed_text.replace("LegacyMap<", "starknet::storage::Map<");
                        fixes_applied += 1;
                    }
                    if fixed_text.contains(" == true") {
                        fixed_text = fixed_text.replace(" == true", "");
                        fixes_applied += 1;
                    }
                    if fixed_text != source_text {
                        if args.diff && !args.json {
                            println!("--- a/{rel_path}\n+++ b/{rel_path}");
                            for (o_line, n_line) in source_text.lines().zip(fixed_text.lines()) {
                                if o_line != n_line {
                                    println!("-{o_line}\n+{n_line}");
                                }
                            }
                        }
                        if args.fix {
                            let _ = fs::write(&abs_path, &fixed_text);
                        }
                    }
                }
            }
        }

        let error_count = findings.iter().filter(|f| f.severity == "error").count();
        let warning_count = findings.iter().filter(|f| f.severity == "warning").count();
        let info_count = findings.iter().filter(|f| f.severity == "info").count();
        let passed = error_count == 0 && (!args.deny_warnings || warning_count == 0);
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

        let report = LintReport {
            schema_version: UC_AGENT_JSON_SCHEMA_VERSION,
            generated_at_epoch_ms: epoch_ms_u64()?,
            manifest_path: manifest_path.display().to_string(),
            passed,
            error_count,
            warning_count,
            info_count,
            fixes_applied,
            elapsed_ms,
            findings,
        };

        if let Some(report_path) = &args.report_path {
            write_json_report(report_path, &report)?;
        }

        if args.sarif {
            let sarif_results: Vec<serde_json::Value> = report
                .findings
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "ruleId": f.rule_id,
                        "level": match f.severity.as_str() {
                            "error" => "error",
                            "warning" => "warning",
                            _ => "note",
                        },
                        "message": { "text": format!("{} ({})", f.message, f.suggestion) },
                        "locations": [{
                            "physicalLocation": {
                                "artifactLocation": { "uri": f.file },
                                "region": { "startLine": f.line, "startColumn": f.column }
                            }
                        }]
                    })
                })
                .collect();
            let sarif = serde_json::json!({
                "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
                "version": "2.1.0",
                "runs": [{
                    "tool": {
                        "driver": {
                            "name": "uc-lint",
                            "version": env!("CARGO_PKG_VERSION"),
                        }
                    },
                    "results": sarif_results
                }]
            });
            println!("{}", serde_json::to_string_pretty(&sarif)?);
        } else if args.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            for f in &report.findings {
                println!(
                    "[{}] {} ({}:{}:{}): {}\n  help: {}",
                    f.severity.to_ascii_uppercase(),
                    f.rule_id,
                    f.file,
                    f.line,
                    f.column,
                    f.message,
                    f.suggestion
                );
            }
            println!(
                "uc lint: {} error(s), {} warning(s), {} info note(s) in {:.1}ms",
                report.error_count, report.warning_count, report.info_count, report.elapsed_ms
            );
        }

        if !passed {
            std::process::exit(1);
        }
        Ok(())
    }
}

// ============================================================================
// 9. `uc profile` / `uc gas-report` Implementation
// ============================================================================

#[cfg(feature = "native-compile")]
fn resolve_nextgen_profile(profile: Option<&str>, release: bool) -> String {
    if let Some(p) = profile.map(str::trim).filter(|p| !p.is_empty()) {
        p.to_string()
    } else if release {
        "release".to_string()
    } else {
        "dev".to_string()
    }
}

#[cfg(feature = "native-compile")]
fn find_starknet_artifacts_manifests(target_dir: &Path) -> Vec<PathBuf> {
    let mut manifests = Vec::new();
    if let Ok(entries) = fs::read_dir(target_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(".starknet_artifacts.json"))
            {
                manifests.push(path);
            }
        }
    }
    manifests.sort();
    manifests
}

#[cfg(feature = "native-compile")]
fn analyze_contract_sierra_entrypoints(
    contract_class: &ContractClass,
) -> Vec<EntrypointProfileSummary> {
    let Ok(extracted) = contract_class.extract_sierra_program(true) else {
        return Vec::new();
    };
    let program = &extracted.program;

    // Map ConcreteLibfuncId debug/numeric id -> libfunc long id string
    let mut libfunc_names: HashMap<u64, String> = HashMap::new();
    for decl in &program.libfunc_declarations {
        libfunc_names.insert(decl.id.id, decl.long_id.to_string());
    }

    // Sort functions by entry_point.0 to determine statement ranges [start, end)
    let mut fn_spans: Vec<(usize, usize, String)> = program
        .funcs
        .iter()
        .map(|f| {
            let name = f
                .id
                .debug_name
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("fn_{}", f.id.id));
            (f.id.id as usize, f.entry_point.0, name)
        })
        .collect();
    fn_spans.sort_by_key(|(_, entry_stmt, _)| *entry_stmt);

    let total_stmts = program.statements.len();
    let mut fn_stmt_ranges: HashMap<usize, (usize, usize, String)> = HashMap::new();
    for i in 0..fn_spans.len() {
        let (fn_id, start, ref name) = fn_spans[i];
        let end = if i + 1 < fn_spans.len() {
            fn_spans[i + 1].1
        } else {
            total_stmts
        };
        fn_stmt_ranges.insert(fn_id, (start, end.max(start), name.clone()));
    }

    // Build per-function call edges and syscall counters
    let mut fn_callees: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut fn_metrics: HashMap<usize, (usize, usize, usize, usize, usize, usize)> = HashMap::new();

    for (&fn_id, &(start, end, _)) in &fn_stmt_ranges {
        let mut callees = Vec::new();
        let mut s_reads = 0usize;
        let mut s_writes = 0usize;
        let mut c_calls = 0usize;
        let mut events = 0usize;
        let mut crypto = 0usize;

        for stmt_idx in start..end.min(total_stmts) {
            if let cairo_lang_sierra::program::GenStatement::Invocation(inv) =
                &program.statements[stmt_idx]
            {
                if let Some(lf_name) = libfunc_names.get(&inv.libfunc_id.id) {
                    if lf_name.starts_with("function_call<user@") {
                        // Find target function by matching debug_name or id
                        let callee_label = lf_name
                            .strip_prefix("function_call<user@")
                            .and_then(|s| s.strip_suffix('>'))
                            .unwrap_or("");
                        for &(cand_id, _, ref cand_name) in &fn_spans {
                            if cand_name == callee_label || format!("fn_{cand_id}") == callee_label
                            {
                                callees.push(cand_id);
                                break;
                            }
                        }
                    } else if lf_name.contains("storage_read_syscall") {
                        s_reads += 1;
                    } else if lf_name.contains("storage_write_syscall") {
                        s_writes += 1;
                    } else if lf_name.contains("call_contract_syscall")
                        || lf_name.contains("library_call_syscall")
                    {
                        c_calls += 1;
                    } else if lf_name.contains("emit_event_syscall") {
                        events += 1;
                    } else if lf_name.contains("poseidon")
                        || lf_name.contains("pedersen")
                        || lf_name.contains("keccak")
                        || lf_name.contains("ec_op")
                    {
                        crypto += 1;
                    }
                }
            }
        }
        callees.sort_unstable();
        callees.dedup();
        fn_callees.insert(fn_id, callees);
        fn_metrics.insert(
            fn_id,
            (
                end.saturating_sub(start),
                s_reads,
                s_writes,
                c_calls,
                events,
                crypto,
            ),
        );
    }

    // Build selector -> ABI function name map so entrypoints always show human-readable Starknet names
    let mut selector_to_abi_name: HashMap<String, String> = HashMap::new();
    selector_to_abi_name.insert(
        format!(
            "0x{:x}",
            cairo_lang_starknet_classes::keccak::starknet_keccak(b"constructor")
        ),
        "constructor".to_string(),
    );
    if let Some(abi) = &contract_class.abi {
        if let Ok(abi_val) = serde_json::to_value(abi) {
            if let Some(arr) = abi_val.as_array() {
                for item in arr {
                    let ty = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    if ty == "function" || ty == "l1_handler" || ty == "constructor" {
                        if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                            let sel = format!(
                                "0x{:x}",
                                cairo_lang_starknet_classes::keccak::starknet_keccak(
                                    name.as_bytes()
                                )
                            );
                            selector_to_abi_name.insert(sel, name.to_string());
                        }
                    } else if ty == "interface" {
                        if let Some(items) = item.get("items").and_then(|v| v.as_array()) {
                            for sub in items {
                                if let Some(name) = sub.get("name").and_then(|v| v.as_str()) {
                                    let sel = format!(
                                        "0x{:x}",
                                        cairo_lang_starknet_classes::keccak::starknet_keccak(
                                            name.as_bytes()
                                        )
                                    );
                                    selector_to_abi_name.insert(sel, name.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut summaries = Vec::new();
    let ep_groups = [
        ("external", &contract_class.entry_points_by_type.external),
        (
            "constructor",
            &contract_class.entry_points_by_type.constructor,
        ),
        (
            "l1_handler",
            &contract_class.entry_points_by_type.l1_handler,
        ),
    ];

    for (kind, eps) in ep_groups {
        for ep in eps {
            let root_fn = ep.function_idx;
            let mut visited = HashSet::new();
            let mut queue = VecDeque::new();
            visited.insert(root_fn);
            queue.push_back(root_fn);

            let mut total_sierra = 0usize;
            let mut total_reads = 0usize;
            let mut total_writes = 0usize;
            let mut total_calls = 0usize;
            let mut total_events = 0usize;
            let mut total_crypto = 0usize;

            while let Some(curr) = queue.pop_front() {
                if let Some(&(stmts, r, w, c, ev, cr)) = fn_metrics.get(&curr) {
                    total_sierra += stmts;
                    total_reads += r;
                    total_writes += w;
                    total_calls += c;
                    total_events += ev;
                    total_crypto += cr;
                }
                if let Some(children) = fn_callees.get(&curr) {
                    for &child in children {
                        if visited.insert(child) {
                            queue.push_back(child);
                        }
                    }
                }
            }

            let selector_hex = format!("0x{:x}", ep.selector);
            let raw_name = selector_to_abi_name
                .get(&selector_hex)
                .cloned()
                .or_else(|| {
                    fn_stmt_ranges.get(&root_fn).map(|(_, _, n)| {
                        n.rsplit_once("::")
                            .map(|(_, short)| short.to_string())
                            .unwrap_or_else(|| n.clone())
                    })
                })
                .unwrap_or_else(|| format!("entrypoint_{root_fn}"));

            let estimated_l2_gas_weight = (total_sierra as u64) * 100
                + (total_reads as u64) * 10_000
                + (total_writes as u64) * 25_000
                + (total_calls as u64) * 35_000
                + (total_events as u64) * 5_000
                + (total_crypto as u64) * 2_500;

            summaries.push(EntrypointProfileSummary {
                kind: kind.to_string(),
                name: raw_name,
                selector: selector_hex,
                function_idx: root_fn,
                reachable_functions: visited.len(),
                reachable_sierra_statements: total_sierra,
                storage_reads: total_reads,
                storage_writes: total_writes,
                contract_calls: total_calls,
                events_emitted: total_events,
                crypto_ops: total_crypto,
                estimated_l2_gas_weight,
            });
        }
    }

    summaries.sort_by(|a, b| b.estimated_l2_gas_weight.cmp(&a.estimated_l2_gas_weight));
    summaries
}

pub(crate) fn run_profile(args: ProfileArgs) -> Result<()> {
    #[cfg(not(feature = "native-compile"))]
    {
        let _ = args;
        bail!("`uc profile` requires a binary built with the `native-compile` feature");
    }
    #[cfg(feature = "native-compile")]
    {
        let started = Instant::now();
        let manifest_path = resolve_manifest_path(&args.manifest_path)?;
        let workspace_root = metadata_cache_workspace_root(&manifest_path)?;
        let profile = resolve_nextgen_profile(args.profile.as_deref(), args.release);
        let target_dir = workspace_root.join("target").join(&profile);

        // Ensure artifacts exist; trigger native build if missing
        let mut artifacts_manifests = find_starknet_artifacts_manifests(&target_dir);
        if artifacts_manifests.is_empty() {
            let build_args = BuildArgs {
                common: BuildCommonArgs {
                    manifest_path: Some(manifest_path.clone()),
                    package: None,
                    workspace: false,
                    features: Vec::new(),
                    offline: false,
                    release: args.release,
                    profile: Some(profile.clone()),
                },
                engine: EngineArg::Uc,
                daemon_mode: DaemonModeArg::Auto,
                report_path: None,
                plan_only: false,
                json: false,
                record_failure: None,
            };
            let _ = run_build_entry(build_args);
            artifacts_manifests = find_starknet_artifacts_manifests(&target_dir);
        }

        let max_casm = native_max_casm_bytecode_size();
        let mut contract_summaries = Vec::new();

        for am_path in artifacts_manifests {
            let bytes = fs::read(&am_path)?;
            let Ok(manifest) = serde_json::from_slice::<StarknetArtifactsManifest>(&bytes) else {
                continue;
            };
            for entry in manifest.contracts {
                if let Some(filter) = &args.target {
                    if !entry.contract_name.contains(filter) && !entry.module_path.contains(filter)
                    {
                        continue;
                    }
                }
                let sierra_path = target_dir.join(&entry.artifacts.sierra);
                if !sierra_path.is_file() {
                    continue;
                }
                let raw_sierra = fs::read(&sierra_path)?;
                let sierra_blake3 = blake3::hash(&raw_sierra).to_hex().to_string();
                let contract_class: ContractClass = serde_json::from_slice(&raw_sierra)?;
                let class_hash =
                    compute_starknet_sierra_class_hash(&contract_class).unwrap_or_default();

                let (casm_felts, casm_hints, compiled_class_hash) =
                    if let Some(casm_rel) = &entry.artifacts.casm {
                        let casm_path = target_dir.join(casm_rel);
                        if casm_path.is_file() {
                            let casm_bytes = fs::read(&casm_path)?;
                            let casm_class: CasmContractClass = serde_json::from_slice(&casm_bytes)?;
                            let hash = format!("0x{:x}", casm_class.compiled_class_hash());
                            (casm_class.bytecode.len(), casm_class.hints.len(), hash)
                        } else {
                            let casm_class =
                                compile_native_casm_contract(contract_class.clone(), max_casm)?;
                            let hash = format!("0x{:x}", casm_class.compiled_class_hash());
                            (casm_class.bytecode.len(), casm_class.hints.len(), hash)
                        }
                    } else if let Some((cached_casm_bytes, cached_hash)) =
                        try_load_global_casm_cache(&sierra_blake3, max_casm)
                    {
                        let casm_class: CasmContractClass =
                            serde_json::from_slice(&cached_casm_bytes)?;
                        (casm_class.bytecode.len(), casm_class.hints.len(), cached_hash)
                    } else {
                        let casm_class =
                            compile_native_casm_contract(contract_class.clone(), max_casm)?;
                        let hash = format!("0x{:x}", casm_class.compiled_class_hash());
                        if let Ok(casm_bytes) = serde_json::to_vec(&casm_class) {
                            persist_global_casm_cache_best_effort(
                                &sierra_blake3,
                                max_casm,
                                &casm_bytes,
                                &hash,
                            );
                        }
                        (casm_class.bytecode.len(), casm_class.hints.len(), hash)
                    };

                let (sierra_stmts, sierra_fns, sierra_lfs, sierra_types, prover_air) =
                    if let Ok(extracted) = contract_class.extract_sierra_program(true) {
                        let air = if args.prover_air {
                            let mut rc = 0usize;
                            let mut ped = 0usize;
                            let mut pos = 0usize;
                            let mut bit = 0usize;
                            let mut ec = 0usize;
                            let mut kec = 0usize;
                            let mut lf_map: HashMap<u64, String> = HashMap::new();
                            for ld in &extracted.program.libfunc_declarations {
                                lf_map.insert(ld.id.id, ld.long_id.generic_id.0.to_string());
                            }
                            for stmt in &extracted.program.statements {
                                if let cairo_lang_sierra::program::GenStatement::Invocation(inv) = stmt {
                                    if let Some(lf) = lf_map.get(&inv.libfunc_id.id) {
                                        if lf.contains("u8_")
                                            || lf.contains("u16_")
                                            || lf.contains("u32_")
                                            || lf.contains("u64_")
                                            || lf.contains("u128_")
                                            || lf.contains("upcast")
                                            || lf.contains("downcast")
                                            || lf.contains("storage_")
                                        {
                                            rc += 1;
                                        }
                                        if lf.contains("pedersen") {
                                            ped += 1;
                                        }
                                        if lf.contains("poseidon") || lf.contains("hades_permutation") {
                                            pos += 1;
                                        }
                                        if lf.contains("bitwise") {
                                            bit += 1;
                                        }
                                        if lf.contains("ec_") {
                                            ec += 1;
                                        }
                                        if lf.contains("keccak") {
                                            kec += 1;
                                        }
                                    }
                                }
                            }
                            let trace_cells = (extracted.program.statements.len() as u64) * 16
                                + (rc as u64) * 32
                                + (pos as u64) * 128
                                + (ped as u64) * 512
                                + (bit as u64) * 64
                                + (ec as u64) * 1024
                                + (kec as u64) * 2048;
                            let bottleneck = [
                                ("range_check", rc * 32),
                                ("poseidon", pos * 128),
                                ("pedersen", ped * 512),
                                ("bitwise", bit * 64),
                                ("ec_op", ec * 1024),
                                ("keccak", kec * 2048),
                            ]
                            .into_iter()
                            .max_by_key(|(_, w)| *w)
                            .map(|(n, _)| n.to_string())
                            .unwrap_or_else(|| "cpu_steps".to_string());
                            Some(ProverAirMultiplicitySummary {
                                range_check_builtin: rc,
                                pedersen_builtin: ped,
                                poseidon_builtin: pos,
                                bitwise_builtin: bit,
                                ec_op_builtin: ec,
                                keccak_builtin: kec,
                                stwo_trace_cells_estimate: trace_cells,
                                bottleneck_builtin: bottleneck,
                            })
                        } else {
                            None
                        };
                        (
                            extracted.program.statements.len(),
                            extracted.program.funcs.len(),
                            extracted.program.libfunc_declarations.len(),
                            extracted.program.type_declarations.len(),
                            air,
                        )
                    } else {
                        (contract_class.sierra_program.len(), 0, 0, 0, None)
                    };

                let mut entrypoints = analyze_contract_sierra_entrypoints(&contract_class);
                if entrypoints.len() > args.top {
                    entrypoints.truncate(args.top);
                }

                let utilization = (casm_felts as f64 / max_casm as f64) * 100.0;
                contract_summaries.push(ContractProfileSummary {
                    package_name: entry.package_name,
                    contract_name: entry.contract_name,
                    module_path: entry.module_path,
                    class_hash,
                    compiled_class_hash,
                    sierra_statements: sierra_stmts,
                    sierra_functions: sierra_fns,
                    sierra_libfuncs: sierra_lfs,
                    sierra_types,
                    casm_bytecode_felts: casm_felts,
                    casm_hints,
                    max_casm_bytecode_felts: max_casm,
                    casm_limit_utilization_pct: (utilization * 100.0).round() / 100.0,
                    prover_air,
                    entrypoints,
                });
            }
        }

        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        let report = ProfileReport {
            schema_version: UC_AGENT_JSON_SCHEMA_VERSION,
            generated_at_epoch_ms: epoch_ms_u64()?,
            manifest_path: manifest_path.display().to_string(),
            profile,
            elapsed_ms,
            contracts: contract_summaries,
        };

        if let Some(report_path) = &args.report_path {
            write_json_report(report_path, &report)?;
        }

        if args.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            for c in &report.contracts {
                println!(
                    "┌─ Contract: {} ({})",
                    c.contract_name, c.module_path
                );
                println!(
                    "│  Sierra Class Hash   : {}",
                    c.class_hash
                );
                println!(
                    "│  CASM Compiled Hash  : {}",
                    c.compiled_class_hash
                );
                println!(
                    "│  Sierra Program      : {} stmts | {} funcs | {} libfuncs | {} types",
                    c.sierra_statements, c.sierra_functions, c.sierra_libfuncs, c.sierra_types
                );
                println!(
                    "│  CASM Bytecode Size  : {} / {} felts ({:.2}% of Starknet max) | {} hints",
                    c.casm_bytecode_felts,
                    c.max_casm_bytecode_felts,
                    c.casm_limit_utilization_pct,
                    c.casm_hints
                );
                if let Some(air) = &c.prover_air {
                    println!(
                        "│  Stwo Prover AIR     : ~{} trace cells | bottleneck: {} (rc={}, pos={}, ped={}, bit={}, ec={}, kec={})",
                        air.stwo_trace_cells_estimate,
                        air.bottleneck_builtin,
                        air.range_check_builtin,
                        air.poseidon_builtin,
                        air.pedersen_builtin,
                        air.bitwise_builtin,
                        air.ec_op_builtin,
                        air.keccak_builtin
                    );
                }
                if !c.entrypoints.is_empty() {
                    println!(
                        "│  ┌──────────────────────┬────────────┬───────────┬───────┬────────┬───────┬────────────┐"
                    );
                    println!(
                        "│  │ Entrypoint           │ Kind       │ Stmts     │ SRead │ SWrite │ Calls │ Gas Weight │"
                    );
                    println!(
                        "│  ├──────────────────────┼────────────┼───────────┼───────┼────────┼───────┼────────────┤"
                    );
                    for ep in &c.entrypoints {
                        let short_name = if ep.name.len() > 20 {
                            format!("{}…", &ep.name[..19])
                        } else {
                            ep.name.clone()
                        };
                        println!(
                            "│  │ {:<20} │ {:<10} │ {:>9} │ {:>5} │ {:>6} │ {:>5} │ {:>10} │",
                            short_name,
                            ep.kind,
                            ep.reachable_sierra_statements,
                            ep.storage_reads,
                            ep.storage_writes,
                            ep.contract_calls,
                            ep.estimated_l2_gas_weight
                        );
                    }
                    println!(
                        "│  └──────────────────────┴────────────┴───────────┴───────┴────────┴───────┴────────────┘"
                    );
                }
                println!("└──────────────────────────────────────────────────────────────────────────────────────────");
            }
        }
        Ok(())
    }
}

// ============================================================================
// 10. `uc diff` Implementation (Semantic Contract & Artifact Diff Engine)
// ============================================================================

#[cfg(feature = "native-compile")]
struct LoadedContractSnapshot {
    contract_name: String,
    class_hash: String,
    sierra_statements: usize,
    casm_felts: Option<usize>,
    selectors: BTreeMap<String, String>,
    abi_functions: BTreeMap<String, String>,
}

#[cfg(feature = "native-compile")]
fn load_contract_snapshots_from_path(path: &Path) -> Result<BTreeMap<String, LoadedContractSnapshot>> {
    let mut map = BTreeMap::new();
    let mut files = Vec::new();
    if path.is_file() {
        files.push(path.to_path_buf());
    } else if path.is_dir() {
        for entry in WalkDir::new(path).max_depth(3).into_iter().flatten() {
            if entry.file_type().is_file()
                && entry
                    .path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(".contract_class.json"))
            {
                files.push(entry.path().to_path_buf());
            }
        }
    } else {
        bail!("diff path does not exist: {}", path.display());
    }

    for file_path in files {
        let fname = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("contract");
        let contract_name = fname
            .strip_suffix(".contract_class.json")
            .unwrap_or(fname)
            .to_string();
        let bytes = fs::read(&file_path)?;
        let cc: ContractClass = if let Ok(cc) = serde_json::from_slice(&bytes) {
            cc
        } else {
            let mut val: serde_json::Value = serde_json::from_slice(&bytes)?;
            if let Some(abi_str) = val.get("abi").and_then(|a| a.as_str()) {
                val["abi"] = serde_json::from_str(abi_str)
                    .unwrap_or(serde_json::Value::Array(Vec::new()));
            }
            serde_json::from_value(val)?
        };
        let class_hash = compute_starknet_sierra_class_hash(&cc).unwrap_or_default();
        let sierra_statements = cc
            .extract_sierra_program(true)
            .map(|p| p.program.statements.len())
            .unwrap_or(cc.sierra_program.len());

        let casm_companion = file_path.with_file_name(format!(
            "{contract_name}.compiled_contract_class.json"
        ));
        let casm_felts = if casm_companion.is_file() {
            fs::read(&casm_companion)
                .ok()
                .and_then(|b| serde_json::from_slice::<CasmContractClass>(&b).ok())
                .map(|c| c.bytecode.len())
        } else {
            None
        };

        let mut selectors = BTreeMap::new();
        for (kind, eps) in [
            ("external", &cc.entry_points_by_type.external),
            ("constructor", &cc.entry_points_by_type.constructor),
            ("l1_handler", &cc.entry_points_by_type.l1_handler),
        ] {
            for ep in eps {
                selectors.insert(format!("0x{:x}", ep.selector), kind.to_string());
            }
        }

        let mut abi_functions = BTreeMap::new();
        if let Some(abi) = &cc.abi {
            if let Ok(abi_json) = serde_json::to_value(abi) {
                if let Some(arr) = abi_json.as_array() {
                    for item in arr {
                        let ty = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if ty == "function" {
                            if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                                abi_functions.insert(
                                    name.to_string(),
                                    serde_json::to_string(item).unwrap_or_default(),
                                );
                            }
                        } else if ty == "interface" {
                            if let Some(items) = item.get("items").and_then(|v| v.as_array()) {
                                for sub in items {
                                    if let Some(name) = sub.get("name").and_then(|v| v.as_str()) {
                                        abi_functions.insert(
                                            name.to_string(),
                                            serde_json::to_string(sub).unwrap_or_default(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        map.insert(
            contract_name.clone(),
            LoadedContractSnapshot {
                contract_name,
                class_hash,
                sierra_statements,
                casm_felts,
                selectors,
                abi_functions,
            },
        );
    }
    Ok(map)
}

pub(crate) fn run_diff(args: DiffArgs) -> Result<()> {
    #[cfg(not(feature = "native-compile"))]
    {
        let _ = args;
        bail!("`uc diff` requires a binary built with the `native-compile` feature");
    }
    #[cfg(feature = "native-compile")]
    {
        let candidate_path = match &args.candidate {
            Some(p) => p.clone(),
            None => {
                let manifest_path = resolve_manifest_path(&args.manifest_path)?;
                let workspace_root = metadata_cache_workspace_root(&manifest_path)?;
                workspace_root.join("target/dev")
            }
        };

        let base_map = load_contract_snapshots_from_path(&args.baseline)?;
        let cand_map = load_contract_snapshots_from_path(&candidate_path)?;
        let all_keys: BTreeSet<String> =
            base_map.keys().chain(cand_map.keys()).cloned().collect();

        let mut entries = Vec::new();
        for key in all_keys {
            match (base_map.get(&key), cand_map.get(&key)) {
                (Some(b), Some(c)) => {
                    let stmts_delta =
                        (c.sierra_statements as isize) - (b.sierra_statements as isize);
                    let casm_delta = match (b.casm_felts, c.casm_felts) {
                        (Some(bf), Some(cf)) => (cf as isize) - (bf as isize),
                        _ => 0,
                    };
                    let added_selectors: Vec<String> = c
                        .selectors
                        .keys()
                        .filter(|s| !b.selectors.contains_key(*s))
                        .cloned()
                        .collect();
                    let removed_selectors: Vec<String> = b
                        .selectors
                        .keys()
                        .filter(|s| !c.selectors.contains_key(*s))
                        .cloned()
                        .collect();
                    let mut breaking = Vec::new();
                    for rem in &removed_selectors {
                        breaking.push(format!("Removed entrypoint selector {rem}"));
                    }
                    for (fn_name, base_sig) in &b.abi_functions {
                        match c.abi_functions.get(fn_name) {
                            None => breaking.push(format!("Removed ABI function `{fn_name}`")),
                            Some(cand_sig) if cand_sig != base_sig => {
                                breaking.push(format!("Modified ABI signature of `{fn_name}`"))
                            }
                            _ => {}
                        }
                    }
                    let status = if b.class_hash == c.class_hash {
                        "unchanged"
                    } else {
                        "modified"
                    };
                    entries.push(ContractDiffEntry {
                        contract_name: key,
                        status: status.to_string(),
                        baseline_class_hash: Some(b.class_hash.clone()),
                        candidate_class_hash: Some(c.class_hash.clone()),
                        baseline_sierra_statements: Some(b.sierra_statements),
                        candidate_sierra_statements: Some(c.sierra_statements),
                        sierra_statements_delta: stmts_delta,
                        baseline_casm_felts: b.casm_felts,
                        candidate_casm_felts: c.casm_felts,
                        casm_felts_delta: casm_delta,
                        added_selectors,
                        removed_selectors,
                        breaking_changes: breaking,
                    });
                }
                (Some(b), None) => {
                    entries.push(ContractDiffEntry {
                        contract_name: b.contract_name.clone(),
                        status: "removed".to_string(),
                        baseline_class_hash: Some(b.class_hash.clone()),
                        candidate_class_hash: None,
                        baseline_sierra_statements: Some(b.sierra_statements),
                        candidate_sierra_statements: None,
                        sierra_statements_delta: -(b.sierra_statements as isize),
                        baseline_casm_felts: b.casm_felts,
                        candidate_casm_felts: None,
                        casm_felts_delta: -(b.casm_felts.unwrap_or(0) as isize),
                        added_selectors: Vec::new(),
                        removed_selectors: b.selectors.keys().cloned().collect(),
                        breaking_changes: vec![format!(
                            "Contract `{}` was removed",
                            b.contract_name
                        )],
                    });
                }
                (None, Some(c)) => {
                    entries.push(ContractDiffEntry {
                        contract_name: c.contract_name.clone(),
                        status: "added".to_string(),
                        baseline_class_hash: None,
                        candidate_class_hash: Some(c.class_hash.clone()),
                        baseline_sierra_statements: None,
                        candidate_sierra_statements: Some(c.sierra_statements),
                        sierra_statements_delta: c.sierra_statements as isize,
                        baseline_casm_felts: None,
                        candidate_casm_felts: c.casm_felts,
                        casm_felts_delta: c.casm_felts.unwrap_or(0) as isize,
                        added_selectors: c.selectors.keys().cloned().collect(),
                        removed_selectors: Vec::new(),
                        breaking_changes: Vec::new(),
                    });
                }
                (None, None) => {}
            }
        }

        let has_changes = entries.iter().any(|e| e.status != "unchanged");
        let has_breaking_changes = entries.iter().any(|e| !e.breaking_changes.is_empty());
        let report = DiffReport {
            schema_version: UC_AGENT_JSON_SCHEMA_VERSION,
            generated_at_epoch_ms: epoch_ms_u64()?,
            baseline_path: args.baseline.display().to_string(),
            candidate_path: candidate_path.display().to_string(),
            has_changes,
            has_breaking_changes,
            contracts: entries,
        };

        if let Some(report_path) = &args.report_path {
            write_json_report(report_path, &report)?;
        }

        if args.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            for e in &report.contracts {
                println!(
                    "Contract `{}` [{}]: Sierra {:+} stmts, CASM {:+} felts",
                    e.contract_name,
                    e.status.to_ascii_uppercase(),
                    e.sierra_statements_delta,
                    e.casm_felts_delta
                );
                for brk in &e.breaking_changes {
                    println!("  [BREAKING] {brk}");
                }
            }
        }

        if args.fail_on_breaking && has_breaking_changes {
            std::process::exit(1);
        }
        Ok(())
    }
}

// ============================================================================
// 11. `uc abi` / `uc bindgen` Implementation
// ============================================================================

pub(crate) fn run_abi(args: AbiArgs) -> Result<()> {
    #[cfg(not(feature = "native-compile"))]
    {
        let _ = args;
        bail!("`uc abi` requires a binary built with the `native-compile` feature");
    }
    #[cfg(feature = "native-compile")]
    {
        let manifest_path = resolve_manifest_path(&args.manifest_path)?;
        let workspace_root = metadata_cache_workspace_root(&manifest_path)?;
        let profile = resolve_nextgen_profile(args.profile.as_deref(), args.release);
        let target_dir = workspace_root.join("target").join(&profile);

        let mut artifacts_manifests = find_starknet_artifacts_manifests(&target_dir);
        if artifacts_manifests.is_empty() {
            let build_args = BuildArgs {
                common: BuildCommonArgs {
                    manifest_path: Some(manifest_path.clone()),
                    package: None,
                    workspace: false,
                    features: Vec::new(),
                    offline: false,
                    release: args.release,
                    profile: Some(profile.clone()),
                },
                engine: EngineArg::Uc,
                daemon_mode: DaemonModeArg::Auto,
                report_path: None,
                plan_only: false,
                json: false,
                record_failure: None,
            };
            let _ = run_build_entry(build_args);
            artifacts_manifests = find_starknet_artifacts_manifests(&target_dir);
        }

        let mut rendered_chunks = Vec::new();
        for am_path in artifacts_manifests {
            let bytes = fs::read(&am_path)?;
            let Ok(manifest) = serde_json::from_slice::<StarknetArtifactsManifest>(&bytes) else {
                continue;
            };
            for entry in manifest.contracts {
                if let Some(filter) = &args.target {
                    if !entry.contract_name.contains(filter) && !entry.module_path.contains(filter)
                    {
                        continue;
                    }
                }
                let sierra_path = target_dir.join(&entry.artifacts.sierra);
                if !sierra_path.is_file() {
                    continue;
                }
                let raw = fs::read(&sierra_path)?;
                let cc: ContractClass = serde_json::from_slice(&raw)?;
                let class_hash = compute_starknet_sierra_class_hash(&cc).unwrap_or_default();
                let abi_val = serde_json::to_value(&cc.abi)?;

                match args.format {
                    AbiOutputFormat::Json => {
                        rendered_chunks.push(serde_json::to_string_pretty(&abi_val)?);
                    }
                    AbiOutputFormat::Ts => {
                        let abi_pretty = serde_json::to_string_pretty(&abi_val)?;
                        let ts = format!(
                            "// Auto-generated by `uc abi --format ts` for {} ({})\nexport const {}_CLASS_HASH = \"{}\" as const;\nexport const {}_ABI = {} as const;\n",
                            entry.contract_name,
                            entry.module_path,
                            entry.contract_name.to_ascii_uppercase(),
                            class_hash,
                            entry.contract_name.to_ascii_uppercase(),
                            abi_pretty
                        );
                        rendered_chunks.push(ts);
                    }
                    AbiOutputFormat::Cairo => {
                        let mut trait_fns = Vec::new();
                        let mut push_cairo_fn = |fn_item: &serde_json::Value| {
                            if fn_item.get("type").and_then(|v| v.as_str()) == Some("function") {
                                let fn_name = fn_item
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("func");
                                let mutability = fn_item
                                    .get("state_mutability")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("external");
                                let self_param = if mutability == "view" {
                                    "self: @TContractState"
                                } else {
                                    "ref self: TContractState"
                                };
                                let mut params = vec![self_param.to_string()];
                                if let Some(inputs) =
                                    fn_item.get("inputs").and_then(|v| v.as_array())
                                {
                                    for inp in inputs {
                                        let in_name = inp
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("arg");
                                        let in_ty = inp
                                            .get("type")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("felt252");
                                        params.push(format!("{in_name}: {in_ty}"));
                                    }
                                }
                                let ret_str = fn_item
                                    .get("outputs")
                                    .and_then(|v| v.as_array())
                                    .and_then(|o| o.first())
                                    .and_then(|o| o.get("type"))
                                    .and_then(|v| v.as_str())
                                    .map(|ty| format!(" -> {ty}"))
                                    .unwrap_or_default();
                                trait_fns.push(format!(
                                    "    fn {}({}){};",
                                    fn_name,
                                    params.join(", "),
                                    ret_str
                                ));
                            }
                        };
                        if let Some(arr) = abi_val.as_array() {
                            for item in arr {
                                match item.get("type").and_then(|v| v.as_str()) {
                                    Some("interface") => {
                                        if let Some(items) =
                                            item.get("items").and_then(|v| v.as_array())
                                        {
                                            for fn_item in items {
                                                push_cairo_fn(fn_item);
                                            }
                                        }
                                    }
                                    Some("function") => push_cairo_fn(item),
                                    _ => {}
                                }
                            }
                        }
                        let cairo_stub = format!(
                            "// Auto-generated by `uc abi --format cairo` for {}\n#[starknet::interface]\npub trait I{}<TContractState> {{\n{}\n}}\n",
                            entry.module_path,
                            entry.contract_name,
                            trait_fns.join("\n")
                        );
                        rendered_chunks.push(cairo_stub);
                    }
                    AbiOutputFormat::Rust => {
                        let sel_names = abi_selector_names(&abi_val);
                        let mut consts = Vec::new();
                        consts.push(format!(
                            "    pub const CLASS_HASH: &str = \"{}\";",
                            class_hash
                        ));
                        for (idx, ep) in cc.entry_points_by_type.external.iter().enumerate() {
                            let sel_hex = format!("0x{:x}", ep.selector);
                            let const_name = sel_names
                                .get(&sel_hex)
                                .map(|n| format!("SELECTOR_{}", n.to_ascii_uppercase()))
                                .unwrap_or_else(|| format!("EXTERNAL_SELECTOR_{idx}"));
                            consts.push(format!(
                                "    pub const {const_name}: &str = \"{sel_hex}\";",
                            ));
                        }
                        let rs = format!(
                            "// Auto-generated by `uc abi --format rust` for {}\npub mod {} {{\n{}\n}}\n",
                            entry.module_path,
                            entry.contract_name.to_ascii_lowercase(),
                            consts.join("\n")
                        );
                        rendered_chunks.push(rs);
                    }
                }
            }
        }

        let output_text = rendered_chunks.join("\n");
        if let Some(out_path) = &args.output {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(out_path, &output_text)?;
        } else {
            print!("{output_text}");
            if !output_text.ends_with('\n') {
                println!();
            }
        }
        Ok(())
    }
}

// ============================================================================
// 12. `uc manifest-path`, `uc run`, `uc completions`
// ============================================================================

pub(crate) fn run_manifest_path(args: ManifestPathArgs) -> Result<()> {
    let manifest_path = resolve_manifest_path(&args.manifest_path)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "manifest_path": manifest_path.display().to_string()
            }))?
        );
    } else {
        println!("{}", manifest_path.display());
    }
    Ok(())
}

pub(crate) fn run_script(args: RunScriptArgs) -> Result<()> {
    let base_manifest_path = resolve_manifest_path(&args.manifest_path)?;
    let workspace_root = metadata_cache_workspace_root(&base_manifest_path)?;
    #[cfg(feature = "native-compile")]
    let manifest_path = resolve_native_effective_manifest_path(
        &base_manifest_path,
        &workspace_root,
        args.package.as_deref(),
    )
    .unwrap_or(base_manifest_path);
    #[cfg(not(feature = "native-compile"))]
    let manifest_path = base_manifest_path;

    let text = read_text_file_with_limit(&manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
    let doc: TomlValue = parse_manifest_toml(&text, &manifest_path, "failed to parse Scarb.toml")?;
    let scripts_table = doc
        .get("scripts")
        .and_then(TomlValue::as_table)
        .cloned()
        .unwrap_or_default();

    if args.list || args.script.is_none() {
        if args.json {
            println!("{}", serde_json::to_string_pretty(&scripts_table)?);
        } else if scripts_table.is_empty() {
            println!("No scripts defined in [scripts] of {}", manifest_path.display());
        } else {
            for (k, v) in &scripts_table {
                println!("{:<20} : {}", k, v.as_str().unwrap_or(""));
            }
        }
        return Ok(());
    }

    let script_name = args.script.as_deref().unwrap();
    let Some(cmd_raw) = scripts_table.get(script_name).and_then(TomlValue::as_str) else {
        bail!(
            "script `{script_name}` is not defined in [scripts] of {}",
            manifest_path.display()
        );
    };

    let full_cmd = if args.args.is_empty() {
        cmd_raw.to_string()
    } else {
        format!("{} {}", cmd_raw, args.args.join(" "))
    };
    let manifest_dir = manifest_path.parent().unwrap_or(&workspace_root);
    let status = Command::new("sh")
        .arg("-c")
        .arg(&full_cmd)
        .current_dir(manifest_dir)
        .status()
        .with_context(|| format!("failed to execute script `{script_name}`"))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

pub(crate) fn run_completions(args: CompletionsArgs) -> Result<()> {
    let subcommands = [
        "build",
        "check",
        "test",
        "fmt",
        "lint",
        "profile",
        "gas-report",
        "diff",
        "abi",
        "bindgen",
        "inspect",
        "verify",
        "expand",
        "clean",
        "new",
        "init",
        "add",
        "remove",
        "rm",
        "tree",
        "run",
        "manifest-path",
        "completions",
        "class-hash",
        "selector",
        "call",
        "declare",
        "deploy",
        "invoke",
        "tx-status",
        "account",
        "verify-onchain",
        "metadata",
        "resolve",
        "fetch",
        "cache",
        "daemon",
        "agent",
        "project",
        "toolchain",
        "support",
        "mcp",
        "session-key",
        "compare-build",
        "migrate",
        "replay",
        "prove",
        "decompile",
        "audit",
        "watch",
        "doctor",
        "devnet",
        "ledger",
        "execute",
        "doc",
        "package",
        "publish",
        "list",
        "update",
        "commands",
        "optimize-inlining",
        "clean-cache",
        "check-requirements",
        "declare-from",
        "multicall",
        "get",
        "balance",
        "show-config",
        "script",
        "utils",
        "trace",
        "lsp",
    ];
    let list = subcommands.join(" ");
    match args.shell {
        ShellKindArg::Bash => {
            println!(
                r#"_uc_completions() {{
    local cur prev opts
    COMPREPLY=()
    cur="${{COMP_WORDS[COMP_CWORD]}}"
    opts="{list} --help --version"
    if [[ ${{COMP_CWORD}} -eq 1 ]]; then
        COMPREPLY=( $(compgen -W "${{opts}}" -- "${{cur}}") )
        return 0
    fi
}}
complete -F _uc_completions uc"#
            );
        }
        ShellKindArg::Zsh => {
            println!(
                r#"#compdef uc
_uc() {{
    local -a commands
    commands=({list})
    _describe 'command' commands
}}
compdef _uc uc"#
            );
        }
        ShellKindArg::Fish => {
            for cmd in subcommands {
                println!("complete -c uc -f -n '__fish_use_subcommand' -a '{cmd}'");
            }
        }
        ShellKindArg::Powershell => {
            println!(
                "Register-ArgumentCompleter -Native -CommandName uc -ScriptBlock {{ param($wordToComplete) @({}) | Where-Object {{ $_ -like \"$wordToComplete*\" }} }}",
                subcommands
                    .iter()
                    .map(|c| format!("'{c}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        ShellKindArg::Elvish => {
            for cmd in subcommands {
                println!("edit:completion:arg-completer[uc] = {{|@args| put {cmd} }}");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestTempDir {
        path: PathBuf,
    }

    impl TestTempDir {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "uc_nextgen_test_{}_{}",
                std::process::id(),
                n
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create temp test dir");
            Self { path }
        }
        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestTempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[cfg(feature = "native-compile")]
    #[test]
    fn fmt_check_and_in_place_formatting_work_with_manifest_tool_fmt_config() {
        let _env = crate::test_env_shared();
        let dir = TestTempDir::new();
        let manifest_path = dir.path().join("Scarb.toml");
        fs::write(
            &manifest_path,
            "[package]\nname = \"fmt_demo\"\nversion = \"0.1.0\"\nedition = \"2024_07\"\n\n[tool.fmt]\nsort-module-level-items = true\n",
        )
        .expect("write manifest");
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&src_dir).expect("create src");
        let lib_path = src_dir.join("lib.cairo");
        fs::write(&lib_path, "use core::poseidon;\nuse core::array;\npub fn   add(a:felt252,b:felt252)->felt252{a+b}\n")
            .expect("write lib.cairo");

        run_fmt(FmtArgs {
            manifest_path: Some(manifest_path.clone()),
            package: None,
            workspace: false,
            check: false,
            no_color: true,
            emit: FmtEmitMode::Files,
            sort_module_level_items: None,
            paths: Vec::new(),
            json: false,
            report_path: None,
        })
        .expect("run_fmt in-place should succeed");

        let formatted = fs::read_to_string(&lib_path).expect("read formatted");
        assert!(
            formatted.contains("pub fn add(a: felt252, b: felt252) -> felt252"),
            "expected formatted function signature, got:\n{formatted}"
        );

        run_fmt(FmtArgs {
            manifest_path: Some(manifest_path),
            package: None,
            workspace: false,
            check: true,
            no_color: true,
            emit: FmtEmitMode::Files,
            sort_module_level_items: None,
            paths: Vec::new(),
            json: false,
            report_path: None,
        })
        .expect("run_fmt --check after formatting should succeed");
    }

    #[test]
    fn new_init_add_remove_tree_and_clean_roundtrip_work() {
        let _env = crate::test_env_shared();
        let dir = TestTempDir::new();
        let proj_dir = dir.path().join("my_nextgen_contract");

        run_new(NewArgs {
            path: proj_dir.clone(),
            name: None,
            edition: "2024_07".to_string(),
            starknet: true,
            legacy_scarb: false,
            no_vcs: true,
            vcs: VcsKindArg::None,
            json: false,
            report_path: None,
        })
        .expect("run_new --starknet should succeed");

        let manifest_path = proj_dir.join("uc.toml");
        assert!(manifest_path.is_file());
        assert!(proj_dir.join("src/lib.cairo").is_file());

        run_add(AddArgs {
            manifest_path: Some(manifest_path.clone()),
            package: None,
            packages: vec!["openzeppelin_token@0.20.0".to_string()],
            path: None,
            git: None,
            branch: None,
            tag: None,
            rev: None,
            dev: false,
            workspace: false,
            dry_run: false,
            json: false,
            report_path: None,
        })
        .expect("run_add should succeed");

        let after_add = fs::read_to_string(&manifest_path).expect("read Scarb.toml");
        assert!(after_add.contains("openzeppelin_token"));

        let tree_report = dir.path().join("tree.json");
        run_tree(TreeArgs {
            manifest_path: Some(manifest_path.clone()),
            package: None,
            no_dev: false,
            depth: None,
            no_dedupe: false,
            charset: TreeCharsetArg::Ascii,
            prune: Vec::new(),
            json: false,
            report_path: Some(tree_report.clone()),
        })
        .expect("run_tree should succeed");
        assert!(tree_report.is_file());

        run_remove(RemoveArgs {
            manifest_path: Some(manifest_path.clone()),
            package: None,
            packages: vec!["openzeppelin_token".to_string()],
            dev: false,
            dry_run: false,
            json: false,
            report_path: None,
        })
        .expect("run_remove should succeed");

        let after_rm = fs::read_to_string(&manifest_path).expect("read Scarb.toml");
        assert!(!after_rm.contains("openzeppelin_token"));

        let target_dir = proj_dir.join("target/dev");
        fs::create_dir_all(&target_dir).expect("create target/dev");
        fs::write(target_dir.join("dummy.json"), "{}").expect("write dummy");

        run_clean(CleanArgs {
            manifest_path: Some(manifest_path),
            target_dir: None,
            all: false,
            uc_cache: false,
            dry_run: false,
            json: false,
            report_path: None,
        })
        .expect("run_clean should succeed");
        assert!(!proj_dir.join("target").exists());
    }

    #[cfg(feature = "native-compile")]
    #[test]
    fn static_starknet_linter_detects_storage_in_loop_and_unindexed_events() {
        let source = r#"
#[starknet::contract]
pub mod BadContract {
    #[storage]
    struct Storage {
        balance: felt252,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        Transfer: Transfer,
    }

    #[derive(Drop, starknet::Event)]
    struct Transfer {
        from: felt252,
        to: felt252,
        amount: felt252,
    }

    #[external(v0)]
    fn bad_loop(ref self: ContractState, recipient: ContractAddress) {
        let mut i: u32 = 0;
        while i < 10 {
            let v = self.balance.read();
            self.balance.write(v + 1);
            i += 1;
        };
    }
}
"#;
        let mut findings = Vec::new();
        scan_cairo_file_for_starknet_lints("src/lib.cairo", source, &mut findings);
        let codes: Vec<&str> = findings.iter().map(|f| f.rule_id.as_str()).collect();
        assert!(
            codes.contains(&"SN_STORAGE_IN_LOOP"),
            "expected SN_STORAGE_IN_LOOP in {codes:?}"
        );
        assert!(
            codes.contains(&"SN_UNCHECKED_ZERO_ADDRESS"),
            "expected SN_UNCHECKED_ZERO_ADDRESS in {codes:?}"
        );
        assert!(
            codes.contains(&"SN_UNPROTECTED_STATE_MUTATION"),
            "expected SN_UNPROTECTED_STATE_MUTATION in {codes:?}"
        );
        assert!(
            codes.contains(&"SN_EVENT_UNINDEXED"),
            "expected SN_EVENT_UNINDEXED in {codes:?}"
        );
    }

    #[test]
    fn globbed_workspace_members_and_wave6_codegen_options_parse_correctly() {
        let dir = TestTempDir::new();
        let ws_manifest = dir.path().join("Scarb.toml");
        fs::write(
            &ws_manifest,
            "[workspace]\nmembers = [\"packages/*\"]\n",
        )
        .expect("write ws Scarb.toml");
        let pkg_a = dir.path().join("packages/alpha");
        let pkg_b = dir.path().join("packages/beta");
        fs::create_dir_all(&pkg_a).expect("mkdir alpha");
        fs::create_dir_all(&pkg_b).expect("mkdir beta");
        fs::write(
            pkg_a.join("Scarb.toml"),
            "[package]\nname = \"alpha\"\nversion = \"0.1.0\"\n[cairo]\ninlining-strategy = \"avoid\"\nsierra-replace-ids = true\n[[target.starknet-contract]]\ncasm = true\ncasm-add-pythonic-hints = true\n",
        )
        .expect("write alpha");
        fs::write(
            pkg_b.join("Scarb.toml"),
            "[package]\nname = \"beta\"\nversion = \"0.1.0\"\n",
        )
        .expect("write beta");

        let members = resolve_all_workspace_member_manifests(&ws_manifest, dir.path())
            .expect("resolve_all_workspace_member_manifests");
        assert_eq!(members.len(), 2, "expected 2 glob-expanded members");

        #[cfg(feature = "native-compile")]
        {
            let resolved_alpha =
                resolve_native_effective_manifest_path(&ws_manifest, dir.path(), Some("alpha"))
                    .expect("resolve alpha from globbed workspace");
            assert_eq!(
                resolved_alpha,
                pkg_a.join("Scarb.toml").canonicalize().unwrap()
            );
            let opts = resolve_manifest_native_cairo_codegen_options(&resolved_alpha);
            assert!(opts.casm_add_pythonic_hints);
            assert_eq!(opts.inlining_strategy, InliningStrategy::Avoid);
            assert_eq!(opts.sierra_replace_ids, Some(true));
        }
    }
}

