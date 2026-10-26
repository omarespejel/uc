use super::*;
use cairo_lang_sierra::ids::ConcreteTypeId;
use cairo_lang_sierra::program::{BranchTarget, GenericArg, Program, Statement};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::rc::Rc;

#[derive(Args, Debug, Clone, Default)]
pub(crate) struct ProveArgs {
    /// Optional substring filter for function, contract, or entrypoint names.
    pub filter: Option<String>,

    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    /// Package to analyze and prove.
    #[arg(short = 'p', long)]
    pub package: Option<String>,

    /// Run formal verification and Caracal analysis across all packages in the workspace.
    #[arg(short = 'w', long)]
    pub workspace: bool,

    /// Space or comma separated list of features to activate.
    #[arg(short = 'F', long)]
    pub features: Vec<String>,

    /// Maximum statements followed along one symbolic path (bounded model checking horizon).
    /// A function that hits it is reported with `exploration_truncated` and never PROVEN_SAFE.
    #[arg(long, default_value_t = 1024)]
    pub max_depth: usize,

    /// Maximum symbolic paths explored per function before exploration stops.
    #[arg(long, default_value_t = 1024)]
    pub max_paths: usize,

    /// Wall-clock budget per function, in milliseconds; a function that exceeds it is reported
    /// with `exploration_truncated` (BOUNDED_SAFE at best), never PROVEN_SAFE.
    #[arg(long, default_value_t = 15_000)]
    pub function_timeout_ms: u64,

    /// Run modernized Trail of Bits Caracal Sierra IR taint, CFG dominance, and dataflow detectors.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub caracal: bool,

    /// Number of stateful multi-transaction invariant sequences to execute (Medusa/Echidna style).
    #[arg(long, default_value_t = 32)]
    pub stateful_runs: usize,

    /// Fail with non-zero exit code if high/critical Caracal findings or counterexamples are found.
    #[arg(long)]
    pub fail_on_findings: bool,

    /// Force recompilation without reading cached compilation artifacts.
    #[arg(long)]
    pub no_cache: bool,

    /// Export OASIS SARIF v2.1.0 report for GitHub Code Scanning / Security tab.
    #[arg(long)]
    pub sarif: Option<PathBuf>,

    /// Emit ready-to-run Cairo `#[test]` reproduction harnesses for all solved counterexamples.
    #[arg(long)]
    pub emit_repro: Option<PathBuf>,

    /// Write one SMT-LIB 2 file per analysed function with every panic path's constraints as a
    /// `(check-sat)` obligation, plus `obligations.json` listing the expected answers, so the
    /// PROVEN_SAFE and COUNTEREXAMPLE_FOUND claims can be checked with z3 or cvc5
    /// (`benchmarks/scripts/check_smt_obligations.py`).
    #[arg(long, value_name = "DIR")]
    pub emit_smt: Option<PathBuf>,

    /// Verify upgrade compatibility (storage slots, entry points, calldata arity, auth guards)
    /// against a baseline: a manifest or package directory, or a deployed contract address or
    /// class hash (`0x...`, fetched from `--url` / `--network`).
    #[arg(long, value_name = "MANIFEST|ADDRESS")]
    pub check_upgrade: Option<String>,

    /// RPC URL for `--check-upgrade 0x...` (else `STARKNET_RPC`, else `--network`).
    #[arg(short = 'u', long)]
    pub url: Option<String>,

    /// Network for `--check-upgrade 0x...`: `mainnet`, `sepolia` or `devnet`.
    #[arg(short = 'n', long)]
    pub network: Option<String>,

    /// Prove formal functional equivalence (return values, panic conditions, and storage writes) against a baseline manifest.
    #[arg(long)]
    pub check_equivalence: Option<PathBuf>,

    /// Directly analyze a compiled `.sierra.json` or `.contract_class.json` file without a Scarb project.
    #[arg(long)]
    pub sierra_file: Option<PathBuf>,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub report_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct CaracalSierraFinding {
    pub detector: String,
    pub severity: String,
    pub confidence: String,
    pub contract_or_module: String,
    pub function: String,
    pub sierra_statement_idx: usize,
    pub description: String,
    pub recommendation: String,
    pub taint_flow: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SymbolicFunctionProofReport {
    pub function: String,
    pub kind: String,
    pub params: Vec<String>,
    pub status: String,
    pub explored_paths: usize,
    pub ok_paths: usize,
    pub panic_paths: usize,
    pub unreachable_panic_paths: usize,
    /// Panic paths whose constraints the in-process solver could neither satisfy nor refute.
    /// Any such path keeps the function out of PROVEN_SAFE (status UNDECIDED).
    #[serde(default)]
    pub undecided_panic_paths: usize,
    /// True when exploration stopped at the depth or path budget, so unexplored paths remain.
    #[serde(default)]
    pub exploration_truncated: bool,
    pub proven_overflow_free: bool,
    pub preconditions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterexample: Option<BTreeMap<String, String>>,
    /// The counterexample as Starknet calldata felts in parameter order (`u256` as low, high),
    /// ready for `uc call` / `uc invoke` or a devnet replay.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterexample_calldata: Option<Vec<String>>,
    /// Path of the SMT-LIB file written by `--emit-smt` for this function.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smt_file: Option<String>,
    /// Every panic path's constraints and the solver's verdict, for the SMT-LIB export.
    #[serde(skip)]
    pub(crate) obligations: Vec<PathObligation>,
    /// `(name, lower bound, upper bound)` per symbolic parameter, for the SMT-LIB export.
    #[serde(skip)]
    pub(crate) param_bounds: Vec<(String, i128, i128)>,
    /// Sierra type name per symbolic parameter, for calldata encoding.
    #[serde(skip)]
    pub(crate) param_types: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterexample_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concrete_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concrete_panic_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repro_test: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct StatefulInvariantResult {
    pub invariant_name: String,
    pub sequences_run: usize,
    pub sequence_length: usize,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shrunk_call_sequence: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct UpgradeSafetyReport {
    /// The baseline manifest, or the deployed address / class hash the candidate replaces.
    pub baseline_manifest: String,
    /// Class hash of the deployed baseline, when the baseline came from chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_class_hash: Option<String>,
    pub status: String,
    pub baseline_storage_slots: usize,
    pub candidate_storage_slots: usize,
    pub removed_storage_slots: Vec<String>,
    pub removed_entrypoints: Vec<String>,
    pub arity_mismatches: Vec<String>,
    pub auth_regressions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct FunctionalEquivalenceEntry {
    pub function: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub divergence_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub witness_inputs: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_output: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct FunctionalEquivalenceReport {
    pub baseline_manifest: String,
    pub status: String,
    pub equivalent_count: usize,
    pub divergent_count: usize,
    pub functions: Vec<FunctionalEquivalenceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PackageProveReport {
    pub package_name: String,
    pub manifest_path: String,
    pub sierra_functions_analyzed: usize,
    pub sierra_statements_analyzed: usize,
    pub proven_safe_count: usize,
    pub guarded_safe_count: usize,
    pub counterexample_count: usize,
    #[serde(default)]
    pub undecided_count: usize,
    #[serde(default)]
    pub bounded_safe_count: usize,
    pub caracal_findings_count: usize,
    pub symbolic_proofs: Vec<SymbolicFunctionProofReport>,
    pub caracal_findings: Vec<CaracalSierraFinding>,
    pub stateful_invariants: Vec<StatefulInvariantResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upgrade_safety: Option<UpgradeSafetyReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equivalence: Option<FunctionalEquivalenceReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProveWorkspaceReport {
    pub schema_version: u32,
    pub engine: String,
    pub sierra_ir_version: String,
    pub elapsed_ms: f64,
    pub total_functions_analyzed: usize,
    pub total_proven_safe: usize,
    pub total_guarded_safe: usize,
    pub total_counterexamples: usize,
    pub total_caracal_findings: usize,
    pub packages: Vec<PackageProveReport>,
}

// ============================================================================
// Sierra IR Metadata & CFG Index
// ============================================================================

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct LibfuncMeta {
    /// Per branch, which results are builtins (range check, gas, bitwise, ...), from the
    /// libfunc's signature; empty when the program registry could not be built.
    builtin_results: Vec<Vec<bool>>,
    generic_id: String,
    debug_name: String,
    user_func_callee: Option<u64>,
    const_value: Option<i128>,
    /// The full constant for `*_const` libfuncs whose value exceeds `i128` (storage base
    /// addresses), as lowercase hex without `0x`.
    big_const_hex: Option<String>,
    /// Generic type arguments in order (`downcast<From, To>` needs `To`).
    type_args: Vec<ConcreteTypeId>,
    enum_variant_idx: Option<usize>,
    is_panic_result_enum: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct TypeMeta {
    generic_id: String,
    debug_name: String,
    const_value: Option<i128>,
    /// Full constant of a `Const<T, v>` type whose value exceeds `i128`, lowercase hex.
    big_const_hex: Option<String>,
    is_panic_result: bool,
}

struct SierraProgramIndex<'a> {
    program: &'a Program,
    libfuncs: HashMap<u64, LibfuncMeta>,
    types: HashMap<u64, TypeMeta>,
    func_ranges: Vec<(usize, usize, usize)>, // (func_idx, start_stmt, end_stmt)
    func_by_id: HashMap<u64, usize>,
}

impl<'a> SierraProgramIndex<'a> {
    fn build(program: &'a Program) -> Self {
        use cairo_lang_sierra::extensions::core::{CoreLibfunc, CoreType};
        use cairo_lang_sierra::extensions::ConcreteLibfunc;
        let registry = cairo_lang_sierra::program_registry::ProgramRegistry::<CoreType, CoreLibfunc>::new(program).ok();
        let mut types: HashMap<u64, TypeMeta> = HashMap::new();
        for td in &program.type_declarations {
            let generic_id = td.long_id.generic_id.0.as_str().to_string();
            let debug_name = td
                .id
                .debug_name
                .as_ref()
                .map(|s| s.as_str().to_string())
                .unwrap_or_else(|| generic_id.clone());
            let mut const_value = None;
            let mut big_const_hex = None;
            let mut is_panic_result = debug_name.contains("PanicResult");
            for arg in &td.long_id.generic_args {
                match arg {
                    GenericArg::Value(v) => {
                        if let Ok(val) = i128::try_from(v.clone()) {
                            const_value = Some(val);
                        } else {
                            let hex = format!("{:x}", v);
                            if !hex.starts_with('-') {
                                big_const_hex = Some(hex);
                            }
                        }
                    }
                    GenericArg::UserType(ut) => {
                        if let Some(dn) = &ut.debug_name {
                            if dn.as_str().contains("PanicResult") {
                                is_panic_result = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
            types.insert(
                td.id.id,
                TypeMeta {
                    generic_id,
                    debug_name,
                    const_value,
                    big_const_hex,
                    is_panic_result,
                },
            );
        }
        // `Const<NonZero<T>, Const<T, v>>` (a constant divisor) carries its value in the inner
        // constant type. Sierra may declare it before that type, hence the second pass.
        for td in &program.type_declarations {
            let [GenericArg::Type(wrapper), GenericArg::Type(inner)] = td.long_id.generic_args.as_slice() else {
                continue;
            };
            if td.long_id.generic_id.0.as_str() != "Const"
                || !types.get(&wrapper.id).is_some_and(|t| t.generic_id == "NonZero")
            {
                continue;
            }
            let Some((value, big)) = types
                .get(&inner.id)
                .filter(|t| t.generic_id == "Const")
                .map(|t| (t.const_value, t.big_const_hex.clone()))
            else {
                continue;
            };
            if let Some(meta) = types.get_mut(&td.id.id) {
                meta.const_value = value;
                meta.big_const_hex = big;
            }
        }

        // Deployed classes carry no debug names, so recognise `PanicResult` structurally: its
        // error variant is `(core::panics::Panic, Array<felt252>)`, and the empty `Panic` struct
        // keeps its user-type id (a hash of the name) even when names are stripped.
        let panic_marker = cairo_lang_sierra::ids::UserTypeId::from_string("core::panics::Panic").id;
        let mut panic_struct_types: HashSet<u64> = HashSet::new();
        for td in &program.type_declarations {
            if td.long_id.generic_id.0.as_str() == "Struct"
                && td.long_id.generic_args.iter().any(|a| {
                    matches!(a, GenericArg::UserType(ut) if ut.id == panic_marker)
                })
            {
                panic_struct_types.insert(td.id.id);
            }
        }
        let mut panic_payload_types: HashSet<u64> = HashSet::new();
        for td in &program.type_declarations {
            if td.long_id.generic_id.0.as_str() == "Struct"
                && td.long_id.generic_args.iter().any(|a| {
                    matches!(a, GenericArg::Type(t) if panic_struct_types.contains(&t.id))
                })
            {
                panic_payload_types.insert(td.id.id);
            }
        }
        // Older Sierra versions hashed user type ids differently; fall back to the shape of the
        // error payload: a two-member struct `(<empty struct>, Array<felt252>)`.
        let decl_by_id: HashMap<u64, &cairo_lang_sierra::program::TypeDeclaration> =
            program.type_declarations.iter().map(|td| (td.id.id, td)).collect();
        let type_args = |id: u64| -> Vec<u64> {
            decl_by_id
                .get(&id)
                .map(|td| {
                    td.long_id
                        .generic_args
                        .iter()
                        .filter_map(|a| match a {
                            GenericArg::Type(t) => Some(t.id),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let generic_of = |id: u64| -> &str {
            decl_by_id
                .get(&id)
                .map(|td| td.long_id.generic_id.0.as_str())
                .unwrap_or("")
        };
        let is_felt252_array = |id: u64| {
            generic_of(id) == "Array"
                && type_args(id).first().is_some_and(|inner| generic_of(*inner) == "felt252")
        };
        for td in &program.type_declarations {
            if td.long_id.generic_id.0.as_str() != "Enum" {
                continue;
            }
            let variants = type_args(td.id.id);
            let marker_match = variants.iter().any(|v| panic_payload_types.contains(v));
            let shape_match = variants.len() == 2 && {
                let err = variants[1];
                generic_of(err) == "Struct" && {
                    let members = type_args(err);
                    members.len() == 2
                        && generic_of(members[0]) == "Struct"
                        && type_args(members[0]).is_empty()
                        && is_felt252_array(members[1])
                }
            };
            if std::env::var("UC_PROVE_DEBUG_TYPES").is_ok() && variants.len() == 2 {
                let err = variants[1];
                let members = type_args(err);
                eprintln!(
                    "enum {} variants={:?} err_generic={} members={:?} member_generics={:?} marker={marker_match} shape={shape_match}",
                    td.id.id, variants, generic_of(err), members,
                    members.iter().map(|m| (generic_of(*m).to_string(), type_args(*m).len())).collect::<Vec<_>>()
                );
            }
            if marker_match || shape_match {
                if let Some(tm) = types.get_mut(&td.id.id) {
                    tm.is_panic_result = true;
                }
            }
        }

        let mut libfuncs: HashMap<u64, LibfuncMeta> = HashMap::new();
        for ld in &program.libfunc_declarations {
            let generic_id = ld.long_id.generic_id.0.as_str().to_string();
            let debug_name = ld
                .id
                .debug_name
                .as_ref()
                .map(|s| s.as_str().to_string())
                .unwrap_or_else(|| generic_id.clone());
            let mut user_func_callee = None;
            let mut const_value = None;
            let mut big_const_hex = None;
            let mut type_args: Vec<ConcreteTypeId> = Vec::new();
            let mut enum_variant_idx = None;
            let mut is_panic_result_enum = debug_name.contains("PanicResult");

            for (idx, arg) in ld.long_id.generic_args.iter().enumerate() {
                match arg {
                    GenericArg::UserFunc(fid) => {
                        user_func_callee = Some(fid.id);
                    }
                    GenericArg::Value(v) => {
                        if let Ok(val) = i128::try_from(v.clone()) {
                            if generic_id == "enum_init" && idx == 1 {
                                enum_variant_idx = usize::try_from(val).ok();
                            } else {
                                const_value = Some(val);
                            }
                        } else {
                            let hex = format!("{:x}", v);
                            if !hex.starts_with('-') {
                                big_const_hex = Some(hex);
                            }
                        }
                    }
                    GenericArg::Type(tid) => {
                        type_args.push(tid.clone());
                        if let Some(tm) = types.get(&tid.id) {
                            if tm.const_value.is_some() && const_value.is_none() {
                                const_value = tm.const_value;
                            }
                            if tm.big_const_hex.is_some() && big_const_hex.is_none() {
                                big_const_hex = tm.big_const_hex.clone();
                            }
                            if tm.is_panic_result {
                                is_panic_result_enum = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
            let builtin_results = registry
                .as_ref()
                .and_then(|r| r.get_libfunc(&ld.id).ok())
                .map(|lf| {
                    lf.branch_signatures()
                        .iter()
                        .map(|b| {
                            b.vars
                                .iter()
                                .map(|v| types.get(&v.ty.id).is_some_and(|t| is_builtin_generic(&t.generic_id)))
                                .collect()
                        })
                        .collect()
                })
                .unwrap_or_default();
            libfuncs.insert(
                ld.id.id,
                LibfuncMeta {
                    builtin_results,
                    generic_id,
                    debug_name,
                    user_func_callee,
                    const_value,
                    big_const_hex,
                    type_args,
                    enum_variant_idx,
                    is_panic_result_enum,
                },
            );
        }

        let mut sorted_funcs: Vec<(usize, usize)> = program
            .funcs
            .iter()
            .enumerate()
            .map(|(idx, f)| (idx, f.entry_point.0))
            .collect();
        sorted_funcs.sort_by_key(|(_, ep)| *ep);

        let total_stmts = program.statements.len();
        let mut func_ranges = vec![(0usize, 0usize, 0usize); program.funcs.len()];
        let mut func_by_id = HashMap::new();
        for (pos, (func_idx, start_stmt)) in sorted_funcs.iter().copied().enumerate() {
            let end_stmt = sorted_funcs
                .get(pos + 1)
                .map(|(_, next_ep)| *next_ep)
                .unwrap_or(total_stmts);
            func_ranges[func_idx] = (func_idx, start_stmt, end_stmt);
            func_by_id.insert(program.funcs[func_idx].id.id, func_idx);
        }

        Self {
            program,
            libfuncs,
            types,
            func_ranges,
            func_by_id,
        }
    }

    fn func_name(&self, func_idx: usize) -> String {
        let f = &self.program.funcs[func_idx];
        f.id.debug_name
            .as_ref()
            .map(|s| s.as_str().to_string())
            .unwrap_or_else(|| format!("func_{}", f.id.id))
    }

    fn type_name(&self, ty_id: &ConcreteTypeId) -> String {
        if let Some(tm) = self.types.get(&ty_id.id) {
            return tm.debug_name.clone();
        }
        ty_id
            .debug_name
            .as_ref()
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("type_{}", ty_id.id))
    }
}

// ============================================================================
// Interprocedural Function Summaries & Caracal Sierra IR Taint/CFG Analysis
// ============================================================================

const TAINT_NONE: u32 = 0;
const TAINT_EXTERNAL_PARAM: u32 = 1 << 0;
const TAINT_CALLER_ADDRESS: u32 = 1 << 1;
const TAINT_BLOCK_TIMESTAMP: u32 = 1 << 2;
const TAINT_STORAGE_READ: u32 = 1 << 3;
const TAINT_SYSCALL_RESULT: u32 = 1 << 4;
const TAINT_FELT_ARITH: u32 = 1 << 5;

#[derive(Debug, Clone, Default)]
struct FunctionInterprocSummary {
    always_panics: bool,
    is_recursive_loop: bool,
    reads_caller_address: bool,
    reads_block_timestamp: bool,
    enforces_caller_guard: bool,
    enforces_timestamp_guard: bool,
    enforces_signature_guard: bool,
    performs_storage_write: bool,
    performs_storage_read: bool,
    performs_external_call: bool,
    performs_replace_class: bool,
    /// Some other function in the program calls this one (self-recursion excluded).
    has_callers: bool,
    /// Bitmask of parameter positions that (transitively) feed a loop bound comparison without
    /// being capped by an `assert`-style comparison on the way.
    bound_params: u64,
    /// Bitmask of parameter positions this function compares outside a loop (a cap).
    validated_params: u64,
    /// Call sites: `(callee_idx, per-argument parameter masks)`, used to propagate `bound_params`.
    call_sites: Vec<(usize, Vec<u64>)>,
}

/// Functions whose every return is the panic variant (`core::panics::panic` and the
/// `panic_with_*` helpers, plus anything that only ever tail-calls them). Name independent.
fn compute_always_panics(index: &SierraProgramIndex<'_>) -> Vec<bool> {
    let n_funcs = index.program.funcs.len();
    let mut always = vec![false; n_funcs];
    for _round in 0..4 {
        let mut changed = false;
        for func_idx in 0..n_funcs {
            if always[func_idx] {
                continue;
            }
            let (_, start, end) = index.func_ranges[func_idx];
            let mut panic_vars: HashSet<u64> = HashSet::new();
            let mut panic_returns = 0usize;
            let mut ok_returns = 0usize;
            for stmt_idx in start..end {
                match &index.program.statements[stmt_idx] {
                    Statement::Invocation(inv) => {
                        let Some(lf) = index.libfuncs.get(&inv.libfunc_id.id) else { continue };
                        let gid = lf.generic_id.as_str();
                        let produces_panic = (gid == "enum_init"
                            && lf.is_panic_result_enum
                            && lf.enum_variant_idx == Some(1))
                            || (gid == "function_call"
                                && lf
                                    .user_func_callee
                                    .and_then(|cid| index.func_by_id.get(&cid))
                                    .is_some_and(|&cidx| always[cidx]))
                            || ((gid == "rename" || gid == "dup" || gid == "struct_construct" || gid == "store_temp")
                                && inv.args.iter().any(|a| panic_vars.contains(&a.id)));
                        if produces_panic {
                            for b in &inv.branches {
                                for r in &b.results {
                                    panic_vars.insert(r.id);
                                }
                            }
                        }
                    }
                    Statement::Return(ret_vars) => {
                        if ret_vars.iter().any(|v| panic_vars.contains(&v.id)) {
                            panic_returns += 1;
                        } else {
                            ok_returns += 1;
                        }
                    }
                }
            }
            if panic_returns > 0 && ok_returns == 0 {
                always[func_idx] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    always
}

fn compute_function_summaries(index: &SierraProgramIndex<'_>) -> Vec<FunctionInterprocSummary> {
    let n_funcs = index.program.funcs.len();
    let mut summaries = vec![FunctionInterprocSummary::default(); n_funcs];
    let structurally_always_panics = compute_always_panics(index);

    // Pass 1: Local statement inspection per function
    let mut callees_per_func: Vec<Vec<usize>> = vec![Vec::new(); n_funcs];
    for func_idx in 0..n_funcs {
        let fname = index.func_name(func_idx);
        let (_, start, end) = index.func_ranges[func_idx];
        let mut s = FunctionInterprocSummary::default();

        if fname.contains("core::panics::")
            || fname.ends_with("::panic")
            || fname.contains("panic_with_")
        {
            s.always_panics = true;
        }
        if fname.contains("is_valid_eddsa_signature")
            || fname.contains("check_ecdsa_signature")
            || fname.contains("verify_eth_signature")
            || fname.contains("assert_only_owner")
            || fname.contains("assert_only_role")
        {
            s.enforces_signature_guard = fname.contains("signature");
            if fname.contains("assert_only_owner") || fname.contains("assert_only_role") {
                s.reads_caller_address = true;
                s.enforces_caller_guard = true;
            }
        }
        if fname.contains("get_caller_address") {
            s.reads_caller_address = true;
        }
        if fname.contains("get_block_timestamp") {
            s.reads_block_timestamp = true;
        }

        let mut panic_vars: HashSet<u64> = HashSet::new();
        let mut ok_returns = 0usize;
        let mut panic_returns = 0usize;
        let mut has_conditional_branch = false;
        // Which parameter positions each variable derives from (bitmask), to find loop bounds
        // and caps on them. Builtins (gas, range check, ...) never carry caller data.
        let mut var_params: HashMap<u64, u64> = HashMap::new();
        let mut builtin_vars: HashSet<u64> = HashSet::new();
        for (i, p) in index.program.funcs[func_idx].params.iter().enumerate() {
            let p_ty = index.type_name(&p.ty);
            if is_builtin_sierra_type(&p_ty) {
                builtin_vars.insert(p.id.id);
            } else if i < 64 {
                var_params.insert(p.id.id, 1u64 << i);
            }
        }
        let mut compare_mask: u64 = 0;

        for stmt_idx in start..end {
            match &index.program.statements[stmt_idx] {
                Statement::Invocation(inv) => {
                    if inv.branches.len() > 1 {
                        has_conditional_branch = true;
                    }
                    if let Some(lf) = index.libfuncs.get(&inv.libfunc_id.id) {
                        let gid = lf.generic_id.as_str();
                        // `array_len` of calldata is bounded by the transaction the caller paid
                        // for; only numeric parameters make a loop bound caller-controlled.
                        mark_threaded_builtin_results(&mut builtin_vars, inv);
                        let arg_mask = inv.args.iter().fold(0u64, |acc, a| {
                            if builtin_vars.contains(&a.id) {
                                acc
                            } else {
                                acc | var_params.get(&a.id).copied().unwrap_or(0)
                            }
                        });
                        // A storage value looked up by a parameter is not the caller's number
                        // either: it was written by an authorised path earlier.
                        // Callee results are the callee's outputs, not the parameter itself
                        // (a `Map::read(key)` result is storage, not caller data); parameter
                        // flow across calls is tracked through `call_sites` instead.
                        if arg_mask != 0
                            && !gid.starts_with("array_len")
                            && gid != "storage_read_syscall"
                            && gid != "function_call"
                        {
                            for b in &inv.branches {
                                for r in &b.results {
                                    if builtin_vars.contains(&r.id) {
                                        continue;
                                    }
                                    var_params.insert(r.id, arg_mask);
                                }
                            }
                            if gid.ends_with("_lt")
                                || gid.ends_with("_le")
                                || gid.ends_with("_overflowing_sub")
                            {
                                compare_mask |= arg_mask;
                            }
                        }
                        if gid == "function_call" {
                            if let Some(callee_idx) =
                                lf.user_func_callee.and_then(|cid| index.func_by_id.get(&cid).copied())
                            {
                                let masks: Vec<u64> = inv
                                    .args
                                    .iter()
                                    .map(|a| var_params.get(&a.id).copied().unwrap_or(0))
                                    .collect();
                                s.call_sites.push((callee_idx, masks));
                            }
                        }
                        if gid.contains("get_execution_info") {
                            s.reads_caller_address = true;
                            s.reads_block_timestamp = true;
                        } else if gid == "storage_write_syscall" {
                            s.performs_storage_write = true;
                        } else if gid == "storage_read_syscall" {
                            s.performs_storage_read = true;
                        } else if gid == "call_contract_syscall" || gid == "library_call_syscall" {
                            s.performs_external_call = true;
                        } else if gid == "replace_class_syscall" {
                            s.performs_replace_class = true;
                        } else if gid == "enum_init"
                            && lf.is_panic_result_enum
                            && lf.enum_variant_idx == Some(1)
                        {
                            for b in &inv.branches {
                                for r in &b.results {
                                    panic_vars.insert(r.id);
                                }
                            }
                        } else if (gid == "rename" || gid == "dup" || gid == "struct_construct" || gid == "store_temp" || gid == "store_local")
                            && inv.args.iter().any(|a| panic_vars.contains(&a.id))
                        {
                            for b in &inv.branches {
                                for r in &b.results {
                                    panic_vars.insert(r.id);
                                }
                            }
                        } else if gid == "function_call" {
                            if let Some(callee_id) = lf.user_func_callee {
                                if let Some(&callee_idx) = index.func_by_id.get(&callee_id) {
                                    callees_per_func[func_idx].push(callee_idx);
                                    let callee_name = index.func_name(callee_idx);
                                    if callee_name.contains("core::panics::")
                                        || callee_name.contains("panic_with_")
                                        || structurally_always_panics[callee_idx]
                                    {
                                        for b in &inv.branches {
                                            for r in &b.results {
                                                panic_vars.insert(r.id);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Statement::Return(ret_vars) => {
                    if ret_vars.iter().any(|v| panic_vars.contains(&v.id)) {
                        panic_returns += 1;
                    } else {
                        ok_returns += 1;
                    }
                }
            }
        }

        if (panic_returns > 0 && ok_returns == 0) || structurally_always_panics[func_idx] {
            s.always_panics = true;
        }
        // Cairo lowers `while`/`loop` bodies into `fn[a-b]` blocks; since 2.18 those are also
        // specialized into clones that recurse through each other, so self-calls alone miss them.
        if callees_per_func[func_idx].contains(&func_idx) || fname.contains('[') {
            s.is_recursive_loop = true;
        }
        if s.is_recursive_loop {
            s.bound_params = compare_mask;
        } else {
            s.validated_params = compare_mask;
        }
        if std::env::var("UC_PROVE_DEBUG_SUMMARIES").is_ok() {
            eprintln!(
                "summary {fname}: reads_caller={} caller_guard={} cond={} panics={} oks={} callees={:?}",
                s.reads_caller_address, s.enforces_caller_guard, has_conditional_branch,
                panic_returns, ok_returns, callees_per_func[func_idx]
            );
        }
        if s.reads_caller_address && has_conditional_branch && panic_returns > 0 {
            s.enforces_caller_guard = true;
        }
        if s.reads_block_timestamp && has_conditional_branch && panic_returns > 0 {
            s.enforces_timestamp_guard = true;
        }
        summaries[func_idx] = s;
    }

    for (caller_idx, callees) in callees_per_func.iter().enumerate() {
        for &callee_idx in callees {
            if callee_idx != caller_idx {
                summaries[callee_idx].has_callers = true;
            }
        }
    }

    // Pass 2: Fixed-point propagation across call graph (up to 6 rounds)
    for _round in 0..6 {
        let mut changed = false;
        for func_idx in 0..n_funcs {
            let mut current = summaries[func_idx].clone();
            for &callee_idx in &callees_per_func[func_idx] {
                if callee_idx == func_idx {
                    continue;
                }
                let callee = &summaries[callee_idx];
                if callee.reads_caller_address && !current.reads_caller_address {
                    current.reads_caller_address = true;
                    changed = true;
                }
                if callee.reads_block_timestamp && !current.reads_block_timestamp {
                    current.reads_block_timestamp = true;
                    changed = true;
                }
                if callee.enforces_caller_guard && !current.enforces_caller_guard {
                    current.enforces_caller_guard = true;
                    changed = true;
                }
                if callee.enforces_timestamp_guard && !current.enforces_timestamp_guard {
                    current.enforces_timestamp_guard = true;
                    changed = true;
                }
                if callee.enforces_signature_guard && !current.enforces_signature_guard {
                    current.enforces_signature_guard = true;
                    changed = true;
                }
                if callee.performs_storage_write && !current.performs_storage_write {
                    current.performs_storage_write = true;
                    changed = true;
                }
                if callee.performs_storage_read && !current.performs_storage_read {
                    current.performs_storage_read = true;
                    changed = true;
                }
                if callee.performs_external_call && !current.performs_external_call {
                    current.performs_external_call = true;
                    changed = true;
                }
                if callee.performs_replace_class && !current.performs_replace_class {
                    current.performs_replace_class = true;
                    changed = true;
                }
            }
            // Loop bounds flow up through call sites: a parameter of this function that is
            // handed to a bound-feeding parameter of a callee is itself a bound, unless this
            // function caps it (`assert(limit <= MAX)`) first.
            for (callee_idx, arg_masks) in &current.call_sites {
                let callee_bound = summaries[*callee_idx].bound_params;
                for (i, mask) in arg_masks.iter().enumerate() {
                    if i < 64 && callee_bound & (1u64 << i) != 0 {
                        let add = mask & !current.validated_params & !current.bound_params;
                        if add != 0 {
                            current.bound_params |= add;
                            changed = true;
                        }
                    }
                }
            }
            summaries[func_idx] = current;
        }
        if !changed {
            break;
        }
    }

    // Pass 3: Propagate caller/signature/timestamp guards down to compiler-generated loop blocks (`foo[start-end]`)
    for _round in 0..4 {
        for func_idx in 0..n_funcs {
            let parent = summaries[func_idx].clone();
            for &callee_idx in &callees_per_func[func_idx] {
                if callee_idx != func_idx && index.func_name(callee_idx).contains('[') {
                    if parent.enforces_caller_guard {
                        summaries[callee_idx].enforces_caller_guard = true;
                    }
                    if parent.enforces_signature_guard {
                        summaries[callee_idx].enforces_signature_guard = true;
                    }
                    if parent.enforces_timestamp_guard {
                        summaries[callee_idx].enforces_timestamp_guard = true;
                    }
                }
            }
        }
    }

    summaries
}

/// Computes which statements within `[start..end)` can reach an `Ok` return (`!is_panic_return`).
fn compute_ok_reachable_statements(
    index: &SierraProgramIndex<'_>,
    summaries: &[FunctionInterprocSummary],
    start: usize,
    end: usize,
) -> (Vec<bool>, HashSet<u64>) {
    let len = end.saturating_sub(start);
    if len == 0 {
        return (Vec::new(), HashSet::new());
    }

    let mut panic_vars: HashSet<u64> = HashSet::new();
    for stmt_idx in start..end {
        if let Statement::Invocation(inv) = &index.program.statements[stmt_idx] {
            if let Some(lf) = index.libfuncs.get(&inv.libfunc_id.id) {
                let gid = lf.generic_id.as_str();
                if gid == "enum_init" && lf.is_panic_result_enum && lf.enum_variant_idx == Some(1) {
                    for b in &inv.branches {
                        for r in &b.results {
                            panic_vars.insert(r.id);
                        }
                    }
                } else if gid == "function_call" {
                    if let Some(callee_id) = lf.user_func_callee {
                        if let Some(&callee_idx) = index.func_by_id.get(&callee_id) {
                            if summaries[callee_idx].always_panics {
                                for b in &inv.branches {
                                    for r in &b.results {
                                        panic_vars.insert(r.id);
                                    }
                                }
                            }
                        }
                    }
                } else if (gid == "rename" || gid == "dup" || gid == "struct_construct" || gid == "store_temp" || gid == "store_local")
                    && inv.args.iter().any(|a| panic_vars.contains(&a.id))
                {
                    for b in &inv.branches {
                        for r in &b.results {
                            panic_vars.insert(r.id);
                        }
                    }
                }
            }
        }
    }

    let mut can_reach_ok = vec![false; len];
    for stmt_idx in start..end {
        if let Statement::Return(ret_vars) = &index.program.statements[stmt_idx] {
            if !ret_vars.iter().any(|v| panic_vars.contains(&v.id)) {
                can_reach_ok[stmt_idx - start] = true;
            }
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        for stmt_idx in (start..end).rev() {
            if can_reach_ok[stmt_idx - start] {
                continue;
            }
            if let Statement::Invocation(inv) = &index.program.statements[stmt_idx] {
                for b in &inv.branches {
                    let target = match b.target {
                        BranchTarget::Fallthrough => stmt_idx + 1,
                        BranchTarget::Statement(s) => s.0,
                    };
                    if target >= start && target < end && can_reach_ok[target - start] {
                        can_reach_ok[stmt_idx - start] = true;
                        changed = true;
                        break;
                    }
                }
            }
        }
    }

    (can_reach_ok, panic_vars)
}

fn clean_starknet_wrapper_short_name(raw: &str) -> String {
    let trimmed = raw.split('[').next().unwrap_or(raw);
    for special in [
        "__execute__",
        "__validate__",
        "__validate_declare__",
        "__validate_deploy__",
    ] {
        if trimmed.ends_with(special) {
            return special.to_string();
        }
    }
    if let Some(rest) = trimmed.strip_prefix("__wrapper__") {
        let rest = rest.trim_start_matches('_');
        if let Some((_, method)) = rest.rsplit_once("__") {
            if !method.is_empty() {
                return method.to_string();
            }
        }
        return rest.to_string();
    }
    trimmed.to_string()
}

fn extract_contract_and_short_name(full_name: &str) -> (String, String) {
    let base = full_name.split('[').next().unwrap_or(full_name);
    for marker in ["::__external::", "::__l1_handler::", "::__constructor::"] {
        if let Some((prefix, short)) = base.split_once(marker) {
            let contract = prefix.rsplit_once("::").map(|(_, c)| c).unwrap_or(prefix);
            return (
                contract.to_string(),
                clean_starknet_wrapper_short_name(short),
            );
        }
    }
    if let Some((prefix, short)) = base.rsplit_once("::") {
        let contract = prefix.rsplit_once("::").map(|(_, c)| c).unwrap_or(prefix);
        return (
            contract.to_string(),
            clean_starknet_wrapper_short_name(short),
        );
    }
    ("crate".to_string(), clean_starknet_wrapper_short_name(base))
}

fn is_sensitive_state_mutator_name(short_name: &str) -> bool {
    let lower = short_name.to_ascii_lowercase();
    lower.starts_with("set_")
        || lower.starts_with("upgrade")
        || lower.contains("upgrade")
        || lower.starts_with("transfer_ownership")
        || lower.starts_with("update_")
        || lower.starts_with("withdraw")
        || lower.starts_with("pause")
        || lower.starts_with("unpause")
        || lower.starts_with("mint")
        || lower.starts_with("burn")
        || lower.starts_with("register")
        || lower.starts_with("schedule_")
        || lower.starts_with("execute_")
        || lower == "__execute__"
}

fn run_caracal_sierra_analysis(
    index: &SierraProgramIndex<'_>,
    summaries: &[FunctionInterprocSummary],
    crate_name: &str,
    filter: Option<&str>,
) -> Vec<CaracalSierraFinding> {
    let mut findings: Vec<CaracalSierraFinding> = Vec::new();
    let mut seen_keys: BTreeSet<(String, String, String)> = BTreeSet::new();

    // Check which contracts have owner/access-control or constructor-initialized state
    let mut contracts_with_auth_or_constructor: HashSet<String> = HashSet::new();
    for func_idx in 0..index.program.funcs.len() {
        let fname = index.func_name(func_idx);
        if !fname.starts_with(&format!("{crate_name}::")) {
            continue;
        }
        let (contract, short) = extract_contract_and_short_name(&fname);
        if fname.contains("::__constructor::")
            || short == "constructor"
            || summaries[func_idx].reads_caller_address
            || summaries[func_idx].enforces_caller_guard
            || summaries[func_idx].enforces_signature_guard
        {
            contracts_with_auth_or_constructor.insert(contract);
        }
    }

    for func_idx in 0..index.program.funcs.len() {
        let fname = index.func_name(func_idx);
        if !fname.starts_with(&format!("{crate_name}::")) {
            continue;
        }
        if fname.contains("core::") || fname.contains("starknet::") {
            continue;
        }
        if let Some(f) = filter {
            if !fname.contains(f) {
                continue;
            }
        }

        let is_external_wrapper =
            fname.contains("::__external::") || fname.contains("::__wrapper__");
        let is_l1_handler_wrapper = fname.contains("::__l1_handler::");
        let is_constructor_wrapper = fname.contains("::__constructor::")
            || fname.ends_with("__constructor");
        let (contract_or_mod, short_name) = extract_contract_and_short_name(&fname);
        let is_constructor = is_constructor_wrapper || short_name == "constructor";
        if contract_or_mod.ends_with("DispatcherImpl")
            || contract_or_mod.ends_with("LibraryDispatcherImpl")
        {
            continue;
        }

        // Skip duplicate analysis on non-wrapper function if an `__external::` or `__wrapper__` exists for it
        let has_wrapper = !is_external_wrapper
            && !is_l1_handler_wrapper
            && !is_constructor_wrapper
            && has_entry_point_wrapper(index, &short_name);

        let (_, start, end) = index.func_ranges[func_idx];
        if start >= end {
            continue;
        }

        let (can_reach_ok, _panic_vars) =
            compute_ok_reachable_statements(index, summaries, start, end);

        // Forward taint, root variable alias tracking, & CFG dominance state
        let mut var_taint: HashMap<u64, u32> = HashMap::new();
        let mut root_var: HashMap<u64, u64> = HashMap::new();
        let mut builtin_vars: HashSet<u64> = HashSet::new();
        for p in &index.program.funcs[func_idx].params {
            root_var.insert(p.id.id, p.id.id);
            let p_ty = index.type_name(&p.ty);
            if is_builtin_sierra_type(&p_ty) {
                builtin_vars.insert(p.id.id);
            } else {
                var_taint.insert(p.id.id, TAINT_EXTERNAL_PARAM);
            }
        }

        let mut caller_checked_before = false;
        let mut timestamp_checked_before = false;
        let mut signature_checked_before = false;
        let mut external_param_validated_before = false;
        let mut seen_external_call_stmt: Option<usize> = None;
        // A storage write that precedes the first external call is the shape of a reentrancy
        // guard or a lock (`start()` writes, call, `end()` writes).
        let mut storage_write_before_external_call = false;
        // Lock recognition: a slot that is read, then written before the external call, is a
        // lock held across the call; writing it back after the call releases it.
        let mut slot_of_var: HashMap<u64, starknet_types_core::felt::Felt> = HashMap::new();
        let mut slots_read: HashSet<starknet_types_core::felt::Felt> = HashSet::new();
        let mut lock_slots: HashSet<starknet_types_core::felt::Felt> = HashSet::new();
        let mut divmod_numerator_vars: HashMap<u64, usize> = HashMap::new();

        for stmt_idx in start..end {
            if !can_reach_ok[stmt_idx - start] {
                continue;
            }
            let Statement::Invocation(inv) = &index.program.statements[stmt_idx] else {
                continue;
            };
            let Some(lf) = index.libfuncs.get(&inv.libfunc_id.id) else {
                continue;
            };
            let gid = lf.generic_id.as_str();

            if (gid == "rename"
                || gid == "dup"
                || gid == "store_temp"
                || gid == "snapshot_take"
                || gid == "unbox")
                && !inv.args.is_empty()
            {
                let src_id = inv.args[inv.args.len() - 1].id;
                let root = root_var.get(&src_id).copied().unwrap_or(src_id);
                for b in &inv.branches {
                    for r in &b.results {
                        root_var.insert(r.id, root);
                    }
                }
            }

            track_detector_storage_slot(&mut slot_of_var, gid, lf, inv);
            if gid == "storage_read_syscall" {
                if let Some(slot) = inv.args.last().and_then(|a| slot_of_var.get(&a.id)) {
                    slots_read.insert(*slot);
                }
            }
            if gid == "storage_write_syscall" && seen_external_call_stmt.is_none() && inv.args.len() >= 2 {
                if let Some(slot) = slot_of_var.get(&inv.args[inv.args.len() - 2].id) {
                    if slots_read.contains(slot) {
                        lock_slots.insert(*slot);
                    }
                }
            }

            mark_threaded_builtin_results(&mut builtin_vars, inv);
            let combined_arg_taint = inv.args.iter().fold(TAINT_NONE, |acc, v| {
                if builtin_vars.contains(&v.id) {
                    acc
                } else {
                    acc | var_taint.get(&v.id).copied().unwrap_or(0)
                }
            });

            // Check if this invocation is a guard branch (one branch cannot reach Ok return)
            let is_guard_branch = inv.branches.len() > 1
                && inv.branches.iter().any(|b| {
                    let t = match b.target {
                        BranchTarget::Fallthrough => stmt_idx + 1,
                        BranchTarget::Statement(s) => s.0,
                    };
                    t >= start && t < end && !can_reach_ok[t - start]
                });

            if is_guard_branch {
                if (combined_arg_taint & TAINT_CALLER_ADDRESS) != 0 {
                    caller_checked_before = true;
                }
                if (combined_arg_taint & TAINT_BLOCK_TIMESTAMP) != 0 {
                    timestamp_checked_before = true;
                }
                if (combined_arg_taint & TAINT_EXTERNAL_PARAM) != 0
                    && !gid.contains("pop_front")
                    && !gid.contains("try_from_felt252")
                    && !gid.starts_with("withdraw_gas")
                    && !gid.starts_with("enum_match")
                {
                    external_param_validated_before = true;
                }
            }

            // Track taint propagation and callee summaries
            let mut out_taint = combined_arg_taint;
            if gid.contains("get_execution_info") {
                out_taint |= TAINT_CALLER_ADDRESS | TAINT_BLOCK_TIMESTAMP;
            } else if gid == "storage_read_syscall" {
                // The value read is storage state (written by an authorised path earlier), not
                // the caller's calldata, even when the address was derived from a parameter.
                out_taint = (combined_arg_taint & !TAINT_EXTERNAL_PARAM) | TAINT_STORAGE_READ;
            } else if gid == "call_contract_syscall" || gid == "library_call_syscall" {
                out_taint |= TAINT_SYSCALL_RESULT;
            } else if gid.starts_with("felt252_add")
                || gid.starts_with("felt252_sub")
                || gid.starts_with("felt252_mul")
            {
                out_taint |= TAINT_FELT_ARITH;
            }

            if gid == "function_call" {
                if let Some(callee_id) = lf.user_func_callee {
                    if let Some(&callee_idx) = index.func_by_id.get(&callee_id) {
                        let callee_sum = &summaries[callee_idx];
                        let callee_name = index.func_name(callee_idx);
                        if callee_name.contains("get_caller_address")
                            || callee_sum.reads_caller_address
                        {
                            out_taint |= TAINT_CALLER_ADDRESS;
                        }
                        if callee_name.contains("get_block_timestamp")
                            || callee_sum.reads_block_timestamp
                        {
                            out_taint |= TAINT_BLOCK_TIMESTAMP;
                        }
                        if callee_sum.enforces_caller_guard {
                            caller_checked_before = true;
                        }
                        if callee_sum.enforces_timestamp_guard {
                            timestamp_checked_before = true;
                        }
                        if callee_sum.enforces_signature_guard {
                            signature_checked_before = true;
                        }
                        if callee_sum.performs_external_call && seen_external_call_stmt.is_none() {
                            seen_external_call_stmt = Some(stmt_idx);
                        }
                        // A storage accessor's result is state, not the caller's parameter.
                        if callee_sum.performs_storage_read && !callee_sum.performs_storage_write {
                            out_taint = (out_taint & !TAINT_EXTERNAL_PARAM) | TAINT_STORAGE_READ;
                        }
                        // Detector 8b: an external entrypoint hands an uncapped parameter to a
                        // loop (directly or through helpers) without any guard on the way.
                        // The argument must land in a callee parameter that feeds a loop bound,
                        // and be a caller-chosen number: iterating over an input `Array`/`Span` is
                        // bounded by the calldata the caller already paid for.
                        let passes_tainted_bound = inv.args.iter().enumerate().any(|(i, a)| {
                            i < 64
                                && callee_sum.bound_params & (1u64 << i) != 0
                                && (var_taint.get(&a.id).copied().unwrap_or(0) & TAINT_EXTERNAL_PARAM)
                                    != 0
                                && index.program.funcs[callee_idx]
                                    .params
                                    .get(i)
                                    .map(|p| index.type_name(&p.ty))
                                    .is_some_and(|ty| is_numeric_sierra_type(&ty))
                        });
                        if is_external_wrapper
                            && passes_tainted_bound
                            && !external_param_validated_before
                        {
                            let key = (
                                "caracal-tainted-loop-bound-sierra".to_string(),
                                contract_or_mod.clone(),
                                short_name.clone(),
                            );
                            if seen_keys.insert(key) {
                                let (_, callee_short) = extract_contract_and_short_name(&callee_name);
                                findings.push(CaracalSierraFinding {
                                    detector: "caracal-tainted-loop-bound-sierra".to_string(),
                                    severity: "Medium".to_string(),
                                    confidence: "Medium".to_string(),
                                    contract_or_module: contract_or_mod.clone(),
                                    function: short_name.clone(),
                                    sierra_statement_idx: stmt_idx,
                                    description: format!(
                                        "External entrypoint `{contract_or_mod}::{short_name}` passes an unchecked calldata value into a loop (`{callee_short}`) at Sierra statement #{stmt_idx}; no `assert(x <= MAX)` style cap dominates the call."
                                    ),
                                    recommendation: "Cap caller-controlled iteration counts before looping (`assert(limit <= MAX_LIMIT, 'LIMIT')`) so a single call cannot exhaust the step limit.".to_string(),
                                    taint_flow: vec![
                                        format!("calldata({short_name})"),
                                        format!("function_call({callee_short})@stmt_{stmt_idx}"),
                                        "recursive_loop_bound".to_string(),
                                    ],
                                });
                            }
                        }
                    }
                }
            }

            for b in &inv.branches {
                for r in &b.results {
                    if out_taint != TAINT_NONE && !builtin_vars.contains(&r.id) {
                        var_taint.insert(r.id, out_taint);
                    }
                }
            }

            // Detector 1 & 5 & 6: Storage Write / Upgrade / State Mutation checks
            let is_state_write = gid == "storage_write_syscall"
                || gid == "replace_class_syscall"
                || (gid == "function_call"
                    && lf
                        .user_func_callee
                        .and_then(|cid| index.func_by_id.get(&cid))
                        .is_some_and(|&cidx| {
                            summaries[cidx].performs_storage_write
                                || summaries[cidx].performs_replace_class
                        }));

            // Internal helpers (`InternalImpl::_mint`) are analysed through the external
            // wrappers that call them; reporting them on their own labels non-entrypoints as
            // "unprotected" even when every caller checks the caller address.
            let is_entry_surface =
                is_external_wrapper || (!has_wrapper && !summaries[func_idx].has_callers);
            if is_state_write
                && !is_constructor
                && !fname.contains('[')
                && (is_external_wrapper || !has_wrapper)
            {
                let has_auth = caller_checked_before
                    || signature_checked_before
                    || summaries[func_idx].enforces_caller_guard
                    || summaries[func_idx].enforces_signature_guard;

                let contract_requires_auth =
                    contracts_with_auth_or_constructor.contains(&contract_or_mod)
                        || is_sensitive_state_mutator_name(&short_name)
                        || gid == "replace_class_syscall";

                if !has_auth && contract_requires_auth && is_entry_surface {
                    let key = (
                        "caracal-unprotected-state-mutation".to_string(),
                        contract_or_mod.clone(),
                        short_name.clone(),
                    );
                    if seen_keys.insert(key) {
                        findings.push(CaracalSierraFinding {
                            detector: "caracal-unprotected-state-mutation".to_string(),
                            severity: if short_name.contains("upgrade")
                                || gid == "replace_class_syscall"
                            {
                                "Critical".to_string()
                            } else {
                                "High".to_string()
                            },
                            confidence: "High".to_string(),
                            contract_or_module: contract_or_mod.clone(),
                            function: short_name.clone(),
                            sierra_statement_idx: stmt_idx,
                            description: format!(
                                "{} `{contract_or_mod}::{short_name}` reaches `{gid}` at Sierra statement #{stmt_idx} on a CFG path with no dominating `get_caller_address()` or signature verification guard.",
                                if is_external_wrapper { "External entrypoint" } else { "Uncalled function" }
                            ),
                            recommendation: "Enforce caller access control (e.g., `assert_only_owner(@self)` or `OwnableComponent`) before mutating contract storage or upgrading class hashes.".to_string(),
                            taint_flow: vec![
                                format!("entrypoint({short_name})"),
                                "no_dominating_caller_check".to_string(),
                                format!("{gid}@stmt_{stmt_idx}"),
                            ],
                        });
                    }
                }

            }

            // Detector 4: Check-Effects-Interactions (CEI) Reentrancy on Sierra IR. Runs on the
            // function that holds the call / write sequence, wrapper or not; one finding per
            // contract method.
            if is_state_write && !is_constructor && !fname.contains('[') {
                if seen_external_call_stmt.is_none() {
                    storage_write_before_external_call = true;
                }
                if let Some(call_stmt) = seen_external_call_stmt {
                    let written_slot = (gid == "storage_write_syscall" && inv.args.len() >= 2)
                        .then(|| slot_of_var.get(&inv.args[inv.args.len() - 2].id).copied())
                        .flatten();
                    // Writing a lock slot back after the call is the release, not a violation.
                    let releases_lock = written_slot.is_some_and(|slot| lock_slots.contains(&slot));
                    if call_stmt < stmt_idx && !releases_lock {
                        let key = (
                            "caracal-cei-reentrancy-sierra".to_string(),
                            contract_or_mod.clone(),
                            short_name.clone(),
                        );
                        if seen_keys.insert(key) {
                            let under_lock = !lock_slots.is_empty();
                            let guarded = under_lock || storage_write_before_external_call;
                            findings.push(CaracalSierraFinding {
                                detector: "caracal-cei-reentrancy-sierra".to_string(),
                                severity: if guarded { "Low".to_string() } else { "High".to_string() },
                                confidence: if under_lock {
                                    "Low".to_string()
                                } else if guarded {
                                    "Medium".to_string()
                                } else {
                                    "Medium".to_string()
                                },
                                contract_or_module: contract_or_mod.clone(),
                                function: short_name.clone(),
                                sierra_statement_idx: stmt_idx,
                                description: if under_lock {
                                    format!(
                                        "`{contract_or_mod}::{short_name}` writes storage at statement #{stmt_idx} after an external call (statement #{call_stmt}) while holding a lock: slot {} is read, written before the call and written back after it. Re-entry is blocked only if every state-changing entrypoint checks that lock.",
                                        lock_slots.iter().map(|f| format!("{f:#x}")).collect::<Vec<_>>().join(", ")
                                    )
                                } else if guarded {
                                    format!(
                                        "`{contract_or_mod}::{short_name}` writes storage before and after an external call (statement #{call_stmt}). The pre-call write is not a lock (its slot is never read first), so it does not block re-entry on its own; the post-call write at statement #{stmt_idx} still runs on reentered state."
                                    )
                                } else {
                                    format!(
                                        "Check-Effects-Interactions violation in `{contract_or_mod}::{short_name}`: external contract call at Sierra statement #{call_stmt} precedes storage write at statement #{stmt_idx} with no prior state write that could act as a guard."
                                    )
                                },
                                recommendation: "Perform all `storage_write_syscall` state updates before invoking `call_contract_syscall`, or hold a lock (read, assert, write) across the call and release it after.".to_string(),
                                taint_flow: vec![
                                    format!("call_contract_syscall@stmt_{call_stmt}"),
                                    format!("{gid}@stmt_{stmt_idx}"),
                                ],
                            });
                        }
                    }
                }
            }

            // Detector 6: Unvalidated external parameter written to sensitive storage in constructor or setter/upgrade
            if gid == "storage_write_syscall"
                && (is_constructor_wrapper
                    || short_name == "constructor"
                    || is_sensitive_state_mutator_name(&short_name))
                && (combined_arg_taint & TAINT_EXTERNAL_PARAM) != 0
                && !external_param_validated_before
                && !timestamp_checked_before
                && !summaries[func_idx].enforces_timestamp_guard
            {
                let key = (
                    "caracal-unvalidated-external-storage-write".to_string(),
                    contract_or_mod.clone(),
                    short_name.clone(),
                );
                if seen_keys.insert(key) {
                    findings.push(CaracalSierraFinding {
                        detector: "caracal-unvalidated-external-storage-write".to_string(),
                        severity: "Medium".to_string(),
                        confidence: "High".to_string(),
                        contract_or_module: contract_or_mod.clone(),
                        function: short_name.clone(),
                        sierra_statement_idx: stmt_idx,
                        description: format!(
                            "Tainted external parameter in `{contract_or_mod}::{short_name}` flows directly into `storage_write_syscall` at Sierra statement #{stmt_idx} without any dominating non-zero (`!is_zero()`) or range (`<= MAX_BPS`) guard branch."
                        ),
                        recommendation: "Validate external parameters (e.g., `assert!(!owner.is_zero())`, `assert!(class_hash != 0)`, `assert!(fee_bps <= 10_000)`) before writing to storage.".to_string(),
                        taint_flow: vec![
                            format!("ExternalParam({short_name})"),
                            "unvalidated_dataflow".to_string(),
                            format!("storage_write_syscall@stmt_{stmt_idx}"),
                        ],
                    });
                }
            }

            // Detector 2 & 3: External `call_contract_syscall` / `library_call_syscall` checks
            if gid == "call_contract_syscall" || gid == "library_call_syscall" {
                if seen_external_call_stmt.is_none() {
                    seen_external_call_stmt = Some(stmt_idx);
                }
                let has_auth = caller_checked_before
                    || signature_checked_before
                    || summaries[func_idx].enforces_caller_guard
                    || summaries[func_idx].enforces_signature_guard;

                // Internal helpers are reached only through entrypoints, which carry the
                // caller checks; report the arbitrary-call risk on the entry surface.
                if !has_auth
                    && (is_external_wrapper || (!has_wrapper && !summaries[func_idx].has_callers))
                {
                    let key = (
                        "caracal-unprotected-arbitrary-call".to_string(),
                        contract_or_mod.clone(),
                        short_name.clone(),
                    );
                    if seen_keys.insert(key) {
                        findings.push(CaracalSierraFinding {
                            detector: "caracal-unprotected-arbitrary-call".to_string(),
                            severity: "Critical".to_string(),
                            confidence: "High".to_string(),
                            contract_or_module: contract_or_mod.clone(),
                            function: short_name.clone(),
                            sierra_statement_idx: stmt_idx,
                            description: format!(
                                "Entrypoint `{contract_or_mod}::{short_name}` executes `{gid}` at Sierra statement #{stmt_idx} with caller-controlled target/calldata and no dominating `get_caller_address()` or signature check."
                            ),
                            recommendation: "Restrict `__execute__` and arbitrary external call dispatchers with `assert(get_caller_address().is_zero(), ...)` or verified signature guards.".to_string(),
                            taint_flow: vec![
                                format!("ExternalParam({short_name})"),
                                "no_caller_or_signature_dominator".to_string(),
                                format!("{gid}@stmt_{stmt_idx}"),
                            ],
                        });
                    }
                }

                // Detector 3: Swallowed syscall error result (`Result::Err` branch reaches Ok return)
                if inv.branches.len() > 1 {
                    let err_branch = &inv.branches[1];
                    let err_target = match err_branch.target {
                        BranchTarget::Fallthrough => stmt_idx + 1,
                        BranchTarget::Statement(s) => s.0,
                    };
                    if err_target >= start
                        && err_target < end
                        && can_reach_ok[err_target - start]
                    {
                        let key = (
                            "caracal-swallowed-syscall-error".to_string(),
                            contract_or_mod.clone(),
                            short_name.clone(),
                        );
                        if seen_keys.insert(key) {
                            findings.push(CaracalSierraFinding {
                                detector: "caracal-swallowed-syscall-error".to_string(),
                                severity: "Medium".to_string(),
                                confidence: "High".to_string(),
                                contract_or_module: contract_or_mod.clone(),
                                function: short_name.clone(),
                                sierra_statement_idx: stmt_idx,
                                description: format!(
                                    "The `Result::Err` branch of `{gid}` at Sierra statement #{stmt_idx} in `{contract_or_mod}::{short_name}` reaches a normal `Ok` return instead of reverting the transaction."
                                ),
                                recommendation: "Unwrap or propagate syscall failures (`syscall_result.unwrap_syscall()`) so failed subcalls cannot silently succeed.".to_string(),
                                taint_flow: vec![
                                    format!("{gid}@stmt_{stmt_idx}::branch[1](Err)"),
                                    format!("reachable_ok_return@stmt_{err_target}"),
                                ],
                            });
                        }
                    }
                }
            }

            // Detector 7a: Redundant duplicate `u*_safe_divmod` on same input variable
            if gid.ends_with("_safe_divmod") && inv.args.len() >= 2 {
                let raw_num_var = inv.args[inv.args.len() - 2].id;
                let num_var = root_var.get(&raw_num_var).copied().unwrap_or(raw_num_var);
                if let Some(prev_stmt) = divmod_numerator_vars.insert(num_var, stmt_idx) {
                    let key = (
                        "caracal-redundant-divrem-sierra".to_string(),
                        contract_or_mod.clone(),
                        short_name.clone(),
                    );
                    if seen_keys.insert(key) {
                        findings.push(CaracalSierraFinding {
                            detector: "caracal-redundant-divrem-sierra".to_string(),
                            severity: "Low".to_string(),
                            confidence: "High".to_string(),
                            contract_or_module: contract_or_mod.clone(),
                            function: short_name.clone(),
                            sierra_statement_idx: stmt_idx,
                            description: format!(
                                "Function `{contract_or_mod}::{short_name}` invokes `{gid}` twice on Sierra variable `v{num_var}` (at statements #{prev_stmt} and #{stmt_idx}) due to separate `/` and `%` operations."
                            ),
                            recommendation: "Use `core::num::traits::DivRem::div_rem(x, d)` to compute quotient and remainder in a single Sierra `safe_divmod` instruction.".to_string(),
                            taint_flow: vec![
                                format!("{gid}(v{num_var})@stmt_{prev_stmt}"),
                                format!("{gid}(v{num_var})@stmt_{stmt_idx}"),
                            ],
                        });
                    }
                }
            }

            // Detector 7b: Bitwise builtin invoked for parity check
            if gid == "bitwise" || gid.ends_with("_bitwise") {
                let key = (
                    "caracal-bitwise-parity-sierra".to_string(),
                    contract_or_mod.clone(),
                    short_name.clone(),
                );
                if seen_keys.insert(key) {
                    findings.push(CaracalSierraFinding {
                        detector: "caracal-bitwise-parity-sierra".to_string(),
                        severity: "Low".to_string(),
                        confidence: "High".to_string(),
                        contract_or_module: contract_or_mod.clone(),
                        function: short_name.clone(),
                        sierra_statement_idx: stmt_idx,
                        description: format!(
                            "Function `{contract_or_mod}::{short_name}` invokes the expensive `Bitwise` builtin (`{gid}`) at Sierra statement #{stmt_idx}."
                        ),
                        recommendation: "Prefer `DivRem::div_rem(value, 2)` over bitwise masking `(value & 1)` when checking parity in Cairo to avoid `Bitwise` builtin overhead.".to_string(),
                        taint_flow: vec![format!("{gid}@stmt_{stmt_idx}")],
                    });
                }
            }

            // Detector 8: Tainted external loop bound with range-check `<` inequality in recursive loop.
            // Only entry-surface functions carry genuinely external parameters; a helper's `limit`
            // param is whatever its caller passed, and callers typically assert the cap first
            // (`assert(limit <= MAX, ..)`), so reporting helpers produced false positives on
            // every paginated getter. A guard on an external parameter earlier on the path also
            // counts as a bound.
            if (gid.ends_with("_lt") || gid.ends_with("_le") || gid.ends_with("_overflowing_sub"))
                && (combined_arg_taint & TAINT_EXTERNAL_PARAM) != 0
                && summaries[func_idx].is_recursive_loop
                && (is_external_wrapper || !summaries[func_idx].has_callers)
                && !external_param_validated_before
            {
                let key = (
                    "caracal-tainted-loop-bound-sierra".to_string(),
                    contract_or_mod.clone(),
                    short_name.clone(),
                );
                if seen_keys.insert(key) {
                    findings.push(CaracalSierraFinding {
                        detector: "caracal-tainted-loop-bound-sierra".to_string(),
                        severity: "Medium".to_string(),
                        confidence: "High".to_string(),
                        contract_or_module: contract_or_mod.clone(),
                        function: short_name.clone(),
                        sierra_statement_idx: stmt_idx,
                        description: format!(
                            "Recursive Sierra loop `{contract_or_mod}::{short_name}` uses range-checked inequality (`{gid}`) over an unbounded external parameter at statement #{stmt_idx}."
                        ),
                        recommendation: "Cap external loop iteration counts to prevent step-limit DoS and prefer `!=` (`while i != n`) over `<` in tight Cairo loops to save `RangeCheck` builtins.".to_string(),
                        taint_flow: vec![
                            format!("ExternalParam({short_name})"),
                            format!("loop_header({gid})@stmt_{stmt_idx}"),
                            "recursive_loop_call".to_string(),
                        ],
                    });
                }
            }
        }
    }

    findings
}

// ============================================================================
// Symbolic Execution & Bounded Model Checking over Sierra IR
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
enum SymExpr {
    Param(usize, String),
    Const(i128),
    SymbolicOpaque(String),
    Add(Box<SymExpr>, Box<SymExpr>),
    Sub(Box<SymExpr>, Box<SymExpr>),
    Mul(Box<SymExpr>, Box<SymExpr>),
    Div(Box<SymExpr>, Box<SymExpr>),
    Rem(Box<SymExpr>, Box<SymExpr>),
    BitAnd(Box<SymExpr>, Box<SymExpr>),
    BitOr(Box<SymExpr>, Box<SymExpr>),
    BitXor(Box<SymExpr>, Box<SymExpr>),
}

impl SymExpr {
    fn format_human(&self) -> String {
        match self {
            Self::Param(_, name) => name.clone(),
            Self::Const(v) => v.to_string(),
            Self::SymbolicOpaque(s) => s.clone(),
            Self::Add(a, b) => format!("({} + {})", a.format_human(), b.format_human()),
            Self::Sub(a, b) => format!("({} - {})", a.format_human(), b.format_human()),
            Self::Mul(a, b) => format!("({} * {})", a.format_human(), b.format_human()),
            Self::Div(a, b) => format!("({} / {})", a.format_human(), b.format_human()),
            Self::Rem(a, b) => format!("({} % {})", a.format_human(), b.format_human()),
            Self::BitAnd(a, b) => format!("({} & {})", a.format_human(), b.format_human()),
            Self::BitOr(a, b) => format!("({} | {})", a.format_human(), b.format_human()),
            Self::BitXor(a, b) => format!("({} ^ {})", a.format_human(), b.format_human()),
        }
    }

    fn eval(&self, env: &[i128]) -> Option<i128> {
        match self {
            Self::Param(idx, _) => env.get(*idx).copied(),
            Self::Const(v) => Some(*v),
            Self::SymbolicOpaque(_) => None,
            Self::Add(a, b) => a.eval(env)?.checked_add(b.eval(env)?),
            Self::Sub(a, b) => a.eval(env)?.checked_sub(b.eval(env)?),
            Self::Mul(a, b) => a.eval(env)?.checked_mul(b.eval(env)?),
            Self::Div(a, b) => {
                let denom = b.eval(env)?;
                if denom == 0 {
                    None
                } else {
                    a.eval(env)?.checked_div(denom)
                }
            }
            Self::Rem(a, b) => {
                let denom = b.eval(env)?;
                if denom == 0 {
                    None
                } else {
                    a.eval(env)?.checked_rem(denom)
                }
            }
            Self::BitAnd(a, b) => Some(a.eval(env)? & b.eval(env)?),
            Self::BitOr(a, b) => Some(a.eval(env)? | b.eval(env)?),
            Self::BitXor(a, b) => Some(a.eval(env)? ^ b.eval(env)?),
        }
    }

    /// True when the tree has more than `cap` nodes (stops counting early).
    fn exceeds(&self, cap: usize) -> bool {
        fn count(e: &SymExpr, left: &mut usize) -> bool {
            if *left == 0 {
                return true;
            }
            *left -= 1;
            match e {
                SymExpr::Param(..) | SymExpr::Const(_) | SymExpr::SymbolicOpaque(_) => false,
                SymExpr::Add(a, b)
                | SymExpr::Sub(a, b)
                | SymExpr::Mul(a, b)
                | SymExpr::Div(a, b)
                | SymExpr::Rem(a, b)
                | SymExpr::BitAnd(a, b)
                | SymExpr::BitOr(a, b)
                | SymExpr::BitXor(a, b) => count(a, left) || count(b, left),
            }
        }
        let mut left = cap;
        count(self, &mut left)
    }

    /// True when the expression contains a value marked by `mark_unknown_results`: an unknown
    /// function of the parameters, which no witness can evaluate.
    fn mentions_unknown(&self) -> bool {
        match self {
            Self::SymbolicOpaque(name) => name.starts_with(UNKNOWN_PREFIX),
            Self::Param(..) | Self::Const(_) => false,
            Self::Add(a, b)
            | Self::Sub(a, b)
            | Self::Mul(a, b)
            | Self::Div(a, b)
            | Self::Rem(a, b)
            | Self::BitAnd(a, b)
            | Self::BitOr(a, b)
            | Self::BitXor(a, b) => a.mentions_unknown() || b.mentions_unknown(),
        }
    }

    /// True when the integer reading of a felt252 value can be negative (a subtraction or a
    /// negative constant): in Cairo such a value wraps to a huge field element.
    fn may_be_negative(&self) -> bool {
        match self {
            Self::Const(v) => *v < 0,
            Self::Sub(..) => true,
            Self::Param(..) | Self::SymbolicOpaque(_) => false,
            Self::Add(a, b)
            | Self::Mul(a, b)
            | Self::Div(a, b)
            | Self::Rem(a, b)
            | Self::BitAnd(a, b)
            | Self::BitOr(a, b)
            | Self::BitXor(a, b) => a.may_be_negative() || b.may_be_negative(),
        }
    }

    /// Sets `seen[i]` for every parameter `i` the expression mentions.
    fn mark_params(&self, seen: &mut [bool]) {
        match self {
            Self::Param(i, _) => {
                if let Some(slot) = seen.get_mut(*i) {
                    *slot = true;
                }
            }
            Self::Const(_) | Self::SymbolicOpaque(_) => {}
            Self::Add(a, b)
            | Self::Sub(a, b)
            | Self::Mul(a, b)
            | Self::Div(a, b)
            | Self::Rem(a, b)
            | Self::BitAnd(a, b)
            | Self::BitOr(a, b)
            | Self::BitXor(a, b) => {
                a.mark_params(seen);
                b.mark_params(seen);
            }
        }
    }

    fn mentions_param(&self) -> bool {
        match self {
            Self::Param(_, _) => true,
            Self::Const(_) | Self::SymbolicOpaque(_) => false,
            Self::Add(a, b)
            | Self::Sub(a, b)
            | Self::Mul(a, b)
            | Self::Div(a, b)
            | Self::Rem(a, b)
            | Self::BitAnd(a, b)
            | Self::BitOr(a, b)
            | Self::BitXor(a, b) => a.mentions_param() || b.mentions_param(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SymConstraint {
    lhs: SymExpr,
    op: CmpOp,
    rhs: SymExpr,
    reason: String,
    is_overflow_branch: bool,
}

impl SymConstraint {
    fn negate_for_precondition(&self) -> String {
        let neg_op = match self.op {
            CmpOp::Eq => "!=",
            CmpOp::Ne => "==",
            CmpOp::Lt => ">=",
            CmpOp::Le => ">",
            CmpOp::Gt => "<=",
            CmpOp::Ge => "<",
        };
        format!(
            "{} {} {}",
            self.lhs.format_human(),
            neg_op,
            self.rhs.format_human()
        )
    }

    fn eval(&self, env: &[i128]) -> Option<bool> {
        let l = self.lhs.eval(env)?;
        let r = self.rhs.eval(env)?;
        Some(match self.op {
            CmpOp::Eq => l == r,
            CmpOp::Ne => l != r,
            CmpOp::Lt => l < r,
            CmpOp::Le => l <= r,
            CmpOp::Gt => l > r,
            CmpOp::Ge => l >= r,
        })
    }
}

/// Supplies concrete values for storage reads at known addresses during symbolic execution
/// and seeds the counterexample replay with the same values. Backed by `starknet_getStorageAt`
/// for a deployed contract, or by a fixed map in tests. Reads are cached and capped.
pub(crate) struct StorageOracle {
    label: String,
    reader: Box<dyn Fn(&str) -> Option<String> + Send + Sync>,
    cache: std::sync::Mutex<BTreeMap<String, Option<String>>>,
    max_reads: usize,
}

impl StorageOracle {
    /// Reads live storage of `contract` over JSON-RPC (`block_id` latest).
    pub(crate) fn rpc(rpc: &str, contract: &str, max_reads: usize) -> Self {
        let rpc = rpc.to_string();
        let contract_owned = contract.to_string();
        Self {
            label: contract.to_string(),
            reader: Box::new(move |key| {
                starknet_rpc_request(
                    &rpc,
                    "starknet_getStorageAt",
                    serde_json::json!({ "block_id": "latest", "contract_address": contract_owned, "key": key }),
                )
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
            }),
            cache: std::sync::Mutex::new(BTreeMap::new()),
            max_reads,
        }
    }

    /// A fixed `{storage key hex: value hex}` map (tests, offline replays).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn from_map(values: BTreeMap<String, String>) -> Self {
        Self {
            label: "map".to_string(),
            reader: Box::new(move |key| values.get(key).cloned()),
            cache: std::sync::Mutex::new(BTreeMap::new()),
            max_reads: usize::MAX,
        }
    }

    /// The value stored at `key` (`0x...`), as a felt hex string, if the backend has one.
    pub(crate) fn read_hex(&self, key: &str) -> Option<String> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = cache.get(key) {
            return v.clone();
        }
        if cache.len() >= self.max_reads {
            return None;
        }
        let value = (self.reader)(key);
        cache.insert(key.to_string(), value.clone());
        value
    }

    /// The value at `key` as a small integer the symbolic engine can reason about; `None` when
    /// unknown or too large for the solver (it then stays opaque).
    pub(crate) fn read_small(&self, key: &str) -> Option<i128> {
        let hex = self.read_hex(key)?;
        let digits = hex.trim_start_matches("0x");
        if digits.len() > 30 {
            return None;
        }
        i128::from_str_radix(digits, 16).ok().filter(|v| *v <= i128::MAX / 4)
    }

    /// Every `(key, value)` the oracle has resolved so far.
    pub(crate) fn resolved(&self) -> Vec<(String, String)> {
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.iter().filter_map(|(k, v)| v.as_ref().map(|v| (k.clone(), v.clone()))).collect()
    }

    pub(crate) fn label(&self) -> &str {
        &self.label
    }
}

/// Lowercase `0x` hex of a felt, with no leading zeros, as JSON-RPC expects storage keys.
fn felt_key_hex(f: &starknet_types_core::felt::Felt) -> String {
    format!("{f:#x}")
}

/// One panic path of a function: its path constraints and what the in-process solver said.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PathObligation {
    /// `unsat` (panic unreachable), `sat` (witness found) or `unknown`.
    pub(crate) expected: &'static str,
    constraints: Vec<SymConstraint>,
    pub(crate) witness: Option<Vec<i128>>,
    pub(crate) reason: String,
}

fn smt_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

/// File stem for a function's obligations: the sanitized name, cut to 100 characters with a
/// hash suffix when longer (generic-heavy Sierra names exceed the filesystem limit).
fn smt_file_stem(function: &str) -> String {
    let full = smt_name(function);
    if full.len() <= 100 {
        return full;
    }
    let digest = blake3::hash(function.as_bytes()).to_hex();
    format!("{}__{}", &full[..100], &digest.as_str()[..12])
}

fn smt_int(v: i128) -> String {
    if v < 0 { format!("(- {})", v.unsigned_abs()) } else { v.to_string() }
}

/// Renders a symbolic expression in SMT-LIB 2 over `Int`. Opaque values (storage reads, call
/// results) become free constants collected in `opaque`, so they are unconstrained in the file,
/// exactly as the in-process solver treats them.
fn smt_expr(e: &SymExpr, opaque: &mut BTreeSet<String>) -> String {
    match e {
        SymExpr::Param(idx, _) => format!("p{idx}"),
        SymExpr::Const(v) => smt_int(*v),
        SymExpr::SymbolicOpaque(name) => {
            let n = format!("o_{}", smt_name(name));
            opaque.insert(n.clone());
            n
        }
        SymExpr::Add(a, b) => format!("(+ {} {})", smt_expr(a, opaque), smt_expr(b, opaque)),
        SymExpr::Sub(a, b) => format!("(- {} {})", smt_expr(a, opaque), smt_expr(b, opaque)),
        SymExpr::Mul(a, b) => format!("(* {} {})", smt_expr(a, opaque), smt_expr(b, opaque)),
        SymExpr::Div(a, b) => format!("(div {} {})", smt_expr(a, opaque), smt_expr(b, opaque)),
        SymExpr::Rem(a, b) => format!("(mod {} {})", smt_expr(a, opaque), smt_expr(b, opaque)),
        // QF_NIA has no bitwise operators: the result is a free constant named after the
        // expression, so equal expressions share it.
        SymExpr::BitAnd(..) | SymExpr::BitOr(..) | SymExpr::BitXor(..) => {
            let n = format!("o_bit_{}", smt_name(&e.format_human()));
            opaque.insert(n.clone());
            n
        }
    }
}

fn smt_constraint(c: &SymConstraint, opaque: &mut BTreeSet<String>) -> String {
    let l = smt_expr(&c.lhs, opaque);
    let r = smt_expr(&c.rhs, opaque);
    match c.op {
        CmpOp::Eq => format!("(= {l} {r})"),
        CmpOp::Ne => format!("(not (= {l} {r}))"),
        CmpOp::Lt => format!("(< {l} {r})"),
        CmpOp::Le => format!("(<= {l} {r})"),
        CmpOp::Gt => format!("(> {l} {r})"),
        CmpOp::Ge => format!("(>= {l} {r})"),
    }
}

/// Renders a function's proof obligations as one SMT-LIB 2 script. Each panic path becomes a
/// `(push 1) ... (check-sat) (pop 1)` block preceded by `; expected: <verdict>`; a `sat` path
/// gets a second block that pins the witness values, which must also be `sat`.
pub(crate) fn render_smt_obligations(proof: &SymbolicFunctionProofReport) -> String {
    let mut opaque = BTreeSet::new();
    let mut blocks = String::new();
    for (i, ob) in proof.obligations.iter().enumerate() {
        let asserts: Vec<String> = ob
            .constraints
            .iter()
            .map(|c| format!("  (assert {}) ; {}", smt_constraint(c, &mut opaque), c.reason.replace('\n', " ")))
            .collect();
        blocks.push_str(&format!("; obligation {i}: expected {}\n(push 1)\n", ob.expected));
        if asserts.is_empty() {
            blocks.push_str("  ; no path constraints\n");
        }
        blocks.push_str(&asserts.join("\n"));
        blocks.push_str("\n(check-sat)\n(pop 1)\n");
        if let Some(w) = &ob.witness {
            blocks.push_str(&format!("; obligation {i} witness: expected sat\n(push 1)\n"));
            blocks.push_str(&asserts.join("\n"));
            for (idx, v) in w.iter().enumerate() {
                blocks.push_str(&format!("\n  (assert (= p{idx} {}))", smt_int(*v)));
            }
            blocks.push_str("\n(check-sat)\n(pop 1)\n");
        }
    }
    let mut out = String::new();
    out.push_str(&format!("; uc prove obligations for {}\n", proof.function));
    out.push_str(&format!("; status: {}; panic paths: {}; unreachable: {}; undecided: {}\n",
        proof.status, proof.panic_paths, proof.unreachable_panic_paths, proof.undecided_panic_paths));
    out.push_str("; Integers model Cairo's unsigned types with their bounds asserted below; div and mod are\n");
    out.push_str("; SMT-LIB Euclidean operations, equal to Cairo's for non-negative operands. Opaque values\n");
    out.push_str("; (storage reads, call results, non-numeric data) are free constants.\n");
    out.push_str("(set-logic QF_NIA)\n");
    for (idx, (name, lo, hi)) in proof.param_bounds.iter().enumerate() {
        out.push_str(&format!("(declare-const p{idx} Int) ; {name}\n(assert (and (<= {} p{idx}) (<= p{idx} {})))\n", smt_int(*lo), smt_int(*hi)));
    }
    for o in &opaque {
        out.push_str(&format!("(declare-const {o} Int)\n"));
    }
    out.push_str(&blocks);
    out
}

/// Writes `<dir>/<function>.smt2` for every function with panic paths and `<dir>/obligations.json`
/// listing each obligation's expected verdict. Returns the number of files written.
pub(crate) fn write_smt_obligations(dir: &Path, proofs: &mut [SymbolicFunctionProofReport]) -> Result<usize> {
    fs::create_dir_all(dir)?;
    let mut manifest = Vec::new();
    let mut written = 0usize;
    for proof in proofs.iter_mut() {
        if proof.obligations.is_empty() {
            continue;
        }
        let file = format!("{}.smt2", smt_file_stem(&proof.function));
        fs::write(dir.join(&file), render_smt_obligations(proof))?;
        for (i, ob) in proof.obligations.iter().enumerate() {
            manifest.push(serde_json::json!({
                "function": proof.function,
                "status": proof.status,
                "file": file,
                "obligation": i,
                "expected": ob.expected,
                "reason": ob.reason,
                "witness": ob.witness,
            }));
        }
        proof.smt_file = Some(dir.join(&file).display().to_string());
        written += 1;
    }
    fs::write(dir.join("obligations.json"), serde_json::to_string_pretty(&manifest)?)?;
    Ok(written)
}

/// Encodes a witness as the calldata an entry point wrapper deserializes, in parameter order:
/// the contract state is skipped, `u256` becomes `(low, high)`, spans and arrays become empty
/// (`[0]`), a `ByteArray` becomes empty (`[0, 0, 0]`), everything else one felt. Negative
/// values wrap into the field.
pub(crate) fn witness_calldata(witness: &[i128], param_types: &[String]) -> Vec<String> {
    use starknet_types_core::felt::Felt;
    let mut out = Vec::new();
    for (i, v) in witness.iter().enumerate() {
        let ty = param_types.get(i).map(String::as_str).unwrap_or("");
        if ty.ends_with("::ContractState") {
            continue;
        }
        if ty.contains("core::array::Span::<") || ty.contains("core::array::Array::<") {
            out.push("0x0".to_string());
            continue;
        }
        if ty.ends_with("::ByteArray") {
            out.extend(["0x0", "0x0", "0x0"].map(str::to_string));
            continue;
        }
        let felt = if *v < 0 { Felt::ZERO - Felt::from(v.unsigned_abs()) } else { Felt::from(*v as u128) };
        out.push(format!("{felt:#x}"));
        if ty.ends_with("::u256") {
            out.push("0x0".to_string());
        }
    }
    out
}

/// Replays every counterexample in `proofs` on a runner built from the slice of `program`
/// reachable from the functions involved, so even a whole test crate's program (far too large
/// to assemble as one CASM program) gets its witnesses executed.
pub(crate) fn replay_counterexamples_on_slice(
    program: &Program,
    proofs: &mut [SymbolicFunctionProofReport],
    storage_seeds: &[(String, String)],
) {
    use cairo_lang_runner::{slice_program_for_roots, SierraCasmRunner};
    let mut roots = Vec::new();
    for proof in proofs.iter() {
        if proof.status != "COUNTEREXAMPLE_FOUND" || proof.function.contains('[') {
            continue;
        }
        if let Some(wrapper) = find_entry_point_wrapper(program, &proof.function) {
            roots.push(wrapper.id.clone());
        } else if let Some(f) = program.funcs.iter().find(|f| {
            f.id.debug_name.as_ref().is_some_and(|dn| dn.as_str() == proof.function)
        }) {
            roots.push(f.id.clone());
        }
    }
    if roots.is_empty() {
        return;
    }
    let debug = std::env::var("UC_PROVE_DEBUG_REPLAY").is_ok();
    let t0 = std::time::Instant::now();
    let (sliced, _) = slice_program_for_roots(program, &roots);
    if debug {
        eprintln!(
            "replay slice: {} of {} statements, {} roots, {:.1} ms",
            sliced.statements.len(),
            program.statements.len(),
            roots.len(),
            t0.elapsed().as_secs_f64() * 1000.0
        );
    }
    // No gas equations: Sierra gas checks are off and syscalls draw on the replay budget.
    let t1 = std::time::Instant::now();
    if let Ok(runner) = SierraCasmRunner::new(sliced.clone(), None, Default::default(), None) {
        if debug {
            eprintln!("replay runner built in {:.1} ms", t1.elapsed().as_secs_f64() * 1000.0);
        }
        let t2 = std::time::Instant::now();
        verify_counterexamples_concretely(&runner, &sliced, proofs, storage_seeds);
        if debug {
            eprintln!("replay ran in {:.1} ms", t2.elapsed().as_secs_f64() * 1000.0);
        }
    }
}

/// Whether a concrete panic confirms the symbolic witness: harness artefacts (malformed
/// calldata, missing contracts, gas) never do, and a predicted arithmetic failure must surface
/// as one (`Overflow`, `Division by 0`, ...), not as an unrelated assertion the path never
/// reasoned about.
fn concrete_panic_confirms_witness(predicted_reason: &str, panic_message: &str) -> bool {
    let harness_artefact = [
        "Failed to deserialize param",
        "Input too long for arguments",
        "CONTRACT_NOT_DEPLOYED",
        "ENTRYPOINT_NOT_FOUND",
        "out of gas",
        "Out of gas",
    ]
    .iter()
    .any(|needle| panic_message.contains(needle));
    if harness_artefact {
        return false;
    }
    let predicted = predicted_reason.to_ascii_lowercase();
    if predicted.contains("overflow") || predicted.contains("underflow") {
        return panic_message.contains("Overflow") || panic_message.contains("overflow");
    }
    if predicted.contains("division") || predicted.contains("div") || predicted.contains("_is_zero") {
        return panic_message.contains("Division by 0")
            || panic_message.contains("Overflow")
            || panic_message.contains("is_zero");
    }
    true
}

/// Finds the entry point wrapper (`__wrapper__<Impl>__<name>` or `__wrapper__<name>`) generated
/// for a contract method, if the function is one.
fn find_entry_point_wrapper<'p>(program: &'p Program, function: &str) -> Option<&'p cairo_lang_sierra::program::Function> {
    let (module, rest) = function.rsplit_once("::")?;
    let short = rest;
    let (contract_module, impl_name) = module.rsplit_once("::")?;
    let candidates = [
        format!("{contract_module}::__wrapper__{impl_name}__{short}"),
        format!("{contract_module}::__wrapper__{short}"),
        format!("{module}::__wrapper__{short}"),
    ];
    program.funcs.iter().find(|f| {
        f.id.debug_name.as_ref().is_some_and(|dn| candidates.iter().any(|c| c == dn.as_str()))
    })
}

/// Whether some generated entry point wrapper (`__wrapper__<name>`, `__wrapper__<Impl>__<name>`,
/// `__external::<name>`, `__constructor::<name>`, `__l1_handler::<name>`) exists for `short_name`,
/// meaning the function is reached from calldata.
fn has_entry_point_wrapper(index: &SierraProgramIndex<'_>, short_name: &str) -> bool {
    let suffix = format!("__{short_name}");
    (0..index.program.funcs.len()).any(|other_idx| {
        let other = index.func_name(other_idx);
        other.ends_with(&format!("::__external::{short_name}"))
            || other.ends_with(&format!("::__constructor::{short_name}"))
            || other.ends_with(&format!("::__l1_handler::{short_name}"))
            || (other.contains("::__wrapper__") && other.ends_with(&suffix))
    })
}

/// Follows concrete storage base addresses through the libfuncs that build and copy them, for
/// the detectors' linear walk (no path state): `storage_base_address_const` / `const_as_immediate`
/// produce one, `storage_address_from_base[_and_offset]`, copies and `StoragePointer` struct
/// construction and deconstruction keep it. Offsets are ignored: the base identifies the field.
fn track_detector_storage_slot(
    slot_of_var: &mut HashMap<u64, starknet_types_core::felt::Felt>,
    gid: &str,
    lf: &LibfuncMeta,
    inv: &cairo_lang_sierra::program::Invocation,
) {
    use starknet_types_core::felt::Felt;
    if inv.branches.len() != 1 {
        return;
    }
    let branch = &inv.branches[0];
    let Some(res) = branch.results.first() else { return };
    if gid == "storage_base_address_const" || gid == "const_as_immediate" {
        let value = lf
            .big_const_hex
            .as_deref()
            .and_then(|h| Felt::from_hex(&format!("0x{h}")).ok())
            .or_else(|| lf.const_value.filter(|v| *v >= 0).map(|v| Felt::from(v as u128)));
        if let Some(addr) = value {
            slot_of_var.insert(res.id, addr);
        }
    } else if matches!(
        gid,
        "storage_address_from_base" | "storage_address_from_base_and_offset" | "rename" | "dup" | "store_temp" | "store_local" | "snapshot_take" | "unbox"
    ) {
        if let Some(arg) = inv.args.first() {
            if let Some(addr) = slot_of_var.get(&arg.id).copied() {
                for r in &branch.results {
                    slot_of_var.insert(r.id, addr);
                }
            }
        }
    } else if gid == "struct_construct" || gid == "struct_deconstruct" {
        if let Some(first) = inv.args.first() {
            if let Some(addr) = slot_of_var.get(&first.id).copied() {
                slot_of_var.insert(res.id, addr);
            }
        }
    }
}

/// Builtin pointer types (`RangeCheck`, `GasBuiltin`, `System`, ...) that libfuncs thread through
/// every call. They carry no caller data, so taint must not ride on them.
fn is_builtin_sierra_type(ty_name: &str) -> bool {
    ty_name.contains("GasBuiltin")
        || ty_name.contains("System")
        || ty_name.contains("RangeCheck")
        || ty_name.contains("Bitwise")
        || ty_name.contains("Pedersen")
        || ty_name.contains("Poseidon")
        || ty_name.contains("SegmentArena")
        || ty_name.contains("EcOp")
        || ty_name.contains("AddMod")
        || ty_name.contains("MulMod")
}

/// Libfuncs return threaded builtins first, in the order they were passed: the leading
/// builtin arguments map onto the leading results of every branch.
fn mark_threaded_builtin_results(
    builtin_vars: &mut HashSet<u64>,
    inv: &cairo_lang_sierra::program::GenInvocation<cairo_lang_sierra::program::StatementIdx>,
) {
    let leading = inv.args.iter().take_while(|a| builtin_vars.contains(&a.id)).count();
    if leading == 0 {
        return;
    }
    for b in &inv.branches {
        for r in b.results.iter().take(leading) {
            builtin_vars.insert(r.id);
        }
    }
}

/// Integer-like Sierra types whose value a caller can pick freely (loop bounds, limits).
fn is_numeric_sierra_type(ty_name: &str) -> bool {
    let t = ty_name.rsplit("::").next().unwrap_or(ty_name);
    matches!(
        t,
        "u8" | "u16" | "u32" | "u64" | "u128" | "u256" | "usize" | "felt252" | "i8" | "i16"
            | "i32" | "i64" | "i128"
    )
}

fn max_bound_for_sierra_type(ty_name: &str) -> i128 {
    if ty_name.ends_with("bool") {
        1
    } else if ty_name.ends_with("u8") {
        u8::MAX as i128
    } else if ty_name.ends_with("u16") {
        u16::MAX as i128
    } else if ty_name.ends_with("u32") {
        u32::MAX as i128
    } else if ty_name.ends_with("u64") {
        u64::MAX as i128
    } else {
        i128::MAX / 4
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SolverOutcome {
    Unsat,
    Sat(Vec<i128>),
    Unknown,
}

/// In-process Interval + Relational Algebraic Constraint Solver over Sierra path conditions.
/// `Σ coefficient · parameter + constant` when `e` is linear in the parameters (sums,
/// differences and products by constants of parameters and constants).
fn linear_form(e: &SymExpr) -> Option<(BTreeMap<usize, i128>, i128)> {
    match e {
        SymExpr::Param(i, _) => Some((BTreeMap::from([(*i, 1)]), 0)),
        SymExpr::Const(v) => Some((BTreeMap::new(), *v)),
        SymExpr::Add(a, b) | SymExpr::Sub(a, b) => {
            let sign: i128 = if matches!(e, SymExpr::Sub(..)) { -1 } else { 1 };
            let (mut coeffs, ka) = linear_form(a)?;
            let (cb, kb) = linear_form(b)?;
            for (i, c) in cb {
                let slot = coeffs.entry(i).or_insert(0);
                *slot = slot.checked_add(sign.checked_mul(c)?)?;
            }
            coeffs.retain(|_, c| *c != 0);
            Some((coeffs, ka.checked_add(sign.checked_mul(kb)?)?))
        }
        SymExpr::Mul(a, b) => {
            let ((ca, ka), (cb, kb)) = (linear_form(a)?, linear_form(b)?);
            let (coeffs, k, factor) = match (ca.is_empty(), cb.is_empty()) {
                (_, true) => (ca, ka, kb),
                (true, false) => (cb, kb, ka),
                (false, false) => return None,
            };
            let scaled: Option<BTreeMap<usize, i128>> = coeffs.into_iter().map(|(i, c)| c.checked_mul(factor).map(|v| (i, v))).collect();
            let mut scaled = scaled?;
            scaled.retain(|_, c| *c != 0);
            Some((scaled, k.checked_mul(factor)?))
        }
        _ => None,
    }
}

/// `lhs - rhs` of a constraint as a linear form, so the constraint reads `form op 0`.
fn constraint_linear_form(c: &SymConstraint) -> Option<(BTreeMap<usize, i128>, i128)> {
    let (mut coeffs, kl) = linear_form(&c.lhs)?;
    let (cr, kr) = linear_form(&c.rhs)?;
    for (i, v) in cr {
        let slot = coeffs.entry(i).or_insert(0);
        *slot = slot.checked_sub(v)?;
    }
    coeffs.retain(|_, v| *v != 0);
    Some((coeffs, kl.checked_sub(kr)?))
}

/// `c · p + k op 0` as a bound on `p`: `(op', b)` meaning `p op' b` over the integers, with
/// the division rounded the way `op'` needs. `None` for `c = 0` or an overflow.
fn linear_bound(coeff: i128, k: i128, op: CmpOp) -> Option<(CmpOp, i128)> {
    // Make the coefficient positive: `-c·p - k op~ 0` with the comparison mirrored.
    let (c, k, op) = if coeff < 0 {
        let mirrored = match op {
            CmpOp::Lt => CmpOp::Gt,
            CmpOp::Le => CmpOp::Ge,
            CmpOp::Gt => CmpOp::Lt,
            CmpOp::Ge => CmpOp::Le,
            other => other,
        };
        (coeff.checked_neg()?, k.checked_neg()?, mirrored)
    } else if coeff > 0 {
        (coeff, k, op)
    } else {
        return None;
    };
    let target = k.checked_neg()?; // c·p op target
    let floor = |a: i128| a.div_euclid(c);
    let ceil = |a: i128| a.checked_neg().map(|n| -n.div_euclid(c));
    Some(match op {
        CmpOp::Lt => (CmpOp::Le, floor(target.checked_sub(1)?)),
        CmpOp::Le => (CmpOp::Le, floor(target)),
        CmpOp::Gt => (CmpOp::Ge, ceil(target.checked_add(1)?)?),
        CmpOp::Ge => (CmpOp::Ge, ceil(target)?),
        // An equality without an integer solution has an empty range: `p >= b+1` and `p <= b`.
        CmpOp::Eq if target.rem_euclid(c) != 0 => (CmpOp::Lt, i128::MIN),
        CmpOp::Eq => (CmpOp::Eq, target / c),
        CmpOp::Ne if target.rem_euclid(c) != 0 => return None,
        CmpOp::Ne => (CmpOp::Ne, target / c),
    })
}

/// Interval of the values `e` takes when each parameter ranges over `[lo, hi]`; `None` when
/// unknown (opaque values) or out of `i128`. An over-approximation, so a constraint that fails
/// on the whole interval fails everywhere.
fn expr_interval(e: &SymExpr, lo: &[i128], hi: &[i128]) -> Option<(i128, i128)> {
    fn pow2_above(v: i128) -> Option<i128> {
        let bits = 128 - (v.max(0) as u128).leading_zeros();
        if bits >= 127 { None } else { Some((1_i128 << bits) - 1) }
    }
    Some(match e {
        SymExpr::Param(i, _) => (*lo.get(*i)?, *hi.get(*i)?),
        SymExpr::Const(v) => (*v, *v),
        SymExpr::SymbolicOpaque(_) => return None,
        SymExpr::Add(a, b) => {
            let ((al, ah), (bl, bh)) = (expr_interval(a, lo, hi)?, expr_interval(b, lo, hi)?);
            (al.checked_add(bl)?, ah.checked_add(bh)?)
        }
        SymExpr::Sub(a, b) => {
            let ((al, ah), (bl, bh)) = (expr_interval(a, lo, hi)?, expr_interval(b, lo, hi)?);
            (al.checked_sub(bh)?, ah.checked_sub(bl)?)
        }
        SymExpr::Mul(a, b) => {
            let ((al, ah), (bl, bh)) = (expr_interval(a, lo, hi)?, expr_interval(b, lo, hi)?);
            let products = [al.checked_mul(bl)?, al.checked_mul(bh)?, ah.checked_mul(bl)?, ah.checked_mul(bh)?];
            (*products.iter().min()?, *products.iter().max()?)
        }
        SymExpr::Div(a, b) => {
            let ((al, ah), (bl, bh)) = (expr_interval(a, lo, hi)?, expr_interval(b, lo, hi)?);
            if al < 0 || bh < 1 {
                return None;
            }
            (al / bh, ah / bl.max(1))
        }
        SymExpr::Rem(a, b) => {
            let ((al, ah), (_, bh)) = (expr_interval(a, lo, hi)?, expr_interval(b, lo, hi)?);
            if al < 0 || bh < 1 {
                return None;
            }
            (0, ah.min(bh - 1))
        }
        SymExpr::BitAnd(a, b) => {
            let ((al, ah), (bl, bh)) = (expr_interval(a, lo, hi)?, expr_interval(b, lo, hi)?);
            if al < 0 || bl < 0 {
                return None;
            }
            (0, ah.min(bh))
        }
        SymExpr::BitOr(a, b) | SymExpr::BitXor(a, b) => {
            let ((al, ah), (bl, bh)) = (expr_interval(a, lo, hi)?, expr_interval(b, lo, hi)?);
            if al < 0 || bl < 0 {
                return None;
            }
            let low = if matches!(e, SymExpr::BitOr(..)) { al.max(bl) } else { 0 };
            (low, pow2_above(ah.max(bh))?)
        }
    })
}

/// True when interval arithmetic shows `c` cannot hold for any parameters in `[lo, hi]`.
fn interval_refutes(c: &SymConstraint, lo: &[i128], hi: &[i128]) -> bool {
    let (Some((ll, lh)), Some((rl, rh))) = (expr_interval(&c.lhs, lo, hi), expr_interval(&c.rhs, lo, hi)) else {
        return false;
    };
    let (Some(dl), Some(dh)) = (ll.checked_sub(rh), lh.checked_sub(rl)) else {
        return false;
    };
    match c.op {
        CmpOp::Eq => dl > 0 || dh < 0,
        CmpOp::Ne => dl == 0 && dh == 0,
        CmpOp::Lt => dl >= 0,
        CmpOp::Le => dl > 0,
        CmpOp::Gt => dh <= 0,
        CmpOp::Ge => dh < 0,
    }
}

fn solve_path_constraints(
    param_bounds: &[(String, i128, i128)],
    constraints: &[SymConstraint],
) -> SolverOutcome {
    if constraints.is_empty() {
        let zeros = param_bounds.iter().map(|(_, lo, _)| *lo).collect();
        return SolverOutcome::Sat(zeros);
    }

    // Check for constant contradictions first (even on 0-param functions!)
    for c in constraints {
        if let (Some(l), Some(r)) = (c.lhs.eval(&[]), c.rhs.eval(&[])) {
            let ok = match c.op {
                CmpOp::Eq => l == r,
                CmpOp::Ne => l != r,
                CmpOp::Lt => l < r,
                CmpOp::Le => l <= r,
                CmpOp::Gt => l > r,
                CmpOp::Ge => l >= r,
            };
            if !ok {
                return SolverOutcome::Unsat;
            }
        }
        // Algebraic identity contradictions: `X != X` or `X < X` or `X > X`
        if c.lhs == c.rhs && matches!(c.op, CmpOp::Ne | CmpOp::Lt | CmpOp::Gt) {
            return SolverOutcome::Unsat;
        }
    }

    // Check pairwise contradictions: `A == B` and `A != B`, or `A < B` and `A >= B`
    let pairwise_n = if constraints.len() > PAIRWISE_SCAN_LIMIT { 0 } else { constraints.len() };
    for i in 0..pairwise_n {
        for j in (i + 1)..pairwise_n {
            let ci = &constraints[i];
            let cj = &constraints[j];
            if ci.lhs == cj.lhs && ci.rhs == cj.rhs {
                let conflict = matches!(
                    (ci.op, cj.op),
                    (CmpOp::Eq, CmpOp::Ne)
                        | (CmpOp::Ne, CmpOp::Eq)
                        | (CmpOp::Lt, CmpOp::Ge)
                        | (CmpOp::Ge, CmpOp::Lt)
                        | (CmpOp::Lt, CmpOp::Eq)
                        | (CmpOp::Eq, CmpOp::Lt)
                        | (CmpOp::Gt, CmpOp::Le)
                        | (CmpOp::Le, CmpOp::Gt)
                        | (CmpOp::Gt, CmpOp::Eq)
                        | (CmpOp::Eq, CmpOp::Gt)
                );
                if conflict {
                    return SolverOutcome::Unsat;
                }
            }
        }
    }

    if param_bounds.is_empty() {
        return SolverOutcome::Sat(Vec::new());
    }

    let n = param_bounds.len();
    let mut lo: Vec<i128> = param_bounds.iter().map(|(_, l, _)| *l).collect();
    let mut hi: Vec<i128> = param_bounds.iter().map(|(_, _, h)| *h).collect();
    let mut excluded: Vec<HashSet<i128>> = vec![HashSet::new(); n];
    let mut candidate_constants: Vec<i128> = vec![0, 1, 2, 10, 42, 75, 100, 1337, 10_000, 10_001];

    // Iterative interval tightening
    for _iter in 0..8 {
        for c in constraints {
            // `c·p + k op 0`, however the sums are nested: a bound on `p` and its boundary values.
            if let Some((coeffs, k)) = constraint_linear_form(c) {
                if let (1, Some((&idx, &coeff))) = (coeffs.len(), coeffs.iter().next()) {
                    if let Some((op, b)) = linear_bound(coeff, k, c.op) {
                        if b != i128::MIN {
                            candidate_constants.extend([b.saturating_sub(1), b, b.saturating_add(1)]);
                        }
                        match op {
                            CmpOp::Eq => {
                                lo[idx] = lo[idx].max(b);
                                hi[idx] = hi[idx].min(b);
                            }
                            CmpOp::Ne => {
                                excluded[idx].insert(b);
                            }
                            CmpOp::Lt => hi[idx] = hi[idx].min(b.saturating_sub(1)),
                            CmpOp::Le => hi[idx] = hi[idx].min(b),
                            CmpOp::Gt => lo[idx] = lo[idx].max(b.saturating_add(1)),
                            CmpOp::Ge => lo[idx] = lo[idx].max(b),
                        }
                        continue;
                    }
                }
            }
            match (&c.lhs, c.op, &c.rhs) {
                (SymExpr::Param(idx, _), op, SymExpr::Const(val)) => {
                    candidate_constants.extend([val.saturating_sub(1), *val, val.saturating_add(1)]);
                    match op {
                        CmpOp::Eq => {
                            lo[*idx] = lo[*idx].max(*val);
                            hi[*idx] = hi[*idx].min(*val);
                        }
                        CmpOp::Ne => {
                            excluded[*idx].insert(*val);
                            if lo[*idx] == *val {
                                lo[*idx] += 1;
                            }
                            if hi[*idx] == *val {
                                hi[*idx] -= 1;
                            }
                        }
                        CmpOp::Lt => hi[*idx] = hi[*idx].min(*val - 1),
                        CmpOp::Le => hi[*idx] = hi[*idx].min(*val),
                        CmpOp::Gt => lo[*idx] = lo[*idx].max(*val + 1),
                        CmpOp::Ge => lo[*idx] = lo[*idx].max(*val),
                    }
                }
                (SymExpr::Const(val), op, SymExpr::Param(idx, _)) => {
                    candidate_constants.extend([val.saturating_sub(1), *val, val.saturating_add(1)]);
                    match op {
                        CmpOp::Eq => {
                            lo[*idx] = lo[*idx].max(*val);
                            hi[*idx] = hi[*idx].min(*val);
                        }
                        CmpOp::Ne => {
                            excluded[*idx].insert(*val);
                        }
                        CmpOp::Lt => lo[*idx] = lo[*idx].max(*val + 1),
                        CmpOp::Le => lo[*idx] = lo[*idx].max(*val),
                        CmpOp::Gt => hi[*idx] = hi[*idx].min(*val - 1),
                        CmpOp::Ge => hi[*idx] = hi[*idx].min(*val),
                    }
                }
                (SymExpr::Sub(a, b), CmpOp::Eq, SymExpr::Const(0))
                | (SymExpr::Sub(a, b), CmpOp::Ne, SymExpr::Const(0)) => {
                    let is_eq = c.op == CmpOp::Eq;
                    match (a.as_ref(), b.as_ref()) {
                        (SymExpr::Param(idx, _), SymExpr::Const(val))
                        | (SymExpr::Const(val), SymExpr::Param(idx, _)) => {
                            candidate_constants.extend([
                                val.saturating_sub(1),
                                *val,
                                val.saturating_add(1),
                            ]);
                            if is_eq {
                                lo[*idx] = lo[*idx].max(*val);
                                hi[*idx] = hi[*idx].min(*val);
                            } else {
                                excluded[*idx].insert(*val);
                            }
                        }
                        (SymExpr::Param(i, _), SymExpr::Param(j, _)) if is_eq => {
                            let new_lo = lo[*i].max(lo[*j]);
                            let new_hi = hi[*i].min(hi[*j]);
                            lo[*i] = new_lo;
                            lo[*j] = new_lo;
                            hi[*i] = new_hi;
                            hi[*j] = new_hi;
                        }
                        _ => {}
                    }
                }
                (SymExpr::Param(i, _), op, SymExpr::Param(j, _)) => match op {
                    CmpOp::Eq => {
                        let new_lo = lo[*i].max(lo[*j]);
                        let new_hi = hi[*i].min(hi[*j]);
                        lo[*i] = new_lo;
                        lo[*j] = new_lo;
                        hi[*i] = new_hi;
                        hi[*j] = new_hi;
                    }
                    CmpOp::Lt => {
                        hi[*i] = hi[*i].min(hi[*j].saturating_sub(1));
                        lo[*j] = lo[*j].max(lo[*i].saturating_add(1));
                    }
                    CmpOp::Le => {
                        hi[*i] = hi[*i].min(hi[*j]);
                        lo[*j] = lo[*j].max(lo[*i]);
                    }
                    CmpOp::Gt => {
                        lo[*i] = lo[*i].max(lo[*j].saturating_add(1));
                        hi[*j] = hi[*j].min(hi[*i].saturating_sub(1));
                    }
                    CmpOp::Ge => {
                        lo[*i] = lo[*i].max(lo[*j]);
                        hi[*j] = hi[*j].min(hi[*i]);
                    }
                    CmpOp::Ne => {}
                },
                (SymExpr::Add(a, b), op, SymExpr::Const(target)) => {
                    candidate_constants.push(*target);
                    if let (SymExpr::Param(i, _), SymExpr::Param(j, _)) = (a.as_ref(), b.as_ref()) {
                        match op {
                            CmpOp::Eq => {
                                lo[*i] = lo[*i].max(*target - hi[*j]);
                                hi[*i] = hi[*i].min(*target - lo[*j]);
                                lo[*j] = lo[*j].max(*target - hi[*i]);
                                hi[*j] = hi[*j].min(*target - lo[*i]);
                            }
                            CmpOp::Gt => {
                                lo[*i] = lo[*i].max((*target + 1).saturating_sub(hi[*j]));
                                lo[*j] = lo[*j].max((*target + 1).saturating_sub(hi[*i]));
                            }
                            _ => {}
                        }
                    } else if let (SymExpr::Param(i, _), SymExpr::Const(k))
                    | (SymExpr::Const(k), SymExpr::Param(i, _)) =
                        (a.as_ref(), b.as_ref())
                    {
                        let needed = target.saturating_sub(*k);
                        candidate_constants.extend([needed - 1, needed, needed + 1]);
                        match op {
                            CmpOp::Eq => {
                                lo[*i] = lo[*i].max(needed);
                                hi[*i] = hi[*i].min(needed);
                            }
                            CmpOp::Ge => {
                                lo[*i] = lo[*i].max(needed);
                            }
                            CmpOp::Gt => {
                                lo[*i] = lo[*i].max(needed + 1);
                            }
                            CmpOp::Le => {
                                hi[*i] = hi[*i].min(needed);
                            }
                            CmpOp::Lt => {
                                hi[*i] = hi[*i].min(needed - 1);
                            }
                            CmpOp::Ne => {
                                excluded[*i].insert(needed);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        for idx in 0..n {
            while lo[idx] <= hi[idx] && excluded[idx].contains(&lo[idx]) {
                lo[idx] += 1;
            }
            while hi[idx] >= lo[idx] && excluded[idx].contains(&hi[idx]) {
                hi[idx] -= 1;
            }
            if lo[idx] > hi[idx] {
                return SolverOutcome::Unsat;
            }
        }
    }

    if constraints.iter().any(|c| interval_refutes(c, &lo, &hi)) {
        return SolverOutcome::Unsat;
    }

    // Build per-parameter candidate witness sets from solved interval boundaries & algebraic
    // relations. A parameter no constraint mentions gets one candidate, so it does not multiply
    // the search.
    let mut mentioned = vec![false; n];
    for c in constraints {
        c.lhs.mark_params(&mut mentioned);
        c.rhs.mark_params(&mut mentioned);
    }
    let mut per_param_candidates: Vec<Vec<i128>> = Vec::with_capacity(n);
    for idx in 0..n {
        if !mentioned[idx] {
            per_param_candidates.push(vec![lo[idx]]);
            continue;
        }
        let mut pts = BTreeSet::new();
        pts.insert(lo[idx]);
        pts.insert(hi[idx]);
        if lo[idx] < hi[idx] {
            pts.insert(lo[idx] + 1);
            pts.insert(hi[idx] - 1);
            pts.insert(lo[idx] + (hi[idx] - lo[idx]) / 2);
        }
        for &c in &candidate_constants {
            if c >= lo[idx] && c <= hi[idx] {
                pts.insert(c);
            }
        }
        pts.retain(|v| !excluded[idx].contains(v));
        per_param_candidates.push(pts.into_iter().collect());
    }

    // Search for a concrete witness satisfying all constraints. Constraints are checked as soon
    // as every parameter they mention is bound, and the search gives up (Unknown) after a fixed
    // number of partial assignments rather than enumerating 24^n combinations.
    let highest_param: Vec<usize> = constraints
        .iter()
        .map(|c| c.lhs.highest_param().max(c.rhs.highest_param()))
        .collect();
    let mut env = vec![0i128; n];
    // Each partial assignment re-evaluates the constraints bound so far, so the budget shrinks
    // with the path length: a path of a thousand constraints gets a few thousand assignments.
    let mut budget = WITNESS_SEARCH_BUDGET.min(WITNESS_EVALUATION_BUDGET / constraints.len().max(1));
    if search_witness_recursive(
        0,
        &mut env,
        &per_param_candidates,
        &lo,
        &hi,
        constraints,
        &highest_param,
        &mut budget,
    ) {
        return SolverOutcome::Sat(env);
    }

    SolverOutcome::Unknown
}

/// Symbolic expressions are trees without sharing, so `x = x + x` repeated doubles the tree
/// every step. Past this many nodes a result is kept opaque instead; the solver then treats it
/// as unknown rather than spending seconds walking one expression.
const MAX_SYM_EXPR_NODES: usize = 96;

fn cap_sym_expr(expr: SymExpr, var_id: u64) -> SymExpr {
    if !expr.exceeds(MAX_SYM_EXPR_NODES) {
        expr
    } else if expr.mentions_param() || expr.mentions_unknown() {
        // Still a function of the parameters: an unknown one, never an environment value.
        opaque_var(UNKNOWN_PREFIX, var_id)
    } else {
        opaque_var("v", var_id)
    }
}

thread_local! {
    /// Serial of the inlined call whose statement is being processed (0 for the analysed
    /// function). Variable ids repeat in every inlined call, so opaque values are named
    /// `<prefix><id>@<serial>` there: equal ids in different calls stay different unknowns,
    /// for the solver and in the SMT-LIB export.
    static FRAME_SERIAL: Cell<u32> = const { Cell::new(0) };
}

fn opaque_var(prefix: &str, id: u64) -> SymExpr {
    let serial = FRAME_SERIAL.with(Cell::get);
    SymExpr::SymbolicOpaque(if serial == 0 { format!("{prefix}{id}") } else { format!("{prefix}{id}@{serial}") })
}

/// Whether panic-path obligations are kept on the reports (only `--emit-smt` needs them; for a
/// large class they are gigabytes of cloned constraints).
pub(crate) static KEEP_OBLIGATIONS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Wall-clock budget per analysed function (set from `--function-timeout-ms`).
pub(crate) static FUNCTION_TIMEOUT_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(15_000);

/// Past this many constraints on one path the pairwise contradiction scan (quadratic) is
/// skipped; interval tightening and the witness search still run.
const PAIRWISE_SCAN_LIMIT: usize = 400;

/// Partial assignments the witness search may try per panic path before answering Unknown.
const WITNESS_SEARCH_BUDGET: usize = 20_000;
/// Upper bound on `assignments * constraints` per panic path (about a second of solving).
const WITNESS_EVALUATION_BUDGET: usize = 4_000_000;

impl SymExpr {
    /// Highest parameter index the expression mentions, or `usize::MAX` when it mentions none
    /// (such constraints are checked only once every parameter is bound).
    fn highest_param(&self) -> usize {
        match self {
            Self::Param(idx, _) => *idx,
            Self::Const(_) | Self::SymbolicOpaque(_) => usize::MAX,
            Self::Add(a, b)
            | Self::Sub(a, b)
            | Self::Mul(a, b)
            | Self::Div(a, b)
            | Self::Rem(a, b)
            | Self::BitAnd(a, b)
            | Self::BitOr(a, b)
            | Self::BitXor(a, b) => {
                let (x, y) = (a.highest_param(), b.highest_param());
                match (x == usize::MAX, y == usize::MAX) {
                    (true, true) => usize::MAX,
                    (true, false) => y,
                    (false, true) => x,
                    (false, false) => x.max(y),
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn search_witness_recursive(
    param_idx: usize,
    env: &mut [i128],
    candidates: &[Vec<i128>],
    lo: &[i128],
    hi: &[i128],
    constraints: &[SymConstraint],
    highest_param: &[usize],
    budget: &mut usize,
) -> bool {
    let n = env.len();
    if param_idx == n {
        return constraints
            .iter()
            .all(|c| c.eval(env).unwrap_or(true));
    }

    // Also derive relational values from already-bound parameters (`x + y == target`, `x == y`, etc.)
    let mut dynamic_pts = candidates[param_idx].clone();
    for c in constraints {
        // A linear constraint whose other parameters are bound pins this one's boundary.
        if let Some((coeffs, k)) = constraint_linear_form(c) {
            if let Some(&coeff) = coeffs.get(&param_idx) {
                if coeffs.keys().all(|&i| i <= param_idx) {
                    let rest = coeffs
                        .iter()
                        .filter(|(&i, _)| i != param_idx)
                        .try_fold(k, |acc, (&i, &ci)| acc.checked_add(ci.checked_mul(env[i])?));
                    if let Some((_, b)) = rest.and_then(|r| linear_bound(coeff, r, c.op)).filter(|(_, b)| *b != i128::MIN) {
                        for v in [b.saturating_sub(1), b, b.saturating_add(1)] {
                            if v >= lo[param_idx] && v <= hi[param_idx] {
                                dynamic_pts.insert(0, v);
                            }
                        }
                    }
                }
            }
        }
        if let (SymExpr::Add(a, b), op, SymExpr::Const(target)) = (&c.lhs, c.op, &c.rhs) {
            if let (SymExpr::Param(i, _), SymExpr::Param(j, _)) = (a.as_ref(), b.as_ref()) {
                let other = if *i == param_idx && *j < param_idx {
                    Some(env[*j])
                } else if *j == param_idx && *i < param_idx {
                    Some(env[*i])
                } else {
                    None
                };
                if let Some(other_val) = other {
                    let exact = target.saturating_sub(other_val);
                    let wanted = if op == CmpOp::Gt { exact + 1 } else { exact };
                    if wanted >= lo[param_idx] && wanted <= hi[param_idx] {
                        dynamic_pts.insert(0, wanted);
                    }
                }
            }
        }
        if let (SymExpr::Param(i, _), _, SymExpr::Param(j, _)) = (&c.lhs, c.op, &c.rhs) {
            let other = if *i == param_idx && *j < param_idx {
                Some(env[*j])
            } else if *j == param_idx && *i < param_idx {
                Some(env[*i])
            } else {
                None
            };
            if let Some(other_val) = other {
                for delta in [-1i128, 0, 1] {
                    let v = other_val.saturating_add(delta);
                    if v >= lo[param_idx] && v <= hi[param_idx] {
                        dynamic_pts.insert(0, v);
                    }
                }
            }
        }
    }

    for val in dynamic_pts.into_iter().take(24) {
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        env[param_idx] = val;
        // Prune: a constraint whose parameters are all bound now must already hold.
        let violated = constraints.iter().zip(highest_param).any(|(c, hp)| {
            *hp == param_idx && c.eval(env) == Some(false)
        });
        if violated {
            continue;
        }
        if search_witness_recursive(param_idx + 1, env, candidates, lo, hi, constraints, highest_param, budget) {
            return true;
        }
    }
    false
}

#[derive(Debug, Clone)]
struct SymPathState {
    stmt_idx: usize,
    depth: usize,
    /// Function whose body `stmt_idx` is in (the root, or an inlined callee).
    cur_func: usize,
    /// Callers of the inlined functions on this path, innermost last.
    frames: Vec<Rc<CallFrame>>,
    /// Struct and enum values (`struct_construct`, `enum_init`), so values cross calls and
    /// `enum_match` on a known variant follows one branch.
    comps: HashMap<u64, CompVal>,
    /// The path went through the `Err` branch of a callee that was not inlined: its panic
    /// condition is unknown, so a panic on this path is undecided, never a counterexample.
    unknown_panic: bool,
    /// Serial of the inlined call the path is in (see `FRAME_SERIAL`), and the last one issued.
    frame_serial: u32,
    next_serial: u32,
    /// Where `unknown_panic` was set (libfunc and statement), for `UC_PROVE_DEBUG_PATHS`.
    unknown_reason: Option<Rc<str>>,
    /// The path is the `Err` branch of a callee whose own verdict is guarded: it panics only
    /// behind the callee's guards, a precondition, never a counterexample.
    guarded_callee: bool,
    /// The path took a branch whose condition the engine does not express (an array bound, an
    /// enum read from data): what panics behind it depends on that data.
    unmodeled_branch: bool,
    vars: HashMap<u64, SymExpr>,
    /// Concrete storage addresses flowing through `storage_base_address_const`,
    /// `storage_address_from_base[_and_offset]` and copies, for the storage oracle.
    storage_addrs: HashMap<u64, starknet_types_core::felt::Felt>,
    /// Upper bound of the integer type a variable carries (`u8` 255, `u64` 2^64-1, ...), for
    /// the bounded-int lowering of checked arithmetic (`bounded_int_trim_max` and friends).
    var_max: HashMap<u64, i128>,
    constraints: Vec<SymConstraint>,
    performed_computation: bool,
}

/// Follows concrete storage addresses through the libfuncs that build and copy them.
fn track_storage_address(
    state: &mut SymPathState,
    gid: &str,
    lf: &LibfuncMeta,
    inv: &cairo_lang_sierra::program::Invocation,
    branch: &cairo_lang_sierra::program::BranchInfo,
) {
    use starknet_types_core::felt::Felt;
    let Some(res) = branch.results.first() else { return };
    if gid == "storage_base_address_const" || gid == "const_as_immediate" {
        let value = lf
            .big_const_hex
            .as_deref()
            .and_then(|h| Felt::from_hex(&format!("0x{h}")).ok())
            .or_else(|| lf.const_value.filter(|v| *v >= 0).map(|v| Felt::from(v as u128)));
        if let Some(addr) = value {
            state.storage_addrs.insert(res.id, addr);
        }
    } else if gid == "storage_address_from_base"
        || gid == "rename"
        || gid == "dup"
        || gid == "store_temp"
        || gid == "store_local"
    {
        if let Some(arg) = inv.args.last() {
            if let Some(addr) = state.storage_addrs.get(&arg.id).copied() {
                for r in &branch.results {
                    state.storage_addrs.insert(r.id, addr);
                }
            }
        }
    } else if gid == "struct_construct" || gid == "struct_deconstruct" || gid == "snapshot_take" {
        // `StoragePointer { address, .. }` wraps the base address as its first member; the
        // address survives construction, deconstruction and snapshots of that struct.
        if let Some(first) = inv.args.first() {
            if let Some(addr) = state.storage_addrs.get(&first.id).copied() {
                state.storage_addrs.insert(res.id, addr);
                if gid == "snapshot_take" {
                    for r in &branch.results {
                        state.storage_addrs.insert(r.id, addr);
                    }
                }
            }
        }
    } else if gid == "storage_address_from_base_and_offset" && inv.args.len() >= 2 {
        let base = state.storage_addrs.get(&inv.args[0].id).copied();
        let offset = match state.vars.get(&inv.args[1].id) {
            Some(SymExpr::Const(v)) if *v >= 0 => Some(*v as u128),
            _ => None,
        };
        if let (Some(base), Some(off)) = (base, offset) {
            state.storage_addrs.insert(res.id, base + Felt::from(off));
        }
    }
}

/// A value the engine tracks: a scalar expression, a struct, or an enum variant with payload.
#[derive(Debug, Clone)]
enum CompVal {
    Expr(Option<SymExpr>),
    Struct(Vec<CompVal>),
    Enum(usize, Box<CompVal>),
    /// The `PanicResult` of a call that was not inlined, with the callee's index.
    CallResult(usize),
}

/// The caller's state while an inlined callee runs.
#[derive(Debug, Clone)]
struct CallFrame {
    caller_func: usize,
    caller_serial: u32,
    return_stmt: usize,
    result_ids: Vec<u64>,
    vars: HashMap<u64, SymExpr>,
    var_max: HashMap<u64, i128>,
    storage_addrs: HashMap<u64, starknet_types_core::felt::Felt>,
    comps: HashMap<u64, CompVal>,
}

/// Inlined calls nested on one path. Loops are recursive functions, so this also bounds loop
/// unrolling; deeper calls are not inlined and their panics count as undecided.
const MAX_INLINE_FRAMES: usize = 10;

/// Builtin types: values threaded through calls that never carry caller data.
fn is_builtin_generic(generic_id: &str) -> bool {
    matches!(
        generic_id,
        "RangeCheck" | "RangeCheck96" | "GasBuiltin" | "Bitwise" | "Pedersen" | "Poseidon" | "EcOp" | "SegmentArena"
            | "System" | "AddMod" | "MulMod" | "Blake"
    )
}

/// Name prefix of opaque values that are unknown functions of the parameters.
const UNKNOWN_PREFIX: &str = "unknown:";

/// Operands of a comparison or a checked operation. When one side has an expression and the
/// other does not (an environment value: storage, caller, call result), the other becomes an
/// opaque value so the condition is still recorded; with neither, it is a condition on state.
fn half_known(a: Option<SymExpr>, b: Option<SymExpr>, lhs_id: u64, rhs_id: u64) -> (Option<SymExpr>, Option<SymExpr>) {
    match (a, b) {
        (None, None) => (None, None),
        (a, b) => (
            Some(a.unwrap_or_else(|| opaque_var("v", lhs_id))),
            Some(b.unwrap_or_else(|| opaque_var("v", rhs_id))),
        ),
    }
}

/// A parameter, a constant or an environment value: no arithmetic.
fn is_raw(e: &SymExpr) -> bool {
    matches!(e, SymExpr::Param(..) | SymExpr::Const(_) | SymExpr::SymbolicOpaque(_))
}

/// A test of the inputs themselves (`a < 5`, `x != 0`, `a == b`, which lowers to
/// `a - b == 0`), as opposed to a condition on a value computed from them.
fn is_raw_test(c: &SymConstraint) -> bool {
    match (&c.lhs, c.op, &c.rhs) {
        (SymExpr::Sub(a, b), CmpOp::Eq | CmpOp::Ne, SymExpr::Const(0))
        | (SymExpr::Const(0), CmpOp::Eq | CmpOp::Ne, SymExpr::Sub(a, b)) => is_raw(a) && is_raw(b),
        (l, _, r) => is_raw(l) && is_raw(r),
    }
}

fn comp_depends_on_params(c: &CompVal) -> bool {
    match c {
        CompVal::Expr(e) => e.as_ref().is_some_and(|e| e.mentions_param() || e.mentions_unknown()),
        CompVal::Struct(members) => members.iter().any(comp_depends_on_params),
        CompVal::Enum(_, payload) => comp_depends_on_params(payload),
        CompVal::CallResult(_) => false,
    }
}

/// Libfuncs that compute a number from their inputs: integer, felt252 and bounded-int
/// arithmetic, conversions and bitwise operations. Libfuncs that take data apart or move it
/// (structs, enums, arrays, boxes, snapshots) or hash it are not on the list: their results are
/// pieces of the input the caller chooses freely, or values no solver can invert.
fn computes_number(gid: &str) -> bool {
    const FAMILIES: [&str; 15] = [
        "u8_", "u16_", "u32_", "u64_", "u128_", "u256_", "u512_", "i8_", "i16_", "i32_", "i64_", "i128_", "felt252_",
        "bounded_int_", "bool_",
    ];
    (FAMILIES.iter().any(|f| gid.starts_with(f)) && !gid.starts_with("felt252_dict"))
        || matches!(gid, "downcast" | "upcast" | "bitwise")
}

/// Results of a numeric libfunc the engine does not model are unknown functions of its inputs.
/// When an input depends on the parameters, the results are marked `unknown:<id>`: a constraint
/// on them cannot confirm a counterexample. Syscall results (storage reads, caller address,
/// call results) are environment values the caller does not choose, so they stay unmarked.
fn mark_unknown_results(
    state: &mut SymPathState,
    lf: &LibfuncMeta,
    inv: &cairo_lang_sierra::program::Invocation,
    b_idx: usize,
) {
    let gid = lf.generic_id.as_str();
    let branch = &inv.branches[b_idx];
    if gid.ends_with("_syscall") || !computes_number(gid) {
        return;
    }
    let depends = inv.args.iter().any(|a| {
        state.vars.get(&a.id).is_some_and(|e| e.mentions_param() || e.mentions_unknown())
            || state.comps.get(&a.id).is_some_and(comp_depends_on_params)
    });
    if !depends {
        return;
    }
    // Without the libfunc's signature the builtin results cannot be told apart: mark nothing.
    let Some(builtins) = lf.builtin_results.get(b_idx) else { return };
    for (r, is_builtin) in branch.results.iter().zip(builtins) {
        if !is_builtin && !state.vars.contains_key(&r.id) && !state.comps.contains_key(&r.id) {
            state.vars.insert(r.id, opaque_var(UNKNOWN_PREFIX, r.id));
        }
    }
}

fn val_of(state: &SymPathState, id: u64) -> CompVal {
    if let Some(c) = state.comps.get(&id) {
        return c.clone();
    }
    CompVal::Expr(state.vars.get(&id).cloned())
}

fn assign_val(state: &mut SymPathState, id: u64, v: CompVal) {
    match v {
        CompVal::Expr(Some(e)) => {
            state.vars.insert(id, e);
            state.comps.remove(&id);
        }
        CompVal::Expr(None) => {
            state.vars.remove(&id);
            state.comps.remove(&id);
        }
        other => {
            state.comps.insert(id, other);
            state.vars.remove(&id);
        }
    }
}

/// Largest value of a numeric Sierra type (`u8`..`u64`, `BoundedInt<lo, hi>`), if known.
fn sierra_type_max(index: &SierraProgramIndex<'_>, ty: &ConcreteTypeId) -> Option<i128> {
    let name = index.type_name(ty);
    if let Some(rest) = name.split("BoundedInt<").nth(1) {
        let hi = rest.trim_end_matches('>').split(',').nth(1)?.trim().to_string();
        return hi.parse::<i128>().ok();
    }
    is_numeric_sierra_type(&name).then(|| max_bound_for_sierra_type(&name))
}

/// Smallest value of a numeric Sierra type (`BoundedInt<lo, hi>`; 0 for unsigned), if known.
fn sierra_type_min(index: &SierraProgramIndex<'_>, ty: &ConcreteTypeId) -> Option<i128> {
    let name = index.type_name(ty);
    if let Some(rest) = name.split("BoundedInt<").nth(1) {
        return rest.split(',').next()?.trim().parse::<i128>().ok();
    }
    is_numeric_sierra_type(&name).then_some(0)
}

/// Panic-valued variables of a function, computed once per program.
fn panic_vars_cached(
    index: &SierraProgramIndex<'_>,
    summaries: &[FunctionInterprocSummary],
    cache: &RefCell<HashMap<usize, Rc<HashSet<u64>>>>,
    func_idx: usize,
) -> Rc<HashSet<u64>> {
    if let Some(v) = cache.borrow().get(&func_idx) {
        return Rc::clone(v);
    }
    let (_, start, end) = index.func_ranges[func_idx];
    let (_, pv) = compute_ok_reachable_statements(index, summaries, start, end);
    let pv = Rc::new(pv);
    cache.borrow_mut().insert(func_idx, Rc::clone(&pv));
    pv
}

fn run_symbolic_verification_for_function(
    index: &SierraProgramIndex<'_>,
    summaries: &[FunctionInterprocSummary],
    func_idx: usize,
    max_depth: usize,
    max_paths: usize,
    oracle: Option<&StorageOracle>,
    panic_cache: &RefCell<HashMap<usize, Rc<HashSet<u64>>>>,
    callee_verdicts: &HashMap<usize, String>,
) -> SymbolicFunctionProofReport {
    let fname = index.func_name(func_idx);
    let (_, short_name) = extract_contract_and_short_name(&fname);
    let is_test_or_property = fname.contains("::tests::")
        || short_name.starts_with("test_")
        || short_name.starts_with("prove_")
        || short_name.starts_with("invariant_")
        || short_name.starts_with("check_");
    let kind = if fname.contains("::__external::") || fname.contains("::__wrapper__") {
        "external_entrypoint"
    } else if fname.contains("::__constructor::") || short_name == "constructor" {
        "constructor"
    } else if fname.contains("::__l1_handler::") {
        "l1_handler"
    } else if is_test_or_property {
        "formal_property"
    } else {
        "function"
    };

    // Whose values are the parameters? Entry points, constructors, handlers, property functions
    // and uncalled functions take caller-chosen values; a method with an entry point wrapper is
    // reached with calldata. Internal helpers and loop bodies receive values their callers
    // already constrained, so a panic that needs a parameter at a type bound is a precondition on
    // the caller, not a counterexample.
    let params_externally_controlled = kind != "function"
        || !summaries[func_idx].has_callers
        || (!fname.contains('[') && has_entry_point_wrapper(index, &short_name));

    let (_, start, end) = index.func_ranges[func_idx];
    let debug_timing = std::env::var("UC_PROVE_DEBUG_TIMING").is_ok();
    // `UC_PROVE_DEBUG_PATHS=<substring>` prints every panic path of matching functions.
    let debug_paths = std::env::var("UC_PROVE_DEBUG_PATHS").ok();
    let phase_started = std::time::Instant::now();
    let panic_vars = panic_vars_cached(index, summaries, panic_cache, func_idx);
    let _ = (start, end);
    if debug_timing && phase_started.elapsed().as_millis() > 200 {
        eprintln!("prove phase: {fname} reachability {} ms ({} statements)", phase_started.elapsed().as_millis(), end - start);
    }

    // Extract symbolic value parameters (excluding builtins like GasBuiltin, RangeCheck, System)
    let mut param_bounds: Vec<(String, i128, i128)> = Vec::new();
    let mut initial_vars: HashMap<u64, SymExpr> = HashMap::new();
    let mut initial_var_max: HashMap<u64, i128> = HashMap::new();
    let mut param_descriptions: Vec<String> = Vec::new();
    let mut param_types: Vec<String> = Vec::new();

    for p in &index.program.funcs[func_idx].params {
        let ty_name = index.type_name(&p.ty);
        if ty_name.contains("GasBuiltin")
            || ty_name.contains("System")
            || ty_name.contains("RangeCheck")
            || ty_name.contains("Bitwise")
            || ty_name.contains("Pedersen")
            || ty_name.contains("Poseidon")
            || ty_name.contains("SegmentArena")
        {
            continue;
        }
        let p_idx = param_bounds.len();
        let pname = p
            .id
            .debug_name
            .as_ref()
            .map(|s| s.as_str().to_string())
            .unwrap_or_else(|| format!("arg_{p_idx}"));
        let max_b = max_bound_for_sierra_type(&ty_name);
        param_bounds.push((pname.clone(), 0, max_b));
        param_types.push(ty_name.clone());
        param_descriptions.push(format!("{pname}: {ty_name}"));
        initial_vars.insert(p.id.id, SymExpr::Param(p_idx, pname));
        if is_numeric_sierra_type(&ty_name) {
            initial_var_max.insert(p.id.id, max_b);
        }
    }

    let mut queue: VecDeque<SymPathState> = VecDeque::new();
    queue.push_back(SymPathState {
        stmt_idx: start,
        depth: 0,
        cur_func: func_idx,
        frames: Vec::new(),
        comps: HashMap::new(),
        unknown_panic: false,
        frame_serial: 0,
        next_serial: 0,
        unknown_reason: None,
        guarded_callee: false,
        unmodeled_branch: false,
        vars: initial_vars,
        storage_addrs: HashMap::new(),
        var_max: initial_var_max,
        constraints: Vec::new(),
        performed_computation: false,
    });

    let mut explored_paths = 0usize;
    let mut ok_paths = 0usize;
    let mut panic_paths = 0usize;
    let mut unreachable_panic_paths = 0usize;
    let mut proven_overflow_free = true;
    let mut preconditions: Vec<String> = Vec::new();
    let mut first_counterexample: Option<(BTreeMap<String, String>, String)> = None;
    let mut first_witness: Option<Vec<i128>> = None;
    let mut undecided_panic_paths = 0usize;
    let mut exploration_truncated = false;
    let mut obligations: Vec<PathObligation> = Vec::new();

    let max_paths = max_paths.max(1);
    let function_started = std::time::Instant::now();
    let function_budget = std::time::Duration::from_millis(FUNCTION_TIMEOUT_MS.load(std::sync::atomic::Ordering::Relaxed));

    // Depth-first: the pending set stays at most `depth x fan-out` states. Breadth-first kept
    // every pending fork of a looping function alive (gigabytes for one Ekubo function).
    let pending_cap = max_paths.saturating_mul(4).max(64);
    while let Some(mut state) = queue.pop_back() {
        FRAME_SERIAL.with(|f| f.set(state.frame_serial));
        if explored_paths >= max_paths || function_started.elapsed() > function_budget {
            exploration_truncated = true;
            break;
        }

        loop {
            let (_, cur_start, cur_end) = index.func_ranges[state.cur_func];
            if state.stmt_idx < cur_start || state.stmt_idx >= cur_end || state.depth > max_depth {
                explored_paths += 1;
                if state.depth > max_depth {
                    exploration_truncated = true;
                }
                break;
            }

            match &index.program.statements[state.stmt_idx] {
                Statement::Return(ret_vars) => {
                    let cur_panic_vars = if state.cur_func == func_idx {
                        Rc::clone(&panic_vars)
                    } else {
                        panic_vars_cached(index, summaries, panic_cache, state.cur_func)
                    };
                    let is_panic = ret_vars.iter().any(|v| cur_panic_vars.contains(&v.id));
                    if !is_panic && !state.frames.is_empty() {
                        // An inlined callee returned normally: back to the caller with the
                        // returned values bound to the call's results.
                        let vals: Vec<CompVal> = ret_vars.iter().map(|v| val_of(&state, v.id)).collect();
                        let frame = state.frames.pop().expect("frame");
                        let frame = Rc::try_unwrap(frame).unwrap_or_else(|rc| (*rc).clone());
                        state.vars = frame.vars;
                        state.var_max = frame.var_max;
                        state.storage_addrs = frame.storage_addrs;
                        state.comps = frame.comps;
                        state.cur_func = frame.caller_func;
                        state.frame_serial = frame.caller_serial;
                        FRAME_SERIAL.with(|f| f.set(state.frame_serial));
                        for (rid, val) in frame.result_ids.iter().zip(vals) {
                            assign_val(&mut state, *rid, val);
                        }
                        state.stmt_idx = frame.return_stmt;
                        state.depth += 1;
                        continue;
                    }
                    // A panic anywhere on the path propagates to the root unchanged (Cairo cannot
                    // catch panics), so it is a panic path of the analysed function.
                    explored_paths += 1;
                    if is_panic {
                        panic_paths += 1;
                        let meaningful_constraints: Vec<SymConstraint> = state
                            .constraints
                            .iter()
                            .filter(|c| {
                                c.lhs.mentions_param()
                                    || c.rhs.mentions_param()
                                    || (c.lhs.eval(&[]).is_some() && c.rhs.eval(&[]).is_some())
                            })
                            .cloned()
                            .collect();

                        let solve_started = std::time::Instant::now();
                        let verdict = solve_path_constraints(&param_bounds, &meaningful_constraints);
                        if debug_timing && solve_started.elapsed().as_millis() > 200 {
                            eprintln!(
                                "prove phase: {fname} solver {} ms ({} constraints, {} params)",
                                solve_started.elapsed().as_millis(),
                                meaningful_constraints.len(),
                                param_bounds.len()
                            );
                        }
                        // A witness from the solver covers the constraints it could evaluate; it
                        // only proves the panic if every path constraint that depends on the
                        // parameters holds for it (constraints on state alone are guards).
                        let witness_confirmed = |w: &[i128]| {
                            state.constraints.iter().all(|c| match c.eval(w) {
                                Some(holds) => holds,
                                None => !(c.lhs.mentions_param()
                                    || c.rhs.mentions_param()
                                    || c.lhs.mentions_unknown()
                                    || c.rhs.mentions_unknown()),
                            })
                        };
                        if KEEP_OBLIGATIONS.load(std::sync::atomic::Ordering::Relaxed) {
                            // A witness the engine cannot confirm (it needs a value of state or
                            // of an unmodeled computation) is not a decision: export `unknown`.
                            let confirmed_witness = match &verdict {
                                SolverOutcome::Sat(w) if witness_confirmed(w) => Some(w.clone()),
                                _ => None,
                            };
                            obligations.push(PathObligation {
                            expected: match (&verdict, &confirmed_witness) {
                                (SolverOutcome::Unsat, _) => "unsat",
                                (SolverOutcome::Sat(_), Some(_)) => "sat",
                                _ => "unknown",
                            },
                            constraints: meaningful_constraints.clone(),
                            witness: confirmed_witness,
                            reason: meaningful_constraints
                                .last()
                                .map(|c| c.reason.clone())
                                .unwrap_or_else(|| "panic path".to_string()),
                            });
                        }
                        let verdict = match verdict {
                            SolverOutcome::Sat(_) if state.unknown_panic => SolverOutcome::Unknown,
                            other => other,
                        };
                        if debug_paths.as_deref().is_some_and(|f| fname.contains(f)) {
                            let confirmed = match &verdict { SolverOutcome::Sat(w) => Some(witness_confirmed(w)), _ => None };
                            eprintln!(
                                "prove path: {fname} verdict={} confirmed={confirmed:?} computation={} unknown_panic={} ({}) meaningful={}",
                                match &verdict { SolverOutcome::Unsat => "unsat", SolverOutcome::Sat(_) => "sat", SolverOutcome::Unknown => "unknown" },
                                state.performed_computation,
                                state.unknown_panic,
                                state.unknown_reason.as_deref().unwrap_or("-"),
                                meaningful_constraints.len()
                            );
                            let witness = match &verdict { SolverOutcome::Sat(w) => Some(w.clone()), _ => None };
                            if let Some(w) = &witness {
                                eprintln!("    witness {w:?}");
                            }
                            for c in &state.constraints {
                                let holds = witness.as_ref().map(|w| c.eval(w));
                                eprintln!("    {} [{}] holds={holds:?}", c.reason, if meaningful_constraints.contains(c) { "meaningful" } else { "dropped" });
                            }
                        }
                        match verdict {
                            SolverOutcome::Unsat => {
                                unreachable_panic_paths += 1;
                            }
                            SolverOutcome::Sat(witness) => {
                                let confirmed = witness_confirmed(&witness);
                                let has_overflow =
                                    meaningful_constraints.iter().any(|c| c.is_overflow_branch);
                                if has_overflow {
                                    proven_overflow_free = false;
                                }
                                // A panic behind tests of the inputs themselves is a precondition the
                                // caller must meet; one that depends only on state is a state guard;
                                // an overflow in a helper is a precondition on its caller.
                                let on_params: Vec<&SymConstraint> = meaningful_constraints
                                    .iter()
                                    .filter(|c| c.lhs.mentions_param() || c.rhs.mentions_param())
                                    .collect();
                                let state_dependent = state.constraints.len() > meaningful_constraints.len();
                                // A witness that holds once state (storage, caller, call results)
                                // takes suitable values is a state guard; one blocked by a value
                                // computed from the parameters, or contradicted, is undecided.
                                let blocked_by_state = !confirmed
                                    && state.constraints.iter().all(|c| match c.eval(&witness) {
                                        Some(holds) => holds,
                                        None => !(c.lhs.mentions_unknown() || c.rhs.mentions_unknown()),
                                    });
                                let is_precondition_guard = state.guarded_callee
                                    || blocked_by_state
                                    || (on_params.is_empty()
                                    && (state_dependent || state.unmodeled_branch || meaningful_constraints.is_empty()))
                                    || (!on_params.is_empty()
                                        && on_params.iter().all(|c| is_raw_test(c))
                                        && !has_overflow
                                        && !is_test_or_property)
                                    || (has_overflow && !params_externally_controlled);

                                if !confirmed && !is_precondition_guard {
                                    if debug_paths.as_deref().is_some_and(|f| fname.contains(f)) {
                                        eprintln!("prove undecided: {fname} cause=unconfirmed-witness");
                                    }
                                    undecided_panic_paths += 1;
                                } else if is_precondition_guard {
                                    let pre = if state.guarded_callee {
                                        "callee_guard".to_string()
                                    } else if meaningful_constraints.is_empty() {
                                        "caller_auth_or_calldata_guard".to_string()
                                    } else {
                                        meaningful_constraints
                                            .last()
                                            .unwrap()
                                            .negate_for_precondition()
                                    };
                                    if !preconditions.contains(&pre) {
                                        preconditions.push(pre);
                                    }
                                } else if first_counterexample.is_none()
                                    && !meaningful_constraints.is_empty()
                                    && (has_overflow || !summaries[func_idx].is_recursive_loop)
                                {
                                    let mut ce_map = BTreeMap::new();
                                    for (idx, (pname, _, _)) in param_bounds.iter().enumerate() {
                                        if let Some(val) = witness.get(idx) {
                                            ce_map.insert(pname.clone(), val.to_string());
                                        }
                                    }
                                    let reason = meaningful_constraints
                                        .last()
                                        .map(|c| c.reason.clone())
                                        .unwrap_or_else(|| "panic path reachable".to_string());
                                    first_counterexample = Some((ce_map, reason));
                                    first_witness = Some(witness.clone());
                                }
                            }
                            SolverOutcome::Unknown => {
                                if debug_paths.as_deref().is_some_and(|f| fname.contains(f)) {
                                    let cause = state.unknown_reason.as_deref().unwrap_or("solver gave up");
                                    eprintln!("prove undecided: {fname} cause={}", cause.split(" at #").next().unwrap_or(cause));
                                }
                                undecided_panic_paths += 1;
                            }
                        }
                    } else {
                        ok_paths += 1;
                    }
                    break;
                }
                Statement::Invocation(inv) => {
                    let Some(lf) = index.libfuncs.get(&inv.libfunc_id.id) else {
                        state.stmt_idx += 1;
                        continue;
                    };
                    let gid = lf.generic_id.as_str();

                    // Single-branch deterministic assignment
                    if inv.branches.len() == 1 {
                        let branch = &inv.branches[0];
                        if oracle.is_some() {
                            track_storage_address(&mut state, gid, lf, inv, branch);
                        }
                        if gid == "function_call" {
                            let callee = lf.user_func_callee.and_then(|cid| index.func_by_id.get(&cid).copied());
                            if let Some(cidx) = callee {
                                let callee_fn = &index.program.funcs[cidx];
                                if state.frames.len() < MAX_INLINE_FRAMES
                                    && !summaries[cidx].always_panics
                                    && callee_fn.params.len() == inv.args.len()
                                {
                                    let arg_vals: Vec<CompVal> = inv.args.iter().map(|a| val_of(&state, a.id)).collect();
                                    let arg_max: Vec<Option<i128>> = inv.args.iter().map(|a| state.var_max.get(&a.id).copied()).collect();
                                    let arg_addr: Vec<Option<starknet_types_core::felt::Felt>> =
                                        inv.args.iter().map(|a| state.storage_addrs.get(&a.id).copied()).collect();
                                    let return_stmt = match branch.target {
                                        BranchTarget::Fallthrough => state.stmt_idx + 1,
                                        BranchTarget::Statement(s) => s.0,
                                    };
                                    let frame = CallFrame {
                                        caller_func: state.cur_func,
                                        caller_serial: state.frame_serial,
                                        return_stmt,
                                        result_ids: branch.results.iter().map(|r| r.id).collect(),
                                        vars: std::mem::take(&mut state.vars),
                                        var_max: std::mem::take(&mut state.var_max),
                                        storage_addrs: std::mem::take(&mut state.storage_addrs),
                                        comps: std::mem::take(&mut state.comps),
                                    };
                                    state.frames.push(Rc::new(frame));
                                    state.next_serial += 1;
                                    state.frame_serial = state.next_serial;
                                    FRAME_SERIAL.with(|f| f.set(state.frame_serial));
                                    for (i, p) in callee_fn.params.iter().enumerate() {
                                        assign_val(&mut state, p.id.id, arg_vals[i].clone());
                                        let ty_max = sierra_type_max(index, &p.ty);
                                        if let Some(m) = arg_max[i].or(ty_max) {
                                            state.var_max.insert(p.id.id, m);
                                        }
                                        if let Some(a) = arg_addr[i] {
                                            state.storage_addrs.insert(p.id.id, a);
                                        }
                                    }
                                    state.cur_func = cidx;
                                    state.stmt_idx = callee_fn.entry_point.0;
                                    state.depth += 1;
                                    continue;
                                }
                            }
                            // Not inlined: the results stay unknown, and the `PanicResult` remembers
                            // its callee so the callee's verdict can decide the `Err` branch.
                            for r in &branch.results {
                                state.vars.remove(&r.id);
                                state.comps.remove(&r.id);
                            }
                            if let (Some(cidx), Some(last)) = (callee, branch.results.last()) {
                                state.comps.insert(last.id, CompVal::CallResult(cidx));
                            }
                        } else if gid == "struct_construct" {
                            if let Some(res) = branch.results.first() {
                                let members: Vec<CompVal> = inv.args.iter().map(|a| val_of(&state, a.id)).collect();
                                assign_val(&mut state, res.id, CompVal::Struct(members));
                            }
                        } else if gid == "struct_deconstruct" {
                            if let Some(CompVal::Struct(members)) = inv.args.last().map(|a| val_of(&state, a.id)) {
                                if members.len() == branch.results.len() {
                                    for (r, m) in branch.results.iter().zip(members) {
                                        assign_val(&mut state, r.id, m);
                                    }
                                }
                            }
                        } else if gid == "enum_init" {
                            if let (Some(res), Some(variant)) = (branch.results.first(), lf.enum_variant_idx) {
                                let payload = inv.args.last().map(|a| val_of(&state, a.id)).unwrap_or(CompVal::Expr(None));
                                assign_val(&mut state, res.id, CompVal::Enum(variant, Box::new(payload)));
                            }
                        } else if (gid == "bitwise" || gid.ends_with("_bitwise")) && inv.args.len() >= 2 && branch.results.len() >= 3 {
                            let lhs_id = inv.args[inv.args.len() - 2].id;
                            let rhs_id = inv.args[inv.args.len() - 1].id;
                            if let (Some(a), Some(b)) = (state.vars.get(&lhs_id).cloned(), state.vars.get(&rhs_id).cloned()) {
                                let n = branch.results.len();
                                let (and_id, xor_id, or_id) = (branch.results[n - 3].id, branch.results[n - 2].id, branch.results[n - 1].id);
                                state.vars.insert(and_id, cap_sym_expr(SymExpr::BitAnd(Box::new(a.clone()), Box::new(b.clone())), and_id));
                                state.vars.insert(xor_id, cap_sym_expr(SymExpr::BitXor(Box::new(a.clone()), Box::new(b.clone())), xor_id));
                                state.vars.insert(or_id, cap_sym_expr(SymExpr::BitOr(Box::new(a), Box::new(b)), or_id));
                                if let Some(max) = state.var_max.get(&lhs_id).copied() {
                                    for id in [and_id, xor_id, or_id] {
                                        state.var_max.insert(id, max);
                                    }
                                }
                            }
                        }
                        if gid == "const_as_immediate" || gid.ends_with("_const") {
                            if let (Some(val), Some(res)) = (lf.const_value, branch.results.first())
                            {
                                state.vars.insert(res.id, SymExpr::Const(val));
                            }
                        } else if gid == "rename"
                            || gid == "dup"
                            || gid == "store_temp"
                            || gid == "store_local"
                            || gid == "snapshot_take"
                            || gid == "unbox"
                            || gid == "into_box"
                            || gid == "upcast"
                            || gid.ends_with("_to_felt252")
                        {
                            if let Some(arg0) = inv.args.last() {
                                if let Some(expr) = state.vars.get(&arg0.id).cloned() {
                                    for r in &branch.results {
                                        state.vars.insert(r.id, expr.clone());
                                    }
                                }
                                if let Some(comp) = state.comps.get(&arg0.id).cloned() {
                                    for r in &branch.results {
                                        state.comps.insert(r.id, comp.clone());
                                    }
                                }
                                if let Some(max) = state.var_max.get(&arg0.id).copied() {
                                    for r in &branch.results {
                                        state.var_max.insert(r.id, max);
                                    }
                                }
                            }
                        } else if (gid.starts_with("felt252_add")
                            || gid.ends_with("_wide_mul")
                            || gid == "bounded_int_add"
                            || gid == "bounded_int_mul")
                            && inv.args.len() >= 2
                        {
                            let lhs_id = inv.args[inv.args.len() - 2].id;
                            let rhs_id = inv.args[inv.args.len() - 1].id;
                            let a = state
                                .vars
                                .get(&lhs_id)
                                .cloned()
                                .unwrap_or_else(|| opaque_var("v", lhs_id));
                            let b = state
                                .vars
                                .get(&rhs_id)
                                .cloned()
                                .unwrap_or_else(|| opaque_var("v", rhs_id));
                            if let Some(r) = branch.results.last() {
                                let expr = if gid.ends_with("_wide_mul") || gid == "bounded_int_mul" {
                                    SymExpr::Mul(Box::new(a), Box::new(b))
                                } else {
                                    SymExpr::Add(Box::new(a), Box::new(b))
                                };
                                state.vars.insert(r.id, cap_sym_expr(expr, r.id));
                                // A felt252 sum is not bounded by its operand's type.
                                if let (false, Some(max)) =
                                    (gid.starts_with("felt252"), state.var_max.get(&lhs_id).copied())
                                {
                                    state.var_max.insert(r.id, max);
                                }
                                state.performed_computation = true;
                            }
                        } else if gid.starts_with("felt252_mul") && inv.args.len() >= 2 {
                            // A field product equals the integer product only while it stays far
                            // below the prime; past the operands' known bounds the result is left
                            // to `mark_unknown_results`.
                            let lhs_id = inv.args[inv.args.len() - 2].id;
                            let rhs_id = inv.args[inv.args.len() - 1].id;
                            let bound = |id: u64| {
                                state.var_max.get(&id).copied().or(match state.vars.get(&id) {
                                    Some(SymExpr::Const(v)) => v.checked_abs(),
                                    _ => None,
                                })
                            };
                            let small = matches!((bound(lhs_id), bound(rhs_id)),
                                (Some(x), Some(y)) if x.checked_mul(y).is_some_and(|p| p < (1_i128 << 120)));
                            if let (true, Some(a), Some(b), Some(r)) = (
                                small,
                                state.vars.get(&lhs_id).cloned(),
                                state.vars.get(&rhs_id).cloned(),
                                branch.results.last(),
                            ) {
                                state.vars.insert(r.id, cap_sym_expr(SymExpr::Mul(Box::new(a), Box::new(b)), r.id));
                                state.performed_computation = true;
                            }
                        } else if (gid.starts_with("felt252_sub") || gid == "bounded_int_sub") && inv.args.len() >= 2 {
                            let lhs_id = inv.args[inv.args.len() - 2].id;
                            let rhs_id = inv.args[inv.args.len() - 1].id;
                            let a = state
                                .vars
                                .get(&lhs_id)
                                .cloned()
                                .unwrap_or_else(|| opaque_var("v", lhs_id));
                            let b = state
                                .vars
                                .get(&rhs_id)
                                .cloned()
                                .unwrap_or_else(|| opaque_var("v", rhs_id));
                            if let Some(r) = branch.results.last() {
                                state
                                    .vars
                                    .insert(r.id, cap_sym_expr(SymExpr::Sub(Box::new(a), Box::new(b)), r.id));
                            }
                        } else if gid.ends_with("_safe_divmod") && inv.args.len() >= 2 {
                            let lhs_id = inv.args[inv.args.len() - 2].id;
                            let rhs_id = inv.args[inv.args.len() - 1].id;
                            let a = state
                                .vars
                                .get(&lhs_id)
                                .cloned()
                                .unwrap_or_else(|| opaque_var("v", lhs_id));
                            let b = state
                                .vars
                                .get(&rhs_id)
                                .cloned()
                                .unwrap_or_else(|| opaque_var("v", rhs_id));
                            if branch.results.len() >= 3 {
                                let type_max = lf.type_args.first().and_then(|t| sierra_type_max(index, t))
                                    .or_else(|| {
                                        let prefix = gid.trim_end_matches("_safe_divmod");
                                        is_numeric_sierra_type(prefix).then(|| max_bound_for_sierra_type(prefix))
                                    });
                                if let Some(m) = type_max {
                                    state.var_max.insert(branch.results[1].id, m);
                                    state.var_max.insert(branch.results[2].id, m);
                                }
                                state.vars.insert(
                                    branch.results[1].id,
                                    cap_sym_expr(SymExpr::Div(Box::new(a.clone()), Box::new(b.clone())), branch.results[1].id),
                                );
                                state
                                    .vars
                                    .insert(branch.results[2].id, cap_sym_expr(SymExpr::Rem(Box::new(a), Box::new(b)), branch.results[2].id));
                                state.performed_computation = true;
                            }
                        }

                        mark_unknown_results(&mut state, lf, inv, 0);
                        state.stmt_idx = match branch.target {
                            BranchTarget::Fallthrough => state.stmt_idx + 1,
                            BranchTarget::Statement(s) => s.0,
                        };
                        state.depth += 1;
                        continue;
                    }

                    // Multi-branch invocation: fork symbolic execution states
                    if inv.branches.len() >= 2 {
                        // `enum_match` on a value whose variant is known follows that branch only.
                        if gid == "enum_match" {
                            if let Some(CompVal::Enum(variant, payload)) = inv.args.last().map(|a| val_of(&state, a.id)) {
                                if let Some(br) = inv.branches.get(variant) {
                                    if let Some(r) = br.results.first() {
                                        assign_val(&mut state, r.id, *payload);
                                    }
                                    state.stmt_idx = match br.target {
                                        BranchTarget::Fallthrough => state.stmt_idx + 1,
                                        BranchTarget::Statement(s) => s.0,
                                    };
                                    state.depth += 1;
                                    continue;
                                }
                            }
                        }
                        // A call that was not inlined: the callee's verdict, when there is one,
                        // decides its `Err` branch. A proven callee cannot panic; a guarded one
                        // panics only behind its own guards.
                        if gid == "enum_match" && lf.is_panic_result_enum {
                            let verdict = match inv.args.last().map(|a| val_of(&state, a.id)) {
                                Some(CompVal::CallResult(cidx)) => callee_verdicts.get(&cidx).map(String::as_str),
                                _ => None,
                            };
                            if let Some(verdict @ ("PROVEN_SAFE" | "GUARDED_SAFE" | "BOUNDED_SAFE")) = verdict {
                                if verdict != "PROVEN_SAFE" {
                                    if let Some(err_branch) = inv.branches.get(1) {
                                        let mut child = state.clone();
                                        child.stmt_idx = match err_branch.target {
                                            BranchTarget::Fallthrough => state.stmt_idx + 1,
                                            BranchTarget::Statement(s) => s.0,
                                        };
                                        child.depth += 1;
                                        child.guarded_callee = true;
                                        if queue.len() < pending_cap {
                                            queue.push_back(child);
                                        } else {
                                            exploration_truncated = true;
                                        }
                                    }
                                }
                                let ok_branch = &inv.branches[0];
                                for r in &ok_branch.results {
                                    state.vars.remove(&r.id);
                                    state.comps.remove(&r.id);
                                }
                                state.stmt_idx = match ok_branch.target {
                                    BranchTarget::Fallthrough => state.stmt_idx + 1,
                                    BranchTarget::Statement(s) => s.0,
                                };
                                state.depth += 1;
                                continue;
                            }
                        }
                        // The result of a call that was not inlined: its `Ok` branch continues, and
                        // its `Err` branch is a panic whose condition is unknown.
                        let cur_panic_vars = if state.cur_func == func_idx {
                            Rc::clone(&panic_vars)
                        } else {
                            panic_vars_cached(index, summaries, panic_cache, state.cur_func)
                        };
                        let is_callee_panic_match = gid == "enum_match"
                            && lf.is_panic_result_enum
                            && !inv.args.iter().any(|a| cur_panic_vars.contains(&a.id));
                        if is_callee_panic_match {
                            if let Some(err_branch) = inv.branches.get(1) {
                                let mut child = state.clone();
                                child.stmt_idx = match err_branch.target {
                                    BranchTarget::Fallthrough => state.stmt_idx + 1,
                                    BranchTarget::Statement(s) => s.0,
                                };
                                child.depth += 1;
                                child.unknown_panic = true;
                                child.unknown_reason = Some(Rc::from(format!("{} at #{} (callee not inlined)", lf.debug_name, state.stmt_idx)));
                                if queue.len() < pending_cap {
                                    queue.push_back(child);
                                } else {
                                    exploration_truncated = true;
                                }
                            }
                        }
                        if gid.starts_with("withdraw_gas")
                            || gid.starts_with("redeposit_gas")
                            || gid.ends_with("_syscall")
                            || is_callee_panic_match
                        {
                            let b0 = &inv.branches[0];
                            if gid == "storage_read_syscall" {
                                if let (Some(oracle), Some(addr_var), Some(res)) =
                                    (oracle, inv.args.last(), b0.results.last())
                                {
                                    if let Some(addr) = state.storage_addrs.get(&addr_var.id) {
                                        if let Some(v) = oracle.read_small(&felt_key_hex(addr)) {
                                            state.vars.insert(res.id, SymExpr::Const(v));
                                        }
                                    }
                                }
                            }
                            state.stmt_idx = match b0.target {
                                BranchTarget::Fallthrough => state.stmt_idx + 1,
                                BranchTarget::Statement(s) => s.0,
                            };
                            state.depth += 1;
                            continue;
                        }

                        for (b_idx, branch) in inv.branches.iter().enumerate() {
                            let next_stmt = match branch.target {
                                BranchTarget::Fallthrough => state.stmt_idx + 1,
                                BranchTarget::Statement(s) => s.0,
                            };
                            let mut child = state.clone();
                            child.stmt_idx = next_stmt;
                            child.depth += 1;
                            let constraints_before = child.constraints.len();
                            let condition_libfunc = gid == "downcast"
                                || gid.ends_with("_is_zero")
                                || gid.ends_with("_eq")
                                || gid.ends_with("_lt")
                                || gid.ends_with("_le")
                                || gid.ends_with("_try_from_felt252")
                                || gid == "bounded_int_trim_max"
                                || gid == "bounded_int_trim_min"
                                || gid.ends_with("_overflowing_add")
                                || gid.ends_with("_overflowing_sub");

                            if gid == "downcast" && !inv.args.is_empty() {
                                // Branch 0: the value fits the target type (and is the result);
                                // branch 1: it does not (the overflow path of checked `*` and
                                // narrowing conversions).
                                let arg_id = inv.args[inv.args.len() - 1].id;
                                let target_max = lf.type_args.get(1).and_then(|t| sierra_type_max(index, t));
                                if let (Some(arg_expr), Some(max_val)) = (state.vars.get(&arg_id).cloned(), target_max) {
                                    if b_idx == 0 {
                                        if let Some(res_var) = branch.results.last() {
                                            child.vars.insert(res_var.id, arg_expr.clone());
                                            child.var_max.insert(res_var.id, max_val);
                                        }
                                    }
                                    child.performed_computation = true;
                                    let op = if b_idx == 0 { CmpOp::Le } else { CmpOp::Gt };
                                    child.constraints.push(SymConstraint {
                                        lhs: arg_expr.clone(),
                                        op,
                                        rhs: SymExpr::Const(max_val),
                                        reason: format!(
                                            "arithmetic overflow in `downcast`: {} {} {max_val}",
                                            arg_expr.format_human(),
                                            if b_idx == 0 { "<=" } else { ">" }
                                        ),
                                        is_overflow_branch: b_idx == 1,
                                    });
                                }
                            } else if gid.ends_with("_is_zero") && !inv.args.is_empty() {
                                let arg_id = inv.args[inv.args.len() - 1].id;
                                if let Some(arg_expr) = state.vars.get(&arg_id).cloned() {
                                    // The non-zero branch hands back the same value as `NonZero<T>`.
                                    if b_idx == 1 {
                                        if let Some(res_var) = branch.results.first() {
                                            child.vars.insert(res_var.id, arg_expr.clone());
                                            if let Some(max) = state.var_max.get(&arg_id).copied() {
                                                child.var_max.insert(res_var.id, max);
                                            }
                                        }
                                    }
                                    let op = if b_idx == 0 { CmpOp::Eq } else { CmpOp::Ne };
                                    child.constraints.push(SymConstraint {
                                        lhs: arg_expr.clone(),
                                        op,
                                        rhs: SymExpr::Const(0),
                                        reason: format!(
                                            "{gid}({}) branch {b_idx}",
                                            arg_expr.format_human()
                                        ),
                                        is_overflow_branch: false,
                                    });
                                }
                            } else if gid.ends_with("_eq") && inv.args.len() >= 2 {
                                let lhs_id = inv.args[inv.args.len() - 2].id;
                                let rhs_id = inv.args[inv.args.len() - 1].id;
                                let a = state.vars.get(&lhs_id).cloned();
                                let b = state.vars.get(&rhs_id).cloned();
                                if let (Some(lhs), Some(rhs)) = half_known(a, b, lhs_id, rhs_id) {
                                    let op = if b_idx == 0 { CmpOp::Ne } else { CmpOp::Eq };
                                    child.constraints.push(SymConstraint {
                                        lhs: lhs.clone(),
                                        op,
                                        rhs: rhs.clone(),
                                        reason: format!(
                                            "{gid}({}, {}) branch {b_idx}",
                                            lhs.format_human(),
                                            rhs.format_human()
                                        ),
                                        is_overflow_branch: false,
                                    });
                                }
                            } else if gid.ends_with("_lt") && inv.args.len() >= 2 {
                                let lhs_id = inv.args[inv.args.len() - 2].id;
                                let rhs_id = inv.args[inv.args.len() - 1].id;
                                let a = state.vars.get(&lhs_id).cloned();
                                let b = state.vars.get(&rhs_id).cloned();
                                if let (Some(lhs), Some(rhs)) = half_known(a, b, lhs_id, rhs_id) {
                                    let op = if b_idx == 0 { CmpOp::Ge } else { CmpOp::Lt };
                                    child.constraints.push(SymConstraint {
                                        lhs: lhs.clone(),
                                        op,
                                        rhs: rhs.clone(),
                                        reason: format!(
                                            "{gid}({}, {}) branch {b_idx}",
                                            lhs.format_human(),
                                            rhs.format_human()
                                        ),
                                        is_overflow_branch: false,
                                    });
                                }
                            } else if gid.ends_with("_le") && inv.args.len() >= 2 {
                                let lhs_id = inv.args[inv.args.len() - 2].id;
                                let rhs_id = inv.args[inv.args.len() - 1].id;
                                let a = state.vars.get(&lhs_id).cloned();
                                let b = state.vars.get(&rhs_id).cloned();
                                if let (Some(lhs), Some(rhs)) = half_known(a, b, lhs_id, rhs_id) {
                                    let op = if b_idx == 0 { CmpOp::Gt } else { CmpOp::Le };
                                    child.constraints.push(SymConstraint {
                                        lhs: lhs.clone(),
                                        op,
                                        rhs: rhs.clone(),
                                        reason: format!(
                                            "{gid}({}, {}) branch {b_idx}",
                                            lhs.format_human(),
                                            rhs.format_human()
                                        ),
                                        is_overflow_branch: false,
                                    });
                                }
                            } else if gid.ends_with("_try_from_felt252") && !inv.args.is_empty() {
                                // Branch 0: the felt fits the integer type; branch 1: it does not.
                                let arg_id = inv.args[inv.args.len() - 1].id;
                                let ty_prefix = gid.split('_').next().unwrap_or("u128");
                                let max_val = max_bound_for_sierra_type(ty_prefix);
                                if let Some(arg_expr) = state.vars.get(&arg_id).cloned() {
                                    // Narrowing a computed felt252 (`(a - b).try_into()`) is
                                    // arithmetic; an equality test (`a - b` then `is_zero`) is not.
                                    if !matches!(arg_expr, SymExpr::Param(..) | SymExpr::Const(_) | SymExpr::SymbolicOpaque(_)) {
                                        child.performed_computation = true;
                                    }
                                    // A negative integer reading is a field element above every
                                    // integer type, so it lands on the failure branch too.
                                    let negative_possible = arg_expr.may_be_negative();
                                    if b_idx == 0 {
                                        if let Some(res_var) = branch.results.last() {
                                            child.vars.insert(res_var.id, arg_expr.clone());
                                            child.var_max.insert(res_var.id, max_val);
                                        }
                                        if negative_possible {
                                            child.constraints.push(SymConstraint {
                                                lhs: arg_expr.clone(),
                                                op: CmpOp::Ge,
                                                rhs: SymExpr::Const(0),
                                                reason: format!("{gid}({}) branch 0", arg_expr.format_human()),
                                                is_overflow_branch: false,
                                            });
                                        }
                                    } else if negative_possible {
                                        let mut below = child.clone();
                                        below.constraints.push(SymConstraint {
                                            lhs: arg_expr.clone(),
                                            op: CmpOp::Lt,
                                            rhs: SymExpr::Const(0),
                                            reason: format!(
                                                "{gid}({}) branch 1: negative, wraps above the type",
                                                arg_expr.format_human()
                                            ),
                                            is_overflow_branch: !is_raw(&arg_expr),
                                        });
                                        if queue.len() < pending_cap {
                                            queue.push_back(below);
                                        } else {
                                            exploration_truncated = true;
                                        }
                                    }
                                    let op = if b_idx == 0 { CmpOp::Le } else { CmpOp::Gt };
                                    child.constraints.push(SymConstraint {
                                        lhs: arg_expr.clone(),
                                        op,
                                        rhs: SymExpr::Const(max_val),
                                        reason: format!("{gid}({}) branch {b_idx}", arg_expr.format_human()),
                                        is_overflow_branch: b_idx == 1 && !is_raw(&arg_expr),
                                    });
                                } else if b_idx == 0 {
                                    if let Some(res_var) = branch.results.last() {
                                        child.var_max.insert(res_var.id, max_val);
                                    }
                                }
                            } else if (gid == "bounded_int_trim_max" || gid == "bounded_int_trim_min")
                                && !inv.args.is_empty()
                            {
                                // Checked `+ 1` / `- 1` lowering: branch 0 is the value already at
                                // the bound (the overflow / underflow panic path), branch 1 the
                                // trimmed value.
                                let arg_id = inv.args[inv.args.len() - 1].id;
                                // The bound is the extreme of the libfunc's type argument
                                // (`bounded_int_trim_max<u8>` trims 255), not a tracked range.
                                let bound = if gid == "bounded_int_trim_max" {
                                    lf.type_args.first().and_then(|t| sierra_type_max(index, t))
                                } else {
                                    Some(lf.type_args.first().and_then(|t| sierra_type_min(index, t)).unwrap_or(0))
                                };
                                if let (Some(arg_expr), Some(bound)) = (state.vars.get(&arg_id).cloned(), bound) {
                                    if b_idx == 1 {
                                        if let Some(res_var) = branch.results.last() {
                                            child.vars.insert(res_var.id, arg_expr.clone());
                                            if let Some(max) = state.var_max.get(&arg_id).copied() {
                                                child.var_max.insert(res_var.id, max);
                                            }
                                        }
                                    }
                                    child.performed_computation = true;
                                    let (op, what) = match (gid, b_idx) {
                                        ("bounded_int_trim_max", 0) => (CmpOp::Eq, "overflow"),
                                        ("bounded_int_trim_max", _) => (CmpOp::Lt, "no overflow"),
                                        (_, 0) => (CmpOp::Eq, "underflow"),
                                        (_, _) => (CmpOp::Gt, "no underflow"),
                                    };
                                    child.constraints.push(SymConstraint {
                                        lhs: arg_expr.clone(),
                                        op,
                                        rhs: SymExpr::Const(bound),
                                        reason: format!(
                                            "arithmetic {what} in `{gid}`: {} {} {bound}",
                                            arg_expr.format_human(),
                                            match op { CmpOp::Eq => "==", CmpOp::Lt => "<", _ => ">" }
                                        ),
                                        is_overflow_branch: b_idx == 0,
                                    });
                                } else if b_idx == 1 {
                                    if let (Some(res_var), Some(max)) = (branch.results.last(), state.var_max.get(&arg_id).copied()) {
                                        child.var_max.insert(res_var.id, max);
                                    }
                                }
                            } else if gid.ends_with("_overflowing_add") && inv.args.len() >= 2 {
                                let lhs_id = inv.args[inv.args.len() - 2].id;
                                let rhs_id = inv.args[inv.args.len() - 1].id;
                                let a = state.vars.get(&lhs_id).cloned();
                                let b = state.vars.get(&rhs_id).cloned();
                                let ty_prefix = gid.split('_').next().unwrap_or("u128");
                                let max_val = max_bound_for_sierra_type(ty_prefix);
                                if let (Some(lhs), Some(rhs)) = half_known(a, b, lhs_id, rhs_id) {
                                    let sum_expr = cap_sym_expr(
                                        SymExpr::Add(Box::new(lhs.clone()), Box::new(rhs.clone())),
                                        branch.results.last().map(|r| r.id).unwrap_or(0),
                                    );
                                    if let Some(res_var) = branch.results.last() {
                                        child.vars.insert(res_var.id, sum_expr.clone());
                                        child.var_max.insert(res_var.id, max_val);
                                    }
                                    child.performed_computation = true;
                                    let op = if b_idx == 0 { CmpOp::Le } else { CmpOp::Gt };
                                    child.constraints.push(SymConstraint {
                                        lhs: sum_expr.clone(),
                                        op,
                                        rhs: SymExpr::Const(max_val),
                                        reason: format!(
                                            "arithmetic overflow in `{gid}`: {} > {max_val}",
                                            sum_expr.format_human()
                                        ),
                                        is_overflow_branch: b_idx == 1,
                                    });
                                }
                            } else if gid.ends_with("_overflowing_sub") && inv.args.len() >= 2 {
                                let lhs_id = inv.args[inv.args.len() - 2].id;
                                let rhs_id = inv.args[inv.args.len() - 1].id;
                                let a = state.vars.get(&lhs_id).cloned();
                                let b = state.vars.get(&rhs_id).cloned();
                                if let (Some(lhs), Some(rhs)) = half_known(a, b, lhs_id, rhs_id) {
                                    let diff_expr = cap_sym_expr(
                                        SymExpr::Sub(Box::new(lhs.clone()), Box::new(rhs.clone())),
                                        branch.results.last().map(|r| r.id).unwrap_or(0),
                                    );
                                    if let Some(res_var) = branch.results.last() {
                                        child.vars.insert(res_var.id, diff_expr);
                                    }
                                    let op = if b_idx == 0 { CmpOp::Ge } else { CmpOp::Lt };
                                    child.constraints.push(SymConstraint {
                                        lhs: lhs.clone(),
                                        op,
                                        rhs: rhs.clone(),
                                        reason: format!(
                                            "underflow / inequality in `{gid}`: {} < {}",
                                            lhs.format_human(),
                                            rhs.format_human()
                                        ),
                                        is_overflow_branch: b_idx == 1 && !(is_raw(&lhs) && is_raw(&rhs)),
                                    });
                                }
                            }

                            // A branch the engine could not express on values computed from the
                            // parameters is not unconstrained: whatever panics behind it has an
                            // unknown condition. On environment values (storage, caller, call
                            // results; they carry no expression) it is a state guard.
                            let on_parameters = inv.args.iter().any(|a| {
                                state.vars.get(&a.id).is_some_and(|e| e.mentions_param() || e.mentions_unknown())
                                    || state.comps.get(&a.id).is_some_and(comp_depends_on_params)
                            });
                            if child.constraints.len() == constraints_before {
                                child.unmodeled_branch = true;
                            }
                            if condition_libfunc && on_parameters && child.constraints.len() == constraints_before {
                                child.unknown_panic = true;
                                child.unknown_reason = Some(Rc::from(format!("{} at #{} branch {b_idx} (condition not modeled)", lf.debug_name, state.stmt_idx)));
                            }
                            mark_unknown_results(&mut child, lf, inv, b_idx);
                            if queue.len() >= pending_cap {
                                exploration_truncated = true;
                                continue;
                            }
                            queue.push_back(child);
                        }
                        break;
                    }

                    break;
                }
            }
        }
    }

    let status = if first_counterexample.is_some() {
        "COUNTEREXAMPLE_FOUND".to_string()
    } else if undecided_panic_paths > 0 {
        // A panic path the solver could not refute is not a proof of anything.
        "UNDECIDED".to_string()
    } else if !preconditions.is_empty() {
        "GUARDED_SAFE".to_string()
    } else if exploration_truncated {
        // Every explored panic path is unreachable, but paths past the budget were not seen.
        "BOUNDED_SAFE".to_string()
    } else {
        "PROVEN_SAFE".to_string()
    };

    let (counterexample, counterexample_reason) = match first_counterexample {
        Some((ce, reason)) => (Some(ce), Some(reason)),
        None => (None, None),
    };
    let counterexample_calldata = first_witness.as_deref().map(|w| witness_calldata(w, &param_types));

    SymbolicFunctionProofReport {
        function: fname,
        kind: kind.to_string(),
        params: param_descriptions,
        status,
        explored_paths,
        ok_paths,
        panic_paths,
        unreachable_panic_paths,
        undecided_panic_paths,
        exploration_truncated,
        proven_overflow_free,
        preconditions,
        counterexample,
        counterexample_calldata,
        smt_file: None,
        obligations,
        param_bounds,
        param_types,
        counterexample_reason,
        concrete_verified: None,
        concrete_panic_data: None,
        repro_test: None,
    }
}

/// Runs formal verification + Caracal Sierra IR analysis directly on an in-memory `Program`.
/// `roots` and every function they call, transitively, each after the functions it calls (a
/// cycle, as a loop is, is cut where it closes).
fn callees_first(summaries: &[FunctionInterprocSummary], roots: &[usize]) -> Vec<usize> {
    let mut order = Vec::new();
    let mut seen = vec![false; summaries.len()];
    for &root in roots {
        if seen[root] {
            continue;
        }
        seen[root] = true;
        let mut stack = vec![(root, 0usize)];
        while let Some((f, next)) = stack.pop() {
            match summaries[f].call_sites.get(next) {
                Some(&(callee, _)) => {
                    stack.push((f, next + 1));
                    if !seen[callee] {
                        seen[callee] = true;
                        stack.push((callee, 0));
                    }
                }
                None => order.push(f),
            }
        }
    }
    order
}

pub(crate) fn analyze_sierra_program(
    program: &Program,
    crate_name: &str,
    filter: Option<&str>,
    max_depth: usize,
    max_paths: usize,
    enable_caracal: bool,
    oracle: Option<&StorageOracle>,
) -> (Vec<SymbolicFunctionProofReport>, Vec<CaracalSierraFinding>) {
    let index = SierraProgramIndex::build(program);
    let summaries = compute_function_summaries(&index);

    let caracal_findings = if enable_caracal {
        run_caracal_sierra_analysis(&index, &summaries, crate_name, filter)
    } else {
        Vec::new()
    };

    let panic_cache: RefCell<HashMap<usize, Rc<HashSet<u64>>>> = RefCell::new(HashMap::new());
    let roots: Vec<usize> = (0..program.funcs.len())
        .filter(|&func_idx| {
            let fname = index.func_name(func_idx);
            if !fname.starts_with(&format!("{crate_name}::")) || filter.is_some_and(|f| !fname.contains(f)) {
                return false;
            }
            // `#[test]`/`#[fuzzer]` bodies deploy contracts and use cheatcodes the symbolic engine
            // does not model; `uc test` runs them concretely. Only property-style functions
            // (`prove_*`, `invariant_*`, `check_*`) are symbolic targets.
            let (_, short) = extract_contract_and_short_name(&fname);
            let is_plain_test = (fname.contains("::tests::") || short.starts_with("test_"))
                && !(short.starts_with("prove_") || short.starts_with("invariant_") || short.starts_with("check_"));
            !is_plain_test
        })
        .collect();
    // Callees before callers, so a call the engine does not inline is decided by the verdict the
    // callee already has; functions outside the crate are analysed for that and not reported.
    let root_set: HashSet<usize> = roots.iter().copied().collect();
    let mut verdicts: HashMap<usize, String> = HashMap::new();
    let mut reports: HashMap<usize, SymbolicFunctionProofReport> = HashMap::new();
    for func_idx in callees_first(&summaries, &roots) {
        let fname = index.func_name(func_idx);
        let started = std::time::Instant::now();
        if std::env::var("UC_PROVE_DEBUG_TIMING").is_ok() {
            eprintln!("prove start: {fname}");
        }
        let proof = run_symbolic_verification_for_function(&index, &summaries, func_idx, max_depth, max_paths, oracle, &panic_cache, &verdicts);
        if std::env::var("UC_PROVE_DEBUG_TIMING").is_ok() {
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            if ms > 100.0 {
                eprintln!(
                    "prove timing: {fname} {ms:.0} ms, paths {}, panic {}, status {}",
                    proof.explored_paths, proof.panic_paths, proof.status
                );
            }
        }
        verdicts.insert(func_idx, proof.status.clone());
        if root_set.contains(&func_idx) {
            reports.insert(func_idx, proof);
        }
    }
    let symbolic_proofs = roots.iter().filter_map(|f| reports.remove(f)).collect();

    (symbolic_proofs, caracal_findings)
}

#[cfg(feature = "native-test-runner")]
fn verify_counterexamples_concretely(
    runner: &cairo_lang_runner::SierraCasmRunner,
    program: &Program,
    proofs: &mut [SymbolicFunctionProofReport],
    storage_seeds: &[(String, String)],
) {
    use cairo_lang_runner::casm_run::format_for_panic;
    use cairo_lang_runner::{Arg, RunResultValue, StarknetState};
    use starknet_types_core::felt::Felt as Felt252;

    let seeded_state = || {
        let mut state = StarknetState::default();
        for (key, value) in storage_seeds {
            if let (Ok(k), Ok(v)) = (Felt252::from_hex(key), Felt252::from_hex(value)) {
                state.seed_storage_for_current_contract(k, v);
            }
        }
        state
    };

    for proof in proofs.iter_mut() {
        if proof.status != "COUNTEREXAMPLE_FOUND" {
            continue;
        }
        let Some(ce_map) = &proof.counterexample else {
            continue;
        };
        let (_, short_name) = extract_contract_and_short_name(&proof.function);
        let arg_literals: Vec<String> = ce_map.values().cloned().collect();
        let base_func = proof.function.split('[').next().unwrap_or(&proof.function);
        let reason_comment = proof
            .counterexample_reason
            .as_deref()
            .unwrap_or("symbolic panic counterexample");
        if !proof.function.contains('[') && !proof.function.contains("__wrapper__") {
            proof.repro_test = Some(format!(
                "// Counterexample witness: {reason_comment}\n#[test]\n#[should_panic]\nfn repro_{short_name}() {{\n    {}({});\n}}",
                base_func,
                arg_literals.join(", ")
            ));
        } else {
            let witness_pairs: Vec<String> =
                ce_map.iter().map(|(k, v)| format!("{k} = {v}")).collect();
            proof.repro_test = Some(format!(
                "// Counterexample in `{}` ({reason_comment})\n// Solved witness: {}\n#[test]\n#[should_panic]\nfn repro_{short_name}() {{\n    // Invoke `{base_func}` with the witness values above:\n    panic!(\"{reason_comment}\");\n}}",
                proof.function,
                witness_pairs.join(", "),
            ));
        }

        if proof.function.contains('[') || proof.function.contains("__wrapper__") {
            continue;
        }
        // Contract methods replay through their entry point wrapper with the witness encoded as
        // calldata, so storage reads see the default (zero) state exactly as a fresh deployment.
        if let (Some(wrapper), Some(calldata)) =
            (find_entry_point_wrapper(program, &proof.function), proof.counterexample_calldata.as_ref())
        {
            let felts: Vec<Arg> = calldata
                .iter()
                .map(|h| Arg::Value(Felt252::from_hex(h).unwrap_or(Felt252::ZERO)))
                .collect();
            match runner.run_function_with_starknet_context(
                wrapper,
                vec![Arg::Array(felts)],
                Some(1_000_000_000),
                seeded_state(),
            ) {
                Ok(run_res) => match run_res.value {
                    RunResultValue::Panic(felts) => {
                        let msg = format_for_panic(felts.into_iter());
                        proof.concrete_verified =
                            Some(concrete_panic_confirms_witness(reason_comment, &msg));
                        proof.concrete_panic_data = Some(msg);
                    }
                    RunResultValue::Success(_) => proof.concrete_verified = Some(false),
                },
                Err(err) => {
                    proof.concrete_verified = Some(false);
                    proof.concrete_panic_data = Some(format!("replay error: {err}"));
                }
            }
            continue;
        }
        let Some(func) = program.funcs.iter().find(|f| {
            f.id.debug_name
                .as_ref()
                .is_some_and(|dn| dn.as_str() == proof.function)
        }) else {
            continue;
        };

        let mut user_args = Vec::new();
        for (_k, v_str) in ce_map {
            let parsed = v_str.parse::<u128>().unwrap_or(0);
            user_args.push(Arg::Value(Felt252::from(parsed)));
        }

        if let Ok(run_res) = runner.run_function_with_starknet_context(
            func,
            user_args,
            Some(10_000_000),
            seeded_state(),
        ) {
            if let RunResultValue::Panic(felts) = run_res.value {
                proof.concrete_verified = Some(true);
                proof.concrete_panic_data = Some(format_for_panic(felts.into_iter()));
            } else {
                proof.concrete_verified = Some(false);
            }
        }
    }
}

#[cfg(feature = "native-test-runner")]
fn count_wrapper_calldata_pops(index: &SierraProgramIndex<'_>, func_idx: usize) -> usize {
    fn visit(index: &SierraProgramIndex<'_>, cur: usize, depth: usize) -> usize {
        if depth > 4 {
            return 0;
        }
        let (_, start, end) = index.func_ranges[cur];
        let mut pops = 0usize;
        for stmt_idx in start..end {
            if let Statement::Invocation(inv) = &index.program.statements[stmt_idx] {
                if let Some(lf) = index.libfuncs.get(&inv.libfunc_id.id) {
                    if lf.generic_id.contains("pop_front") {
                        pops += 1;
                    }
                    if let Some(callee_id) = lf.user_func_callee {
                        if let Some(&callee_idx) = index.func_by_id.get(&callee_id) {
                            if callee_idx != cur {
                                pops += visit(index, callee_idx, depth + 1);
                            }
                        }
                    }
                }
            }
        }
        pops
    }
    // Every Starknet `__wrapper__*` ends argument deserialization with
    // `assert(SpanTrait::is_empty(data), 'Input too long for arguments')`, where
    // `SpanTrait::is_empty` invokes `pop_front` once. Subtracting that trailing check gives
    // the exact felt252 calldata arity.
    visit(index, func_idx, 0).saturating_sub(1)
}

#[cfg(feature = "native-test-runner")]
fn replay_call_sequence_and_check_invariant(
    runner: &cairo_lang_runner::SierraCasmRunner,
    program: &Program,
    base_state: &cairo_lang_runner::StarknetState,
    seq: &[(usize, Vec<u128>, String)],
    inv_func_idx: usize,
    inv_is_wrapper: bool,
) -> bool {
    use cairo_lang_runner::{Arg, RunResultValue};
    use starknet_types_core::felt::Felt as Felt252;

    let mut state = base_state.clone();
    for (m_func_idx, args_u128, _) in seq {
        let felts: Vec<Arg> = args_u128
            .iter()
            .copied()
            .map(|v| Arg::Value(Felt252::from(v)))
            .collect();
        let Ok(res) = runner.run_function_with_starknet_context(
            &program.funcs[*m_func_idx],
            vec![Arg::Array(felts)],
            Some(10_000_000),
            state.clone(),
        ) else {
            continue;
        };
        if matches!(res.value, RunResultValue::Success(_)) {
            state = res.starknet_state;
        }
    }

    let inv_args = if inv_is_wrapper {
        vec![Arg::Array(Vec::new())]
    } else {
        Vec::new()
    };
    let Ok(inv_res) = runner.run_function_with_starknet_context(
        &program.funcs[inv_func_idx],
        inv_args,
        Some(10_000_000),
        state,
    ) else {
        return false;
    };
    match inv_res.value {
        RunResultValue::Panic(_) => true,
        RunResultValue::Success(ret_felts) => {
            // If an Echidna/Medusa style `#[external(v0)] fn invariant_*() -> bool` returns `false` (0)
            inv_is_wrapper && ret_felts.last().is_some_and(|f| *f == Felt252::from(0u64))
                && ret_felts.len() == 1
        }
    }
}

#[cfg(feature = "native-test-runner")]
fn run_medusa_stateful_campaign(
    runner: &cairo_lang_runner::SierraCasmRunner,
    index: &SierraProgramIndex<'_>,
    crate_name: &str,
    named_tests: &[(String, cairo_lang_test_plugin::TestConfig)],
    stateful_runs: usize,
) -> Vec<StatefulInvariantResult> {
    use cairo_lang_runner::{Arg, RunResultValue, StarknetState};
    use starknet_types_core::felt::Felt as Felt252;

    if stateful_runs == 0 {
        return Vec::new();
    }

    let program = index.program;
    let mut results = Vec::new();

    // Group contract wrapper functions by contract name
    let mut contract_constructors: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut contract_mutators: BTreeMap<String, Vec<(usize, String, usize)>> = BTreeMap::new();
    let mut contract_invariants: BTreeMap<String, Vec<(usize, String, bool)>> = BTreeMap::new();

    for func_idx in 0..program.funcs.len() {
        let fname = index.func_name(func_idx);
        if !fname.starts_with(&format!("{crate_name}::")) {
            continue;
        }
        let is_wrapper = fname.contains("::__wrapper__") || fname.contains("::__external::");
        if !is_wrapper {
            continue;
        }
        let (contract, short) = extract_contract_and_short_name(&fname);
        let arity = count_wrapper_calldata_pops(index, func_idx);
        if short == "constructor" || fname.contains("::__constructor::") {
            contract_constructors.insert(contract, (func_idx, arity));
        } else if short.starts_with("invariant_") || short.starts_with("echidna_") {
            contract_invariants
                .entry(contract)
                .or_default()
                .push((func_idx, format!("{fname}"), true));
        } else if !short.starts_with("get_") && !short.starts_with("is_") {
            contract_mutators
                .entry(contract)
                .or_default()
                .push((func_idx, short, arity));
        }
    }

    // Also include standalone `#[test] fn invariant_*` functions
    for (test_name, _cfg) in named_tests {
        let short = test_name
            .rsplit_once("::")
            .map(|(_, s)| s)
            .unwrap_or(test_name);
        if short.starts_with("invariant_") || short.starts_with("stateful_") {
            if let Some(func_idx) = (0..program.funcs.len()).find(|&i| {
                let n = index.func_name(i);
                n == *test_name || n.ends_with(&format!("::{short}"))
            }) {
                let first_contract = contract_mutators
                    .keys()
                    .next()
                    .cloned()
                    .unwrap_or_else(|| "standalone".to_string());
                contract_invariants
                    .entry(first_contract)
                    .or_default()
                    .push((func_idx, test_name.clone(), false));
            }
        }
    }

    let seed_pool: &[u128] = &[0, 1, 2, 10, 42, 100, 500, 1000, 10_000, 10_001, 20_000, 65_535];

    for (contract, invariants) in contract_invariants {
        let mutators = contract_mutators.get(&contract).cloned().unwrap_or_default();
        let mut base_state = StarknetState::default();
        if let Some((ctor_idx, ctor_arity)) = contract_constructors.get(&contract).copied() {
            let init_args = vec![Arg::Value(Felt252::from(100u64)); ctor_arity];
            if let Ok(res) = runner.run_function_with_starknet_context(
                &program.funcs[ctor_idx],
                vec![Arg::Array(init_args)],
                Some(10_000_000),
                base_state.clone(),
            ) {
                if matches!(res.value, RunResultValue::Success(_)) {
                    base_state = res.starknet_state;
                }
            }
        }

        for (inv_func_idx, inv_name, inv_is_wrapper) in invariants {
            let mut violated_seq: Option<Vec<(usize, Vec<u128>, String)>> = None;
            let seq_len = if mutators.is_empty() { 1 } else { 4 };

            for run_idx in 0..stateful_runs {
                let mut state = base_state.clone();
                let mut current_seq: Vec<(usize, Vec<u128>, String)> = Vec::new();

                for step_idx in 0..seq_len {
                    if !mutators.is_empty() {
                        let (m_idx, m_short, m_arity) =
                            &mutators[(run_idx + step_idx) % mutators.len()];
                        let mut call_args = Vec::with_capacity(*m_arity);
                        for a_i in 0..*m_arity {
                            let val = seed_pool[(run_idx * 3 + step_idx * 5 + a_i) % seed_pool.len()];
                            call_args.push(val);
                        }
                        let felts: Vec<Arg> = call_args
                            .iter()
                            .copied()
                            .map(|v| Arg::Value(Felt252::from(v)))
                            .collect();
                        if let Ok(res) = runner.run_function_with_starknet_context(
                            &program.funcs[*m_idx],
                            vec![Arg::Array(felts)],
                            Some(10_000_000),
                            state.clone(),
                        ) {
                            if matches!(res.value, RunResultValue::Success(_)) {
                                state = res.starknet_state;
                                let formatted = format!(
                                    "{contract}::{m_short}({})",
                                    call_args
                                        .iter()
                                        .map(|v| v.to_string())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                );
                                current_seq.push((*m_idx, call_args, formatted));
                            }
                        }
                    }

                    // Check invariant after this state transition
                    if replay_call_sequence_and_check_invariant(
                        runner,
                        program,
                        &base_state,
                        &current_seq,
                        inv_func_idx,
                        inv_is_wrapper,
                    ) {
                        violated_seq = Some(current_seq);
                        break;
                    }
                }

                if violated_seq.is_some() {
                    break;
                }
            }

            if let Some(mut seq) = violated_seq {
                // Shrink failing transaction sequence via delta-debugging
                let mut idx = 0usize;
                while idx < seq.len() && seq.len() > 1 {
                    let mut candidate = seq.clone();
                    candidate.remove(idx);
                    if replay_call_sequence_and_check_invariant(
                        runner,
                        program,
                        &base_state,
                        &candidate,
                        inv_func_idx,
                        inv_is_wrapper,
                    ) {
                        seq = candidate;
                    } else {
                        idx += 1;
                    }
                }
                let shrunk_calls: Vec<String> = seq.into_iter().map(|(_, _, s)| s).collect();
                results.push(StatefulInvariantResult {
                    invariant_name: inv_name,
                    sequences_run: stateful_runs,
                    sequence_length: seq_len,
                    status: "INVARIANT_VIOLATED".to_string(),
                    shrunk_call_sequence: Some(shrunk_calls),
                });
            } else {
                results.push(StatefulInvariantResult {
                    invariant_name: inv_name,
                    sequences_run: stateful_runs,
                    sequence_length: seq_len,
                    status: "INVARIANT_HELD".to_string(),
                    shrunk_call_sequence: None,
                });
            }
        }
    }

    results
}

/// Storage slots keyed by their hex base address, with the recovered field name when the
/// program's names let us invert the keccak; entry points by short name with (contract, arity);
/// and the entry points that enforce a caller or signature guard.
fn extract_sierra_upgrade_signature(
    program: &Program,
    crate_name: &str,
) -> (
    BTreeMap<String, Option<String>>,
    BTreeMap<String, (String, usize)>,
    BTreeSet<String>,
) {
    let index = SierraProgramIndex::build(program);
    let summaries = compute_function_summaries(&index);

    let mut keccak_reverse: HashMap<String, String> = HashMap::new();
    #[cfg(feature = "native-compile")]
    {
        for common in [
            "owner",
            "fee_bps",
            "admin",
            "governance",
            "paused",
            "total_supply",
            "balances",
            "allowances",
            "nonce",
            "public_key",
            "implementation",
        ] {
            let h = cairo_lang_starknet_classes::keccak::starknet_keccak(common.as_bytes());
            keccak_reverse.insert(format!("0x{h:x}"), common.to_string());
        }
        for func_idx in 0..program.funcs.len() {
            let fname = index.func_name(func_idx);
            for part in fname.split("::") {
                let clean = part.split('[').next().unwrap_or(part);
                if !clean.is_empty() {
                    let h = cairo_lang_starknet_classes::keccak::starknet_keccak(clean.as_bytes());
                    keccak_reverse.insert(format!("0x{h:x}"), clean.to_string());
                }
            }
        }
    }

    let mut storage_slots: BTreeMap<String, Option<String>> = BTreeMap::new();
    for ld in &program.libfunc_declarations {
        if ld.long_id.generic_id.0.as_str() == "storage_base_address_const" {
            for arg in &ld.long_id.generic_args {
                if let GenericArg::Value(v) = arg {
                    let hex_slot = format!("0x{:x}", v);
                    let name = keccak_reverse.get(&hex_slot).cloned();
                    storage_slots.entry(hex_slot).or_insert(name);
                }
            }
        }
    }

    let mut entrypoints: BTreeMap<String, (String, usize)> = BTreeMap::new();
    let mut protected_entrypoints = BTreeSet::new();
    for func_idx in 0..program.funcs.len() {
        let fname = index.func_name(func_idx);
        if !fname.starts_with(&format!("{crate_name}::")) {
            continue;
        }
        if fname.contains("::__wrapper__") || fname.contains("::__external::") {
            let (contract, short) = extract_contract_and_short_name(&fname);
            if short == "constructor" {
                continue;
            }
            #[cfg(feature = "native-test-runner")]
            let arity = count_wrapper_calldata_pops(&index, func_idx);
            #[cfg(not(feature = "native-test-runner"))]
            let arity = 0usize;
            entrypoints.insert(short.clone(), (contract, arity));
            if summaries[func_idx].enforces_caller_guard
                || summaries[func_idx].enforces_signature_guard
            {
                protected_entrypoints.insert(short);
            }
        }
    }

    (storage_slots, entrypoints, protected_entrypoints)
}

#[cfg(feature = "native-test-runner")]
fn check_upgrade_safety_against_baseline(
    baseline_manifest_arg: &Path,
    candidate_program: &Program,
    candidate_crate_name: &str,
    features: &[String],
    no_cache: bool,
) -> Result<UpgradeSafetyReport> {
    let base_manifest = if baseline_manifest_arg.is_dir() {
        baseline_manifest_arg.join("Scarb.toml")
    } else {
        baseline_manifest_arg.to_path_buf()
    };
    let base_ws = metadata_cache_workspace_root(&base_manifest)?;
    let base_text = read_text_file_with_limit(&base_manifest, MAX_MANIFEST_BYTES, "manifest")?;
    let base_toml = parse_manifest_toml(&base_text, &base_manifest, "manifest")?;
    let base_pkg = base_toml
        .get("package")
        .and_then(TomlValue::as_table)
        .and_then(|p| p.get("name"))
        .and_then(TomlValue::as_str)
        .unwrap_or("baseline")
        .to_string();
    let base_crate = normalize_package_name_for_cairo_crate(&base_pkg);

    let (base_comp, _, _) = compile_or_load_native_tests(
        &base_manifest,
        &base_ws,
        &base_pkg,
        &base_crate,
        features,
        no_cache,
    )?;

    Ok(compare_upgrade_signatures(
        &base_comp.sierra_program.program,
        &base_crate,
        candidate_program,
        candidate_crate_name,
        base_manifest.display().to_string(),
        None,
    ))
}

/// Checks a candidate class against the class deployed at `target` (address or class hash):
/// the baseline is fetched over RPC and named from its ABI, so the comparison works on classes
/// without debug info.
#[cfg(feature = "native-test-runner")]
pub(crate) fn check_upgrade_safety_against_onchain(
    target: &str,
    rpc: &str,
    candidate_program: &Program,
    candidate_crate_name: &str,
) -> Result<UpgradeSafetyReport> {
    let onchain = fetch_onchain_sierra_program(rpc, target)?;
    Ok(compare_upgrade_signatures(
        &onchain.program,
        &onchain.crate_name,
        candidate_program,
        candidate_crate_name,
        target.to_string(),
        Some(onchain.class_hash),
    ))
}

/// Compares two programs' upgrade signatures: storage slots that disappear, entry points that
/// disappear or change calldata arity, and caller / signature guards that are lost.
pub(crate) fn compare_upgrade_signatures(
    base_program: &Program,
    base_crate: &str,
    candidate_program: &Program,
    candidate_crate_name: &str,
    baseline_label: String,
    baseline_class_hash: Option<String>,
) -> UpgradeSafetyReport {
    let (base_slots, base_eps, base_protected) =
        extract_sierra_upgrade_signature(base_program, base_crate);
    let (cand_slots, cand_eps, cand_protected) =
        extract_sierra_upgrade_signature(candidate_program, candidate_crate_name);

    // Slots compare by address; the label comes from whichever side could name it, so a
    // baseline without debug names never makes a renamed-but-identical slot look removed.
    let removed_storage_slots: Vec<String> = base_slots
        .iter()
        .filter(|(hex, _)| !cand_slots.contains_key(*hex))
        .map(|(hex, name)| match name.as_ref().or_else(|| cand_slots.get(hex).and_then(|n| n.as_ref())) {
            Some(n) => format!("{n} ({hex})"),
            None => hex.clone(),
        })
        .collect();
    let mut removed_entrypoints = Vec::new();
    let mut arity_mismatches = Vec::new();
    for (ep_short, (base_contract, base_arity)) in &base_eps {
        match cand_eps.get(ep_short) {
            None => removed_entrypoints.push(format!("{base_contract}::{ep_short}")),
            Some((cand_contract, cand_arity)) if cand_arity != base_arity => {
                arity_mismatches.push(format!(
                    "{cand_contract}::{ep_short}: baseline arity {base_arity} -> candidate arity {cand_arity}"
                ));
            }
            _ => {}
        }
    }
    let mut auth_regressions = Vec::new();
    for ep_short in &base_protected {
        if let Some((cand_contract, _)) = cand_eps.get(ep_short) {
            if !cand_protected.contains(ep_short) {
                auth_regressions.push(format!(
                    "entrypoint `{cand_contract}::{ep_short}` enforced caller/signature auth in baseline but is unprotected in candidate"
                ));
            }
        }
    }

    let is_safe = removed_storage_slots.is_empty()
        && removed_entrypoints.is_empty()
        && arity_mismatches.is_empty()
        && auth_regressions.is_empty();

    UpgradeSafetyReport {
        baseline_manifest: baseline_label,
        baseline_class_hash,
        status: if is_safe {
            "UPGRADE_SAFE".to_string()
        } else {
            "UPGRADE_UNSAFE".to_string()
        },
        baseline_storage_slots: base_slots.len(),
        candidate_storage_slots: cand_slots.len(),
        removed_storage_slots,
        removed_entrypoints,
        arity_mismatches,
        auth_regressions,
    }
}

#[cfg(feature = "native-test-runner")]
fn check_functional_equivalence_against_baseline(
    baseline_manifest_arg: &Path,
    candidate_compilation: &cairo_lang_test_plugin::TestCompilation<'static>,
    candidate_crate_name: &str,
    candidate_proofs: &[SymbolicFunctionProofReport],
    features: &[String],
    no_cache: bool,
) -> Result<FunctionalEquivalenceReport> {
    use cairo_lang_runner::casm_run::format_for_panic;
    use cairo_lang_runner::{Arg, RunResultValue, SierraCasmRunner, StarknetState};
    use cairo_lang_sierra_to_casm::metadata::MetadataComputationConfig;
    use starknet_types_core::felt::Felt as Felt252;

    let base_manifest = if baseline_manifest_arg.is_dir() {
        baseline_manifest_arg.join("Scarb.toml")
    } else {
        baseline_manifest_arg.to_path_buf()
    };
    let base_ws = metadata_cache_workspace_root(&base_manifest)?;
    let base_text = read_text_file_with_limit(&base_manifest, MAX_MANIFEST_BYTES, "manifest")?;
    let base_toml = parse_manifest_toml(&base_text, &base_manifest, "manifest")?;
    let base_pkg = base_toml
        .get("package")
        .and_then(TomlValue::as_table)
        .and_then(|p| p.get("name"))
        .and_then(TomlValue::as_str)
        .unwrap_or("baseline")
        .to_string();
    let base_crate = normalize_package_name_for_cairo_crate(&base_pkg);

    let (base_comp, _, _) = compile_or_load_native_tests(
        &base_manifest,
        &base_ws,
        &base_pkg,
        &base_crate,
        features,
        no_cache,
    )?;

    let base_prog = &base_comp.sierra_program.program;
    let cand_prog = &candidate_compilation.sierra_program.program;
    let (base_proofs, _) = analyze_sierra_program(base_prog, &base_crate, None, 1024, 1024, false, None);
    let base_index = SierraProgramIndex::build(base_prog);
    let cand_index = SierraProgramIndex::build(cand_prog);

    let base_runner = SierraCasmRunner::new(
        base_prog.clone(),
        Some(MetadataComputationConfig {
            function_set_costs: base_comp.metadata.function_set_costs.clone(),
            ..Default::default()
        }),
        base_comp.metadata.contracts_info.clone(),
        None,
    )?;
    let cand_runner = SierraCasmRunner::new(
        cand_prog.clone(),
        Some(MetadataComputationConfig {
            function_set_costs: candidate_compilation.metadata.function_set_costs.clone(),
            ..Default::default()
        }),
        candidate_compilation.metadata.contracts_info.clone(),
        None,
    )?;

    // Map short function names to (func_idx, is_wrapper, arity, proof)
    let collect_entrypoints = |index: &SierraProgramIndex<'_>,
                               crate_prefix: &str,
                               proofs: &[SymbolicFunctionProofReport]|
     -> BTreeMap<String, (usize, bool, usize, Option<SymbolicFunctionProofReport>)> {
        let mut map = BTreeMap::new();
        for func_idx in 0..index.program.funcs.len() {
            let fname = index.func_name(func_idx);
            if !fname.starts_with(&format!("{crate_prefix}::")) {
                continue;
            }
            let is_wrapper = fname.contains("::__wrapper__") || fname.contains("::__external::");
            let (_, short) = extract_contract_and_short_name(&fname);
            if short == "constructor" || short.starts_with("test_") {
                continue;
            }
            if is_wrapper || !map.contains_key(&short) {
                let arity = if is_wrapper {
                    count_wrapper_calldata_pops(index, func_idx)
                } else {
                    proofs
                        .iter()
                        .find(|p| p.function.ends_with(&format!("::{short}")))
                        .map(|p| p.params.len())
                        .unwrap_or(0)
                };
                let proof = proofs
                    .iter()
                    .find(|p| p.function.ends_with(&format!("::{short}")))
                    .cloned();
                map.insert(short, (func_idx, is_wrapper, arity, proof));
            }
        }
        map
    };

    let base_map = collect_entrypoints(&base_index, &base_crate, &base_proofs);
    let cand_map = collect_entrypoints(&cand_index, candidate_crate_name, candidate_proofs);

    let boundary_values: &[u128] = &[0, 1, 2, 3, 7, 10, 42, 100, 255, 1000, 10_000, 10_001, 65_535];
    let mut entries = Vec::new();

    for (short, (b_idx, b_wrap, b_arity, b_proof)) in &base_map {
        let Some((c_idx, c_wrap, c_arity, c_proof)) = cand_map.get(short) else {
            entries.push(FunctionalEquivalenceEntry {
                function: short.clone(),
                status: "DIVERGENCE_FOUND".to_string(),
                divergence_reason: Some("function removed in candidate".to_string()),
                witness_inputs: None,
                baseline_output: Some("present".to_string()),
                candidate_output: Some("missing".to_string()),
            });
            continue;
        };

        if b_arity != c_arity {
            entries.push(FunctionalEquivalenceEntry {
                function: short.clone(),
                status: "DIVERGENCE_FOUND".to_string(),
                divergence_reason: Some(format!(
                    "parameter arity mismatch: baseline takes {b_arity} felt(s), candidate takes {c_arity}"
                )),
                witness_inputs: None,
                baseline_output: Some(format!("arity={b_arity}")),
                candidate_output: Some(format!("arity={c_arity}")),
            });
            continue;
        }

        // Check symbolic precondition & panic-path equivalence first
        if let (Some(bp), Some(cp)) = (b_proof, c_proof) {
            if bp.preconditions != cp.preconditions || bp.status != cp.status {
                // Probe boundary witnesses in SierraCasmRunner to find a concrete diverging witness
                let mut found_witness = None;
                for &v0 in boundary_values {
                    let witness_words = vec![v0; *b_arity];
                    let make_args = |is_wrap: bool| -> Vec<Arg> {
                        let felts: Vec<Arg> = witness_words
                            .iter()
                            .copied()
                            .map(|w| Arg::Value(Felt252::from(w)))
                            .collect();
                        if is_wrap {
                            vec![Arg::Array(felts)]
                        } else {
                            felts
                        }
                    };
                    let b_res = base_runner.run_function_with_starknet_context(
                        &base_prog.funcs[*b_idx],
                        make_args(*b_wrap),
                        Some(10_000_000),
                        StarknetState::default(),
                    );
                    let c_res = cand_runner.run_function_with_starknet_context(
                        &cand_prog.funcs[*c_idx],
                        make_args(*c_wrap),
                        Some(10_000_000),
                        StarknetState::default(),
                    );
                    if let (Ok(br), Ok(cr)) = (b_res, c_res) {
                        let b_str = match &br.value {
                            RunResultValue::Success(vals) => format!("Ok({vals:?})"),
                            RunResultValue::Panic(p) => format!("Panic({})", format_for_panic(p.iter().cloned())),
                        };
                        let c_str = match &cr.value {
                            RunResultValue::Success(vals) => format!("Ok({vals:?})"),
                            RunResultValue::Panic(p) => format!("Panic({})", format_for_panic(p.iter().cloned())),
                        };
                        if b_str != c_str {
                            found_witness = Some((
                                witness_words.iter().map(|w| w.to_string()).collect::<Vec<_>>(),
                                b_str,
                                c_str,
                            ));
                            break;
                        }
                    }
                }
                if let Some((w_in, b_out, c_out)) = found_witness {
                    entries.push(FunctionalEquivalenceEntry {
                        function: short.clone(),
                        status: "DIVERGENCE_FOUND".to_string(),
                        divergence_reason: Some(
                            "concrete execution diverged on boundary witness".to_string(),
                        ),
                        witness_inputs: Some(w_in),
                        baseline_output: Some(b_out),
                        candidate_output: Some(c_out),
                    });
                    continue;
                }
                entries.push(FunctionalEquivalenceEntry {
                    function: short.clone(),
                    status: "DIVERGENCE_FOUND".to_string(),
                    divergence_reason: Some(format!(
                        "symbolic preconditions diverged: baseline {:?} vs candidate {:?}",
                        bp.preconditions, cp.preconditions
                    )),
                    witness_inputs: None,
                    baseline_output: Some(format!("{:?}", bp.preconditions)),
                    candidate_output: Some(format!("{:?}", cp.preconditions)),
                });
                continue;
            }
        }

        // Sweep boundary witnesses in SierraCasmRunner even when preconditions look similar
        let mut concrete_divergence = None;
        for &v0 in boundary_values {
            let witness_words = vec![v0; *b_arity];
            let make_args = |is_wrap: bool| -> Vec<Arg> {
                let felts: Vec<Arg> = witness_words
                    .iter()
                    .copied()
                    .map(|w| Arg::Value(Felt252::from(w)))
                    .collect();
                if is_wrap {
                    vec![Arg::Array(felts)]
                } else {
                    felts
                }
            };
            let b_res = base_runner.run_function_with_starknet_context(
                &base_prog.funcs[*b_idx],
                make_args(*b_wrap),
                Some(10_000_000),
                StarknetState::default(),
            );
            let c_res = cand_runner.run_function_with_starknet_context(
                &cand_prog.funcs[*c_idx],
                make_args(*c_wrap),
                Some(10_000_000),
                StarknetState::default(),
            );
            if let (Ok(br), Ok(cr)) = (b_res, c_res) {
                let b_str = match &br.value {
                    RunResultValue::Success(vals) => format!("Ok({vals:?})"),
                    RunResultValue::Panic(p) => format!("Panic({})", format_for_panic(p.iter().cloned())),
                };
                let c_str = match &cr.value {
                    RunResultValue::Success(vals) => format!("Ok({vals:?})"),
                    RunResultValue::Panic(p) => format!("Panic({})", format_for_panic(p.iter().cloned())),
                };
                if b_str != c_str {
                    concrete_divergence = Some((
                        witness_words.iter().map(|w| w.to_string()).collect::<Vec<_>>(),
                        b_str,
                        c_str,
                    ));
                    break;
                }
            }
        }

        if let Some((w_in, b_out, c_out)) = concrete_divergence {
            entries.push(FunctionalEquivalenceEntry {
                function: short.clone(),
                status: "DIVERGENCE_FOUND".to_string(),
                divergence_reason: Some("return value or panic behavior diverged".to_string()),
                witness_inputs: Some(w_in),
                baseline_output: Some(b_out),
                candidate_output: Some(c_out),
            });
        } else {
            entries.push(FunctionalEquivalenceEntry {
                function: short.clone(),
                status: "PROVEN_EQUIVALENT".to_string(),
                divergence_reason: None,
                witness_inputs: None,
                baseline_output: None,
                candidate_output: None,
            });
        }
    }

    let equivalent_count = entries
        .iter()
        .filter(|e| e.status == "PROVEN_EQUIVALENT")
        .count();
    let divergent_count = entries
        .iter()
        .filter(|e| e.status == "DIVERGENCE_FOUND")
        .count();

    Ok(FunctionalEquivalenceReport {
        baseline_manifest: base_manifest.display().to_string(),
        status: if divergent_count == 0 {
            "EQUIVALENT".to_string()
        } else {
            "NON_EQUIVALENT".to_string()
        },
        equivalent_count,
        divergent_count,
        functions: entries,
    })
}

fn write_sarif_report(sarif_path: &Path, report: &ProveWorkspaceReport) -> Result<()> {
    let mut results_json = Vec::new();
    for pkg in &report.packages {
        for f in &pkg.caracal_findings {
            let level = match f.severity.as_str() {
                "Critical" | "High" => "error",
                "Medium" => "warning",
                _ => "note",
            };
            results_json.push(serde_json::json!({
                "ruleId": f.detector,
                "level": level,
                "message": {
                    "text": format!("{} ({})", f.description, f.recommendation)
                },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": {
                            "uri": pkg.manifest_path
                        },
                        "region": {
                            "startLine": 1
                        }
                    },
                    "logicalLocations": [{
                        "name": f.function,
                        "fullyQualifiedName": format!("{}::{}", f.contract_or_module, f.function),
                        "kind": "function"
                    }]
                }]
            }));
        }
        for p in &pkg.symbolic_proofs {
            if p.status == "COUNTEREXAMPLE_FOUND" {
                results_json.push(serde_json::json!({
                    "ruleId": "symbolic-counterexample",
                    "level": "error",
                    "message": {
                        "text": format!(
                            "Symbolic execution found panic counterexample in `{}`: {:?} ({})",
                            p.function,
                            p.counterexample,
                            p.counterexample_reason.as_deref().unwrap_or("panic")
                        )
                    },
                    "locations": [{
                        "physicalLocation": {
                            "artifactLocation": {
                                "uri": pkg.manifest_path
                            },
                            "region": {
                                "startLine": 1
                            }
                        },
                        "logicalLocations": [{
                            "name": p.function,
                            "kind": "function"
                        }]
                    }]
                }));
            }
        }
        for inv in &pkg.stateful_invariants {
            if inv.status == "INVARIANT_VIOLATED" {
                let seq_str = inv
                    .shrunk_call_sequence
                    .as_ref()
                    .map(|s| s.join(" -> "))
                    .unwrap_or_default();
                results_json.push(serde_json::json!({
                    "ruleId": "medusa-stateful-invariant-violated",
                    "level": "error",
                    "message": {
                        "text": format!(
                            "Stateful contract invariant `{}` violated by shrunk call sequence: [{}]",
                            inv.invariant_name, seq_str
                        )
                    },
                    "locations": [{
                        "physicalLocation": {
                            "artifactLocation": { "uri": pkg.manifest_path },
                            "region": { "startLine": 1 }
                        },
                        "logicalLocations": [{
                            "name": inv.invariant_name,
                            "kind": "function"
                        }]
                    }]
                }));
            }
        }
        if let Some(upg) = &pkg.upgrade_safety {
            if upg.status == "UPGRADE_UNSAFE" {
                results_json.push(serde_json::json!({
                    "ruleId": "obelisk-upgrade-unsafe",
                    "level": "error",
                    "message": {
                        "text": format!(
                            "Unsafe contract upgrade vs baseline `{}`: removed_slots={:?}, auth_regressions={:?}, arity_mismatches={:?}",
                            upg.baseline_manifest,
                            upg.removed_storage_slots,
                            upg.auth_regressions,
                            upg.arity_mismatches
                        )
                    },
                    "locations": [{
                        "physicalLocation": {
                            "artifactLocation": { "uri": pkg.manifest_path },
                            "region": { "startLine": 1 }
                        }
                    }]
                }));
            }
        }
    }

    let sarif_doc = serde_json::json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "uc-prove-caracal",
                    "informationUri": "https://github.com/omarespejel/uc",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            },
            "results": results_json
        }]
    });

    write_json_report(sarif_path, &sarif_doc)
}

#[cfg(feature = "native-test-runner")]
fn prove_single_package(
    manifest_path: &Path,
    workspace_root: &Path,
    package_name: &str,
    crate_name: &str,
    args: &ProveArgs,
) -> Result<PackageProveReport> {
    use cairo_lang_runner::SierraCasmRunner;
    use cairo_lang_sierra_to_casm::metadata::MetadataComputationConfig;

    let (compilation, _cache_hit, _compile_ms) = compile_or_load_native_tests(
        manifest_path,
        workspace_root,
        package_name,
        crate_name,
        &args.features,
        args.no_cache,
    )?;

    let program = &compilation.sierra_program.program;
    let (mut symbolic_proofs, caracal_findings) = analyze_sierra_program(
        program,
        crate_name,
        args.filter.as_deref(),
        args.max_depth,
        args.max_paths,
        args.caracal,
        None,
    );
    // A `#[should_panic]` test exists to panic: a satisfiable panic path there is the test
    // doing its job, not a counterexample.
    let package_dir = manifest_path.parent().unwrap_or(workspace_root);
    let expected_panic_tests: HashSet<String> = ["src", "tests"]
        .iter()
        .flat_map(|dir| discover_source_test_locations(workspace_root, &package_dir.join(dir)))
        .filter(|(_, (_, _, should_panic))| *should_panic)
        .map(|(name, _)| name)
        .collect();
    if !expected_panic_tests.is_empty() {
        symbolic_proofs.retain(|p| {
            let short = p.function.rsplit("::").next().unwrap_or(&p.function);
            !(p.status == "COUNTEREXAMPLE_FOUND" && expected_panic_tests.contains(short))
        });
    }

    let has_counterexamples = symbolic_proofs
        .iter()
        .any(|p| p.status == "COUNTEREXAMPLE_FOUND");
    let has_invariant_candidates = args.stateful_runs > 0
        && (compilation
            .metadata
            .named_tests
            .iter()
            .any(|(n, _)| n.contains("invariant_") || n.contains("stateful_"))
            || program.funcs.iter().any(|f| {
                f.id.debug_name.as_ref().is_some_and(|dn| {
                    let s = dn.as_str();
                    s.starts_with(&format!("{crate_name}::"))
                        && (s.contains("invariant_") || s.contains("echidna_"))
                })
            }));

    if has_counterexamples {
        replay_counterexamples_on_slice(program, &mut symbolic_proofs, &[]);
    }
    let mut stateful_invariants = Vec::new();
    if has_invariant_candidates && program.statements.len() <= 8_000 {
        let metadata_cfg = if program.statements.len() <= 2_500 {
            Some(MetadataComputationConfig {
                function_set_costs: compilation.metadata.function_set_costs.clone(),
                ..Default::default()
            })
        } else {
            None
        };
        if let Ok(runner) = SierraCasmRunner::new(
            program.clone(),
            metadata_cfg,
            compilation.metadata.contracts_info.clone(),
            None,
        ) {
            {
                let index = SierraProgramIndex::build(program);
                stateful_invariants = run_medusa_stateful_campaign(
                    &runner,
                    &index,
                    crate_name,
                    &compilation.metadata.named_tests,
                    args.stateful_runs,
                );
            }
        }
    }

    let upgrade_safety = if let Some(baseline) = args.check_upgrade.as_deref().filter(|b| b.starts_with("0x")) {
        let rpc = resolve_read_rpc_url(args.url.as_deref(), args.network.as_deref())?;
        Some(check_upgrade_safety_against_onchain(baseline, &rpc, program, crate_name)?)
    } else if let Some(baseline_path) = args.check_upgrade.as_deref().map(Path::new) {
        Some(check_upgrade_safety_against_baseline(
            baseline_path,
            program,
            crate_name,
            &args.features,
            args.no_cache,
        )?)
    } else {
        None
    };

    let equivalence = if let Some(eq_baseline_path) = &args.check_equivalence {
        Some(check_functional_equivalence_against_baseline(
            eq_baseline_path,
            &compilation,
            crate_name,
            &symbolic_proofs,
            &args.features,
            args.no_cache,
        )?)
    } else {
        None
    };

    let proven_safe_count = symbolic_proofs
        .iter()
        .filter(|p| p.status == "PROVEN_SAFE")
        .count();
    let guarded_safe_count = symbolic_proofs
        .iter()
        .filter(|p| p.status == "GUARDED_SAFE")
        .count();
    let counterexample_count = symbolic_proofs
        .iter()
        .filter(|p| p.status == "COUNTEREXAMPLE_FOUND")
        .count();
    let undecided_count = symbolic_proofs.iter().filter(|p| p.status == "UNDECIDED").count();
    let bounded_safe_count = symbolic_proofs.iter().filter(|p| p.status == "BOUNDED_SAFE").count();
    if let Some(dir) = &args.emit_smt {
        write_smt_obligations(&dir.join(package_name), &mut symbolic_proofs)?;
    }

    Ok(PackageProveReport {
        package_name: package_name.to_string(),
        manifest_path: manifest_path.display().to_string(),
        sierra_functions_analyzed: symbolic_proofs.len(),
        sierra_statements_analyzed: program.statements.len(),
        proven_safe_count,
        guarded_safe_count,
        counterexample_count,
        undecided_count,
        bounded_safe_count,
        caracal_findings_count: caracal_findings.len(),
        symbolic_proofs,
        caracal_findings,
        stateful_invariants,
        upgrade_safety,
        equivalence,
    })
}

pub(crate) fn run_prove(args: ProveArgs) -> Result<()> {
    FUNCTION_TIMEOUT_MS.store(args.function_timeout_ms.max(1), std::sync::atomic::Ordering::Relaxed);
    KEEP_OBLIGATIONS.store(args.emit_smt.is_some(), std::sync::atomic::Ordering::Relaxed);
    let start = Instant::now();
    let mut pkg_reports = Vec::new();

    if let Some(sierra_file) = &args.sierra_file {
        let mut text = fs::read_to_string(sierra_file)
            .with_context(|| format!("failed to read {}", sierra_file.display()))?;
        // Classes saved from JSON-RPC carry the ABI as a string; the class type wants the array.
        if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(abi_text) = v.get("abi").and_then(|a| a.as_str()) {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(abi_text) {
                    v["abi"] = parsed;
                    text = v.to_string();
                }
            }
        }
        #[cfg(feature = "native-compile")]
        {
            let program = if let Ok(versioned) =
                serde_json::from_str::<cairo_lang_sierra::program::VersionedProgram>(&text)
            {
                versioned.into_v1()?.program
            } else if let Ok(prog) =
                serde_json::from_str::<cairo_lang_sierra::program::Program>(&text)
            {
                prog
            } else {
                let cc: ContractClass = serde_json::from_str(&text).with_context(|| {
                    format!(
                        "failed to parse {} as Sierra program or Starknet ContractClass",
                        sierra_file.display()
                    )
                })?;
                let mut program = cc
                    .extract_sierra_program(true)
                    .map_err(|e| anyhow!("failed to extract Sierra program: {e}"))?
                    .program;
                // Classes without debug info (deployed, or built with `sierra-replace-ids = false`)
                // get analyzer-friendly names from their entry points and ABI.
                #[cfg(feature = "native-test-runner")]
                {
                    let abi_json = serde_json::from_str::<serde_json::Value>(&text)
                        .ok()
                        .and_then(|v| v.get("abi").cloned());
                    synthesize_contract_function_names(
                        &mut program,
                        &cc.entry_points_by_type,
                        abi_json.as_ref(),
                    );
                }
                program
            };
            let inferred_crate = program
                .funcs
                .iter()
                .filter_map(|f| f.id.debug_name.as_ref().map(|d| d.as_str()))
                .find(|dn| !dn.starts_with("core::"))
                .and_then(|dn| dn.split("::").next())
                .unwrap_or("sierra_target")
                .to_string();
            let (mut symbolic_proofs, caracal_findings) = analyze_sierra_program(
                &program,
                &inferred_crate,
                args.filter.as_deref(),
                args.max_depth,
                args.max_paths,
                args.caracal,
                None,
            );
            if symbolic_proofs.iter().any(|p| p.status == "COUNTEREXAMPLE_FOUND") {
                replay_counterexamples_on_slice(&program, &mut symbolic_proofs, &[]);
            }
            let undecided_count = symbolic_proofs.iter().filter(|p| p.status == "UNDECIDED").count();
            let bounded_safe_count = symbolic_proofs.iter().filter(|p| p.status == "BOUNDED_SAFE").count();
            if let Some(dir) = &args.emit_smt {
                write_smt_obligations(&dir.join(&inferred_crate), &mut symbolic_proofs)?;
            }
            let proven_safe_count = symbolic_proofs
                .iter()
                .filter(|p| p.status == "PROVEN_SAFE")
                .count();
            let guarded_safe_count = symbolic_proofs
                .iter()
                .filter(|p| p.status == "GUARDED_SAFE")
                .count();
            let counterexample_count = symbolic_proofs
                .iter()
                .filter(|p| p.status == "COUNTEREXAMPLE_FOUND")
                .count();
            pkg_reports.push(PackageProveReport {
                package_name: inferred_crate,
                manifest_path: sierra_file.display().to_string(),
                sierra_functions_analyzed: symbolic_proofs.len(),
                sierra_statements_analyzed: program.statements.len(),
                proven_safe_count,
                guarded_safe_count,
                counterexample_count,
                undecided_count,
                bounded_safe_count,
                caracal_findings_count: caracal_findings.len(),
                symbolic_proofs,
                caracal_findings,
                stateful_invariants: Vec::new(),
                upgrade_safety: None,
                equivalence: None,
            });
        }
    } else {
        let manifest_path = resolve_manifest_path(&args.manifest_path)?;
        let workspace_root = metadata_cache_workspace_root(&manifest_path)?;

        let is_virtual_workspace =
            read_text_file_with_limit(&manifest_path, MAX_MANIFEST_BYTES, "manifest")
                .ok()
                .and_then(|txt| parse_manifest_toml(&txt, &manifest_path, "manifest").ok())
                .is_some_and(|m| m.get("package").is_none() && m.get("workspace").is_some());

        let target_manifests: Vec<PathBuf> = if (args.workspace || is_virtual_workspace)
            && args.package.is_none()
        {
            let members = resolve_all_workspace_member_manifests(&manifest_path, &workspace_root)?;
            if members.is_empty() {
                vec![manifest_path.clone()]
            } else {
                members
            }
        } else {
            let effective = resolve_native_effective_manifest_path(
                &manifest_path,
                &workspace_root,
                args.package.as_deref(),
            )
            .unwrap_or_else(|_| manifest_path.clone());
            vec![effective]
        };

        for pkg_manifest in target_manifests {
            let manifest_text =
                read_text_file_with_limit(&pkg_manifest, MAX_MANIFEST_BYTES, "manifest")?;
            let manifest = parse_manifest_toml(&manifest_text, &pkg_manifest, "manifest")?;
            let pkg_name = manifest
                .get("package")
                .and_then(TomlValue::as_table)
                .and_then(|p| p.get("name"))
                .and_then(TomlValue::as_str)
                .unwrap_or("package")
                .to_string();
            let crate_name = normalize_package_name_for_cairo_crate(&pkg_name);
            #[cfg(feature = "native-test-runner")]
            {
                match with_native_compat_toolchain_fallback(|| {
                    prove_single_package(
                        &pkg_manifest,
                        &workspace_root,
                        &pkg_name,
                        &crate_name,
                        &args,
                    )
                }) {
                    Ok(rep) => pkg_reports.push(rep),
                    Err(err) => {
                        if !args.workspace {
                            return Err(err);
                        }
                    }
                }
            }
        }
    }

    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    let total_functions_analyzed: usize = pkg_reports
        .iter()
        .map(|p| p.sierra_functions_analyzed)
        .sum();
    let total_proven_safe: usize = pkg_reports.iter().map(|p| p.proven_safe_count).sum();
    let total_guarded_safe: usize = pkg_reports.iter().map(|p| p.guarded_safe_count).sum();
    let total_counterexamples: usize = pkg_reports.iter().map(|p| p.counterexample_count).sum();
    let total_caracal_findings: usize = pkg_reports.iter().map(|p| p.caracal_findings_count).sum();

    let report = ProveWorkspaceReport {
        schema_version: 1,
        engine: "uc-prove-caracal-sierra-v1".to_string(),
        sierra_ir_version: format!("{} (Cairo {})", cairo_lang_starknet_classes::compiler_version::current_sierra_version_id(), native_cairo_lang_compiler_version()),
        elapsed_ms,
        total_functions_analyzed,
        total_proven_safe,
        total_guarded_safe,
        total_counterexamples,
        total_caracal_findings,
        packages: pkg_reports,
    };

    if let Some(report_path) = &args.report_path {
        write_json_report(report_path, &report)?;
    }

    if let Some(sarif_path) = &args.sarif {
        write_sarif_report(sarif_path, &report)?;
    }

    if let Some(repro_path) = &args.emit_repro {
        let mut repro_content =
            String::from("// Auto-generated by `uc prove --emit-repro`\n#[cfg(test)]\nmod uc_symbolic_repros {\n");
        for pkg in &report.packages {
            for p in &pkg.symbolic_proofs {
                if let Some(snippet) = &p.repro_test {
                    for line in snippet.lines() {
                        repro_content.push_str("    ");
                        repro_content.push_str(line);
                        repro_content.push('\n');
                    }
                }
            }
        }
        repro_content.push_str("}\n");
        if let Some(parent) = repro_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(repro_path, repro_content)?;
    }

    for pkg in &report.packages {
        if pkg.sierra_functions_analyzed == 0 {
            eprintln!(
                "warning: no Sierra functions of `{}` were analysed ({} statements compiled); nothing is proven",
                pkg.package_name, pkg.sierra_statements_analyzed
            );
        }
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        for pkg in &report.packages {
            println!(
                "uc prove — Formal Verification & Sierra IR Analysis: {} ({} Sierra statements, {} functions)",
                pkg.package_name, pkg.sierra_statements_analyzed, pkg.sierra_functions_analyzed
            );
            for proof in &pkg.symbolic_proofs {
                let badge = match proof.status.as_str() {
                    "PROVEN_SAFE" => "[PROVEN SAFE]",
                    "GUARDED_SAFE" => "[GUARDED SAFE]",
                    "COUNTEREXAMPLE_FOUND" => "[COUNTEREXAMPLE]",
                    "UNDECIDED" => "[UNDECIDED]",
                    "BOUNDED_SAFE" => "[BOUNDED SAFE]",
                    _ => "[BOUNDED]",
                };
                println!(
                    "  {} {} (paths={}, ok={}, panic={}, unsat_panic={})",
                    badge,
                    proof.function,
                    proof.explored_paths,
                    proof.ok_paths,
                    proof.panic_paths,
                    proof.unreachable_panic_paths
                );
                if !proof.preconditions.is_empty() {
                    println!("      preconditions: {}", proof.preconditions.join(" && "));
                }
                if let Some(ce) = &proof.counterexample {
                    let formatted_ce = ce
                        .iter()
                        .map(|(k, v)| format!("{k} = {v}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let verified_tag = match proof.concrete_verified {
                        Some(true) => " [CONCRETE_EXECUTION_VERIFIED]",
                        Some(false) => " [UNVERIFIED_IN_RUNNER]",
                        None => "",
                    };
                    let panic_msg = proof
                        .concrete_panic_data
                        .as_deref()
                        .or(proof.counterexample_reason.as_deref())
                        .unwrap_or("panic");
                    println!(
                        "      counterexample: ({}) -> {}{}",
                        formatted_ce, panic_msg, verified_tag
                    );
                    if let Some(calldata) = &proof.counterexample_calldata {
                        println!("      calldata: [{}]", calldata.join(", "));
                    }
                }
                if let Some(smt) = &proof.smt_file {
                    println!("      smt: {smt}");
                }
            }
            if !pkg.stateful_invariants.is_empty() {
                println!(
                    "\n  Stateful Contract Invariants ({}):",
                    pkg.stateful_invariants.len()
                );
                for inv in &pkg.stateful_invariants {
                    let badge = if inv.status == "INVARIANT_VIOLATED" {
                        "[INVARIANT VIOLATED]"
                    } else {
                        "[INVARIANT HELD]"
                    };
                    println!(
                        "    {} {} (sequences={}, seq_len={})",
                        badge, inv.invariant_name, inv.sequences_run, inv.sequence_length
                    );
                    if let Some(seq) = &inv.shrunk_call_sequence {
                        println!("      shrunk sequence: {}", seq.join(" -> "));
                    }
                }
            }
            if let Some(upg) = &pkg.upgrade_safety {
                println!(
                    "\n  Formal Upgrade & Storage Compatibility ({}): [{}]",
                    upg.baseline_manifest, upg.status
                );
                for c in &upg.removed_storage_slots {
                    println!("    [STORAGE SLOT REMOVED] {c}");
                }
                for a in &upg.removed_entrypoints {
                    println!("    [ENTRYPOINT REMOVED] {a}");
                }
                for m in &upg.arity_mismatches {
                    println!("    [ABI ARITY MISMATCH] {m}");
                }
                for r in &upg.auth_regressions {
                    println!("    [AUTH REGRESSION] {r}");
                }
            }
            if let Some(eq) = &pkg.equivalence {
                println!(
                    "\n  Formal Functional Equivalence vs `{}`: [{}] ({} equivalent, {} divergent)",
                    eq.baseline_manifest, eq.status, eq.equivalent_count, eq.divergent_count
                );
                for fn_eq in &eq.functions {
                    println!("    [{}] {}", fn_eq.status, fn_eq.function);
                    if let Some(reason) = &fn_eq.divergence_reason {
                        println!("      reason: {reason}");
                    }
                    if let (Some(b_out), Some(c_out)) = (&fn_eq.baseline_output, &fn_eq.candidate_output) {
                        println!("      baseline: {b_out} | candidate: {c_out}");
                    }
                }
            }
            if !pkg.caracal_findings.is_empty() {
                println!(
                    "\n  Sierra IR Static & Taint Findings ({}):",
                    pkg.caracal_findings.len()
                );
                for f in &pkg.caracal_findings {
                    println!(
                        "    [{}][{}] {}::{} (stmt #{}) - {}",
                        f.severity,
                        f.detector,
                        f.contract_or_module,
                        f.function,
                        f.sierra_statement_idx,
                        f.description
                    );
                }
            }
        }
        println!(
            "\nSummary: {} proven safe, {} guarded safe, {} counterexamples, {} Sierra IR findings ({:.1}ms)",
            report.total_proven_safe,
            report.total_guarded_safe,
            report.total_counterexamples,
            report.total_caracal_findings,
            report.elapsed_ms
        );
    }

    let any_invariant_violated = report.packages.iter().any(|p| {
        p.stateful_invariants
            .iter()
            .any(|i| i.status == "INVARIANT_VIOLATED")
    });
    let any_upgrade_unsafe = report.packages.iter().any(|p| {
        p.upgrade_safety
            .as_ref()
            .is_some_and(|u| u.status == "UPGRADE_UNSAFE")
    });
    let any_non_equivalent = report.packages.iter().any(|p| {
        p.equivalence
            .as_ref()
            .is_some_and(|e| e.status == "NON_EQUIVALENT")
    });

    if args.fail_on_findings
        && (total_counterexamples > 0
            || total_caracal_findings > 0
            || any_invariant_violated
            || any_upgrade_unsafe
            || any_non_equivalent)
    {
        std::process::exit(1);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn linear_bounds_round_toward_the_integer_solutions() {
        use super::*;
        // 2p - 255 <= 0  ->  p <= 127;  2p > 0  ->  p >= 1;  3p - 7 < 0  ->  p <= 2;  -2p + 9 >= 0  ->  p <= 4
        assert_eq!(linear_bound(2, -255, CmpOp::Le), Some((CmpOp::Le, 127)));
        assert_eq!(linear_bound(2, 0, CmpOp::Gt), Some((CmpOp::Ge, 1)));
        assert_eq!(linear_bound(3, -7, CmpOp::Lt), Some((CmpOp::Le, 2)));
        assert_eq!(linear_bound(-2, 9, CmpOp::Ge), Some((CmpOp::Le, 4)));
        // 2p - 7 == 0 has no integer solution; 2p - 8 == 0 has p = 4.
        assert_eq!(linear_bound(2, -7, CmpOp::Eq), Some((CmpOp::Lt, i128::MIN)));
        assert_eq!(linear_bound(2, -8, CmpOp::Eq), Some((CmpOp::Eq, 4)));
        // The betshare loop: p0 + 2 > 255, p0 < 2·p1 - 1, 2·p1 <= 255 is unsatisfiable over u8.
        let p0 = SymExpr::Param(0, "a".into());
        let p1 = SymExpr::Param(1, "b".into());
        let two_p1 = SymExpr::Mul(Box::new(p1.clone()), Box::new(SymExpr::Const(2)));
        let mk = |lhs: SymExpr, op, rhs: SymExpr| SymConstraint { lhs, op, rhs, reason: String::new(), is_overflow_branch: false };
        let constraints = vec![
            mk(two_p1.clone(), CmpOp::Le, SymExpr::Const(255)),
            mk(p0.clone(), CmpOp::Lt, SymExpr::Sub(Box::new(two_p1), Box::new(SymExpr::Const(1)))),
            mk(SymExpr::Add(Box::new(p0), Box::new(SymExpr::Const(2))), CmpOp::Gt, SymExpr::Const(255)),
        ];
        let bounds = vec![("a".to_string(), 0, 255), ("b".to_string(), 0, 255)];
        assert!(matches!(solve_path_constraints(&bounds, &constraints), SolverOutcome::Unsat));
    }

    #[test]
    fn smt_export_renders_bounds_obligations_and_witness_blocks() {
        use super::*;
        let p0 = SymExpr::Param(0, "amount".to_string());
        let unsat = SymConstraint {
            lhs: p0.clone(),
            op: CmpOp::Gt,
            rhs: SymExpr::Const(300),
            reason: "overflow in u8_overflowing_add".to_string(),
            is_overflow_branch: true,
        };
        let sat = SymConstraint {
            lhs: SymExpr::Add(Box::new(p0.clone()), Box::new(SymExpr::SymbolicOpaque("storage read".to_string()))),
            op: CmpOp::Ge,
            rhs: SymExpr::Const(-1),
            reason: "line\nbreak".to_string(),
            is_overflow_branch: false,
        };
        let proof = SymbolicFunctionProofReport {
            function: "demo::Vault::deposit".to_string(),
            kind: "function".to_string(),
            params: vec!["amount: core::integer::u8".to_string()],
            status: "COUNTEREXAMPLE_FOUND".to_string(),
            explored_paths: 2,
            ok_paths: 0,
            panic_paths: 2,
            unreachable_panic_paths: 1,
            undecided_panic_paths: 0,
            exploration_truncated: false,
            proven_overflow_free: false,
            preconditions: Vec::new(),
            counterexample: None,
            counterexample_calldata: None,
            smt_file: None,
            obligations: vec![
                PathObligation { expected: "unsat", constraints: vec![unsat], witness: None, reason: "a".to_string() },
                PathObligation { expected: "sat", constraints: vec![sat], witness: Some(vec![7]), reason: "b".to_string() },
            ],
            param_bounds: vec![("amount".to_string(), 0, 255)],
            param_types: vec!["core::integer::u8".to_string()],
            counterexample_reason: None,
            concrete_verified: None,
            concrete_panic_data: None,
            repro_test: None,
        };
        let text = render_smt_obligations(&proof);
        assert!(text.contains("(set-logic QF_NIA)"));
        assert!(text.contains("(declare-const p0 Int) ; amount"));
        assert!(text.contains("(assert (and (<= 0 p0) (<= p0 255)))"));
        assert!(text.contains("(declare-const o_storage_read Int)"));
        assert!(text.contains("; obligation 0: expected unsat"));
        assert!(text.contains("(assert (> p0 300)) ; overflow in u8_overflowing_add"));
        assert!(text.contains("; obligation 1: expected sat"));
        assert!(text.contains("(assert (>= (+ p0 o_storage_read) (- 1))) ; line break"));
        assert!(text.contains("; obligation 1 witness: expected sat"));
        assert!(text.contains("(assert (= p0 7))"));
        assert_eq!(text.matches("(check-sat)").count(), 3);
        assert_eq!(text.matches("(push 1)").count(), text.matches("(pop 1)").count());

        let dir = std::env::temp_dir().join(format!("uc-smt-export-{}", std::process::id()));
        let mut proofs = vec![proof];
        let written = write_smt_obligations(&dir, &mut proofs).expect("write smt");
        assert_eq!(written, 1);
        assert_eq!(proofs[0].smt_file.as_deref(), Some(dir.join("demo__Vault__deposit.smt2").to_str().unwrap()));
        let manifest: Vec<serde_json::Value> =
            serde_json::from_slice(&fs::read(dir.join("obligations.json")).unwrap()).unwrap();
        assert_eq!(manifest.len(), 2);
        assert_eq!(manifest[1]["expected"], "sat");
        assert_eq!(manifest[1]["witness"][0], 7);
        let _ = fs::remove_dir_all(&dir);

        let long_name = format!("demo::{}::f", "x".repeat(300));
        let stem = smt_file_stem(&long_name);
        assert!(stem.len() < 120, "{stem}");
        assert_ne!(stem, smt_file_stem(&format!("{long_name}2")));
    }

    #[test]
    fn concrete_panic_confirmation_rejects_harness_artefacts_and_unrelated_asserts() {
        use super::concrete_panic_confirms_witness;
        let overflow = "arithmetic overflow in `u64_overflowing_add`: (a + b) > 18446744073709551615";
        assert!(concrete_panic_confirms_witness(overflow, "Panicked with 'u64_add Overflow'."));
        assert!(!concrete_panic_confirms_witness(overflow, "Panicked with 'clientAddresses required'."));
        assert!(!concrete_panic_confirms_witness(overflow, "Panicked with 'Syscall out of gas'."));
        assert!(!concrete_panic_confirms_witness("panic path", "Failed to deserialize param #2"));
        assert!(concrete_panic_confirms_witness("panic path", "Panicked with 'THRESHOLD_EXCEEDED'."));
        assert!(concrete_panic_confirms_witness("u64_is_zero(x) branch 1", "Panicked with 'Division by 0'."));
    }

    #[test]
    #[cfg(feature = "native-test-runner")]
    fn upgrade_check_against_class_without_debug_names_detects_removed_slot_and_entrypoint() {
        let _env = crate::test_env_shared();
        let root = std::env::temp_dir().join(format!(
            "uc-upgrade-onchain-test-{}-{}",
            std::process::id(),
            epoch_ms_u64().unwrap_or(1)
        ));
        let _ = fs::remove_dir_all(&root);
        let write_project = |dir: &Path, name: &str, source: &str| {
            fs::create_dir_all(dir.join("src")).unwrap();
            fs::write(
                dir.join("Scarb.toml"),
                format!(
                    "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024_07\"\n\n[dependencies]\nstarknet = \"2.18.0\"\n\n[[target.starknet-contract]]\nsierra = true\ncasm = false\n"
                ),
            )
            .unwrap();
            fs::write(dir.join("src/lib.cairo"), source).unwrap();
        };
        let v1 = r#"
#[starknet::interface]
pub trait IVault<T> {
    fn set_fee(ref self: T, new_fee_bps: u16);
    fn set_limit(ref self: T, limit: u64, cap: u64);
    fn fee(self: @T) -> u16;
}
#[starknet::contract]
pub mod Vault {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use starknet::{ContractAddress, get_caller_address};
    #[storage]
    struct Storage { owner: ContractAddress, fee_bps: u16, limit: u64 }
    #[abi(embed_v0)]
    impl VaultImpl of super::IVault<ContractState> {
        fn set_fee(ref self: ContractState, new_fee_bps: u16) {
            assert!(get_caller_address() == self.owner.read(), "not_owner");
            self.fee_bps.write(new_fee_bps);
        }
        fn set_limit(ref self: ContractState, limit: u64, cap: u64) {
            assert!(limit <= cap, "limit_over_cap");
            self.limit.write(limit);
        }
        fn fee(self: @ContractState) -> u16 { self.fee_bps.read() }
    }
}
"#;
        // v2 drops the `limit` slot and the `set_limit` entry point, changes nothing else.
        let v2 = r#"
#[starknet::interface]
pub trait IVault<T> {
    fn set_fee(ref self: T, new_fee_bps: u16);
    fn fee(self: @T) -> u16;
}
#[starknet::contract]
pub mod Vault {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use starknet::{ContractAddress, get_caller_address};
    #[storage]
    struct Storage { owner: ContractAddress, fee_bps: u16 }
    #[abi(embed_v0)]
    impl VaultImpl of super::IVault<ContractState> {
        fn set_fee(ref self: ContractState, new_fee_bps: u16) {
            assert!(get_caller_address() == self.owner.read(), "not_owner");
            self.fee_bps.write(new_fee_bps);
        }
        fn fee(self: @ContractState) -> u16 { self.fee_bps.read() }
    }
}
"#;
        let base_dir = root.join("v1");
        let cand_dir = root.join("v2");
        write_project(&base_dir, "vault_v1", v1);
        write_project(&cand_dir, "vault_v2", v2);

        // Build v1 to a contract class file and reload it the way `uc audit` loads a deployed
        // class: no debug names, wrappers named from the ABI.
        run_build(BuildArgs {
            common: BuildCommonArgs {
                manifest_path: Some(base_dir.join("Scarb.toml")),
                package: None,
                workspace: false,
                features: Vec::new(),
                offline: true,
                release: false,
                profile: Some("dev".to_string()),
            },
            engine: EngineArg::Uc,
            daemon_mode: DaemonModeArg::Off,
            report_path: Some(base_dir.join("build.json")),
            record_failure: None,
            json: false,
            plan_only: false,
        })
        .expect("build v1");
        let class_text = fs::read_to_string(base_dir.join("target/dev/vault_v1_Vault.contract_class.json"))
            .expect("v1 contract class");
        let class_json: serde_json::Value = serde_json::from_str(&class_text).unwrap();
        let class: ContractClass = serde_json::from_str(&class_text).unwrap();
        let mut base_program = class.extract_sierra_program(true).unwrap().program;
        for f in &mut base_program.funcs {
            f.id.debug_name = None;
        }
        synthesize_contract_function_names(&mut base_program, &class.entry_points_by_type, class_json.get("abi"));

        let (cand, _, _) = compile_or_load_native_tests(
            &cand_dir.join("Scarb.toml"),
            &cand_dir,
            "vault_v2",
            "vault_v2",
            &[],
            true,
        )
        .expect("compile v2");
        let report = compare_upgrade_signatures(
            &base_program,
            "audit_target",
            &cand.sierra_program.program,
            "vault_v2",
            "0xbaseline".to_string(),
            Some("0xclass".to_string()),
        );

        assert_eq!(report.status, "UPGRADE_UNSAFE", "{report:?}");
        assert_eq!(report.baseline_class_hash.as_deref(), Some("0xclass"));
        assert!(
            report.removed_entrypoints.iter().any(|e| e.ends_with("::set_limit")),
            "set_limit must be reported removed: {:?}",
            report.removed_entrypoints
        );
        let limit_slot = format!(
            "0x{:x}",
            cairo_lang_starknet_classes::keccak::starknet_keccak(b"limit")
        );
        assert_eq!(
            report.removed_storage_slots,
            vec![limit_slot],
            "exactly the limit slot must be reported removed (by address: the deployed class has no names)"
        );
        assert!(report.arity_mismatches.is_empty(), "{:?}", report.arity_mismatches);
        assert!(report.auth_regressions.is_empty(), "{:?}", report.auth_regressions);

        // The same class against its own source is safe.
        let (same, _, _) = compile_or_load_native_tests(
            &base_dir.join("Scarb.toml"), &base_dir, "vault_v1", "vault_v1", &[], true,
        )
        .expect("compile v1");
        let safe = compare_upgrade_signatures(
            &base_program, "audit_target", &same.sierra_program.program, "vault_v1", "0xbaseline".to_string(), None,
        );
        let _ = fs::remove_dir_all(&root);
        assert_eq!(safe.status, "UPGRADE_SAFE", "{safe:?}");
    }

    #[test]
    #[cfg(feature = "native-test-runner")]
    fn storage_oracle_decides_storage_dependent_overflow_paths() {
        let _env = crate::test_env_shared();
        let root = std::env::temp_dir().join(format!(
            "uc-storage-oracle-test-{}-{}",
            std::process::id(),
            epoch_ms_u64().unwrap_or(1)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Scarb.toml"),
            "[package]\nname = \"counter_demo\"\nversion = \"0.1.0\"\nedition = \"2024_07\"\n\n[dependencies]\nstarknet = \"2.18.0\"\n\n[[target.starknet-contract]]\nsierra = true\ncasm = false\n",
        )
        .unwrap();
        fs::write(
            root.join("src/lib.cairo"),
            r#"
#[starknet::interface]
pub trait ICounter<T> {
    fn next_id(self: @T) -> u64;
}
#[starknet::contract]
pub mod Counter {
    use starknet::storage::StoragePointerReadAccess;
    #[storage]
    struct Storage { len: u64 }
    #[abi(embed_v0)]
    impl CounterImpl of super::ICounter<ContractState> {
        #[inline(never)]
        fn next_id(self: @ContractState) -> u64 { self.len.read() + 1 }
    }
}
"#,
        )
        .unwrap();
        let (comp, _, _) = compile_or_load_native_tests(
            &root.join("Scarb.toml"), &root, "counter_demo", "counter_demo", &[], true,
        )
        .expect("compile");
        let program = &comp.sierra_program.program;
        let len_key = format!("{:#x}", cairo_lang_starknet_classes::keccak::starknet_keccak(b"len"));
        let status_with = |oracle: Option<&StorageOracle>| {
            let (proofs, _) = analyze_sierra_program(program, "counter_demo", Some("next_id"), 1024, 1024, false, oracle);
            // Small methods are inlined into their entry point wrapper.
            let p = proofs
                .iter()
                .find(|p| p.function.ends_with("::next_id") && !p.function.contains("__wrapper__"))
                .or_else(|| proofs.iter().find(|p| p.function.contains("__wrapper__") && p.function.ends_with("next_id")))
                .unwrap_or_else(|| panic!("next_id proof missing in {:?}", proofs.iter().map(|p| &p.function).collect::<Vec<_>>()));
            (p.status.clone(), p.counterexample_reason.clone())
        };
        // Storage unknown: the overflow depends on an opaque read, nothing is decided.
        let (opaque, _) = status_with(None);
        assert_ne!(opaque, "PROVEN_SAFE", "opaque storage must not prove safety");
        // len = 5 on chain: 5 + 1 cannot overflow.
        let small = StorageOracle::from_map(BTreeMap::from([(len_key.clone(), "0x5".to_string())]));
        let small_status = status_with(Some(&small));
        assert_eq!(small.resolved(), vec![(len_key.clone(), "0x5".to_string())], "the len slot must be read");
        assert_eq!(small_status.0, "PROVEN_SAFE", "{small_status:?}");
        // len = u64::MAX on chain: the very next call overflows.
        let full = StorageOracle::from_map(BTreeMap::from([(len_key.clone(), "0xffffffffffffffff".to_string())]));
        let (status, reason) = status_with(Some(&full));
        assert_eq!(status, "COUNTEREXAMPLE_FOUND", "{reason:?}");
        assert!(reason.unwrap_or_default().contains("overflow"));
        // Values the solver cannot hold stay opaque instead of being misread.
        let huge = StorageOracle::from_map(BTreeMap::from([(len_key.clone(), "0x800000000000011000000000000000000000000000000000000000000000000".to_string())]));
        assert_eq!(huge.read_small(&len_key), None);
        assert_eq!(status_with(Some(&huge)).0, opaque);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(feature = "native-test-runner")]
    fn cei_detector_tells_a_held_lock_from_an_unrelated_pre_call_write() {
        let _env = crate::test_env_shared();
        let root = std::env::temp_dir().join(format!("uc-cei-lock-test-{}-{}", std::process::id(), epoch_ms_u64().unwrap_or(1)));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("Scarb.toml"), "[package]\nname = \"cei_demo\"\nversion = \"0.1.0\"\nedition = \"2024_07\"\n\n[dependencies]\nstarknet = \"2.18.0\"\n").unwrap();
        fs::write(root.join("src/lib.cairo"), r#"
use starknet::ContractAddress;
#[starknet::interface]
pub trait ICallee<T> { fn ping(ref self: T) -> felt252; }

#[starknet::interface]
pub trait IVault<T> { fn withdraw(ref self: T, target: ContractAddress); }

#[starknet::contract]
pub mod LockedVault {
    use starknet::ContractAddress;
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use super::{ICalleeDispatcher, ICalleeDispatcherTrait};
    #[storage]
    struct Storage { locked: bool, balance: u64 }
    #[abi(embed_v0)]
    impl VaultImpl of super::IVault<ContractState> {
        fn withdraw(ref self: ContractState, target: ContractAddress) {
            assert(!self.locked.read(), 'LOCKED');
            self.locked.write(true);
            ICalleeDispatcher { contract_address: target }.ping();
            self.balance.write(0);
            self.locked.write(false);
        }
    }
}

#[starknet::contract]
pub mod UnrelatedWriteVault {
    use starknet::ContractAddress;
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use super::{ICalleeDispatcher, ICalleeDispatcherTrait};
    #[storage]
    struct Storage { last_target: ContractAddress, balance: u64 }
    #[abi(embed_v0)]
    impl VaultImpl of super::IVault<ContractState> {
        fn withdraw(ref self: ContractState, target: ContractAddress) {
            self.last_target.write(target);
            ICalleeDispatcher { contract_address: target }.ping();
            self.balance.write(0);
        }
    }
}

#[starknet::contract]
pub mod NakedVault {
    use starknet::ContractAddress;
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use super::{ICalleeDispatcher, ICalleeDispatcherTrait};
    #[storage]
    struct Storage { balance: u64 }
    #[abi(embed_v0)]
    impl VaultImpl of super::IVault<ContractState> {
        fn withdraw(ref self: ContractState, target: ContractAddress) {
            ICalleeDispatcher { contract_address: target }.ping();
            self.balance.write(0);
        }
    }
}
"#).unwrap();
        let (comp, _, _) = compile_or_load_native_tests(&root.join("Scarb.toml"), &root, "cei_demo", "cei_demo", &[], true).expect("compile");
        let (_, findings) = analyze_sierra_program(&comp.sierra_program.program, "cei_demo", None, 1024, 1024, true, None);
        let _ = fs::remove_dir_all(&root);
        // Methods that stay out of line keep their impl name as module (`VaultImpl::withdraw`),
        // so the three verdicts are told apart by their text.
        let cei: Vec<(String, String)> = findings
            .iter()
            .filter(|f| f.detector == "caracal-cei-reentrancy-sierra" && f.function == "withdraw")
            .map(|f| (f.severity.clone(), f.description.clone()))
            .collect();
        assert_eq!(cei.len(), 3, "one verdict per vault: {cei:?}");
        let locked = cei.iter().find(|(_, d)| d.contains("holding a lock")).expect("lock verdict");
        assert_eq!(locked.0, "Low", "{locked:?}");
        let unrelated = cei.iter().find(|(_, d)| d.contains("not a lock")).expect("unrelated pre-write verdict");
        assert_eq!(unrelated.0, "Low", "{unrelated:?}");
        let naked = cei.iter().find(|(_, d)| d.contains("no prior state write")).expect("naked verdict");
        assert_eq!(naked.0, "High", "{naked:?}");
    }

    /// Functions in which the differential campaigns (`benchmarks/differential/run_arith.py`)
    /// found unsound verdicts or false counterexamples, with the verdict brute force gives.
    #[test]
    #[cfg(feature = "native-test-runner")]
    fn prover_regressions_found_by_the_differential_campaigns() {
        let _env = crate::test_env_shared();
        let root = std::env::temp_dir().join(format!("uc-prove-regressions-{}-{}", std::process::id(), epoch_ms_u64().unwrap_or(1)));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("Scarb.toml"), "[package]\nname = \"regress\"\nversion = \"0.1.0\"\nedition = \"2024_07\"\n").unwrap();
        fs::write(root.join("src/lib.cairo"), r#"
// The divisor is `Const<NonZero<u8>, Const<u8, 252>>`, declared before its inner constant.
pub fn p670(a: u8) -> u8 {
    let x0: u8 = 252_u8;
    let x1: u8 = (x0 & 147_u8);
    let x2: u8 = (x1 % x0);
    (1_u8 | ((if x2 > 255_u8 { x0 } else { a }) + x1))
}
pub fn felt_wrap(a: u8, b: u8) -> u8 {
    let fa: felt252 = a.into();
    let fb: felt252 = b.into();
    (fa - fb).try_into().unwrap()
}
pub fn felt_fits(a: u8) -> u16 {
    let fa: felt252 = a.into();
    (fa + 1).try_into().unwrap()
}
fn add_200(x: u8) -> u8 {
    x + 200
}
pub fn calls_panicking_helper(a: u8) -> u8 {
    add_200(a)
}
pub fn masked_never_overflows(a: u8) -> u8 {
    (a & 15) + 240
}
pub fn narrow(a: u16) -> u8 {
    a.try_into().unwrap()
}
pub fn checked_never_panics(a: u8, b: u8) -> u8 {
    match core::num::traits::CheckedAdd::checked_add(a, b) {
        Option::Some(v) => v,
        Option::None => 0,
    }
}
// The loop grows `acc`'s expression past the engine's size cap; the capped value is still a
// function of the parameters, so the never-failing `try_into` must not become a counterexample.
pub fn p1800(a: u8) -> u8 {
    let mut acc: u8 = (a ^ a);
    let mut i: u32 = 0;
    while i < 4 {
        acc = ({ let fa: felt252 = (acc).into(); let fb: felt252 = (acc).into(); let n: u8 = (fa - fb).try_into().unwrap(); n }) & (128_u8 | acc);
        i += 1;
    }
    0_u8
}
// `a != 0` is a test of the input, a precondition, not a bug.
pub fn rejects_zero(a: felt252) -> felt252 {
    assert(a != 0, 'zero');
    a
}
// Twenty iterations are deeper than the engine inlines: the caller takes the loop's own verdict.
fn sum_below(n: u32) -> u64 {
    let mut i: u32 = 0;
    let mut total: u64 = 0;
    while i < n {
        total += i.into();
        i += 1;
    }
    total
}
pub fn sum_below_twenty() -> u64 {
    sum_below(20)
}
"#).unwrap();
        let (comp, _, _) = compile_or_load_native_tests(&root.join("Scarb.toml"), &root, "regress", "regress", &[], true).expect("compile");
        let (proofs, _) = analyze_sierra_program(&comp.sierra_program.program, "regress", None, 1024, 1024, false, None);
        let _ = fs::remove_dir_all(&root);
        let proof = |name: &str| {
            proofs
                .iter()
                .find(|p| p.function.ends_with(&format!("::{name}")))
                .unwrap_or_else(|| panic!("{name} not analysed: {:?}", proofs.iter().map(|p| &p.function).collect::<Vec<_>>()))
        };
        let witness = |name: &str| -> Vec<i128> {
            let p = proof(name);
            assert_eq!(p.status, "COUNTEREXAMPLE_FOUND", "{name}: {p:?}");
            p.counterexample.as_ref().unwrap().values().map(|v| v.parse().unwrap()).collect()
        };
        // `x2 > 255` cannot hold, so only `a + 144` overflows.
        assert!(witness("p670")[0] >= 112);
        // `a - b` below zero wraps to a field element no `u8` holds.
        let w = witness("felt_wrap");
        assert!(w[0] < w[1], "felt_wrap witness {w:?} does not panic");
        assert!(matches!(proof("felt_fits").status.as_str(), "PROVEN_SAFE" | "GUARDED_SAFE"), "{:?}", proof("felt_fits"));
        // The callee's overflow is the caller's panic.
        assert!(witness("calls_panicking_helper")[0] >= 56);
        assert_ne!(proof("masked_never_overflows").status, "COUNTEREXAMPLE_FOUND");
        assert!(witness("narrow")[0] > 255);
        assert_eq!(proof("checked_never_panics").status, "PROVEN_SAFE");
        assert_ne!(proof("p1800").status, "COUNTEREXAMPLE_FOUND", "{:?}", proof("p1800"));
        assert_eq!(proof("rejects_zero").status, "GUARDED_SAFE", "{:?}", proof("rejects_zero"));
        assert!(matches!(proof("sum_below_twenty").status.as_str(), "PROVEN_SAFE" | "GUARDED_SAFE" | "BOUNDED_SAFE"), "{:?}", proof("sum_below_twenty"));
    }

    #[test]
    fn witness_calldata_encodes_by_parameter_type() {
        use super::witness_calldata;
        let types: Vec<String> = [
            "demo::Vault::ContractState",
            "core::integer::u256",
            "core::array::Span::<core::starknet::contract_address::ContractAddress>",
            "core::byte_array::ByteArray",
            "core::integer::u64",
            "core::felt252",
        ]
        .map(str::to_string)
        .to_vec();
        let calldata = witness_calldata(&[0, 5, 0, 0, 18446744073709551615, -1], &types);
        assert_eq!(
            calldata,
            vec![
                "0x5", "0x0",            // u256 low, high
                "0x0",                   // empty span
                "0x0", "0x0", "0x0",     // empty ByteArray
                "0xffffffffffffffff",    // u64
                "0x800000000000011000000000000000000000000000000000000000000000000", // -1 mod P
            ]
        );
    }

    use super::*;

    #[test]
    fn algebraic_solver_proves_contradiction_unsat_and_solves_linear_counterexample() {
        let bounds = vec![
            ("x".to_string(), 0, 10_000),
            ("y".to_string(), 0, 10_000),
        ];

        // 1. Contradictory path: x < 50 && x >= 100 -> UNSAT
        let unsat_constraints = vec![
            SymConstraint {
                lhs: SymExpr::Param(0, "x".to_string()),
                op: CmpOp::Lt,
                rhs: SymExpr::Const(50),
                reason: "x < 50".to_string(),
                is_overflow_branch: false,
            },
            SymConstraint {
                lhs: SymExpr::Param(0, "x".to_string()),
                op: CmpOp::Ge,
                rhs: SymExpr::Const(100),
                reason: "x >= 100".to_string(),
                is_overflow_branch: false,
            },
        ];
        assert_eq!(
            solve_path_constraints(&bounds, &unsat_constraints),
            SolverOutcome::Unsat
        );

        // 2. Relational bug path: x >= 100 && y >= 200 && (x + y) == 1337 -> SAT
        let sat_constraints = vec![
            SymConstraint {
                lhs: SymExpr::Param(0, "x".to_string()),
                op: CmpOp::Ge,
                rhs: SymExpr::Const(100),
                reason: "x >= 100".to_string(),
                is_overflow_branch: false,
            },
            SymConstraint {
                lhs: SymExpr::Param(1, "y".to_string()),
                op: CmpOp::Ge,
                rhs: SymExpr::Const(200),
                reason: "y >= 200".to_string(),
                is_overflow_branch: false,
            },
            SymConstraint {
                lhs: SymExpr::Add(
                    Box::new(SymExpr::Param(0, "x".to_string())),
                    Box::new(SymExpr::Param(1, "y".to_string())),
                ),
                op: CmpOp::Eq,
                rhs: SymExpr::Const(1337),
                reason: "x + y == 1337".to_string(),
                is_overflow_branch: false,
            },
        ];
        match solve_path_constraints(&bounds, &sat_constraints) {
            SolverOutcome::Sat(w) => {
                assert!(w[0] >= 100);
                assert!(w[1] >= 200);
                assert_eq!(w[0] + w[1], 1337);
            }
            other => panic!("expected Sat witness, got {other:?}"),
        }
    }

    #[test]
    #[cfg(feature = "native-test-runner")]
    fn caracal_loop_bound_detector_flags_only_uncapped_numeric_entrypoint_bounds() {
        let _env = crate::test_env_shared();
        let root = std::env::temp_dir().join(format!(
            "uc-prove-loopbound-test-{}-{}",
            std::process::id(),
            epoch_ms_u64().unwrap_or(1)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).expect("create src");
        fs::write(
            root.join("Scarb.toml"),
            r#"[package]
name = "loop_demo"
version = "0.1.0"
edition = "2024_07"

[dependencies]
starknet = "2.18.0"

[[target.starknet-contract]]
sierra = true
"#,
        )
        .expect("write Scarb.toml");
        fs::write(
            root.join("src/lib.cairo"),
            r#"#[starknet::interface]
pub trait ILoops<T> {
    fn sum_unbounded(self: @T, n: u64) -> u64;
    fn sum_bounded(self: @T, n: u64) -> u64;
    fn sum_unbounded_via_helper(self: @T, n: u64) -> u64;
    fn sum_bounded_via_helper(self: @T, n: u64) -> u64;
    fn sum_span(self: @T, values: Span<u64>) -> u64;
    fn clear_entries(ref self: T, key: felt252);
}

fn helper_sum(n: u64) -> u64 {
    let mut i: u64 = 0;
    let mut acc: u64 = 0;
    while i < n {
        acc += i;
        i += 1;
    }
    acc
}

#[starknet::contract]
pub mod Loops {
    use starknet::storage::{Map, StorageMapReadAccess, StorageMapWriteAccess};

    #[storage]
    struct Storage {
        entry_count: Map<felt252, u64>,
        entries: Map<(felt252, u64), felt252>,
    }

    #[abi(embed_v0)]
    impl LoopsImpl of super::ILoops<ContractState> {
        fn sum_unbounded(self: @ContractState, n: u64) -> u64 {
            let mut i: u64 = 0;
            let mut acc: u64 = 0;
            while i < n {
                acc += i;
                i += 1;
            }
            acc
        }
        fn sum_bounded(self: @ContractState, n: u64) -> u64 {
            assert(n <= 100, 'N_TOO_LARGE');
            let mut i: u64 = 0;
            let mut acc: u64 = 0;
            while i < n {
                acc += i;
                i += 1;
            }
            acc
        }
        fn sum_unbounded_via_helper(self: @ContractState, n: u64) -> u64 {
            super::helper_sum(n)
        }
        fn sum_bounded_via_helper(self: @ContractState, n: u64) -> u64 {
            assert(n <= 100, 'N_TOO_LARGE');
            super::helper_sum(n)
        }
        fn sum_span(self: @ContractState, values: Span<u64>) -> u64 {
            let mut i: u32 = 0;
            let mut acc: u64 = 0;
            while i < values.len() {
                acc += *values.at(i);
                i += 1;
            }
            acc
        }
        fn clear_entries(ref self: ContractState, key: felt252) {
            let count = self.entry_count.read(key);
            let mut i: u64 = 0;
            while i < count {
                self.entries.write((key, i), 0);
                i += 1;
            }
            self.entry_count.write(key, 0);
        }
    }
}
"#,
        )
        .expect("write lib.cairo");

        let report = prove_single_package(
            &root.join("Scarb.toml"),
            &root,
            "loop_demo",
            "loop_demo",
            &ProveArgs {
                max_depth: 64,
                max_paths: 1024,
                caracal: true,
                stateful_runs: 0,
                ..ProveArgs::default()
            },
        )
        .expect("prove_single_package");
        let _ = fs::remove_dir_all(&root);

        let flagged: Vec<&str> = report
            .caracal_findings
            .iter()
            .filter(|f| f.detector == "caracal-tainted-loop-bound-sierra")
            .map(|f| f.function.as_str())
            .collect();
        // Caller-chosen numeric bound with no cap, directly or through a helper: flagged.
        assert!(flagged.contains(&"sum_unbounded"), "missing sum_unbounded in {flagged:?}");
        assert!(
            flagged.contains(&"sum_unbounded_via_helper"),
            "missing sum_unbounded_via_helper in {flagged:?}"
        );
        // Capped bounds, calldata-array lengths and storage-derived counts: not flagged.
        for quiet in ["sum_bounded", "sum_bounded_via_helper", "sum_span", "clear_entries"] {
            assert!(!flagged.contains(&quiet), "{quiet} must not be flagged: {flagged:?}");
        }
    }

    #[test]
    #[cfg(feature = "native-test-runner")]
    fn prove_and_caracal_sierra_analyzer_distinguishes_insecure_from_secure_contracts() {
        let _env = crate::test_env_shared();
        let root = std::env::temp_dir().join(format!(
            "uc-prove-caracal-test-{}-{}",
            std::process::id(),
            epoch_ms_u64().unwrap_or(1)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).expect("create src");
        fs::write(
            root.join("Scarb.toml"),
            r#"[package]
name = "formal_demo"
version = "0.1.0"
edition = "2024_07"

[dependencies]
starknet = "2.16.0"
"#,
        )
        .expect("write Scarb.toml");

        fs::write(
            root.join("src/lib.cairo"),
            r#"
#[starknet::contract]
pub mod VulnerableVault {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        fee_bps: u16,
    }

    #[external(v0)]
    pub fn set_fee(ref self: ContractState, new_fee_bps: u16) {
        self.fee_bps.write(new_fee_bps);
    }
}

#[starknet::contract]
pub mod GuardedVault {
    use core::num::traits::Zero;
    use starknet::{ContractAddress, get_caller_address};
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        owner: ContractAddress,
        fee_bps: u16,
    }

    fn assert_only_owner(self: @ContractState) {
        let caller = get_caller_address();
        let owner = self.owner.read();
        assert!(caller == owner, "not_owner");
    }

    #[external(v0)]
    pub fn set_fee(ref self: ContractState, new_fee_bps: u16) {
        assert_only_owner(@self);
        assert!(new_fee_bps <= 10_000_u16, "fee_range");
        self.fee_bps.write(new_fee_bps);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn prove_safe_clamped_math(x: u16) {
        if x < 100 {
            let y = x + 50;
            assert!(y < 200, "unreachable_panic");
        }
    }

    #[test]
    fn prove_buggy_threshold(x: u16) {
        if x > 50 {
            assert!(x < 75, "fails_when_x_ge_75");
        }
    }
}
"#,
        )
        .expect("write lib.cairo");

        let report = prove_single_package(
            &root.join("Scarb.toml"),
            &root,
            "formal_demo",
            "formal_demo",
            &ProveArgs {
                max_depth: 64,
                max_paths: 1024,
                caracal: true,
                stateful_runs: 16,
                ..ProveArgs::default()
            },
        )
        .expect("prove_single_package");
        let _ = fs::remove_dir_all(&root);

        // 1. VulnerableVault::set_fee must trigger `caracal-unprotected-state-mutation`
        assert!(
            report.caracal_findings.iter().any(|f| {
                f.contract_or_module == "VulnerableVault"
                    && f.function == "set_fee"
                    && f.detector == "caracal-unprotected-state-mutation"
            }),
            "expected VulnerableVault::set_fee to trigger caracal-unprotected-state-mutation, got: {:?}",
            report.caracal_findings
        );

        // 2. GuardedVault::set_fee must have 0 Caracal findings!
        assert!(
            !report
                .caracal_findings
                .iter()
                .any(|f| f.contract_or_module == "GuardedVault"),
            "expected 0 Caracal findings on GuardedVault, got: {:?}",
            report.caracal_findings
        );

        // 3. Symbolic execution proves `prove_safe_clamped_math` is PROVEN_SAFE (panic path is UNSAT!)
        let safe_proof = report
            .symbolic_proofs
            .iter()
            .find(|p| p.function.ends_with("prove_safe_clamped_math"))
            .expect("prove_safe_clamped_math proof");
        assert_eq!(
            safe_proof.status, "PROVEN_SAFE",
            "expected PROVEN_SAFE for prove_safe_clamped_math, got: {safe_proof:?}"
        );
        assert!(safe_proof.unreachable_panic_paths >= 1);

        // 4. Symbolic execution finds concrete counterexample `x >= 75` for `prove_buggy_threshold`
        let buggy_proof = report
            .symbolic_proofs
            .iter()
            .find(|p| p.function.ends_with("prove_buggy_threshold"))
            .expect("prove_buggy_threshold proof");
        assert_eq!(
            buggy_proof.status, "COUNTEREXAMPLE_FOUND",
            "expected COUNTEREXAMPLE_FOUND for prove_buggy_threshold, got: {buggy_proof:?}"
        );
        let ce = buggy_proof
            .counterexample
            .as_ref()
            .expect("counterexample map");
        let x_val: i128 = ce.values().next().unwrap().parse().unwrap();
        assert!(x_val >= 75, "expected counterexample x >= 75, got {x_val}");
        assert_eq!(
            buggy_proof.concrete_verified,
            Some(true),
            "expected concrete_verified == Some(true) via SierraCasmRunner execution, got: {buggy_proof:?}"
        );
        assert!(
            buggy_proof
                .repro_test
                .as_deref()
                .is_some_and(|r| r.contains("#[should_panic]") && r.contains("prove_buggy_threshold")),
            "expected auto-generated repro_test for prove_buggy_threshold, got: {:?}",
            buggy_proof.repro_test
        );
    }

    #[test]
    #[cfg(feature = "native-test-runner")]
    fn medusa_stateful_campaign_upgrade_safety_and_sarif_export_work() {
        let _env = crate::test_env_shared();
        let root = std::env::temp_dir().join(format!(
            "uc-prove-medusa-upgrade-{}-{}",
            std::process::id(),
            epoch_ms_u64().unwrap_or(2)
        ));
        let _ = fs::remove_dir_all(&root);
        let baseline_dir = root.join("baseline");
        let candidate_dir = root.join("candidate");
        fs::create_dir_all(baseline_dir.join("src")).expect("create baseline src");
        fs::create_dir_all(candidate_dir.join("src")).expect("create candidate src");

        let scarb_toml = r#"[package]
name = "vault_upgrade_test"
version = "0.1.0"
edition = "2024_07"

[dependencies]
starknet = "2.16.0"
"#;
        fs::write(baseline_dir.join("Scarb.toml"), scarb_toml).expect("write baseline Scarb.toml");
        fs::write(candidate_dir.join("Scarb.toml"), scarb_toml).expect("write candidate Scarb.toml");

        // Baseline: secure vault with `owner` + `fee_bps` and caller check on `set_fee`
        fs::write(
            baseline_dir.join("src/lib.cairo"),
            r#"
#[starknet::contract]
pub mod Vault {
    use starknet::{ContractAddress, get_caller_address};
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        owner: ContractAddress,
        fee_bps: u16,
    }

    #[external(v0)]
    pub fn set_fee(ref self: ContractState, new_fee_bps: u16) {
        let caller = get_caller_address();
        assert!(caller == self.owner.read(), "not_owner");
        assert!(new_fee_bps <= 10_000_u16, "fee_too_high");
        self.fee_bps.write(new_fee_bps);
    }
}
"#,
        )
        .expect("write baseline lib.cairo");

        // Candidate: broken upgrade that drops `owner` from storage, removes caller guard & bound check,
        // and defines a Medusa/Echidna stateful invariant `invariant_fee_bounded`
        fs::write(
            candidate_dir.join("src/lib.cairo"),
            r#"
#[starknet::contract]
pub mod Vault {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        fee_bps: u16,
    }

    #[external(v0)]
    pub fn set_fee(ref self: ContractState, new_fee_bps: u16) {
        self.fee_bps.write(new_fee_bps);
    }

    #[external(v0)]
    pub fn invariant_fee_bounded(self: @ContractState) {
        assert!(self.fee_bps.read() <= 10_000_u16, "fee_exceeded_10000_bps");
    }
}
"#,
        )
        .expect("write candidate lib.cairo");

        let args = ProveArgs {
            max_depth: 64,
            max_paths: 1024,
            caracal: true,
            stateful_runs: 16,
            check_upgrade: Some(baseline_dir.display().to_string()),
            ..ProveArgs::default()
        };

        let pkg_report = prove_single_package(
            &candidate_dir.join("Scarb.toml"),
            &candidate_dir,
            "vault_upgrade_test",
            "vault_upgrade_test",
            &args,
        )
        .expect("prove_single_package candidate");

        // 1. Medusa stateful campaign must violate `invariant_fee_bounded` after calling `Vault::set_fee`
        //    and delta-debug shrink the sequence to a single call!
        let inv = pkg_report
            .stateful_invariants
            .iter()
            .find(|i| i.invariant_name.contains("invariant_fee_bounded"))
            .expect("invariant_fee_bounded report");
        assert_eq!(
            inv.status, "INVARIANT_VIOLATED",
            "expected INVARIANT_VIOLATED on Vault::invariant_fee_bounded, got: {inv:?}"
        );
        let shrunk = inv.shrunk_call_sequence.as_ref().expect("shrunk sequence");
        assert_eq!(
            shrunk.len(),
            1,
            "expected delta-debugging to shrink violating sequence to 1 tx, got: {shrunk:?}"
        );
        assert!(shrunk[0].starts_with("Vault::set_fee("));

        // 2. Formal Upgrade Prover must report UPGRADE_UNSAFE (removed `owner` slot + auth regression on `set_fee`)
        let upg = pkg_report.upgrade_safety.as_ref().expect("upgrade_safety");
        assert_eq!(
            upg.status, "UPGRADE_UNSAFE",
            "expected UPGRADE_UNSAFE when upgrading baseline -> candidate, got: {upg:?}"
        );
        assert!(
            upg.removed_storage_slots
                .iter()
                .any(|c| c.contains("owner")),
            "expected removed_storage_slots for owner slot, got: {:?}",
            upg.removed_storage_slots
        );
        assert!(
            upg.auth_regressions
                .iter()
                .any(|r| r.contains("Vault::set_fee")),
            "expected auth regression for Vault::set_fee, got: {:?}",
            upg.auth_regressions
        );

        // 3. SARIF v2.1.0 export must produce valid JSON with rules and results
        let sarif_path = root.join("report.sarif.json");
        let full_report = ProveWorkspaceReport {
            schema_version: 1,
            engine: "uc-caracal-symbolic-sierra-v1".to_string(),
            sierra_ir_version: "1.7.0".to_string(),
            elapsed_ms: 12.0,
            total_functions_analyzed: 4,
            total_proven_safe: 0,
            total_guarded_safe: 0,
            total_counterexamples: 1,
            total_caracal_findings: 1,
            packages: vec![pkg_report],
        };
        write_sarif_report(&sarif_path, &full_report).expect("write_sarif_report");
        let sarif_str = fs::read_to_string(&sarif_path).expect("read sarif");
        let sarif_json: serde_json::Value = serde_json::from_str(&sarif_str).expect("valid sarif json");
        assert_eq!(sarif_json["version"], "2.1.0");
        assert!(
            sarif_json["runs"][0]["results"]
                .as_array()
                .is_some_and(|arr| arr.len() >= 3),
            "expected SARIF results to include caracal + stateful invariant + upgrade findings, got: {sarif_str}"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
