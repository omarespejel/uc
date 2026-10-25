use super::*;
use cairo_lang_sierra::ids::ConcreteTypeId;
use cairo_lang_sierra::program::{BranchTarget, GenericArg, Program, Statement};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

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

    /// Maximum symbolic execution branch depth (bounded model checking horizon).
    #[arg(long, default_value_t = 64)]
    pub max_depth: usize,

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

    /// Verify formal upgrade compatibility (storage slots, ABI arity, and auth guards) against a baseline manifest.
    #[arg(long)]
    pub check_upgrade: Option<PathBuf>,

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
    pub proven_overflow_free: bool,
    pub preconditions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterexample: Option<BTreeMap<String, String>>,
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
    pub baseline_manifest: String,
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
    generic_id: String,
    debug_name: String,
    user_func_callee: Option<u64>,
    const_value: Option<i128>,
    enum_variant_idx: Option<usize>,
    is_panic_result_enum: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct TypeMeta {
    generic_id: String,
    debug_name: String,
    const_value: Option<i128>,
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
            let mut is_panic_result = debug_name.contains("PanicResult");
            for arg in &td.long_id.generic_args {
                match arg {
                    GenericArg::Value(v) => {
                        if let Ok(val) = i128::try_from(v.clone()) {
                            const_value = Some(val);
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
                    is_panic_result,
                },
            );
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
                        }
                    }
                    GenericArg::Type(tid) => {
                        if let Some(tm) = types.get(&tid.id) {
                            if tm.const_value.is_some() && const_value.is_none() {
                                const_value = tm.const_value;
                            }
                            if tm.is_panic_result {
                                is_panic_result_enum = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
            libfuncs.insert(
                ld.id.id,
                LibfuncMeta {
                    generic_id,
                    debug_name,
                    user_func_callee,
                    const_value,
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
            && (0..index.program.funcs.len()).any(|other_idx| {
                let other_name = index.func_name(other_idx);
                other_name.ends_with(&format!("::__external::{short_name}"))
                    || other_name.ends_with(&format!("::__constructor::{short_name}"))
                    || other_name.ends_with(&format!("::__l1_handler::{short_name}"))
                    || other_name.ends_with(&format!("__wrapper__{short_name}"))
                    || other_name.ends_with(&format!("__{short_name}"))
            });

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

                // Detector 4: Check-Effects-Interactions (CEI) Reentrancy on Sierra IR
                if seen_external_call_stmt.is_none() {
                    storage_write_before_external_call = true;
                }
                if let Some(call_stmt) = seen_external_call_stmt {
                    if call_stmt < stmt_idx {
                        let key = (
                            "caracal-cei-reentrancy-sierra".to_string(),
                            contract_or_mod.clone(),
                            short_name.clone(),
                        );
                        if seen_keys.insert(key) {
                            let guarded = storage_write_before_external_call;
                            findings.push(CaracalSierraFinding {
                                detector: "caracal-cei-reentrancy-sierra".to_string(),
                                severity: if guarded { "Low".to_string() } else { "High".to_string() },
                                confidence: if guarded { "Low".to_string() } else { "Medium".to_string() },
                                contract_or_module: contract_or_mod.clone(),
                                function: short_name.clone(),
                                sierra_statement_idx: stmt_idx,
                                description: if guarded {
                                    format!(
                                        "`{contract_or_mod}::{short_name}` writes storage before and after an external call (statement #{call_stmt}); this is the shape of a reentrancy guard or lock. Verify the pre-call write actually blocks re-entry into every state-changing entrypoint."
                                    )
                                } else {
                                    format!(
                                        "Check-Effects-Interactions violation in `{contract_or_mod}::{short_name}`: external contract call at Sierra statement #{call_stmt} precedes storage write at statement #{stmt_idx} with no prior state write that could act as a guard."
                                    )
                                },
                                recommendation: "Perform all `storage_write_syscall` state updates before invoking `call_contract_syscall`.".to_string(),
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
            | Self::Rem(a, b) => a.mentions_param() || b.mentions_param(),
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
    for i in 0..constraints.len() {
        for j in (i + 1)..constraints.len() {
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

    // Build per-parameter candidate witness sets from solved interval boundaries & algebraic relations
    let mut per_param_candidates: Vec<Vec<i128>> = Vec::with_capacity(n);
    for idx in 0..n {
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

    // Search for a concrete witness satisfying all constraints
    let mut env = vec![0i128; n];
    if search_witness_recursive(
        0,
        &mut env,
        &per_param_candidates,
        &lo,
        &hi,
        constraints,
    ) {
        return SolverOutcome::Sat(env);
    }

    SolverOutcome::Unknown
}

fn search_witness_recursive(
    param_idx: usize,
    env: &mut [i128],
    candidates: &[Vec<i128>],
    lo: &[i128],
    hi: &[i128],
    constraints: &[SymConstraint],
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
        env[param_idx] = val;
        if search_witness_recursive(param_idx + 1, env, candidates, lo, hi, constraints) {
            return true;
        }
    }
    false
}

#[derive(Debug, Clone)]
struct SymPathState {
    stmt_idx: usize,
    depth: usize,
    vars: HashMap<u64, SymExpr>,
    constraints: Vec<SymConstraint>,
    performed_computation: bool,
}

fn run_symbolic_verification_for_function(
    index: &SierraProgramIndex<'_>,
    summaries: &[FunctionInterprocSummary],
    func_idx: usize,
    max_depth: usize,
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

    let (_, start, end) = index.func_ranges[func_idx];
    let (_can_reach_ok, panic_vars) =
        compute_ok_reachable_statements(index, summaries, start, end);

    // Extract symbolic value parameters (excluding builtins like GasBuiltin, RangeCheck, System)
    let mut param_bounds: Vec<(String, i128, i128)> = Vec::new();
    let mut initial_vars: HashMap<u64, SymExpr> = HashMap::new();
    let mut param_descriptions: Vec<String> = Vec::new();

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
        param_descriptions.push(format!("{pname}: {ty_name}"));
        initial_vars.insert(p.id.id, SymExpr::Param(p_idx, pname));
    }

    let mut queue: VecDeque<SymPathState> = VecDeque::new();
    queue.push_back(SymPathState {
        stmt_idx: start,
        depth: 0,
        vars: initial_vars,
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

    let max_paths = 256usize;

    while let Some(mut state) = queue.pop_front() {
        if explored_paths >= max_paths {
            break;
        }

        loop {
            if state.stmt_idx < start || state.stmt_idx >= end || state.depth > max_depth {
                explored_paths += 1;
                break;
            }

            match &index.program.statements[state.stmt_idx] {
                Statement::Return(ret_vars) => {
                    explored_paths += 1;
                    let is_panic = ret_vars.iter().any(|v| panic_vars.contains(&v.id));
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

                        match solve_path_constraints(&param_bounds, &meaningful_constraints) {
                            SolverOutcome::Unsat => {
                                unreachable_panic_paths += 1;
                            }
                            SolverOutcome::Sat(witness) => {
                                let has_overflow =
                                    meaningful_constraints.iter().any(|c| c.is_overflow_branch);
                                if has_overflow {
                                    proven_overflow_free = false;
                                }
                                let is_precondition_guard = meaningful_constraints.is_empty()
                                    || (!state.performed_computation
                                        && !has_overflow
                                        && !is_test_or_property
                                        && !summaries[func_idx].is_recursive_loop);

                                if is_precondition_guard {
                                    let pre = if meaningful_constraints.is_empty() {
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
                                }
                            }
                            SolverOutcome::Unknown => {}
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
                        if gid == "const_as_immediate" || gid.ends_with("_const") {
                            if let (Some(val), Some(res)) = (lf.const_value, branch.results.first())
                            {
                                state.vars.insert(res.id, SymExpr::Const(val));
                            }
                        } else if gid == "rename"
                            || gid == "dup"
                            || gid == "store_temp"
                            || gid == "snapshot_take"
                            || gid == "unbox"
                            || gid == "into_box"
                            || gid == "upcast"
                        {
                            if let Some(arg0) = inv.args.last() {
                                if let Some(expr) = state.vars.get(&arg0.id).cloned() {
                                    for r in &branch.results {
                                        state.vars.insert(r.id, expr.clone());
                                    }
                                }
                            }
                        } else if (gid.starts_with("felt252_add") || gid.ends_with("_wide_mul"))
                            && inv.args.len() >= 2
                        {
                            let lhs_id = inv.args[inv.args.len() - 2].id;
                            let rhs_id = inv.args[inv.args.len() - 1].id;
                            let a = state
                                .vars
                                .get(&lhs_id)
                                .cloned()
                                .unwrap_or_else(|| SymExpr::SymbolicOpaque(format!("v{lhs_id}")));
                            let b = state
                                .vars
                                .get(&rhs_id)
                                .cloned()
                                .unwrap_or_else(|| SymExpr::SymbolicOpaque(format!("v{rhs_id}")));
                            if let Some(r) = branch.results.last() {
                                let expr = if gid.ends_with("_wide_mul") {
                                    SymExpr::Mul(Box::new(a), Box::new(b))
                                } else {
                                    SymExpr::Add(Box::new(a), Box::new(b))
                                };
                                state.vars.insert(r.id, expr);
                                state.performed_computation = true;
                            }
                        } else if gid.starts_with("felt252_sub") && inv.args.len() >= 2 {
                            let lhs_id = inv.args[inv.args.len() - 2].id;
                            let rhs_id = inv.args[inv.args.len() - 1].id;
                            let a = state
                                .vars
                                .get(&lhs_id)
                                .cloned()
                                .unwrap_or_else(|| SymExpr::SymbolicOpaque(format!("v{lhs_id}")));
                            let b = state
                                .vars
                                .get(&rhs_id)
                                .cloned()
                                .unwrap_or_else(|| SymExpr::SymbolicOpaque(format!("v{rhs_id}")));
                            if let Some(r) = branch.results.last() {
                                state
                                    .vars
                                    .insert(r.id, SymExpr::Sub(Box::new(a), Box::new(b)));
                            }
                        } else if gid.ends_with("_safe_divmod") && inv.args.len() >= 2 {
                            let lhs_id = inv.args[inv.args.len() - 2].id;
                            let rhs_id = inv.args[inv.args.len() - 1].id;
                            let a = state
                                .vars
                                .get(&lhs_id)
                                .cloned()
                                .unwrap_or_else(|| SymExpr::SymbolicOpaque(format!("v{lhs_id}")));
                            let b = state
                                .vars
                                .get(&rhs_id)
                                .cloned()
                                .unwrap_or_else(|| SymExpr::SymbolicOpaque(format!("v{rhs_id}")));
                            if branch.results.len() >= 3 {
                                state.vars.insert(
                                    branch.results[1].id,
                                    SymExpr::Div(Box::new(a.clone()), Box::new(b.clone())),
                                );
                                state
                                    .vars
                                    .insert(branch.results[2].id, SymExpr::Rem(Box::new(a), Box::new(b)));
                                state.performed_computation = true;
                            }
                        }

                        state.stmt_idx = match branch.target {
                            BranchTarget::Fallthrough => state.stmt_idx + 1,
                            BranchTarget::Statement(s) => s.0,
                        };
                        state.depth += 1;
                        continue;
                    }

                    // Multi-branch invocation: fork symbolic execution states
                    if inv.branches.len() >= 2 {
                        // Ignore gas/array/PanicResult-propagation infrastructure branches by taking branch 0 (Ok)
                        let is_callee_panic_match = gid == "enum_match"
                            && lf.is_panic_result_enum
                            && !inv.args.iter().any(|a| panic_vars.contains(&a.id));
                        if gid.starts_with("withdraw_gas")
                            || gid.starts_with("redeposit_gas")
                            || gid.ends_with("_syscall")
                            || is_callee_panic_match
                        {
                            let b0 = &inv.branches[0];
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

                            if gid.ends_with("_is_zero") && !inv.args.is_empty() {
                                let arg_id = inv.args[inv.args.len() - 1].id;
                                if let Some(arg_expr) = state.vars.get(&arg_id).cloned() {
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
                                if let (Some(lhs), Some(rhs)) = (a, b) {
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
                                if let (Some(lhs), Some(rhs)) = (a, b) {
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
                                if let (Some(lhs), Some(rhs)) = (a, b) {
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
                            } else if gid.ends_with("_overflowing_add") && inv.args.len() >= 2 {
                                let lhs_id = inv.args[inv.args.len() - 2].id;
                                let rhs_id = inv.args[inv.args.len() - 1].id;
                                let a = state.vars.get(&lhs_id).cloned();
                                let b = state.vars.get(&rhs_id).cloned();
                                let ty_prefix = gid.split('_').next().unwrap_or("u128");
                                let max_val = max_bound_for_sierra_type(ty_prefix);
                                if let (Some(lhs), Some(rhs)) = (a, b) {
                                    let sum_expr =
                                        SymExpr::Add(Box::new(lhs.clone()), Box::new(rhs.clone()));
                                    if let Some(res_var) = branch.results.last() {
                                        child.vars.insert(res_var.id, sum_expr.clone());
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
                                if let (Some(lhs), Some(rhs)) = (a, b) {
                                    let diff_expr =
                                        SymExpr::Sub(Box::new(lhs.clone()), Box::new(rhs.clone()));
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
                                        is_overflow_branch: b_idx == 1 && state.performed_computation,
                                    });
                                }
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
    } else if !preconditions.is_empty() {
        "GUARDED_SAFE".to_string()
    } else {
        "PROVEN_SAFE".to_string()
    };

    let (counterexample, counterexample_reason) = match first_counterexample {
        Some((ce, reason)) => (Some(ce), Some(reason)),
        None => (None, None),
    };

    SymbolicFunctionProofReport {
        function: fname,
        kind: kind.to_string(),
        params: param_descriptions,
        status,
        explored_paths,
        ok_paths,
        panic_paths,
        unreachable_panic_paths,
        proven_overflow_free,
        preconditions,
        counterexample,
        counterexample_reason,
        concrete_verified: None,
        concrete_panic_data: None,
        repro_test: None,
    }
}

/// Runs formal verification + Caracal Sierra IR analysis directly on an in-memory `Program`.
pub(crate) fn analyze_sierra_program(
    program: &Program,
    crate_name: &str,
    filter: Option<&str>,
    max_depth: usize,
    enable_caracal: bool,
) -> (Vec<SymbolicFunctionProofReport>, Vec<CaracalSierraFinding>) {
    let index = SierraProgramIndex::build(program);
    let summaries = compute_function_summaries(&index);

    let caracal_findings = if enable_caracal {
        run_caracal_sierra_analysis(&index, &summaries, crate_name, filter)
    } else {
        Vec::new()
    };

    let mut symbolic_proofs = Vec::new();
    for func_idx in 0..program.funcs.len() {
        let fname = index.func_name(func_idx);
        if !fname.starts_with(&format!("{crate_name}::")) {
            continue;
        }
        if let Some(f) = filter {
            if !fname.contains(f) {
                continue;
            }
        }
        // `#[test]`/`#[fuzzer]` bodies deploy contracts and use cheatcodes the symbolic engine
        // does not model; `uc test` runs them concretely. Only property-style functions
        // (`prove_*`, `invariant_*`, `check_*`) are symbolic targets.
        let (_, short) = extract_contract_and_short_name(&fname);
        let is_plain_test = (fname.contains("::tests::") || short.starts_with("test_"))
            && !(short.starts_with("prove_") || short.starts_with("invariant_") || short.starts_with("check_"));
        if is_plain_test {
            continue;
        }
        let proof = run_symbolic_verification_for_function(&index, &summaries, func_idx, max_depth);
        symbolic_proofs.push(proof);
    }

    (symbolic_proofs, caracal_findings)
}

#[cfg(feature = "native-test-runner")]
fn verify_counterexamples_concretely(
    runner: &cairo_lang_runner::SierraCasmRunner,
    program: &Program,
    proofs: &mut [SymbolicFunctionProofReport],
) {
    use cairo_lang_runner::casm_run::format_for_panic;
    use cairo_lang_runner::{Arg, RunResultValue, StarknetState};
    use starknet_types_core::felt::Felt as Felt252;

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
            StarknetState::default(),
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

fn extract_sierra_upgrade_signature(
    program: &Program,
    crate_name: &str,
) -> (
    BTreeSet<String>,
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

    let mut storage_slots = BTreeSet::new();
    for ld in &program.libfunc_declarations {
        if ld.long_id.generic_id.0.as_str() == "storage_base_address_const" {
            for arg in &ld.long_id.generic_args {
                if let GenericArg::Value(v) = arg {
                    let hex_slot = format!("0x{:x}", v);
                    if let Some(name) = keccak_reverse.get(&hex_slot) {
                        storage_slots.insert(format!("{name} ({hex_slot})"));
                    } else {
                        storage_slots.insert(hex_slot);
                    }
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

    let (base_slots, base_eps, base_protected) =
        extract_sierra_upgrade_signature(&base_comp.sierra_program.program, &base_crate);
    let (cand_slots, cand_eps, cand_protected) =
        extract_sierra_upgrade_signature(candidate_program, candidate_crate_name);

    let removed_storage_slots: Vec<String> = base_slots.difference(&cand_slots).cloned().collect();
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

    Ok(UpgradeSafetyReport {
        baseline_manifest: base_manifest.display().to_string(),
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
    })
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
    let (base_proofs, _) = analyze_sierra_program(base_prog, &base_crate, None, 64, false);
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
        args.caracal,
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

    let mut stateful_invariants = Vec::new();
    if (has_counterexamples || has_invariant_candidates) && program.statements.len() <= 8_000 {
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
            if has_counterexamples {
                verify_counterexamples_concretely(&runner, program, &mut symbolic_proofs);
            }
            if has_invariant_candidates {
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

    let upgrade_safety = if let Some(baseline_path) = &args.check_upgrade {
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

    Ok(PackageProveReport {
        package_name: package_name.to_string(),
        manifest_path: manifest_path.display().to_string(),
        sierra_functions_analyzed: symbolic_proofs.len(),
        sierra_statements_analyzed: program.statements.len(),
        proven_safe_count,
        guarded_safe_count,
        counterexample_count,
        caracal_findings_count: caracal_findings.len(),
        symbolic_proofs,
        caracal_findings,
        stateful_invariants,
        upgrade_safety,
        equivalence,
    })
}

pub(crate) fn run_prove(args: ProveArgs) -> Result<()> {
    let start = Instant::now();
    let mut pkg_reports = Vec::new();

    if let Some(sierra_file) = &args.sierra_file {
        let text = fs::read_to_string(sierra_file)
            .with_context(|| format!("failed to read {}", sierra_file.display()))?;
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
            let (symbolic_proofs, caracal_findings) = analyze_sierra_program(
                &program,
                &inferred_crate,
                args.filter.as_deref(),
                args.max_depth,
                args.caracal,
            );
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
            caracal: true,
            stateful_runs: 16,
            check_upgrade: Some(baseline_dir.clone()),
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
