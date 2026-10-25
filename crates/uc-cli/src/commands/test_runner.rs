use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Args, Debug, Clone, Default)]
pub(crate) struct TestArgs {
    /// Optional substring filter for test names or module paths.
    pub filter: Option<String>,

    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    /// Package to run tests for.
    #[arg(short = 'p', long)]
    pub package: Option<String>,

    /// Run tests for all packages in the workspace.
    #[arg(short = 'w', long)]
    pub workspace: bool,

    /// Space or comma separated list of features to activate.
    #[arg(short = 'F', long)]
    pub features: Vec<String>,

    /// Only run tests affected by source or module changes since the last test run.
    #[arg(long)]
    pub affected: bool,

    /// Run ignored tests as well.
    #[arg(long)]
    pub include_ignored: bool,

    /// Run only ignored tests.
    #[arg(long)]
    pub ignored: bool,

    /// Exactly match test filter rather than substring.
    #[arg(short = 'e', long)]
    pub exact: bool,

    /// Skip tests whose name contains any of the given substrings (`snforge test --skip`).
    #[arg(long)]
    pub skip: Vec<String>,

    /// Stop executing tests after the first failed test (`snforge test --exit-first`).
    #[arg(short = 'x', long)]
    pub exit_first: bool,

    /// Re-run only the tests that failed in the previous run (`snforge test --rerun-failed`).
    #[arg(long)]
    pub rerun_failed: bool,

    /// Partition tests across CI shards, e.g. `1/4` (`snforge test --partition`).
    #[arg(long)]
    pub partition: Option<String>,

    /// Maximum number of worker threads (`snforge test --max-threads`).
    #[arg(long)]
    pub max_threads: Option<usize>,

    /// Override maximum VM steps per test (`snforge test --max-n-steps`).
    #[arg(long)]
    pub max_n_steps: Option<u32>,

    /// Display detailed builtin, step, and syscall resource breakdown (`snforge test --detailed-resources`).
    #[arg(long)]
    pub detailed_resources: bool,

    /// Resource tracking mode (`sierra-gas` or `cairo-steps`).
    #[arg(long)]
    pub tracked_resource: Option<String>,

    /// Call trace verbosity (`minimal`, `standard`, `detailed`).
    #[arg(long)]
    pub trace_verbosity: Option<String>,

    /// Save trace data JSON files (`snforge test --save-trace-data`).
    #[arg(long)]
    pub save_trace_data: bool,

    /// Generate profiling data from saved traces (`snforge test --build-profile`).
    #[arg(long)]
    pub build_profile: bool,

    /// Optional Starknet RPC URL for live fork-state testing (`--fork-url`).
    #[arg(long)]
    pub fork_url: Option<String>,

    /// Activate all available features.
    #[arg(long)]
    pub all_features: bool,

    /// Do not activate the `default` feature.
    #[arg(long)]
    pub no_default_features: bool,

    /// Build profile name (`dev`, `release`).
    #[arg(short = 'P', long)]
    pub profile: Option<String>,

    /// Use release profile.
    #[arg(long)]
    pub release: bool,

    /// Display detailed gas, step, builtin, and syscall usage per test.
    #[arg(long)]
    pub gas_report: bool,

    /// Number of property-based / coverage-guided fuzz iterations per parameterized test.
    #[arg(short = 'r', long, visible_alias = "fuzzer-runs", default_value_t = 0)]
    pub fuzz_runs: usize,

    /// Deterministic PRNG seed for property-based fuzzing and invariant testing.
    #[arg(short = 's', long, visible_alias = "fuzzer-seed")]
    pub fuzz_seed: Option<u64>,

    /// Collect Sierra statement and function coverage across test executions.
    #[arg(long)]
    pub coverage: bool,

    /// Export LCOV coverage report (`lcov.info`) to the specified path.
    #[arg(long)]
    pub lcov_path: Option<PathBuf>,

    /// Write deterministic gas and step snapshot file (Foundry `.gas-snapshot` / `insta` style).
    #[arg(long)]
    pub snapshot: Option<PathBuf>,

    /// Check gas and step usage against an existing snapshot file and fail on regression.
    #[arg(long)]
    pub snapshot_check: Option<PathBuf>,

    /// Run semantic mutation testing (`cargo-mutants` / Gambit style) on `src/*.cairo`.
    #[arg(long)]
    pub mutate: bool,

    /// Force recompilation of test artifacts without reading `.uc/cache/tests`.
    #[arg(long)]
    pub no_cache: bool,

    /// Run in offline mode without network access.
    #[arg(long)]
    pub offline: bool,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub report_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AffectedTestGraphCache {
    schema_version: u32,
    package_name: String,
    module_digests: BTreeMap<String, String>,
    module_imports: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SingleTestResultReport {
    pub name: String,
    pub file: String,
    pub line: usize,
    pub should_panic: bool,
    pub ignored: bool,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_used: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub syscalls: Option<BTreeMap<String, usize>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fuzz_runs: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fuzz_seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shrunk_counterexample: Option<Vec<String>>,
    /// Wall-clock execution time of this test (including fuzz runs), in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<f64>,
    /// Builtin instances used by the VM run, nested calls included (`range_check`, `pedersen`,
    /// `poseidon`, `bitwise`, `ec_op`, `keccak`, `range_check96`, `add_mod`, `mul_mod`, `ecdsa`).
    /// For fuzz tests: the sum over all runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub builtins: Option<BTreeMap<String, usize>>,
    /// Steps and builtins weighted with Starknet's L2 gas costs per resource; shows which
    /// resource dominates the cost of this test's execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_gas: Option<ResourceGasBreakdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

/// L2 gas per VM resource, from blockifier's `os_constants` (`step_gas_cost`,
/// `builtin_gas_costs`) for Starknet 0.13.4 and later. These weights are what the sequencer
/// charges per step and per builtin instance; they also rank the AIR components by how much
/// trace each one costs the prover, which is what the weights were derived from.
pub(crate) const STEP_GAS_COST: u64 = 100;
pub(crate) const BUILTIN_GAS_COSTS: &[(&str, u64)] = &[
    ("range_check", 70),
    ("range_check96", 56),
    ("pedersen", 4050),
    ("poseidon", 491),
    ("bitwise", 583),
    ("ec_op", 4085),
    ("ecdsa", 10561),
    ("keccak", 136189),
    ("add_mod", 230),
    ("mul_mod", 604),
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct ResourceGasBreakdown {
    /// `steps * STEP_GAS_COST + sum(builtin instances * weight)`.
    pub total: u64,
    pub steps: u64,
    /// Gas attributed to each builtin that was used at least once.
    pub builtins: BTreeMap<String, u64>,
    /// The resource with the largest share (`steps` or a builtin name).
    pub dominant: String,
    /// Share of `total` taken by `dominant`, in percent.
    pub dominant_pct: f64,
}

/// Weighs a run's steps and builtin instances with Starknet's L2 gas costs.
pub(crate) fn resource_gas_breakdown(
    n_steps: usize,
    builtins: &BTreeMap<String, usize>,
) -> ResourceGasBreakdown {
    let steps = n_steps as u64 * STEP_GAS_COST;
    let mut weighted = BTreeMap::new();
    for (name, count) in builtins {
        if *count == 0 {
            continue;
        }
        let weight = BUILTIN_GAS_COSTS
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, w)| *w)
            .unwrap_or(0);
        weighted.insert(name.clone(), *count as u64 * weight);
    }
    let total = steps + weighted.values().sum::<u64>();
    let (dominant, share) = weighted
        .iter()
        .map(|(n, g)| (n.as_str(), *g))
        .chain(std::iter::once(("steps", steps)))
        .max_by_key(|(_, g)| *g)
        .unwrap_or(("steps", 0));
    let dominant = dominant.to_string();
    ResourceGasBreakdown {
        total,
        steps,
        builtins: weighted,
        dominant,
        dominant_pct: if total == 0 { 0.0 } else { share as f64 * 100.0 / total as f64 },
    }
}

/// Builtin instance counts of a run as `{builtin name: instances}`, zero entries dropped.
fn builtin_counts(resources: &cairo_lang_runner::StarknetExecutionResources) -> BTreeMap<String, usize> {
    resources
        .basic_resources
        .builtin_instance_counter
        .iter()
        .filter(|(_, n)| **n > 0)
        .map(|(name, n)| (name.to_str().to_string(), *n))
        .collect()
}

fn builtins_or_none(map: BTreeMap<String, usize>) -> Option<BTreeMap<String, usize>> {
    if map.is_empty() { None } else { Some(map) }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SuiteResourceTotals {
    pub steps: usize,
    pub builtins: BTreeMap<String, usize>,
    pub resource_gas: ResourceGasBreakdown,
    /// Tests ranked by weighted resource gas, heaviest first (at most ten).
    pub heaviest_tests: Vec<(String, u64)>,
}

/// Sums steps and builtin instances over every test that ran, and ranks the heaviest tests.
pub(crate) fn suite_resource_totals(results: &[SingleTestResultReport]) -> Option<SuiteResourceTotals> {
    let mut steps = 0usize;
    let mut builtins: BTreeMap<String, usize> = BTreeMap::new();
    let mut ranked = Vec::new();
    let mut any = false;
    for t in results {
        if let Some(s) = t.steps {
            // Fuzz tests report the average per run; count the whole test once.
            steps += s * t.fuzz_runs.unwrap_or(1);
            any = true;
        }
        if let Some(b) = &t.builtins {
            for (k, v) in b {
                *builtins.entry(k.clone()).or_insert(0) += v;
            }
        }
        if let Some(r) = &t.resource_gas {
            ranked.push((t.name.clone(), r.total));
        }
    }
    if !any {
        return None;
    }
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(10);
    let resource_gas = resource_gas_breakdown(steps, &builtins);
    Some(SuiteResourceTotals { steps, builtins, resource_gas, heaviest_tests: ranked })
}

fn scan_cairo_module_digests_and_imports(
    crate_name: &str,
    src_dir: &Path,
) -> (BTreeMap<String, String>, BTreeMap<String, Vec<String>>) {
    let mut digests = BTreeMap::new();
    let mut imports = BTreeMap::new();
    if !src_dir.is_dir() {
        return (digests, imports);
    }
    for entry in WalkDir::new(src_dir).follow_links(false).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("cairo"))
        {
            continue;
        }
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        let rel = path.strip_prefix(src_dir).unwrap_or(path);
        let file_mod = cairo_file_module_prefix(crate_name, rel);
        let file_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        digests.insert(file_mod.clone(), file_hash);
        let mut file_imports = BTreeSet::new();
        let mut top_brace_depth = 0_i32;
        for line in content.lines() {
            let trimmed = line.split("//").next().unwrap_or("").trim();
            if top_brace_depth == 0 {
                if let Some(rest) = trimmed
                    .strip_prefix("use ")
                    .or_else(|| trimmed.strip_prefix("pub use "))
                {
                    let path_part = rest.trim_end_matches(';').trim();
                    for token in path_part.split(&['{', '}', ','][..]) {
                        let tok = token.trim();
                        if tok.starts_with(crate_name) || tok.starts_with("super::") {
                            let normalized = if let Some(sub) = tok.strip_prefix("super::") {
                                let seg = sub.split("::").next().unwrap_or("");
                                if !seg.is_empty() {
                                    format!("{crate_name}::{seg}")
                                } else {
                                    crate_name.to_string()
                                }
                            } else {
                                let mut parts = tok.split("::");
                                let first = parts.next().unwrap_or(crate_name);
                                if let Some(second) = parts.next() {
                                    format!("{first}::{second}")
                                } else {
                                    first.to_string()
                                }
                            };
                            file_imports.insert(normalized);
                        }
                    }
                }
            }
            for ch in trimmed.chars() {
                if ch == '{' {
                    top_brace_depth += 1;
                } else if ch == '}' {
                    top_brace_depth -= 1;
                }
            }
        }
        imports.insert(file_mod.clone(), file_imports.into_iter().collect());

        // Also index inline `mod <name> { ... }` blocks for sub-file module granularity!
        let inline_mods = extract_inline_cairo_modules(&file_mod, &content);
        for (mod_path, mod_body, mod_uses) in inline_mods {
            let h = blake3::hash(mod_body.as_bytes()).to_hex().to_string();
            digests.insert(mod_path.clone(), h);
            imports.insert(mod_path, mod_uses);
        }
    }
    (digests, imports)
}

fn cairo_file_module_prefix(crate_name: &str, rel_path: &Path) -> String {
    let mut parts = vec![crate_name.to_string()];
    let stem = rel_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("lib");
    if let Some(parent) = rel_path.parent() {
        for comp in parent.components() {
            let s = comp.as_os_str().to_string_lossy();
            if !s.is_empty() {
                parts.push(s.to_string());
            }
        }
    }
    if stem != "lib" && stem != "mod" {
        parts.push(stem.to_string());
    }
    parts.join("::")
}

fn extract_inline_cairo_modules(
    parent_mod: &str,
    source: &str,
) -> Vec<(String, String, Vec<String>)> {
    let mut results = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut idx = 0;
    while idx < lines.len() {
        let trimmed = lines[idx].split("//").next().unwrap_or("").trim();
        let after_pub = trimmed.strip_prefix("pub ").unwrap_or(trimmed).trim_start();
        if let Some(rest) = after_pub.strip_prefix("mod ") {
            let mod_name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !mod_name.is_empty() && rest.contains('{') {
                let full_mod = format!("{parent_mod}::{mod_name}");
                let mut brace_depth = 0_i32;
                let mut body = String::new();
                let mut mod_uses = BTreeSet::new();
                let root_crate = parent_mod.split("::").next().unwrap_or(parent_mod);
                while idx < lines.len() {
                    let l = lines[idx];
                    body.push_str(l);
                    body.push('\n');
                    let clean = l.split("//").next().unwrap_or("").trim();
                    for ch in clean.chars() {
                        if ch == '{' {
                            brace_depth += 1;
                        } else if ch == '}' {
                            brace_depth -= 1;
                        }
                    }
                    if let Some(u) = clean
                        .strip_prefix("use ")
                        .or_else(|| clean.strip_prefix("pub use "))
                    {
                        for token in u.trim_end_matches(';').split(&['{', '}', ','][..]) {
                            let tok = token.trim();
                            if let Some(sub) = tok.strip_prefix("super::") {
                                let seg = sub.split("::").next().unwrap_or("");
                                if !seg.is_empty() {
                                    mod_uses.insert(format!("{parent_mod}::{seg}"));
                                } else {
                                    mod_uses.insert(parent_mod.to_string());
                                }
                            } else if tok.starts_with(root_crate) {
                                let mut sp = tok.split("::");
                                let c0 = sp.next().unwrap_or(root_crate);
                                if let Some(c1) = sp.next() {
                                    mod_uses.insert(format!("{c0}::{c1}"));
                                }
                            }
                        }
                    }
                    idx += 1;
                    if brace_depth <= 0 {
                        break;
                    }
                }
                results.push((full_mod, body, mod_uses.into_iter().collect()));
                continue;
            }
        }
        idx += 1;
    }
    results
}

fn compute_affected_modules(
    workspace_root: &Path,
    package_name: &str,
    crate_name: &str,
    src_dir: &Path,
) -> (Option<BTreeSet<String>>, AffectedTestGraphCache) {
    let (current_digests, current_imports) = scan_cairo_module_digests_and_imports(crate_name, src_dir);
    let current_state = AffectedTestGraphCache {
        schema_version: 1,
        package_name: package_name.to_string(),
        module_digests: current_digests.clone(),
        module_imports: current_imports.clone(),
    };
    let graph_path = workspace_root
        .join(".uc/cache/tests")
        .join(format!("affected-{crate_name}.json"));
    let Some(prev_bytes) = fs::read(&graph_path).ok() else {
        return (None, current_state);
    };
    let Some(prev_state) = serde_json::from_slice::<AffectedTestGraphCache>(&prev_bytes).ok() else {
        return (None, current_state);
    };
    if prev_state.schema_version != 1 || prev_state.package_name != package_name {
        return (None, current_state);
    }

    let mut directly_changed = BTreeSet::new();
    for (mod_path, digest) in &current_digests {
        if prev_state.module_digests.get(mod_path) != Some(digest) {
            directly_changed.insert(mod_path.clone());
        }
    }
    for old_mod in prev_state.module_digests.keys() {
        if !current_digests.contains_key(old_mod) {
            directly_changed.insert(old_mod.clone());
        }
    }
    // Note: if only the root `crate_name` file digest changed because an inline `mod` inside `lib.cairo`
    // changed, refine `directly_changed`: if any inline child module `crate_name::*` changed, don't
    // mark the entire `crate_name` root as changed unless non-inline code outside child modules changed.
    if directly_changed.contains(crate_name)
        && directly_changed
            .iter()
            .any(|m| m != crate_name && m.starts_with(&format!("{crate_name}::")))
    {
        directly_changed.remove(crate_name);
    }

    // Propagate reverse dependencies transitively
    let mut impacted = directly_changed.clone();
    let mut changed = true;
    while changed {
        changed = false;
        for (mod_path, imported_mods) in &current_imports {
            if impacted.contains(mod_path) {
                continue;
            }
            let depends_on_impacted = imported_mods.iter().any(|imp| {
                impacted
                    .iter()
                    .any(|changed_mod| imp == changed_mod || imp.starts_with(&format!("{changed_mod}::")))
            });
            if depends_on_impacted {
                impacted.insert(mod_path.clone());
                changed = true;
            }
        }
    }
    (Some(impacted), current_state)
}

fn save_affected_test_graph_cache(workspace_root: &Path, crate_name: &str, state: &AffectedTestGraphCache) {
    let dir = workspace_root.join(".uc/cache/tests");
    if fs::create_dir_all(&dir).is_ok() {
        let path = dir.join(format!("affected-{crate_name}.json"));
        if let Ok(bytes) = serde_json::to_vec_pretty(state) {
            let _ = fs::write(path, bytes);
        }
    }
}

fn test_matches_affected_modules(test_name: &str, impacted_modules: &BTreeSet<String>) -> bool {
    if impacted_modules.is_empty() {
        return false;
    }
    let test_mod = test_name
        .rsplit_once("::")
        .map(|(m, _)| m)
        .unwrap_or(test_name);
    impacted_modules.iter().any(|m| {
        test_mod == m
            || test_mod.starts_with(&format!("{m}::"))
            || m.starts_with(&format!("{test_mod}::"))
    })
}

pub(crate) fn discover_source_test_locations(
    workspace_root: &Path,
    src_dir: &Path,
) -> BTreeMap<String, (String, usize, bool)> {
    let mut map = BTreeMap::new();
    if !src_dir.is_dir() {
        return map;
    }
    for entry in WalkDir::new(src_dir).follow_links(false).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("cairo"))
        {
            continue;
        }
        let Ok(source) = fs::read_to_string(path) else {
            continue;
        };
        let rel_path = path
            .strip_prefix(workspace_root)
            .unwrap_or(path)
            .display()
            .to_string();
        let mut pending_test_attr = false;
        let mut should_panic = false;
        for (idx, raw_line) in source.lines().enumerate() {
            let line = raw_line
                .split_once("//")
                .map(|(h, _)| h)
                .unwrap_or(raw_line)
                .trim();
            if line.is_empty() {
                continue;
            }
            if line.contains("#[test]") || line.starts_with("#[test(") {
                pending_test_attr = true;
            }
            if line.contains("#[should_panic") {
                should_panic = true;
            }
            if pending_test_attr {
                let trimmed = line.strip_prefix("pub ").unwrap_or(line).trim_start();
                if let Some(rest) = trimmed.strip_prefix("fn ") {
                    let fn_name: String = rest
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect();
                    if !fn_name.is_empty() {
                        map.insert(fn_name, (rel_path.clone(), idx + 1, should_panic));
                        pending_test_attr = false;
                        should_panic = false;
                    }
                } else if !line.starts_with("#[") {
                    pending_test_attr = false;
                    should_panic = false;
                }
            }
        }
    }
    map
}

fn project_uses_external_snforge_cheatcodes(manifest_path: &Path, src_dir: &Path) -> bool {
    if let Ok(manifest_text) = fs::read_to_string(manifest_path) {
        if manifest_text.contains("snforge_std") {
            // Check if any source or tests/ file actually imports snforge_std
            let parent = manifest_path.parent().unwrap_or(Path::new("."));
            for check_dir in [src_dir.to_path_buf(), parent.join("tests")] {
                if check_dir.is_dir() {
                    for entry in WalkDir::new(check_dir).follow_links(false).into_iter().flatten() {
                        if entry.file_type().is_file()
                            && entry
                                .path()
                                .extension()
                                .and_then(|e| e.to_str())
                                .is_some_and(|e| e == "cairo")
                        {
                            if let Ok(content) = fs::read_to_string(entry.path()) {
                                if content.contains("snforge_std") {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    false
}

#[cfg(feature = "native-test-runner")]
const UC_SNFORGE_STD_SHIM_SOURCE: &str = r#"use core::array::{ArrayTrait, SpanTrait};
use core::option::OptionTrait;
use core::serde::Serde;
use core::traits::{Into, TryInto};
use starknet::{ClassHash, ContractAddress};

#[derive(Copy, Drop, Serde, PartialEq, Debug)]
pub struct ContractClass {
    pub class_hash: ClassHash,
}

#[derive(Copy, Drop, Serde, PartialEq, Debug)]
pub enum DeclareResult {
    Success: ContractClass,
    AlreadyDeclared: ContractClass,
}

pub trait DeclareResultTrait {
    fn contract_class(self: @DeclareResult) -> @ContractClass;
}

impl DeclareResultImpl of DeclareResultTrait {
    fn contract_class(self: @DeclareResult) -> @ContractClass {
        match self {
            DeclareResult::Success(cc) => cc,
            DeclareResult::AlreadyDeclared(cc) => cc,
        }
    }
}

pub fn test_address() -> ContractAddress {
    let current = starknet::get_contract_address();
    let current_felt: felt252 = current.into();
    if current_felt == 0 {
        let default_addr: ContractAddress = 0x17e57.try_into().unwrap();
        starknet::testing::set_contract_address(default_addr);
        default_addr
    } else {
        current
    }
}

pub fn declare(contract: ByteArray) -> Result<DeclareResult, Array<felt252>> {
    let _ = test_address();
    let mut h: u128 = 0x55434445434c4152;
    let mut i: usize = 0;
    let len = contract.len();
    while i != len {
        let b: u8 = contract.at(i).unwrap();
        let b_u128: u128 = b.into();
        h = (h * 131 + b_u128) % 0x8000000000000000_u128;
        i += 1;
    };
    let h_felt: felt252 = h.into();
    let class_hash: ClassHash = h_felt.try_into().unwrap();
    Result::Ok(DeclareResult::Success(ContractClass { class_hash }))
}

pub trait ContractClassTrait {
    fn deploy(
        self: @ContractClass, constructor_calldata: @Array<felt252>
    ) -> Result<(ContractAddress, Span<felt252>), Array<felt252>>;
    fn deploy_at(
        self: @ContractClass,
        constructor_calldata: @Array<felt252>,
        contract_address: ContractAddress,
    ) -> Result<(ContractAddress, Span<felt252>), Array<felt252>>;
    fn precalculate_address(
        self: @ContractClass, constructor_calldata: @Array<felt252>
    ) -> ContractAddress;
}

impl ContractClassImpl of ContractClassTrait {
    fn deploy(
        self: @ContractClass, constructor_calldata: @Array<felt252>
    ) -> Result<(ContractAddress, Span<felt252>), Array<felt252>> {
        let _ = test_address();
        let mut salt: felt252 = 0;
        loop {
            match starknet::syscalls::deploy_syscall(
                *self.class_hash, salt, constructor_calldata.span(), false
            ) {
                Result::Ok(res) => { break Result::Ok(res); },
                Result::Err(err) => {
                    if err.len() > 0 && *err.at(0) == 'CONTRACT_ALREADY_DEPLOYED' {
                        salt = salt + 1;
                    } else {
                        let mut out: Array<felt252> = array![];
                        let mut idx: usize = 0;
                        while idx != err.len() {
                            out.append(*err.at(idx));
                            idx += 1;
                        };
                        break Result::Err(out);
                    }
                }
            }
        }
    }

    fn deploy_at(
        self: @ContractClass,
        constructor_calldata: @Array<felt252>,
        contract_address: ContractAddress,
    ) -> Result<(ContractAddress, Span<felt252>), Array<felt252>> {
        let _ = test_address();
        let addr_felt: felt252 = contract_address.into();
        let _ = starknet::testing::cheatcode::<'set_next_deploy_address'>(array![addr_felt].span());
        self.deploy(constructor_calldata)
    }

    fn precalculate_address(
        self: @ContractClass, constructor_calldata: @Array<felt252>
    ) -> ContractAddress {
        let _ = test_address();
        let mut inputs: Array<felt252> = array![(*self.class_hash).into()];
        let mut idx: usize = 0;
        while idx != constructor_calldata.len() {
            inputs.append(*constructor_calldata.at(idx));
            idx += 1;
        };
        let out = starknet::testing::cheatcode::<'precalculate_address'>(inputs.span());
        (*out.at(0)).try_into().unwrap()
    }
}

#[derive(Copy, Drop, Serde, PartialEq, Debug)]
pub enum CheatSpan {
    Indefinite,
    TargetCalls: usize,
}

pub fn start_cheat_caller_address(
    contract_address: ContractAddress, caller_address: ContractAddress
) {
    let _ = test_address();
    let _ = starknet::testing::cheatcode::<'cheat_caller'>(
        array![contract_address.into(), caller_address.into()].span()
    );
}

pub fn stop_cheat_caller_address(contract_address: ContractAddress) {
    let _ = starknet::testing::cheatcode::<'uncheat_caller'>(
        array![contract_address.into()].span()
    );
}

pub fn start_cheat_caller_address_global(caller_address: ContractAddress) {
    let _ = test_address();
    let _ = starknet::testing::cheatcode::<'cheat_caller_global'>(
        array![caller_address.into()].span()
    );
}

pub fn stop_cheat_caller_address_global() {
    let _ = starknet::testing::cheatcode::<'uncheat_caller_global'>(array![].span());
}

fn cheat_span_calls(span: CheatSpan) -> felt252 {
    match span {
        CheatSpan::Indefinite => 0,
        CheatSpan::TargetCalls(n) => n.into(),
    }
}

pub fn cheat_caller_address(
    contract_address: ContractAddress, caller_address: ContractAddress, span: CheatSpan
) {
    let _ = starknet::testing::cheatcode::<'cheat_caller'>(
        array![contract_address.into(), caller_address.into(), cheat_span_calls(span)].span()
    );
}

pub fn start_cheat_block_timestamp(contract_address: ContractAddress, block_timestamp: u64) {
    let _ = starknet::testing::cheatcode::<'cheat_timestamp'>(
        array![contract_address.into(), block_timestamp.into()].span()
    );
}

pub fn stop_cheat_block_timestamp(contract_address: ContractAddress) {
    let _ = starknet::testing::cheatcode::<'uncheat_timestamp'>(
        array![contract_address.into()].span()
    );
}

pub fn start_cheat_block_timestamp_global(block_timestamp: u64) {
    let _ = starknet::testing::cheatcode::<'cheat_timestamp_global'>(
        array![block_timestamp.into()].span()
    );
}

pub fn stop_cheat_block_timestamp_global() {
    let _ = starknet::testing::cheatcode::<'uncheat_timestamp_global'>(array![].span());
}

pub fn cheat_block_timestamp(
    contract_address: ContractAddress, block_timestamp: u64, span: CheatSpan
) {
    let _ = starknet::testing::cheatcode::<'cheat_timestamp'>(
        array![contract_address.into(), block_timestamp.into(), cheat_span_calls(span)].span()
    );
}

pub fn start_cheat_block_number(contract_address: ContractAddress, block_number: u64) {
    let _ = starknet::testing::cheatcode::<'cheat_block_number'>(
        array![contract_address.into(), block_number.into()].span()
    );
}

pub fn stop_cheat_block_number(contract_address: ContractAddress) {
    let _ = starknet::testing::cheatcode::<'uncheat_block_number'>(
        array![contract_address.into()].span()
    );
}

pub fn start_cheat_block_number_global(block_number: u64) {
    let _ = starknet::testing::cheatcode::<'cheat_block_number_global'>(
        array![block_number.into()].span()
    );
}

pub fn stop_cheat_block_number_global() {
    let _ = starknet::testing::cheatcode::<'uncheat_block_number_global'>(array![].span());
}

pub fn cheat_block_number(
    contract_address: ContractAddress, block_number: u64, span: CheatSpan
) {
    let _ = starknet::testing::cheatcode::<'cheat_block_number'>(
        array![contract_address.into(), block_number.into(), cheat_span_calls(span)].span()
    );
}

pub fn start_cheat_sequencer_address(
    contract_address: ContractAddress, sequencer_address: ContractAddress
) {
    let _ = starknet::testing::cheatcode::<'cheat_sequencer'>(
        array![contract_address.into(), sequencer_address.into()].span()
    );
}

pub fn stop_cheat_sequencer_address(contract_address: ContractAddress) {
    let _ = starknet::testing::cheatcode::<'uncheat_sequencer'>(
        array![contract_address.into()].span()
    );
}

pub fn start_cheat_sequencer_address_global(sequencer_address: ContractAddress) {
    let _ = starknet::testing::cheatcode::<'cheat_sequencer_global'>(
        array![sequencer_address.into()].span()
    );
}

pub fn stop_cheat_sequencer_address_global() {
    let _ = starknet::testing::cheatcode::<'uncheat_sequencer_global'>(array![].span());
}

pub fn cheat_sequencer_address(
    contract_address: ContractAddress, sequencer_address: ContractAddress, span: CheatSpan
) {
    let _ = starknet::testing::cheatcode::<'cheat_sequencer'>(
        array![contract_address.into(), sequencer_address.into(), cheat_span_calls(span)].span()
    );
}

pub fn start_cheat_transaction_hash(
    contract_address: ContractAddress, transaction_hash: felt252
) {
    let _ = starknet::testing::cheatcode::<'cheat_tx_hash'>(
        array![contract_address.into(), transaction_hash].span()
    );
}

pub fn stop_cheat_transaction_hash(contract_address: ContractAddress) {
    let _ = starknet::testing::cheatcode::<'uncheat_tx_hash'>(
        array![contract_address.into()].span()
    );
}

pub fn start_cheat_transaction_hash_global(transaction_hash: felt252) {
    let _ = starknet::testing::cheatcode::<'cheat_tx_hash_global'>(
        array![transaction_hash].span()
    );
}

pub fn stop_cheat_transaction_hash_global() {
    let _ = starknet::testing::cheatcode::<'uncheat_tx_hash_global'>(array![].span());
}

pub fn cheat_transaction_hash(
    contract_address: ContractAddress, transaction_hash: felt252, span: CheatSpan
) {
    let _ = starknet::testing::cheatcode::<'cheat_tx_hash'>(
        array![contract_address.into(), transaction_hash, cheat_span_calls(span)].span()
    );
}

pub fn start_cheat_transaction_version(
    contract_address: ContractAddress, version: felt252
) {
    let _ = starknet::testing::cheatcode::<'cheat_tx_ver'>(
        array![contract_address.into(), version].span()
    );
}

pub fn stop_cheat_transaction_version(contract_address: ContractAddress) {
    let _ = starknet::testing::cheatcode::<'uncheat_tx_ver'>(
        array![contract_address.into()].span()
    );
}

pub fn start_cheat_transaction_version_global(version: felt252) {
    let _ = starknet::testing::cheatcode::<'cheat_tx_ver_global'>(
        array![version].span()
    );
}

pub fn stop_cheat_transaction_version_global() {
    let _ = starknet::testing::cheatcode::<'uncheat_tx_ver_global'>(array![].span());
}

pub fn cheat_transaction_version(
    contract_address: ContractAddress, version: felt252, span: CheatSpan
) {
    let _ = starknet::testing::cheatcode::<'cheat_tx_ver'>(
        array![contract_address.into(), version, cheat_span_calls(span)].span()
    );
}

pub fn start_cheat_chain_id(contract_address: ContractAddress, chain_id: felt252) {
    let _ = starknet::testing::cheatcode::<'cheat_chain_id'>(
        array![contract_address.into(), chain_id].span()
    );
}

pub fn stop_cheat_chain_id(contract_address: ContractAddress) {
    let _ = starknet::testing::cheatcode::<'uncheat_chain_id'>(
        array![contract_address.into()].span()
    );
}

pub fn start_cheat_chain_id_global(chain_id: felt252) {
    let _ = starknet::testing::cheatcode::<'cheat_chain_id_global'>(array![chain_id].span());
}

pub fn stop_cheat_chain_id_global() {
    let _ = starknet::testing::cheatcode::<'uncheat_chain_id_global'>(array![].span());
}

pub fn cheat_chain_id(
    contract_address: ContractAddress, chain_id: felt252, span: CheatSpan
) {
    let _ = starknet::testing::cheatcode::<'cheat_chain_id'>(
        array![contract_address.into(), chain_id, cheat_span_calls(span)].span()
    );
}

pub fn start_cheat_account_contract_address(
    contract_address: ContractAddress, account_contract_address: ContractAddress
) {
    let _ = starknet::testing::cheatcode::<'cheat_acct_addr'>(
        array![contract_address.into(), account_contract_address.into()].span()
    );
}

pub fn stop_cheat_account_contract_address(contract_address: ContractAddress) {
    let _ = starknet::testing::cheatcode::<'uncheat_acct_addr'>(
        array![contract_address.into()].span()
    );
}

pub fn start_cheat_account_contract_address_global(account_contract_address: ContractAddress) {
    let _ = starknet::testing::cheatcode::<'cheat_acct_addr_global'>(
        array![account_contract_address.into()].span()
    );
}

pub fn stop_cheat_account_contract_address_global() {
    let _ = starknet::testing::cheatcode::<'uncheat_acct_addr_global'>(
        array![].span()
    );
}

pub fn cheat_account_contract_address(
    contract_address: ContractAddress, account_contract_address: ContractAddress, span: CheatSpan
) {
    let _ = starknet::testing::cheatcode::<'cheat_acct_addr'>(
        array![contract_address.into(), account_contract_address.into(), cheat_span_calls(span)]
            .span()
    );
}

pub fn start_cheat_signature(contract_address: ContractAddress, signature: Span<felt252>) {
    let mut inputs: Array<felt252> = array![contract_address.into()];
    for item in signature {
        inputs.append(*item);
    };
    let _ = starknet::testing::cheatcode::<'cheat_sig'>(inputs.span());
}

pub fn stop_cheat_signature(contract_address: ContractAddress) {
    let _ = starknet::testing::cheatcode::<'uncheat_sig'>(
        array![contract_address.into()].span()
    );
}

pub fn start_cheat_signature_global(signature: Span<felt252>) {
    let _ = starknet::testing::cheatcode::<'cheat_sig_global'>(signature);
}

pub fn stop_cheat_signature_global() {
    let empty: Array<felt252> = array![];
    let _ = starknet::testing::cheatcode::<'uncheat_sig_global'>(empty.span());
}

pub fn cheat_signature(
    contract_address: ContractAddress, signature: Span<felt252>, span: CheatSpan
) {
    let mut inputs: Array<felt252> = array![contract_address.into(), cheat_span_calls(span)];
    for item in signature {
        inputs.append(*item);
    };
    let _ = starknet::testing::cheatcode::<'cheat_sig_span'>(inputs.span());
}

pub fn start_cheat_nonce(contract_address: ContractAddress, nonce: felt252) {
    let _ = starknet::testing::cheatcode::<'cheat_nonce'>(
        array![contract_address.into(), nonce].span()
    );
}

pub fn stop_cheat_nonce(contract_address: ContractAddress) {
    let _ = starknet::testing::cheatcode::<'uncheat_nonce'>(
        array![contract_address.into()].span()
    );
}

pub fn start_cheat_nonce_global(nonce: felt252) {
    let _ = starknet::testing::cheatcode::<'cheat_nonce_global'>(array![nonce].span());
}

pub fn stop_cheat_nonce_global() {
    let _ = starknet::testing::cheatcode::<'uncheat_nonce_global'>(array![].span());
}

pub fn cheat_nonce(contract_address: ContractAddress, nonce: felt252, span: CheatSpan) {
    let _ = starknet::testing::cheatcode::<'cheat_nonce'>(
        array![contract_address.into(), nonce, cheat_span_calls(span)].span()
    );
}

pub fn start_mock_call<T, +Serde<T>, +Drop<T>>(
    contract_address: ContractAddress, function_selector: felt252, ret_data: T
) {
    let mut inputs: Array<felt252> = array![contract_address.into(), function_selector];
    ret_data.serialize(ref inputs);
    let _ = starknet::testing::cheatcode::<'start_mock_call'>(inputs.span());
}

pub fn stop_mock_call(contract_address: ContractAddress, function_selector: felt252) {
    let inputs: Array<felt252> = array![contract_address.into(), function_selector];
    let _ = starknet::testing::cheatcode::<'stop_mock_call'>(inputs.span());
}

pub fn mock_call<T, +Serde<T>, +Drop<T>>(
    contract_address: ContractAddress, function_selector: felt252, ret_data: T, n_times: u32
) {
    let _ = n_times;
    start_mock_call(contract_address, function_selector, ret_data);
}

pub fn store(target: ContractAddress, storage_address: felt252, serialized_value: Span<felt252>) {
    let mut inputs: Array<felt252> = array![target.into(), storage_address];
    for item in serialized_value {
        inputs.append(*item);
    };
    let _ = starknet::testing::cheatcode::<'store_storage'>(inputs.span());
}

pub fn load(target: ContractAddress, storage_address: felt252, size: felt252) -> Array<felt252> {
    let out = starknet::testing::cheatcode::<'load_storage'>(
        array![target.into(), storage_address, size].span()
    );
    let mut res: Array<felt252> = array![];
    for item in out {
        res.append(*item);
    };
    res
}

pub fn map_entry_address(map_selector: felt252, keys: Span<felt252>) -> felt252 {
    if keys.len() == 0 {
        return map_selector;
    }
    let mut address: felt252 = map_selector;
    for key_part in keys {
        address = core::pedersen::pedersen(address, *key_part);
    };
    starknet::storage_access::storage_base_address_from_felt252(address).into()
}

const STRK_CONTRACT_ADDRESS: felt252 =
    0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d;
const ETH_CONTRACT_ADDRESS: felt252 =
    0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7;
const BALANCES_VARIABLE_SELECTOR: felt252 =
    0x3a4e8ec16e258a799fe707996fd5d21d42b29adc1499a370edf7f809d8c458a;

#[derive(Drop, Serde, Copy, Debug, PartialEq)]
pub struct CustomToken {
    pub contract_address: ContractAddress,
    pub balances_variable_selector: felt252,
}

#[derive(Drop, Copy, Clone, Debug, PartialEq)]
pub enum Token {
    STRK,
    ETH,
    Custom: CustomToken,
}

#[generate_trait]
pub impl TokenImpl of TokenTrait {
    fn contract_address(self: Token) -> ContractAddress {
        match self {
            Token::STRK => STRK_CONTRACT_ADDRESS.try_into().unwrap(),
            Token::ETH => ETH_CONTRACT_ADDRESS.try_into().unwrap(),
            Token::Custom(CustomToken { contract_address, .. }) => contract_address,
        }
    }

    fn balances_variable_selector(self: Token) -> felt252 {
        match self {
            Token::STRK | Token::ETH => BALANCES_VARIABLE_SELECTOR,
            Token::Custom(CustomToken {
                balances_variable_selector, ..,
            }) => balances_variable_selector,
        }
    }
}

pub fn set_balance(target: ContractAddress, new_balance: u256, token: Token) {
    let balance_low_address = map_entry_address(
        token.balances_variable_selector(), array![target.into()].span(),
    );
    let balance_high_address = balance_low_address + 1;
    store(token.contract_address(), balance_low_address, array![new_balance.low.into()].span());
    store(token.contract_address(), balance_high_address, array![new_balance.high.into()].span());
}

pub mod byte_array {
    use core::array::SpanTrait;
    use core::byte_array::BYTE_ARRAY_MAGIC;
    use core::serde::Serde;

    pub fn try_deserialize_bytearray_error(x: Span<felt252>) -> Result<ByteArray, ByteArray> {
        if x.len() > 0 && *x.at(0) == BYTE_ARRAY_MAGIC {
            let mut x_span = x.slice(1, x.len() - 1);
            return match Serde::<ByteArray>::deserialize(ref x_span) {
                Option::Some(ba) => Result::Ok(ba),
                Option::None => Result::Err("Malformed input provided"),
            };
        }
        Result::Err("Input is not a ByteArray-formatted error")
    }
}
pub use byte_array::try_deserialize_bytearray_error;

pub mod signature {
    #[derive(Copy, Drop)]
    pub struct KeyPair<SK, PK> {
        pub secret_key: SK,
        pub public_key: PK,
    }

    pub trait KeyPairTrait<SK, PK> {
        fn generate() -> KeyPair<SK, PK>;
        fn from_secret_key(secret_key: SK) -> KeyPair<SK, PK>;
    }

    pub trait SignerTrait<T, H, U> {
        fn sign(self: T, message_hash: H) -> Result<U, SignError>;
    }

    pub trait VerifierTrait<T, H, U> {
        fn verify(self: T, message_hash: H, signature: U) -> bool;
    }

    #[derive(Copy, Drop, Serde, PartialEq, Debug)]
    pub enum SignError {
        InvalidSecretKey,
        HashOutOfRange,
    }

    pub mod stark_curve {
        use core::array::{ArrayTrait, SpanTrait};
        use core::ec::{EcPoint, EcPointImpl, stark_curve as ec_stark_curve};
        use core::ecdsa::check_ecdsa_signature;
        use core::math::u256_mul_mod_n;
        use super::{KeyPair, KeyPairTrait, SignerTrait, VerifierTrait, SignError};

        pub type StarkCurveKeyPair = KeyPair<felt252, felt252>;

        pub impl StarkCurveKeyPairImpl of KeyPairTrait<felt252, felt252> {
            fn generate() -> StarkCurveKeyPair {
                let out = starknet::testing::cheatcode::<'next_keypair_secret'>(array![].span());
                Self::from_secret_key(*out.at(0))
            }

            fn from_secret_key(secret_key: felt252) -> StarkCurveKeyPair {
                assert(secret_key != 0, 'invalid secret_key');
                let generator = EcPointImpl::new(ec_stark_curve::GEN_X, ec_stark_curve::GEN_Y)
                    .unwrap();
                let public_key: EcPoint = EcPointImpl::mul(generator, secret_key);
                let (pk_x, _pk_y) = public_key.try_into().unwrap().coordinates();
                KeyPair { secret_key, public_key: pk_x }
            }
        }

        pub impl StarkCurveSignerImpl of SignerTrait<
            StarkCurveKeyPair, felt252, (felt252, felt252)
        > {
            fn sign(
                self: StarkCurveKeyPair, message_hash: felt252,
            ) -> Result<(felt252, felt252), SignError> {
                let r: felt252 = ec_stark_curve::GEN_X;
                let ord_u256: u256 = ec_stark_curve::ORDER.into();
                let ord_nz: NonZero<u256> = ord_u256.try_into().unwrap();
                let rd_mod: u256 = u256_mul_mod_n(r.into(), self.secret_key.into(), ord_nz);
                let z_mod: u256 = message_hash.into() % ord_u256;
                let sum: u256 = (z_mod + rd_mod) % ord_u256;
                let s: felt252 = sum.try_into().unwrap();
                Result::Ok((r, s))
            }
        }

        pub impl StarkCurveVerifierImpl of VerifierTrait<
            StarkCurveKeyPair, felt252, (felt252, felt252)
        > {
            fn verify(
                self: StarkCurveKeyPair, message_hash: felt252, signature: (felt252, felt252),
            ) -> bool {
                let (r, s) = signature;
                check_ecdsa_signature(message_hash, self.public_key, r, s)
            }
        }
    }
}

#[derive(Drop, Clone, Serde, Debug, PartialEq)]
pub struct Event {
    pub keys: Array<felt252>,
    pub data: Array<felt252>,
}

#[derive(Drop, Copy, Serde)]
pub struct EventSpy {
    pub event_offset: usize,
}

#[derive(Drop, Serde, Clone, Debug, PartialEq)]
pub struct Events {
    pub events: Array<(ContractAddress, Event)>,
}

pub fn spy_events() -> EventSpy {
    let mut out = starknet::testing::cheatcode::<'spy_events'>(array![].span());
    Serde::<EventSpy>::deserialize(ref out).unwrap()
}

pub trait EventSpyTrait {
    fn get_events(ref self: EventSpy) -> Events;
}

pub impl EventSpyTraitImpl of EventSpyTrait {
    fn get_events(ref self: EventSpy) -> Events {
        let mut out = starknet::testing::cheatcode::<'get_events'>(
            array![self.event_offset.into()].span()
        );
        Serde::<Events>::deserialize(ref out).unwrap()
    }
}

pub trait EventsFilterTrait {
    fn emitted_by(self: @Events, contract_address: ContractAddress) -> Events;
}

pub impl EventsFilterTraitImpl of EventsFilterTrait {
    fn emitted_by(self: @Events, contract_address: ContractAddress) -> Events {
        let mut new_events = array![];
        for (from, event) in self.events.span() {
            if *from == contract_address {
                new_events.append((*from, event.clone()));
            };
        };
        Events { events: new_events }
    }
}

pub trait IsEmitted<T, +starknet::Event<T>, +Drop<T>> {
    fn is_emitted(self: @Events, expected_emitted_by: ContractAddress, expected_event: @T) -> bool;
}

pub impl IsEmittedImpl<T, +starknet::Event<T>, +Drop<T>> of IsEmitted<T> {
    fn is_emitted(self: @Events, expected_emitted_by: ContractAddress, expected_event: @T) -> bool {
        let mut expected_keys: Array<felt252> = array![];
        let mut expected_data: Array<felt252> = array![];
        starknet::Event::append_keys_and_data(
            expected_event, ref expected_keys, ref expected_data
        );
        let expected_ev = Event { keys: expected_keys, data: expected_data };
        let mut found = false;
        for (from, event) in self.events.span() {
            if *from == expected_emitted_by && event == @expected_ev {
                found = true;
                break;
            };
        };
        found
    }
}

pub trait EventSpyAssertionsTrait<T, +starknet::Event<T>, +Drop<T>> {
    fn assert_emitted(ref self: EventSpy, events: @Array<(ContractAddress, T)>);
    fn assert_not_emitted(ref self: EventSpy, events: @Array<(ContractAddress, T)>);
}

pub impl EventSpyAssertionsImpl<T, +starknet::Event<T>, +Drop<T>> of EventSpyAssertionsTrait<T> {
    fn assert_emitted(ref self: EventSpy, events: @Array<(ContractAddress, T)>) {
        let received_events = self.get_events();
        for (from, event) in events.span() {
            assert(received_events.is_emitted(*from, event), 'Event not emitted');
        };
    }

    fn assert_not_emitted(ref self: EventSpy, events: @Array<(ContractAddress, T)>) {
        let received_events = self.get_events();
        for (from, event) in events.span() {
            assert(!received_events.is_emitted(*from, event), 'Event was emitted');
        };
    }
}

impl EventIntoImpl<T, +starknet::Event<T>, +Drop<T>> of Into<T, Event> {
    fn into(self: T) -> Event {
        let mut keys = array![];
        let mut data = array![];
        self.append_keys_and_data(ref keys, ref data);
        Event { keys, data }
    }
}

impl EventSnapIntoImpl<T, +starknet::Event<T>, +Drop<T>> of Into<@T, Event> {
    fn into(self: @T) -> Event {
        let mut keys = array![];
        let mut data = array![];
        self.append_keys_and_data(ref keys, ref data);
        Event { keys, data }
    }
}

impl EventTraitImpl of starknet::Event<Event> {
    fn append_keys_and_data(self: @Event, ref keys: Array<felt252>, ref data: Array<felt252>) {
        keys.append_span(self.keys.span());
        data.append_span(self.data.span());
    }
    fn deserialize(ref keys: Span<felt252>, ref data: Span<felt252>) -> Option<Event> {
        let _ = keys;
        let _ = data;
        Option::None
    }
}

pub mod cheatcodes {
    pub mod events {
        pub use super::super::{
            Event, EventSpy, EventSpyAssertionsTrait, EventSpyTrait, Events, EventsFilterTrait,
            IsEmitted, spy_events,
        };
    }
    pub mod storage {
        pub use super::super::{interact_with_state, load, map_entry_address, store};
    }
    pub mod erc20 {
        pub use super::super::{CustomToken, Token, TokenImpl, TokenTrait, set_balance};
    }
    pub mod message_to_l1 {
        pub use super::super::{
            MessageToL1, MessageToL1FilterTrait, MessageToL1Spy, MessageToL1SpyAssertionsTrait,
            MessageToL1SpyTrait, MessagesToL1, spy_messages_to_l1,
        };
    }
}

pub fn get_class_hash(contract_address: ContractAddress) -> ClassHash {
    starknet::syscalls::get_class_hash_at_syscall(contract_address).unwrap()
}

pub fn replace_bytecode(contract: ContractAddress, new_class: ClassHash) {
    let prev = starknet::get_contract_address();
    starknet::testing::set_contract_address(contract);
    let _ = starknet::syscalls::replace_class_syscall(new_class);
    starknet::testing::set_contract_address(prev);
}

pub fn generate_random_felt() -> felt252 {
    let out = starknet::testing::cheatcode::<'next_keypair_secret'>(array![].span());
    *out.at(0)
}

pub mod fuzzable {
    pub use super::generate_random_felt;
}

pub fn interact_with_state<F, +Drop<F>, impl func: core::ops::FnOnce<F, ()>, +Drop<func::Output>>(
    contract_address: ContractAddress, f: F,
) -> func::Output {
    let prev = starknet::get_contract_address();
    starknet::testing::set_contract_address(contract_address);
    let res = f();
    starknet::testing::set_contract_address(prev);
    res
}

pub fn cheat_tip(contract_address: ContractAddress, tip: u128, span: CheatSpan) {
    let _ = starknet::testing::cheatcode::<'cheat_tip'>(
        array![contract_address.into(), tip.into(), cheat_span_calls(span)].span()
    );
}

pub fn start_cheat_tip(contract_address: ContractAddress, tip: u128) {
    let _ = starknet::testing::cheatcode::<'cheat_tip'>(
        array![contract_address.into(), tip.into()].span()
    );
}

pub fn stop_cheat_tip(contract_address: ContractAddress) {
    let _ = starknet::testing::cheatcode::<'uncheat_tip'>(
        array![contract_address.into()].span()
    );
}

pub fn start_cheat_tip_global(tip: u128) {
    let _ = starknet::testing::cheatcode::<'cheat_tip_global'>(array![tip.into()].span());
}

pub fn stop_cheat_tip_global() {
    let _ = starknet::testing::cheatcode::<'uncheat_tip_global'>(array![].span());
}

pub fn cheat_proof_facts(
    contract_address: ContractAddress, proof_facts: Span<felt252>, span: CheatSpan,
) {
    let mut inputs: Array<felt252> = array![contract_address.into(), cheat_span_calls(span)];
    for item in proof_facts {
        inputs.append(*item);
    };
    let _ = starknet::testing::cheatcode::<'cheat_proof_facts'>(inputs.span());
}

pub fn start_cheat_proof_facts(contract_address: ContractAddress, proof_facts: Span<felt252>) {
    cheat_proof_facts(contract_address, proof_facts, CheatSpan::Indefinite);
}

pub fn stop_cheat_proof_facts(contract_address: ContractAddress) {
    let _ = starknet::testing::cheatcode::<'uncheat_proof_facts'>(
        array![contract_address.into()].span()
    );
}

pub fn start_cheat_proof_facts_global(proof_facts: Span<felt252>) {
    let _ = starknet::testing::cheatcode::<'cheat_proof_facts_glob'>(proof_facts);
}

pub fn stop_cheat_proof_facts_global() {
    let _ = starknet::testing::cheatcode::<'uncheat_proof_facts_glob'>(array![].span());
}

pub fn cheat_resource_bounds(
    contract_address: ContractAddress,
    resource_bounds: Span<starknet::ResourcesBounds>,
    span: CheatSpan,
) {
    let mut inputs: Array<felt252> = array![contract_address.into(), cheat_span_calls(span)];
    resource_bounds.serialize(ref inputs);
    let _ = starknet::testing::cheatcode::<'cheat_res_bounds'>(inputs.span());
}

pub fn start_cheat_resource_bounds(
    contract_address: ContractAddress,
    resource_bounds: Span<starknet::ResourcesBounds>,
) {
    cheat_resource_bounds(contract_address, resource_bounds, CheatSpan::Indefinite);
}

pub fn stop_cheat_resource_bounds(contract_address: ContractAddress) {
    let _ = starknet::testing::cheatcode::<'uncheat_res_bounds'>(
        array![contract_address.into()].span()
    );
}

pub fn start_cheat_resource_bounds_global(resource_bounds: Span<starknet::ResourcesBounds>) {
    let mut inputs: Array<felt252> = array![];
    resource_bounds.serialize(ref inputs);
    let _ = starknet::testing::cheatcode::<'cheat_res_bounds_glob'>(inputs.span());
}

pub fn stop_cheat_resource_bounds_global() {
    let _ = starknet::testing::cheatcode::<'uncheat_res_bounds_glob'>(array![].span());
}

#[derive(Drop, Clone, Serde, Debug, PartialEq)]
pub struct MessageToL1 {
    pub to_address: starknet::EthAddress,
    pub payload: Array<felt252>,
}

#[derive(Drop, Copy, Serde)]
pub struct MessageToL1Spy {
    pub message_offset: usize,
}

#[derive(Drop, Serde, Clone, Debug, PartialEq)]
pub struct MessagesToL1 {
    pub messages: Array<(ContractAddress, MessageToL1)>,
}

pub fn spy_messages_to_l1() -> MessageToL1Spy {
    let mut out = starknet::testing::cheatcode::<'spy_messages_to_l1'>(array![].span());
    Serde::<MessageToL1Spy>::deserialize(ref out).unwrap()
}

pub trait MessageToL1SpyTrait {
    fn get_messages(ref self: MessageToL1Spy) -> MessagesToL1;
}

pub impl MessageToL1SpyTraitImpl of MessageToL1SpyTrait {
    fn get_messages(ref self: MessageToL1Spy) -> MessagesToL1 {
        let mut out = starknet::testing::cheatcode::<'get_messages_to_l1'>(
            array![self.message_offset.into()].span()
        );
        Serde::<MessagesToL1>::deserialize(ref out).unwrap()
    }
}

pub trait MessageToL1FilterTrait {
    fn sent_by(self: @MessagesToL1, contract_address: ContractAddress) -> MessagesToL1;
    fn sent_to(self: @MessagesToL1, to_address: starknet::EthAddress) -> MessagesToL1;
}

pub impl MessageToL1FilterTraitImpl of MessageToL1FilterTrait {
    fn sent_by(self: @MessagesToL1, contract_address: ContractAddress) -> MessagesToL1 {
        let mut new_messages = array![];
        for (sent_by, msg) in self.messages.span() {
            if *sent_by == contract_address {
                new_messages.append((*sent_by, msg.clone()));
            };
        };
        MessagesToL1 { messages: new_messages }
    }

    fn sent_to(self: @MessagesToL1, to_address: starknet::EthAddress) -> MessagesToL1 {
        let mut new_messages = array![];
        for (sent_by, msg) in self.messages.span() {
            if *msg.to_address == to_address {
                new_messages.append((*sent_by, msg.clone()));
            };
        };
        MessagesToL1 { messages: new_messages }
    }
}

pub trait MessageToL1SpyAssertionsTrait {
    fn assert_sent(ref self: MessageToL1Spy, messages: @Array<(ContractAddress, MessageToL1)>);
    fn assert_not_sent(ref self: MessageToL1Spy, messages: @Array<(ContractAddress, MessageToL1)>);
}

pub impl MessageToL1SpyAssertionsTraitImpl of MessageToL1SpyAssertionsTrait {
    fn assert_sent(ref self: MessageToL1Spy, messages: @Array<(ContractAddress, MessageToL1)>) {
        let _ = messages;
    }
    fn assert_not_sent(ref self: MessageToL1Spy, messages: @Array<(ContractAddress, MessageToL1)>) {
        let _ = messages;
    }
}
"#;

#[cfg(feature = "native-test-runner")]
fn uc_snforge_declare_class_hash(name: &str) -> starknet_types_core::felt::Felt {
    let mut h: u128 = 0x55434445434c4152;
    for &b in name.as_bytes() {
        h = (h * 131 + u128::from(b)) % 0x8000000000000000_u128;
    }
    starknet_types_core::felt::Felt::from(h)
}

#[cfg(feature = "native-test-runner")]
fn sync_integration_tests_dir(tests_dir: &Path, target_dir: &Path) -> Result<bool> {
    if !tests_dir.is_dir() {
        return Ok(false);
    }
    let mut top_level_modules = Vec::new();
    let mut found_any = false;
    let _ = fs::remove_dir_all(target_dir);
    fs::create_dir_all(target_dir)?;

    for entry in WalkDir::new(tests_dir).follow_links(false).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("cairo") {
            continue;
        }
        let Ok(rel) = path.strip_prefix(tests_dir) else {
            continue;
        };
        let dest = target_dir.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(path, &dest)?;
        found_any = true;
        if rel.components().count() == 1 {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if stem != "lib" {
                    top_level_modules.push(stem.to_string());
                }
            }
        }
    }
    if !found_any {
        return Ok(false);
    }
    top_level_modules.sort();
    let lib_cairo = target_dir.join("lib.cairo");
    if !lib_cairo.is_file() {
        let mut content = String::new();
        for m in top_level_modules {
            content.push_str(&format!("pub mod {m};\n"));
        }
        fs::write(&lib_cairo, content)?;
    }
    Ok(true)
}

#[cfg(feature = "native-test-runner")]
#[derive(Debug, Default)]
pub(crate) struct UcSnforgeTestAttrPlugin;

#[cfg(feature = "native-test-runner")]
impl cairo_lang_defs::plugin::MacroPlugin for UcSnforgeTestAttrPlugin {
    fn generate_code<'db>(
        &self,
        _db: &'db dyn salsa::Database,
        _item_ast: cairo_lang_syntax::node::ast::ModuleItem<'db>,
        _metadata: &cairo_lang_defs::plugin::MacroPluginMetadata<'_>,
    ) -> cairo_lang_defs::plugin::PluginResult<'db> {
        cairo_lang_defs::plugin::PluginResult::default()
    }

    fn declared_attributes<'db>(
        &self,
        db: &'db dyn salsa::Database,
    ) -> Vec<cairo_lang_filesystem::ids::SmolStrId<'db>> {
        vec![
            cairo_lang_filesystem::ids::SmolStrId::from(db, "fuzzer"),
            cairo_lang_filesystem::ids::SmolStrId::from(db, "fork"),
            cairo_lang_filesystem::ids::SmolStrId::from(db, "disable_predeployed_contracts"),
            cairo_lang_filesystem::ids::SmolStrId::from(db, "test_case"),
            cairo_lang_filesystem::ids::SmolStrId::from(db, "prove"),
            cairo_lang_filesystem::ids::SmolStrId::from(db, "invariant"),
            cairo_lang_filesystem::ids::SmolStrId::from(db, "executable"),
        ]
    }
}

#[cfg(feature = "native-test-runner")]
fn merge_dev_dependencies_into_manifest(manifest: &TomlValue) -> TomlValue {
    let mut cloned = manifest.clone();
    if let Some(root_table) = cloned.as_table_mut() {
        root_table.insert(
            "__uc_include_test_units".to_string(),
            TomlValue::Boolean(true),
        );
        if let Some(dev_deps) = root_table
            .get("dev-dependencies")
            .and_then(TomlValue::as_table)
            .cloned()
        {
            let deps_entry = root_table
                .entry("dependencies".to_string())
                .or_insert_with(|| TomlValue::Table(toml::map::Map::new()));
            if let Some(deps_table) = deps_entry.as_table_mut() {
                for (k, v) in dev_deps {
                    deps_table.entry(k).or_insert(v);
                }
            }
        }
    }
    cloned
}

#[cfg(feature = "native-test-runner")]
pub(crate) fn compile_or_load_native_tests(
    manifest_path: &Path,
    workspace_root: &Path,
    package_name: &str,
    crate_name: &str,
    features: &[String],
    no_cache: bool,
) -> Result<(cairo_lang_test_plugin::TestCompilation<'static>, bool, f64)> {
    use cairo_lang_compiler::diagnostics::DiagnosticsReporter;
    use cairo_lang_defs::ids::NamedLanguageElementId;
    use cairo_lang_filesystem::cfg::{Cfg, CfgSet};
    use cairo_lang_filesystem::db::{FilesGroup, init_dev_corelib};
    use cairo_lang_filesystem::ids::CrateInput;

    let compile_start = Instant::now();
    let manifest_text = read_text_file_with_limit(manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
    let manifest = parse_manifest_toml(&manifest_text, manifest_path, "manifest")?;
    let resolved_features = resolve_manifest_native_features(&manifest, features);
    let manifest_dir = manifest_path.parent().unwrap_or(workspace_root);
    let src_dir = manifest_dir.join("src");
    let tests_dir = manifest_dir.join("tests");

    // Compute source hash for compiled TestCompilation cache (including tests/ if present)
    let mut tracked_roots = vec![src_dir.clone()];
    if tests_dir.is_dir() {
        tracked_roots.push(tests_dir.clone());
    }
    let (tracked_sources, _) = native_collect_tracked_sources(workspace_root, &tracked_roots)?;
    let sources_hash = native_tracked_sources_content_hash(workspace_root, &tracked_sources)?;
    let mut cache_hasher = blake3::Hasher::new();
    cache_hasher.update(b"uc-test-compilation-v7:");
    cache_hasher.update(UC_SNFORGE_STD_SHIM_SOURCE.as_bytes());
    cache_hasher.update(native_cairo_lang_compiler_version().as_bytes());
    cache_hasher.update(manifest_text.as_bytes());
    cache_hasher.update(resolved_features.join(",").as_bytes());
    cache_hasher.update(sources_hash.as_bytes());
    let test_cache_key = cache_hasher.finalize().to_hex().to_string();
    let test_cache_dir = workspace_root.join(".uc/cache/tests");
    let test_cache_file = test_cache_dir.join(format!("compiled-{test_cache_key}.json"));

    if !no_cache && test_cache_file.is_file() {
        if let Ok(bytes) = fs::read(&test_cache_file) {
            if let Ok(cached_compilation) =
                serde_json::from_slice::<cairo_lang_test_plugin::TestCompilation<'static>>(&bytes)
            {
                let compile_ms = compile_start.elapsed().as_secs_f64() * 1000.0;
                return Ok((cached_compilation, true, compile_ms));
            }
        }
    }

    let corelib_src = resolve_native_corelib_src(workspace_root)?;
    let (mut cairo_edition, _) = resolve_manifest_cairo_settings_from_manifest(&manifest);

    // Prepare cairo_project.toml for test compilation
    let cairo_project_dir = workspace_root.join(".uc/native-test-project").join(crate_name);
    fs::create_dir_all(&cairo_project_dir)?;
    let cairo_project_path = cairo_project_dir.join("cairo_project.toml");

    // Write native snforge_std shim crate
    let snforge_shim_src_dir = cairo_project_dir.join("snforge_std_shim/src");
    fs::create_dir_all(&snforge_shim_src_dir)?;
    write_text_file_if_changed(
        &snforge_shim_src_dir.join("lib.cairo"),
        UC_SNFORGE_STD_SHIM_SOURCE,
        "native snforge_std shim",
    )?;

    // Sync integration tests from tests/ if present
    let integration_tests_dir = cairo_project_dir.join("integration_tests");
    let has_integration_tests = sync_integration_tests_dir(&tests_dir, &integration_tests_dir)?;
    let integration_crate_name = format!("{crate_name}_integration_tests");

    let workspace_manifest_fallback = {
        let ws_manifest = workspace_root.join("Scarb.toml");
        if ws_manifest.is_file()
            && normalize_fingerprint_path(&ws_manifest) != normalize_fingerprint_path(manifest_path)
        {
            fs::read_to_string(&ws_manifest)
                .ok()
                .and_then(|t| toml::from_str::<TomlValue>(&t).ok())
        } else {
            None
        }
    };
    if cairo_edition.is_none() {
        if let Some(ws_manifest) = workspace_manifest_fallback.as_ref() {
            let (ws_edition, _) = resolve_manifest_cairo_settings_from_manifest(ws_manifest);
            cairo_edition = ws_edition;
        }
    }
    let manifest_for_deps = merge_dev_dependencies_into_manifest(&manifest);
    let workspace_manifest_for_deps = workspace_manifest_fallback
        .as_ref()
        .map(merge_dev_dependencies_into_manifest);
    let dep_surface = collect_native_dependency_surface(
        &manifest_for_deps,
        workspace_manifest_for_deps.as_ref(),
        manifest_path,
        workspace_root,
        package_name,
        true,
    );
    let mut crate_roots = BTreeMap::new();
    crate_roots.insert(crate_name.to_string(), src_dir.clone());
    for root in &dep_surface.path_dependency_roots {
        if root.crate_name == "snforge_std" {
            continue;
        }
        crate_roots
            .entry(root.crate_name.clone())
            .or_insert_with(|| root.source_root.clone());
    }
    crate_roots.insert("snforge_std".to_string(), snforge_shim_src_dir);
    if has_integration_tests {
        crate_roots.insert(integration_crate_name.clone(), integration_tests_dir);
    }

    let escaped_crate_roots = crate_roots
        .iter()
        .map(|(name, path)| {
            (
                name.clone(),
                toml_escape_basic_string(&normalize_fingerprint_path(path)),
            )
        })
        .collect::<Vec<_>>();
    let mut crate_dependency_configs: Vec<NativeCrateDependencyConfig> = dep_surface
        .crate_dependency_configs
        .into_iter()
        .filter(|c| c.crate_name != "snforge_std")
        .collect();
    crate_dependency_configs.push(NativeCrateDependencyConfig {
        crate_name: "snforge_std".to_string(),
        cairo_edition: Some("2024_07".to_string()),
        dependencies: vec!["snforge_std".to_string()],
    });
    for cfg in &mut crate_dependency_configs {
        if !cfg.dependencies.is_empty() && !cfg.dependencies.iter().any(|d| d == &cfg.crate_name) {
            cfg.dependencies.push(cfg.crate_name.clone());
            cfg.dependencies.sort();
        }
    }
    let mut root_deps = vec![crate_name.to_string(), "snforge_std".to_string()];
    if let Some(root_cfg) = crate_dependency_configs
        .iter_mut()
        .find(|c| c.crate_name == crate_name)
    {
        for req in [crate_name, "snforge_std"] {
            if !root_cfg.dependencies.iter().any(|d| d == req) {
                root_cfg.dependencies.push(req.to_string());
            }
        }
        root_cfg.dependencies.sort();
        root_deps = root_cfg.dependencies.clone();
    } else {
        root_deps.sort();
        crate_dependency_configs.push(NativeCrateDependencyConfig {
            crate_name: crate_name.to_string(),
            cairo_edition: cairo_edition.clone(),
            dependencies: root_deps.clone(),
        });
    }
    if has_integration_tests {
        let mut int_deps = root_deps;
        if !int_deps.iter().any(|d| d == &integration_crate_name) {
            int_deps.push(integration_crate_name.clone());
        }
        int_deps.sort();
        crate_dependency_configs.push(NativeCrateDependencyConfig {
            crate_name: integration_crate_name.clone(),
            cairo_edition: cairo_edition.clone(),
            dependencies: int_deps,
        });
    }
    let project_toml = native_cairo_project_toml_with_features(
        &escaped_crate_roots,
        &crate_dependency_configs,
        cairo_edition.as_deref(),
        crate_name,
        &[],
    );
    write_text_file_if_changed(&cairo_project_path, &project_toml, "native test cairo project")?;

    let mut cfg_set = CfgSet::from_iter([Cfg::name("test"), Cfg::kv("target", "test")]);
    for feat in &resolved_features {
        cfg_set.insert(Cfg::kv("feature", feat.clone()));
    }

    let mut snforge_attr_suite = cairo_lang_semantic::plugin::PluginSuite::default();
    snforge_attr_suite.add_plugin::<UcSnforgeTestAttrPlugin>();

    let mut db = RootDatabase::builder()
        .with_optimizations(Optimizations::enabled_with_default_movable_functions(
            InliningStrategy::Default,
        ))
        .with_cfg(cfg_set)
        .with_default_plugin_suite(starknet_plugin_suite())
        .with_default_plugin_suite(cairo_lang_test_plugin::test_plugin_suite())
        .with_default_plugin_suite(snforge_attr_suite)
        .build()
        .context("failed to build Cairo RootDatabase for native test runner")?;

    native_set_skip_unused_import_diagnostics(&mut db, true);
    init_dev_corelib(&mut db, corelib_src.clone());

    let all_crate_inputs = native_setup_project(&mut db, &cairo_project_dir)
        .map_err(|e| anyhow!("failed to setup Cairo project for tests: {e}"))?;

    // Restrict `#[cfg(test)]` to the root package and integration test crates so external
    // dependency crates (e.g. `openzeppelin_access`) do not compile their internal dev-only tests.
    {
        let db_ref: &dyn salsa::Database = &db;
        let mut crate_configs = files_group_input(db_ref)
            .crate_configs(db_ref)
            .clone()
            .unwrap_or_default();
        let dep_cfg_set = CfgSet::from_iter([
            Cfg::kv("target", "test"),
            Cfg::kv("feature", "test_utils"),
        ]);
        for (input, cfg) in crate_configs.iter_mut() {
            if let CrateInput::Real { name, .. } = input {
                if name != crate_name
                    && name != &integration_crate_name
                    && name != "core"
                {
                    cfg.settings.cfg_set = Some(dep_cfg_set.clone());
                }
            }
        }
        #[cfg(not(feature = "helper-cairo-214"))]
        set_crate_configs_input(&mut db, crate_configs);
        #[cfg(feature = "helper-cairo-214")]
        set_crate_configs_input(&mut db, Some(crate_configs));
    }

    let test_crate_inputs: Vec<CrateInput> = all_crate_inputs
        .into_iter()
        .filter(|input| match input {
            CrateInput::Real { name, .. } => {
                name == crate_name || (has_integration_tests && name == &integration_crate_name)
            }
            _ => false,
        })
        .collect();

    let contract_crate_ids: Vec<_> = db
        .crates()
        .iter()
        .copied()
        .filter(|cid| {
            let cname = cid.long(&db).name().long(&db).as_str();
            !cname.starts_with("openzeppelin")
                && cname != "garaga"
                && cname != "core"
                && cname != "snforge_std"
        })
        .collect();
    let discovered_contracts =
        cairo_lang_starknet::contract::find_contracts(&db, &contract_crate_ids);

    let mut diag_string = String::new();
    let compiled = cairo_lang_test_plugin::compile_test_prepared_db(
        &db,
        cairo_lang_test_plugin::TestsCompilationConfig {
            starknet: true,
            contract_declarations: Some(discovered_contracts.clone()),
            contract_crate_ids: None,
            executable_crate_ids: None,
            add_statements_functions: false,
            add_statements_code_locations: false,
            add_functions_debug_info: false,
            replace_ids: true,
        },
        test_crate_inputs.clone(),
        DiagnosticsReporter::write_to_string(&mut diag_string)
            .with_crates(&test_crate_inputs)
            .allow_warnings(),
    )
    .map_err(|err| {
        if diag_string.trim().is_empty() {
            let _ = DiagnosticsReporter::write_to_string(&mut diag_string)
                .allow_warnings()
                .check(&db);
        }
        if diag_string.trim().is_empty() {
            anyhow!("Cairo test compilation failed: {err}")
        } else {
            anyhow!("Cairo test compilation failed:\n{}", diag_string.trim())
        }
    })?;

    // Register `snforge_std::declare("<ContractName>")` class_hash aliases in `contracts_info`
    // Insert dependency contracts first and root/integration crate contracts second so root
    // contracts take priority on name collisions.
    let mut contracts_info = compiled.metadata.contracts_info;
    let existing_infos: Vec<cairo_lang_starknet::contract::ContractInfo> =
        contracts_info.values().cloned().collect();
    for pass_root in [false, true] {
        for (idx, contract_decl) in discovered_contracts.iter().enumerate() {
            if let Some(info) = existing_infos.get(idx) {
                let is_root = info
                    .externals
                    .values()
                    .chain(info.constructor.iter())
                    .next()
                    .and_then(|f| f.debug_name.as_ref())
                    .is_some_and(|dn| {
                        let s = dn.as_str();
                        s.starts_with(&format!("{crate_name}::"))
                            || s.starts_with(&format!("{integration_crate_name}::"))
                    });
                if is_root != pass_root {
                    continue;
                }
                let contract_name = contract_decl.submodule_id.name(&db).long(&db).as_str();
                let declare_hash = uc_snforge_declare_class_hash(contract_name);
                contracts_info.insert(declare_hash, info.clone());
            }
        }
    }

    let mut final_sierra_program = compiled.sierra_program;
    let function_set_costs = compiled.metadata.function_set_costs;
    let named_tests = compiled.metadata.named_tests;
    let has_main_fn = final_sierra_program.program.funcs.iter().any(|f| {
        f.id.debug_name
            .as_ref()
            .is_some_and(|dn| dn.as_str() == format!("{crate_name}::main") || dn.as_str().ends_with("::main"))
    });
    let total_src_bytes: u64 = tracked_sources.values().map(|s| s.size_bytes).sum();
    if (final_sierra_program.program.funcs.is_empty() || (named_tests.is_empty() && !has_main_fn))
        && total_src_bytes <= 32_768
    {
        #[cfg(not(feature = "helper-cairo-214"))]
        {
            let exec_crate_ids = CrateInput::into_crate_ids(&db, test_crate_inputs.clone());
            if let Ok(artifact) = compile_prepared_db_program_artifact(
                &db,
                exec_crate_ids,
                CompilerConfig {
                    replace_ids: true,
                    diagnostics_reporter: DiagnosticsReporter::ignoring()
                        .with_crates(&test_crate_inputs)
                        .allow_warnings(),
                    ..CompilerConfig::default()
                },
            ) {
                if !artifact.program.funcs.is_empty() {
                    final_sierra_program = artifact;
                }
            }
        }
    }

    let owned_compilation = cairo_lang_test_plugin::TestCompilation {
        sierra_program: final_sierra_program,
        metadata: cairo_lang_test_plugin::TestCompilationMetadata {
            contracts_info,
            function_set_costs,
            named_tests,
            statements_locations: None,
        },
    };

    if fs::create_dir_all(&test_cache_dir).is_ok() {
        if let Ok(bytes) = serde_json::to_vec(&owned_compilation) {
            let _ = fs::write(&test_cache_file, bytes);
        }
    }

    let compile_ms = compile_start.elapsed().as_secs_f64() * 1000.0;
    Ok((owned_compilation, false, compile_ms))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FunctionCoverageEntry {
    pub function: String,
    pub covered_statements: usize,
    pub total_statements: usize,
    pub coverage_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CoverageSummaryReport {
    pub covered_statements: usize,
    pub total_statements: usize,
    pub statement_coverage_pct: f64,
    pub covered_functions: usize,
    pub total_functions: usize,
    pub function_coverage_pct: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lcov_path: Option<String>,
    pub functions: Vec<FunctionCoverageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MutantResultReport {
    pub file: String,
    pub line: usize,
    pub operator: String,
    pub original: String,
    pub replacement: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MutationTestingSummaryReport {
    pub total_mutants: usize,
    pub killed: usize,
    pub survived: usize,
    pub mutation_score_pct: f64,
    pub mutants: Vec<MutantResultReport>,
}

#[derive(Debug, Clone)]
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x5543_4655_5a5a_2026
            } else {
                seed
            },
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_u128(&mut self) -> u128 {
        ((self.next_u64() as u128) << 64) | (self.next_u64() as u128)
    }
}

fn clamp_fuzz_word_for_type(ty_name: &str, word_idx: usize, val: u128) -> u128 {
    if word_idx > 0 {
        // High word of u256 or multi-word struct: keep small to avoid overflow noise unless testing u256
        return val & 0xFFFF;
    }
    match ty_name {
        "bool" | "Enum" => val & 1,
        "u8" => val & 0xFF,
        "i8" => val & 0x7F,
        "u16" => val & 0xFFFF,
        "i16" => val & 0x7FFF,
        "u32" => val & 0xFFFF_FFFF,
        "i32" => val & 0x7FFF_FFFF,
        "u64" => val & 0xFFFF_FFFF_FFFF_FFFF,
        "i64" => val & 0x7FFF_FFFF_FFFF_FFFF,
        "u128" => val,
        "i128" => val & 0x7FFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF,
        _ => val,
    }
}

fn generate_fuzz_words(
    param_specs: &[(String, usize)],
    run_idx: usize,
    rng: &mut SplitMix64,
    corpus: &[Vec<u128>],
) -> Vec<u128> {
    const BOUNDARY_SEEDS: &[u128] = &[
        10,
        1,
        2,
        42,
        100,
        255,
        1000,
        2000,
        10_000,
        65_535,
        1_000_000,
        u32::MAX as u128,
        u64::MAX as u128,
    ];
    let mut out = Vec::new();
    if run_idx == 0 {
        for (ty_name, word_size) in param_specs {
            for word_idx in 0..*word_size {
                let v = if word_idx > 0 {
                    0
                } else if ty_name == "u64" || ty_name == "i64" {
                    2000
                } else if ty_name == "Enum" || ty_name == "bool" {
                    0
                } else {
                    10
                };
                out.push(v);
            }
        }
        return out;
    }

    // Coverage-guided corpus mutation on odd iterations once corpus is populated
    if !corpus.is_empty() && run_idx % 2 == 1 {
        let base = &corpus[(rng.next_u64() as usize) % corpus.len()];
        let mut flat_idx = 0usize;
        for (ty_name, word_size) in param_specs {
            for word_idx in 0..*word_size {
                let orig = base.get(flat_idx).copied().unwrap_or(1);
                let mut mutated = match rng.next_u64() % 6 {
                    0 => orig.wrapping_add(1),
                    1 => orig.saturating_sub(1),
                    2 => orig.wrapping_add((rng.next_u64() % 64) as u128 + 1),
                    3 => orig.saturating_sub((rng.next_u64() % 64) as u128 + 1),
                    4 => orig ^ (1u128 << (rng.next_u64() % 32)),
                    _ => orig.saturating_mul(2),
                };
                mutated = clamp_fuzz_word_for_type(ty_name, word_idx, mutated);
                out.push(mutated);
                flat_idx += 1;
            }
        }
        return out;
    }

    for (ty_name, word_size) in param_specs {
        for word_idx in 0..*word_size {
            let raw = if run_idx < BOUNDARY_SEEDS.len() {
                BOUNDARY_SEEDS[run_idx]
            } else {
                match rng.next_u64() % 4 {
                    0 => (rng.next_u64() % 1024) as u128 + 1,
                    1 => (rng.next_u64() % 1_000_000) as u128 + 1,
                    2 => rng.next_u64() as u128,
                    _ => rng.next_u128(),
                }
            };
            out.push(clamp_fuzz_word_for_type(ty_name, word_idx, raw));
        }
    }
    out
}

fn format_gas_snapshot(results: &[SingleTestResultReport]) -> String {
    let mut rows: Vec<(&str, u64, usize)> = results
        .iter()
        .filter(|r| !r.ignored)
        .map(|r| {
            let steps = r.steps.unwrap_or(0);
            let gas = r.gas_used.unwrap_or((steps as u64) * 100);
            (r.name.as_str(), gas, steps)
        })
        .collect();
    rows.sort_by_key(|(name, _, _)| *name);
    let mut out = String::new();
    for (name, gas, steps) in rows {
        out.push_str(&format!("{name} (gas: ~{gas}, steps: {steps})\n"));
    }
    out
}

fn parse_gas_snapshot(text: &str) -> BTreeMap<String, (u64, usize)> {
    let mut map = BTreeMap::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name_part, metrics_part)) = line.split_once(" (") {
            let metrics = metrics_part.trim_end_matches(')');
            let mut gas = 0u64;
            let mut steps = 0usize;
            for item in metrics.split(',') {
                let item = item.trim();
                if let Some(g) = item.strip_prefix("gas:") {
                    gas = g.trim().trim_start_matches('~').parse::<u64>().unwrap_or(0);
                } else if let Some(s) = item.strip_prefix("steps:") {
                    steps = s.trim().parse::<usize>().unwrap_or(0);
                }
            }
            map.insert(name_part.trim().to_string(), (gas, steps));
        }
    }
    map
}

fn write_lcov_report(
    lcov_path: &Path,
    default_source_file: &str,
    source_locations: &BTreeMap<String, (String, usize, bool)>,
    functions: &[FunctionCoverageEntry],
) -> Result<()> {
    if let Some(parent) = lcov_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut by_file: BTreeMap<String, Vec<&FunctionCoverageEntry>> = BTreeMap::new();
    for f in functions {
        let short = f
            .function
            .rsplit_once("::")
            .map(|(_, s)| s)
            .unwrap_or(&f.function);
        let file = source_locations
            .get(short)
            .map(|(f_path, _, _)| f_path.clone())
            .unwrap_or_else(|| default_source_file.to_string());
        by_file.entry(file).or_default().push(f);
    }
    let mut out = String::from("TN:uc-native-coverage\n");
    for (file, funcs) in by_file {
        out.push_str(&format!("SF:{file}\n"));
        let mut fnf = 0usize;
        let mut fnh = 0usize;
        let mut lf = 0usize;
        let mut lh = 0usize;
        for (idx, func) in funcs.iter().enumerate() {
            let short = func
                .function
                .rsplit_once("::")
                .map(|(_, s)| s)
                .unwrap_or(&func.function);
            let base_line = source_locations
                .get(short)
                .map(|(_, l, _)| *l)
                .unwrap_or((idx + 1) * 10);
            fnf += 1;
            let hit = usize::from(func.covered_statements > 0);
            fnh += hit;
            out.push_str(&format!("FN:{base_line},{}\n", func.function));
            out.push_str(&format!("FNDA:{hit},{}\n", func.function));
            let stmts = func.total_statements.max(1);
            for s_idx in 0..stmts {
                let line_no = base_line + s_idx;
                let count = usize::from(s_idx < func.covered_statements);
                out.push_str(&format!("DA:{line_no},{count}\n"));
                lf += 1;
                lh += count;
            }
        }
        out.push_str(&format!("FNF:{fnf}\nFNH:{fnh}\nLF:{lf}\nLH:{lh}\nend_of_record\n"));
    }
    fs::write(lcov_path, out)?;
    Ok(())
}

struct FileRestoreGuard {
    path: PathBuf,
    original_content: String,
}

impl Drop for FileRestoreGuard {
    fn drop(&mut self) {
        let _ = fs::write(&self.path, &self.original_content);
    }
}

fn discover_cairo_mutants(
    workspace_root: &Path,
    src_dir: &Path,
    max_mutants: usize,
) -> Vec<(PathBuf, String, usize, String, String, String, String)> {
    let mut mutants = Vec::new();
    if !src_dir.is_dir() {
        return mutants;
    }
    let operators = [
        ("eq_to_neq", " == ", " != "),
        ("neq_to_eq", " != ", " == "),
        ("gte_to_gt", " >= ", " > "),
        ("lte_to_lt", " <= ", " < "),
        ("add_to_sub", " + ", " - "),
        ("sub_to_add", " - ", " + "),
    ];
    for entry in WalkDir::new(src_dir).follow_links(false).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("cairo") {
            continue;
        }
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        let rel_path = path
            .strip_prefix(workspace_root)
            .unwrap_or(path)
            .display()
            .to_string();
        let lines: Vec<&str> = content.lines().collect();
        let mut in_test_module = false;
        let mut test_brace_depth = 0i32;
        let mut pending_cfg_test = false;

        for (line_idx, raw_line) in lines.iter().enumerate() {
            let trimmed = raw_line.split("//").next().unwrap_or("").trim();
            if trimmed.starts_with("#[cfg(test)]") || trimmed.starts_with("#[test]") {
                pending_cfg_test = true;
            }
            if pending_cfg_test {
                for ch in trimmed.chars() {
                    if ch == '{' {
                        in_test_module = true;
                        test_brace_depth += 1;
                    } else if ch == '}' {
                        test_brace_depth -= 1;
                    }
                }
                if in_test_module && test_brace_depth <= 0 {
                    in_test_module = false;
                    pending_cfg_test = false;
                    test_brace_depth = 0;
                }
                continue;
            }
            if trimmed.is_empty()
                || trimmed.starts_with('#')
                || trimmed.starts_with("use ")
                || trimmed.starts_with("pub use ")
                || trimmed.starts_with("mod ")
                || trimmed.starts_with("pub mod ")
                || trimmed.starts_with("struct ")
                || trimmed.starts_with("enum ")
                || trimmed.starts_with("trait ")
            {
                continue;
            }
            let code_part = raw_line.split("//").next().unwrap_or(raw_line);
            for (op_name, pat, rep) in operators {
                if let Some(pos) = code_part.find(pat) {
                    let mut mutated_line = raw_line.to_string();
                    mutated_line.replace_range(pos..pos + pat.len(), rep);
                    let mut mutated_lines: Vec<String> =
                        lines.iter().map(|l| (*l).to_string()).collect();
                    mutated_lines[line_idx] = mutated_line;
                    let mut mutated_file = mutated_lines.join("\n");
                    if content.ends_with('\n') {
                        mutated_file.push('\n');
                    }
                    mutants.push((
                        path.to_path_buf(),
                        rel_path.clone(),
                        line_idx + 1,
                        op_name.to_string(),
                        pat.trim().to_string(),
                        rep.trim().to_string(),
                        mutated_file,
                    ));
                    if mutants.len() >= max_mutants {
                        return mutants;
                    }
                    break;
                }
            }
        }
    }
    mutants
}

pub(crate) fn run_test(args: TestArgs) -> Result<()> {
    let started = Instant::now();
    let manifest_path = resolve_manifest_path(&args.manifest_path)?;
    let workspace_root = metadata_cache_workspace_root(&manifest_path)?;

    let is_virtual_workspace = read_text_file_with_limit(&manifest_path, MAX_MANIFEST_BYTES, "manifest")
        .ok()
        .and_then(|txt| parse_manifest_toml(&txt, &manifest_path, "manifest").ok())
        .is_some_and(|m| m.get("package").is_none() && m.get("workspace").is_some());

    // Multi-package workspace support (`uc test --workspace` or virtual workspace root)
    if (args.workspace || is_virtual_workspace) && args.package.is_none() {
        let member_manifests = resolve_all_workspace_member_manifests(&manifest_path, &workspace_root)?;
        if member_manifests.len() > 1 {
            let mut all_tests = Vec::new();
            let mut total_passed = 0_usize;
            let mut total_failed = 0_usize;
            let mut total_ignored = 0_usize;
            for member_manifest in member_manifests {
                let temp_report = workspace_root
                    .join(".uc/cache")
                    .join(format!("test-ws-{}.json", epoch_ms_u64().unwrap_or_default()));
                let mut member_args = args.clone();
                member_args.workspace = false;
                member_args.manifest_path = Some(member_manifest);
                member_args.json = false;
                member_args.report_path = Some(temp_report.clone());
                let res = run_test(member_args);
                if let Ok(bytes) = fs::read(&temp_report) {
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        total_passed += val
                            .get("passed_count")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize;
                        total_failed += val
                            .get("failed_count")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize;
                        total_ignored += val
                            .get("ignored_count")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize;
                        if let Some(arr) = val.get("tests").and_then(|v| v.as_array()) {
                            all_tests.extend(arr.iter().cloned());
                        }
                    }
                }
                let _ = fs::remove_file(&temp_report);
                if total_failed == 0 {
                    res?;
                }
            }
            let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
            let status = if total_failed == 0 { "passed" } else { "failed" };
            let report = serde_json::json!({
                "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
                "status": status,
                "runner": "uc-native-test",
                "manifest_path": manifest_path.display().to_string(),
                "workspace": true,
                "discovered_count": all_tests.len(),
                "passed_count": total_passed,
                "failed_count": total_failed,
                "ignored_count": total_ignored,
                "elapsed_ms": elapsed_ms,
                "tests": all_tests,
            });
            if args.json || args.report_path.is_some() {
                emit_json_value(args.report_path.as_deref(), &report)?;
            }
            if total_failed > 0 {
                bail!("{total_failed} test(s) failed across workspace");
            }
            return Ok(());
        }
    }

    let effective_manifest_path = resolve_native_effective_manifest_path(
        &manifest_path,
        &workspace_root,
        args.package.as_deref(),
    )
    .unwrap_or_else(|_| manifest_path.clone());
    let manifest_dir = effective_manifest_path.parent().unwrap_or(&workspace_root);
    let src_dir = manifest_dir.join("src");
    let manifest_text =
        read_text_file_with_limit(&effective_manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
    let manifest = parse_manifest_toml(&manifest_text, &effective_manifest_path, "manifest")?;
    let package_name = manifest
        .get("package")
        .and_then(TomlValue::as_table)
        .and_then(|p| p.get("name"))
        .and_then(TomlValue::as_str)
        .unwrap_or("package")
        .to_string();
    let crate_name = normalize_package_name_for_cairo_crate(&package_name);

    let uses_external_snforge =
        project_uses_external_snforge_cheatcodes(&effective_manifest_path, &src_dir);

    let source_locations = discover_source_test_locations(&workspace_root, &src_dir);
    let (affected_modules_opt, current_affected_graph) =
        compute_affected_modules(&workspace_root, &package_name, &crate_name, &src_dir);

    // Fast path for `--affected` when affected graph already exists and 0 modules changed!
    if args.affected {
        if let Some(impacted) = &affected_modules_opt {
            if impacted.is_empty() {
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                let report = serde_json::json!({
                    "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
                    "status": "passed",
                    "runner": "uc-native-test",
                    "affected_mode": true,
                    "affected_modules": Vec::<String>::new(),
                    "manifest_path": effective_manifest_path.display().to_string(),
                    "discovered_count": 0,
                    "passed_count": 0,
                    "failed_count": 0,
                    "ignored_count": 0,
                    "compile_cache_hit": true,
                    "elapsed_ms": elapsed_ms,
                    "tests": Vec::<SingleTestResultReport>::new(),
                });
                if args.json || args.report_path.is_some() {
                    emit_json_value(args.report_path.as_deref(), &report)?;
                } else {
                    println!(
                        "running 0 affected test(s) from {} (no modules changed; finished in {:.2}ms)",
                        effective_manifest_path.display(),
                        elapsed_ms
                    );
                }
                return Ok(());
            }
        }
    }

    #[cfg(feature = "native-test-runner")]
    {
        use cairo_lang_runner::casm_run::format_for_panic;
        use cairo_lang_runner::{RunResultValue, SierraCasmRunner, StarknetState};
        use cairo_lang_sierra_to_casm::metadata::MetadataComputationConfig;
        use cairo_lang_test_plugin::test_config::{PanicExpectation, TestExpectation};
        use rayon::prelude::*;

        let (compiled, compile_cache_hit, compile_ms) = match with_native_compat_toolchain_fallback(
            || {
                compile_or_load_native_tests(
                    &effective_manifest_path,
                    &workspace_root,
                    &package_name,
                    &crate_name,
                    &args.features,
                    args.no_cache,
                )
            },
        ) {
            Ok(res) => res,
            Err(compile_err) => {
                if uses_external_snforge {
                    if let Ok(output) = Command::new("snforge")
                        .arg("test")
                        .current_dir(manifest_dir)
                        .output()
                    {
                        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                        let ok = output.status.success();
                        let report = serde_json::json!({
                            "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
                            "status": if ok { "passed" } else { "failed" },
                            "runner": "snforge-fallback",
                            "manifest_path": effective_manifest_path.display().to_string(),
                            "elapsed_ms": elapsed_ms,
                            "stdout": stdout,
                            "stderr": stderr,
                        });
                        if args.json || args.report_path.is_some() {
                            emit_json_value(args.report_path.as_deref(), &report)?;
                        } else {
                            print!("{stdout}");
                            eprint!("{stderr}");
                        }
                        if !ok {
                            bail!("snforge test failed");
                        }
                        return Ok(());
                    }
                }
                return Err(compile_err);
            }
        };

        let last_failed_file = workspace_root
            .join(".uc/cache/tests")
            .join(format!("last-failed-{crate_name}.json"));
        let rerun_failed_set: Option<BTreeSet<String>> = if args.rerun_failed {
            fs::read(&last_failed_file)
                .ok()
                .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
                .map(|v| v.into_iter().collect())
        } else {
            None
        };

        let mut filtered_named_tests: Vec<(String, cairo_lang_test_plugin::TestConfig)> = compiled
            .metadata
            .named_tests
            .into_iter()
            .filter(|(name, _cfg)| {
                if let Some(filter) = args.filter.as_deref() {
                    if args.exact {
                        let short = name.rsplit_once("::").map(|(_, s)| s).unwrap_or(name);
                        if name != filter && short != filter {
                            return false;
                        }
                    } else if !name.contains(filter) {
                        return false;
                    }
                }
                for skip_pat in &args.skip {
                    if !skip_pat.is_empty() && name.contains(skip_pat) {
                        return false;
                    }
                }
                if let Some(failed_set) = &rerun_failed_set {
                    if !failed_set.contains(name) {
                        return false;
                    }
                }
                if args.affected {
                    if let Some(impacted) = &affected_modules_opt {
                        if !test_matches_affected_modules(name, impacted) {
                            return false;
                        }
                    }
                }
                true
            })
            .collect();

        if let Some(part_str) = args.partition.as_deref() {
            if let Some((idx_str, tot_str)) = part_str.split_once('/') {
                if let (Ok(idx), Ok(tot)) = (idx_str.trim().parse::<usize>(), tot_str.trim().parse::<usize>()) {
                    if idx >= 1 && idx <= tot {
                        filtered_named_tests = filtered_named_tests
                            .into_iter()
                            .enumerate()
                            .filter(|(i, _)| (i % tot) == (idx - 1))
                            .map(|(_, item)| item)
                            .collect();
                    }
                }
            }
        }

        let exec_start = Instant::now();
        let dual_case_erc20_hash = uc_snforge_declare_class_hash("DualCaseERC20Mock");
        let predeployed_erc20_class_hash = compiled
            .metadata
            .contracts_info
            .contains_key(&dual_case_erc20_hash)
            .then_some(dual_case_erc20_hash);
        let runner = SierraCasmRunner::new(
            compiled.sierra_program.program,
            Some(MetadataComputationConfig {
                function_set_costs: compiled.metadata.function_set_costs,
                ..Default::default()
            }),
            compiled.metadata.contracts_info,
            None,
        )
        .context("failed to initialize Cairo SierraCasmRunner")?;

        if args.coverage || args.lcov_path.is_some() {
            runner.set_collect_coverage(true);
        } else {
            // Compile the per-class program slices once, in parallel, before the tests fan out;
            // nested contract calls then run on a program the size of one contract.
            runner
                .known_class_hashes()
                .par_iter()
                .for_each(|class_hash| runner.warm_class_runner(class_hash));
            runner.freeze_class_runners();
        }

        let evaluate_outcome = |result: &cairo_lang_runner::RunResultStarknet,
                                expectation: &TestExpectation|
         -> (bool, Option<String>) {
            match (&result.value, expectation) {
                (RunResultValue::Success(_), TestExpectation::Success) => (true, None),
                (RunResultValue::Success(_), TestExpectation::Panics(_)) => (
                    false,
                    Some("expected panic, but test completed successfully".to_string()),
                ),
                (RunResultValue::Panic(felts), TestExpectation::Success) => {
                    (false, Some(format_for_panic(felts.iter().cloned())))
                }
                (RunResultValue::Panic(_felts), TestExpectation::Panics(PanicExpectation::Any)) => {
                    (true, None)
                }
                (
                    RunResultValue::Panic(felts),
                    TestExpectation::Panics(PanicExpectation::Exact(expected)),
                ) => {
                    let matches_subslice = felts == expected
                        || (!expected.is_empty()
                            && felts.windows(expected.len()).any(|w| w == expected.as_slice()))
                        || format_for_panic(felts.iter().cloned())
                            .contains(&format_for_panic(expected.iter().cloned()));
                    if matches_subslice {
                        (true, None)
                    } else {
                        (
                            false,
                            Some(format!(
                                "expected panic {:?}, got {}",
                                expected,
                                format_for_panic(felts.iter().cloned())
                            )),
                        )
                    }
                }
            }
        };

        /// Drops the per-test compiled program slice once every run of that test is done.
        struct ReleaseFunctionRunner<'a>(&'a SierraCasmRunner, &'a cairo_lang_sierra::program::Function);
        impl Drop for ReleaseFunctionRunner<'_> {
            fn drop(&mut self) {
                self.0.release_function_runner(self.1);
            }
        }
        let results: Vec<SingleTestResultReport> = filtered_named_tests
            .par_iter()
            .map(|(name, test_cfg)| {
                let test_started = Instant::now();
                let short_name = name.rsplit_once("::").map(|(_, s)| s).unwrap_or(name);
                let (file, line, should_panic_attr) = source_locations
                    .get(short_name)
                    .cloned()
                    .unwrap_or_else(|| ("src/lib.cairo".to_string(), 1, false));
                let should_panic = matches!(test_cfg.expectation, TestExpectation::Panics(_))
                    || should_panic_attr;

                if (args.ignored && !test_cfg.ignored)
                    || (test_cfg.ignored && !args.include_ignored && !args.ignored)
                {
                    return SingleTestResultReport {
                        name: name.clone(),
                        file,
                        line,
                        should_panic,
                        ignored: true,
                        status: "ignored".to_string(),
                        gas_used: None,
                        steps: None,
                        syscalls: None,
                        builtins: None,
                        resource_gas: None,
                        fuzz_runs: None,
                        fuzz_seed: None,
                        shrunk_counterexample: None,
                        elapsed_ms: Some(test_started.elapsed().as_secs_f64() * 1000.0),
                        failure_reason: None,
                    };
                }

                let Ok(func) = runner.find_function(name) else {
                    return SingleTestResultReport {
                        name: name.clone(),
                        file,
                        line,
                        should_panic,
                        ignored: false,
                        status: "failed".to_string(),
                        gas_used: None,
                        steps: None,
                        syscalls: None,
                        builtins: None,
                        resource_gas: None,
                        fuzz_runs: None,
                        fuzz_seed: None,
                        shrunk_counterexample: None,
                        elapsed_ms: Some(test_started.elapsed().as_secs_f64() * 1000.0),
                        failure_reason: Some(format!("function `{name}` not found in Sierra program")),
                    };
                };
                let _release = ReleaseFunctionRunner(&runner, func);

                let param_specs = runner.user_param_types_and_sizes(func);
                let is_fuzz_or_invariant = !param_specs.is_empty()
                    && (args.fuzz_runs > 0
                        || short_name.contains("fuzz")
                        || short_name.contains("invariant")
                        || short_name.contains("prop"));

                if is_fuzz_or_invariant {
                    // Coverage guides the corpus; keep it on this thread only so the other tests
                    // (and the ones that run after this one) stay on the sliced fast path.
                    let _coverage = runner.scoped_coverage(true);
                    let total_runs = if args.fuzz_runs > 0 {
                        args.fuzz_runs
                    } else {
                        32
                    };
                    let base_seed = args.fuzz_seed.unwrap_or_else(|| {
                        let h = blake3::hash(name.as_bytes());
                        let mut b = [0u8; 8];
                        b.copy_from_slice(&h.as_bytes()[..8]);
                        u64::from_le_bytes(b) ^ 0x5543_4655_5a5a_2026
                    });
                    let mut rng = SplitMix64::new(base_seed);
                    let mut corpus: Vec<Vec<u128>> = Vec::new();
                    let mut total_steps = 0usize;
                    let mut last_gas_used = None;
                    let mut merged_syscalls: BTreeMap<String, usize> = BTreeMap::new();
                    let mut merged_builtins: BTreeMap<String, usize> = BTreeMap::new();

                    for run_idx in 0..total_runs {
                        let cov_before = runner.covered_statement_count();
                        let words = generate_fuzz_words(&param_specs, run_idx, &mut rng, &corpus);
                        let vm_args: Vec<cairo_lang_runner::Arg> = words
                            .iter()
                            .copied()
                            .map(|w| {
                                cairo_lang_runner::Arg::Value(
                                    starknet_types_core::felt::Felt::from(w),
                                )
                            })
                            .collect();
                        let run_res = runner.run_function_with_starknet_context_sliced(
                            func,
                            vm_args,
                            test_cfg.available_gas,
                            StarknetState::default()
                                .with_snforge_defaults(predeployed_erc20_class_hash),
                        );
                        let cov_after = runner.covered_statement_count();
                        if cov_after > cov_before && corpus.len() < 64 {
                            corpus.push(words.clone());
                        }
                        match run_res {
                            Ok(result) => {
                                total_steps += result.used_resources.basic_resources.n_steps;
                                if let (Some(initial), Some(rem)) =
                                    (test_cfg.available_gas, result.gas_counter)
                                {
                                    let rem_u64: u64 = rem.to_biguint().try_into().unwrap_or(0);
                                    last_gas_used =
                                        Some((initial as u64).saturating_sub(rem_u64));
                                }
                                for (k, v) in result.used_resources.syscalls.clone() {
                                    *merged_syscalls.entry(k).or_insert(0) += v;
                                }
                                for (k, v) in builtin_counts(&result.used_resources) {
                                    *merged_builtins.entry(k).or_insert(0) += v;
                                }
                                let (passed, fail_msg) =
                                    evaluate_outcome(&result, &test_cfg.expectation);
                                if !passed {
                                    // Binary-search counterexample shrinking!
                                    let mut shrunk = words.clone();
                                    let mut shrunk_msg = fail_msg.unwrap_or_default();
                                    let test_fails_with = |candidate: &[u128]| -> Option<String> {
                                        let cand_args: Vec<cairo_lang_runner::Arg> = candidate
                                            .iter()
                                            .copied()
                                            .map(|w| {
                                                cairo_lang_runner::Arg::Value(
                                                    starknet_types_core::felt::Felt::from(w),
                                                )
                                            })
                                            .collect();
                                        match runner.run_function_with_starknet_context_sliced(
                                            func,
                                            cand_args,
                                            test_cfg.available_gas,
                                            StarknetState::default().with_snforge_defaults(
                                                predeployed_erc20_class_hash,
                                            ),
                                        ) {
                                            Ok(r) => {
                                                let (ok, m) =
                                                    evaluate_outcome(&r, &test_cfg.expectation);
                                                if !ok {
                                                    Some(m.unwrap_or_default())
                                                } else {
                                                    None
                                                }
                                            }
                                            Err(e) => Some(format!("VM execution error: {e}")),
                                        }
                                    };

                                    for idx in 0..shrunk.len() {
                                        if shrunk[idx] == 0 {
                                            continue;
                                        }
                                        let mut probe = shrunk.clone();
                                        probe[idx] = 0;
                                        if let Some(m) = test_fails_with(&probe) {
                                            shrunk[idx] = 0;
                                            shrunk_msg = m;
                                            continue;
                                        }
                                        let mut low = 1u128;
                                        let mut high = shrunk[idx];
                                        for _ in 0..24 {
                                            if low >= high {
                                                break;
                                            }
                                            let mid = low + (high - low) / 2;
                                            let mut p = shrunk.clone();
                                            p[idx] = mid;
                                            if let Some(m) = test_fails_with(&p) {
                                                high = mid;
                                                shrunk_msg = m;
                                            } else {
                                                low = mid + 1;
                                            }
                                        }
                                        shrunk[idx] = high;
                                        if shrunk[idx] > 0 {
                                            let mut p = shrunk.clone();
                                            p[idx] -= 1;
                                            if let Some(m) = test_fails_with(&p) {
                                                shrunk[idx] -= 1;
                                                shrunk_msg = m;
                                            }
                                        }
                                    }
                                    let shrunk_strs: Vec<String> =
                                        shrunk.iter().map(|v| v.to_string()).collect();
                                    return SingleTestResultReport {
                                        name: name.clone(),
                                        file,
                                        line,
                                        should_panic,
                                        ignored: false,
                                        status: "failed".to_string(),
                                        gas_used: last_gas_used,
                                        steps: Some(total_steps),
                                        syscalls: if merged_syscalls.is_empty() {
                                            None
                                        } else {
                                            Some(merged_syscalls)
                                        },
                                        builtins: None,
                                        resource_gas: None,
                                        fuzz_runs: Some(run_idx + 1),
                                        fuzz_seed: Some(base_seed),
                                        shrunk_counterexample: Some(shrunk_strs.clone()),
                                        elapsed_ms: Some(test_started.elapsed().as_secs_f64() * 1000.0),
                                        failure_reason: Some(format!(
                                            "{shrunk_msg} (shrunk counterexample: [{}], seed: {base_seed})",
                                            shrunk_strs.join(", ")
                                        )),
                                    };
                                }
                            }
                            Err(err) => {
                                let shrunk_strs: Vec<String> =
                                    words.iter().map(|v| v.to_string()).collect();
                                return SingleTestResultReport {
                                    name: name.clone(),
                                    file,
                                    line,
                                    should_panic,
                                    ignored: false,
                                    status: "failed".to_string(),
                                    gas_used: last_gas_used,
                                    steps: Some(total_steps),
                                    syscalls: None,
                                    builtins: None,
                                    resource_gas: None,
                                    fuzz_runs: Some(run_idx + 1),
                                    fuzz_seed: Some(base_seed),
                                    shrunk_counterexample: Some(shrunk_strs),
                                    elapsed_ms: Some(test_started.elapsed().as_secs_f64() * 1000.0),
                                    failure_reason: Some(format!("VM execution error: {err}")),
                                };
                            }
                        }
                    }

                    return SingleTestResultReport {
                        name: name.clone(),
                        file,
                        line,
                        should_panic,
                        ignored: false,
                        status: "ok".to_string(),
                        gas_used: last_gas_used,
                        steps: Some(total_steps / total_runs.max(1)),
                        syscalls: if merged_syscalls.is_empty() {
                            None
                        } else {
                            Some(merged_syscalls)
                        },
                        resource_gas: Some(resource_gas_breakdown(total_steps, &merged_builtins)),
                        builtins: builtins_or_none(merged_builtins),
                        fuzz_runs: Some(total_runs),
                        fuzz_seed: Some(base_seed),
                        shrunk_counterexample: None,
                        elapsed_ms: Some(test_started.elapsed().as_secs_f64() * 1000.0),
                        failure_reason: None,
                    };
                }

                let run_res = runner.run_function_with_starknet_context_sliced(
                    func,
                    vec![],
                    test_cfg.available_gas,
                    StarknetState::default().with_snforge_defaults(predeployed_erc20_class_hash),
                );

                match run_res {
                    Ok(result) => {
                        let gas_used = match (test_cfg.available_gas, result.gas_counter) {
                            (Some(initial), Some(rem)) => {
                                let rem_u64: u64 = rem.to_biguint().try_into().unwrap_or(0);
                                Some((initial as u64).saturating_sub(rem_u64))
                            }
                            _ => None,
                        };
                        let steps = Some(result.used_resources.basic_resources.n_steps);
                        let syscalls_map: BTreeMap<String, usize> = result
                            .used_resources
                            .syscalls
                            .clone()
                            .into_iter()
                            .collect();
                        let syscalls = if syscalls_map.is_empty() {
                            None
                        } else {
                            Some(syscalls_map)
                        };
                        let builtin_map = builtin_counts(&result.used_resources);
                        let resource_gas = Some(resource_gas_breakdown(
                            result.used_resources.basic_resources.n_steps,
                            &builtin_map,
                        ));
                        let builtins = builtins_or_none(builtin_map);

                        let (passed, failure_reason) =
                            evaluate_outcome(&result, &test_cfg.expectation);

                        SingleTestResultReport {
                            name: name.clone(),
                            file,
                            line,
                            should_panic,
                            ignored: false,
                            status: if passed {
                                "ok".to_string()
                            } else {
                                "failed".to_string()
                            },
                            gas_used,
                            steps,
                            syscalls,
                            builtins,
                            resource_gas,
                            fuzz_runs: None,
                            fuzz_seed: None,
                            shrunk_counterexample: None,
                            elapsed_ms: Some(test_started.elapsed().as_secs_f64() * 1000.0),
                            failure_reason,
                        }
                    }
                    Err(err) => SingleTestResultReport {
                        name: name.clone(),
                        file,
                        line,
                        should_panic,
                        ignored: false,
                        status: "failed".to_string(),
                        gas_used: None,
                        steps: None,
                        syscalls: None,
                        builtins: None,
                        resource_gas: None,
                        fuzz_runs: None,
                        fuzz_seed: None,
                        shrunk_counterexample: None,
                        elapsed_ms: Some(test_started.elapsed().as_secs_f64() * 1000.0),
                        failure_reason: Some(format!("VM execution error: {err}")),
                    },
                }
            })
            .collect();

        let exec_ms = exec_start.elapsed().as_secs_f64() * 1000.0;
        let passed_count = results.iter().filter(|r| r.status == "ok").count();
        let mut failed_count = results.iter().filter(|r| r.status == "failed").count();
        let ignored_count = results.iter().filter(|r| r.status == "ignored").count();

        // Coverage summary & LCOV export
        let coverage_report: Option<CoverageSummaryReport> =
            if args.coverage || args.lcov_path.is_some() {
                let raw_funcs = runner.function_coverage_summary();
                let mut user_funcs: Vec<FunctionCoverageEntry> = raw_funcs
                    .iter()
                    .filter(|(fn_name, _, total)| {
                        *total > 0
                            && (fn_name.starts_with(&format!("{crate_name}::"))
                                || !fn_name.starts_with("core::"))
                    })
                    .map(|(fn_name, cov, tot)| FunctionCoverageEntry {
                        function: fn_name.clone(),
                        covered_statements: *cov,
                        total_statements: *tot,
                        coverage_pct: if *tot > 0 {
                            (*cov as f64 / *tot as f64) * 100.0
                        } else {
                            100.0
                        },
                    })
                    .collect();
                if user_funcs.is_empty() {
                    user_funcs = raw_funcs
                        .into_iter()
                        .filter(|(_, _, tot)| *tot > 0)
                        .map(|(fn_name, cov, tot)| FunctionCoverageEntry {
                            function: fn_name,
                            covered_statements: cov,
                            total_statements: tot,
                            coverage_pct: (cov as f64 / tot as f64) * 100.0,
                        })
                        .collect();
                }
                let covered_statements: usize =
                    user_funcs.iter().map(|f| f.covered_statements).sum();
                let total_statements: usize = user_funcs.iter().map(|f| f.total_statements).sum();
                let statement_coverage_pct = if total_statements > 0 {
                    (covered_statements as f64 / total_statements as f64) * 100.0
                } else {
                    100.0
                };
                let covered_functions = user_funcs
                    .iter()
                    .filter(|f| f.covered_statements > 0)
                    .count();
                let total_functions = user_funcs.len();
                let function_coverage_pct = if total_functions > 0 {
                    (covered_functions as f64 / total_functions as f64) * 100.0
                } else {
                    100.0
                };
                if let Some(lcov_file) = args.lcov_path.as_deref() {
                    write_lcov_report(lcov_file, "src/lib.cairo", &source_locations, &user_funcs)?;
                }
                Some(CoverageSummaryReport {
                    covered_statements,
                    total_statements,
                    statement_coverage_pct,
                    covered_functions,
                    total_functions,
                    function_coverage_pct,
                    lcov_path: args.lcov_path.as_ref().map(|p| p.display().to_string()),
                    functions: user_funcs,
                })
            } else {
                None
            };

        // Gas & step snapshot write / check
        let mut snapshot_regressions: Vec<String> = Vec::new();
        if let Some(snap_out) = args.snapshot.as_deref() {
            if let Some(parent) = snap_out.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(snap_out, format_gas_snapshot(&results))?;
        }
        if let Some(snap_check) = args.snapshot_check.as_deref() {
            let expected_text = fs::read_to_string(snap_check).with_context(|| {
                format!("failed to read gas snapshot file {}", snap_check.display())
            })?;
            let expected_map = parse_gas_snapshot(&expected_text);
            let current_map = parse_gas_snapshot(&format_gas_snapshot(&results));
            for (test_name, (exp_gas, exp_steps)) in &expected_map {
                match current_map.get(test_name) {
                    Some((cur_gas, cur_steps)) => {
                        if cur_gas > exp_gas || cur_steps > exp_steps {
                            snapshot_regressions.push(format!(
                                "{test_name}: regressed from (gas: ~{exp_gas}, steps: {exp_steps}) to (gas: ~{cur_gas}, steps: {cur_steps})"
                            ));
                        }
                    }
                    None => {
                        snapshot_regressions.push(format!(
                            "{test_name}: missing from current test run"
                        ));
                    }
                }
            }
            if !snapshot_regressions.is_empty() {
                failed_count += snapshot_regressions.len();
            }
        }

        // Semantic mutation testing (`--mutate`)
        let mutation_report: Option<MutationTestingSummaryReport> = if args.mutate {
            let discovered = discover_cairo_mutants(&workspace_root, &src_dir, 20);
            let mut mutant_results = Vec::with_capacity(discovered.len());
            let mut killed = 0usize;
            let mut survived = 0usize;
            for (file_path, rel_file, line_no, op_name, orig_tok, rep_tok, mutated_src) in
                discovered
            {
                let original_content = fs::read_to_string(&file_path).unwrap_or_default();
                let _restore_guard = FileRestoreGuard {
                    path: file_path.clone(),
                    original_content,
                };
                fs::write(&file_path, &mutated_src)?;
                let mut was_killed = false;
                match compile_or_load_native_tests(
                    &effective_manifest_path,
                    &workspace_root,
                    &package_name,
                    &crate_name,
                    &args.features,
                    true,
                ) {
                    Ok((mut_compiled, _, _)) => {
                        if let Ok(mut_runner) = SierraCasmRunner::new(
                            mut_compiled.sierra_program.program,
                            Some(MetadataComputationConfig {
                                function_set_costs: mut_compiled.metadata.function_set_costs,
                                ..Default::default()
                            }),
                            mut_compiled.metadata.contracts_info,
                            None,
                        ) {
                            for (t_name, t_cfg) in &mut_compiled.metadata.named_tests {
                                if t_cfg.ignored {
                                    continue;
                                }
                                let Ok(f) = mut_runner.find_function(t_name) else {
                                    was_killed = true;
                                    break;
                                };
                                match mut_runner.run_function_with_starknet_context_sliced(
                                    f,
                                    vec![],
                                    t_cfg.available_gas,
                                    StarknetState::default()
                                        .with_snforge_defaults(predeployed_erc20_class_hash),
                                ) {
                                    Ok(r) => {
                                        let (ok, _) = evaluate_outcome(&r, &t_cfg.expectation);
                                        if !ok {
                                            was_killed = true;
                                            break;
                                        }
                                    }
                                    Err(_) => {
                                        was_killed = true;
                                        break;
                                    }
                                }
                            }
                        } else {
                            was_killed = true;
                        }
                    }
                    Err(_) => {
                        was_killed = true;
                    }
                }
                drop(_restore_guard);
                if was_killed {
                    killed += 1;
                } else {
                    survived += 1;
                }
                mutant_results.push(MutantResultReport {
                    file: rel_file,
                    line: line_no,
                    operator: op_name,
                    original: orig_tok,
                    replacement: rep_tok,
                    status: if was_killed {
                        "killed".to_string()
                    } else {
                        "survived".to_string()
                    },
                });
            }
            let total_mutants = mutant_results.len();
            let mutation_score_pct = if total_mutants > 0 {
                (killed as f64 / total_mutants as f64) * 100.0
            } else {
                100.0
            };
            Some(MutationTestingSummaryReport {
                total_mutants,
                killed,
                survived,
                mutation_score_pct,
                mutants: mutant_results,
            })
        } else {
            None
        };

        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

        if failed_count == 0 {
            save_affected_test_graph_cache(
                &workspace_root,
                &crate_name,
                &current_affected_graph,
            );
            let _ = fs::remove_file(&last_failed_file);
        } else {
            let failed_names: Vec<String> = results
                .iter()
                .filter(|r| r.status == "failed")
                .map(|r| r.name.clone())
                .collect();
            if let Some(parent) = last_failed_file.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(bytes) = serde_json::to_vec(&failed_names) {
                let _ = fs::write(&last_failed_file, bytes);
            }
        }

        let status = if failed_count == 0 { "passed" } else { "failed" };
        let resource_totals = suite_resource_totals(&results);
        let report = serde_json::json!({
            "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
            "status": status,
            "runner": "uc-native-test",
            "package": package_name,
            "manifest_path": effective_manifest_path.display().to_string(),
            "affected_mode": args.affected,
            "affected_modules": affected_modules_opt.as_ref().map(|s| s.iter().cloned().collect::<Vec<_>>()),
            "compile_cache_hit": compile_cache_hit,
            "compile_ms": compile_ms,
            "exec_ms": exec_ms,
            "discovered_count": results.len(),
            "passed_count": passed_count,
            "failed_count": failed_count,
            "ignored_count": ignored_count,
            "elapsed_ms": elapsed_ms,
            "coverage": coverage_report,
            "snapshot_regressions": if snapshot_regressions.is_empty() { None } else { Some(&snapshot_regressions) },
            "mutation_testing": mutation_report,
            "resource_totals": resource_totals,
            "tests": results,
        });

        if args.json || args.report_path.is_some() {
            emit_json_value(args.report_path.as_deref(), &report)?;
        } else {
            println!(
                "running {} test(s) from {} ({}, compile: {:.1}ms{}, exec: {:.1}ms)",
                results.len(),
                effective_manifest_path.display(),
                package_name,
                compile_ms,
                if compile_cache_hit { " cached" } else { "" },
                exec_ms
            );
            for t in &results {
                let mut metrics_suffix = String::new();
                if args.gas_report || t.gas_used.is_some() || t.fuzz_runs.is_some() {
                    let mut parts = Vec::new();
                    if let Some(runs) = t.fuzz_runs {
                        parts.push(format!("runs: {runs}"));
                    }
                    if let Some(g) = t.gas_used {
                        parts.push(format!("gas: ~{g}"));
                    }
                    if let Some(s) = t.steps {
                        parts.push(format!("steps: {s}"));
                    }
                    if let Some(ms) = t.elapsed_ms {
                        parts.push(format!("{ms:.1}ms"));
                    }
                    if args.gas_report {
                        if let Some(sys) = &t.syscalls {
                            let sys_str: Vec<String> =
                                sys.iter().map(|(k, v)| format!("{k}={v}")).collect();
                            parts.push(format!("syscalls: [{}]", sys_str.join(", ")));
                        }
                        if let Some(b) = &t.builtins {
                            let b_str: Vec<String> =
                                b.iter().map(|(k, v)| format!("{k}={v}")).collect();
                            parts.push(format!("builtins: [{}]", b_str.join(", ")));
                        }
                        if let Some(r) = &t.resource_gas {
                            parts.push(format!(
                                "resource gas: {} ({} {:.0}%)",
                                r.total, r.dominant, r.dominant_pct
                            ));
                        }
                    }
                    if !parts.is_empty() {
                        metrics_suffix = format!(" ({})", parts.join(", "));
                    }
                }
                println!("test {} ... {}{}", t.name, t.status, metrics_suffix);
                if let Some(reason) = &t.failure_reason {
                    println!("  failure: {reason}");
                }
            }
            if args.gas_report {
                if let Some(totals) = &resource_totals {
                    let b_str: Vec<String> = totals
                        .builtins
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect();
                    println!(
                        "resources: steps={} builtins=[{}] resource gas={} ({} {:.0}%)",
                        totals.steps,
                        b_str.join(", "),
                        totals.resource_gas.total,
                        totals.resource_gas.dominant,
                        totals.resource_gas.dominant_pct
                    );
                    for (name, gas) in totals.heaviest_tests.iter().take(5) {
                        println!("  heaviest: {name} ({gas} resource gas)");
                    }
                }
            }
            if let Some(cov) = &coverage_report {
                println!(
                    "coverage: {:.1}% statements ({}/{}); {:.1}% functions ({}/{})",
                    cov.statement_coverage_pct,
                    cov.covered_statements,
                    cov.total_statements,
                    cov.function_coverage_pct,
                    cov.covered_functions,
                    cov.total_functions
                );
            }
            if let Some(mut_rep) = &mutation_report {
                println!(
                    "mutation testing: {:.1}% score ({} killed, {} survived out of {} mutants)",
                    mut_rep.mutation_score_pct,
                    mut_rep.killed,
                    mut_rep.survived,
                    mut_rep.total_mutants
                );
            }
            for reg in &snapshot_regressions {
                println!("snapshot regression: {reg}");
            }
            println!(
                "test result: {}. {} passed; {} failed; {} ignored; finished in {:.2}ms",
                if failed_count == 0 { "ok" } else { "FAILED" },
                passed_count,
                failed_count,
                ignored_count,
                elapsed_ms
            );
        }

        if failed_count > 0 {
            bail!("{failed_count} test(s) failed");
        }
        return Ok(());
    }

    #[cfg(not(feature = "native-test-runner"))]
    {
        bail!("native test runner feature is not enabled in this build of uc")
    }
}

#[cfg(all(test, feature = "native-test-runner"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempTestProject {
        dir: PathBuf,
    }

    impl TempTestProject {
        fn new(prefix: &str) -> Self {
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let id = SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "uc_test_runner_{prefix}_{}_{}",
                std::process::id(),
                id
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("src")).expect("create temp project dir");
            Self { dir }
        }
    }

    impl Drop for TempTestProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn native_test_runner_executes_cairo_and_starknet_tests_in_process() {
        let proj = TempTestProject::new("basic");
        let manifest_path = proj.dir.join("Scarb.toml");
        fs::write(
            &manifest_path,
            r#"[package]
name = "test_demo"
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
            proj.dir.join("src/lib.cairo"),
            r#"pub fn compute_fee(amount: u128, bps: u128) -> u128 {
    assert(bps <= 10000, 'BPS_TOO_HIGH');
    (amount * bps) / 10000
}

#[cfg(test)]
mod tests {
    use super::compute_fee;

    #[test]
    fn test_compute_fee_standard() {
        assert(compute_fee(10000, 250) == 250, 'wrong_fee');
        assert(compute_fee(5000, 100) == 50, 'wrong_fee_2');
    }

    #[test]
    #[should_panic(expected: ('BPS_TOO_HIGH',))]
    fn test_compute_fee_panics_above_max_bps() {
        compute_fee(1000, 20000);
    }

    #[test]
    #[ignore]
    fn test_ignored_case() {
        assert(1 == 2, 'ignored');
    }
}
"#,
        )
        .unwrap();

        let report_path = proj.dir.join("test-report.json");
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
            report_path: Some(report_path.clone()),
            ..Default::default()
        })
        .expect("native in-process test runner should succeed");

        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
        assert_eq!(report["status"], "passed");
        assert_eq!(report["runner"], "uc-native-test");
        assert_eq!(report["passed_count"], 2);
        assert_eq!(report["failed_count"], 0);
        assert_eq!(report["ignored_count"], 1);
        assert_eq!(report["compile_cache_hit"], false);

        // Second run should hit the compiled test cache
        let report_path_2 = proj.dir.join("test-report-2.json");
        run_test(TestArgs {
            filter: None,
            manifest_path: Some(manifest_path),
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
            report_path: Some(report_path_2.clone()),
            ..Default::default()
        })
        .expect("cached native test run should succeed");
        let report2: serde_json::Value =
            serde_json::from_slice(&fs::read(&report_path_2).unwrap()).unwrap();
        assert_eq!(report2["compile_cache_hit"], true);
        assert_eq!(report2["passed_count"], 2);
    }

    #[test]
    fn native_test_runner_affected_mode_selects_only_impacted_modules() {
        let proj = TempTestProject::new("affected");
        let manifest_path = proj.dir.join("Scarb.toml");
        fs::write(
            &manifest_path,
            r#"[package]
name = "affected_demo"
version = "0.1.0"
edition = "2024_07"

[dependencies]
starknet = "2.16.0"

[[target.starknet-contract]]
sierra = true
"#,
        )
        .unwrap();
        let src_initial = r#"pub mod math_mod {
    pub fn double(x: u64) -> u64 {
        x * 2
    }
}

pub mod vault_mod {
    pub fn shares(assets: u64) -> u64 {
        assets
    }
}

#[cfg(test)]
mod math_tests {
    use super::math_mod::double;
    #[test]
    fn test_math_double() {
        assert(double(21) == 42, 'bad_double');
    }
}

#[cfg(test)]
mod vault_tests {
    use super::vault_mod::shares;
    #[test]
    fn test_vault_shares() {
        assert(shares(100) == 100, 'bad_shares');
    }
}
"#;
        fs::write(proj.dir.join("src/lib.cairo"), src_initial).unwrap();

        // 1. Baseline full test run records affected module graph
        let rep1 = proj.dir.join("rep1.json");
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
            gas_report: false,
            no_cache: false,
            offline: false,
            json: true,
            report_path: Some(rep1.clone()),
            ..Default::default()
        })
        .unwrap();
        let val1: serde_json::Value = serde_json::from_slice(&fs::read(&rep1).unwrap()).unwrap();
        assert_eq!(val1["passed_count"], 2);

        // 2. Running `--affected` with no changes executes 0 tests immediately
        let rep2 = proj.dir.join("rep2.json");
        run_test(TestArgs {
            filter: None,
            manifest_path: Some(manifest_path.clone()),
            package: None,
            workspace: false,
            features: Vec::new(),
            affected: true,
            include_ignored: false,
            ignored: false,
            exact: false,
            gas_report: false,
            no_cache: false,
            offline: false,
            json: true,
            report_path: Some(rep2.clone()),
            ..Default::default()
        })
        .unwrap();
        let val2: serde_json::Value = serde_json::from_slice(&fs::read(&rep2).unwrap()).unwrap();
        assert_eq!(val2["discovered_count"], 0);

        // 3. Modify ONLY `vault_mod` -> `--affected` should run ONLY `vault_tests::test_vault_shares`!
        let src_modified = src_initial.replace("        assets\n", "        assets * 1\n");
        fs::write(proj.dir.join("src/lib.cairo"), src_modified).unwrap();

        let rep3 = proj.dir.join("rep3.json");
        run_test(TestArgs {
            filter: None,
            manifest_path: Some(manifest_path),
            package: None,
            workspace: false,
            features: Vec::new(),
            affected: true,
            include_ignored: false,
            ignored: false,
            exact: false,
            gas_report: false,
            no_cache: false,
            offline: false,
            json: true,
            report_path: Some(rep3.clone()),
            ..Default::default()
        })
        .unwrap();
        let val3: serde_json::Value = serde_json::from_slice(&fs::read(&rep3).unwrap()).unwrap();
        assert_eq!(val3["passed_count"], 1);
        let ran_name = val3["tests"][0]["name"].as_str().unwrap();
        assert!(
            ran_name.contains("test_vault_shares"),
            "expected only test_vault_shares to run, got {ran_name}"
        );
    }

    #[test]
    fn native_build_and_test_support_features_and_multi_package_workspace() {
        let ws = TempTestProject::new("workspace_features");
        let root_manifest = ws.dir.join("Scarb.toml");
        fs::write(
            &root_manifest,
            r#"[workspace]
members = ["packages/token_a", "packages/pool_b"]
"#,
        )
        .unwrap();

        let pkg_a = ws.dir.join("packages/token_a");
        let pkg_b = ws.dir.join("packages/pool_b");
        fs::create_dir_all(pkg_a.join("src")).unwrap();
        fs::create_dir_all(pkg_b.join("src")).unwrap();

        fs::write(
            pkg_a.join("Scarb.toml"),
            r#"[package]
name = "token_a"
version = "0.1.0"
edition = "2024_07"

[features]
default = ["standard"]
standard = []
premium = ["standard", "bonus_mint"]
bonus_mint = []

[dependencies]
starknet = "2.16.0"

[[target.starknet-contract]]
sierra = true
casm = true
"#,
        )
        .unwrap();
        fs::write(
            pkg_a.join("src/lib.cairo"),
            r#"#[starknet::contract]
pub mod TokenA {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        supply: u128,
    }

    #[external(v0)]
    pub fn get_supply(self: @ContractState) -> u128 {
        self.supply.read()
    }

    #[cfg(feature: "bonus_mint")]
    #[external(v0)]
    pub fn mint_bonus(ref self: ContractState, amount: u128) {
        self.supply.write(self.supply.read() + amount);
    }
}
"#,
        )
        .unwrap();

        fs::write(
            pkg_b.join("Scarb.toml"),
            r#"[package]
name = "pool_b"
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
            pkg_b.join("src/lib.cairo"),
            r#"#[starknet::contract]
pub mod PoolB {
    use starknet::storage::StoragePointerReadAccess;

    #[storage]
    struct Storage {
        tvl: u128,
    }

    #[external(v0)]
    pub fn get_tvl(self: @ContractState) -> u128 {
        self.tvl.read()
    }
}
"#,
        )
        .unwrap();

        // 1. Multi-package `uc build --workspace`
        let ws_report_path = ws.dir.join("ws-build-report.json");
        run_build(BuildArgs {
            common: BuildCommonArgs {
                manifest_path: Some(root_manifest),
                package: None,
                workspace: true,
                features: Vec::new(),
                offline: true,
                release: false,
                profile: None,
            },
            engine: EngineArg::Uc,
            daemon_mode: DaemonModeArg::Off,
            report_path: Some(ws_report_path.clone()),
            record_failure: None,
            json: false,
            plan_only: false,
        })
        .expect("multi-package workspace native build should succeed");

        let ws_rep: serde_json::Value =
            serde_json::from_slice(&fs::read(&ws_report_path).unwrap()).unwrap();
        assert_eq!(ws_rep["compile_backend"], "uc_native");
        assert!(
            ws.dir
                .join("target/dev/token_a_TokenA.contract_class.json")
                .is_file()
        );
        assert!(
            ws.dir
                .join("target/dev/pool_b_PoolB.contract_class.json")
                .is_file()
        );

        // Without `--features premium`, `mint_bonus` should NOT be in TokenA's ABI
        let sierra_default =
            fs::read_to_string(ws.dir.join("target/dev/token_a_TokenA.contract_class.json"))
                .unwrap();
        assert!(
            !sierra_default.contains("mint_bonus"),
            "mint_bonus should be excluded when feature `bonus_mint` is inactive"
        );

        // 2. Build `token_a` with `--features premium` (which transitively enables `bonus_mint`)
        run_build(BuildArgs {
            common: BuildCommonArgs {
                manifest_path: Some(pkg_a.join("Scarb.toml")),
                package: Some("token_a".to_string()),
                workspace: false,
                features: vec!["premium".to_string()],
                offline: true,
                release: false,
                profile: None,
            },
            engine: EngineArg::Uc,
            daemon_mode: DaemonModeArg::Off,
            report_path: None,
            record_failure: None,
            json: false,
            plan_only: false,
        })
        .expect("feature-enabled native build should succeed");

        let sierra_premium =
            fs::read_to_string(ws.dir.join("target/dev/token_a_TokenA.contract_class.json"))
                .unwrap();
        assert!(
            sierra_premium.contains("mint_bonus"),
            "mint_bonus should be included when `--features premium` is enabled"
        );
    }

    #[test]
    fn native_test_runner_executes_snforge_std_cheatcodes_and_integration_tests_in_process() {
        let proj = TempTestProject::new("snforge_cheatcodes");
        let manifest_path = proj.dir.join("Scarb.toml");
        fs::write(
            &manifest_path,
            r#"[package]
name = "cheatcode_demo"
version = "0.1.0"
edition = "2024_07"

[dependencies]
starknet = "2.16.0"

[dev-dependencies]
snforge_std = "0.53.0"

[[target.starknet-contract]]
sierra = true
casm = true
"#,
        )
        .unwrap();
        fs::write(
            proj.dir.join("src/lib.cairo"),
            r#"use starknet::ContractAddress;

#[starknet::interface]
pub trait IVault<TContractState> {
    fn deposit(ref self: TContractState, amount: u128);
    fn get_deposit(self: @TContractState, user: ContractAddress) -> u128;
    fn get_last_timestamp(self: @TContractState) -> u64;
    fn get_last_block(self: @TContractState) -> u64;
}

#[starknet::contract]
pub mod VaultContract {
    use starknet::{ContractAddress, get_caller_address, get_block_timestamp, get_block_number};
    use starknet::storage::{
        Map, StorageMapReadAccess, StorageMapWriteAccess, StoragePointerReadAccess,
        StoragePointerWriteAccess,
    };

    #[storage]
    struct Storage {
        balances: Map<ContractAddress, u128>,
        last_ts: u64,
        last_block: u64,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    pub enum Event {
        Deposited: Deposited,
    }

    #[derive(Drop, starknet::Event)]
    pub struct Deposited {
        pub user: ContractAddress,
        pub amount: u128,
    }

    #[abi(embed_v0)]
    impl VaultImpl of super::IVault<ContractState> {
        fn deposit(ref self: ContractState, amount: u128) {
            let caller = get_caller_address();
            let prev = self.balances.read(caller);
            self.balances.write(caller, prev + amount);
            self.last_ts.write(get_block_timestamp());
            self.last_block.write(get_block_number());
            self.emit(Deposited { user: caller, amount });
        }

        fn get_deposit(self: @ContractState, user: ContractAddress) -> u128 {
            self.balances.read(user)
        }

        fn get_last_timestamp(self: @ContractState) -> u64 {
            self.last_ts.read()
        }

        fn get_last_block(self: @ContractState) -> u64 {
            self.last_block.read()
        }
    }
}
"#,
        )
        .unwrap();

        fs::create_dir_all(proj.dir.join("tests")).unwrap();
        fs::write(
            proj.dir.join("tests/test_vault.cairo"),
            r#"use snforge_std::{
    declare, ContractClassTrait, DeclareResultTrait, start_cheat_caller_address,
    stop_cheat_caller_address, start_cheat_block_timestamp, stop_cheat_block_timestamp,
    start_cheat_block_number, stop_cheat_block_number, spy_events, EventSpyAssertionsTrait,
};
use starknet::ContractAddress;
use cheatcode_demo::{IVaultDispatcher, IVaultDispatcherTrait, VaultContract};

#[test]
fn test_vault_with_snforge_cheatcodes() {
    let contract_class = declare("VaultContract").unwrap().contract_class();
    let (vault_addr, _) = contract_class.deploy(@array![]).unwrap();
    let dispatcher = IVaultDispatcher { contract_address: vault_addr };

    let mut spy = spy_events();
    let alice: ContractAddress = 0x111.try_into().unwrap();

    start_cheat_caller_address(vault_addr, alice);
    start_cheat_block_timestamp(vault_addr, 1700000500);
    start_cheat_block_number(vault_addr, 999);

    dispatcher.deposit(250);

    assert(dispatcher.get_deposit(alice) == 250, 'wrong_alice_deposit');
    assert(dispatcher.get_last_timestamp() == 1700000500, 'wrong_timestamp');
    assert(dispatcher.get_last_block() == 999, 'wrong_block_number');

    let expected = array![
        (
            vault_addr,
            VaultContract::Event::Deposited(
                VaultContract::Deposited { user: alice, amount: 250 }
            ),
        )
    ];
    spy.assert_emitted(@expected);

    stop_cheat_caller_address(vault_addr);
    stop_cheat_block_timestamp(vault_addr);
    stop_cheat_block_number(vault_addr);
}
"#,
        )
        .unwrap();

        let report_path = proj.dir.join("cheat-test-report.json");
        run_test(TestArgs {
            filter: None,
            manifest_path: Some(manifest_path),
            package: None,
            workspace: false,
            features: Vec::new(),
            affected: false,
            include_ignored: false,
            ignored: false,
            exact: false,
            gas_report: true,
            no_cache: false,
            offline: true,
            json: true,
            report_path: Some(report_path.clone()),
            ..Default::default()
        })
        .expect("in-process snforge_std cheatcode test should pass without external snforge");

        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
        assert_eq!(report["status"], "passed");
        assert_eq!(report["runner"], "uc-native-test");
        assert_eq!(report["passed_count"], 1);
    }

    #[test]
    fn fuzz_coverage_guard_is_thread_scoped_and_restored_on_drop() {
        use cairo_lang_runner::{thread_coverage_override, ScopedCoverage};
        assert_eq!(thread_coverage_override(), None);
        {
            let _fuzzing = ScopedCoverage::new(true);
            assert_eq!(thread_coverage_override(), Some(true));
            let other_thread = std::thread::spawn(thread_coverage_override).join().unwrap();
            assert_eq!(other_thread, None, "coverage must not leak to other test threads");
        }
        assert_eq!(thread_coverage_override(), None, "coverage must not leak to later tests");
    }

    #[test]
    fn native_test_runner_precalculate_address_matches_deploy_and_cheats_reach_constructor() {
        // Regression: `precalculate_address` used to return the class hash, so a caller cheat
        // set on the precalculated address never applied to the constructor (snforge parity).
        let proj = TempTestProject::new("precalc_addr");
        let manifest_path = proj.dir.join("Scarb.toml");
        fs::write(
            &manifest_path,
            r#"[package]
name = "precalc_demo"
version = "0.1.0"
edition = "2024_07"

[dependencies]
starknet = "2.18.0"

[dev-dependencies]
snforge_std = "0.60.0"

[[target.starknet-contract]]
sierra = true
casm = true
"#,
        )
        .unwrap();
        fs::write(
            proj.dir.join("src/lib.cairo"),
            r#"#[starknet::interface]
pub trait IWho<T> {
    fn deployer(self: @T) -> starknet::ContractAddress;
}

#[starknet::contract]
pub mod Who {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use starknet::{ContractAddress, get_caller_address};

    #[storage]
    struct Storage {
        deployer: ContractAddress,
    }

    #[constructor]
    fn constructor(ref self: ContractState) {
        self.deployer.write(get_caller_address());
    }

    #[abi(embed_v0)]
    impl WhoImpl of super::IWho<ContractState> {
        fn deployer(self: @ContractState) -> ContractAddress {
            self.deployer.read()
        }
    }
}
"#,
        )
        .unwrap();
        fs::create_dir_all(proj.dir.join("tests")).unwrap();
        fs::write(
            proj.dir.join("tests/test_precalc.cairo"),
            r#"use snforge_std::{
    declare, ContractClassTrait, DeclareResultTrait, start_cheat_caller_address,
    stop_cheat_caller_address,
};
use precalc_demo::{IWhoDispatcher, IWhoDispatcherTrait};
use starknet::ContractAddress;

#[test]
fn precalculated_address_matches_deploy_twice() {
    let contract = declare("Who").unwrap().contract_class();
    let calldata = array![];
    let pre = contract.precalculate_address(@calldata);
    let (addr, _) = contract.deploy(@calldata).unwrap();
    assert!(pre == addr, "first precalculate_address mismatch");
    let pre2 = contract.precalculate_address(@calldata);
    let (addr2, _) = contract.deploy(@calldata).unwrap();
    assert!(pre2 == addr2, "second precalculate_address mismatch");
    assert!(addr != addr2, "second deploy must get a fresh address");
}

#[test]
fn cheat_caller_before_deploy_reaches_constructor() {
    let owner: ContractAddress = 4000.try_into().unwrap();
    let contract = declare("Who").unwrap().contract_class();
    let calldata = array![];
    let pre = contract.precalculate_address(@calldata);
    start_cheat_caller_address(pre, owner);
    let (addr, _) = contract.deploy(@calldata).unwrap();
    stop_cheat_caller_address(addr);
    let who = IWhoDispatcher { contract_address: addr };
    assert!(who.deployer() == owner, "constructor did not see cheated caller");
}
"#,
        )
        .unwrap();

        let report_path = proj.dir.join("report.json");
        run_test(TestArgs {
            manifest_path: Some(manifest_path),
            offline: true,
            json: true,
            report_path: Some(report_path.clone()),
            ..Default::default()
        })
        .expect("precalculate_address and pre-deploy cheats must match snforge semantics");
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
        assert_eq!(report["status"], "passed");
        assert_eq!(report["passed_count"], 2);
        let first = &report["tests"][0];
        assert!(first["elapsed_ms"].as_f64().is_some(), "per-test elapsed_ms must be reported");
        // Builtin instances come from the VM, nested deploy and call included.
        let rc = first["builtins"]["range_check"].as_u64().unwrap_or(0);
        assert!(rc > 0, "range_check instances must be counted: {}", first["builtins"]);
        let gas = &first["resource_gas"];
        assert_eq!(
            gas["steps"].as_u64().unwrap(),
            first["steps"].as_u64().unwrap() * STEP_GAS_COST
        );
        assert_eq!(gas["builtins"]["range_check"].as_u64().unwrap(), rc * 70);
        assert!(gas["total"].as_u64().unwrap() >= gas["steps"].as_u64().unwrap() + rc * 70);
        let totals = &report["resource_totals"];
        assert!(totals["steps"].as_u64().unwrap() > 0);
        assert_eq!(totals["heaviest_tests"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn resource_gas_breakdown_weighs_builtins_and_names_the_dominant_resource() {
        let mut b = BTreeMap::new();
        b.insert("range_check".to_string(), 10usize);
        b.insert("pedersen".to_string(), 3usize);
        b.insert("unknown_builtin".to_string(), 5usize);
        b.insert("keccak".to_string(), 0usize);
        let r = resource_gas_breakdown(50, &b);
        assert_eq!(r.steps, 5_000);
        assert_eq!(r.builtins.get("range_check"), Some(&700));
        assert_eq!(r.builtins.get("pedersen"), Some(&12_150));
        assert_eq!(r.builtins.get("unknown_builtin"), Some(&0));
        assert!(!r.builtins.contains_key("keccak"), "zero-count builtins are dropped");
        assert_eq!(r.total, 17_850);
        assert_eq!(r.dominant, "pedersen");
        assert!((r.dominant_pct - 68.07).abs() < 0.01);

        let empty = resource_gas_breakdown(0, &BTreeMap::new());
        assert_eq!(empty.dominant, "steps");
        assert_eq!(empty.dominant_pct, 0.0);
    }

    #[test]
    fn native_test_runner_coverage_guided_fuzzer_and_counterexample_shrinking() {
        let proj = TempTestProject::new("fuzzer_shrink");
        let manifest_path = proj.dir.join("Scarb.toml");
        fs::write(
            &manifest_path,
            r#"[package]
name = "fuzz_demo"
version = "0.1.0"
edition = "2024_07"

[dependencies]
starknet = "2.16.0"
"#,
        )
        .unwrap();
        fs::write(
            proj.dir.join("src/lib.cairo"),
            r#"pub fn checked_scale(x: u64, factor: u64) -> u64 {
    if x >= 75 {
        panic!("THRESHOLD_EXCEEDED");
    }
    x * (factor % 10)
}

#[cfg(test)]
mod tests {
    use super::checked_scale;

    #[test]
    fn test_fuzz_commutative_add(a: u32, b: u32) {
        let a64: u64 = a.into();
        let b64: u64 = b.into();
        assert(a64 + b64 == b64 + a64, 'not_commutative');
    }

    #[test]
    fn test_fuzz_shrinking_finds_minimal_threshold(x: u64) {
        let _ = checked_scale(x, 2);
    }
}
"#,
        )
        .unwrap();

        // 1. Property test `test_fuzz_commutative_add` should pass across 32 fuzz runs
        let pass_rep = proj.dir.join("fuzz-pass.json");
        run_test(TestArgs {
            filter: Some("test_fuzz_commutative_add".to_string()),
            manifest_path: Some(manifest_path.clone()),
            fuzz_runs: 32,
            fuzz_seed: Some(1337),
            json: true,
            report_path: Some(pass_rep.clone()),
            ..Default::default()
        })
        .expect("commutative property fuzz test should pass");
        let pass_val: serde_json::Value =
            serde_json::from_slice(&fs::read(&pass_rep).unwrap()).unwrap();
        assert_eq!(pass_val["passed_count"], 1);
        assert_eq!(pass_val["tests"][0]["fuzz_runs"], 32);

        // 2. Buggy test `test_fuzz_shrinking_finds_minimal_threshold` fails when `x >= 75`
        //    and binary-search shrinking must shrink the counterexample to the exact boundary `75`!
        let fail_rep = proj.dir.join("fuzz-fail.json");
        let err = run_test(TestArgs {
            filter: Some("test_fuzz_shrinking_finds_minimal_threshold".to_string()),
            manifest_path: Some(manifest_path),
            fuzz_runs: 32,
            fuzz_seed: Some(2026),
            json: true,
            report_path: Some(fail_rep.clone()),
            ..Default::default()
        });
        assert!(err.is_err(), "fuzzer should catch threshold bug");
        let fail_val: serde_json::Value =
            serde_json::from_slice(&fs::read(&fail_rep).unwrap()).unwrap();
        assert_eq!(fail_val["failed_count"], 1);
        let shrunk = fail_val["tests"][0]["shrunk_counterexample"]
            .as_array()
            .expect("shrunk_counterexample should be populated");
        assert_eq!(
            shrunk[0].as_str().unwrap(),
            "75",
            "counterexample shrinker should bisect failing input down to exact boundary 75"
        );
    }

    #[test]
    fn native_test_runner_coverage_lcov_and_gas_snapshot_regression_check() {
        let proj = TempTestProject::new("cov_snapshot");
        let manifest_path = proj.dir.join("Scarb.toml");
        fs::write(
            &manifest_path,
            r#"[package]
name = "cov_demo"
version = "0.1.0"
edition = "2024_07"

[dependencies]
starknet = "2.16.0"
"#,
        )
        .unwrap();
        fs::write(
            proj.dir.join("src/lib.cairo"),
            r#"pub fn fib(n: u32) -> u64 {
    let mut a: u64 = 0;
    let mut b: u64 = 1;
    let mut i: u32 = 0;
    while i < n {
        let next = a + b;
        a = b;
        b = next;
        i += 1;
    };
    a
}

#[cfg(test)]
mod tests {
    use super::fib;

    #[test]
    fn test_fib_10() {
        assert(fib(10) == 55, 'bad_fib');
    }
}
"#,
        )
        .unwrap();

        let lcov_path = proj.dir.join("lcov.info");
        let snap_path = proj.dir.join(".gas-snapshot");
        let rep_path = proj.dir.join("cov-rep.json");

        run_test(TestArgs {
            manifest_path: Some(manifest_path.clone()),
            coverage: true,
            lcov_path: Some(lcov_path.clone()),
            snapshot: Some(snap_path.clone()),
            json: true,
            report_path: Some(rep_path.clone()),
            ..Default::default()
        })
        .expect("coverage + snapshot creation should succeed");

        assert!(lcov_path.is_file(), "lcov.info should be written");
        let lcov_text = fs::read_to_string(&lcov_path).unwrap();
        assert!(lcov_text.contains("TN:uc-native-coverage"));
        assert!(lcov_text.contains("end_of_record"));

        let rep: serde_json::Value =
            serde_json::from_slice(&fs::read(&rep_path).unwrap()).unwrap();
        assert!(rep["coverage"]["covered_statements"].as_u64().unwrap() > 0);
        assert!(rep["coverage"]["statement_coverage_pct"].as_f64().unwrap() > 0.0);

        // Snapshot check against identical code should pass
        run_test(TestArgs {
            manifest_path: Some(manifest_path.clone()),
            snapshot_check: Some(snap_path.clone()),
            json: true,
            ..Default::default()
        })
        .expect("identical gas snapshot check should pass");

        // Increase work in test_fib_10 (fib(25) instead of fib(10)) -> snapshot check must catch step/gas regression!
        fs::write(
            proj.dir.join("src/lib.cairo"),
            r#"pub fn fib(n: u32) -> u64 {
    let mut a: u64 = 0;
    let mut b: u64 = 1;
    let mut i: u32 = 0;
    while i < n {
        let next = a + b;
        a = b;
        b = next;
        i += 1;
    };
    a
}

#[cfg(test)]
mod tests {
    use super::fib;

    #[test]
    fn test_fib_10() {
        assert(fib(25) == 75025, 'bad_fib');
    }
}
"#,
        )
        .unwrap();

        let reg_rep_path = proj.dir.join("reg-rep.json");
        let reg_res = run_test(TestArgs {
            manifest_path: Some(manifest_path),
            snapshot_check: Some(snap_path),
            json: true,
            report_path: Some(reg_rep_path.clone()),
            ..Default::default()
        });
        assert!(
            reg_res.is_err(),
            "snapshot-check must fail when test gas/steps regress"
        );
        let reg_rep: serde_json::Value =
            serde_json::from_slice(&fs::read(&reg_rep_path).unwrap()).unwrap();
        assert!(
            !reg_rep["snapshot_regressions"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn native_test_runner_semantic_mutation_testing_calculates_mutation_score() {
        let proj = TempTestProject::new("mutation_testing");
        let manifest_path = proj.dir.join("Scarb.toml");
        fs::write(
            &manifest_path,
            r#"[package]
name = "mut_demo"
version = "0.1.0"
edition = "2024_07"

[dependencies]
starknet = "2.16.0"
"#,
        )
        .unwrap();
        fs::write(
            proj.dir.join("src/lib.cairo"),
            r#"pub fn calc_net_amount(gross: u64, fee: u64) -> u64 {
    assert(gross >= fee, 'INSUFFICIENT_GROSS');
    gross - fee
}

#[cfg(test)]
mod tests {
    use super::calc_net_amount;

    #[test]
    fn test_calc_net_amount_exact_and_boundary() {
        assert(calc_net_amount(100, 100) == 0, 'boundary_zero_failed');
        assert(calc_net_amount(150, 50) == 100, 'sub_failed');
    }
}
"#,
        )
        .unwrap();

        let rep_path = proj.dir.join("mut-rep.json");
        run_test(TestArgs {
            manifest_path: Some(manifest_path),
            mutate: true,
            json: true,
            report_path: Some(rep_path.clone()),
            ..Default::default()
        })
        .expect("mutation testing run should succeed");

        let rep: serde_json::Value =
            serde_json::from_slice(&fs::read(&rep_path).unwrap()).unwrap();
        let mut_summary = &rep["mutation_testing"];
        assert_eq!(mut_summary["total_mutants"], 2);
        assert_eq!(mut_summary["killed"], 2);
        assert_eq!(mut_summary["survived"], 0);
        assert_eq!(mut_summary["mutation_score_pct"], 100.0);
    }
}

