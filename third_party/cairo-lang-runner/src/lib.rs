//! Basic runner for running a Sierra program on the VM.
use std::collections::HashMap;
use std::sync::Arc;

use cairo_lang_casm::hints::Hint;
use cairo_lang_runnable_utils::builder::{BuildError, EntryCodeConfig, RunnableBuilder};
use cairo_lang_sierra::extensions::NamedType;
use cairo_lang_sierra::extensions::enm::EnumType;
use cairo_lang_sierra::extensions::gas::{CostTokenType, GasBuiltinType};
use cairo_lang_sierra::ids::{ConcreteTypeId, GenericTypeId};
use cairo_lang_sierra::program::{Function, GenericArg};
use cairo_lang_sierra_to_casm::metadata::MetadataComputationConfig;
use cairo_lang_starknet::contract::ContractInfo;
use cairo_lang_utils::casts::IntoOrPanic;
use cairo_lang_utils::ordered_hash_map::OrderedHashMap;
use cairo_lang_utils::{extract_matches, require};
use cairo_vm::hint_processor::hint_processor_definition::HintProcessor;
use cairo_vm::serde::deserialize_program::HintParams;
use cairo_vm::types::builtin_name::BuiltinName;
use cairo_vm::vm::errors::cairo_run_errors::CairoRunError;
use cairo_vm::vm::runners::cairo_runner::{ExecutionResources, RunResources};
use cairo_vm::types::relocatable::{MaybeRelocatable, Relocatable};
use cairo_vm::vm::vm_core::VirtualMachine;
use casm_run::hint_to_hint_params;
pub use casm_run::{CairoHintProcessor, StarknetState};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use profiling::ProfilingInfo;
use starknet_types_core::felt::Felt as Felt252;
use thiserror::Error;

use crate::casm_run::{HintMap, RunFunctionResult, RunProgramLazyResult, StarknetHintProcessor};
use crate::profiling::ProfilerConfig;

pub mod casm_run;
pub mod clap;
pub mod profiling;
pub mod short_string;

const MAX_STACK_TRACE_DEPTH_DEFAULT: usize = 100;

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error(transparent)]
    BuildError(#[from] BuildError),
    #[error("Not enough gas to call function.")]
    NotEnoughGasToCall,
    #[error("Function param {param_index} only partially contains argument {arg_index}.")]
    ArgumentUnaligned { param_index: usize, arg_index: usize },
    #[error("Function expects arguments of size {expected} and received {actual} instead.")]
    ArgumentsSizeMismatch { expected: usize, actual: usize },
    #[error(transparent)]
    CairoRunError(#[from] Box<CairoRunError>),
}

/// The full result of a run with Starknet state.
pub struct RunResultStarknet {
    pub gas_counter: Option<Felt252>,
    pub memory: Vec<Option<Felt252>>,
    /// Contents of a returned `Array<felt252>`/`Span<felt252>` when the run skipped memory
    /// relocation (`memory` is then empty). Entry point wrappers return exactly this shape.
    pub return_array: Option<Vec<Felt252>>,
    pub value: RunResultValue,
    pub starknet_state: StarknetState,
    pub used_resources: StarknetExecutionResources,
    /// The profiling info of the run, if requested.
    pub profiling_info: Option<ProfilingInfo>,
}

/// The full result of a run.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct RunResult {
    pub gas_counter: Option<Felt252>,
    pub memory: Vec<Option<Felt252>>,
    /// See [RunResultStarknet::return_array].
    pub return_array: Option<Vec<Felt252>>,
    pub value: RunResultValue,
    pub used_resources: ExecutionResources,
    /// The profiling info of the run, if requested.
    pub profiling_info: Option<ProfilingInfo>,
}

/// The execution resources in a run.
/// Extends [ExecutionResources] by including the used syscalls for Starknet.
#[derive(Debug, Eq, PartialEq, Clone, Default)]
pub struct StarknetExecutionResources {
    /// The basic execution resources.
    pub basic_resources: ExecutionResources,
    /// The used syscalls.
    pub syscalls: HashMap<String, usize>,
}

impl std::ops::AddAssign<StarknetExecutionResources> for StarknetExecutionResources {
    /// Adds the resources of `other` to `self`.
    fn add_assign(&mut self, other: Self) {
        self.basic_resources += &other.basic_resources;
        for (k, v) in other.syscalls {
            *self.syscalls.entry(k).or_default() += v;
        }
    }
}

/// The ran function return value.
#[derive(Debug, Eq, PartialEq, Clone)]
pub enum RunResultValue {
    /// Run ended successfully, returning the memory of the non-implicit returns.
    Success(Vec<Felt252>),
    /// Run panicked, returning the carried error data.
    Panic(Vec<Felt252>),
}

/// Returns the approximated gas cost for each token type.
pub fn token_gas_cost(token_type: CostTokenType) -> usize {
    match token_type {
        CostTokenType::Const => 1,
        CostTokenType::Step
        | CostTokenType::Hole
        | CostTokenType::RangeCheck
        | CostTokenType::RangeCheck96 => {
            panic!("Token type {token_type:?} has no gas cost.")
        }
        CostTokenType::Pedersen => 4050,
        CostTokenType::Poseidon => 491,
        CostTokenType::Bitwise => 583,
        CostTokenType::EcOp => 4085,
        CostTokenType::AddMod => 230,
        CostTokenType::MulMod => 604,
        CostTokenType::Blake => 3334,
    }
}

/// An argument to a Sierra function run.
#[derive(Debug, Clone)]
pub enum Arg {
    Value(Felt252),
    Array(Vec<Arg>),
}
impl Arg {
    /// Returns the size of the argument in the VM.
    pub fn size(&self) -> usize {
        match self {
            Self::Value(_) => 1,
            Self::Array(_) => 2,
        }
    }
}
impl From<Felt252> for Arg {
    fn from(value: Felt252) -> Self {
        Self::Value(value)
    }
}

/// Builds `hints_dict` required in `cairo_vm::types::program::Program` from instructions.
pub fn build_hints_dict(
    hints: &[(usize, Vec<Hint>)],
) -> (HashMap<usize, Vec<HintParams>>, HintMap) {
    let mut hints_dict: HashMap<usize, Vec<HintParams>> = HashMap::new();
    let mut string_to_hint: HintMap = HintMap::default();

    for (offset, offset_hints) in hints {
        // Register hint with string for the hint processor.
        for hint in offset_hints {
            string_to_hint.insert(hint.representing_string(), Arc::new(hint.clone()));
        }
        // Add hint, associated with the instruction offset.
        hints_dict.insert(*offset, offset_hints.iter().map(hint_to_hint_params).collect());
    }
    (hints_dict, string_to_hint)
}

/// A struct representing a prepared execution context for starting a function within a given
/// Starknet state.
///
/// Pass fields from this object to
/// [`SierraCasmRunner::run_function_with_prepared_starknet_context`] to run the function.
/// For typical use-cases you should use [`SierraCasmRunner::run_function_with_starknet_context`],
/// which does all the preparation, running, and result composition for you.
pub struct PreparedStarknetContext {
    pub hints_dict: HashMap<usize, Vec<HintParams>>,
    pub bytecode: Vec<BigInt>,
    pub builtins: Vec<BuiltinName>,
}

thread_local! {
    static COVERAGE_OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Returns the current thread's coverage override, if any (for tests and diagnostics).
pub fn thread_coverage_override() -> Option<bool> {
    COVERAGE_OVERRIDE.with(|c| c.get())
}

/// Restores the previous thread-scoped coverage setting on drop.
pub struct ScopedCoverage {
    previous: Option<bool>,
}

impl ScopedCoverage {
    pub fn new(enabled: bool) -> Self {
        let previous = COVERAGE_OVERRIDE.with(|c| c.replace(Some(enabled)));
        Self { previous }
    }
}

impl Drop for ScopedCoverage {
    fn drop(&mut self) {
        COVERAGE_OVERRIDE.with(|c| c.set(self.previous));
    }
}


/// Cached assembled VM program and header hints for a single Sierra function.
#[derive(Clone)]
struct CachedFunctionProgram {
    program: cairo_vm::types::program::Program,
    data_len: usize,
    header_string_to_hint: HintMap,
}

/// Runner enabling running a Sierra program on the VM.
pub struct SierraCasmRunner {
    /// Builder for runnable functions.
    builder: RunnableBuilder,
    /// Mapping from class_hash to contract info.
    starknet_contracts_info: OrderedHashMap<Felt252, ContractInfo>,
    /// Whether to run the profiler when running using this runner.
    run_profiler: Option<ProfilingInfoCollectionConfig>,
    /// Pre-assembled base CASM bytecode (without entry header/footer).
    base_bytecode: Vec<cairo_vm::types::relocatable::MaybeRelocatable>,
    /// Pre-computed base CASM hint params (offset relative to base_bytecode start).
    base_hints: Vec<(usize, Vec<HintParams>)>,
    /// Pre-computed mapping from hint representation string to `Hint` for the base program.
    pub(crate) base_string_to_hint: HintMap,
    /// Cache of assembled `cairo_vm::types::program::Program` per function ID.
    program_cache:
        std::sync::RwLock<HashMap<cairo_lang_sierra::ids::FunctionId, CachedFunctionProgram>>,
    /// Lock-free bitmap of executed Sierra statement indices (`StatementIdx`).
    coverage_bitmap: Vec<std::sync::atomic::AtomicU64>,
    /// Total number of unique Sierra statements covered so far.
    new_coverage_statements_counter: std::sync::atomic::AtomicUsize,
    /// Whether to record statement coverage from `relocated_trace` during execution.
    collect_coverage: std::sync::atomic::AtomicBool,
    /// The full Sierra program and metadata config, kept so nested contract calls can run on a
    /// per-class slice of the program instead of reloading everything for every call.
    source_program: Option<Arc<(cairo_lang_sierra::program::Program, Option<MetadataComputationConfig>)>>,
    /// Per-class sub-runners, shared between the root runner and every sub-runner.
    class_runners: Arc<std::sync::RwLock<HashMap<Felt252, Arc<SierraCasmRunner>>>>,
    /// Lock-free snapshot of `class_runners` once every class has been built (`freeze`).
    /// Nested calls on the hot path read this instead of taking the RwLock.
    class_runners_frozen: Arc<std::sync::OnceLock<HashMap<Felt252, Arc<SierraCasmRunner>>>>,
    /// Lock-free snapshot of `program_cache` for a class runner's entry points.
    programs_frozen: std::sync::OnceLock<HashMap<cairo_lang_sierra::ids::FunctionId, CachedFunctionProgram>>,
    /// Per-test-function sub-runners (the slice reachable from one entry function).
    function_runners: std::sync::RwLock<HashMap<cairo_lang_sierra::ids::FunctionId, Arc<SierraCasmRunner>>>,
}
impl SierraCasmRunner {
    pub fn new(
        sierra_program: cairo_lang_sierra::program::Program,
        metadata_config: Option<MetadataComputationConfig>,
        starknet_contracts_info: OrderedHashMap<Felt252, ContractInfo>,
        run_profiler: Option<ProfilingInfoCollectionConfig>,
    ) -> Result<Self, RunnerError> {
        let source = Arc::new((sierra_program.clone(), metadata_config.clone()));
        let mut runner = Self::new_inner(sierra_program, metadata_config, starknet_contracts_info, run_profiler)?;
        runner.source_program = Some(source);
        Ok(runner)
    }

    fn new_inner(
        sierra_program: cairo_lang_sierra::program::Program,
        metadata_config: Option<MetadataComputationConfig>,
        starknet_contracts_info: OrderedHashMap<Felt252, ContractInfo>,
        run_profiler: Option<ProfilingInfoCollectionConfig>,
    ) -> Result<Self, RunnerError> {
        let builder = RunnableBuilder::new(sierra_program, metadata_config)?;
        let num_statements = builder.sierra_program().statements.len();
        let bitmap_words = num_statements.div_ceil(64);
        let coverage_bitmap = (0..bitmap_words)
            .map(|_| std::sync::atomic::AtomicU64::new(0))
            .collect();
        let base_assembled = builder.casm_program().assemble();
        let base_bytecode: Vec<cairo_vm::types::relocatable::MaybeRelocatable> = base_assembled
            .bytecode
            .iter()
            .map(Felt252::from)
            .map(cairo_vm::types::relocatable::MaybeRelocatable::from)
            .collect();
        let mut base_hints = Vec::with_capacity(base_assembled.hints.len());
        let mut base_string_to_hint = HintMap::default();
        for (offset, offset_hints) in &base_assembled.hints {
            let mut params = Vec::with_capacity(offset_hints.len());
            for hint in offset_hints {
                let param = hint_to_hint_params(hint);
                base_string_to_hint
                    .entry(param.code.clone())
                    .or_insert_with(|| Arc::new(hint.clone()));
                params.push(param);
            }
            base_hints.push((*offset, params));
        }
        Ok(Self {
            builder,
            starknet_contracts_info,
            run_profiler,
            base_bytecode,
            base_hints,
            base_string_to_hint,
            program_cache: std::sync::RwLock::new(HashMap::new()),
            coverage_bitmap,
            new_coverage_statements_counter: std::sync::atomic::AtomicUsize::new(0),
            collect_coverage: std::sync::atomic::AtomicBool::new(false),
            source_program: None,
            class_runners: Arc::new(std::sync::RwLock::new(HashMap::new())),
            class_runners_frozen: Arc::new(std::sync::OnceLock::new()),
            programs_frozen: std::sync::OnceLock::new(),
            function_runners: std::sync::RwLock::new(HashMap::new()),
        })
    }

    /// Builds a sub-runner for the slice of the program reachable from `roots`, sharing this
    /// runner's class runners so nested calls keep working. `None` when slicing is disabled
    /// (coverage, profiling, `UC_NESTED_CALL_FULL_PROGRAM`) or fails.
    fn sliced_runner(&self, roots: &[cairo_lang_sierra::ids::FunctionId]) -> Option<SierraCasmRunner> {
        if self.run_profiler.is_some()
            || self.coverage_enabled()
            || std::env::var("UC_NESTED_CALL_FULL_PROGRAM").is_ok()
        {
            return None;
        }
        let source = self.source_program.as_ref()?;
        let (program, kept) = slice_program_for_roots(&source.0, roots);
        let metadata_config = source.1.as_ref().map(|cfg| MetadataComputationConfig {
            function_set_costs: cfg
                .function_set_costs
                .iter()
                .filter(|(fid, _)| kept.contains(&fid.id))
                .map(|(fid, costs)| (fid.clone(), costs.clone()))
                .collect(),
            ..cfg.clone()
        });
        let mut sub = Self::new_inner(program, metadata_config, self.starknet_contracts_info.clone(), None).ok()?;
        sub.source_program = Some(Arc::clone(source));
        sub.class_runners = Arc::clone(&self.class_runners);
        sub.class_runners_frozen = Arc::clone(&self.class_runners_frozen);
        Some(sub)
    }

    /// Runs `func` (a test or entry function) on a sub-runner holding only the code reachable
    /// from it. The whole test crate's program is never loaded into the VM for a single test.
    /// Falls back to the full program when slicing is unavailable.
    pub fn run_function_with_starknet_context_sliced(
        &self,
        func: &Function,
        args: Vec<Arg>,
        available_gas: Option<usize>,
        starknet_state: StarknetState,
    ) -> Result<RunResultStarknet, RunnerError> {
        let cached = self.function_runners.read().ok().and_then(|m| m.get(&func.id).cloned());
        let debug = std::env::var("UC_NESTED_CALL_DEBUG").is_ok();
        let build_started = std::time::Instant::now();
        let sub = match cached {
            Some(sub) => Some(sub),
            None => match self.sliced_runner(std::slice::from_ref(&func.id)) {
                Some(built) => {
                    if debug {
                        eprintln!(
                            "function runner {}: {} statements built in {:.1} ms",
                            func.id,
                            built.builder.sierra_program().statements.len(),
                            build_started.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                    let built = Arc::new(built);
                    if let Ok(mut m) = self.function_runners.write() {
                        m.insert(func.id.clone(), Arc::clone(&built));
                    }
                    Some(built)
                }
                None => None,
            },
        };
        match sub {
            Some(sub) => {
                let Ok(sub_func) = sub.builder.registry().get_function(&func.id) else {
                    return self.run_function_with_starknet_context(func, args, available_gas, starknet_state);
                };
                let run_started = std::time::Instant::now();
                let res = sub.run_function_with_starknet_context(sub_func, args, available_gas, starknet_state);
                if debug {
                    eprintln!("function run {}: {:.1} ms", func.id, run_started.elapsed().as_secs_f64() * 1000.0);
                }
                res
            }
            None => self.run_function_with_starknet_context(func, args, available_gas, starknet_state),
        }
    }

    /// Finalizes the per-class runners: snapshots them into a lock-free map and pre-assembles
    /// every entry point's program, so nested calls never take a lock. Call once after warming
    /// all classes with [Self::class_runner]; later classes fall back to `self`.
    pub fn freeze_class_runners(&self) {
        let Ok(map) = self.class_runners.read() else { return };
        for (class_hash, sub) in map.iter() {
            let Some(info) = self.starknet_contracts_info.get(class_hash) else { continue };
            let mut programs = HashMap::new();
            for fid in info.constructor.iter().chain(info.externals.values()).chain(info.l1_handlers.values()) {
                if let Ok(func) = sub.builder.registry().get_function(fid) {
                    if let Ok(cached) = sub.get_or_build_cached_program(func) {
                        programs.insert(fid.clone(), cached);
                    }
                }
            }
            let _ = sub.programs_frozen.set(programs);
        }
        let _ = self.class_runners_frozen.set(map.clone());
    }

    /// Returns a runner whose program contains only the functions reachable from `class_hash`'s
    /// entry points. Nested `call_contract` / `library_call` / `deploy` run on it, so each call
    /// loads a program the size of one contract instead of the whole test crate. Built once per
    /// class and shared. Returns `None` (caller keeps using `self`) when coverage or profiling
    /// is on, since those are keyed to the full program's statement indices, or when the class
    /// cannot be sliced.
    pub fn class_runner(&self, class_hash: &Felt252) -> Option<Arc<SierraCasmRunner>> {
        if self.run_profiler.is_some()
            || self.coverage_enabled()
            || std::env::var("UC_NESTED_CALL_FULL_PROGRAM").is_ok()
        {
            return None;
        }
        if let Some(frozen) = self.class_runners_frozen.get() {
            return frozen.get(class_hash).cloned();
        }
        if let Some(found) = self.class_runners.read().ok()?.get(class_hash) {
            return Some(Arc::clone(found));
        }
        // Build under the write lock so parallel tests do not each compile the same slice.
        let mut map = self.class_runners.write().ok()?;
        if let Some(found) = map.get(class_hash) {
            return Some(Arc::clone(found));
        }
        let info = self.starknet_contracts_info.get(class_hash)?;
        let roots: Vec<cairo_lang_sierra::ids::FunctionId> = info
            .constructor
            .iter()
            .chain(info.externals.values())
            .chain(info.l1_handlers.values())
            .cloned()
            .collect();
        let sub = self.sliced_runner(&roots)?;
        if std::env::var("UC_NESTED_CALL_DEBUG").is_ok() {
            eprintln!(
                "class runner {class_hash:#x}: {} statements, {} funcs",
                sub.builder.sierra_program().statements.len(),
                sub.builder.sierra_program().funcs.len()
            );
        }
        let sub = Arc::new(sub);
        map.insert(*class_hash, Arc::clone(&sub));
        Some(sub)
    }

    /// Drops the cached per-function runner for `func` (call when a test has finished all of
    /// its runs); keeping hundreds of compiled slices alive costs memory, not time.
    pub fn release_function_runner(&self, func: &Function) {
        if let Ok(mut m) = self.function_runners.write() {
            m.remove(&func.id);
        }
    }

    /// Builds and registers the per-class runner for `class_hash` without holding the lock
    /// during compilation (used for parallel warmup over distinct classes).
    pub fn warm_class_runner(&self, class_hash: &Felt252) {
        if self.class_runners.read().ok().is_some_and(|m| m.contains_key(class_hash)) {
            return;
        }
        let Some(info) = self.starknet_contracts_info.get(class_hash) else { return };
        let roots: Vec<cairo_lang_sierra::ids::FunctionId> = info
            .constructor
            .iter()
            .chain(info.externals.values())
            .chain(info.l1_handlers.values())
            .cloned()
            .collect();
        if let Some(sub) = self.sliced_runner(&roots) {
            if let Ok(mut m) = self.class_runners.write() {
                m.entry(*class_hash).or_insert_with(|| Arc::new(sub));
            }
        }
    }

    /// Class hashes known to this runner (for warming per-class runners up front).
    pub fn known_class_hashes(&self) -> Vec<Felt252> {
        self.starknet_contracts_info.keys().copied().collect()
    }

    /// Enables or disables Sierra statement coverage collection during VM execution.
    pub fn set_collect_coverage(&self, enabled: bool) {
        self.collect_coverage
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether statement coverage is recorded for runs on the current thread: a thread-scoped
    /// override (see [Self::scoped_coverage]) wins over the runner-wide flag.
    pub fn coverage_enabled(&self) -> bool {
        COVERAGE_OVERRIDE
            .with(|c| c.get())
            .unwrap_or_else(|| self.collect_coverage.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Enables coverage recording for runs on the current thread only, until the guard drops.
    /// Used by fuzzing for corpus guidance so other tests, and the tests that run afterwards,
    /// keep the fast sliced path.
    pub fn scoped_coverage(&self, enabled: bool) -> ScopedCoverage {
        ScopedCoverage::new(enabled)
    }

    /// Returns user-visible parameter `(generic_type_id, felt_word_size)` pairs for `func`.
    pub fn user_param_types_and_sizes(&self, func: &Function) -> Vec<(String, usize)> {
        self.builder
            .generic_id_and_size_from_concrete(&func.signature.param_types)
            .into_iter()
            .filter(|(ty, _)| self.builder.is_user_arg_type(ty))
            .map(|(ty, sz)| {
                let size_usize: usize = sz.into_or_panic();
                (ty.0.to_string(), size_usize)
            })
            .collect()
    }

    /// Returns the number of unique Sierra statements executed across all runs on this runner.
    pub fn covered_statement_count(&self) -> usize {
        self.new_coverage_statements_counter
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Returns the total number of Sierra statements in the compiled program.
    pub fn total_statement_count(&self) -> usize {
        self.builder.sierra_program().statements.len()
    }

    /// Returns per-function Sierra statement coverage `(function_debug_name, covered_stmts, total_stmts)`.
    pub fn function_coverage_summary(&self) -> Vec<(String, usize, usize)> {
        let total_stmts = self.builder.sierra_program().statements.len();
        let mut sorted_funcs: Vec<(usize, String)> = self
            .builder
            .sierra_program()
            .funcs
            .iter()
            .map(|f| {
                let name = f
                    .id
                    .debug_name
                    .as_ref()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| format!("func_{}", f.id.id));
                (f.entry_point.0, name)
            })
            .collect();
        sorted_funcs.sort_by_key(|(entry, _)| *entry);
        let mut out = Vec::with_capacity(sorted_funcs.len());
        for i in 0..sorted_funcs.len() {
            let start = sorted_funcs[i].0;
            let end = sorted_funcs
                .get(i + 1)
                .map(|(next_start, _)| *next_start)
                .unwrap_or(total_stmts)
                .max(start);
            let mut covered = 0usize;
            for stmt_idx in start..end {
                let word_idx = stmt_idx / 64;
                let mask = 1u64 << (stmt_idx % 64);
                if let Some(word) = self.coverage_bitmap.get(word_idx)
                    && (word.load(std::sync::atomic::Ordering::Relaxed) & mask) != 0
                {
                    covered += 1;
                }
            }
            out.push((sorted_funcs[i].1.clone(), covered, end - start));
        }
        out
    }

    fn get_or_build_cached_program(
        &self,
        func: &Function,
    ) -> Result<CachedFunctionProgram, RunnerError> {
        if let Some(cached) = self.programs_frozen.get().and_then(|m| m.get(&func.id)) {
            return Ok(cached.clone());
        }
        if let Some(cached) = self.program_cache.read().unwrap().get(&func.id).cloned() {
            return Ok(cached);
        }
        let info = self.builder.create_wrapper_info(func, EntryCodeConfig::testing())?;
        let mut header_bytecode = Vec::new();
        let mut hints_dict: HashMap<usize, Vec<HintParams>> =
            HashMap::with_capacity(self.base_hints.len() + 4);
        let mut header_string_to_hint = HintMap::default();
        for instruction in &info.header {
            if !instruction.hints.is_empty() {
                let offset = header_bytecode.len();
                let mut params = Vec::with_capacity(instruction.hints.len());
                for hint in &instruction.hints {
                    let param = hint_to_hint_params(hint);
                    header_string_to_hint.insert(param.code.clone(), Arc::new(hint.clone()));
                    params.push(param);
                }
                hints_dict.insert(offset, params);
            }
            for word in instruction.assemble().encode() {
                header_bytecode.push(cairo_vm::types::relocatable::MaybeRelocatable::from(
                    Felt252::from(word),
                ));
            }
        }
        let header_len = header_bytecode.len();
        for (offset, params) in &self.base_hints {
            hints_dict.insert(header_len + *offset, params.clone());
        }
        let mut footer_bytecode = Vec::new();
        for instruction in &info.footer {
            for word in instruction.assemble().encode() {
                footer_bytecode.push(cairo_vm::types::relocatable::MaybeRelocatable::from(
                    Felt252::from(word),
                ));
            }
        }
        let data_len = header_len + self.base_bytecode.len() + footer_bytecode.len();
        let mut data = Vec::with_capacity(data_len);
        data.extend(header_bytecode);
        data.extend(self.base_bytecode.iter().cloned());
        data.extend(footer_bytecode);
        let program = casm_run::build_cairo_program(data, info.builtins, hints_dict)?;
        let cached = CachedFunctionProgram { program, data_len, header_string_to_hint };
        self.program_cache.write().unwrap().insert(func.id.clone(), cached.clone());
        Ok(cached)
    }

    /// Runs the VM starting from a function in the context of a given Starknet state.
    pub fn run_function_with_starknet_context(
        &self,
        func: &Function,
        args: Vec<Arg>,
        available_gas: Option<usize>,
        starknet_state: StarknetState,
    ) -> Result<RunResultStarknet, RunnerError> {
        let cached = self.get_or_build_cached_program(func)?;
        let user_args = self.prepare_args(func, available_gas, args)?;
        let mut hint_processor = CairoHintProcessor {
            runner: Some(self),
            user_args,
            starknet_state,
            string_to_hint: cached.header_string_to_hint,
            run_resources: RunResources::default(),
            syscalls_used_resources: Default::default(),
            no_temporary_segments: true,
            markers: Default::default(),
            panic_traceback: Default::default(),
        };
        let RunResult { gas_counter, memory, return_array, value, used_resources, profiling_info } =
            self.run_cached_program(func, &mut hint_processor, &cached.program, cached.data_len)?;
        let mut all_used_resources = hint_processor.take_syscalls_used_resources();
        all_used_resources.basic_resources += &used_resources;
        Ok(RunResultStarknet {
            gas_counter,
            memory,
            return_array,
            value,
            starknet_state: hint_processor.take_starknet_state(),
            used_resources: all_used_resources,
            profiling_info,
        })
    }

    /// Runs the VM starting from a function in the context of a given Starknet state and a
    /// (possibly) amended hint processor.
    pub fn run_function_with_prepared_starknet_context(
        &self,
        func: &Function,
        hint_processor: &mut dyn StarknetHintProcessor,
        PreparedStarknetContext { hints_dict, bytecode, builtins }: PreparedStarknetContext,
    ) -> Result<RunResultStarknet, RunnerError> {
        let RunResult { gas_counter, memory, return_array, value, used_resources, profiling_info } =
            self.run_function(func, hint_processor, hints_dict, bytecode.iter(), builtins)?;
        let mut all_used_resources = hint_processor.take_syscalls_used_resources();
        all_used_resources.basic_resources += &used_resources;
        Ok(RunResultStarknet {
            gas_counter,
            memory,
            return_array,
            value,
            starknet_state: hint_processor.take_starknet_state(),
            used_resources: all_used_resources,
            profiling_info,
        })
    }

    /// Extract inner type if `ty` is a panic wrapper.
    fn inner_type_from_panic_wrapper(
        &self,
        ty: &GenericTypeId,
        func: &Function,
    ) -> Option<ConcreteTypeId> {
        let generic_args = &func
            .signature
            .ret_types
            .iter()
            .find_map(|rt| {
                let long_id = self.builder.type_long_id(rt);
                (long_id.generic_id == *ty).then_some(long_id)
            })
            .unwrap()
            .generic_args;

        if *ty == EnumType::ID
            && matches!(&generic_args[0], GenericArg::UserType(ut)
                if ut.debug_name.as_ref().unwrap().starts_with("core::panics::PanicResult::"))
        {
            return Some(extract_matches!(&generic_args[1], GenericArg::Type).clone());
        }
        None
    }

    /// Runs the VM starting from a function with a custom hint processor. The function may have
    /// implicits, but no other ref params. The cost of the function is deducted from
    /// `available_gas` before the execution begins.
    pub fn run_function<'a, Bytecode>(
        &self,
        func: &Function,
        hint_processor: &mut dyn HintProcessor,
        hints_dict: HashMap<usize, Vec<HintParams>>,
        bytecode: Bytecode,
        builtins: Vec<BuiltinName>,
    ) -> Result<RunResult, RunnerError>
    where
        Bytecode: ExactSizeIterator<Item = &'a BigInt> + Clone,
    {
        let return_types =
            self.builder.generic_id_and_size_from_concrete(&func.signature.ret_types);
        let data_len = bytecode.len();
        let RunFunctionResult { ap, mut used_resources, memory, relocated_trace } =
            casm_run::run_function(
                bytecode,
                builtins,
                |vm| initialize_vm(vm, data_len),
                hint_processor,
                hints_dict,
            )?;

        // The execution from the header created by self.builder.create_entry_code().
        // We expect the last trace entry to be the `ret` instruction at the end of the header.
        let header_end = relocated_trace.last().unwrap().pc;
        used_resources.n_steps -=
            relocated_trace.iter().position(|e| e.pc > header_end).unwrap() - 1;
        used_resources.n_steps -=
            relocated_trace.iter().rev().position(|e| e.pc > header_end).unwrap() - 1;

        let (results_data, gas_counter) = self.get_results_data(&return_types, &memory, ap);
        assert!(results_data.len() <= 1);

        let value = match results_data.into_iter().next() {
            // No result type - no panic.
            None => RunResultValue::Success(vec![]),
            Some((ty, values)) => {
                let inner_ty = self
                    .inner_type_from_panic_wrapper(&ty, func)
                    .map(|it| self.builder.type_size(&it));
                Self::handle_main_return_value(inner_ty, values, &memory)
            }
        };

        let Self { builder, starknet_contracts_info: _, run_profiler, .. } = self;

        // The real program starts right after the header.
        let load_offset = header_end + 1;
        if self.coverage_enabled() {
            let base_len = self.base_bytecode.len();
            for entry in &relocated_trace {
                if entry.pc >= load_offset {
                    let rel_pc = entry.pc - load_offset;
                    if rel_pc < base_len {
                        let stmt_idx = self
                            .builder
                            .casm_program()
                            .sierra_statement_index_by_pc(rel_pc)
                            .0;
                        let word_idx = stmt_idx / 64;
                        let bit_mask = 1u64 << (stmt_idx % 64);
                        if let Some(word) = self.coverage_bitmap.get(word_idx) {
                            let prev =
                                word.fetch_or(bit_mask, std::sync::atomic::Ordering::Relaxed);
                            if prev & bit_mask == 0 {
                                self.new_coverage_statements_counter
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                    }
                }
            }
        }

        let profiling_info = run_profiler.as_ref().map(|config| {
            ProfilingInfo::from_trace(builder, load_offset, config, &relocated_trace)
        });

        Ok(RunResult { gas_counter, memory, return_array: None, value, used_resources, profiling_info })
    }

    /// Runs a pre-built cached VM `Program` starting from `func`.
    pub fn run_cached_program(
        &self,
        func: &Function,
        hint_processor: &mut dyn HintProcessor,
        program: &cairo_vm::types::program::Program,
        data_len: usize,
    ) -> Result<RunResult, RunnerError> {
        let return_types =
            self.builder.generic_id_and_size_from_concrete(&func.signature.ret_types);
        let RunProgramLazyResult { ap, mut used_resources, relocated_trace, relocation_table, runner } =
            casm_run::run_program_lazy(
                program,
                |vm| initialize_vm(vm, data_len),
                hint_processor,
            )?;

        let header_end = relocated_trace.last().unwrap().pc;
        used_resources.n_steps -=
            relocated_trace.iter().position(|e| e.pc > header_end).unwrap() - 1;
        used_resources.n_steps -=
            relocated_trace.iter().rev().position(|e| e.pc > header_end).unwrap() - 1;

        let (results_data, gas_counter) =
            self.get_results_data_from_vm(&return_types, &runner.vm, &relocation_table, ap);
        assert!(results_data.len() <= 1);

        let (value, return_array) = match results_data.into_iter().next() {
            None => (RunResultValue::Success(vec![]), None),
            Some((ty, values)) => {
                let inner_ty = self
                    .inner_type_from_panic_wrapper(&ty, func)
                    .map(|it| self.builder.type_size(&it));
                Self::handle_main_return_value_from_vm(
                    inner_ty,
                    values,
                    &runner.vm,
                    &relocation_table,
                )
            }
        };
        // Memory is intentionally not relocated on this path (see `run_program_lazy`).
        let memory = Vec::new();

        let Self { builder, starknet_contracts_info: _, run_profiler, .. } = self;

        let load_offset = header_end + 1;
        if self.coverage_enabled() {
            let base_len = self.base_bytecode.len();
            for entry in &relocated_trace {
                if entry.pc >= load_offset {
                    let rel_pc = entry.pc - load_offset;
                    if rel_pc < base_len {
                        let stmt_idx = self
                            .builder
                            .casm_program()
                            .sierra_statement_index_by_pc(rel_pc)
                            .0;
                        let word_idx = stmt_idx / 64;
                        let bit_mask = 1u64 << (stmt_idx % 64);
                        if let Some(word) = self.coverage_bitmap.get(word_idx) {
                            let prev =
                                word.fetch_or(bit_mask, std::sync::atomic::Ordering::Relaxed);
                            if prev & bit_mask == 0 {
                                self.new_coverage_statements_counter
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                    }
                }
            }
        }

        let profiling_info = run_profiler.as_ref().map(|config| {
            ProfilingInfo::from_trace(builder, load_offset, config, &relocated_trace)
        });

        Ok(RunResult { gas_counter, memory, return_array, value, used_resources, profiling_info })
    }

    /// Prepares context for running a function in the context of a given Starknet state.
    ///
    /// The returned hint processor instance is set up for interpreting and executing the hints
    /// provided during the Cairo program's execution. Can be customized by wrapping into a custom
    /// hint processor implementation and passing that to the `run_function` method.
    pub fn prepare_starknet_context(
        &self,
        func: &Function,
        args: Vec<Arg>,
        available_gas: Option<usize>,
        starknet_state: StarknetState,
    ) -> Result<(CairoHintProcessor<'_>, PreparedStarknetContext), RunnerError> {
        let (assembled_program, builtins) =
            self.builder.assemble_function_program(func, EntryCodeConfig::testing())?;
        let (hints_dict, string_to_hint) = build_hints_dict(&assembled_program.hints);
        let user_args = self.prepare_args(func, available_gas, args)?;
        let hint_processor = CairoHintProcessor {
            runner: Some(self),
            user_args,
            starknet_state,
            string_to_hint,
            run_resources: RunResources::default(),
            syscalls_used_resources: Default::default(),
            no_temporary_segments: true,
            markers: Default::default(),
            panic_traceback: Default::default(),
        };
        Ok((
            hint_processor,
            PreparedStarknetContext { hints_dict, bytecode: assembled_program.bytecode, builtins },
        ))
    }

    /// Groups the args by parameters, and additionally adds `gas` as the first if required.
    fn prepare_args(
        &self,
        func: &Function,
        available_gas: Option<usize>,
        args: Vec<Arg>,
    ) -> Result<Vec<Vec<Arg>>, RunnerError> {
        let mut user_args = vec![];
        if let Some(gas) = self
            .requires_gas_builtin(func)
            .then_some(self.get_initial_available_gas(func, available_gas)?)
        {
            user_args.push(vec![Arg::Value(Felt252::from(gas))]);
        }
        let mut expected_arguments_size = 0;
        let actual_args_size = args_size(&args);
        let synthesize_fuzzer_defaults = actual_args_size == 0;
        let mut arg_iter = args.into_iter().enumerate();
        for (param_index, (ty, param_size)) in self
            .builder
            .generic_id_and_size_from_concrete(&func.signature.param_types)
            .into_iter()
            .filter(|(ty, _)| self.builder.is_user_arg_type(ty))
            .enumerate()
        {
            let mut curr_arg = vec![];
            let param_size: usize = param_size.into_or_panic();
            expected_arguments_size += param_size;
            let mut taken_size = 0;
            while taken_size < param_size {
                let Some((arg_index, arg)) = arg_iter.next() else {
                    break;
                };
                taken_size += arg.size();
                if taken_size > param_size {
                    return Err(RunnerError::ArgumentUnaligned { param_index, arg_index });
                }
                curr_arg.push(arg);
            }
            if synthesize_fuzzer_defaults && taken_size < param_size {
                let ty_name = ty.0.as_str();
                for word_idx in taken_size..param_size {
                    let default_felt = if word_idx > 0 {
                        Felt252::from(0_u64)
                    } else if ty_name == "u64" || ty_name == "i64" {
                        Felt252::from(2000_u64)
                    } else if ty_name == "Enum" {
                        Felt252::from(0_u64)
                    } else {
                        Felt252::from(10_u64)
                    };
                    curr_arg.push(Arg::Value(default_felt));
                }
            }
            user_args.push(curr_arg);
        }
        if !synthesize_fuzzer_defaults && expected_arguments_size != actual_args_size {
            return Err(RunnerError::ArgumentsSizeMismatch {
                expected: expected_arguments_size,
                actual: actual_args_size,
            });
        }
        Ok(user_args)
    }

    /// Handling the main return value to create a `RunResultValue`.
    pub fn handle_main_return_value(
        inner_type_size: Option<i16>,
        values: Vec<Felt252>,
        cells: &[Option<Felt252>],
    ) -> RunResultValue {
        if let Some(inner_type_size) = inner_type_size {
            // The function includes a panic wrapper.
            if values[0] == Felt252::from(0) {
                // The run resulted successfully, returning the inner value.
                let inner_ty_size = inner_type_size as usize;
                let skip_size = values.len() - inner_ty_size;
                RunResultValue::Success(values.into_iter().skip(skip_size).collect())
            } else {
                // The run resulted in a panic, returning the error data.
                let err_data_start = values[values.len() - 2].to_usize().unwrap();
                let err_data_end = values[values.len() - 1].to_usize().unwrap();
                RunResultValue::Panic(
                    cells[err_data_start..err_data_end]
                        .iter()
                        .cloned()
                        .map(Option::unwrap)
                        .collect(),
                )
            }
        } else {
            // No panic wrap - so always successful.
            RunResultValue::Success(values)
        }
    }

    /// Returns the final values and type of all `func`s returning variables.
    /// Converts a raw VM cell to its relocated felt value.
    fn relocate_cell(cell: &MaybeRelocatable, relocation_table: &[usize]) -> Felt252 {
        match cell {
            MaybeRelocatable::Int(value) => *value,
            MaybeRelocatable::RelocatableValue(ptr) => Felt252::from(
                relocation_table[ptr.segment_index as usize] + ptr.offset,
            ),
        }
    }

    /// Reads `len` consecutive cells starting at `start` directly from the unrelocated VM.
    fn read_vm_cells(vm: &VirtualMachine, start: Relocatable, len: usize) -> Vec<MaybeRelocatable> {
        vm.get_range(start, len)
            .into_iter()
            .map(|cell| cell.expect("return data cell should be initialized").into_owned())
            .collect()
    }

    /// Like [Self::handle_main_return_value] but reads panic data from the unrelocated VM.
    pub fn handle_main_return_value_from_vm(
        inner_type_size: Option<i16>,
        values: Vec<MaybeRelocatable>,
        vm: &VirtualMachine,
        relocation_table: &[usize],
    ) -> (RunResultValue, Option<Vec<Felt252>>) {
        let to_felts = |cells: Vec<MaybeRelocatable>| -> Vec<Felt252> {
            cells.iter().map(|c| Self::relocate_cell(c, relocation_table)).collect()
        };
        // A `[start, end]` pointer pair is an `Array<felt252>` / `Span<felt252>` return value;
        // materialize it now since the caller cannot read unrelocated memory later.
        let array_of = |cells: &[MaybeRelocatable]| -> Option<Vec<Felt252>> {
            match cells {
                [MaybeRelocatable::RelocatableValue(start), MaybeRelocatable::RelocatableValue(end)]
                    if start.segment_index == end.segment_index && end.offset >= start.offset =>
                {
                    Some(to_felts(Self::read_vm_cells(vm, *start, end.offset - start.offset)))
                }
                _ => None,
            }
        };
        if let Some(inner_type_size) = inner_type_size {
            if values[0] == MaybeRelocatable::Int(Felt252::from(0)) {
                let inner_ty_size = inner_type_size as usize;
                let skip_size = values.len() - inner_ty_size;
                let inner: Vec<MaybeRelocatable> = values.into_iter().skip(skip_size).collect();
                let array = array_of(&inner);
                (RunResultValue::Success(to_felts(inner)), array)
            } else {
                let start = match &values[values.len() - 2] {
                    MaybeRelocatable::RelocatableValue(ptr) => *ptr,
                    MaybeRelocatable::Int(_) => panic!("panic data start is not a pointer"),
                };
                let end = match &values[values.len() - 1] {
                    MaybeRelocatable::RelocatableValue(ptr) => *ptr,
                    MaybeRelocatable::Int(_) => panic!("panic data end is not a pointer"),
                };
                let len = end.offset.saturating_sub(start.offset);
                (RunResultValue::Panic(to_felts(Self::read_vm_cells(vm, start, len))), None)
            }
        } else {
            let array = array_of(&values);
            (RunResultValue::Success(to_felts(values)), array)
        }
    }

    /// Like [Self::get_results_data] but reads the cells from the unrelocated VM.
    pub fn get_results_data_from_vm(
        &self,
        return_types: &[(GenericTypeId, i16)],
        vm: &VirtualMachine,
        relocation_table: &[usize],
        mut ap: Relocatable,
    ) -> (Vec<(GenericTypeId, Vec<MaybeRelocatable>)>, Option<Felt252>) {
        let mut results_data = vec![];
        for (ty, ty_size) in return_types.iter().rev() {
            let size = *ty_size as usize;
            ap.offset -= size;
            results_data.push((ty.clone(), Self::read_vm_cells(vm, ap, size)));
        }

        let mut gas_counter = None;
        results_data.retain_mut(|(ty, values)| {
            if *ty == GasBuiltinType::ID {
                gas_counter = Some(Self::relocate_cell(&values.remove(0), relocation_table));
                assert!(values.is_empty());
                false
            } else {
                self.builder.is_user_arg_type(ty)
            }
        });

        (results_data, gas_counter)
    }

    pub fn get_results_data(
        &self,
        return_types: &[(GenericTypeId, i16)],
        cells: &[Option<Felt252>],
        mut ap: usize,
    ) -> (Vec<(GenericTypeId, Vec<Felt252>)>, Option<Felt252>) {
        let mut results_data = vec![];
        for (ty, ty_size) in return_types.iter().rev() {
            let size = *ty_size as usize;
            let values: Vec<Felt252> =
                ((ap - size)..ap).map(|index| cells[index].unwrap()).collect();
            ap -= size;
            results_data.push((ty.clone(), values));
        }

        // Handling implicits.
        let mut gas_counter = None;
        results_data.retain_mut(|(ty, values)| {
            let generic_ty = ty;
            if *generic_ty == GasBuiltinType::ID {
                gas_counter = Some(values.remove(0));
                assert!(values.is_empty());
                false
            } else {
                self.builder.is_user_arg_type(generic_ty)
            }
        });

        (results_data, gas_counter)
    }

    /// Finds the first function ending with `name_suffix`.
    pub fn find_function(&self, name_suffix: &str) -> Result<&Function, RunnerError> {
        Ok(self.builder.find_function(name_suffix)?)
    }

    /// Returns whether the gas builtin is required in the given function.
    fn requires_gas_builtin(&self, func: &Function) -> bool {
        func.signature
            .param_types
            .iter()
            .any(|ty| self.builder.type_long_id(ty).generic_id == GasBuiltinType::ID)
    }

    /// Returns the initial value for the gas counter.
    /// If `available_gas` is None returns 0.
    pub fn get_initial_available_gas(
        &self,
        func: &Function,
        available_gas: Option<usize>,
    ) -> Result<usize, RunnerError> {
        let Some(available_gas) = available_gas else {
            return Ok(0);
        };

        // In case we don't have any costs - it means no gas equations were solved (and we are in
        // the case of no gas checking enabled) - so the gas builtin is irrelevant, and we
        // can return any value.
        let Some(required_gas) = self.initial_required_gas(func) else {
            return Ok(0);
        };

        available_gas.checked_sub(required_gas).ok_or(RunnerError::NotEnoughGasToCall)
    }

    pub fn initial_required_gas(&self, func: &Function) -> Option<usize> {
        let gas_info = &self.builder.metadata().gas_info;
        require(!gas_info.function_costs.is_empty())?;
        Some(
            gas_info.function_costs[&func.id]
                .iter()
                .map(|(token_type, val)| val.into_or_panic::<usize>() * token_gas_cost(*token_type))
                .sum(),
        )
    }
}

/// Configuration for the profiling info collection phase.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct ProfilingInfoCollectionConfig {
    /// The maximum depth of the stack trace to collect.
    pub max_stack_trace_depth: usize,
    /// If this flag is set, in addition to the Sierra statement weights and stack trace weights
    /// the runner will also collect weights for Sierra statements taking into account current call
    /// stack and collapsing recursive calls (which also includes loops).
    /// The resulting dictionary can be pretty huge hence this feature is optional and disabled by
    /// default.
    pub collect_scoped_sierra_statement_weights: bool,
}

impl ProfilingInfoCollectionConfig {
    pub fn set_max_stack_trace_depth(&mut self, max_stack_depth: usize) -> &mut Self {
        self.max_stack_trace_depth = max_stack_depth;
        self
    }

    pub fn from_profiler_config(profiler_config: &ProfilerConfig) -> Self {
        match profiler_config {
            ProfilerConfig::Cairo | ProfilerConfig::Sierra => Self::default(),
            ProfilerConfig::Scoped => ProfilingInfoCollectionConfig {
                collect_scoped_sierra_statement_weights: true,
                ..Self::default()
            },
        }
    }
}

impl Default for ProfilingInfoCollectionConfig {
    // TODO(yuval): consider changing this setting to use flags.
    /// Gets the max_stack_trace_depth according to the environment variable
    /// `MAX_STACK_TRACE_DEPTH`, if set.
    fn default() -> Self {
        Self {
            max_stack_trace_depth: if let Ok(max) = std::env::var("MAX_STACK_TRACE_DEPTH") {
                if max.is_empty() {
                    MAX_STACK_TRACE_DEPTH_DEFAULT
                } else {
                    max.parse::<usize>().expect("MAX_STACK_TRACE_DEPTH env var is not numeric")
                }
            } else {
                MAX_STACK_TRACE_DEPTH_DEFAULT
            },
            collect_scoped_sierra_statement_weights: false,
        }
    }
}

/// Initializes a VM by adding a new segment with builtins cost and a necessary pointer at the end
/// of the program, as well as placing the arguments at the initial AP values.
pub fn initialize_vm(vm: &mut VirtualMachine, data_len: usize) -> Result<(), Box<CairoRunError>> {
    // Create the builtin cost segment, with dummy values.
    let builtin_cost_segment = vm.add_memory_segment();
    for token_type in CostTokenType::iter_precost() {
        vm.insert_value(
            (builtin_cost_segment + (token_type.offset_in_builtin_costs() as usize)).unwrap(),
            Felt252::from(token_gas_cost(*token_type)),
        )
        .map_err(|e| Box::new(e.into()))?;
    }
    // Put a pointer to the builtin cost segment at the end of the program (after the
    // additional `ret` statement).
    vm.insert_value((vm.get_pc() + data_len).unwrap(), builtin_cost_segment)
        .map_err(|e| Box::new(e.into()))?;
    Ok(())
}

/// The size in memory of the arguments.
fn args_size(args: &[Arg]) -> usize {
    args.iter().map(Arg::size).sum()
}


/// Keeps only the functions reachable from `roots` (through `function_call` and coupon
/// libfuncs), re-laying their statements contiguously and remapping intra-function branch
/// targets. Type declarations are kept; libfunc declarations that name a dropped function are
/// removed so the registry does not look it up. Returns the sliced program and the kept ids.
fn slice_program_for_roots(
    program: &cairo_lang_sierra::program::Program,
    roots: &[cairo_lang_sierra::ids::FunctionId],
) -> (cairo_lang_sierra::program::Program, std::collections::HashSet<u64>) {
    use cairo_lang_sierra::program::{GenBranchTarget, GenStatement, StatementIdx};
    use std::collections::{HashSet, VecDeque};

    let n = program.funcs.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| program.funcs[i].entry_point.0);
    let mut ranges = vec![(0usize, 0usize); n];
    for (pos, &i) in order.iter().enumerate() {
        let start = program.funcs[i].entry_point.0;
        let end = order.get(pos + 1).map(|&j| program.funcs[j].entry_point.0).unwrap_or(program.statements.len());
        ranges[i] = (start, end);
    }
    let by_id: HashMap<u64, usize> = program.funcs.iter().enumerate().map(|(i, f)| (f.id.id, i)).collect();
    let libfunc_callee: HashMap<u64, u64> = program
        .libfunc_declarations
        .iter()
        .filter_map(|ld| {
            ld.long_id.generic_args.iter().find_map(|a| match a {
                GenericArg::UserFunc(fid) => Some((ld.id.id, fid.id)),
                _ => None,
            })
        })
        .collect();

    let mut kept: HashSet<u64> = HashSet::new();
    let mut queue: VecDeque<usize> = roots.iter().filter_map(|r| by_id.get(&r.id).copied()).collect();
    while let Some(i) = queue.pop_front() {
        if !kept.insert(program.funcs[i].id.id) {
            continue;
        }
        let (start, end) = ranges[i];
        for stmt in &program.statements[start..end] {
            if let GenStatement::Invocation(inv) = stmt {
                if let Some(callee) = libfunc_callee.get(&inv.libfunc_id.id) {
                    if let Some(&ci) = by_id.get(callee) {
                        if !kept.contains(callee) {
                            queue.push_back(ci);
                        }
                    }
                }
            }
        }
    }

    let mut statements = Vec::new();
    let mut funcs = Vec::new();
    for &i in &order {
        if !kept.contains(&program.funcs[i].id.id) {
            continue;
        }
        let (start, end) = ranges[i];
        let new_start = statements.len();
        for stmt in &program.statements[start..end] {
            statements.push(match stmt {
                GenStatement::Invocation(inv) => GenStatement::Invocation(cairo_lang_sierra::program::GenInvocation {
                    libfunc_id: inv.libfunc_id.clone(),
                    args: inv.args.clone(),
                    branches: inv
                        .branches
                        .iter()
                        .map(|b| cairo_lang_sierra::program::GenBranchInfo {
                            target: match b.target {
                                GenBranchTarget::Fallthrough => GenBranchTarget::Fallthrough,
                                GenBranchTarget::Statement(StatementIdx(t)) => {
                                    GenBranchTarget::Statement(StatementIdx(t - start + new_start))
                                }
                            },
                            results: b.results.clone(),
                        })
                        .collect(),
                }),
                GenStatement::Return(vars) => GenStatement::Return(vars.clone()),
            });
        }
        let mut f = program.funcs[i].clone();
        f.entry_point = StatementIdx(new_start);
        funcs.push(f);
    }
    let libfunc_declarations = program
        .libfunc_declarations
        .iter()
        .filter(|ld| {
            ld.long_id.generic_args.iter().all(|a| match a {
                GenericArg::UserFunc(fid) => kept.contains(&fid.id),
                _ => true,
            })
        })
        .cloned()
        .collect();
    (
        cairo_lang_sierra::program::Program {
            type_declarations: program.type_declarations.clone(),
            libfunc_declarations,
            statements,
            funcs,
        },
        kept,
    )
}
