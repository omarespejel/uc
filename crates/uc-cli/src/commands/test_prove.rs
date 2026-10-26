//! `uc test --prove` and `uc test --verify-proofs`: Stwo proofs that tests pass.
//!
//! Each passing test without parameters is compiled the way `#[executable]` functions are: no
//! gas metering, a raw wrapper, and the standalone proof-mode header. A Stwo prover (Chihuahua by
//! default) proves the run and its verifier checks the proof. The entry code asserts that the
//! test did not panic, so a valid proof shows the test returned. Every proof's public memory
//! holds the program it proves, and uc checks it against the test's bytecode. `--verify-proofs`
//! rebuilds the programs from the current sources before verifying a bundle.

use super::*;
use cairo_lang_compiler::diagnostics::DiagnosticsReporter;
use cairo_lang_defs::plugin::{
    MacroPlugin, MacroPluginMetadata, PluginGeneratedFile, PluginResult,
};
use cairo_lang_filesystem::ids::{CrateInput, SmolStrId};
use cairo_lang_lowering::ids::ConcreteFunctionWithBodyId;
use cairo_lang_runnable_utils::builder::{EntryCodeConfig, RunnableBuilder};
use cairo_lang_syntax::node::{Terminal, ast};
use cairo_lang_syntax::node::helpers::{OptionWrappedGenericParamListHelper, QueryAttrs};
use cairo_vm::types::builtin_name::BuiltinName;
use starknet_types_core::felt::Felt;

const WRAPPER_PREFIX: &str = "__uc_prove__";
const EXECUTABLE_RAW_ATTR: &str = "executable_raw";
/// The implicit order of the Cairo executable plugin, so the wrappers match `#[executable]` ones.
const IMPLICIT_PRECEDENCE: &str = "core::pedersen::Pedersen, core::RangeCheck, \
    core::integer::Bitwise, core::ec::EcOp, core::poseidon::Poseidon, core::circuit::RangeCheck96, \
    core::circuit::AddMod, core::circuit::MulMod";
const BUNDLE_SCHEMA_VERSION: u32 = 1;
const BUNDLE_MANIFEST: &str = "manifest.json";
/// Stwo's adapter reads the entry's builtin pointers the way the bootloader passes them: all
/// eleven, in this order, used or not.
const STWO_BUILTINS: [BuiltinName; 11] = [
    BuiltinName::output,
    BuiltinName::pedersen,
    BuiltinName::range_check,
    BuiltinName::ecdsa,
    BuiltinName::bitwise,
    BuiltinName::ec_op,
    BuiltinName::keccak,
    BuiltinName::poseidon,
    BuiltinName::range_check96,
    BuiltinName::add_mod,
    BuiltinName::mul_mod,
];
/// StarkWare's mainnet settings for Cairo proofs, at which Chihuahua was measured: 27 PoW bits
/// plus 3 x 23 FRI queries, 96 bits of security. The small preprocessed trace keeps a test's
/// proof under 1 GiB; the prover's own defaults use the full trace and about 10 GiB.
const MIN_SECURITY_BITS: u64 = 96;
fn prover_params() -> serde_json::Value {
    serde_json::json!({
        "channel_hash": "blake2s_m31",
        "channel_salt": 0,
        "fri_config": {
            "pow_bits": 27,
            "log_blowup_factor": 3,
            "log_last_layer_degree_bound": 0,
            "n_queries": 23,
            "fold_step": 4,
        },
        "preprocessed_trace": "canonical_small",
        "store_polynomials_coefficients": false,
        "include_all_preprocessed_columns": false,
        "opt_n_id_to_big_components": null,
        "lifting_size_policy": "auto",
    })
}
const PROVER_PARAMS_FILE: &str = "prover_params.json";

/// The Chihuahua build `uc test --prove` was tested with.
pub(crate) const CHIHUAHUA_REV: &str = "5143288a5c0cdbbfd50a6ad61cea944a193e1ad8";

/// Adds, next to each `#[test]` function without parameters, the raw wrapper the executable
/// plugin generates for `#[executable]` functions.
#[derive(Debug, Default)]
struct TestProofWrapperPlugin;

impl MacroPlugin for TestProofWrapperPlugin {
    fn generate_code<'db>(
        &self,
        db: &'db dyn salsa::Database,
        item_ast: ast::ModuleItem<'db>,
        _metadata: &MacroPluginMetadata<'_>,
    ) -> PluginResult<'db> {
        let ast::ModuleItem::FreeFunction(item) = item_ast else {
            return PluginResult::default();
        };
        let declaration = item.declaration(db);
        if !item.has_attr(db, "test")
            || !declaration.generic_params(db).is_empty(db)
            || !declaration.signature(db).parameters(db).elements_vec(db).is_empty()
        {
            return PluginResult::default();
        }
        let name = declaration.name(db).text(db).long(db).to_string();
        let content = format!(
            "#[implicit_precedence({IMPLICIT_PRECEDENCE})]\n#[{EXECUTABLE_RAW_ATTR}]\n\
             fn {WRAPPER_PREFIX}{name}(mut input: Span<felt252>, ref _output: Array<felt252>) {{\n    \
             assert(core::array::SpanTrait::is_empty(input), 'Input too long for params.');\n    \
             {name}();\n}}\n"
        );
        PluginResult {
            code: Some(PluginGeneratedFile {
                name: "uc_prove_wrapper".into(),
                content,
                code_mappings: vec![],
                aux_data: None,
                diagnostics_note: Default::default(),
                is_unhygienic: false,
            }),
            diagnostics: vec![],
            remove_original_item: false,
        }
    }

    fn declared_attributes<'db>(&self, db: &'db dyn salsa::Database) -> Vec<SmolStrId<'db>> {
        vec![SmolStrId::from(db, EXECUTABLE_RAW_ATTR)]
    }

    fn executable_attributes<'db>(&self, db: &'db dyn salsa::Database) -> Vec<SmolStrId<'db>> {
        vec![SmolStrId::from(db, EXECUTABLE_RAW_ATTR)]
    }
}

pub(crate) fn test_proof_plugin_suite() -> cairo_lang_semantic::plugin::PluginSuite {
    let mut suite = cairo_lang_semantic::plugin::PluginSuite::default();
    suite.add_plugin::<TestProofWrapperPlugin>();
    suite
}

/// A test compiled to a Cairo executable, or why it cannot be proved.
pub(crate) enum TestExecutable {
    Ready {
        json: serde_json::Value,
        bytecode: Vec<Felt>,
    },
    Unprovable(String),
}

/// Compiles the named tests of a package to Cairo executables.
pub(crate) fn compile_test_executables(
    project: &NativeTestProject,
    crate_name: &str,
    features: &[String],
    workspace_root: &Path,
    tests: &BTreeSet<String>,
) -> Result<BTreeMap<String, TestExecutable>> {
    let NativeTestDb { db, test_crate_inputs } =
        build_native_test_db(project, crate_name, features, workspace_root, true)?;
    let mut diagnostics = String::new();
    if DiagnosticsReporter::write_to_string(&mut diagnostics)
        .with_crates(&test_crate_inputs)
        .allow_warnings()
        .check(&db)
    {
        bail!(
            "the tests do not compile without gas metering:\n{}",
            diagnostics.trim()
        );
    }
    let crate_ids = CrateInput::into_crate_ids(&db, test_crate_inputs);
    let wrappers = cairo_lang_sierra_generator::executables::find_executable_function_ids(
        &db, crate_ids,
    );
    let mut executables = BTreeMap::new();
    for function in wrappers.keys() {
        let path = function.base_semantic_function(&db).full_path(&db);
        let Some((module, wrapper)) = path.rsplit_once("::") else {
            continue;
        };
        let Some(test) = wrapper.strip_prefix(WRAPPER_PREFIX) else {
            continue;
        };
        let test_name = format!("{module}::{test}");
        if tests.contains(&test_name) {
            executables.insert(test_name, compile_test_executable(&db, *function));
        }
    }
    for test in tests {
        executables.entry(test.clone()).or_insert_with(|| {
            TestExecutable::Unprovable("takes parameters or generic arguments".to_string())
        });
    }
    Ok(executables)
}

fn compile_test_executable(
    db: &RootDatabase,
    function: ConcreteFunctionWithBodyId<'_>,
) -> TestExecutable {
    let program = match cairo_lang_compiler::get_sierra_program_for_functions(db, vec![function])
    {
        Ok(compiled) => compiled.program.clone(),
        Err(err) => return TestExecutable::Unprovable(format!("does not compile: {err}")),
    };
    let libfuncs = program.libfunc_declarations.iter();
    if let Some(libfunc) = libfuncs
        .map(|declaration| declaration.long_id.generic_id.0.as_str())
        .find(|id| id.ends_with("_syscall") || *id == "cheatcode")
    {
        return TestExecutable::Unprovable(format!(
            "calls `{libfunc}`; proofs cover Cairo without syscalls or cheatcodes"
        ));
    }
    if program
        .type_declarations
        .iter()
        .any(|declaration| declaration.long_id.generic_id.0 == "System")
    {
        return TestExecutable::Unprovable("uses the `System` implicit of syscalls".to_string());
    }
    let entry = program.funcs[0].clone();
    let builder = match RunnableBuilder::new(program, None) {
        Ok(builder) => builder,
        Err(err) => return TestExecutable::Unprovable(format!("cannot run unmetered: {err}")),
    };
    let info = match builder.create_wrapper_info(
        &entry,
        EntryCodeConfig::executable(false, Some(STWO_BUILTINS.to_vec())),
    )
    {
        Ok(info) => info,
        Err(err) => return TestExecutable::Unprovable(format!("has no executable entry: {err}")),
    };
    // The standalone proof-mode header of `Executable::new` in cairo-lang-executable.
    let header = cairo_lang_casm::casm! {
        ap += (info.builtins.len());
        call rel 4;
        jmp rel 0;
    };
    let assembled = builder
        .casm_program()
        .assemble_ex(header.instructions.iter().chain(&info.header), &info.footer);
    let bytecode = assembled.bytecode.iter().map(felt_from_bigint).collect();
    let json = serde_json::json!({
        "program": assembled,
        "entrypoints": [
            {"builtins": info.builtins, "offset": 0, "kind": "Standalone"},
            {"builtins": info.builtins, "offset": header.current_code_offset, "kind": "Bootloader"},
        ],
    });
    TestExecutable::Ready { json, bytecode }
}

fn felt_from_bigint(value: &impl std::fmt::Display) -> Felt {
    let text = value.to_string();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.as_str()),
    };
    let magnitude = Felt::from_dec_str(digits).unwrap_or(Felt::ZERO);
    if negative { -magnitude } else { magnitude }
}

fn program_digest(bytecode: &[Felt]) -> String {
    let mut hasher = blake3::Hasher::new();
    for felt in bytecode {
        hasher.update(&felt.to_bytes_be());
    }
    hasher.finalize().to_hex().to_string()
}

/// The `run_and_prove` and `verify` binaries of a Stwo prover build.
pub(crate) struct ProverTools {
    pub dir: PathBuf,
    pub run_and_prove: PathBuf,
    pub verify: PathBuf,
}

pub(crate) fn default_prover_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".uc/provers/chihuahua/bin"))
}

pub(crate) fn locate_prover(explicit: Option<&Path>) -> Result<ProverTools> {
    let dir = explicit
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("UC_PROVER_DIR").map(PathBuf::from))
        .or_else(default_prover_dir)
        .context("no prover directory: pass --prover-dir or set UC_PROVER_DIR")?;
    let tools = ProverTools {
        run_and_prove: dir.join("run_and_prove"),
        verify: dir.join("verify"),
        dir,
    };
    if !tools.run_and_prove.is_file() || !tools.verify.is_file() {
        bail!(
            "no Stwo prover in {}: expected `run_and_prove` and `verify`. Install Chihuahua with\n  \
             cargo +nightly-2026-01-15 install --locked --git https://github.com/starknet-innovation/proving \
             --rev {CHIHUAHUA_REV} stwo-cairo-dev-utils --bin run_and_prove --bin verify --root ~/.uc/provers/chihuahua",
            tools.dir.display()
        );
    }
    Ok(tools)
}

/// One test's entry in a proof bundle.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct TestProofRecord {
    pub name: String,
    /// `proven`, `unprovable` or `failed`; `--verify-proofs` adds `verified` and `rejected`.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program_blake3: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program_len: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prove_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_rss_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProverRecord {
    pub dir: String,
    pub run_and_prove_blake3: String,
    pub verify_blake3: String,
}

/// `target/proofs/<crate>/manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProofBundle {
    pub schema_version: u32,
    pub package: String,
    pub crate_name: String,
    pub uc_version: String,
    pub cairo_version: String,
    pub prover: ProverRecord,
    pub tests: Vec<TestProofRecord>,
}

pub(crate) fn proof_bundle_dir(workspace_root: &Path, crate_name: &str) -> PathBuf {
    workspace_root.join("target/proofs").join(crate_name)
}

fn file_blake3(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn file_stem_for(test_name: &str) -> String {
    test_name
        .replace("::", "__")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

struct Measured {
    success: bool,
    wall_ms: f64,
    peak_rss_bytes: u64,
}

/// Runs `command` with its output in `log`, and reports its peak resident memory.
fn run_measured(command: &mut Command, log: &Path) -> Result<Measured> {
    let log_file = File::create(log).with_context(|| format!("failed to create {}", log.display()))?;
    command
        .stdin(Stdio::null())
        .stdout(log_file.try_clone()?)
        .stderr(log_file);
    let started = Instant::now();
    let child = command
        .spawn()
        .with_context(|| format!("failed to start {:?}", command.get_program()))?;
    let pid = child.id() as libc::pid_t;
    let mut status: libc::c_int = 0;
    // SAFETY: `rusage` is plain old data, and wait4 fills it for the child we spawned.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    loop {
        // SAFETY: waits on our own child; `status` and `usage` outlive the call.
        let reaped = unsafe { libc::wait4(pid, &mut status, 0, &mut usage) };
        if reaped == pid {
            break;
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err).context("failed to wait for the prover");
        }
    }
    drop(child);
    // macOS reports ru_maxrss in bytes, Linux in KiB.
    let peak = usage.ru_maxrss.max(0) as u64;
    let peak_rss_bytes = if cfg!(target_os = "macos") { peak } else { peak * 1024 };
    Ok(Measured {
        success: libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
        wall_ms: started.elapsed().as_secs_f64() * 1000.0,
        peak_rss_bytes,
    })
}

fn log_tail(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(3)..].join(" | ")
}

fn read_proof(proof_path: &Path) -> Result<serde_json::Value> {
    let text = fs::read_to_string(proof_path)
        .with_context(|| format!("failed to read {}", proof_path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("{} is not a JSON proof", proof_path.display()))
}

/// The security the proof's own FRI configuration gives; verifiers accept whatever it declares.
fn proof_security_bits(proof: &serde_json::Value) -> Result<u64> {
    let fri = proof
        .pointer("/stark_proof/config/fri_config")
        .context("the proof has no FRI configuration")?;
    let field = |name: &str| {
        fri.get(name)
            .and_then(serde_json::Value::as_u64)
            .with_context(|| format!("the proof's FRI configuration has no `{name}`"))
    };
    Ok(field("pow_bits")? + field("log_blowup_factor")? * field("n_queries")?)
}

/// The program a proof proves: the felts of its public program memory.
fn proof_public_program(proof: &serde_json::Value) -> Result<Vec<Felt>> {
    let program = proof
        .pointer("/claim/public_data/public_memory/program")
        .and_then(serde_json::Value::as_array)
        .context("the proof has no public program memory")?;
    program
        .iter()
        .map(|entry| {
            let limbs = entry
                .get(1)
                .and_then(serde_json::Value::as_array)
                .filter(|limbs| limbs.len() == 8)
                .context("malformed public memory entry")?;
            let mut bytes = Vec::with_capacity(32);
            for limb in limbs {
                let limb = limb.as_u64().context("malformed memory limb")?;
                bytes.extend_from_slice(&(limb as u32).to_le_bytes());
            }
            Ok(Felt::from_bytes_le_slice(&bytes))
        })
        .collect()
}

/// Runs the prover's verifier on a proof and checks that the proof is about `bytecode`.
fn check_proof(tools: &ProverTools, proof: &Path, bytecode: &[Felt], log: &Path) -> Result<f64> {
    let measured = run_measured(
        Command::new(&tools.verify)
            .arg("--proof_path")
            .arg(proof)
            .arg("--channel_hash")
            .arg("blake2s_m31"),
        log,
    )?;
    if !measured.success {
        bail!("the verifier rejected the proof: {}", log_tail(log));
    }
    let proof = read_proof(proof)?;
    let bits = proof_security_bits(&proof)?;
    if bits < MIN_SECURITY_BITS {
        bail!("the proof gives {bits} bits of security, under {MIN_SECURITY_BITS}");
    }
    let public_program = proof_public_program(&proof)?;
    if public_program != bytecode {
        bail!(
            "the proof is about a different program ({} public words, the test has {})",
            public_program.len(),
            bytecode.len()
        );
    }
    Ok(measured.wall_ms)
}

fn prove_one(
    tools: &ProverTools,
    dir: &Path,
    name: &str,
    json: &serde_json::Value,
    bytecode: &[Felt],
) -> TestProofRecord {
    let stem = file_stem_for(name);
    let executable = dir.join(format!("{stem}.executable.json"));
    let proof = dir.join(format!("{stem}.proof.json"));
    let mut record = TestProofRecord {
        name: name.to_string(),
        executable: Some(format!("{stem}.executable.json")),
        program_blake3: Some(program_digest(bytecode)),
        program_len: Some(bytecode.len()),
        ..Default::default()
    };
    let attempt = (|| -> Result<()> {
        fs::write(&executable, serde_json::to_vec(json)?)?;
        let prove_log = dir.join(format!("{stem}.prove.log"));
        let measured = run_measured(
            Command::new(&tools.run_and_prove)
                .arg("--program")
                .arg(&executable)
                .arg("--program_type")
                .arg("executable")
                .arg("--proof_path")
                .arg(&proof)
                .arg("--params_json")
                .arg(dir.join(PROVER_PARAMS_FILE)),
            &prove_log,
        )?;
        record.prove_ms = Some(measured.wall_ms);
        record.peak_rss_bytes = Some(measured.peak_rss_bytes);
        if !measured.success {
            bail!("the prover failed: {}", log_tail(&prove_log));
        }
        let verify_log = dir.join(format!("{stem}.verify.log"));
        record.verify_ms = Some(check_proof(tools, &proof, bytecode, &verify_log)?);
        record.proof = Some(format!("{stem}.proof.json"));
        record.proof_bytes = Some(fs::metadata(&proof)?.len());
        Ok(())
    })();
    match attempt {
        Ok(()) => record.status = "proven".to_string(),
        Err(err) => {
            record.status = "failed".to_string();
            record.reason = Some(format!("{err:#}"));
        }
    }
    record
}

/// Proves the given passing tests and writes the bundle under `target/proofs/<crate>/`.
pub(crate) fn prove_passing_tests(
    project: &NativeTestProject,
    package_name: &str,
    crate_name: &str,
    features: &[String],
    workspace_root: &Path,
    tools: &ProverTools,
    candidates: &BTreeMap<String, Option<String>>,
) -> Result<(ProofBundle, PathBuf)> {
    let provable: BTreeSet<String> = candidates
        .iter()
        .filter(|(_, skip)| skip.is_none())
        .map(|(name, _)| name.clone())
        .collect();
    let executables =
        compile_test_executables(project, crate_name, features, workspace_root, &provable)?;
    let dir = proof_bundle_dir(workspace_root, crate_name);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    fs::write(
        dir.join(PROVER_PARAMS_FILE),
        serde_json::to_vec_pretty(&prover_params())?,
    )?;
    let mut tests = Vec::new();
    for (name, skip) in candidates {
        let record = match (skip, executables.get(name)) {
            (Some(reason), _) => unprovable(name, reason),
            (None, Some(TestExecutable::Unprovable(reason))) => unprovable(name, reason),
            (None, Some(TestExecutable::Ready { json, bytecode })) => {
                prove_one(tools, &dir, name, json, bytecode)
            }
            (None, None) => unprovable(name, "was not compiled"),
        };
        tests.push(record);
    }
    let bundle = ProofBundle {
        schema_version: BUNDLE_SCHEMA_VERSION,
        package: package_name.to_string(),
        crate_name: crate_name.to_string(),
        uc_version: env!("CARGO_PKG_VERSION").to_string(),
        cairo_version: native_cairo_lang_compiler_version().to_string(),
        prover: ProverRecord {
            dir: tools.dir.display().to_string(),
            run_and_prove_blake3: file_blake3(&tools.run_and_prove)?,
            verify_blake3: file_blake3(&tools.verify)?,
        },
        tests,
    };
    fs::write(dir.join(BUNDLE_MANIFEST), serde_json::to_vec_pretty(&bundle)?)?;
    Ok((bundle, dir))
}

fn unprovable(name: &str, reason: &str) -> TestProofRecord {
    TestProofRecord {
        name: name.to_string(),
        status: "unprovable".to_string(),
        reason: Some(reason.to_string()),
        ..Default::default()
    }
}

/// Checks every proven test of a bundle: its program, rebuilt from the current sources, must be
/// the one the manifest names and the proof's public memory holds, and the verifier must accept
/// the proof.
pub(crate) fn verify_proof_bundle(
    project: &NativeTestProject,
    crate_name: &str,
    features: &[String],
    workspace_root: &Path,
    tools: &ProverTools,
) -> Result<(ProofBundle, PathBuf)> {
    let dir = proof_bundle_dir(workspace_root, crate_name);
    let manifest_path = dir.join(BUNDLE_MANIFEST);
    let text = fs::read_to_string(&manifest_path).with_context(|| {
        format!(
            "no proof bundle at {}; create one with `uc test --prove`",
            manifest_path.display()
        )
    })?;
    let mut bundle: ProofBundle = serde_json::from_str(&text)
        .with_context(|| format!("{} is not a uc proof bundle", manifest_path.display()))?;
    let proven: BTreeSet<String> = bundle
        .tests
        .iter()
        .filter(|record| record.status == "proven")
        .map(|record| record.name.clone())
        .collect();
    let executables =
        compile_test_executables(project, crate_name, features, workspace_root, &proven)?;
    for record in bundle.tests.iter_mut().filter(|r| r.status == "proven") {
        let outcome = (|| -> Result<f64> {
            let Some(TestExecutable::Ready { bytecode, .. }) = executables.get(&record.name) else {
                bail!("the test no longer compiles to an executable");
            };
            if record.program_blake3.as_deref() != Some(program_digest(bytecode).as_str()) {
                bail!("the test's program changed since it was proven");
            }
            let proof = record.proof.as_deref().context("the manifest names no proof file")?;
            let stem = file_stem_for(&record.name);
            check_proof(tools, &dir.join(proof), bytecode, &dir.join(format!("{stem}.verify.log")))
        })();
        match outcome {
            Ok(verify_ms) => {
                record.status = "verified".to_string();
                record.verify_ms = Some(verify_ms);
            }
            Err(err) => {
                record.status = "rejected".to_string();
                record.reason = Some(format!("{err:#}"));
            }
        }
    }
    Ok((bundle, dir))
}

/// `uc test --prove`: proves the tests that passed in this run.
pub(crate) fn prove_test_results(
    args: &TestArgs,
    manifest_path: &Path,
    workspace_root: &Path,
    package_name: &str,
    crate_name: &str,
    manifest: &TomlValue,
    results: &[SingleTestResultReport],
) -> Result<(ProofBundle, PathBuf)> {
    let tools = locate_prover(args.prover_dir.as_deref())?;
    let candidates: BTreeMap<String, Option<String>> = results
        .iter()
        .filter(|result| result.status == "ok")
        .map(|result| {
            let skip = result
                .should_panic
                .then(|| "expects a panic, and a proof shows that a test returned".to_string());
            (result.name.clone(), skip)
        })
        .collect();
    let project =
        write_native_test_project(manifest_path, workspace_root, package_name, crate_name, manifest)?;
    let features = resolve_manifest_native_features(manifest, &args.features);
    prove_passing_tests(
        &project,
        package_name,
        crate_name,
        &features,
        workspace_root,
        &tools,
        &candidates,
    )
}

/// `uc test --verify-proofs`: checks a package's proof bundle without running its tests.
pub(crate) fn run_verify_proofs(
    args: &TestArgs,
    manifest_path: &Path,
    workspace_root: &Path,
    package_name: &str,
    crate_name: &str,
    manifest: &TomlValue,
) -> Result<()> {
    let started = Instant::now();
    let tools = locate_prover(args.prover_dir.as_deref())?;
    let project =
        write_native_test_project(manifest_path, workspace_root, package_name, crate_name, manifest)?;
    let features = resolve_manifest_native_features(manifest, &args.features);
    let (bundle, dir) = verify_proof_bundle(&project, crate_name, &features, workspace_root, &tools)?;
    let count = |status: &str| bundle.tests.iter().filter(|t| t.status == status).count();
    let (verified, rejected) = (count("verified"), count("rejected"));
    let report = serde_json::json!({
        "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
        "status": if rejected == 0 && verified > 0 { "passed" } else { "failed" },
        "runner": "uc-proof-verifier",
        "package": package_name,
        "manifest_path": manifest_path.display().to_string(),
        "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        "proofs": proof_bundle_report(&bundle, &dir),
    });
    if args.json || args.report_path.is_some() {
        emit_json_value(args.report_path.as_deref(), &report)?;
    } else {
        print_proof_bundle(&bundle, &dir);
    }
    if rejected > 0 {
        bail!("{rejected} proof(s) rejected");
    }
    if verified == 0 {
        bail!("the bundle in {} holds no proofs", dir.display());
    }
    Ok(())
}

pub(crate) fn proof_bundle_report(bundle: &ProofBundle, dir: &Path) -> serde_json::Value {
    let count = |status: &str| bundle.tests.iter().filter(|t| t.status == status).count();
    serde_json::json!({
        "dir": dir.display().to_string(),
        "prover_dir": bundle.prover.dir,
        "proven": count("proven"),
        "verified": count("verified"),
        "rejected": count("rejected"),
        "unprovable": count("unprovable"),
        "failed": count("failed"),
        "tests": bundle.tests,
    })
}

pub(crate) fn print_proof_bundle(bundle: &ProofBundle, dir: &Path) {
    for test in &bundle.tests {
        match (test.status.as_str(), test.reason.as_deref()) {
            ("proven", _) => println!(
                "proof {} ... proven in {:.1} s, {:.2} GiB peak, {} KiB",
                test.name,
                test.prove_ms.unwrap_or_default() / 1000.0,
                test.peak_rss_bytes.unwrap_or_default() as f64 / f64::from(1u32 << 30),
                test.proof_bytes.unwrap_or_default() / 1024
            ),
            (status, Some(reason)) => println!("proof {} ... {status}: {reason}", test.name),
            (status, None) => println!("proof {} ... {status}", test.name),
        }
    }
    println!("proofs: {}", dir.display());
}
