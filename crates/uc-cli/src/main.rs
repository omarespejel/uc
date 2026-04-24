use anyhow::{bail, Context, Result};
use blake3::Hasher;
#[cfg(feature = "native-compile")]
use cairo_lang_compiler::db::RootDatabase;
#[cfg(feature = "native-compile")]
use cairo_lang_compiler::project::setup_project;
#[cfg(feature = "native-compile")]
use cairo_lang_compiler::{compile_prepared_db_program, CompilerConfig};
#[cfg(feature = "native-compile")]
use cairo_lang_defs::db::DefsGroup;
#[cfg(feature = "native-compile")]
use cairo_lang_defs::ids::ModuleId;
#[cfg(feature = "native-compile")]
use cairo_lang_defs::ids::NamedLanguageElementId;
#[cfg(feature = "native-compile")]
use cairo_lang_defs::ids::TopLevelLanguageElementId;
#[cfg(feature = "native-compile")]
use cairo_lang_filesystem::db::{
    ensure_keyed_file_override_slots, files_group_input, init_dev_corelib, set_crate_configs_input,
    set_file_override_content_keyed, CrateConfigurationInput, FilesGroup,
};
#[cfg(feature = "native-compile")]
use cairo_lang_filesystem::detect::detect_corelib;
#[cfg(feature = "native-compile")]
use cairo_lang_filesystem::ids::{BlobLongId, CrateId, CrateInput, CrateLongId, Directory};
#[cfg(feature = "native-compile")]
use cairo_lang_filesystem::ids::{FileId, FileLongId};
#[cfg(feature = "native-compile")]
use cairo_lang_lowering::cache::generate_crate_cache;
#[cfg(feature = "native-compile")]
use cairo_lang_lowering::optimizations::config::Optimizations;
#[cfg(feature = "native-compile")]
use cairo_lang_lowering::utils::InliningStrategy;
#[cfg(feature = "native-compile")]
use cairo_lang_starknet::compile::compile_prepared_db as compile_starknet_prepared_db;
#[cfg(feature = "native-compile")]
use cairo_lang_starknet::contract::{find_contracts, module_contract, ContractDeclaration};
#[cfg(feature = "native-compile")]
use cairo_lang_starknet::starknet_plugin_suite;
#[cfg(feature = "native-compile")]
use cairo_lang_starknet_classes::casm_contract_class::CasmContractClass;
#[cfg(feature = "native-compile")]
use cairo_lang_starknet_classes::contract_class::ContractClass;
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
#[cfg(feature = "native-compile")]
use notify::event::{ModifyKind as NotifyModifyKind, RenameMode as NotifyRenameMode};
#[cfg(feature = "native-compile")]
use notify::{EventKind as NotifyEventKind, RecommendedWatcher, RecursiveMode, Watcher};
#[cfg(feature = "native-compile")]
use scarb_stable_hash::short_hash;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
#[cfg(target_os = "macos")]
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use toml::Value as TomlValue;
use tracing_subscriber::EnvFilter;
use uc_core::artifacts::{
    collect_artifact_digests, compare_artifact_sets, ArtifactDigest, ArtifactMismatch,
};
use uc_core::compare::{compare_diagnostics, extract_diagnostic_lines, DiagnosticsComparison};
use uc_core::session::SessionInput;
use walkdir::WalkDir;

#[cfg(feature = "dev-benchmark-command")]
mod benchmark_cmd;
mod cache;
mod commands;
mod daemon;
mod fingerprint;
use cache::*;
use commands::{run_build, run_compare_build, run_metadata, run_migrate};
#[allow(unused_imports)]
use daemon::*;
use fingerprint::*;

const BUILD_CACHE_SCHEMA_VERSION: u32 = 1;
const MIN_HASH_LEN: usize = 2;
const SESSION_KEY_LEN: usize = 64;
const MAX_FINGERPRINT_FILES: usize = 50_000;
const MAX_FINGERPRINT_DEPTH: usize = 32;
const MAX_FINGERPRINT_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FINGERPRINT_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const FINGERPRINT_TIMEOUT_MS: u64 = 30_000;
const FINGERPRINT_MTIME_RECHECK_WINDOW_MS: u64 = 2_000;
const MAX_CACHEABLE_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_LOCKFILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CACHE_ENTRY_BYTES: u64 = 10 * 1024 * 1024;
const MAX_FINGERPRINT_INDEX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ARTIFACT_INDEX_BYTES: u64 = 32 * 1024 * 1024;
const DEFAULT_MAX_RESTORE_EXISTING_HASH_BYTES: u64 = 1024 * 1024;
const MAX_CAPTURE_STDOUT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CAPTURE_STDERR_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_MAX_CACHE_BYTES: u64 = 10 * 1024 * 1024 * 1024;
const DEFAULT_CACHE_BUDGET_MIN_INTERVAL_MS: u64 = 5 * 60 * 1000;
const FINGERPRINT_INDEX_SCHEMA_VERSION: u32 = 2;
const ARTIFACT_INDEX_SCHEMA_VERSION: u32 = 1;
const DEFAULT_DIAGNOSTICS_SIMILARITY_THRESHOLD: f64 = 99.5;
// Match Scarb's default package edition when manifests omit `[package].edition`.
// Keep this aligned to Scarb defaults for output parity in native mode.
const DEFAULT_CAIRO_EDITION: &str = "2023_01";
const DAEMON_REQUEST_SIZE_LIMIT_BYTES: usize = 1024 * 1024;
const DAEMON_RESPONSE_SIZE_OVERHEAD_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_DAEMON_RESPONSE_SIZE_LIMIT_BYTES: usize = (MAX_CAPTURE_STDOUT_BYTES as usize)
    + (MAX_CAPTURE_STDERR_BYTES as usize)
    + DAEMON_RESPONSE_SIZE_OVERHEAD_BYTES;
const DAEMON_RATE_WINDOW_SECONDS: u64 = 1;
const DAEMON_MAX_REQUESTS_PER_WINDOW: usize = 32;
const DAEMON_LOG_ROTATE_BYTES: u64 = 10 * 1024 * 1024;
const DAEMON_UNHEALTHY_RECOVERY_SECONDS: u64 = 5;
const DEFAULT_DAEMON_CLIENT_READ_TIMEOUT_SECS: u64 = 120;
const DEFAULT_DAEMON_BUILD_READ_TIMEOUT_SECS: u64 = 0;
const DEFAULT_DAEMON_CLIENT_WRITE_TIMEOUT_SECS: u64 = 30;
const DEFAULT_DAEMON_MAX_CONNECTION_HANDLERS: usize = 256;
const ASYNC_PERSIST_ERROR_QUEUE_LIMIT: usize = 32;
const ASYNC_PERSIST_QUEUE_LIMIT: usize = 128;
const DEFAULT_ASYNC_PERSIST_ERROR_LOG_MAX_BYTES: u64 = 4 * 1024 * 1024;
const CACHE_LOCK_STALE_AFTER_SECONDS: u64 = 300;
const DEFAULT_CACHE_BUDGET_ENFORCE_EVERY: u64 = 8;
const DEFAULT_MIN_SCARB_VERSION: &str = "2.14.0";
const DEFAULT_SESSION_INPUT_CACHE_MAX_ENTRIES: usize = 1024;
const DEFAULT_FINGERPRINT_INDEX_CACHE_MAX_ENTRIES: usize = 512;
const DEFAULT_BUILD_ENTRY_CACHE_MAX_ENTRIES: usize = 1024;
const DEFAULT_ARTIFACT_INDEX_CACHE_MAX_ENTRIES: usize = 512;
const DEFAULT_DAEMON_BUILD_PLAN_CACHE_MAX_ENTRIES: usize = 512;
const DEFAULT_DAEMON_LOCK_HASH_CACHE_MAX_ENTRIES: usize = 512;
const DEFAULT_METADATA_RESULT_CACHE_MAX_ENTRIES: usize = 512;
const DEFAULT_METADATA_RESULT_CACHE_MAX_BYTES: u64 = 256 * 1024 * 1024;
const METADATA_RESULT_CACHE_SCHEMA_VERSION: u32 = 1;
const MAX_METADATA_RESULT_CACHE_ENTRY_BYTES: u64 = 32 * 1024 * 1024;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_COMPILE_SESSION_CACHE_MAX_ENTRIES: usize = 16;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_COMPILE_CONTEXT_CACHE_MAX_ENTRIES: usize = 256;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_COMPILE_SESSION_CACHE_MAX_BYTES: u64 = 1024 * 1024 * 1024;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_COMPILE_CONTEXT_CACHE_MAX_BYTES: u64 = 128 * 1024 * 1024;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_COMPILE_SESSION_CACHE_TTL_MS: u64 = 30 * 60 * 1000;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_COMPILE_CONTEXT_CACHE_TTL_MS: u64 = 30 * 60 * 1000;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_INCREMENTAL_MAX_CHANGED_FILES: usize = 256;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_IMPACTED_SUBSET_ENABLED: bool = true;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_CAPTURE_STATEMENT_LOCATIONS: bool = true;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_CAPTURE_STATEMENT_LOCATIONS_ON_COLD: bool = false;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_PROGRESS_ENABLED: bool = false;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_PROGRESS_HEARTBEAT_SECS: u64 = 5;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_PROGRESS_COMPILE_BATCH_SIZE: usize = 0;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_DEPENDENCY_METADATA_ENABLED: bool = false;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_EAGER_KEYED_SLOT_PRIME: bool = true;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_COMPILE_SESSION_MEMORY_MULTIPLIER: u64 = 64;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_COMPILE_SESSION_MEMORY_BASE_OVERHEAD_BYTES: u64 = 32 * 1024 * 1024;
#[cfg(feature = "native-compile")]
const NATIVE_COMPILE_SESSION_IMAGE_SCHEMA_VERSION: u32 = 2;
#[cfg(feature = "native-compile")]
const MAX_NATIVE_COMPILE_SESSION_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
#[cfg(feature = "native-compile")]
const NATIVE_BUILDINFO_SCHEMA_VERSION: u32 = 2;
#[cfg(feature = "native-compile")]
const MAX_NATIVE_BUILDINFO_BYTES: u64 = 16 * 1024 * 1024;
#[cfg(feature = "native-compile")]
const NATIVE_CRATE_CACHE_ENTRY_SCHEMA_VERSION: u32 = 1;
#[cfg(feature = "native-compile")]
const MAX_NATIVE_CRATE_CACHE_ENTRY_BYTES: u64 = 64 * 1024;
#[cfg(feature = "native-compile")]
const MAX_NATIVE_CRATE_CACHE_BLOB_BYTES: u64 = 128 * 1024 * 1024;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_CRATE_CACHE_MAX_BYTES: u64 = 512 * 1024 * 1024;
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_CRATE_CACHE_ENABLED: bool = true;
#[cfg(feature = "native-compile")]
const NATIVE_SOURCE_JOURNAL_SCHEMA_VERSION: u32 = 1;
#[cfg(feature = "native-compile")]
const MAX_NATIVE_SOURCE_JOURNAL_BYTES: u64 = 1024 * 1024;
const DEFAULT_DAEMON_SHARED_CACHE_ENABLED: bool = true;
const DEFAULT_DAEMON_SHARED_CACHE_MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
// Default-off for Scarb's artifact fingerprint check in uc's Scarb-backed path.
// This removes an extra post-build verification step on the hot path; users who
// require Scarb's artifact fingerprint verification can opt back in with
// `UC_DISABLE_SCARB_ARTIFACTS_FINGERPRINT=0`.
const DEFAULT_UC_DISABLE_SCARB_ARTIFACTS_FINGERPRINT: bool = true;
const DEFAULT_UC_NATIVE_BUILD_MODE: &str = "auto";
const DEFAULT_UC_NATIVE_DISALLOW_SCARB_FALLBACK: bool = false;
const TOOLCHAIN_CHECK_CACHE_SCHEMA_VERSION: u32 = 1;
const MAX_TOOLCHAIN_CHECK_CACHE_BYTES: u64 = 64 * 1024;
/// Default Starknet CASM bytecode limit used by native compile.
/// Mirrors the cairo-lang/scarb default used by contract class validation
/// (81_290 as of cairo-lang 2.16.0 / Scarb 2.14.x) and can be overridden with
/// `UC_NATIVE_MAX_CASM_BYTECODE_SIZE` for network-specific tuning.
/// Reference:
/// https://docs.starknet.io/architecture-and-concepts/smart-contracts/contract-classes/
#[cfg(feature = "native-compile")]
const DEFAULT_NATIVE_MAX_CASM_BYTECODE_SIZE: usize = 81_290;
const DEFAULT_SCARB_TOOLCHAIN_CACHE_TTL_MS: u64 = 5 * 60 * 1000;
const DAEMON_PROTOCOL_VERSION: &str = env!("CARGO_PKG_VERSION");
const CACHEABLE_ARTIFACT_SUFFIXES: [&str; 7] = [
    ".sierra.json",
    ".sierra",
    ".casm",
    ".contract_class.json",
    ".compiled_contract_class.json",
    ".starknet_artifacts.json",
    ".executable.json",
];

#[derive(Parser, Debug)]
#[command(name = "uc")]
#[command(about = "uc: Cairo package manager and build/prove engine", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Daemon(DaemonArgs),
    #[cfg(feature = "dev-benchmark-command")]
    #[command(hide = true)]
    Benchmark(benchmark_cmd::BenchmarkArgs),
    Cache(CacheArgs),
    SessionKey(SessionKeyArgs),
    Build(BuildArgs),
    Metadata(MetadataArgs),
    CompareBuild(CompareBuildArgs),
    Migrate(MigrateArgs),
}

#[derive(Args, Debug)]
struct DaemonArgs {
    #[command(subcommand)]
    command: DaemonCommand,
}

#[derive(Subcommand, Debug)]
enum DaemonCommand {
    Start(DaemonSocketArgs),
    Status(DaemonSocketArgs),
    Health(DaemonSocketArgs),
    Stop(DaemonSocketArgs),
    #[command(hide = true)]
    Serve(DaemonSocketArgs),
}

#[derive(Args, Debug)]
struct CacheArgs {
    #[command(subcommand)]
    command: CacheCommand,
}

#[derive(Subcommand, Debug)]
enum CacheCommand {
    Clean(CacheCleanArgs),
}

#[derive(Args, Debug, Clone)]
struct CacheCleanArgs {
    #[arg(long)]
    manifest_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
struct DaemonSocketArgs {
    #[arg(long)]
    socket_path: Option<PathBuf>,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum EngineArg {
    Scarb,
    Uc,
}

impl EngineArg {
    fn as_str(self) -> &'static str {
        match self {
            EngineArg::Scarb => "scarb",
            EngineArg::Uc => "uc",
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum DaemonModeArg {
    Off,
    Auto,
    Require,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum BuildCompileBackend {
    Scarb,
    Native,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum DaemonBuildBackend {
    #[default]
    Scarb,
    Native,
}

impl DaemonBuildBackend {
    fn from_compile_backend(backend: BuildCompileBackend) -> Self {
        match backend {
            BuildCompileBackend::Scarb => Self::Scarb,
            BuildCompileBackend::Native => Self::Native,
        }
    }

    fn into_compile_backend(self) -> BuildCompileBackend {
        match self {
            Self::Scarb => BuildCompileBackend::Scarb,
            Self::Native => BuildCompileBackend::Native,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum NativeBuildMode {
    Off,
    Auto,
    Require,
}

#[derive(Debug)]
struct NativeFallbackEligibleTag;

impl std::fmt::Display for NativeFallbackEligibleTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native fallback to scarb is allowed")
    }
}

impl std::error::Error for NativeFallbackEligibleTag {}

fn native_fallback_eligible_error(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(NativeFallbackEligibleTag).context(message.into())
}

#[cfg(feature = "native-compile")]
fn mark_native_fallback_eligible(err: anyhow::Error) -> anyhow::Error {
    err.context(NativeFallbackEligibleTag)
}

fn native_error_allows_scarb_fallback(err: &anyhow::Error) -> bool {
    err.downcast_ref::<NativeFallbackEligibleTag>().is_some()
}

#[cfg(feature = "native-compile")]
static NATIVE_DAEMON_BACKEND_POISONED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "native-compile")]
fn native_daemon_backend_is_poisoned() -> bool {
    NATIVE_DAEMON_BACKEND_POISONED.load(Ordering::Acquire)
}

#[cfg(feature = "native-compile")]
fn mark_native_daemon_backend_poisoned() {
    NATIVE_DAEMON_BACKEND_POISONED.store(true, Ordering::Release);
}

#[cfg(feature = "native-compile")]
fn ensure_native_daemon_backend_available() -> Result<()> {
    if native_daemon_backend_is_poisoned() {
        return Err(native_fallback_eligible_error(
            "native daemon backend is disabled after a previous panic; restart `uc daemon` to re-enable native requests",
        ));
    }
    Ok(())
}

#[cfg(all(test, feature = "native-compile"))]
fn set_native_daemon_backend_poisoned_for_test(value: bool) {
    NATIVE_DAEMON_BACKEND_POISONED.store(value, Ordering::Release);
}

#[derive(Args, Debug)]
struct SessionKeyArgs {
    #[arg(long)]
    compiler_version: String,

    #[arg(long)]
    profile: String,

    #[arg(long, default_value_t = false)]
    offline: bool,

    #[arg(long)]
    package: Option<String>,

    #[arg(long, value_delimiter = ',')]
    features: Vec<String>,

    #[arg(long = "cfg", value_delimiter = ',')]
    cfg_set: Vec<String>,

    #[arg(long = "manifest-content-hash", alias = "plugin-signature")]
    manifest_content_hash: String,

    #[arg(long)]
    target_family: String,

    #[arg(long)]
    cairo_edition: Option<String>,

    #[arg(long)]
    cairo_lang_version: Option<String>,

    #[arg(long, default_value = "")]
    build_env_fingerprint: String,
}

#[derive(Args, Debug, Clone)]
struct BuildCommonArgs {
    #[arg(long)]
    manifest_path: Option<PathBuf>,

    #[arg(long)]
    package: Option<String>,

    #[arg(long)]
    workspace: bool,

    #[arg(long, value_delimiter = ',')]
    features: Vec<String>,

    #[arg(long)]
    offline: bool,

    #[arg(long, conflicts_with = "profile")]
    release: bool,

    #[arg(long, conflicts_with = "release")]
    profile: Option<String>,
}

#[derive(Args, Debug)]
struct BuildArgs {
    #[command(flatten)]
    common: BuildCommonArgs,

    #[arg(long, value_enum, default_value_t = EngineArg::Uc)]
    engine: EngineArg,

    #[arg(long, value_enum, default_value_t = DaemonModeArg::Auto)]
    daemon_mode: DaemonModeArg,

    #[arg(long)]
    report_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
struct MetadataArgs {
    #[arg(long)]
    manifest_path: Option<PathBuf>,

    #[arg(long, default_value_t = 1, value_parser = parse_metadata_format_version)]
    format_version: u32,

    #[arg(long, value_enum, default_value_t = DaemonModeArg::Off)]
    daemon_mode: DaemonModeArg,

    #[arg(long)]
    offline: bool,

    #[arg(long)]
    global_cache_dir: Option<PathBuf>,

    #[arg(long)]
    report_path: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct CompareBuildArgs {
    #[command(flatten)]
    common: BuildCommonArgs,

    #[arg(long)]
    output_path: Option<PathBuf>,

    #[arg(long, action = ArgAction::Set, default_value_t = true)]
    clean_before_each: bool,

    #[arg(long, value_parser = parse_diagnostics_threshold)]
    diagnostics_threshold: Option<f64>,
}

#[derive(Args, Debug)]
struct MigrateArgs {
    #[arg(long)]
    manifest_path: Option<PathBuf>,

    #[arg(long)]
    report_path: Option<PathBuf>,

    #[arg(long)]
    emit_uc_toml: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CommandRun {
    command: Vec<String>,
    exit_code: i32,
    elapsed_ms: f64,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct BuildPhaseTelemetry {
    fingerprint_ms: f64,
    cache_lookup_ms: f64,
    cache_restore_ms: f64,
    compile_ms: f64,
    cache_persist_ms: f64,
    cache_persist_async: bool,
    cache_persist_scheduled: bool,
    #[serde(default)]
    native_context_ms: f64,
    #[serde(default)]
    native_target_dir_ms: f64,
    #[serde(default)]
    native_session_prepare_ms: f64,
    #[serde(default)]
    native_frontend_compile_ms: f64,
    #[serde(default)]
    native_casm_ms: f64,
    #[serde(default)]
    native_artifact_write_ms: f64,
    #[serde(default)]
    native_changed_files: u64,
    #[serde(default)]
    native_removed_files: u64,
    #[serde(default)]
    native_total_contracts: u64,
    #[serde(default)]
    native_compiled_contracts: u64,
    #[serde(default)]
    native_impacted_subset_used: bool,
    #[serde(default)]
    native_journal_fallback_full_scan: bool,
}

#[derive(Debug, Clone, Default)]
struct NativeBuildPhaseTelemetry {
    context_ms: f64,
    target_dir_ms: f64,
    session_prepare_ms: f64,
    frontend_compile_ms: f64,
    casm_ms: f64,
    artifact_write_ms: f64,
    changed_files: u64,
    removed_files: u64,
    total_contracts: u64,
    compiled_contracts: u64,
    impacted_subset_used: bool,
    journal_fallback_full_scan: bool,
}

#[derive(Copy, Clone)]
struct BuildRunOptions {
    capture_output: bool,
    inherit_output_when_uncaptured: bool,
    async_cache_persist: bool,
    use_daemon_shared_cache: bool,
}

#[derive(Copy, Clone)]
struct BuildCacheRunContext<'a> {
    manifest_path: &'a Path,
    workspace_root: &'a Path,
    profile: &'a str,
    session_key: &'a str,
    compiler_version: &'a str,
    compile_backend: BuildCompileBackend,
    options: BuildRunOptions,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg(feature = "native-compile")]
struct NativeCompileContext {
    package_name: String,
    crate_name: String,
    main_source_root: PathBuf,
    workspace_mode_supported: bool,
    cairo_project_dir: PathBuf,
    corelib_src: PathBuf,
    starknet_target: NativeStarknetTargetProps,
    manifest_content_hash: String,
    external_non_starknet_dependencies: Vec<String>,
    path_dependency_roots: Vec<NativePathDependencyRoot>,
    crate_dependency_configs: Vec<NativeCrateDependencyConfig>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[cfg(feature = "native-compile")]
struct NativeStarknetTargetProps {
    sierra: bool,
    casm: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg(feature = "native-compile")]
struct NativePathDependencyRoot {
    crate_name: String,
    source_root: PathBuf,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg(feature = "native-compile")]
struct NativeCrateDependencyConfig {
    crate_name: String,
    cairo_edition: Option<String>,
    dependencies: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg(feature = "native-compile")]
struct NativeDependencySurface {
    external_non_starknet_dependencies: Vec<String>,
    path_dependency_roots: Vec<NativePathDependencyRoot>,
    crate_dependency_configs: Vec<NativeCrateDependencyConfig>,
}

#[derive(Debug, Deserialize)]
#[cfg(feature = "native-compile")]
struct NativeScarbMetadataDocument {
    #[serde(default)]
    packages: Vec<NativeScarbMetadataPackage>,
    #[serde(default)]
    compilation_units: Vec<NativeScarbMetadataCompilationUnit>,
}

#[derive(Debug, Deserialize)]
#[cfg(feature = "native-compile")]
struct NativeScarbMetadataPackage {
    id: String,
    manifest_path: String,
    #[serde(default)]
    edition: Option<String>,
}

#[derive(Debug, Deserialize)]
#[cfg(feature = "native-compile")]
struct NativeScarbMetadataCompilationUnit {
    #[serde(default)]
    package: String,
    #[serde(default)]
    target: NativeScarbMetadataTarget,
    #[serde(default)]
    components_data: Vec<NativeScarbMetadataComponentData>,
}

#[derive(Debug, Default, Deserialize)]
#[cfg(feature = "native-compile")]
struct NativeScarbMetadataTarget {
    #[serde(default)]
    kind: String,
}

#[derive(Debug, Deserialize)]
#[cfg(feature = "native-compile")]
struct NativeScarbMetadataComponentData {
    id: String,
    name: String,
    source_path: String,
    #[serde(default)]
    dependencies: Vec<NativeScarbMetadataDependencyRef>,
}

#[derive(Debug, Deserialize)]
#[cfg(feature = "native-compile")]
struct NativeScarbMetadataDependencyRef {
    id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg(feature = "native-compile")]
struct NativeCompileSessionSignature {
    manifest_path: PathBuf,
    manifest_content_hash: String,
    context: NativeCompileContext,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg(feature = "native-compile")]
struct StarknetArtifactsManifest {
    version: u32,
    contracts: Vec<StarknetArtifactEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg(feature = "native-compile")]
struct StarknetArtifactEntry {
    id: String,
    package_name: String,
    contract_name: String,
    module_path: String,
    artifacts: StarknetArtifactFiles,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg(feature = "native-compile")]
struct StarknetArtifactFiles {
    sierra: String,
    casm: Option<String>,
}

#[derive(Debug, Serialize)]
struct BuildReport {
    generated_at_epoch_ms: u128,
    engine: String,
    daemon_used: bool,
    manifest_path: String,
    workspace_root: String,
    profile: String,
    session_key: String,
    command: Vec<String>,
    exit_code: i32,
    elapsed_ms: f64,
    cache_hit: bool,
    fingerprint: String,
    artifact_count: usize,
    phase_telemetry: Option<BuildPhaseTelemetry>,
}

#[derive(Debug, Serialize)]
struct MetadataReport {
    generated_at_epoch_ms: u128,
    manifest_path: String,
    command: Vec<String>,
    exit_code: i32,
    elapsed_ms: f64,
}

#[derive(Debug, Serialize)]
struct CompareRunSnapshot {
    label: String,
    command: Vec<String>,
    exit_code: i32,
    elapsed_ms: f64,
    artifact_count: usize,
    diagnostics: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CompareBuildReport {
    generated_at_epoch_ms: u128,
    manifest_path: String,
    workspace_root: String,
    clean_before_each: bool,
    diagnostics_threshold: f64,
    baseline: CompareRunSnapshot,
    candidate: CompareRunSnapshot,
    diagnostics: DiagnosticsComparison,
    artifact_mismatch_count: usize,
    artifact_mismatches: Vec<ArtifactMismatch>,
    passed: bool,
}

#[derive(Debug, Serialize)]
struct MigrationReport {
    generated_at_epoch_ms: u128,
    manifest_path: String,
    workspace_root: String,
    package_name: Option<String>,
    package_version: Option<String>,
    edition: Option<String>,
    dependency_count: usize,
    dev_dependency_count: usize,
    unknown_sections: Vec<String>,
    warnings: Vec<String>,
    suggested_next_steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedArtifact {
    relative_path: String,
    blake3_hex: String,
    size_bytes: u64,
    object_rel_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BuildCacheEntry {
    schema_version: u32,
    fingerprint: String,
    profile: String,
    artifacts: Vec<CachedArtifact>,
}

#[derive(Clone)]
struct SessionInputCacheEntry {
    manifest_size_bytes: u64,
    manifest_modified_unix_ms: u64,
    input: SessionInput,
    last_access_epoch_ms: u64,
}

#[derive(Clone)]
struct FingerprintIndexCacheEntry {
    index: FingerprintIndex,
    last_access_epoch_ms: u64,
}

#[derive(Clone)]
struct ArtifactIndexCacheEntry {
    index: ArtifactIndex,
    last_access_epoch_ms: u64,
}

#[derive(Clone)]
struct DaemonBuildPlan {
    manifest_path: PathBuf,
    workspace_root: PathBuf,
    profile: String,
    session_key: String,
    strict_invalidation_key: String,
}

#[derive(Clone)]
struct DaemonBuildPlanCacheEntry {
    manifest_size_bytes: u64,
    manifest_modified_unix_ms: u64,
    lock_size_bytes: Option<u64>,
    lock_modified_unix_ms: Option<u64>,
    lock_hash: String,
    plan: DaemonBuildPlan,
    last_access_epoch_ms: u64,
}

#[derive(Clone)]
struct LockfileHashCacheEntry {
    size_bytes: u64,
    modified_unix_ms: u64,
    change_unix_ms: Option<u64>,
    lock_hash: String,
    last_access_epoch_ms: u64,
}

#[derive(Clone)]
struct MetadataResultCacheEntry {
    manifest_size_bytes: u64,
    manifest_modified_unix_ms: u64,
    lock_hash: String,
    workspace_manifests_hash: String,
    run: CommandRun,
    last_access_epoch_ms: u64,
    estimated_bytes: u64,
}

#[derive(Clone)]
#[cfg(feature = "native-compile")]
struct NativeCompileSessionCacheEntry {
    session: Arc<Mutex<NativeCompileSessionState>>,
    last_access_epoch_ms: u64,
    estimated_bytes: u64,
}

#[derive(Clone)]
#[cfg(feature = "native-compile")]
struct NativeCompileContextCacheEntry {
    manifest_size_bytes: u64,
    manifest_modified_unix_ms: u64,
    manifest_change_unix_ms: Option<u64>,
    workspace_manifest_size_bytes: Option<u64>,
    workspace_manifest_modified_unix_ms: Option<u64>,
    workspace_manifest_change_unix_ms: Option<u64>,
    context: NativeCompileContext,
    last_access_epoch_ms: u64,
    estimated_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg(feature = "native-compile")]
struct NativeTrackedFileState {
    size_bytes: u64,
    modified_unix_ms: u64,
}

#[cfg(feature = "native-compile")]
struct NativeCompileSessionState {
    signature: NativeCompileSessionSignature,
    db: RootDatabase,
    main_crate_inputs: Vec<CrateInput>,
    tracked_sources: BTreeMap<String, NativeTrackedFileState>,
    tracked_source_bytes: u64,
    tracked_sources_content_hash: String,
    journal_cursor_applied: u64,
    source_root_modified_unix_ms: u64,
    contract_source_dependencies: BTreeMap<String, BTreeSet<String>>,
    contract_output_plans: Vec<NativeContractOutputPlan>,
}

#[cfg(feature = "native-compile")]
struct NativeCompileSessionSnapshot {
    db: RootDatabase,
    main_crate_inputs: Vec<CrateInput>,
    changed_files: Vec<String>,
    removed_files: Vec<String>,
    journal_fallback_full_scan: bool,
    contract_source_dependencies: BTreeMap<String, BTreeSet<String>>,
    contract_output_plans: Vec<NativeContractOutputPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg(feature = "native-compile")]
struct NativeCompileSessionImageFile {
    schema_version: u32,
    signature_hash: String,
    source_root_modified_unix_ms: u64,
    tracked_sources: BTreeMap<String, NativeTrackedFileState>,
    tracked_source_bytes: u64,
    #[serde(default)]
    tracked_sources_content_hash: String,
    contract_source_dependencies: BTreeMap<String, BTreeSet<String>>,
    contract_output_plans: Vec<NativeContractOutputPlan>,
    #[serde(default)]
    journal_cursor_applied: u64,
    generated_at_epoch_ms: u64,
}

#[derive(Debug, Clone)]
#[cfg(feature = "native-compile")]
struct NativeCompileSessionImageSnapshot {
    signature_hash: String,
    source_root_modified_unix_ms: u64,
    tracked_sources: BTreeMap<String, NativeTrackedFileState>,
    tracked_source_bytes: u64,
    tracked_sources_content_hash: String,
    contract_source_dependencies: BTreeMap<String, BTreeSet<String>>,
    contract_output_plans: Vec<NativeContractOutputPlan>,
    journal_cursor_applied: u64,
}

#[derive(Debug, Clone, Default)]
#[cfg(feature = "native-compile")]
struct NativeSourceChangeJournal {
    changed_files: BTreeSet<String>,
    removed_files: BTreeSet<String>,
    overflowed: bool,
    cursor: u64,
    applied_cursor: u64,
}

#[cfg(feature = "native-compile")]
struct NativeSourceChangeWatcher {
    _watcher: Option<RecommendedWatcher>,
    journal: Arc<Mutex<NativeSourceChangeJournal>>,
    watched_roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg(feature = "native-compile")]
struct NativeBuildInfoFile {
    schema_version: u32,
    signature_hash: String,
    source_root_modified_unix_ms: u64,
    tracked_sources: BTreeMap<String, NativeTrackedFileState>,
    tracked_source_bytes: u64,
    tracked_sources_signature: String,
    #[serde(default)]
    tracked_sources_content_hash: String,
    #[serde(default)]
    contract_source_dependencies: BTreeMap<String, BTreeSet<String>>,
    #[serde(default)]
    contract_output_plans: Vec<NativeContractOutputPlan>,
    #[serde(default)]
    journal_cursor_applied: u64,
    generated_at_epoch_ms: u64,
}

#[derive(Debug, Clone)]
#[cfg(feature = "native-compile")]
struct NativeBuildInfoSnapshot {
    tracked_sources: BTreeMap<String, NativeTrackedFileState>,
    tracked_source_bytes: u64,
    tracked_sources_content_hash: String,
    contract_source_dependencies: BTreeMap<String, BTreeSet<String>>,
    contract_output_plans: Vec<NativeContractOutputPlan>,
    journal_cursor_applied: u64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg(feature = "native-compile")]
struct NativeCrateCacheDescriptor {
    cache_key: String,
    label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg(feature = "native-compile")]
struct NativeCrateCacheEntryFile {
    schema_version: u32,
    signature_hash: String,
    crate_cache_key: String,
    blob_hash: String,
    blob_size: u64,
    generated_at_epoch_ms: u64,
}

#[derive(Debug, Clone, Default)]
#[cfg(feature = "native-compile")]
struct NativeCrateCacheRestoreStats {
    restored: usize,
    missing: usize,
    rejected: usize,
    skipped: usize,
}

#[derive(Debug, Clone, Default)]
#[cfg(feature = "native-compile")]
struct NativeCrateCachePersistStats {
    saved: usize,
    skipped: usize,
    failed: usize,
    bytes_written: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg(feature = "native-compile")]
struct NativeSourceJournalFile {
    schema_version: u32,
    #[serde(default)]
    changed_files: Vec<String>,
    #[serde(default)]
    removed_files: Vec<String>,
    #[serde(default)]
    overflowed: bool,
    #[serde(default)]
    cursor: u64,
    #[serde(default)]
    applied_cursor: u64,
    #[serde(default)]
    updated_at_epoch_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolchainCheckCacheEntry {
    schema_version: u32,
    checked_epoch_ms: u64,
    version_line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MetadataResultCacheFile {
    schema_version: u32,
    manifest_size_bytes: u64,
    manifest_modified_unix_ms: u64,
    lock_hash: String,
    #[serde(default)]
    workspace_manifests_hash: String,
    run: CommandRun,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonStatusPayload {
    pid: u32,
    started_at_epoch_ms: u64,
    socket_path: String,
    protocol_version: String,
    healthy: bool,
    total_requests: u64,
    failed_requests: u64,
    rate_limited_requests: u64,
    last_error: Option<String>,
    #[serde(default)]
    native_compile_session_cache_entries: u64,
    #[serde(default)]
    native_compile_session_cache_estimated_bytes: u64,
    #[serde(default)]
    native_compile_context_cache_entries: u64,
    #[serde(default)]
    native_compile_context_cache_estimated_bytes: u64,
    #[serde(default)]
    native_compile_session_build_locks: u64,
    #[serde(default)]
    metadata_result_cache_entries: u64,
    #[serde(default)]
    metadata_result_cache_estimated_bytes: u64,
    #[serde(default)]
    native_refresh_none_count: u64,
    #[serde(default)]
    native_refresh_incremental_count: u64,
    #[serde(default)]
    native_refresh_full_rebuild_count: u64,
    #[serde(default)]
    native_refresh_changed_files_total: u64,
    #[serde(default)]
    native_refresh_removed_files_total: u64,
    #[serde(default)]
    native_fallback_preflight_ineligible_count: u64,
    #[serde(default)]
    native_fallback_local_native_error_count: u64,
    #[serde(default)]
    native_fallback_daemon_native_error_count: u64,
    #[serde(default)]
    native_fallback_daemon_backend_downgrade_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonBuildRequest {
    #[serde(default)]
    protocol_version: String,
    manifest_path: String,
    package: Option<String>,
    workspace: bool,
    features: Vec<String>,
    offline: bool,
    release: bool,
    profile: Option<String>,
    #[serde(default)]
    async_cache_persist: bool,
    #[serde(default = "default_daemon_capture_output")]
    capture_output: bool,
    #[serde(default)]
    compile_backend: DaemonBuildBackend,
    #[serde(default)]
    native_fallback_to_scarb: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonBuildResponse {
    run: CommandRun,
    cache_hit: bool,
    fingerprint: String,
    session_key: String,
    #[serde(default)]
    telemetry: BuildPhaseTelemetry,
    #[serde(default)]
    compile_backend: DaemonBuildBackend,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonMetadataRequest {
    #[serde(default)]
    protocol_version: String,
    manifest_path: String,
    format_version: u32,
    offline: bool,
    global_cache_dir: Option<String>,
    capture_output: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonMetadataResponse {
    run: CommandRun,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DaemonRequest {
    Ping,
    Shutdown,
    Build {
        #[serde(flatten)]
        payload: DaemonBuildRequest,
    },
    Metadata {
        #[serde(flatten)]
        payload: DaemonMetadataRequest,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DaemonResponse {
    Pong {
        #[serde(flatten)]
        payload: DaemonStatusPayload,
    },
    Ack,
    Build {
        #[serde(flatten)]
        payload: DaemonBuildResponse,
    },
    Metadata {
        #[serde(flatten)]
        payload: DaemonMetadataResponse,
    },
    Error {
        message: String,
    },
}

struct CacheLockGuard {
    path: PathBuf,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct FingerprintIndex {
    schema_version: u32,
    #[serde(default)]
    entries: BTreeMap<String, FingerprintIndexEntry>,
    #[serde(default)]
    directories: BTreeMap<String, u64>,
    #[serde(default)]
    context_digest: Option<String>,
    #[serde(default)]
    last_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct FingerprintIndexEntry {
    size_bytes: u64,
    modified_unix_ms: u64,
    blake3_hex: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct ArtifactIndex {
    schema_version: u32,
    entries: BTreeMap<String, ArtifactIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArtifactIndexEntry {
    size_bytes: u64,
    modified_unix_ms: u64,
    blake3_hex: String,
}

struct DaemonRateLimiter {
    events: VecDeque<Instant>,
}

#[derive(Debug, Default, Clone)]
struct DaemonHealth {
    total_requests: u64,
    failed_requests: u64,
    rate_limited_requests: u64,
    consecutive_failures: u64,
    last_error: Option<String>,
    last_failure_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy)]
enum NativeFallbackReason {
    PreflightIneligible,
    LocalNativeError,
    DaemonNativeError,
    DaemonBackendDowngrade,
}

#[derive(Default, Clone, Copy)]
struct NativeCacheTelemetrySnapshot {
    session_entries: u64,
    session_estimated_bytes: u64,
    context_entries: u64,
    context_estimated_bytes: u64,
    build_locks: u64,
    metadata_entries: u64,
    metadata_estimated_bytes: u64,
    refresh_none_count: u64,
    refresh_incremental_count: u64,
    refresh_full_rebuild_count: u64,
    refresh_changed_files_total: u64,
    refresh_removed_files_total: u64,
    fallback_preflight_ineligible_count: u64,
    fallback_local_native_error_count: u64,
    fallback_daemon_native_error_count: u64,
    fallback_daemon_backend_downgrade_count: u64,
}

#[cfg(feature = "native-compile")]
fn native_refresh_none_counter() -> &'static AtomicU64 {
    static VALUE: OnceLock<AtomicU64> = OnceLock::new();
    VALUE.get_or_init(|| AtomicU64::new(0))
}

#[cfg(feature = "native-compile")]
fn native_refresh_incremental_counter() -> &'static AtomicU64 {
    static VALUE: OnceLock<AtomicU64> = OnceLock::new();
    VALUE.get_or_init(|| AtomicU64::new(0))
}

#[cfg(feature = "native-compile")]
fn native_refresh_full_rebuild_counter() -> &'static AtomicU64 {
    static VALUE: OnceLock<AtomicU64> = OnceLock::new();
    VALUE.get_or_init(|| AtomicU64::new(0))
}

#[cfg(feature = "native-compile")]
fn native_refresh_changed_files_counter() -> &'static AtomicU64 {
    static VALUE: OnceLock<AtomicU64> = OnceLock::new();
    VALUE.get_or_init(|| AtomicU64::new(0))
}

#[cfg(feature = "native-compile")]
fn native_refresh_removed_files_counter() -> &'static AtomicU64 {
    static VALUE: OnceLock<AtomicU64> = OnceLock::new();
    VALUE.get_or_init(|| AtomicU64::new(0))
}

fn native_fallback_preflight_counter() -> &'static AtomicU64 {
    static VALUE: OnceLock<AtomicU64> = OnceLock::new();
    VALUE.get_or_init(|| AtomicU64::new(0))
}

fn native_fallback_local_error_counter() -> &'static AtomicU64 {
    static VALUE: OnceLock<AtomicU64> = OnceLock::new();
    VALUE.get_or_init(|| AtomicU64::new(0))
}

fn native_fallback_daemon_error_counter() -> &'static AtomicU64 {
    static VALUE: OnceLock<AtomicU64> = OnceLock::new();
    VALUE.get_or_init(|| AtomicU64::new(0))
}

fn native_fallback_daemon_backend_downgrade_counter() -> &'static AtomicU64 {
    static VALUE: OnceLock<AtomicU64> = OnceLock::new();
    VALUE.get_or_init(|| AtomicU64::new(0))
}

fn record_native_fallback(reason: NativeFallbackReason) {
    match reason {
        NativeFallbackReason::PreflightIneligible => {
            native_fallback_preflight_counter().fetch_add(1, Ordering::Relaxed);
        }
        NativeFallbackReason::LocalNativeError => {
            native_fallback_local_error_counter().fetch_add(1, Ordering::Relaxed);
        }
        NativeFallbackReason::DaemonNativeError => {
            native_fallback_daemon_error_counter().fetch_add(1, Ordering::Relaxed);
        }
        NativeFallbackReason::DaemonBackendDowngrade => {
            native_fallback_daemon_backend_downgrade_counter().fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn native_fallback_telemetry_snapshot() -> (u64, u64, u64, u64) {
    (
        native_fallback_preflight_counter().load(Ordering::Relaxed),
        native_fallback_local_error_counter().load(Ordering::Relaxed),
        native_fallback_daemon_error_counter().load(Ordering::Relaxed),
        native_fallback_daemon_backend_downgrade_counter().load(Ordering::Relaxed),
    )
}

#[cfg(feature = "native-compile")]
fn record_native_refresh_telemetry(
    action: NativeSessionRefreshAction,
    changed_files: usize,
    removed_files: usize,
) {
    match action {
        NativeSessionRefreshAction::None => {
            native_refresh_none_counter().fetch_add(1, Ordering::Relaxed);
        }
        NativeSessionRefreshAction::IncrementalChangedSet => {
            native_refresh_incremental_counter().fetch_add(1, Ordering::Relaxed);
        }
        NativeSessionRefreshAction::FullRebuild => {
            native_refresh_full_rebuild_counter().fetch_add(1, Ordering::Relaxed);
        }
    }
    native_refresh_changed_files_counter().fetch_add(changed_files as u64, Ordering::Relaxed);
    native_refresh_removed_files_counter().fetch_add(removed_files as u64, Ordering::Relaxed);
}

#[cfg(feature = "native-compile")]
fn native_refresh_telemetry_snapshot() -> (u64, u64, u64, u64, u64) {
    (
        native_refresh_none_counter().load(Ordering::Relaxed),
        native_refresh_incremental_counter().load(Ordering::Relaxed),
        native_refresh_full_rebuild_counter().load(Ordering::Relaxed),
        native_refresh_changed_files_counter().load(Ordering::Relaxed),
        native_refresh_removed_files_counter().load(Ordering::Relaxed),
    )
}

#[cfg(feature = "native-compile")]
fn evict_expired_native_compile_session_cache_entries(
    cache: &mut HashMap<String, NativeCompileSessionCacheEntry>,
    now_ms: u64,
    ttl_ms: u64,
) {
    if ttl_ms == 0 {
        return;
    }
    cache.retain(|_, entry| now_ms.saturating_sub(entry.last_access_epoch_ms) <= ttl_ms);
}

#[cfg(feature = "native-compile")]
fn evict_expired_native_compile_context_cache_entries(
    cache: &mut HashMap<String, NativeCompileContextCacheEntry>,
    now_ms: u64,
    ttl_ms: u64,
) {
    if ttl_ms == 0 {
        return;
    }
    cache.retain(|_, entry| now_ms.saturating_sub(entry.last_access_epoch_ms) <= ttl_ms);
}

#[cfg(feature = "native-compile")]
fn metadata_result_cache_stats() -> (u64, u64) {
    let cache = metadata_result_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let bytes = cache
        .values()
        .map(|entry| entry.estimated_bytes)
        .fold(0_u64, u64::saturating_add);
    (cache.len() as u64, bytes)
}

#[cfg(feature = "native-compile")]
fn native_cache_telemetry_snapshot() -> NativeCacheTelemetrySnapshot {
    let now_ms = epoch_ms_u64().unwrap_or_default();
    let (metadata_entries, metadata_estimated_bytes) = metadata_result_cache_stats();
    let (session_entries, session_estimated_bytes) = {
        let mut cache = native_compile_session_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        evict_expired_native_compile_session_cache_entries(
            &mut cache,
            now_ms,
            native_compile_session_cache_ttl_ms(),
        );
        let bytes = cache
            .values()
            .map(|entry| entry.estimated_bytes)
            .fold(0_u64, u64::saturating_add);
        (cache.len() as u64, bytes)
    };
    let (context_entries, context_estimated_bytes) = {
        let mut cache = native_compile_context_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        evict_expired_native_compile_context_cache_entries(
            &mut cache,
            now_ms,
            native_compile_context_cache_ttl_ms(),
        );
        let bytes = cache
            .values()
            .map(|entry| entry.estimated_bytes)
            .fold(0_u64, u64::saturating_add);
        (cache.len() as u64, bytes)
    };
    let build_locks = {
        let locks = native_compile_session_build_locks()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks.len() as u64
    };
    let (
        refresh_none_count,
        refresh_incremental_count,
        refresh_full_rebuild_count,
        refresh_changed_files_total,
        refresh_removed_files_total,
    ) = native_refresh_telemetry_snapshot();
    let (
        fallback_preflight_ineligible_count,
        fallback_local_native_error_count,
        fallback_daemon_native_error_count,
        fallback_daemon_backend_downgrade_count,
    ) = native_fallback_telemetry_snapshot();
    NativeCacheTelemetrySnapshot {
        session_entries,
        session_estimated_bytes,
        context_entries,
        context_estimated_bytes,
        build_locks,
        metadata_entries,
        metadata_estimated_bytes,
        refresh_none_count,
        refresh_incremental_count,
        refresh_full_rebuild_count,
        refresh_changed_files_total,
        refresh_removed_files_total,
        fallback_preflight_ineligible_count,
        fallback_local_native_error_count,
        fallback_daemon_native_error_count,
        fallback_daemon_backend_downgrade_count,
    }
}

#[cfg(not(feature = "native-compile"))]
fn native_cache_telemetry_snapshot() -> NativeCacheTelemetrySnapshot {
    NativeCacheTelemetrySnapshot::default()
}

impl DaemonRateLimiter {
    fn new() -> Self {
        Self {
            events: VecDeque::new(),
        }
    }

    fn allow(&mut self) -> bool {
        let now = Instant::now();
        let window = Duration::from_secs(DAEMON_RATE_WINDOW_SECONDS);
        while let Some(oldest) = self.events.front() {
            if now.duration_since(*oldest) < window {
                break;
            }
            self.events.pop_front();
        }
        if self.events.len() >= DAEMON_MAX_REQUESTS_PER_WINDOW {
            return false;
        }
        self.events.push_back(now);
        true
    }
}

impl FingerprintIndex {
    fn empty() -> Self {
        Self {
            schema_version: FINGERPRINT_INDEX_SCHEMA_VERSION,
            entries: BTreeMap::new(),
            directories: BTreeMap::new(),
            context_digest: None,
            last_fingerprint: None,
        }
    }
}

impl ArtifactIndex {
    fn empty() -> Self {
        Self {
            schema_version: ARTIFACT_INDEX_SCHEMA_VERSION,
            entries: BTreeMap::new(),
        }
    }
}

impl Drop for CacheLockGuard {
    fn drop(&mut self) {
        if let Err(err) = fs::remove_file(&self.path) {
            if err.kind() != io::ErrorKind::NotFound {
                eprintln!(
                    "uc: warning: failed to remove cache lock {}: {err}",
                    self.path.display()
                );
            }
        }
    }
}

fn main() -> Result<()> {
    init_observability();
    let cli = Cli::parse();

    match cli.command {
        Commands::Daemon(args) => run_daemon(args),
        #[cfg(feature = "dev-benchmark-command")]
        Commands::Benchmark(args) => benchmark_cmd::run(args),
        Commands::Cache(args) => run_cache(args),
        Commands::SessionKey(args) => run_session_key(args),
        Commands::Build(args) => run_build(args),
        Commands::Metadata(args) => run_metadata(args),
        Commands::CompareBuild(args) => run_compare_build(args),
        Commands::Migrate(args) => run_migrate(args),
    }
}

fn init_observability() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let filter = EnvFilter::try_from_default_env()
            .or_else(|_| {
                let fallback = std::env::var("UC_LOG").unwrap_or_else(|_| "uc=info".to_string());
                EnvFilter::try_new(fallback)
            })
            .unwrap_or_else(|_| EnvFilter::new("info"));
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .with_ansi(false)
            .without_time()
            .try_init();
    });
}

fn parse_diagnostics_threshold(input: &str) -> std::result::Result<f64, String> {
    let parsed = input
        .parse::<f64>()
        .map_err(|_| format!("invalid diagnostics threshold `{input}`"))?;
    if !(0.0..=100.0).contains(&parsed) {
        return Err(format!(
            "diagnostics threshold must be between 0 and 100, got {parsed}"
        ));
    }
    Ok(parsed)
}

fn parse_metadata_format_version(input: &str) -> std::result::Result<u32, String> {
    let parsed = input
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("invalid metadata format version `{input}`"))?;
    if matches!(parsed, 1 | 2) {
        return Ok(parsed);
    }
    Err(format!(
        "unsupported metadata format version `{parsed}` (expected 1 or 2)"
    ))
}

fn parse_env_u64(name: &str, default: u64) -> u64 {
    match std::env::var(name) {
        Ok(raw) => match raw.parse::<u64>() {
            Ok(value) => value,
            Err(_) => {
                tracing::warn!(env = name, value = %raw, default, "invalid numeric setting; using default");
                default
            }
        },
        Err(_) => default,
    }
}

fn parse_env_usize(name: &str, default: usize) -> usize {
    match std::env::var(name) {
        Ok(raw) => match raw.parse::<usize>() {
            Ok(value) => value,
            Err(_) => {
                tracing::warn!(env = name, value = %raw, default, "invalid numeric setting; using default");
                default
            }
        },
        Err(_) => default,
    }
}

fn parse_env_bool(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(raw) => {
            let normalized = raw.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => true,
                "0" | "false" | "no" | "off" => false,
                _ => {
                    tracing::warn!(env = name, value = %raw, default, "invalid boolean setting; using default");
                    default
                }
            }
        }
        Err(_) => default,
    }
}

#[cfg(feature = "native-compile")]
fn native_crate_cache_enabled() -> bool {
    parse_env_bool(
        "UC_NATIVE_CRATE_CACHE_ENABLED",
        DEFAULT_NATIVE_CRATE_CACHE_ENABLED,
    )
}

#[cfg(feature = "native-compile")]
fn native_crate_cache_max_bytes() -> u64 {
    let configured = parse_env_u64(
        "UC_NATIVE_CRATE_CACHE_MAX_BYTES",
        DEFAULT_NATIVE_CRATE_CACHE_MAX_BYTES,
    );
    if configured < MAX_NATIVE_CRATE_CACHE_BLOB_BYTES {
        tracing::warn!(
            env = "UC_NATIVE_CRATE_CACHE_MAX_BYTES",
            configured_bytes = configured,
            floor_bytes = MAX_NATIVE_CRATE_CACHE_BLOB_BYTES,
            "configured cache budget is below single-entry floor; using floor value"
        );
        return MAX_NATIVE_CRATE_CACHE_BLOB_BYTES;
    }
    configured
}

#[cfg(feature = "native-compile")]
fn parse_version_component_leading_u64(raw: &str) -> Option<u64> {
    let digits = raw
        .trim()
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u64>().ok()
}

#[cfg(feature = "native-compile")]
fn parse_cairo_version_major_minor(raw: &str) -> Option<(u64, u64)> {
    let normalized = raw.trim().trim_start_matches('v');
    let mut parts = normalized.split('.');
    let major = parse_version_component_leading_u64(parts.next()?)?;
    let minor = parse_version_component_leading_u64(parts.next()?)?;
    Some((major, minor))
}

#[cfg(feature = "native-compile")]
fn manifest_package_cairo_version(manifest: &TomlValue) -> Option<String> {
    manifest
        .get("package")
        .and_then(TomlValue::as_table)
        .and_then(|table| {
            table
                .get("cairo-version")
                .or_else(|| table.get("cairo_version"))
        })
        .and_then(TomlValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(feature = "native-compile")]
fn ensure_native_manifest_cairo_version_supported(manifest: &TomlValue) -> Result<()> {
    let Some(requested) = manifest_package_cairo_version(manifest) else {
        return Ok(());
    };
    let Some(requested_major_minor) = parse_cairo_version_major_minor(&requested) else {
        return Err(native_fallback_eligible_error(format!(
            "native compile requires an exact cairo-version (major.minor[.patch]); unsupported constraint `{requested}`",
        )));
    };
    let compiler = native_cairo_lang_compiler_version();
    let Some(compiler_major_minor) = parse_cairo_version_major_minor(compiler) else {
        return Ok(());
    };
    let (compiler_major, compiler_minor) = compiler_major_minor;
    let (requested_major, requested_minor) = requested_major_minor;
    if compiler_major == requested_major && compiler_minor >= requested_minor {
        if compiler_minor > requested_minor {
            tracing::debug!(
                compiler = %compiler,
                requested = %requested,
                "native cairo compiler minor is newer than manifest cairo-version; accepting compatibility"
            );
        }
        return Ok(());
    }
    Err(native_fallback_eligible_error(format!(
        "native cairo-lang {compiler} is incompatible with package cairo-version {requested}; native requires same major and compiler minor >= requested minor"
    )))
}

fn default_native_build_mode() -> NativeBuildMode {
    match DEFAULT_UC_NATIVE_BUILD_MODE {
        "off" => NativeBuildMode::Off,
        "auto" => NativeBuildMode::Auto,
        "require" => NativeBuildMode::Require,
        _ => NativeBuildMode::Off,
    }
}

fn parse_native_build_mode(raw: &str) -> NativeBuildMode {
    match raw.trim().to_ascii_lowercase().as_str() {
        "off" => NativeBuildMode::Off,
        "auto" => NativeBuildMode::Auto,
        "require" => NativeBuildMode::Require,
        _ => {
            tracing::warn!(
                env = "UC_NATIVE_BUILD_MODE",
                value = %raw,
                default = DEFAULT_UC_NATIVE_BUILD_MODE,
                "invalid native build mode; using default"
            );
            default_native_build_mode()
        }
    }
}

fn native_build_mode() -> NativeBuildMode {
    let raw = std::env::var("UC_NATIVE_BUILD_MODE")
        .unwrap_or_else(|_| DEFAULT_UC_NATIVE_BUILD_MODE.to_string());
    parse_native_build_mode(&raw)
}

fn native_disallow_scarb_fallback() -> bool {
    parse_env_bool(
        "UC_NATIVE_DISALLOW_SCARB_FALLBACK",
        DEFAULT_UC_NATIVE_DISALLOW_SCARB_FALLBACK,
    )
}

fn parse_lockfile_dependency_version(lockfile: &str, dependency_name: &str) -> Option<String> {
    let mut in_target_package = false;
    for line in lockfile.lines() {
        let trimmed = line.trim();
        if trimmed == "[[package]]" {
            in_target_package = false;
            continue;
        }
        if let Some(name) = trimmed
            .strip_prefix("name = \"")
            .and_then(|raw| raw.strip_suffix('"'))
        {
            in_target_package = name == dependency_name;
            continue;
        }
        if in_target_package {
            if let Some(version) = trimmed
                .strip_prefix("version = \"")
                .and_then(|raw| raw.strip_suffix('"'))
            {
                return Some(version.to_string());
            }
        }
    }
    None
}

fn native_lockfile_fallback_version(lockfile: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(lockfile.as_bytes());
    let digest = hasher.finalize().to_hex().to_string();
    format!("lockhash-{}", &digest[..32])
}

#[cfg(feature = "native-compile")]
fn native_cairo_lang_compiler_version() -> &'static str {
    static VALUE: OnceLock<String> = OnceLock::new();
    VALUE
        .get_or_init(|| {
            let lockfile = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"));
            parse_lockfile_dependency_version(lockfile, "cairo-lang-compiler").unwrap_or_else(
                || {
                    let fallback = native_lockfile_fallback_version(lockfile);
                    tracing::warn!(
                        fallback = %fallback,
                        "cairo-lang-compiler version missing in embedded Cargo.lock; using lock-hash fallback namespace"
                    );
                    fallback
                },
            )
        })
        .as_str()
}

#[cfg(not(feature = "native-compile"))]
fn native_cairo_lang_compiler_version() -> &'static str {
    "disabled"
}

#[cfg(feature = "native-compile")]
fn native_compiler_version_line() -> String {
    static VALUE: OnceLock<String> = OnceLock::new();
    VALUE
        .get_or_init(|| {
            format!(
                "uc-native {} cairo-lang {}",
                env!("CARGO_PKG_VERSION"),
                native_cairo_lang_compiler_version()
            )
        })
        .clone()
}

#[cfg(not(feature = "native-compile"))]
fn native_compiler_version_line() -> String {
    static VALUE: OnceLock<String> = OnceLock::new();
    VALUE
        .get_or_init(|| {
            format!(
                "uc-native {} cairo-lang {}",
                env!("CARGO_PKG_VERSION"),
                native_cairo_lang_compiler_version()
            )
        })
        .clone()
}

#[cfg(feature = "native-compile")]
fn native_max_casm_bytecode_size() -> usize {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_usize(
            "UC_NATIVE_MAX_CASM_BYTECODE_SIZE",
            DEFAULT_NATIVE_MAX_CASM_BYTECODE_SIZE,
        )
    })
}

fn max_fingerprint_files() -> usize {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| parse_env_usize("UC_MAX_FINGERPRINT_FILES", MAX_FINGERPRINT_FILES))
}

fn max_fingerprint_file_bytes() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE
        .get_or_init(|| parse_env_u64("UC_MAX_FINGERPRINT_FILE_BYTES", MAX_FINGERPRINT_FILE_BYTES))
}

fn max_fingerprint_total_bytes() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_u64(
            "UC_MAX_FINGERPRINT_TOTAL_BYTES",
            MAX_FINGERPRINT_TOTAL_BYTES,
        )
    })
}

fn fingerprint_timeout_ms() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| parse_env_u64("UC_FINGERPRINT_TIMEOUT_MS", FINGERPRINT_TIMEOUT_MS))
}

fn fingerprint_mtime_recheck_window_ms() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_u64(
            "UC_FINGERPRINT_MTIME_RECHECK_WINDOW_MS",
            FINGERPRINT_MTIME_RECHECK_WINDOW_MS,
        )
    })
}

fn max_cache_entry_bytes() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| parse_env_u64("UC_MAX_CACHE_ENTRY_BYTES", MAX_CACHE_ENTRY_BYTES))
}

fn max_fingerprint_index_bytes() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_u64(
            "UC_MAX_FINGERPRINT_INDEX_BYTES",
            MAX_FINGERPRINT_INDEX_BYTES,
        )
    })
}

fn max_artifact_index_bytes() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| parse_env_u64("UC_MAX_ARTIFACT_INDEX_BYTES", MAX_ARTIFACT_INDEX_BYTES))
}

fn max_restore_existing_hash_bytes() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_u64(
            "UC_MAX_RESTORE_EXISTING_HASH_BYTES",
            DEFAULT_MAX_RESTORE_EXISTING_HASH_BYTES,
        )
    })
}

fn max_capture_stdout_bytes() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| parse_env_u64("UC_MAX_CAPTURE_STDOUT_BYTES", MAX_CAPTURE_STDOUT_BYTES))
}

fn max_capture_stderr_bytes() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| parse_env_u64("UC_MAX_CAPTURE_STDERR_BYTES", MAX_CAPTURE_STDERR_BYTES))
}

fn max_cache_bytes() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| parse_env_u64("UC_MAX_CACHE_BYTES", DEFAULT_MAX_CACHE_BYTES))
}

fn cache_budget_enforce_every() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_u64(
            "UC_CACHE_BUDGET_ENFORCE_EVERY",
            DEFAULT_CACHE_BUDGET_ENFORCE_EVERY,
        )
        .max(1)
    })
}

fn cache_budget_min_interval_ms() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_u64(
            "UC_CACHE_BUDGET_MIN_INTERVAL_MS",
            DEFAULT_CACHE_BUDGET_MIN_INTERVAL_MS,
        )
    })
}

fn session_input_cache_max_entries() -> usize {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_usize(
            "UC_SESSION_INPUT_CACHE_MAX_ENTRIES",
            DEFAULT_SESSION_INPUT_CACHE_MAX_ENTRIES,
        )
        .max(1)
    })
}

fn fingerprint_index_cache_max_entries() -> usize {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_usize(
            "UC_FINGERPRINT_INDEX_CACHE_MAX_ENTRIES",
            DEFAULT_FINGERPRINT_INDEX_CACHE_MAX_ENTRIES,
        )
        .max(1)
    })
}

fn build_entry_cache_max_entries() -> usize {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_usize(
            "UC_BUILD_ENTRY_CACHE_MAX_ENTRIES",
            DEFAULT_BUILD_ENTRY_CACHE_MAX_ENTRIES,
        )
        .max(1)
    })
}

fn artifact_index_cache_max_entries() -> usize {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_usize(
            "UC_ARTIFACT_INDEX_CACHE_MAX_ENTRIES",
            DEFAULT_ARTIFACT_INDEX_CACHE_MAX_ENTRIES,
        )
        .max(1)
    })
}

fn daemon_build_plan_cache_max_entries() -> usize {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_usize(
            "UC_DAEMON_BUILD_PLAN_CACHE_MAX_ENTRIES",
            DEFAULT_DAEMON_BUILD_PLAN_CACHE_MAX_ENTRIES,
        )
        .max(1)
    })
}

fn daemon_lock_hash_cache_max_entries() -> usize {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_usize(
            "UC_DAEMON_LOCK_HASH_CACHE_MAX_ENTRIES",
            DEFAULT_DAEMON_LOCK_HASH_CACHE_MAX_ENTRIES,
        )
        .max(1)
    })
}

fn metadata_result_cache_max_entries() -> usize {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_usize(
            "UC_METADATA_RESULT_CACHE_MAX_ENTRIES",
            DEFAULT_METADATA_RESULT_CACHE_MAX_ENTRIES,
        )
        .max(1)
    })
}

fn metadata_result_cache_max_bytes() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_u64(
            "UC_METADATA_RESULT_CACHE_MAX_BYTES",
            DEFAULT_METADATA_RESULT_CACHE_MAX_BYTES,
        )
    })
}

#[cfg(feature = "native-compile")]
fn native_compile_session_cache_max_entries() -> usize {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_usize(
            "UC_NATIVE_COMPILE_SESSION_CACHE_MAX_ENTRIES",
            DEFAULT_NATIVE_COMPILE_SESSION_CACHE_MAX_ENTRIES,
        )
        .max(1)
    })
}

#[cfg(feature = "native-compile")]
fn native_compile_context_cache_max_entries() -> usize {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_usize(
            "UC_NATIVE_COMPILE_CONTEXT_CACHE_MAX_ENTRIES",
            DEFAULT_NATIVE_COMPILE_CONTEXT_CACHE_MAX_ENTRIES,
        )
        .max(1)
    })
}

#[cfg(feature = "native-compile")]
fn native_compile_session_cache_max_bytes() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_u64(
            "UC_NATIVE_COMPILE_SESSION_CACHE_MAX_BYTES",
            DEFAULT_NATIVE_COMPILE_SESSION_CACHE_MAX_BYTES,
        )
    })
}

#[cfg(feature = "native-compile")]
fn native_compile_context_cache_max_bytes() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_u64(
            "UC_NATIVE_COMPILE_CONTEXT_CACHE_MAX_BYTES",
            DEFAULT_NATIVE_COMPILE_CONTEXT_CACHE_MAX_BYTES,
        )
    })
}

#[cfg(feature = "native-compile")]
fn native_compile_session_cache_ttl_ms() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_u64(
            "UC_NATIVE_COMPILE_SESSION_CACHE_TTL_MS",
            DEFAULT_NATIVE_COMPILE_SESSION_CACHE_TTL_MS,
        )
    })
}

#[cfg(feature = "native-compile")]
fn native_compile_context_cache_ttl_ms() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_u64(
            "UC_NATIVE_COMPILE_CONTEXT_CACHE_TTL_MS",
            DEFAULT_NATIVE_COMPILE_CONTEXT_CACHE_TTL_MS,
        )
    })
}

#[cfg(feature = "native-compile")]
fn native_incremental_max_changed_files() -> usize {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_usize(
            "UC_NATIVE_INCREMENTAL_MAX_CHANGED_FILES",
            DEFAULT_NATIVE_INCREMENTAL_MAX_CHANGED_FILES,
        )
    })
}

#[cfg(feature = "native-compile")]
fn native_impacted_subset_enabled() -> bool {
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_bool(
            "UC_NATIVE_IMPACTED_SUBSET_ENABLED",
            DEFAULT_NATIVE_IMPACTED_SUBSET_ENABLED,
        )
    })
}

#[cfg(feature = "native-compile")]
fn native_capture_statement_locations() -> bool {
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_bool(
            "UC_NATIVE_CAPTURE_STATEMENT_LOCATIONS",
            DEFAULT_NATIVE_CAPTURE_STATEMENT_LOCATIONS,
        )
    })
}

#[cfg(feature = "native-compile")]
fn native_capture_statement_locations_on_cold() -> bool {
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_bool(
            "UC_NATIVE_CAPTURE_STATEMENT_LOCATIONS_ON_COLD",
            DEFAULT_NATIVE_CAPTURE_STATEMENT_LOCATIONS_ON_COLD,
        )
    })
}

#[cfg(feature = "native-compile")]
fn native_progress_enabled() -> bool {
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| parse_env_bool("UC_NATIVE_PROGRESS", DEFAULT_NATIVE_PROGRESS_ENABLED))
}

#[cfg(feature = "native-compile")]
fn native_progress_heartbeat_secs() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_u64(
            "UC_NATIVE_PROGRESS_HEARTBEAT_SECS",
            DEFAULT_NATIVE_PROGRESS_HEARTBEAT_SECS,
        )
    })
}

#[cfg(feature = "native-compile")]
fn load_native_progress_compile_batch_size() -> usize {
    parse_env_usize(
        "UC_NATIVE_PROGRESS_COMPILE_BATCH_SIZE",
        DEFAULT_NATIVE_PROGRESS_COMPILE_BATCH_SIZE,
    )
}

#[cfg(feature = "native-compile")]
fn native_progress_compile_batch_size() -> usize {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(load_native_progress_compile_batch_size)
}

#[cfg(feature = "native-compile")]
fn native_progress_log(message: impl AsRef<str>) {
    let message = message.as_ref();
    #[cfg(test)]
    {
        let hook = native_progress_test_hook()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(hook) = hook {
            hook(message.to_string());
        }
    }
    if native_progress_enabled() {
        eprintln!("uc: {message}");
    }
}

#[cfg(all(feature = "native-compile", test))]
type NativeProgressTestHook = Arc<dyn Fn(String) + Send + Sync>;

#[cfg(all(feature = "native-compile", test))]
fn native_progress_test_hook() -> &'static Mutex<Option<NativeProgressTestHook>> {
    static VALUE: OnceLock<Mutex<Option<NativeProgressTestHook>>> = OnceLock::new();
    VALUE.get_or_init(|| Mutex::new(None))
}

#[cfg(all(feature = "native-compile", test))]
fn set_native_progress_test_hook(
    hook: Option<NativeProgressTestHook>,
) -> Option<NativeProgressTestHook> {
    let mut slot = native_progress_test_hook()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::mem::replace(&mut *slot, hook)
}

#[cfg(feature = "native-compile")]
struct NativeProgressHeartbeat {
    stop_tx: Option<mpsc::Sender<()>>,
    handle: Option<thread::JoinHandle<()>>,
}

#[cfg(feature = "native-compile")]
impl NativeProgressHeartbeat {
    fn start(label: impl Into<String>) -> Self {
        if !native_progress_enabled() {
            return Self {
                stop_tx: None,
                handle: None,
            };
        }
        let interval_secs = native_progress_heartbeat_secs();
        if interval_secs == 0 {
            return Self {
                stop_tx: None,
                handle: None,
            };
        }
        let label = label.into();
        let (stop_tx, stop_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let started_at = Instant::now();
            loop {
                match stop_rx.recv_timeout(Duration::from_secs(interval_secs)) {
                    Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        eprintln!(
                            "uc: {label} still running ({:.1}s elapsed)",
                            started_at.elapsed().as_secs_f64()
                        );
                    }
                }
            }
        });
        Self {
            stop_tx: Some(stop_tx),
            handle: Some(handle),
        }
    }
}

#[cfg(feature = "native-compile")]
impl Drop for NativeProgressHeartbeat {
    fn drop(&mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(feature = "native-compile")]
fn native_dependency_metadata_enabled() -> bool {
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_bool(
            "UC_NATIVE_DEPENDENCY_METADATA_ENABLED",
            DEFAULT_NATIVE_DEPENDENCY_METADATA_ENABLED,
        )
    })
}

#[cfg(feature = "native-compile")]
fn native_eager_keyed_slot_prime_enabled() -> bool {
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_bool(
            "UC_NATIVE_EAGER_KEYED_SLOT_PRIME",
            DEFAULT_NATIVE_EAGER_KEYED_SLOT_PRIME,
        )
    })
}

#[cfg(feature = "native-compile")]
fn native_compile_session_memory_multiplier() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_u64(
            "UC_NATIVE_COMPILE_SESSION_MEMORY_MULTIPLIER",
            DEFAULT_NATIVE_COMPILE_SESSION_MEMORY_MULTIPLIER,
        )
        .max(1)
    })
}

#[cfg(feature = "native-compile")]
fn native_compile_session_memory_base_overhead_bytes() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_u64(
            "UC_NATIVE_COMPILE_SESSION_MEMORY_BASE_OVERHEAD_BYTES",
            DEFAULT_NATIVE_COMPILE_SESSION_MEMORY_BASE_OVERHEAD_BYTES,
        )
    })
}

fn daemon_shared_cache_enabled() -> bool {
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_bool(
            "UC_DAEMON_SHARED_CACHE_ENABLED",
            DEFAULT_DAEMON_SHARED_CACHE_ENABLED,
        )
    })
}

fn daemon_shared_cache_max_bytes() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_u64(
            "UC_DAEMON_SHARED_CACHE_MAX_BYTES",
            DEFAULT_DAEMON_SHARED_CACHE_MAX_BYTES,
        )
    })
}

fn async_persist_error_log_max_bytes() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_u64(
            "UC_ASYNC_PERSIST_ERROR_LOG_MAX_BYTES",
            DEFAULT_ASYNC_PERSIST_ERROR_LOG_MAX_BYTES,
        )
    })
}

fn should_enforce_cache_size_budget_for_persist_index(
    persist_index: u64,
    enforce_every: u64,
) -> bool {
    if enforce_every <= 1 {
        return true;
    }
    persist_index.is_multiple_of(enforce_every)
}

fn should_enforce_cache_size_budget_for_state(
    persist_index: u64,
    enforce_every: u64,
    now_ms: u64,
    last_enforced_ms: u64,
    min_interval_ms: u64,
) -> bool {
    if !should_enforce_cache_size_budget_for_persist_index(persist_index, enforce_every) {
        return false;
    }
    if min_interval_ms == 0 {
        return true;
    }
    if last_enforced_ms == 0 {
        return false;
    }
    now_ms.saturating_sub(last_enforced_ms) >= min_interval_ms
}

fn should_enforce_cache_size_budget_now() -> bool {
    static PERSIST_COUNT: AtomicU64 = AtomicU64::new(0);
    static LAST_ENFORCED_MS: AtomicU64 = AtomicU64::new(0);
    let persist_index = PERSIST_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    let enforce_every = cache_budget_enforce_every();
    let min_interval_ms = cache_budget_min_interval_ms();
    // Dual gate to avoid expensive full-tree cache scans on high-frequency builds:
    // 1) enforce every N persists, and 2) never more often than min_interval_ms.
    if !should_enforce_cache_size_budget_for_persist_index(persist_index, enforce_every) {
        return false;
    }
    if min_interval_ms == 0 {
        return true;
    }
    let now_ms = epoch_ms_u64().unwrap_or_default();
    loop {
        let last = LAST_ENFORCED_MS.load(Ordering::Relaxed);
        if !should_enforce_cache_size_budget_for_state(
            persist_index,
            enforce_every,
            now_ms,
            last,
            min_interval_ms,
        ) {
            if last == 0 {
                let _ = LAST_ENFORCED_MS.compare_exchange(
                    0,
                    now_ms,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                );
            }
            return false;
        }
        match LAST_ENFORCED_MS.compare_exchange(last, now_ms, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => return true,
            Err(_) => continue,
        }
    }
}

fn fail_on_async_cache_error() -> bool {
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| parse_env_bool("UC_FAIL_ON_ASYNC_CACHE_ERROR", false))
}

fn daemon_async_cache_persist_enabled() -> bool {
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| parse_env_bool("UC_DAEMON_ASYNC_CACHE_PERSIST", false))
}

fn daemon_capture_output_enabled() -> bool {
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| parse_env_bool("UC_DAEMON_CAPTURE_OUTPUT", true))
}

fn default_daemon_capture_output() -> bool {
    true
}

fn should_log_phase_telemetry() -> bool {
    match std::env::var("UC_PHASE_TIMING") {
        Ok(raw) => {
            let normalized = raw.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        }
        Err(_) => false,
    }
}

fn timeout_duration_from_secs(seconds: u64) -> Option<Duration> {
    if seconds == 0 {
        return None;
    }
    Some(Duration::from_secs(seconds))
}

fn daemon_control_read_timeout() -> Option<Duration> {
    timeout_duration_from_secs(parse_env_u64(
        "UC_DAEMON_CLIENT_READ_TIMEOUT_SECS",
        DEFAULT_DAEMON_CLIENT_READ_TIMEOUT_SECS,
    ))
}

fn daemon_build_read_timeout() -> Option<Duration> {
    timeout_duration_from_secs(parse_env_u64(
        "UC_DAEMON_BUILD_READ_TIMEOUT_SECS",
        DEFAULT_DAEMON_BUILD_READ_TIMEOUT_SECS,
    ))
}

fn daemon_client_write_timeout() -> Option<Duration> {
    timeout_duration_from_secs(parse_env_u64(
        "UC_DAEMON_CLIENT_WRITE_TIMEOUT_SECS",
        DEFAULT_DAEMON_CLIENT_WRITE_TIMEOUT_SECS,
    ))
}

fn daemon_max_connection_handlers() -> usize {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_usize(
            "UC_DAEMON_MAX_CONNECTION_HANDLERS",
            DEFAULT_DAEMON_MAX_CONNECTION_HANDLERS,
        )
        .max(1)
    })
}

fn daemon_response_size_limit_bytes() -> usize {
    let compute = || {
        let configured = parse_env_usize(
            "UC_DAEMON_RESPONSE_SIZE_LIMIT_BYTES",
            DEFAULT_DAEMON_RESPONSE_SIZE_LIMIT_BYTES,
        );
        let minimum = max_capture_stdout_bytes()
            .saturating_add(max_capture_stderr_bytes())
            .saturating_add(DAEMON_RESPONSE_SIZE_OVERHEAD_BYTES as u64)
            .min(usize::MAX as u64) as usize;
        configured.max(minimum).max(DAEMON_REQUEST_SIZE_LIMIT_BYTES)
    };

    if cfg!(test) {
        return compute();
    }

    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(compute)
}

fn daemon_request_read_timeout(request: &DaemonRequest) -> Option<Duration> {
    match request {
        DaemonRequest::Build { .. } => daemon_build_read_timeout(),
        _ => daemon_control_read_timeout(),
    }
}

fn daemon_request_write_timeout() -> Option<Duration> {
    daemon_client_write_timeout()
}

fn resolve_diagnostics_threshold(cli_value: Option<f64>) -> Result<f64> {
    if let Some(value) = cli_value {
        return Ok(value);
    }
    if let Ok(raw) = std::env::var("UC_DIAGNOSTICS_THRESHOLD") {
        return parse_diagnostics_threshold(&raw)
            .map_err(anyhow::Error::msg)
            .context("failed to parse UC_DIAGNOSTICS_THRESHOLD");
    }
    Ok(DEFAULT_DIAGNOSTICS_SIMILARITY_THRESHOLD)
}

fn validate_metadata_format_version(version: u32) -> Result<()> {
    if matches!(version, 1 | 2) {
        return Ok(());
    }
    bail!("unsupported metadata format version `{version}` (expected 1 or 2)");
}

fn validate_daemon_protocol_version(version: &str) -> Result<()> {
    if version == DAEMON_PROTOCOL_VERSION {
        return Ok(());
    }
    bail!(
        "daemon protocol mismatch (daemon={}, client={})",
        version,
        DAEMON_PROTOCOL_VERSION
    );
}

fn daemon_response_protocol_mismatch(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("protocol mismatch")
        || normalized.contains("daemon build request protocol mismatch")
        || normalized.contains("daemon metadata request protocol mismatch")
}

fn run_daemon(args: DaemonArgs) -> Result<()> {
    match args.command {
        DaemonCommand::Start(socket) => run_daemon_start(socket),
        DaemonCommand::Status(socket) => run_daemon_status(socket),
        DaemonCommand::Health(socket) => run_daemon_health(socket),
        DaemonCommand::Stop(socket) => run_daemon_stop(socket),
        DaemonCommand::Serve(socket) => run_daemon_serve(socket),
    }
}

fn run_cache(args: CacheArgs) -> Result<()> {
    match args.command {
        CacheCommand::Clean(clean) => run_cache_clean(clean),
    }
}

fn run_cache_clean(args: CacheCleanArgs) -> Result<()> {
    let manifest_path = resolve_manifest_path(&args.manifest_path)?;
    let workspace_root = manifest_path
        .parent()
        .context("manifest path has no parent")?
        .to_path_buf();
    let cache_root = workspace_root.join(".uc/cache");
    if !cache_root.exists() {
        println!("uc cache is already clean: {}", cache_root.display());
        return Ok(());
    }
    fs::remove_dir_all(&cache_root)
        .with_context(|| format!("failed to remove cache directory {}", cache_root.display()))?;
    println!("uc cache cleaned: {}", cache_root.display());
    Ok(())
}

fn run_daemon_start(args: DaemonSocketArgs) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = args;
        bail!("daemon mode is currently supported on Unix platforms only");
    }
    #[cfg(unix)]
    {
        let socket_path = daemon_socket_path(args.socket_path)?;
        if daemon_ping(&socket_path).is_ok() {
            println!("uc daemon already running on {}", socket_path.display());
            return Ok(());
        }

        if let Some(parent) = socket_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        remove_socket_if_exists(&socket_path)?;

        let log_path = daemon_log_path(&socket_path);
        rotate_daemon_log_if_needed(&log_path)?;
        let (log_file, log_file_err) = open_daemon_log_file(&log_path)?;

        let exe = std::env::current_exe().context("failed to resolve uc binary path")?;
        let mut command = Command::new(exe);
        command
            .arg("daemon")
            .arg("serve")
            .arg("--socket-path")
            .arg(&socket_path)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(log_file_err));
        #[cfg(unix)]
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command.spawn().with_context(|| {
            format!(
                "failed to launch daemon process for {}",
                socket_path.display()
            )
        })?;

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(status) = daemon_ping(&socket_path) {
                println!(
                    "uc daemon started (pid={}, socket={})",
                    status.pid, status.socket_path
                );
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        bail!(
            "daemon failed to become ready; inspect log at {}",
            log_path.display()
        );
    }
}

fn run_daemon_status(args: DaemonSocketArgs) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = args;
        bail!("daemon mode is currently supported on Unix platforms only");
    }
    #[cfg(unix)]
    {
        let socket_path = daemon_socket_path(args.socket_path)?;
        let status = daemon_ping(&socket_path)
            .with_context(|| format!("daemon not reachable on {}", socket_path.display()))?;
        println!(
            "uc daemon running (pid={}, started_at_epoch_ms={}, socket={}, protocol={}, healthy={}, total_requests={}, failed_requests={}, rate_limited_requests={}, native_session_cache_entries={}, native_session_cache_estimated_bytes={}, native_context_cache_entries={}, native_context_cache_estimated_bytes={}, native_build_locks={}, metadata_cache_entries={}, metadata_cache_estimated_bytes={}, native_refresh_none={}, native_refresh_incremental={}, native_refresh_full_rebuild={}, native_refresh_changed_files_total={}, native_refresh_removed_files_total={}, native_fallback_preflight_ineligible={}, native_fallback_local_native_error={}, native_fallback_daemon_native_error={}, native_fallback_daemon_backend_downgrade={}, last_error={})",
            status.pid,
            status.started_at_epoch_ms,
            status.socket_path,
            status.protocol_version,
            status.healthy,
            status.total_requests,
            status.failed_requests,
            status.rate_limited_requests,
            status.native_compile_session_cache_entries,
            status.native_compile_session_cache_estimated_bytes,
            status.native_compile_context_cache_entries,
            status.native_compile_context_cache_estimated_bytes,
            status.native_compile_session_build_locks,
            status.metadata_result_cache_entries,
            status.metadata_result_cache_estimated_bytes,
            status.native_refresh_none_count,
            status.native_refresh_incremental_count,
            status.native_refresh_full_rebuild_count,
            status.native_refresh_changed_files_total,
            status.native_refresh_removed_files_total,
            status.native_fallback_preflight_ineligible_count,
            status.native_fallback_local_native_error_count,
            status.native_fallback_daemon_native_error_count,
            status.native_fallback_daemon_backend_downgrade_count,
            status
                .last_error
                .clone()
                .unwrap_or_else(|| "none".to_string())
        );
        Ok(())
    }
}

fn run_daemon_health(args: DaemonSocketArgs) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = args;
        bail!("daemon mode is currently supported on Unix platforms only");
    }
    #[cfg(unix)]
    {
        let socket_path = daemon_socket_path(args.socket_path)?;
        let status = daemon_ping(&socket_path)
            .with_context(|| format!("daemon not reachable on {}", socket_path.display()))?;
        if status.healthy {
            println!(
                "healthy (pid={}, total_requests={}, failed_requests={}, rate_limited_requests={}, native_session_cache_entries={}, native_session_cache_estimated_bytes={}, native_context_cache_entries={}, native_context_cache_estimated_bytes={}, native_build_locks={}, metadata_cache_entries={}, metadata_cache_estimated_bytes={}, native_refresh_none={}, native_refresh_incremental={}, native_refresh_full_rebuild={}, native_refresh_changed_files_total={}, native_refresh_removed_files_total={}, native_fallback_preflight_ineligible={}, native_fallback_local_native_error={}, native_fallback_daemon_native_error={}, native_fallback_daemon_backend_downgrade={})",
                status.pid,
                status.total_requests,
                status.failed_requests,
                status.rate_limited_requests,
                status.native_compile_session_cache_entries,
                status.native_compile_session_cache_estimated_bytes,
                status.native_compile_context_cache_entries,
                status.native_compile_context_cache_estimated_bytes,
                status.native_compile_session_build_locks,
                status.metadata_result_cache_entries,
                status.metadata_result_cache_estimated_bytes,
                status.native_refresh_none_count,
                status.native_refresh_incremental_count,
                status.native_refresh_full_rebuild_count,
                status.native_refresh_changed_files_total,
                status.native_refresh_removed_files_total,
                status.native_fallback_preflight_ineligible_count,
                status.native_fallback_local_native_error_count,
                status.native_fallback_daemon_native_error_count,
                status.native_fallback_daemon_backend_downgrade_count
            );
            Ok(())
        } else {
            bail!(
                "unhealthy daemon on {}: last_error={}",
                status.socket_path,
                status
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string())
            )
        }
    }
}

fn run_daemon_stop(args: DaemonSocketArgs) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = args;
        bail!("daemon mode is currently supported on Unix platforms only");
    }
    #[cfg(unix)]
    {
        let socket_path = daemon_socket_path(args.socket_path)?;
        if !socket_path.exists() {
            println!("uc daemon is not running ({})", socket_path.display());
            return Ok(());
        }

        let response =
            daemon_request(&socket_path, &DaemonRequest::Shutdown).with_context(|| {
                format!(
                    "failed to request daemon shutdown {}",
                    socket_path.display()
                )
            })?;
        match response {
            DaemonResponse::Ack => {}
            DaemonResponse::Error { message } => bail!("daemon shutdown failed: {message}"),
            _ => {}
        }

        let deadline = Instant::now() + Duration::from_secs(3);
        let mut stopped = false;
        while Instant::now() < deadline {
            if daemon_ping(&socket_path).is_err() {
                stopped = true;
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        if !stopped && daemon_ping(&socket_path).is_ok() {
            bail!(
                "daemon did not stop within timeout and is still reachable on {}",
                socket_path.display()
            );
        }
        if socket_path.exists() {
            remove_socket_if_exists(&socket_path)?;
        }
        println!("uc daemon stopped ({})", socket_path.display());
        Ok(())
    }
}

fn run_daemon_serve(args: DaemonSocketArgs) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = args;
        bail!("daemon mode is currently supported on Unix platforms only");
    }
    #[cfg(unix)]
    {
        let socket_path = daemon_socket_path(args.socket_path)?;
        if let Some(parent) = socket_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        remove_socket_if_exists(&socket_path)?;
        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("failed to bind daemon socket {}", socket_path.display()))?;
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600)).with_context(
            || {
                format!(
                    "failed to set daemon socket permissions for {}",
                    socket_path.display()
                )
            },
        )?;
        let native_cache = native_cache_telemetry_snapshot();
        let status = Arc::new(DaemonStatusPayload {
            pid: std::process::id(),
            started_at_epoch_ms: epoch_ms_u64()?,
            socket_path: socket_path.display().to_string(),
            protocol_version: DAEMON_PROTOCOL_VERSION.to_string(),
            healthy: true,
            total_requests: 0,
            failed_requests: 0,
            rate_limited_requests: 0,
            last_error: None,
            native_compile_session_cache_entries: native_cache.session_entries,
            native_compile_session_cache_estimated_bytes: native_cache.session_estimated_bytes,
            native_compile_context_cache_entries: native_cache.context_entries,
            native_compile_context_cache_estimated_bytes: native_cache.context_estimated_bytes,
            native_compile_session_build_locks: native_cache.build_locks,
            metadata_result_cache_entries: native_cache.metadata_entries,
            metadata_result_cache_estimated_bytes: native_cache.metadata_estimated_bytes,
            native_refresh_none_count: native_cache.refresh_none_count,
            native_refresh_incremental_count: native_cache.refresh_incremental_count,
            native_refresh_full_rebuild_count: native_cache.refresh_full_rebuild_count,
            native_refresh_changed_files_total: native_cache.refresh_changed_files_total,
            native_refresh_removed_files_total: native_cache.refresh_removed_files_total,
            native_fallback_preflight_ineligible_count: native_cache
                .fallback_preflight_ineligible_count,
            native_fallback_local_native_error_count: native_cache
                .fallback_local_native_error_count,
            native_fallback_daemon_native_error_count: native_cache
                .fallback_daemon_native_error_count,
            native_fallback_daemon_backend_downgrade_count: native_cache
                .fallback_daemon_backend_downgrade_count,
        });
        let health = Arc::new(Mutex::new(DaemonHealth::default()));
        let rate_limiter = Arc::new(Mutex::new(DaemonRateLimiter::new()));
        let should_shutdown = Arc::new(AtomicBool::new(false));
        let active_handlers = Arc::new(AtomicUsize::new(0));
        let max_connection_handlers = daemon_max_connection_handlers();
        prewarm_daemon_compiler_version_cache();

        loop {
            if should_shutdown.load(Ordering::Acquire) {
                break;
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let in_flight = active_handlers.fetch_add(1, Ordering::AcqRel) + 1;
                    if in_flight > max_connection_handlers {
                        active_handlers.fetch_sub(1, Ordering::AcqRel);
                        if let Err(err) = write_daemon_over_capacity_response(&mut stream) {
                            tracing::warn!(
                                error = %format!("{err:#}"),
                                "failed to reply to over-capacity daemon connection"
                            );
                        }
                        continue;
                    }
                    let status = Arc::clone(&status);
                    let health = Arc::clone(&health);
                    let should_shutdown = Arc::clone(&should_shutdown);
                    let rate_limiter = Arc::clone(&rate_limiter);
                    let active_handlers = Arc::clone(&active_handlers);
                    thread::spawn(move || {
                        struct ActiveDaemonConnectionGuard(Arc<AtomicUsize>);
                        impl Drop for ActiveDaemonConnectionGuard {
                            fn drop(&mut self) {
                                self.0.fetch_sub(1, Ordering::AcqRel);
                            }
                        }
                        let _active_guard = ActiveDaemonConnectionGuard(active_handlers);
                        if let Err(err) = handle_daemon_connection(
                            stream,
                            status.as_ref(),
                            &health,
                            &should_shutdown,
                            &rate_limiter,
                        ) {
                            record_daemon_failure(&health, format!("{err:#}"));
                            tracing::error!(
                                error = %format!("{err:#}"),
                                "daemon request handling failed"
                            );
                            eprintln!("uc daemon: request handling failed: {err:#}");
                        }
                    });
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => {
                    if should_shutdown.load(Ordering::Acquire) {
                        break;
                    }
                    tracing::error!(error = %err, "daemon socket accept failed");
                    eprintln!("uc daemon: socket accept failed: {err}");
                    thread::sleep(Duration::from_millis(50));
                }
            }
        }
        remove_socket_if_exists(&socket_path)?;
        Ok(())
    }
}

#[cfg(unix)]
fn write_daemon_over_capacity_response(stream: &mut UnixStream) -> Result<()> {
    let response = DaemonResponse::Error {
        message: format!(
            "daemon is busy (max {} concurrent handlers); retry shortly",
            daemon_max_connection_handlers()
        ),
    };
    let payload = serde_json::to_vec(&response).context("failed to encode daemon response")?;
    stream
        .write_all(&payload)
        .context("failed to write daemon response")?;
    stream
        .write_all(b"\n")
        .context("failed to write daemon response newline")?;
    stream.flush().context("failed to flush daemon response")?;
    Ok(())
}

fn prewarm_daemon_compiler_version_cache() {
    if let Err(err) = scarb_version_line() {
        tracing::debug!(
            error = %format!("{err:#}"),
            "daemon compiler version prewarm skipped"
        );
    }
}

fn run_session_key(args: SessionKeyArgs) -> Result<()> {
    let input = SessionInput {
        compiler_version: args.compiler_version,
        profile: args.profile,
        offline: args.offline,
        package: args.package,
        features: args.features,
        cfg_set: args.cfg_set,
        manifest_content_hash: args.manifest_content_hash,
        target_family: args.target_family,
        cairo_edition: args.cairo_edition,
        cairo_lang_version: args.cairo_lang_version,
        build_env_fingerprint: args.build_env_fingerprint,
    };

    println!("{}", input.deterministic_key_hex());
    Ok(())
}

fn try_uc_build_via_daemon(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    fallback_to_local: bool,
    compile_backend: BuildCompileBackend,
    native_fallback_to_scarb: bool,
) -> Result<Option<DaemonBuildResponse>> {
    #[cfg(not(unix))]
    {
        let _ = (
            common,
            manifest_path,
            fallback_to_local,
            compile_backend,
            native_fallback_to_scarb,
        );
        return Ok(None);
    }
    #[cfg(unix)]
    {
        let socket_path = daemon_socket_path(None)?;
        if !socket_path.exists() {
            return Ok(None);
        }

        let request = DaemonRequest::Build {
            payload: daemon_build_request_from_common(
                common,
                manifest_path,
                daemon_async_cache_persist_enabled(),
                daemon_capture_output_enabled(),
                compile_backend,
                native_fallback_to_scarb,
            ),
        };
        let response = match daemon_request(&socket_path, &request) {
            Ok(response) => response,
            Err(err) => {
                if fallback_to_local {
                    eprintln!(
                        "uc: daemon request failed ({}), falling back to local engine",
                        err
                    );
                    return Ok(None);
                }
                return Err(err).context("daemon build request failed");
            }
        };

        match response {
            DaemonResponse::Build { payload } => {
                let actual_backend = payload.compile_backend.into_compile_backend();
                let backend_mismatch = actual_backend != compile_backend
                    && !(compile_backend == BuildCompileBackend::Native
                        && native_fallback_to_scarb
                        && actual_backend == BuildCompileBackend::Scarb);
                if backend_mismatch {
                    let message = format!(
                        "daemon returned backend {:?} when {:?} was requested",
                        payload.compile_backend, compile_backend
                    );
                    if fallback_to_local {
                        eprintln!("uc: {message}, falling back to local engine");
                        Ok(None)
                    } else {
                        bail!("daemon build request failed: {message}");
                    }
                } else {
                    if actual_backend == BuildCompileBackend::Scarb
                        && compile_backend == BuildCompileBackend::Native
                    {
                        if native_disallow_scarb_fallback() {
                            bail!(
                                "daemon build request failed: native fallback is disallowed (UC_NATIVE_DISALLOW_SCARB_FALLBACK=1)"
                            );
                        }
                        record_native_fallback(NativeFallbackReason::DaemonBackendDowngrade);
                        eprintln!(
                            "uc: daemon native build failed; daemon fell back to scarb backend"
                        );
                    }
                    Ok(Some(payload))
                }
            }
            DaemonResponse::Error { message } => {
                if fallback_to_local {
                    if daemon_response_protocol_mismatch(&message) {
                        eprintln!(
                            "uc: daemon protocol mismatch ({message}), falling back to local engine"
                        );
                    } else {
                        eprintln!(
                            "uc: daemon returned error ({}), falling back to local engine",
                            message
                        );
                    }
                    Ok(None)
                } else {
                    if daemon_response_protocol_mismatch(&message) {
                        bail!("{message}");
                    }
                    bail!("daemon build request failed: {message}");
                }
            }
            _ => {
                if fallback_to_local {
                    Ok(None)
                } else {
                    bail!("daemon build request failed: unexpected response type");
                }
            }
        }
    }
}

fn try_uc_metadata_via_daemon(
    args: &MetadataArgs,
    manifest_path: &Path,
    capture_output: bool,
    fallback_to_local: bool,
) -> Result<Option<CommandRun>> {
    #[cfg(not(unix))]
    {
        let _ = (args, manifest_path, capture_output, fallback_to_local);
        return Ok(None);
    }
    #[cfg(unix)]
    {
        let socket_path = daemon_socket_path(None)?;
        if !socket_path.exists() {
            return Ok(None);
        }

        let request = DaemonRequest::Metadata {
            payload: daemon_metadata_request_from_args(args, manifest_path, capture_output),
        };
        let response = match daemon_request(&socket_path, &request) {
            Ok(response) => response,
            Err(err) => {
                if fallback_to_local {
                    eprintln!(
                        "uc: daemon request failed ({}), falling back to local metadata",
                        err
                    );
                    return Ok(None);
                }
                return Err(err).context("daemon metadata request failed");
            }
        };

        match response {
            DaemonResponse::Metadata { payload } => Ok(Some(payload.run)),
            DaemonResponse::Error { message } => {
                if fallback_to_local {
                    if daemon_response_protocol_mismatch(&message) {
                        eprintln!(
                            "uc: daemon protocol mismatch ({message}), falling back to local metadata"
                        );
                    } else {
                        eprintln!(
                            "uc: daemon returned error ({}), falling back to local metadata",
                            message
                        );
                    }
                    Ok(None)
                } else {
                    if daemon_response_protocol_mismatch(&message) {
                        bail!("{message}");
                    }
                    bail!("daemon metadata request failed: {message}");
                }
            }
            _ => Ok(None),
        }
    }
}

fn daemon_build_request_from_common(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    async_cache_persist: bool,
    capture_output: bool,
    compile_backend: BuildCompileBackend,
    native_fallback_to_scarb: bool,
) -> DaemonBuildRequest {
    DaemonBuildRequest {
        protocol_version: DAEMON_PROTOCOL_VERSION.to_string(),
        manifest_path: manifest_path.display().to_string(),
        package: common.package.clone(),
        workspace: common.workspace,
        features: common.features.clone(),
        offline: common.offline,
        release: common.release,
        profile: common.profile.clone(),
        async_cache_persist,
        capture_output,
        compile_backend: DaemonBuildBackend::from_compile_backend(compile_backend),
        native_fallback_to_scarb,
    }
}

fn common_args_from_daemon_request(request: &DaemonBuildRequest) -> BuildCommonArgs {
    BuildCommonArgs {
        manifest_path: Some(PathBuf::from(&request.manifest_path)),
        package: request.package.clone(),
        workspace: request.workspace,
        features: request.features.clone(),
        offline: request.offline,
        release: request.release,
        profile: request.profile.clone(),
    }
}

fn daemon_metadata_request_from_args(
    args: &MetadataArgs,
    manifest_path: &Path,
    capture_output: bool,
) -> DaemonMetadataRequest {
    DaemonMetadataRequest {
        protocol_version: DAEMON_PROTOCOL_VERSION.to_string(),
        manifest_path: manifest_path.display().to_string(),
        format_version: args.format_version,
        offline: args.offline,
        global_cache_dir: args
            .global_cache_dir
            .as_ref()
            .map(|path| path.display().to_string()),
        capture_output,
    }
}

fn metadata_args_from_daemon_request(request: &DaemonMetadataRequest) -> MetadataArgs {
    MetadataArgs {
        manifest_path: Some(PathBuf::from(&request.manifest_path)),
        format_version: request.format_version,
        daemon_mode: DaemonModeArg::Off,
        offline: request.offline,
        global_cache_dir: request.global_cache_dir.as_ref().map(PathBuf::from),
        report_path: None,
    }
}

fn compiler_version_for_backend(backend: BuildCompileBackend) -> Result<String> {
    match backend {
        BuildCompileBackend::Scarb => {
            // Daemon mode validates the Scarb toolchain in the daemon process so clients avoid
            // repeated `scarb --version` subprocess overhead per request.
            validate_scarb_toolchain()?;
            scarb_version_line()
        }
        BuildCompileBackend::Native => Ok(native_compiler_version_line()),
    }
}

fn execute_daemon_build_with_backend(
    request: &DaemonBuildRequest,
    common: &BuildCommonArgs,
    manifest_path: &Path,
    compile_backend: BuildCompileBackend,
    compiler_version: &str,
) -> Result<DaemonBuildResponse> {
    let (plan, plan_cache_hit) = prepare_daemon_build_plan_with_compiler_version(
        common,
        manifest_path,
        compile_backend,
        compiler_version,
    )?;
    if plan_cache_hit {
        tracing::debug!(
            manifest_path = %plan.manifest_path.display(),
            invalidation_key = %plan.strict_invalidation_key,
            compile_backend = ?compile_backend,
            "uc daemon build plan cache hit"
        );
    } else {
        tracing::debug!(
            manifest_path = %plan.manifest_path.display(),
            invalidation_key = %plan.strict_invalidation_key,
            compile_backend = ?compile_backend,
            "uc daemon build plan cache miss"
        );
    }

    let (run, cache_hit, fingerprint, telemetry) = run_build_with_uc_cache(
        common,
        BuildCacheRunContext {
            manifest_path: &plan.manifest_path,
            workspace_root: &plan.workspace_root,
            profile: &plan.profile,
            session_key: &plan.session_key,
            compiler_version,
            compile_backend,
            options: BuildRunOptions {
                capture_output: request.capture_output,
                inherit_output_when_uncaptured: request.capture_output,
                async_cache_persist: request.async_cache_persist,
                use_daemon_shared_cache: true,
            },
        },
    )?;

    Ok(DaemonBuildResponse {
        run,
        cache_hit,
        fingerprint,
        session_key: plan.session_key,
        telemetry,
        compile_backend: DaemonBuildBackend::from_compile_backend(compile_backend),
    })
}

fn execute_daemon_build(request: DaemonBuildRequest) -> Result<DaemonBuildResponse> {
    validate_daemon_protocol_version(&request.protocol_version)
        .context("daemon build request protocol mismatch")?;
    let common = common_args_from_daemon_request(&request);
    let manifest_path = resolve_manifest_path(&common.manifest_path)?;
    let requested_backend = request.compile_backend.into_compile_backend();
    let requested_compiler_version = compiler_version_for_backend(requested_backend)?;
    match execute_daemon_build_with_backend(
        &request,
        &common,
        &manifest_path,
        requested_backend,
        &requested_compiler_version,
    ) {
        Ok(response) => Ok(response),
        Err(native_err)
            if requested_backend == BuildCompileBackend::Native
                && request.native_fallback_to_scarb
                && native_error_allows_scarb_fallback(&native_err) =>
        {
            if native_disallow_scarb_fallback() {
                return Err(native_err).context(
                    "native fallback is disallowed (UC_NATIVE_DISALLOW_SCARB_FALLBACK=1)",
                );
            }
            record_native_fallback(NativeFallbackReason::DaemonNativeError);
            tracing::warn!(
                error = %native_err,
                "daemon native build unavailable; falling back to scarb backend"
            );
            let scarb_compiler_version = compiler_version_for_backend(BuildCompileBackend::Scarb)?;
            execute_daemon_build_with_backend(
                &request,
                &common,
                &manifest_path,
                BuildCompileBackend::Scarb,
                &scarb_compiler_version,
            )
            .context("daemon native fallback to scarb failed")
        }
        Err(err) => Err(err),
    }
}

fn try_metadata_result_cache_hit(
    cache_key: &str,
    entry_path: &Path,
    manifest_size_bytes: u64,
    manifest_modified_unix_ms: u64,
    lock_hash: &str,
    workspace_manifests_hash: &str,
) -> Result<Option<CommandRun>> {
    let now_ms = epoch_ms_u64().unwrap_or_default();
    {
        let mut cache = metadata_result_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = cache.get_mut(cache_key) {
            if metadata_cache_entry_matches(
                entry,
                manifest_size_bytes,
                manifest_modified_unix_ms,
                lock_hash,
                workspace_manifests_hash,
            ) {
                entry.last_access_epoch_ms = now_ms;
                return Ok(Some(entry.run.clone()));
            }
            cache.remove(cache_key);
        }
    }

    if !entry_path.exists() {
        return Ok(None);
    }

    let metadata = match fs::metadata(entry_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).with_context(|| format!("failed to stat {}", entry_path.display()));
        }
    };
    if metadata.len() > MAX_METADATA_RESULT_CACHE_ENTRY_BYTES {
        let _ = fs::remove_file(entry_path);
        tracing::warn!(
            path = %entry_path.display(),
            bytes = metadata.len(),
            max_bytes = MAX_METADATA_RESULT_CACHE_ENTRY_BYTES,
            "ignoring oversized metadata cache entry"
        );
        return Ok(None);
    }
    let bytes = read_bytes_with_limit(
        entry_path,
        MAX_METADATA_RESULT_CACHE_ENTRY_BYTES,
        "metadata cache entry",
    )?;
    let decoded: MetadataResultCacheFile =
        match serde_json::from_slice::<MetadataResultCacheFile>(&bytes) {
            Ok(entry) if entry.schema_version == METADATA_RESULT_CACHE_SCHEMA_VERSION => entry,
            Ok(_) => {
                let _ = fs::remove_file(entry_path);
                return Ok(None);
            }
            Err(err) => {
                let _ = fs::remove_file(entry_path);
                tracing::warn!(
                    path = %entry_path.display(),
                    error = %err,
                    "ignoring unreadable metadata cache entry"
                );
                return Ok(None);
            }
        };
    if !metadata_cache_file_matches(
        &decoded,
        manifest_size_bytes,
        manifest_modified_unix_ms,
        lock_hash,
        workspace_manifests_hash,
    ) {
        return Ok(None);
    }

    let estimated_bytes = metadata_run_estimated_bytes(&decoded.run);
    {
        let mut cache = metadata_result_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.insert(
            cache_key.to_string(),
            MetadataResultCacheEntry {
                manifest_size_bytes,
                manifest_modified_unix_ms,
                lock_hash: lock_hash.to_string(),
                workspace_manifests_hash: workspace_manifests_hash.to_string(),
                run: decoded.run.clone(),
                last_access_epoch_ms: now_ms,
                estimated_bytes,
            },
        );
        evict_oldest_metadata_result_cache_entries(
            &mut cache,
            metadata_result_cache_max_entries(),
            metadata_result_cache_max_bytes(),
        );
    }
    Ok(Some(decoded.run))
}

struct MetadataResultCacheWriteContext<'a> {
    cache_key: &'a str,
    cache_root: &'a Path,
    entry_path: &'a Path,
    manifest_size_bytes: u64,
    manifest_modified_unix_ms: u64,
    lock_hash: &'a str,
    workspace_manifests_hash: &'a str,
}

fn store_metadata_result_cache_entry(
    context: &MetadataResultCacheWriteContext<'_>,
    run: &CommandRun,
) -> Result<()> {
    let cache_entry = MetadataResultCacheFile {
        schema_version: METADATA_RESULT_CACHE_SCHEMA_VERSION,
        manifest_size_bytes: context.manifest_size_bytes,
        manifest_modified_unix_ms: context.manifest_modified_unix_ms,
        lock_hash: context.lock_hash.to_string(),
        workspace_manifests_hash: context.workspace_manifests_hash.to_string(),
        run: run.clone(),
    };
    let bytes =
        serde_json::to_vec(&cache_entry).context("failed to encode metadata cache entry")?;
    if bytes.len() as u64 > MAX_METADATA_RESULT_CACHE_ENTRY_BYTES {
        tracing::warn!(
            path = %context.entry_path.display(),
            bytes = bytes.len(),
            max_bytes = MAX_METADATA_RESULT_CACHE_ENTRY_BYTES,
            "skipping metadata cache write: entry exceeds max size"
        );
        return Ok(());
    }

    let _cache_lock = acquire_cache_lock(context.cache_root)?;
    atomic_write_bytes(context.entry_path, &bytes, "metadata cache entry")?;
    if should_enforce_cache_size_budget_now() {
        enforce_cache_size_budget(context.cache_root)?;
    }

    let now_ms = epoch_ms_u64().unwrap_or_default();
    let estimated_bytes = metadata_run_estimated_bytes(run);
    {
        let mut cache = metadata_result_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.insert(
            context.cache_key.to_string(),
            MetadataResultCacheEntry {
                manifest_size_bytes: context.manifest_size_bytes,
                manifest_modified_unix_ms: context.manifest_modified_unix_ms,
                lock_hash: context.lock_hash.to_string(),
                workspace_manifests_hash: context.workspace_manifests_hash.to_string(),
                run: run.clone(),
                last_access_epoch_ms: now_ms,
                estimated_bytes,
            },
        );
        evict_oldest_metadata_result_cache_entries(
            &mut cache,
            metadata_result_cache_max_entries(),
            metadata_result_cache_max_bytes(),
        );
    }
    Ok(())
}

fn manifest_has_workspace_table(manifest_path: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(manifest_path) else {
        return false;
    };
    let Ok(manifest) = toml::from_str::<TomlValue>(&contents) else {
        return false;
    };
    manifest.get("workspace").is_some()
}

fn metadata_cache_workspace_root(manifest_path: &Path) -> Result<PathBuf> {
    let manifest_parent = manifest_path
        .parent()
        .context("manifest path has no parent")?;
    for ancestor in manifest_parent.ancestors() {
        let ancestor_manifest = ancestor.join("Scarb.toml");
        if !ancestor_manifest.is_file() {
            continue;
        }
        if ancestor.join("Scarb.lock").is_file() {
            return Ok(ancestor.to_path_buf());
        }
        if ancestor != manifest_parent && manifest_has_workspace_table(&ancestor_manifest) {
            return Ok(ancestor.to_path_buf());
        }
    }
    Ok(manifest_parent.to_path_buf())
}

fn run_scarb_metadata_with_uc_cache(
    args: &MetadataArgs,
    manifest_path: &Path,
    capture_output: bool,
) -> Result<CommandRun> {
    let (command, command_vec) = scarb_metadata_command(args, manifest_path);
    if !capture_output {
        return run_command_status(command, command_vec);
    }

    let workspace_root = metadata_cache_workspace_root(manifest_path)?;
    let cache_root = workspace_root.join(".uc/cache");
    ensure_path_within_root(&workspace_root, &cache_root, "metadata cache root")?;
    let manifest_metadata = fs::metadata(manifest_path)
        .with_context(|| format!("failed to stat {}", manifest_path.display()))?;
    let manifest_size_bytes = manifest_metadata.len();
    let manifest_modified_unix_ms = metadata_modified_unix_ms(&manifest_metadata)?;
    let workspace_manifest_path = workspace_root.join("Scarb.toml");
    let (_, _, lock_hash) = daemon_lock_state(&workspace_manifest_path)?;
    let workspace_manifests_hash = metadata_workspace_manifests_hash(&workspace_root)?;
    let scarb_version = scarb_version_line()?;
    let build_env_fingerprint = current_build_env_fingerprint();
    let cache_key =
        metadata_result_cache_key(args, manifest_path, &scarb_version, &build_env_fingerprint);
    let entry_path = metadata_cache_entry_path(&workspace_root, &cache_key);
    ensure_path_within_root(&workspace_root, &entry_path, "metadata cache entry path")?;

    let lookup_start = Instant::now();
    if let Some(mut cached_run) = try_metadata_result_cache_hit(
        &cache_key,
        &entry_path,
        manifest_size_bytes,
        manifest_modified_unix_ms,
        &lock_hash,
        &workspace_manifests_hash,
    )? {
        cached_run.elapsed_ms = lookup_start.elapsed().as_secs_f64() * 1000.0;
        tracing::debug!(
            manifest_path = %manifest_path.display(),
            cache_key = %cache_key,
            lookup_ms = cached_run.elapsed_ms,
            "uc metadata result cache hit"
        );
        return Ok(cached_run);
    }
    tracing::debug!(
        manifest_path = %manifest_path.display(),
        cache_key = %cache_key,
        lookup_ms = lookup_start.elapsed().as_secs_f64() * 1000.0,
        "uc metadata result cache miss"
    );

    let run_start = Instant::now();
    let run = run_command_capture(command, command_vec)?;
    let run_ms = run_start.elapsed().as_secs_f64() * 1000.0;
    tracing::debug!(
        manifest_path = %manifest_path.display(),
        command_ms = run_ms,
        exit_code = run.exit_code,
        "uc metadata command completed"
    );

    if run.exit_code == 0 {
        let persist_start = Instant::now();
        let write_context = MetadataResultCacheWriteContext {
            cache_key: &cache_key,
            cache_root: &cache_root,
            entry_path: &entry_path,
            manifest_size_bytes,
            manifest_modified_unix_ms,
            lock_hash: &lock_hash,
            workspace_manifests_hash: &workspace_manifests_hash,
        };
        store_metadata_result_cache_entry(&write_context, &run)?;
        tracing::debug!(
            manifest_path = %manifest_path.display(),
            persist_ms = persist_start.elapsed().as_secs_f64() * 1000.0,
            "uc metadata cache persisted"
        );
    }

    Ok(run)
}

fn execute_daemon_metadata(request: DaemonMetadataRequest) -> Result<DaemonMetadataResponse> {
    validate_daemon_protocol_version(&request.protocol_version)
        .context("daemon metadata request protocol mismatch")?;
    validate_metadata_format_version(request.format_version)?;
    let args = metadata_args_from_daemon_request(&request);
    let manifest_path = resolve_manifest_path(&args.manifest_path)?;
    let run = run_scarb_metadata_with_uc_cache(&args, &manifest_path, request.capture_output)?;
    Ok(DaemonMetadataResponse { run })
}

fn try_local_uc_cache_hit(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    workspace_root: &Path,
    profile: &str,
    session_key: &str,
    compiler_version: &str,
) -> Result<Option<(CommandRun, String, BuildPhaseTelemetry)>> {
    let mut telemetry = BuildPhaseTelemetry::default();
    let canonical_workspace_root = workspace_root.to_path_buf();
    validate_hex_digest("session key", session_key, SESSION_KEY_LEN)?;
    let cache_root = canonical_workspace_root.join(".uc/cache");
    ensure_path_within_root(&canonical_workspace_root, &cache_root, "cache root")?;
    let objects_dir = cache_root.join("objects");
    let entry_path = cache_root.join("build").join(format!("{session_key}.json"));
    ensure_path_within_root(
        &canonical_workspace_root,
        &objects_dir,
        "cache objects directory",
    )?;
    ensure_path_within_root(&canonical_workspace_root, &entry_path, "cache entry path")?;

    let cache_lookup_start = Instant::now();
    let cached_entry = load_cache_entry_cached(&entry_path)?;
    telemetry.cache_lookup_ms = cache_lookup_start.elapsed().as_secs_f64() * 1000.0;

    let Some(entry) = cached_entry else {
        return Ok(None);
    };
    if entry.schema_version != BUILD_CACHE_SCHEMA_VERSION || entry.profile != profile {
        return Ok(None);
    }

    let fingerprint_start = Instant::now();
    let fingerprint = compute_build_fingerprint_with_scarb_version(
        &canonical_workspace_root,
        manifest_path,
        common,
        profile,
        Some(&cache_root),
        compiler_version,
    )?;
    telemetry.fingerprint_ms = fingerprint_start.elapsed().as_secs_f64() * 1000.0;
    if entry.fingerprint != fingerprint {
        return Ok(None);
    }

    let restore_start = Instant::now();
    let restored = cached_artifacts_already_materialized(
        &canonical_workspace_root,
        profile,
        &cache_root,
        &entry.artifacts,
    )? || restore_cached_artifacts(
        &canonical_workspace_root,
        profile,
        &cache_root,
        &objects_dir,
        &entry.artifacts,
    )?;
    telemetry.cache_restore_ms = restore_start.elapsed().as_secs_f64() * 1000.0;
    if !restored {
        return Ok(None);
    }
    let total_elapsed_ms =
        telemetry.fingerprint_ms + telemetry.cache_lookup_ms + telemetry.cache_restore_ms;
    let run = CommandRun {
        command: vec![
            "uc".to_string(),
            "build".to_string(),
            "--engine".to_string(),
            "uc".to_string(),
            "--cache-hit".to_string(),
            "--local-probe".to_string(),
        ],
        exit_code: 0,
        elapsed_ms: total_elapsed_ms,
        stdout: format!(
            "uc: cache hit, restored {} artifacts\n",
            entry.artifacts.len()
        ),
        stderr: String::new(),
    };
    Ok(Some((run, fingerprint, telemetry)))
}

fn run_build_with_uc_cache(
    common: &BuildCommonArgs,
    context: BuildCacheRunContext<'_>,
) -> Result<(CommandRun, bool, String, BuildPhaseTelemetry)> {
    let BuildCacheRunContext {
        manifest_path,
        workspace_root,
        profile,
        session_key,
        compiler_version,
        compile_backend,
        options,
    } = context;
    let async_errors = take_async_persist_errors();
    if !async_errors.is_empty() {
        if fail_on_async_cache_error() {
            bail!(
                "previous async cache persistence failed: {}",
                async_errors.join(" | ")
            );
        }
        for err in async_errors {
            tracing::warn!(error = %err, "previous async cache persistence failed");
            eprintln!("uc: warning: previous async cache persistence failed: {err}");
        }
    }
    let mut telemetry = BuildPhaseTelemetry::default();
    let canonical_workspace_root = workspace_root.to_path_buf();
    validate_hex_digest("session key", session_key, SESSION_KEY_LEN)?;
    let cache_root = canonical_workspace_root.join(".uc/cache");
    let local_cache_preexisted = cache_root.exists();
    ensure_path_within_root(&canonical_workspace_root, &cache_root, "cache root")?;
    let objects_dir = cache_root.join("objects");
    let entry_path = cache_root.join("build").join(format!("{session_key}.json"));
    ensure_path_within_root(
        &canonical_workspace_root,
        &objects_dir,
        "cache objects directory",
    )?;
    ensure_path_within_root(&canonical_workspace_root, &entry_path, "cache entry path")?;
    let cache_lookup_start = Instant::now();
    let cached_entry = load_cache_entry_cached(&entry_path)?;
    telemetry.cache_lookup_ms = cache_lookup_start.elapsed().as_secs_f64() * 1000.0;

    let mut fingerprint: Option<String> = None;
    let ensure_fingerprint =
        |fingerprint: &mut Option<String>, telemetry: &mut BuildPhaseTelemetry| -> Result<()> {
            if fingerprint.is_some() {
                return Ok(());
            }
            let fingerprint_start = Instant::now();
            *fingerprint = Some(compute_build_fingerprint_with_scarb_version(
                &canonical_workspace_root,
                manifest_path,
                common,
                profile,
                Some(&cache_root),
                compiler_version,
            )?);
            telemetry.fingerprint_ms += fingerprint_start.elapsed().as_secs_f64() * 1000.0;
            Ok(())
        };

    if let Some(entry) = cached_entry.filter(|entry| {
        entry.schema_version == BUILD_CACHE_SCHEMA_VERSION && entry.profile == profile
    }) {
        ensure_fingerprint(&mut fingerprint, &mut telemetry)?;
        if entry.fingerprint == fingerprint.as_deref().unwrap_or_default() {
            let restore_start = Instant::now();
            if cached_artifacts_already_materialized(
                &canonical_workspace_root,
                profile,
                &cache_root,
                &entry.artifacts,
            )? || restore_cached_artifacts(
                &canonical_workspace_root,
                profile,
                &cache_root,
                &objects_dir,
                &entry.artifacts,
            )? {
                telemetry.cache_restore_ms = restore_start.elapsed().as_secs_f64() * 1000.0;
                let total_elapsed_ms = telemetry.fingerprint_ms
                    + telemetry.cache_lookup_ms
                    + telemetry.cache_restore_ms;
                let run = CommandRun {
                    command: vec![
                        "uc".to_string(),
                        "build".to_string(),
                        "--engine".to_string(),
                        "uc".to_string(),
                        "--cache-hit".to_string(),
                    ],
                    exit_code: 0,
                    elapsed_ms: total_elapsed_ms,
                    stdout: format!(
                        "uc: cache hit, restored {} artifacts\n",
                        entry.artifacts.len()
                    ),
                    stderr: String::new(),
                };
                if compile_backend == BuildCompileBackend::Native && options.use_daemon_shared_cache
                {
                    schedule_native_daemon_session_prewarm(
                        common,
                        manifest_path,
                        &canonical_workspace_root,
                    );
                }
                return Ok((
                    run,
                    true,
                    fingerprint.clone().unwrap_or_default(),
                    telemetry,
                ));
            }
            telemetry.cache_restore_ms = restore_start.elapsed().as_secs_f64() * 1000.0;
        }
    }

    if options.use_daemon_shared_cache {
        let shared_lookup_start = Instant::now();
        let shared_restore =
            if daemon_shared_cache_entry_exists(&canonical_workspace_root, session_key)? {
                ensure_fingerprint(&mut fingerprint, &mut telemetry)?;
                try_restore_daemon_shared_cache(
                    &canonical_workspace_root,
                    profile,
                    session_key,
                    fingerprint.as_deref().unwrap_or_default(),
                )?
            } else {
                None
            };
        telemetry.cache_lookup_ms += shared_lookup_start.elapsed().as_secs_f64() * 1000.0;
        if let Some(restored_count) = shared_restore {
            let total_elapsed_ms =
                telemetry.fingerprint_ms + telemetry.cache_lookup_ms + telemetry.cache_restore_ms;
            let run = CommandRun {
                command: vec![
                    "uc".to_string(),
                    "build".to_string(),
                    "--engine".to_string(),
                    "uc".to_string(),
                    "--cache-hit".to_string(),
                    "--daemon-shared-cache".to_string(),
                ],
                exit_code: 0,
                elapsed_ms: total_elapsed_ms,
                stdout: format!("uc: cache hit, restored {} artifacts\n", restored_count),
                stderr: String::new(),
            };
            if compile_backend == BuildCompileBackend::Native && options.use_daemon_shared_cache {
                schedule_native_daemon_session_prewarm(
                    common,
                    manifest_path,
                    &canonical_workspace_root,
                );
            }
            return Ok((
                run,
                true,
                fingerprint.clone().unwrap_or_default(),
                telemetry,
            ));
        }
    }

    let (run, native_phase_telemetry, native_artifact_relative_paths) = match compile_backend {
        BuildCompileBackend::Scarb => {
            let (command, command_vec) = scarb_build_command(common, manifest_path);
            let run = if options.capture_output {
                run_command_capture(command, command_vec)?
            } else if options.inherit_output_when_uncaptured {
                run_command_status(command, command_vec)?
            } else {
                run_command_status_silent(command, command_vec)?
            };
            (run, None, None)
        }
        BuildCompileBackend::Native => {
            let (run, native_phase_telemetry, native_artifact_relative_paths) = run_native_build(
                common,
                manifest_path,
                &canonical_workspace_root,
                profile,
                options.use_daemon_shared_cache,
            )?;
            (
                run,
                Some(native_phase_telemetry),
                Some(native_artifact_relative_paths),
            )
        }
    };
    telemetry.compile_ms = run.elapsed_ms;
    if let Some(native_phase_telemetry) = native_phase_telemetry {
        telemetry.native_context_ms = native_phase_telemetry.context_ms;
        telemetry.native_target_dir_ms = native_phase_telemetry.target_dir_ms;
        telemetry.native_session_prepare_ms = native_phase_telemetry.session_prepare_ms;
        telemetry.native_frontend_compile_ms = native_phase_telemetry.frontend_compile_ms;
        telemetry.native_casm_ms = native_phase_telemetry.casm_ms;
        telemetry.native_artifact_write_ms = native_phase_telemetry.artifact_write_ms;
        telemetry.native_changed_files = native_phase_telemetry.changed_files;
        telemetry.native_removed_files = native_phase_telemetry.removed_files;
        telemetry.native_total_contracts = native_phase_telemetry.total_contracts;
        telemetry.native_compiled_contracts = native_phase_telemetry.compiled_contracts;
        telemetry.native_impacted_subset_used = native_phase_telemetry.impacted_subset_used;
        telemetry.native_journal_fallback_full_scan =
            native_phase_telemetry.journal_fallback_full_scan;
    }

    if run.exit_code == 0 {
        ensure_fingerprint(&mut fingerprint, &mut telemetry)?;
        let fingerprint_value = fingerprint.as_deref().unwrap_or_default();
        if options.async_cache_persist {
            telemetry.cache_persist_async = true;
            let mut precomputed_cached_artifacts: Option<Vec<CachedArtifact>> = None;
            if options.use_daemon_shared_cache && !local_cache_preexisted {
                let shared_persist_start = Instant::now();
                let shared_cached_artifacts = collect_cached_artifacts_for_entry_with_paths(
                    &canonical_workspace_root,
                    profile,
                    &cache_root,
                    &objects_dir,
                    native_artifact_relative_paths.as_deref(),
                )?;
                if let Err(err) = persist_daemon_shared_cache_entry_with_artifacts(
                    &canonical_workspace_root,
                    profile,
                    session_key,
                    fingerprint_value,
                    &objects_dir,
                    &shared_cached_artifacts,
                ) {
                    tracing::warn!(
                        error = %format!("{err:#}"),
                        "daemon shared cache persistence failed"
                    );
                    eprintln!("uc: warning: daemon shared cache persistence failed: {err:#}");
                } else {
                    telemetry.cache_persist_ms =
                        shared_persist_start.elapsed().as_secs_f64() * 1000.0;
                }
                precomputed_cached_artifacts = Some(shared_cached_artifacts);
            }
            let persist_scope_key = async_persist_scope_key(&canonical_workspace_root, profile);
            if try_mark_async_persist_in_flight(&persist_scope_key) {
                telemetry.cache_persist_scheduled = true;
                let task = AsyncPersistTask {
                    scope_key: persist_scope_key.clone(),
                    workspace_root: canonical_workspace_root.clone(),
                    profile: profile.to_string(),
                    fingerprint: fingerprint_value.to_string(),
                    artifact_relative_paths: native_artifact_relative_paths.clone(),
                    cached_artifacts: precomputed_cached_artifacts,
                    cache_root: cache_root.clone(),
                    objects_dir: objects_dir.clone(),
                    entry_path: entry_path.clone(),
                };
                match async_persist_sender().try_send(task) {
                    Ok(()) => {}
                    Err(TrySendError::Full(task)) => {
                        clear_async_persist_in_flight(&persist_scope_key);
                        let error = format!(
                            "async cache persistence queue is full (limit={ASYNC_PERSIST_QUEUE_LIMIT}); dropping task for {}",
                            task.workspace_root.display()
                        );
                        record_async_persist_error(error.clone());
                        tracing::warn!(error = %error, "failed to enqueue async cache persistence task");
                        eprintln!("uc: warning: {error}");
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        clear_async_persist_in_flight(&persist_scope_key);
                        let error = "async cache persistence worker is unavailable; task dropped"
                            .to_string();
                        record_async_persist_error(error.clone());
                        tracing::warn!(error = %error, "failed to enqueue async cache persistence task");
                        eprintln!("uc: warning: {error}");
                    }
                }
            }
        } else {
            let persist_start = Instant::now();
            let cached_artifacts = persist_cache_entry_for_build_with_artifacts(
                &canonical_workspace_root,
                profile,
                fingerprint_value,
                native_artifact_relative_paths.as_deref(),
                &cache_root,
                &objects_dir,
                &entry_path,
            )?;
            if options.use_daemon_shared_cache && !local_cache_preexisted {
                persist_daemon_shared_cache_entry_with_artifacts(
                    &canonical_workspace_root,
                    profile,
                    session_key,
                    fingerprint_value,
                    &objects_dir,
                    &cached_artifacts,
                )?;
            }
            telemetry.cache_persist_ms = persist_start.elapsed().as_secs_f64() * 1000.0;
        }
    }

    let reported_fingerprint = fingerprint.unwrap_or_default();
    Ok((run, false, reported_fingerprint, telemetry))
}

#[cfg(feature = "native-compile")]
fn native_daemon_prewarm_inflight() -> &'static Mutex<HashSet<String>> {
    static VALUE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    VALUE.get_or_init(|| Mutex::new(HashSet::new()))
}

#[cfg(feature = "native-compile")]
fn schedule_native_daemon_session_prewarm(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    workspace_root: &Path,
) {
    let prewarm_key = format!("{}::{}", workspace_root.display(), manifest_path.display());
    {
        let mut inflight = native_daemon_prewarm_inflight()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !inflight.insert(prewarm_key.clone()) {
            return;
        }
    }
    let common = common.clone();
    let manifest_path = manifest_path.to_path_buf();
    let workspace_root = workspace_root.to_path_buf();
    thread::spawn(move || {
        let prewarm_result = (|| -> Result<()> {
            let context = build_native_compile_context(&common, &manifest_path, &workspace_root)?;
            let signature = native_compile_session_signature(&manifest_path, &context);
            let _ = native_compile_session_handle(&workspace_root, &signature)?;
            Ok(())
        })();
        if let Err(err) = prewarm_result {
            tracing::debug!(
                workspace_root = %workspace_root.display(),
                manifest_path = %manifest_path.display(),
                error = %format!("{err:#}"),
                "daemon native session prewarm skipped"
            );
        } else {
            tracing::debug!(
                workspace_root = %workspace_root.display(),
                manifest_path = %manifest_path.display(),
                "daemon native session prewarmed"
            );
        }
        let mut inflight = native_daemon_prewarm_inflight()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inflight.remove(&prewarm_key);
    });
}

#[cfg(not(feature = "native-compile"))]
fn schedule_native_daemon_session_prewarm(
    _common: &BuildCommonArgs,
    _manifest_path: &Path,
    _workspace_root: &Path,
) {
}

/// Maps Scarb package names into a safe Cairo crate key used in `cairo_project.toml`.
/// The output keeps ASCII alphanumerics and `_`, normalizes all other characters to `_`,
/// and prefixes a leading digit with `_` so the key remains TOML-bare-key safe.
#[cfg(feature = "native-compile")]
fn normalize_package_name_for_cairo_crate(package_name: &str) -> String {
    let mut normalized = String::with_capacity(package_name.len());
    for ch in package_name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            normalized.push(ch);
        } else {
            normalized.push('_');
        }
    }
    if normalized.is_empty() {
        return "crate".to_string();
    }
    if normalized
        .as_bytes()
        .first()
        .is_some_and(|first| first.is_ascii_digit())
    {
        return format!("_{normalized}");
    }
    normalized
}

/// Produces artifact-safe stem components for generated native outputs.
/// The result contains only ASCII alphanumerics and `_`; empty results fall back to `contract`.
#[cfg(feature = "native-compile")]
fn sanitize_artifact_component(raw: &str) -> String {
    let mut value = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            value.push(ch);
        } else {
            value.push('_');
        }
    }
    if value.is_empty() {
        return "contract".to_string();
    }
    value
}

#[cfg(feature = "native-compile")]
fn native_corelib_layout_looks_compatible(corelib_src: &Path) -> bool {
    corelib_src.join("lib.cairo").is_file()
        && corelib_src.join("prelude.cairo").is_file()
        && corelib_src.join("ops.cairo").is_file()
}

#[cfg(feature = "native-compile")]
fn native_corelib_manifest_version(corelib_src: &Path) -> Option<String> {
    let manifest_path = corelib_src.parent()?.join("Scarb.toml");
    let manifest_text = fs::read_to_string(manifest_path).ok()?;
    let manifest: TomlValue = toml::from_str(&manifest_text).ok()?;
    manifest
        .get("package")
        .and_then(TomlValue::as_table)
        .and_then(|table| table.get("version"))
        .and_then(TomlValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(feature = "native-compile")]
fn native_corelib_version_matches_compiler(corelib_src: &Path) -> bool {
    let Some(found) = native_corelib_manifest_version(corelib_src) else {
        // Some dev layouts do not include Scarb.toml near corelib/src; treat as best-effort.
        return true;
    };
    let expected = native_cairo_lang_compiler_version();
    if found == expected {
        return true;
    }
    tracing::debug!(
        corelib_src = %corelib_src.display(),
        found_version = %found,
        expected_version = %expected,
        "skipping corelib candidate due to cairo-lang version mismatch"
    );
    false
}

#[cfg(feature = "native-compile")]
fn native_corelib_candidate_paths(workspace_root: &Path) -> Vec<(&'static str, PathBuf)> {
    let mut candidates: Vec<(&'static str, PathBuf)> = Vec::new();

    if let Some(parent) = workspace_root.parent() {
        candidates.push(("workspace-parent", parent.join("cairo/corelib/src")));
    }
    for ancestor in workspace_root.ancestors().skip(1).take(6) {
        candidates.push(("workspace-ancestor", ancestor.join("cairo/corelib/src")));
    }
    if let Some(path) = detect_corelib() {
        candidates.push(("cairo-detect", path));
    }
    if let Ok(current_exe) = std::env::current_exe() {
        for ancestor in current_exe.ancestors().skip(1).take(8) {
            candidates.push(("exe-ancestor", ancestor.join("corelib/src")));
            candidates.push(("exe-ancestor", ancestor.join("cairo/corelib/src")));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(("home", PathBuf::from(home).join(".cairo/corelib/src")));
    }
    candidates
}

#[cfg(feature = "native-compile")]
fn toml_escape_basic_string(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0C}' => escaped.push_str("\\f"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if c.is_control() => {
                let codepoint = c as u32;
                if codepoint <= 0xFFFF {
                    escaped.push_str(&format!("\\u{:04X}", codepoint));
                } else {
                    escaped.push_str(&format!("\\U{:08X}", codepoint));
                }
            }
            c => escaped.push(c),
        }
    }
    escaped
}

#[cfg(feature = "native-compile")]
fn resolve_native_corelib_src(workspace_root: &Path) -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("UC_NATIVE_CORELIB_SRC") {
        let candidate = PathBuf::from(path);
        let canonical = candidate.canonicalize().with_context(|| {
            format!(
                "failed to canonicalize native corelib override {}",
                candidate.display()
            )
        })?;
        if native_corelib_layout_looks_compatible(&canonical) {
            if !native_corelib_version_matches_compiler(&canonical) {
                return Err(native_fallback_eligible_error(format!(
                    "native corelib override {} version does not match cairo-lang {}; set UC_NATIVE_CORELIB_SRC to a compatible corelib/src",
                    canonical.display(),
                    native_cairo_lang_compiler_version()
                )));
            }
            tracing::debug!(
                source = "env",
                corelib_src = %canonical.display(),
                "selected native corelib source path"
            );
            return Ok(canonical);
        }
        return Err(native_fallback_eligible_error(format!(
            "native corelib override {} is incompatible; expected corelib/src to contain lib.cairo, prelude.cairo, and ops.cairo compatible with cairo-lang {}",
            canonical.display(),
            native_cairo_lang_compiler_version()
        )));
    }

    let candidates = native_corelib_candidate_paths(workspace_root);
    let mut seen = HashSet::new();
    let mut attempted = Vec::new();

    for (source, candidate) in candidates {
        if !candidate.exists() {
            continue;
        }
        let canonical = candidate.canonicalize().with_context(|| {
            format!(
                "failed to canonicalize native corelib path {}",
                candidate.display()
            )
        })?;
        let canonical_key = normalize_fingerprint_path(&canonical);
        if !seen.insert(canonical_key) {
            continue;
        }
        attempted.push(canonical.display().to_string());
        if native_corelib_layout_looks_compatible(&canonical)
            && native_corelib_version_matches_compiler(&canonical)
        {
            tracing::debug!(
                source,
                corelib_src = %canonical.display(),
                "selected native corelib source path"
            );
            return Ok(canonical);
        }
        tracing::debug!(
            source,
            corelib_src = %canonical.display(),
            "skipping incompatible native corelib candidate"
        );
    }

    Err(native_fallback_eligible_error(
        format!(
            "native compile requires a Cairo corelib source path compatible with cairo-lang {}; set UC_NATIVE_CORELIB_SRC=<.../corelib/src> (tried {} candidates)",
            native_cairo_lang_compiler_version(),
            attempted.len()
        ),
    ))
}

#[cfg(feature = "native-compile")]
fn native_manifest_package_name(manifest: &TomlValue) -> Option<String> {
    manifest
        .get("package")
        .and_then(TomlValue::as_table)
        .and_then(|table| table.get("name"))
        .and_then(TomlValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(feature = "native-compile")]
fn resolve_native_effective_manifest_path(
    manifest_path: &Path,
    workspace_root: &Path,
    requested_package: Option<&str>,
) -> Result<PathBuf> {
    let canonical_workspace_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let manifest_text = read_text_file_with_limit(manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
    let manifest = parse_manifest_toml(
        &manifest_text,
        manifest_path,
        "failed to parse manifest for native compile resolution",
    )?;
    if native_manifest_package_name(&manifest).is_some() {
        return Ok(manifest_path.to_path_buf());
    }

    let Some(workspace_table) = manifest.get("workspace").and_then(TomlValue::as_table) else {
        return Err(native_fallback_eligible_error(
            "native compile requires [package] or [workspace] in Scarb.toml",
        ));
    };
    let Some(members) = workspace_table.get("members").and_then(TomlValue::as_array) else {
        return Err(native_fallback_eligible_error(
            "native compile requires [package] or [workspace].members in Scarb.toml",
        ));
    };

    let mut candidates = Vec::<(String, PathBuf)>::new();
    for member in members {
        let Some(member_path) = member
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if member_path.contains('*') {
            return Err(native_fallback_eligible_error(
                "native compile does not support globbed [workspace].members yet",
            ));
        }
        let member_path = Path::new(member_path);
        if member_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            continue;
        }
        let candidate_manifest = workspace_root.join(member_path).join("Scarb.toml");
        let canonical_candidate_manifest = match candidate_manifest.canonicalize() {
            Ok(path) => path,
            Err(_) => continue,
        };
        if ensure_path_within_root(
            &canonical_workspace_root,
            &canonical_candidate_manifest,
            "native workspace member manifest",
        )
        .is_err()
        {
            continue;
        }
        if !canonical_candidate_manifest.is_file() {
            continue;
        }
        let candidate_text = read_text_file_with_limit(
            &canonical_candidate_manifest,
            MAX_MANIFEST_BYTES,
            "manifest",
        )?;
        let candidate_manifest_value = parse_manifest_toml(
            &candidate_text,
            &canonical_candidate_manifest,
            "failed to parse workspace member manifest for native compile",
        )?;
        let Some(package_name) = native_manifest_package_name(&candidate_manifest_value) else {
            continue;
        };
        candidates.push((package_name, canonical_candidate_manifest));
    }

    if candidates.is_empty() {
        return Err(native_fallback_eligible_error(
            "native compile could not resolve a package-bearing workspace member from [workspace].members",
        ));
    }

    let selected_manifest = if let Some(requested_package) = requested_package {
        let requested = requested_package.trim();
        candidates
            .iter()
            .find(|(package_name, _)| package_name == requested)
            .map(|(_, manifest)| manifest.clone())
            .ok_or_else(|| {
                native_fallback_eligible_error(format!(
                    "native compile could not find workspace member package `{requested}`"
                ))
            })?
    } else if candidates.len() == 1 {
        candidates[0].1.clone()
    } else {
        let package_names = candidates
            .iter()
            .map(|(package_name, _)| package_name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(native_fallback_eligible_error(format!(
            "native compile requires --package when workspace has multiple package members ({package_names})"
        )));
    };
    tracing::debug!(
        requested_manifest = %manifest_path.display(),
        selected_manifest = %selected_manifest.display(),
        "native compile resolved workspace-root manifest to member manifest"
    );
    Ok(selected_manifest)
}

#[cfg(feature = "native-compile")]
fn build_native_compile_context(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    workspace_root: &Path,
) -> Result<NativeCompileContext> {
    if !common.features.is_empty() {
        return Err(native_fallback_eligible_error(
            "native compile does not support --features yet",
        ));
    }

    let effective_manifest_path = resolve_native_effective_manifest_path(
        manifest_path,
        workspace_root,
        common.package.as_deref(),
    )?;
    let manifest_metadata = fs::metadata(&effective_manifest_path)
        .with_context(|| format!("failed to stat {}", effective_manifest_path.display()))?;
    let manifest_size_bytes = manifest_metadata.len();
    let manifest_modified_unix_ms = metadata_modified_unix_ms(&manifest_metadata)?;
    let manifest_change_unix_ms = metadata_change_unix_ms(&manifest_metadata);
    let workspace_manifest_path = workspace_root.join("Scarb.toml");
    let workspace_manifest_stats = if normalize_fingerprint_path(&workspace_manifest_path)
        == normalize_fingerprint_path(&effective_manifest_path)
        || !workspace_manifest_path.is_file()
    {
        None
    } else {
        let workspace_manifest_metadata = fs::metadata(&workspace_manifest_path)
            .with_context(|| format!("failed to stat {}", workspace_manifest_path.display()))?;
        Some((
            workspace_manifest_metadata.len(),
            metadata_modified_unix_ms(&workspace_manifest_metadata)?,
            metadata_change_unix_ms(&workspace_manifest_metadata),
        ))
    };
    let corelib_override = normalized_env_var("UC_NATIVE_CORELIB_SRC");
    let home_dir = normalized_env_var("HOME");
    let cache_key = native_compile_context_cache_key(
        &effective_manifest_path,
        workspace_root,
        corelib_override.as_deref(),
        home_dir.as_deref(),
    );
    let cache_now_ms = epoch_ms_u64().unwrap_or_default();
    {
        let mut cache = native_compile_context_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        evict_expired_native_compile_context_cache_entries(
            &mut cache,
            cache_now_ms,
            native_compile_context_cache_ttl_ms(),
        );
        if let Some(entry) = cache.get_mut(&cache_key) {
            entry.last_access_epoch_ms = cache_now_ms;
            let cairo_project_path = entry.context.cairo_project_dir.join("cairo_project.toml");
            if entry.manifest_size_bytes == manifest_size_bytes
                && entry.manifest_modified_unix_ms == manifest_modified_unix_ms
                && entry.manifest_change_unix_ms == manifest_change_unix_ms
                && entry.workspace_manifest_size_bytes
                    == workspace_manifest_stats.map(|stats| stats.0)
                && entry.workspace_manifest_modified_unix_ms
                    == workspace_manifest_stats.map(|stats| stats.1)
                && entry.workspace_manifest_change_unix_ms
                    == workspace_manifest_stats.and_then(|stats| stats.2)
                && cairo_project_path.is_file()
                && native_corelib_layout_looks_compatible(&entry.context.corelib_src)
            {
                validate_native_requested_package(
                    common.package.as_deref(),
                    &entry.context.package_name,
                )?;
                validate_native_workspace_mode(common.workspace, &entry.context)?;
                return Ok(entry.context.clone());
            }
        }
    }

    let context = build_native_compile_context_uncached(
        &effective_manifest_path,
        workspace_root,
        common.offline,
    )?;
    validate_native_requested_package(common.package.as_deref(), &context.package_name)?;
    validate_native_workspace_mode(common.workspace, &context)?;
    {
        let mut cache = native_compile_context_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.insert(
            cache_key,
            NativeCompileContextCacheEntry {
                manifest_size_bytes,
                manifest_modified_unix_ms,
                manifest_change_unix_ms,
                workspace_manifest_size_bytes: workspace_manifest_stats.map(|stats| stats.0),
                workspace_manifest_modified_unix_ms: workspace_manifest_stats.map(|stats| stats.1),
                workspace_manifest_change_unix_ms: workspace_manifest_stats
                    .and_then(|stats| stats.2),
                context: context.clone(),
                last_access_epoch_ms: cache_now_ms,
                estimated_bytes: native_compile_context_estimated_bytes(&context),
            },
        );
        evict_expired_native_compile_context_cache_entries(
            &mut cache,
            cache_now_ms,
            native_compile_context_cache_ttl_ms(),
        );
        evict_oldest_native_compile_context_cache_entries(
            &mut cache,
            native_compile_context_cache_max_entries(),
            native_compile_context_cache_max_bytes(),
        );
    }
    Ok(context)
}

#[cfg(feature = "native-compile")]
fn native_compile_preflight(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    workspace_root: &Path,
) -> Result<()> {
    build_native_compile_context(common, manifest_path, workspace_root).map(|_| ())
}

#[cfg(not(feature = "native-compile"))]
fn native_compile_preflight(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    workspace_root: &Path,
) -> Result<()> {
    let _ = (common, manifest_path, workspace_root);
    Err(native_fallback_eligible_error(
        "native compile backend is disabled at build time; rebuild uc with `native-compile` feature",
    ))
}

#[cfg(feature = "native-compile")]
fn normalized_env_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(feature = "native-compile")]
fn validate_native_requested_package(
    requested_package: Option<&str>,
    package_name: &str,
) -> Result<()> {
    if let Some(requested_package) = requested_package {
        if requested_package != package_name {
            return Err(native_fallback_eligible_error(format!(
                "native compile only supports the manifest package `{}` (got --package `{}`)",
                package_name, requested_package
            )));
        }
    }
    Ok(())
}

#[cfg(feature = "native-compile")]
fn native_manifest_workspace_mode_supported(manifest: &TomlValue) -> bool {
    if manifest
        .get("package")
        .and_then(TomlValue::as_table)
        .is_none()
    {
        return false;
    }
    let Some(workspace_table) = manifest.get("workspace").and_then(TomlValue::as_table) else {
        return false;
    };
    match workspace_table.get("members") {
        None => true,
        Some(members_value) => {
            let Some(members) = members_value.as_array() else {
                return false;
            };
            !members.is_empty()
                && members.iter().all(|member| {
                    member
                        .as_str()
                        .map(str::trim)
                        .is_some_and(|entry| matches!(entry, "." | "./"))
                })
        }
    }
}

#[cfg(feature = "native-compile")]
fn validate_native_workspace_mode(
    workspace_requested: bool,
    context: &NativeCompileContext,
) -> Result<()> {
    if workspace_requested && !context.workspace_mode_supported {
        return Err(native_fallback_eligible_error(
            "native compile does not support --workspace for multi-package or non-workspace manifests yet",
        ));
    }
    Ok(())
}

#[cfg(feature = "native-compile")]
fn native_workspace_dependency_entry_from_manifest<'a>(
    manifest: &'a TomlValue,
    dependency_name: &str,
) -> Option<&'a TomlValue> {
    manifest
        .get("workspace")
        .and_then(TomlValue::as_table)
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(TomlValue::as_table)
        .and_then(|deps| deps.get(dependency_name))
}

#[cfg(feature = "native-compile")]
fn native_workspace_dependency_entry<'a>(
    manifest: &'a TomlValue,
    workspace_manifest_fallback: Option<&'a TomlValue>,
    dependency_name: &str,
) -> Option<&'a TomlValue> {
    native_workspace_dependency_entry_from_manifest(manifest, dependency_name).or_else(|| {
        workspace_manifest_fallback.and_then(|fallback| {
            native_workspace_dependency_entry_from_manifest(fallback, dependency_name)
        })
    })
}

#[cfg(feature = "native-compile")]
fn native_dependency_path_from_value(
    manifest: &TomlValue,
    workspace_manifest_fallback: Option<&TomlValue>,
    dependency_name: &str,
    dependency_value: &TomlValue,
) -> Option<(String, bool)> {
    let table = dependency_value.as_table()?;
    if let Some(path) = table.get("path").and_then(TomlValue::as_str) {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Some((trimmed.to_string(), false));
        }
    }
    if table
        .get("workspace")
        .and_then(TomlValue::as_bool)
        .is_some_and(|value| value)
    {
        let workspace_entry = native_workspace_dependency_entry(
            manifest,
            workspace_manifest_fallback,
            dependency_name,
        )?;
        if let Some(path) = workspace_entry
            .as_table()
            .and_then(|entry| entry.get("path"))
            .and_then(TomlValue::as_str)
        {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                return Some((trimmed.to_string(), true));
            }
        }
    }
    None
}

#[cfg(feature = "native-compile")]
struct NativeDependencyCollectionContext<'a> {
    dependency_label_prefix: &'a str,
    manifest: &'a TomlValue,
    workspace_manifest_fallback: Option<&'a TomlValue>,
    manifest_dir: &'a Path,
    workspace_root: &'a Path,
}

#[cfg(feature = "native-compile")]
fn collect_native_dependency_table_surface(
    context: &NativeDependencyCollectionContext<'_>,
    section_label: &str,
    table: &toml::map::Map<String, TomlValue>,
    external: &mut BTreeSet<String>,
    path_roots: &mut BTreeMap<String, PathBuf>,
) {
    for (dependency_name, dependency_value) in table {
        // `starknet` is provided by the compiler/plugin suite directly; every
        // other dependency must resolve to a supported local path source.
        if dependency_name == "starknet" {
            continue;
        }
        let dependency_label = format!(
            "{}{section_label}.{dependency_name}",
            context.dependency_label_prefix
        );
        let Some((raw_path, workspace_relative)) = native_dependency_path_from_value(
            context.manifest,
            context.workspace_manifest_fallback,
            dependency_name,
            dependency_value,
        ) else {
            external.insert(dependency_label);
            continue;
        };

        let dependency_root = if workspace_relative {
            context.workspace_root.join(&raw_path)
        } else {
            context.manifest_dir.join(&raw_path)
        };
        let dependency_root = dependency_root.canonicalize().unwrap_or(dependency_root);
        if ensure_path_within_root(
            context.workspace_root,
            &dependency_root,
            "native path dependency root",
        )
        .is_err()
        {
            external.insert(dependency_label);
            continue;
        }

        let dependency_source_root = dependency_root.join("src");
        let dependency_lib_path = dependency_source_root.join("lib.cairo");
        if !dependency_lib_path.is_file() {
            external.insert(dependency_label);
            continue;
        }
        let dependency_source_root = match dependency_source_root.canonicalize() {
            Ok(path) => path,
            Err(_) => {
                external.insert(dependency_label);
                continue;
            }
        };

        let dependency_crate_name = normalize_package_name_for_cairo_crate(dependency_name);
        match path_roots.entry(dependency_crate_name) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(dependency_source_root);
            }
            std::collections::btree_map::Entry::Occupied(existing)
                if existing.get() == &dependency_source_root => {}
            std::collections::btree_map::Entry::Occupied(_) => {
                external.insert(dependency_label);
            }
        }
    }
}

#[cfg(feature = "native-compile")]
fn collect_native_manifest_dependency_surface(
    manifest: &TomlValue,
    workspace_manifest_fallback: Option<&TomlValue>,
    manifest_dir: &Path,
    workspace_root: &Path,
    dependency_label_prefix: &str,
) -> (BTreeSet<String>, BTreeMap<String, PathBuf>) {
    let mut external = BTreeSet::new();
    let mut path_roots = BTreeMap::new();
    let context = NativeDependencyCollectionContext {
        dependency_label_prefix,
        manifest,
        workspace_manifest_fallback,
        manifest_dir,
        workspace_root,
    };

    for section_name in ["dependencies"] {
        if let Some(table) = manifest.get(section_name).and_then(TomlValue::as_table) {
            let section_label = format!("[{}]", section_name);
            collect_native_dependency_table_surface(
                &context,
                &section_label,
                table,
                &mut external,
                &mut path_roots,
            );
        }
    }

    if let Some(target_table) = manifest.get("target").and_then(TomlValue::as_table) {
        for (target_name, target_section) in target_table {
            let Some(target_section_table) = target_section.as_table() else {
                continue;
            };
            for section_name in ["dependencies"] {
                if let Some(table) = target_section_table
                    .get(section_name)
                    .and_then(TomlValue::as_table)
                {
                    let section_label = format!("[target.{}.{}]", target_name, section_name);
                    collect_native_dependency_table_surface(
                        &context,
                        &section_label,
                        table,
                        &mut external,
                        &mut path_roots,
                    );
                }
            }
        }
    }
    (external, path_roots)
}

#[cfg(feature = "native-compile")]
fn native_dependency_manifest_path(source_root: &Path) -> PathBuf {
    source_root
        .parent()
        .unwrap_or(source_root)
        .join("Scarb.toml")
}

#[cfg(feature = "native-compile")]
fn parse_native_scarb_metadata_document(stdout: &str) -> Result<NativeScarbMetadataDocument> {
    let trimmed = stdout.trim_start();
    if let Ok(metadata) = serde_json::from_str::<NativeScarbMetadataDocument>(trimmed) {
        return Ok(metadata);
    }

    // Some Scarb versions emit progress lines before JSON when fetching dependencies.
    if let Some(json_start) = trimmed.find('{') {
        let candidate = &trimmed[json_start..];
        if let Ok(metadata) = serde_json::from_str::<NativeScarbMetadataDocument>(candidate) {
            return Ok(metadata);
        }
    }

    bail!("failed to decode scarb metadata JSON payload from command output");
}

#[cfg(feature = "native-compile")]
fn collect_native_dependency_surface_from_scarb_metadata(
    manifest_path: &Path,
    root_package_name: &str,
    offline: bool,
) -> Result<Option<NativeDependencySurface>> {
    let metadata_args = MetadataArgs {
        manifest_path: Some(manifest_path.to_path_buf()),
        format_version: 1,
        daemon_mode: DaemonModeArg::Off,
        offline,
        global_cache_dir: None,
        report_path: None,
    };
    let run = match run_scarb_metadata_with_uc_cache(&metadata_args, manifest_path, true) {
        Ok(run) => run,
        Err(err) => {
            tracing::debug!(
                manifest_path = %manifest_path.display(),
                error = %format!("{err:#}"),
                "native dependency metadata resolution skipped: failed to resolve scarb metadata"
            );
            return Ok(None);
        }
    };
    if run.exit_code != 0 {
        tracing::debug!(
            manifest_path = %manifest_path.display(),
            exit_code = run.exit_code,
            stderr = %run.stderr.trim(),
            "native dependency metadata resolution skipped: scarb metadata failed"
        );
        return Ok(None);
    }

    let metadata: NativeScarbMetadataDocument = match parse_native_scarb_metadata_document(
        &run.stdout,
    ) {
        Ok(metadata) => metadata,
        Err(err) => {
            tracing::debug!(
                manifest_path = %manifest_path.display(),
                error = %err,
                "native dependency metadata resolution skipped: failed to decode scarb metadata JSON"
            );
            return Ok(None);
        }
    };

    let package_by_id = metadata
        .packages
        .iter()
        .map(|package| (package.id.as_str(), package))
        .collect::<HashMap<_, _>>();
    let root_manifest_key = normalize_fingerprint_path(manifest_path);
    let root_crate_name = normalize_package_name_for_cairo_crate(root_package_name);

    let matching_component =
        |component: &NativeScarbMetadataComponentData,
         unit: &NativeScarbMetadataCompilationUnit| {
            if normalize_package_name_for_cairo_crate(&component.name) == root_crate_name {
                return true;
            }
            if unit.package == component.id {
                return true;
            }
            package_by_id
                .get(component.id.as_str())
                .is_some_and(|package| {
                    normalize_fingerprint_path(Path::new(&package.manifest_path))
                        == root_manifest_key
                })
        };

    let selected_unit = metadata
        .compilation_units
        .iter()
        .find(|unit| {
            unit.target.kind == "starknet-contract"
                && unit
                    .components_data
                    .iter()
                    .any(|component| matching_component(component, unit))
        })
        .or_else(|| {
            metadata.compilation_units.iter().find(|unit| {
                unit.components_data
                    .iter()
                    .any(|component| matching_component(component, unit))
            })
        });
    let Some(selected_unit) = selected_unit else {
        return Ok(None);
    };
    let component_by_id = selected_unit
        .components_data
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<HashMap<_, _>>();
    let root_component_ids = selected_unit
        .components_data
        .iter()
        .filter(|component| matching_component(component, selected_unit))
        .map(|component| component.id.clone())
        .collect::<Vec<_>>();
    if root_component_ids.is_empty() {
        return Ok(None);
    }

    let mut queue = root_component_ids.into_iter().collect::<VecDeque<_>>();
    let mut visited = HashSet::<String>::new();
    let mut crate_roots = BTreeMap::<String, PathBuf>::new();
    let mut crate_dependencies = BTreeMap::<String, BTreeSet<String>>::new();
    let mut crate_editions = BTreeMap::<String, Option<String>>::new();
    while let Some(component_id) = queue.pop_front() {
        if !visited.insert(component_id.clone()) {
            continue;
        }
        let Some(component) = component_by_id.get(component_id.as_str()) else {
            continue;
        };
        let source_path = PathBuf::from(&component.source_path);
        let source_root = source_path.parent().unwrap_or(&source_path).to_path_buf();
        if !source_root.join("lib.cairo").is_file() {
            continue;
        }
        let crate_name = normalize_package_name_for_cairo_crate(&component.name);
        if crate_name == "core" {
            continue;
        }
        crate_roots
            .entry(crate_name.clone())
            .or_insert_with(|| source_root.clone());
        if let Some(package) = package_by_id.get(component.id.as_str()) {
            crate_editions
                .entry(crate_name.clone())
                .or_insert_with(|| package.edition.clone());
        }
        let dependency_names = crate_dependencies.entry(crate_name.clone()).or_default();
        for dependency in &component.dependencies {
            queue.push_back(dependency.id.clone());
            let Some(dependency_component) = component_by_id.get(dependency.id.as_str()) else {
                continue;
            };
            let dependency_name =
                normalize_package_name_for_cairo_crate(&dependency_component.name);
            if dependency_name != "core" {
                dependency_names.insert(dependency_name);
            }
        }
    }

    if !crate_roots.contains_key(&root_crate_name) {
        return Ok(None);
    }
    let known_crates = crate_roots.keys().cloned().collect::<HashSet<_>>();
    for dependencies in crate_dependencies.values_mut() {
        dependencies.retain(|dependency| known_crates.contains(dependency));
    }
    let path_dependency_roots = crate_roots
        .iter()
        .filter(|(crate_name, _)| **crate_name != root_crate_name)
        .map(|(crate_name, source_root)| NativePathDependencyRoot {
            crate_name: crate_name.clone(),
            source_root: source_root.clone(),
        })
        .collect::<Vec<_>>();
    let crate_dependency_configs = crate_roots
        .keys()
        .map(|crate_name| NativeCrateDependencyConfig {
            crate_name: crate_name.clone(),
            cairo_edition: crate_editions.get(crate_name).cloned().unwrap_or_default(),
            dependencies: crate_dependencies
                .get(crate_name)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect(),
        })
        .collect::<Vec<_>>();
    Ok(Some(NativeDependencySurface {
        external_non_starknet_dependencies: Vec::new(),
        path_dependency_roots,
        crate_dependency_configs,
    }))
}

#[cfg(feature = "native-compile")]
fn collect_native_dependency_surface(
    manifest: &TomlValue,
    workspace_manifest_fallback: Option<&TomlValue>,
    manifest_path: &Path,
    workspace_root: &Path,
    root_package_name: &str,
    offline: bool,
) -> NativeDependencySurface {
    let canonical_workspace_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let manifest_dir = manifest_path.parent().unwrap_or(&canonical_workspace_root);
    let workspace_manifest_fallback = workspace_manifest_fallback.or(Some(manifest));
    let (manifest_external, mut path_roots) = collect_native_manifest_dependency_surface(
        manifest,
        workspace_manifest_fallback,
        manifest_dir,
        &canonical_workspace_root,
        "",
    );
    let use_dependency_metadata = native_should_use_dependency_metadata_with_flags(
        native_dependency_metadata_enabled(),
        path_roots.len(),
        manifest_external.len(),
    );
    if use_dependency_metadata {
        match collect_native_dependency_surface_from_scarb_metadata(
            manifest_path,
            root_package_name,
            offline,
        ) {
            Ok(Some(mut surface)) => {
                if !manifest_external.is_empty() {
                    let mut external = manifest_external.iter().cloned().collect::<Vec<_>>();
                    external.sort();
                    external.dedup();
                    surface.external_non_starknet_dependencies = external;
                }
                tracing::debug!(
                    manifest_path = %manifest_path.display(),
                    dependencies = surface.path_dependency_roots.len(),
                    "native dependency surface resolved from scarb metadata"
                );
                return surface;
            }
            Ok(None) => {}
            Err(err) => {
                tracing::debug!(
                    manifest_path = %manifest_path.display(),
                    error = %format!("{err:#}"),
                    "native dependency metadata resolution failed; falling back to manifest-only dependency discovery"
                );
            }
        }
    } else {
        tracing::debug!(
            manifest_path = %manifest_path.display(),
            metadata_enabled = native_dependency_metadata_enabled(),
            dependency_count = path_roots.len(),
            external_count = manifest_external.len(),
            "native dependency surface using manifest-only discovery"
        );
    }

    let mut external = manifest_external;
    let root_crate_name = normalize_package_name_for_cairo_crate(root_package_name);
    let (root_cairo_edition, _) = resolve_manifest_cairo_settings_from_manifest(manifest);
    let mut crate_dependency_configs =
        BTreeMap::<String, (Option<String>, BTreeSet<String>)>::new();
    crate_dependency_configs.insert(
        root_crate_name,
        (root_cairo_edition, path_roots.keys().cloned().collect()),
    );

    let mut dependency_queue: VecDeque<(String, PathBuf)> = path_roots
        .iter()
        .map(|(crate_name, source_root)| (crate_name.clone(), source_root.clone()))
        .collect();
    let mut visited_dependency_manifests = HashSet::new();
    while let Some((dependency_crate_name, dependency_source_root)) = dependency_queue.pop_front() {
        let dependency_manifest_path = native_dependency_manifest_path(&dependency_source_root);
        let dependency_manifest_key = normalize_fingerprint_path(&dependency_manifest_path);
        if !visited_dependency_manifests.insert(dependency_manifest_key) {
            continue;
        }
        let dependency_manifest_text = match read_text_file_with_limit(
            &dependency_manifest_path,
            MAX_MANIFEST_BYTES,
            "manifest",
        ) {
            Ok(text) => text,
            Err(err) => {
                tracing::debug!(
                    dependency = %dependency_crate_name,
                    manifest_path = %dependency_manifest_path.display(),
                    error = %err,
                    "skipping native dependency manifest parsing due to read failure"
                );
                continue;
            }
        };
        let dependency_manifest = match parse_manifest_toml(
            &dependency_manifest_text,
            &dependency_manifest_path,
            "failed to parse path dependency manifest for native compile",
        ) {
            Ok(manifest) => manifest,
            Err(err) => {
                tracing::debug!(
                    dependency = %dependency_crate_name,
                    manifest_path = %dependency_manifest_path.display(),
                    error = %err,
                    "skipping native dependency manifest parsing due to parse failure"
                );
                continue;
            }
        };
        if let Err(err) = validate_manifest_dependency_sanity_from_manifest(
            &dependency_manifest_path,
            &dependency_manifest,
        ) {
            tracing::debug!(
                dependency = %dependency_crate_name,
                manifest_path = %dependency_manifest_path.display(),
                error = %err,
                "path dependency manifest has invalid dependency table entries"
            );
            continue;
        }
        let (dependency_cairo_edition, _) =
            resolve_manifest_cairo_settings_from_manifest(&dependency_manifest);
        let dependency_manifest_dir = dependency_manifest_path
            .parent()
            .unwrap_or(&canonical_workspace_root);
        let dependency_label_prefix = format!("[dependency.{dependency_crate_name}]");
        let (dependency_external, dependency_path_roots) =
            collect_native_manifest_dependency_surface(
                &dependency_manifest,
                workspace_manifest_fallback,
                dependency_manifest_dir,
                &canonical_workspace_root,
                &dependency_label_prefix,
            );
        external.extend(dependency_external);

        let dependency_entry = crate_dependency_configs
            .entry(dependency_crate_name.clone())
            .or_insert_with(|| (dependency_cairo_edition.clone(), BTreeSet::new()));
        if dependency_entry.0.is_none() {
            dependency_entry.0 = dependency_cairo_edition;
        }
        dependency_entry
            .1
            .extend(dependency_path_roots.keys().cloned());

        for (nested_crate_name, nested_source_root) in dependency_path_roots {
            match path_roots.entry(nested_crate_name.clone()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(nested_source_root.clone());
                    dependency_queue.push_back((nested_crate_name, nested_source_root));
                }
                std::collections::btree_map::Entry::Occupied(existing)
                    if existing.get() == &nested_source_root => {}
                std::collections::btree_map::Entry::Occupied(existing) => {
                    external.insert(format!(
                        "[dependency.{dependency_crate_name}] conflicting path roots for `{nested_crate_name}` ({} vs {})",
                        existing.get().display(),
                        nested_source_root.display()
                    ));
                }
            }
        }
    }

    let path_dependency_roots = path_roots
        .into_iter()
        .map(|(crate_name, source_root)| NativePathDependencyRoot {
            crate_name,
            source_root,
        })
        .collect();
    let crate_dependency_configs = crate_dependency_configs
        .into_iter()
        .map(
            |(crate_name, (cairo_edition, dependencies))| NativeCrateDependencyConfig {
                crate_name,
                cairo_edition,
                dependencies: dependencies.into_iter().collect(),
            },
        )
        .collect();
    NativeDependencySurface {
        external_non_starknet_dependencies: external.into_iter().collect(),
        path_dependency_roots,
        crate_dependency_configs,
    }
}

#[cfg(feature = "native-compile")]
fn native_should_use_dependency_metadata_with_flags(
    metadata_enabled: bool,
    path_dependency_count: usize,
    external_dependency_count: usize,
) -> bool {
    if external_dependency_count != 0 {
        // Registry/non-path dependencies require metadata graph resolution to wire
        // crate roots for native compile coverage.
        return true;
    }
    metadata_enabled && path_dependency_count != 0
}

#[cfg(feature = "native-compile")]
fn build_native_compile_context_uncached(
    manifest_path: &Path,
    workspace_root: &Path,
    offline: bool,
) -> Result<NativeCompileContext> {
    let manifest_text = read_text_file_with_limit(manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
    let manifest = parse_manifest_toml(
        &manifest_text,
        manifest_path,
        "failed to parse manifest for native compile",
    )?;
    ensure_native_manifest_cairo_version_supported(&manifest)?;
    let manifest_content_hash = compute_manifest_content_hash_bytes(manifest_text.as_bytes());
    validate_manifest_dependency_sanity_from_manifest(manifest_path, &manifest)?;
    let starknet_target = resolve_manifest_native_starknet_target_props(&manifest)
        .map_err(mark_native_fallback_eligible)?;
    let package_table = manifest
        .get("package")
        .and_then(TomlValue::as_table)
        .context("native compile requires [package] section in Scarb.toml")?;
    let package_name = package_table
        .get("name")
        .and_then(TomlValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .context("native compile requires [package].name in Scarb.toml")?;
    let workspace_mode_supported = native_manifest_workspace_mode_supported(&manifest);
    let workspace_manifest_fallback = {
        let workspace_manifest_path = workspace_root.join("Scarb.toml");
        if normalize_fingerprint_path(&workspace_manifest_path)
            == normalize_fingerprint_path(manifest_path)
        {
            None
        } else if workspace_manifest_path.is_file() {
            let workspace_manifest_text = read_text_file_with_limit(
                &workspace_manifest_path,
                MAX_MANIFEST_BYTES,
                "workspace manifest",
            )?;
            Some(parse_manifest_toml(
                &workspace_manifest_text,
                &workspace_manifest_path,
                "failed to parse workspace manifest for native dependency resolution",
            )?)
        } else {
            None
        }
    };
    let dependency_surface = collect_native_dependency_surface(
        &manifest,
        workspace_manifest_fallback.as_ref(),
        manifest_path,
        workspace_root,
        &package_name,
        offline,
    );
    let external_non_starknet_dependencies = dependency_surface.external_non_starknet_dependencies;
    let path_dependency_roots = dependency_surface.path_dependency_roots;
    let crate_dependency_configs = dependency_surface.crate_dependency_configs;
    if !path_dependency_roots.is_empty() {
        tracing::debug!(
            roots = ?path_dependency_roots
                .iter()
                .map(|root| (root.crate_name.as_str(), root.source_root.display().to_string()))
                .collect::<Vec<_>>(),
            "native compile detected local path dependencies; wiring crate roots in cairo_project.toml"
        );
    }
    if !external_non_starknet_dependencies.is_empty() {
        tracing::debug!(
            dependencies = ?external_non_starknet_dependencies,
            "native compile detected external non-starknet dependencies; scarb fallback remains eligible on native compile failures"
        );
    }

    if let Some(lib_table) = manifest.get("lib").and_then(TomlValue::as_table) {
        if let Some(path) = lib_table.get("path").and_then(TomlValue::as_str) {
            if path.trim() != "src/lib.cairo" {
                return Err(native_fallback_eligible_error(
                    "native compile only supports [lib].path = \"src/lib.cairo\"",
                ));
            }
        }
    }
    if !starknet_target.sierra {
        return Err(native_fallback_eligible_error(
            "native compile currently requires [target.starknet-contract].sierra = true",
        ));
    }

    let manifest_dir = manifest_path
        .parent()
        .unwrap_or(workspace_root)
        .to_path_buf();
    let source_root = manifest_dir.join("src");
    let lib_path = source_root.join("lib.cairo");
    if !lib_path.is_file() {
        return Err(native_fallback_eligible_error(format!(
            "native compile expects src/lib.cairo (missing at {})",
            lib_path.display()
        )));
    }

    let crate_name = normalize_package_name_for_cairo_crate(&package_name);
    let cairo_project_dir = workspace_root.join(".uc/native-project");
    ensure_path_within_root(
        workspace_root,
        &cairo_project_dir,
        "native project directory",
    )?;
    fs::create_dir_all(&cairo_project_dir).with_context(|| {
        format!(
            "failed to create native project directory {}",
            cairo_project_dir.display()
        )
    })?;
    let cairo_project_path = cairo_project_dir.join("cairo_project.toml");
    let mut crate_roots = BTreeMap::new();
    crate_roots.insert(crate_name.clone(), source_root.clone());
    for root in &path_dependency_roots {
        crate_roots
            .entry(root.crate_name.clone())
            .or_insert_with(|| root.source_root.clone());
    }
    let escaped_crate_roots = crate_roots
        .iter()
        .map(|(root_name, root_path)| {
            (
                root_name.clone(),
                toml_escape_basic_string(&normalize_fingerprint_path(root_path)),
            )
        })
        .collect::<Vec<_>>();
    let (cairo_edition, _) = resolve_manifest_cairo_settings_from_manifest(&manifest);
    let cairo_project_toml = native_cairo_project_toml(
        &escaped_crate_roots,
        &crate_dependency_configs,
        cairo_edition.as_deref(),
    );
    write_text_file_if_changed(
        &cairo_project_path,
        &cairo_project_toml,
        "native cairo project",
    )?;

    let corelib_src =
        resolve_native_corelib_src(workspace_root).map_err(mark_native_fallback_eligible)?;

    Ok(NativeCompileContext {
        package_name,
        crate_name,
        main_source_root: source_root,
        workspace_mode_supported,
        cairo_project_dir,
        corelib_src,
        starknet_target,
        manifest_content_hash,
        external_non_starknet_dependencies,
        path_dependency_roots,
        crate_dependency_configs,
    })
}

#[cfg(feature = "native-compile")]
fn write_text_file_if_changed(path: &Path, contents: &str, label: &str) -> Result<()> {
    match fs::read_to_string(path) {
        Ok(existing) if existing == contents => return Ok(()),
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed to read {} {}", label, path.display()));
        }
    }
    fs::write(path, contents)
        .with_context(|| format!("failed to write {} {}", label, path.display()))?;
    Ok(())
}

#[cfg(feature = "native-compile")]
fn native_compile_context_estimated_bytes(context: &NativeCompileContext) -> u64 {
    let path_bytes = normalize_fingerprint_path(&context.cairo_project_dir).len()
        + normalize_fingerprint_path(&context.corelib_src).len()
        + normalize_fingerprint_path(&context.main_source_root).len()
        + context
            .path_dependency_roots
            .iter()
            .map(|root| normalize_fingerprint_path(&root.source_root).len())
            .sum::<usize>();
    let scalar_bytes = context.package_name.len()
        + context.crate_name.len()
        + context.manifest_content_hash.len()
        + context
            .external_non_starknet_dependencies
            .iter()
            .map(String::len)
            .sum::<usize>()
        + context
            .path_dependency_roots
            .iter()
            .map(|root| root.crate_name.len())
            .sum::<usize>()
        + context
            .crate_dependency_configs
            .iter()
            .map(|config| {
                config.crate_name.len()
                    + config.cairo_edition.as_ref().map_or(0, String::len)
                    + config.dependencies.iter().map(String::len).sum::<usize>()
            })
            .sum::<usize>()
        + path_bytes;
    u64::try_from(scalar_bytes).unwrap_or(u64::MAX)
}

#[cfg(feature = "native-compile")]
fn mark_native_fallback_eligible_for_external_dependencies(
    err: anyhow::Error,
    context: &NativeCompileContext,
) -> anyhow::Error {
    if context.external_non_starknet_dependencies.is_empty() {
        return err;
    }
    let deps = context.external_non_starknet_dependencies.join(", ");
    mark_native_fallback_eligible(err.context(format!(
        "native compile manifest includes non-starknet dependencies ({deps}); retrying with scarb fallback is allowed"
    )))
}

#[cfg(feature = "native-compile")]
fn native_compile_source_roots(context: &NativeCompileContext) -> Vec<PathBuf> {
    let mut roots = Vec::with_capacity(context.path_dependency_roots.len().saturating_add(1));
    roots.push(context.main_source_root.clone());
    roots.extend(
        context
            .path_dependency_roots
            .iter()
            .map(|root| root.source_root.clone()),
    );
    roots.sort_by_key(|path| normalize_fingerprint_path(path));
    roots.dedup_by(|left, right| {
        normalize_fingerprint_path(left) == normalize_fingerprint_path(right)
    });
    roots
}

#[cfg(feature = "native-compile")]
struct NativeTrackedSourceScanResult {
    tracked_sources: BTreeMap<String, NativeTrackedFileState>,
    tracked_source_bytes: u64,
    latest_source_root_modified_unix_ms: u64,
}

#[cfg(feature = "native-compile")]
fn native_scan_tracked_sources(
    workspace_root: &Path,
    source_roots: &[PathBuf],
) -> Result<NativeTrackedSourceScanResult> {
    let mut tracked_files = Vec::new();
    let mut tracked_source_bytes = 0_u64;
    let mut latest = 0_u64;
    for source_root in source_roots {
        let metadata = match fs::metadata(source_root) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to stat {}", source_root.display()));
            }
        };
        if !metadata.is_dir() {
            continue;
        }
        let root_modified_unix_ms = metadata_modified_unix_ms(&metadata)?;
        let root_change_unix_ms =
            metadata_change_unix_ms(&metadata).unwrap_or(root_modified_unix_ms);
        latest = latest.max(root_modified_unix_ms.max(root_change_unix_ms));
        let walker = WalkDir::new(source_root)
            .follow_links(false)
            .max_depth(MAX_FINGERPRINT_DEPTH)
            .into_iter()
            .filter_entry(|entry| !is_ignored_entry(workspace_root, entry.path()));
        for entry in walker.filter_map(|entry| entry.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let is_cairo = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("cairo"));
            if !is_cairo {
                continue;
            }
            let metadata = match fs::metadata(path) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => {
                    return Err(err).with_context(|| format!("failed to stat {}", path.display()));
                }
            };
            let modified_unix_ms = metadata_modified_unix_ms(&metadata)?;
            let change_unix_ms = metadata_change_unix_ms(&metadata).unwrap_or(modified_unix_ms);
            latest = latest.max(modified_unix_ms.max(change_unix_ms));
            if tracked_files.len() >= max_fingerprint_files() {
                bail!(
                    "native source tracker found too many files (>{}); refusing to continue",
                    max_fingerprint_files()
                );
            }
            let size_bytes = metadata.len();
            if size_bytes > max_fingerprint_file_bytes() {
                bail!(
                    "native source tracker file {} exceeds size limit ({} bytes > {} bytes)",
                    path.display(),
                    size_bytes,
                    max_fingerprint_file_bytes()
                );
            }
            tracked_source_bytes = tracked_source_bytes.saturating_add(size_bytes);
            if tracked_source_bytes > max_fingerprint_total_bytes() {
                bail!(
                    "native source tracker budget exceeded ({} bytes > {} bytes)",
                    tracked_source_bytes,
                    max_fingerprint_total_bytes()
                );
            }
            tracked_files.push((
                path.to_path_buf(),
                NativeTrackedFileState {
                    size_bytes,
                    modified_unix_ms,
                },
            ));
        }
    }
    tracked_files.sort_by(|(left, _), (right, _)| left.cmp(right));

    let mut tracked_sources = BTreeMap::new();
    for (path, state) in tracked_files {
        let relative = path
            .strip_prefix(workspace_root)
            .unwrap_or(&path)
            .to_path_buf();
        let relative = normalize_fingerprint_path(&relative);
        tracked_sources.insert(relative, state);
    }

    Ok(NativeTrackedSourceScanResult {
        tracked_sources,
        tracked_source_bytes,
        latest_source_root_modified_unix_ms: latest,
    })
}

#[cfg(feature = "native-compile")]
fn native_source_roots_modified_unix_ms(
    workspace_root: &Path,
    source_roots: &[PathBuf],
) -> Result<u64> {
    Ok(native_scan_tracked_sources(workspace_root, source_roots)?
        .latest_source_root_modified_unix_ms)
}

#[cfg(feature = "native-compile")]
fn native_collect_tracked_sources(
    workspace_root: &Path,
    source_roots: &[PathBuf],
) -> Result<(BTreeMap<String, NativeTrackedFileState>, u64)> {
    let scan = native_scan_tracked_sources(workspace_root, source_roots)?;
    Ok((scan.tracked_sources, scan.tracked_source_bytes))
}

#[cfg(feature = "native-compile")]
fn native_collect_tracked_sources_with_source_root_mtime(
    workspace_root: &Path,
    source_roots: &[PathBuf],
) -> Result<(BTreeMap<String, NativeTrackedFileState>, u64, u64)> {
    let scan = native_scan_tracked_sources(workspace_root, source_roots)?;
    Ok((
        scan.tracked_sources,
        scan.tracked_source_bytes,
        scan.latest_source_root_modified_unix_ms,
    ))
}

#[cfg(feature = "native-compile")]
fn native_diff_tracked_sources(
    previous: &BTreeMap<String, NativeTrackedFileState>,
    current: &BTreeMap<String, NativeTrackedFileState>,
) -> (Vec<String>, Vec<String>) {
    let mut changed = Vec::new();
    let mut removed = Vec::new();
    for (rel, state) in current {
        if previous.get(rel) != Some(state) {
            changed.push(rel.clone());
        }
    }
    for rel in previous.keys() {
        if !current.contains_key(rel) {
            removed.push(rel.clone());
        }
    }
    (changed, removed)
}

#[cfg(feature = "native-compile")]
fn native_compile_session_estimated_heap_bytes(tracked_source_bytes: u64) -> u64 {
    tracked_source_bytes
        .saturating_mul(native_compile_session_memory_multiplier())
        .saturating_add(native_compile_session_memory_base_overhead_bytes())
}

#[cfg(feature = "native-compile")]
fn native_compile_session_state_estimated_bytes(state: &NativeCompileSessionState) -> u64 {
    let tracked_meta_bytes = state.tracked_sources.len() as u64 * 96;
    let dependency_bytes =
        state
            .contract_source_dependencies
            .iter()
            .fold(0_u64, |acc, (module_path, deps)| {
                let module_bytes = module_path.len() as u64;
                let deps_bytes = deps
                    .iter()
                    .fold(0_u64, |inner, dep| inner.saturating_add(dep.len() as u64));
                acc.saturating_add(module_bytes.saturating_add(deps_bytes))
            });
    let plan_bytes = state.contract_output_plans.iter().fold(0_u64, |acc, plan| {
        acc.saturating_add(plan.module_path.len() as u64)
            .saturating_add(plan.artifact_id.len() as u64)
            .saturating_add(plan.package_name.len() as u64)
            .saturating_add(plan.contract_name.len() as u64)
            .saturating_add(plan.artifact_file.len() as u64)
            .saturating_add(
                plan.casm_file
                    .as_ref()
                    .map(|value| value.len() as u64)
                    .unwrap_or(0),
            )
    });
    // RootDatabase memory is dominated by interned/query state, not raw file bytes.
    // Use a conservative source-scaled heuristic so byte-based eviction is meaningful.
    native_compile_session_estimated_heap_bytes(state.tracked_source_bytes)
        .saturating_add(tracked_meta_bytes)
        .saturating_add(dependency_bytes)
        .saturating_add(plan_bytes)
}

#[cfg(feature = "native-compile")]
fn update_native_compile_session_cached_estimated_bytes(
    workspace_root: &Path,
    estimated_bytes: u64,
) {
    let cache_key = native_compile_session_cache_key(workspace_root);
    let now_ms = epoch_ms_u64().unwrap_or_default();
    let mut cache = native_compile_session_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    evict_expired_native_compile_session_cache_entries(
        &mut cache,
        now_ms,
        native_compile_session_cache_ttl_ms(),
    );
    if let Some(entry) = cache.get_mut(&cache_key) {
        entry.estimated_bytes = estimated_bytes;
        entry.last_access_epoch_ms = now_ms;
        evict_oldest_native_compile_session_cache_entries(
            &mut cache,
            native_compile_session_cache_max_entries(),
            native_compile_session_cache_max_bytes(),
        );
    }
}

#[cfg(feature = "native-compile")]
fn native_cairo_project_toml(
    crate_roots: &[(String, String)],
    crate_dependency_configs: &[NativeCrateDependencyConfig],
    cairo_edition: Option<&str>,
) -> String {
    let mut content = String::from("[crate_roots]\n");
    for (crate_name, escaped_source_root) in crate_roots {
        content.push_str(crate_name);
        content.push_str(" = \"");
        content.push_str(escaped_source_root);
        content.push_str("\"\n");
    }
    let normalized_global_edition = cairo_edition
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_CAIRO_EDITION);
    let escaped_edition = toml_escape_basic_string(normalized_global_edition);
    content.push_str("\n[config.global]\n");
    content.push_str("edition = \"");
    content.push_str(&escaped_edition);
    content.push_str("\"\n");
    for crate_config in crate_dependency_configs
        .iter()
        .filter(|config| !config.dependencies.is_empty())
    {
        content.push_str("\n[config.override.");
        content.push_str(&crate_config.crate_name);
        content.push_str("]\n");

        let effective_edition = crate_config
            .cairo_edition
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(normalized_global_edition);
        let escaped_edition = toml_escape_basic_string(effective_edition);
        content.push_str("edition = \"");
        content.push_str(&escaped_edition);
        content.push_str("\"\n");

        content.push_str("\n[config.override.");
        content.push_str(&crate_config.crate_name);
        content.push_str(".dependencies]\n");
        for dependency_name in &crate_config.dependencies {
            let escaped_dependency_name = toml_escape_basic_string(dependency_name);
            content.push_str(dependency_name);
            content.push_str(" = { discriminator = \"");
            content.push_str(&escaped_dependency_name);
            content.push_str("\" }\n");
        }
    }
    content
}

#[cfg(feature = "native-compile")]
fn native_contract_package_name(module_path: &str) -> &str {
    module_path
        .split_once("::")
        .map(|(package, _)| package)
        .unwrap_or(module_path)
}

#[cfg(feature = "native-compile")]
fn native_contract_name(module_path: &str) -> &str {
    module_path
        .rsplit_once("::")
        .map(|(_, contract)| contract)
        .unwrap_or(module_path)
}

#[cfg(feature = "native-compile")]
fn native_contract_file_stems(module_paths: &[String]) -> Vec<String> {
    let mut seen_names = HashSet::new();
    let duplicate_names: HashSet<String> = module_paths
        .iter()
        .map(|path| native_contract_name(path).to_string())
        .filter(|contract_name| !seen_names.insert(contract_name.clone()))
        .collect();
    let mut stems: Vec<String> = module_paths
        .iter()
        .map(|path| {
            let contract_name = native_contract_name(path);
            if duplicate_names.contains(contract_name) {
                path.replace("::", "_")
            } else {
                contract_name.to_string()
            }
        })
        .collect();
    let mut stem_counts = HashMap::new();
    for stem in &stems {
        *stem_counts.entry(stem.clone()).or_insert(0_usize) += 1;
    }
    let mut occupied = HashSet::new();
    for (index, stem) in stems.iter_mut().enumerate() {
        let original = stem.clone();
        if stem_counts.get(&original).copied().unwrap_or_default() <= 1
            && occupied.insert(original.clone())
        {
            continue;
        }
        let mut candidate = format!(
            "{}_{}",
            original,
            short_hash((module_paths[index].as_str(), "native-stem"))
        );
        let mut disambiguator = 0_u32;
        while !occupied.insert(candidate.clone()) {
            disambiguator = disambiguator.saturating_add(1);
            candidate = format!(
                "{}_{}_{}",
                original,
                short_hash((module_paths[index].as_str(), disambiguator)),
                disambiguator
            );
        }
        *stem = candidate;
    }
    stems
}

#[cfg(feature = "native-compile")]
fn native_starknet_artifact_id(package_name: &str, contract_path: &str) -> String {
    short_hash((package_name, contract_path))
}

#[cfg(feature = "native-compile")]
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct NativeContractOutputPlan {
    module_path: String,
    artifact_id: String,
    package_name: String,
    contract_name: String,
    artifact_file: String,
    casm_file: Option<String>,
}

#[cfg(feature = "native-compile")]
fn native_contract_source_relative_path(
    db: &RootDatabase,
    workspace_root: &Path,
    contract: &ContractDeclaration<'_>,
) -> Option<String> {
    let file_id = db
        .module_main_file(ModuleId::Submodule(contract.submodule_id))
        .ok()?;
    let file_path = match file_id.long(db) {
        FileLongId::OnDisk(path) => path.as_path(),
        _ => return None,
    };
    let relative = file_path.strip_prefix(workspace_root).ok()?;
    Some(normalize_fingerprint_path(relative))
}

#[cfg(feature = "native-compile")]
fn native_workspace_relative_cairo_path_from_debug(
    workspace_root: &Path,
    debug_path: &str,
) -> Option<String> {
    let path = Path::new(debug_path);
    let relative = if path.is_absolute() {
        path.strip_prefix(workspace_root).ok()?
    } else {
        path
    };
    if relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return None;
    }
    let is_cairo = relative
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cairo"));
    if !is_cairo {
        return None;
    }
    Some(normalize_fingerprint_path(relative))
}

#[cfg(feature = "native-compile")]
fn native_contract_dependency_paths_from_debug_info(
    workspace_root: &Path,
    contract_class: &ContractClass,
) -> BTreeSet<String> {
    let mut dependencies = BTreeSet::new();
    let Some(debug_info) = contract_class.sierra_program_debug_info.as_ref() else {
        return dependencies;
    };
    let Some(cairo_coverage) = debug_info
        .annotations
        .get("github.com/software-mansion/cairo-coverage")
    else {
        return dependencies;
    };
    let Some(statements_locations) = cairo_coverage
        .get("statements_code_locations")
        .and_then(serde_json::Value::as_object)
    else {
        return dependencies;
    };
    for locations in statements_locations.values() {
        let Some(entries) = locations.as_array() else {
            continue;
        };
        for entry in entries {
            let Some(tuple) = entry.as_array() else {
                continue;
            };
            let Some(path) = tuple.first().and_then(serde_json::Value::as_str) else {
                continue;
            };
            if let Some(relative) =
                native_workspace_relative_cairo_path_from_debug(workspace_root, path)
            {
                dependencies.insert(relative);
            }
        }
    }
    dependencies
}

#[cfg(feature = "native-compile")]
fn native_collect_contract_dependency_updates(
    workspace_root: &Path,
    plans: &[NativeContractOutputPlan],
    contract_source_paths: &[Option<String>],
    selected_indices: &[usize],
    contract_classes: &[ContractClass],
) -> Vec<(String, BTreeSet<String>)> {
    selected_indices
        .iter()
        .copied()
        .zip(contract_classes.iter())
        .map(|(contract_index, contract_class)| {
            let mut dependencies =
                native_contract_dependency_paths_from_debug_info(workspace_root, contract_class);
            if !dependencies.is_empty() {
                if let Some(Some(source_path)) = contract_source_paths.get(contract_index) {
                    dependencies.insert(source_path.clone());
                }
            }
            (plans[contract_index].module_path.clone(), dependencies)
        })
        .collect()
}

#[cfg(feature = "native-compile")]
fn native_prune_contract_source_dependencies_for_output_plans(
    contract_source_dependencies: &mut BTreeMap<String, BTreeSet<String>>,
    contract_output_plans: &[NativeContractOutputPlan],
) -> bool {
    let allowed_module_paths = contract_output_plans
        .iter()
        .map(|plan| plan.module_path.as_str())
        .collect::<HashSet<_>>();
    let dependency_count_before = contract_source_dependencies.len();
    contract_source_dependencies
        .retain(|module_path, _| allowed_module_paths.contains(module_path.as_str()));
    contract_source_dependencies.len() != dependency_count_before
}

#[cfg(feature = "native-compile")]
fn native_update_compile_session_post_build_state(
    workspace_root: &Path,
    signature: &NativeCompileSessionSignature,
    dependency_updates: &[(String, BTreeSet<String>)],
    contract_output_plans: Option<&[NativeContractOutputPlan]>,
) {
    if dependency_updates.is_empty() && contract_output_plans.is_none() {
        return;
    }
    let session_handle = match native_compile_session_handle(workspace_root, signature) {
        Ok((handle, _session_cache_hit)) => handle,
        Err(err) => {
            tracing::warn!(
                workspace_root = %workspace_root.display(),
                error = %format!("{err:#}"),
                "failed to update native session post-build state"
            );
            return;
        }
    };
    let (estimated_bytes, image_snapshot, buildinfo_snapshot) = {
        let mut session = session_handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if session.signature != *signature {
            return;
        }
        let mut state_changed = false;
        for (module_path, dependencies) in dependency_updates {
            if dependencies.is_empty() {
                if session
                    .contract_source_dependencies
                    .remove(module_path)
                    .is_some()
                {
                    state_changed = true;
                }
            } else {
                match session.contract_source_dependencies.get(module_path) {
                    Some(existing) if existing == dependencies => {}
                    _ => {
                        session
                            .contract_source_dependencies
                            .insert(module_path.clone(), dependencies.clone());
                        state_changed = true;
                    }
                }
            }
        }
        if let Some(contract_output_plans) = contract_output_plans {
            if session.contract_output_plans != contract_output_plans {
                native_prune_contract_source_dependencies_for_output_plans(
                    &mut session.contract_source_dependencies,
                    contract_output_plans,
                );
                session.contract_output_plans = contract_output_plans.to_vec();
                state_changed = true;
            }
        }
        (
            native_compile_session_state_estimated_bytes(&session),
            state_changed.then(|| native_compile_session_image_snapshot_from_state(&session)),
            state_changed.then(|| {
                native_buildinfo_file_from_state(&session, session.journal_cursor_applied)
            }),
        )
    };
    update_native_compile_session_cached_estimated_bytes(workspace_root, estimated_bytes);
    if let Some(image_snapshot) = image_snapshot {
        persist_native_compile_session_image_snapshot_best_effort(workspace_root, &image_snapshot);
    }
    if let Some(buildinfo_snapshot) = buildinfo_snapshot {
        persist_native_buildinfo_sidecar_best_effort(workspace_root, &buildinfo_snapshot);
    }
}

#[cfg(feature = "native-compile")]
fn native_contract_source_index_for_module_paths(
    module_paths: &[String],
    contract_source_dependencies: &BTreeMap<String, BTreeSet<String>>,
) -> (HashMap<String, Vec<usize>>, bool) {
    let mut by_source_sets: HashMap<String, BTreeSet<usize>> = HashMap::new();
    let mut dependency_index_complete = !module_paths.is_empty();
    for (index, module_path) in module_paths.iter().enumerate() {
        let Some(dependencies) = contract_source_dependencies.get(module_path) else {
            dependency_index_complete = false;
            continue;
        };
        if dependencies.is_empty() {
            dependency_index_complete = false;
            continue;
        }
        for dependency in dependencies {
            by_source_sets
                .entry(dependency.clone())
                .or_default()
                .insert(index);
        }
    }
    let by_source = by_source_sets
        .into_iter()
        .map(|(source, indices)| (source, indices.into_iter().collect::<Vec<_>>()))
        .collect();
    (by_source, dependency_index_complete)
}

#[cfg(feature = "native-compile")]
fn native_contract_source_index(
    contract_output_plans: &[NativeContractOutputPlan],
    contract_source_dependencies: &BTreeMap<String, BTreeSet<String>>,
) -> (HashMap<String, Vec<usize>>, bool) {
    let module_paths = contract_output_plans
        .iter()
        .map(|plan| plan.module_path.clone())
        .collect::<Vec<_>>();
    native_contract_source_index_for_module_paths(&module_paths, contract_source_dependencies)
}

#[cfg(feature = "native-compile")]
fn native_collect_impacted_contract_indices_from_source_index(
    by_source: &HashMap<String, Vec<usize>>,
    changed_files: &[String],
    removed_files: &[String],
) -> (BTreeSet<usize>, BTreeSet<String>) {
    let mut impacted = BTreeSet::new();
    let mut unmatched_sources = BTreeSet::new();
    for source in changed_files.iter().chain(removed_files.iter()) {
        let Some(indices) = by_source.get(source) else {
            unmatched_sources.insert(source.clone());
            continue;
        };
        for index in indices {
            impacted.insert(*index);
        }
    }
    (impacted, unmatched_sources)
}

#[cfg(feature = "native-compile")]
fn native_indexed_source_metric(
    changed_files: &[String],
    removed_files: &[String],
    unmatched_sources: &BTreeSet<String>,
) -> usize {
    // Logging-only metric: duplicates across changed/removed paths are uncommon and
    // acceptable for this coarse visibility counter.
    changed_files
        .len()
        .saturating_add(removed_files.len())
        .saturating_sub(unmatched_sources.len())
}

#[cfg(feature = "native-compile")]
fn native_filter_changed_files_to_contract_source_index(
    changed_files: &[String],
    removed_files: &[String],
    by_source: &HashMap<String, Vec<usize>>,
    dependency_index_complete: bool,
) -> (Vec<String>, Vec<String>) {
    let has_absolute_paths = changed_files
        .iter()
        .chain(removed_files.iter())
        .any(|source| Path::new(source).is_absolute());
    if has_absolute_paths {
        return (changed_files.to_vec(), removed_files.to_vec());
    }
    if !dependency_index_complete {
        return (changed_files.to_vec(), removed_files.to_vec());
    }
    let scoped_changed = changed_files
        .iter()
        .filter(|source| by_source.contains_key(source.as_str()))
        .cloned()
        .collect();
    let scoped_removed = removed_files
        .iter()
        .filter(|source| by_source.contains_key(source.as_str()))
        .cloned()
        .collect();
    (scoped_changed, scoped_removed)
}

#[cfg(feature = "native-compile")]
fn native_changed_files_affect_tracked_contracts(
    changed_files: &[String],
    removed_files: &[String],
    contract_output_plans: &[NativeContractOutputPlan],
    contract_source_dependencies: &BTreeMap<String, BTreeSet<String>>,
) -> bool {
    if changed_files.is_empty() && removed_files.is_empty() {
        return false;
    }
    if changed_files
        .iter()
        .chain(removed_files.iter())
        .any(|source| Path::new(source).is_absolute())
    {
        return true;
    }
    let (by_source, dependency_index_complete) =
        native_contract_source_index(contract_output_plans, contract_source_dependencies);
    if !dependency_index_complete {
        // Keep the conservative behavior when dependency coverage is incomplete.
        return true;
    }
    changed_files
        .iter()
        .chain(removed_files.iter())
        .any(|source| by_source.contains_key(source.as_str()))
}

#[cfg(feature = "native-compile")]
fn native_compile_batch_ranges(
    total_contracts: usize,
    configured_batch_size: usize,
) -> Vec<(usize, usize)> {
    if total_contracts == 0 {
        return Vec::new();
    }
    let batch_size = if configured_batch_size == 0 {
        total_contracts
    } else {
        configured_batch_size.min(total_contracts).max(1)
    };
    (0..total_contracts)
        .step_by(batch_size)
        .map(|start| (start, (start + batch_size).min(total_contracts)))
        .collect()
}

#[cfg(feature = "native-compile")]
fn native_compile_batch_summary(
    all_plans: &[NativeContractOutputPlan],
    selected_indices: &[usize],
) -> String {
    let preview = selected_indices
        .iter()
        .take(3)
        .filter_map(|index| all_plans.get(*index))
        .map(|plan| plan.module_path.clone())
        .collect::<Vec<_>>();
    let remaining = selected_indices.len().saturating_sub(preview.len());
    if preview.is_empty() {
        return format!("{} contract(s)", selected_indices.len());
    }
    if remaining == 0 {
        return preview.join(", ");
    }
    format!("{} (+{} more)", preview.join(", "), remaining)
}

#[cfg(feature = "native-compile")]
fn native_run_contract_compile_batches<T>(
    all_plans: &[NativeContractOutputPlan],
    selected_indices: &[usize],
    configured_batch_size: usize,
    mut compile_batch: impl FnMut(&[usize]) -> Result<Vec<T>>,
) -> Result<(f64, Vec<T>)> {
    if selected_indices.is_empty() {
        return Ok((0.0, Vec::new()));
    }
    let batch_ranges = native_compile_batch_ranges(selected_indices.len(), configured_batch_size);
    native_progress_log(format!(
        "native contract compile start (selected={}, total={}, batches={}, summary={})",
        selected_indices.len(),
        all_plans.len(),
        batch_ranges.len(),
        native_compile_batch_summary(all_plans, selected_indices)
    ));
    let frontend_compile_start = Instant::now();
    let mut compiled = Vec::with_capacity(selected_indices.len());
    for (batch_ordinal, (start, end)) in batch_ranges.iter().copied().enumerate() {
        let batch_indices = &selected_indices[start..end];
        let batch_label = format!(
            "native contract compile batch {}/{}",
            batch_ordinal + 1,
            batch_ranges.len()
        );
        native_progress_log(format!(
            "{batch_label} start (contracts={}, summary={})",
            batch_indices.len(),
            native_compile_batch_summary(all_plans, batch_indices)
        ));
        let _heartbeat = NativeProgressHeartbeat::start(batch_label.clone());
        let batch_compile_start = Instant::now();
        let mut batch_results = compile_batch(batch_indices)?;
        if batch_results.len() != batch_indices.len() {
            bail!(
                "native compile returned mismatched batch result count in batch {} (expected {}, got {})",
                batch_ordinal + 1,
                batch_indices.len(),
                batch_results.len()
            );
        }
        native_progress_log(format!(
            "{batch_label} finished in {:.1}ms",
            batch_compile_start.elapsed().as_secs_f64() * 1000.0
        ));
        compiled.append(&mut batch_results);
    }
    let frontend_compile_ms = frontend_compile_start.elapsed().as_secs_f64() * 1000.0;
    native_progress_log(format!(
        "native contract compile finished in {:.1}ms",
        frontend_compile_ms
    ));
    Ok((frontend_compile_ms, compiled))
}

#[cfg(feature = "native-compile")]
fn native_contract_from_module_path<'db>(
    db: &'db RootDatabase,
    crate_ids: &[CrateId<'db>],
    module_path: &str,
) -> Option<ContractDeclaration<'db>> {
    let mut segments = module_path.split("::");
    let crate_name = segments.next()?;
    let crate_id = crate_ids
        .iter()
        .copied()
        .find(|crate_id| ModuleId::CrateRoot(*crate_id).name(db).long(db) == crate_name)?;
    let mut module_id = ModuleId::CrateRoot(crate_id);
    for segment in segments {
        let submodule_id = db
            .module_submodules_ids(module_id)
            .ok()?
            .iter()
            .copied()
            .find(|submodule_id| submodule_id.name(db).long(db) == segment)?;
        module_id = ModuleId::Submodule(submodule_id);
    }
    module_contract(db, module_id)
}

#[cfg(feature = "native-compile")]
fn native_resolve_contracts_from_output_plans<'db>(
    db: &'db RootDatabase,
    crate_ids: &[CrateId<'db>],
    contract_output_plans: &[NativeContractOutputPlan],
) -> Option<Vec<ContractDeclaration<'db>>> {
    if contract_output_plans.is_empty() {
        return Some(Vec::new());
    }
    let mut contracts = Vec::with_capacity(contract_output_plans.len());
    for plan in contract_output_plans {
        contracts.push(native_contract_from_module_path(
            db,
            crate_ids,
            &plan.module_path,
        )?);
    }
    Some(contracts)
}

#[cfg(all(feature = "native-compile", test))]
fn native_impacted_contract_indices_from_source_index(
    by_source: &HashMap<String, Vec<usize>>,
    changed_files: &[String],
    removed_files: &[String],
    dependency_index_complete: bool,
) -> Option<Vec<usize>> {
    if changed_files
        .iter()
        .chain(removed_files.iter())
        .any(|source| Path::new(source).is_absolute())
    {
        return None;
    }
    let (impacted, unmatched_sources) = native_collect_impacted_contract_indices_from_source_index(
        by_source,
        changed_files,
        removed_files,
    );
    if !unmatched_sources.is_empty() && !dependency_index_complete {
        tracing::debug!(
            changed_files = changed_files.len(),
            removed_files = removed_files.len(),
            indexed_sources =
                native_indexed_source_metric(changed_files, removed_files, &unmatched_sources),
            "native impacted subset index is incomplete for changed file set; falling back to full compile"
        );
        return None;
    }
    Some(impacted.into_iter().collect())
}

#[cfg(feature = "native-compile")]
fn native_impacted_contract_indices(
    module_paths: &[String],
    contract_source_paths: &[Option<String>],
    changed_files: &[String],
    removed_files: &[String],
    contract_source_dependencies: &BTreeMap<String, BTreeSet<String>>,
) -> Option<Vec<usize>> {
    if changed_files.is_empty() && removed_files.is_empty() {
        return Some(Vec::new());
    }
    if changed_files
        .iter()
        .chain(removed_files.iter())
        .any(|source| Path::new(source).is_absolute())
    {
        return None;
    }
    let (by_source, dependency_index_complete) =
        native_contract_source_index_for_module_paths(module_paths, contract_source_dependencies);
    let (mut impacted, mut unmatched_sources) =
        native_collect_impacted_contract_indices_from_source_index(
            &by_source,
            changed_files,
            removed_files,
        );
    if dependency_index_complete {
        // With a complete dependency index, unmatched changed/removed paths are
        // intentionally treated as non-impacting only when they are not tracked
        // contract sources. If this invariant is violated, force conservative
        // fallback to avoid stale outputs in release builds.
        let missing_tracked_sources = unmatched_sources
            .iter()
            .filter(|source| {
                contract_source_paths
                    .iter()
                    .flatten()
                    .any(|tracked_source| tracked_source == *source)
            })
            .cloned()
            .collect::<Vec<_>>();
        if !missing_tracked_sources.is_empty() {
            tracing::warn!(
                missing_tracked_sources = ?missing_tracked_sources,
                changed_files = changed_files.len(),
                removed_files = removed_files.len(),
                "dependency index completeness invariant violated; falling back to full compile"
            );
            return None;
        }
    }
    if !unmatched_sources.is_empty() && !dependency_index_complete {
        for (index, source_path) in contract_source_paths.iter().enumerate() {
            let Some(source_path) = source_path else {
                continue;
            };
            if unmatched_sources.remove(source_path) {
                impacted.insert(index);
            }
        }
        if !unmatched_sources.is_empty() {
            tracing::debug!(
                changed_files = changed_files.len(),
                removed_files = removed_files.len(),
                indexed_sources =
                    native_indexed_source_metric(changed_files, removed_files, &unmatched_sources),
                "native impacted subset index is incomplete for changed file set; falling back to full compile"
            );
            return None;
        }
    }
    Some(impacted.into_iter().collect())
}

#[cfg(feature = "native-compile")]
fn native_reusable_unaffected_manifest_entries(
    target_dir: &Path,
    package_name: &str,
    plans: &[NativeContractOutputPlan],
    impacted_indices: &BTreeSet<usize>,
) -> Result<Option<(Vec<StarknetArtifactEntry>, BTreeSet<String>)>> {
    let manifest_path = target_dir.join(format!("{package_name}.starknet_artifacts.json"));
    let manifest_bytes = match fs::read(&manifest_path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).with_context(|| format!("failed to read {}", manifest_path.display()));
        }
    };
    let manifest = serde_json::from_slice::<StarknetArtifactsManifest>(&manifest_bytes)
        .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    if manifest.version != 1 {
        return Ok(None);
    }
    let entry_by_id: HashMap<String, StarknetArtifactEntry> = manifest
        .contracts
        .into_iter()
        .map(|entry| (entry.id.clone(), entry))
        .collect();
    let mut keep_files = BTreeSet::new();
    let mut reusable_entries = Vec::new();
    for (index, plan) in plans.iter().enumerate() {
        if impacted_indices.contains(&index) {
            continue;
        }
        let Some(entry) = entry_by_id.get(&plan.artifact_id) else {
            return Ok(None);
        };
        if entry.module_path != plan.module_path
            || entry.artifacts.sierra != plan.artifact_file
            || entry.artifacts.casm != plan.casm_file
        {
            return Ok(None);
        }
        let sierra_path = target_dir.join(&entry.artifacts.sierra);
        if !sierra_path.is_file() {
            return Ok(None);
        }
        keep_files.insert(entry.artifacts.sierra.clone());
        if let Some(casm_file) = &entry.artifacts.casm {
            let casm_path = target_dir.join(casm_file);
            if !casm_path.is_file() {
                return Ok(None);
            }
            keep_files.insert(casm_file.clone());
        }
        reusable_entries.push(entry.clone());
    }
    Ok(Some((reusable_entries, keep_files)))
}

#[cfg(feature = "native-compile")]
fn native_cached_noop_keep_files(
    target_dir: &Path,
    package_name: &str,
    plans: &[NativeContractOutputPlan],
) -> Result<Option<BTreeSet<String>>> {
    if plans.is_empty() {
        return Ok(None);
    }
    let impacted = BTreeSet::new();
    let Some((_entries, mut keep_files)) =
        native_reusable_unaffected_manifest_entries(target_dir, package_name, plans, &impacted)?
    else {
        return Ok(None);
    };
    keep_files.insert(format!("{package_name}.starknet_artifacts.json"));
    Ok(Some(keep_files))
}

#[cfg(feature = "native-compile")]
fn prune_native_target_outputs(
    target_dir: &Path,
    package_name: &str,
    keep_files: &BTreeSet<String>,
) -> Result<()> {
    if !target_dir.exists() {
        return Ok(());
    }
    let sierra_name = format!("{package_name}.sierra");
    let artifacts_name = format!("{package_name}.starknet_artifacts.json");
    for entry in fs::read_dir(target_dir)
        .with_context(|| format!("failed to read {}", target_dir.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read {}", target_dir.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to stat {}", entry.path().display()))?;
        if !file_type.is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        let should_remove = file_name == sierra_name
            || file_name == artifacts_name
            || (file_name.starts_with(&format!("{package_name}_"))
                && (file_name.ends_with(".contract_class.json")
                    || file_name.ends_with(".compiled_contract_class.json")));
        if keep_files.contains(&file_name) {
            continue;
        }
        if should_remove {
            fs::remove_file(entry.path())
                .with_context(|| format!("failed to remove {}", entry.path().display()))?;
        }
    }
    Ok(())
}

#[cfg(feature = "native-compile")]
fn compile_native_casm_contract(
    contract_class: ContractClass,
    max_casm_bytecode_size: usize,
) -> Result<CasmContractClass> {
    let extracted_program = contract_class
        .extract_sierra_program(false)
        .context("failed to extract native Sierra program for CASM emission")?;
    CasmContractClass::from_contract_class(
        contract_class,
        extracted_program,
        false,
        max_casm_bytecode_size,
    )
    .context("failed to compile native CASM contract class")
}

#[cfg(feature = "native-compile")]
fn native_compile_session_cache_key(workspace_root: &Path) -> String {
    normalize_fingerprint_path(workspace_root)
}

#[cfg(feature = "native-compile")]
fn native_compile_session_signature(
    manifest_path: &Path,
    context: &NativeCompileContext,
) -> NativeCompileSessionSignature {
    NativeCompileSessionSignature {
        manifest_path: manifest_path.to_path_buf(),
        manifest_content_hash: context.manifest_content_hash.clone(),
        context: context.clone(),
    }
}

#[cfg(feature = "native-compile")]
fn native_compile_session_signature_hash(signature: &NativeCompileSessionSignature) -> String {
    let mut hasher = Hasher::new();
    hasher.update(b"uc-native-session-image-signature-v1");
    hasher.update(normalize_fingerprint_path(&signature.manifest_path).as_bytes());
    hasher.update(signature.manifest_content_hash.as_bytes());
    hasher.update(signature.context.package_name.as_bytes());
    hasher.update(signature.context.crate_name.as_bytes());
    hasher.update(normalize_fingerprint_path(&signature.context.cairo_project_dir).as_bytes());
    hasher.update(normalize_fingerprint_path(&signature.context.corelib_src).as_bytes());
    hasher.update(if signature.context.starknet_target.sierra {
        b"1"
    } else {
        b"0"
    });
    hasher.update(if signature.context.starknet_target.casm {
        b"1"
    } else {
        b"0"
    });

    let mut external_dependencies = signature.context.external_non_starknet_dependencies.clone();
    external_dependencies.sort();
    external_dependencies.dedup();
    for dependency in external_dependencies {
        hasher.update(b"\x1Fdep\x1F");
        hasher.update(dependency.as_bytes());
    }

    let mut dependency_roots = signature
        .context
        .path_dependency_roots
        .iter()
        .map(|root| {
            format!(
                "{}={}",
                root.crate_name,
                normalize_fingerprint_path(&root.source_root)
            )
        })
        .collect::<Vec<_>>();
    dependency_roots.sort();
    dependency_roots.dedup();
    for root in dependency_roots {
        hasher.update(b"\x1Froot\x1F");
        hasher.update(root.as_bytes());
    }

    let mut crate_dependency_configs = signature.context.crate_dependency_configs.clone();
    crate_dependency_configs.sort_by(|left, right| {
        left.crate_name
            .cmp(&right.crate_name)
            .then_with(|| left.cairo_edition.cmp(&right.cairo_edition))
    });
    for config in crate_dependency_configs {
        hasher.update(b"\x1Fcfg\x1F");
        hasher.update(config.crate_name.as_bytes());
        hasher.update(b"=");
        hasher.update(
            config
                .cairo_edition
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
        );
        let mut dependencies = config.dependencies;
        dependencies.sort();
        dependencies.dedup();
        for dependency in dependencies {
            hasher.update(b",");
            hasher.update(dependency.as_bytes());
        }
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(feature = "native-compile")]
fn native_source_hash_index_path(workspace_root: &Path) -> Result<PathBuf> {
    let path = workspace_root.join(".uc/cache/native-session/source-hash-index-v2.json");
    ensure_path_within_root(workspace_root, &path, "native source hash index path")?;
    Ok(path)
}

#[cfg(feature = "native-compile")]
fn native_normalize_tracked_source_key(
    workspace_root: &Path,
    tracked_path: &str,
) -> Result<String> {
    let path = Path::new(tracked_path);
    if path.is_absolute() {
        if let Ok(relative) = path.strip_prefix(workspace_root) {
            let relative = normalize_fingerprint_path(relative);
            let relative = validated_relative_artifact_path(&relative).with_context(|| {
                format!("native tracked source key contains invalid components: {tracked_path}")
            })?;
            return Ok(normalize_fingerprint_path(&relative));
        }
        return Ok(normalize_fingerprint_path(path));
    }
    let relative = validated_relative_artifact_path(tracked_path).with_context(|| {
        format!("native tracked source key contains invalid components: {tracked_path}")
    })?;
    Ok(normalize_fingerprint_path(&relative))
}

#[cfg(feature = "native-compile")]
fn native_normalize_tracked_sources(
    workspace_root: &Path,
    tracked_sources: BTreeMap<String, NativeTrackedFileState>,
) -> Result<(BTreeMap<String, NativeTrackedFileState>, bool)> {
    let mut normalized = BTreeMap::new();
    let mut changed = false;
    for (tracked_path, state) in tracked_sources {
        let canonical_key = native_normalize_tracked_source_key(workspace_root, &tracked_path)?;
        if canonical_key != tracked_path {
            changed = true;
        }
        match normalized.entry(canonical_key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(state);
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                if entry.get() != &state {
                    bail!(
                        "duplicate canonical tracked source key with conflicting metadata: {}",
                        entry.key()
                    );
                }
                changed = true;
            }
        }
    }
    Ok((normalized, changed))
}

#[cfg(feature = "native-compile")]
fn native_tracked_sources_content_hash(
    workspace_root: &Path,
    tracked_sources: &BTreeMap<String, NativeTrackedFileState>,
) -> Result<String> {
    let index_path = native_source_hash_index_path(workspace_root)?;
    let mut index = load_fingerprint_index_cached(&index_path).unwrap_or_else(|err| {
        tracing::warn!(
            path = %index_path.display(),
            error = %format!("{err:#}"),
            "failed to load native source hash index; rebuilding"
        );
        FingerprintIndex::empty()
    });
    let now_ms = epoch_ms_u64().unwrap_or_default();
    let recheck_window_ms = fingerprint_mtime_recheck_window_ms();
    let mut updated_entries = BTreeMap::new();
    let mut hasher = Hasher::new();
    hasher.update(b"uc-native-tracked-sources-content-hash-v2");

    let mut canonical_sources = Vec::with_capacity(tracked_sources.len());
    for tracked_path in tracked_sources.keys() {
        let canonical_key = native_normalize_tracked_source_key(workspace_root, tracked_path)?;
        let absolute = {
            let path = Path::new(tracked_path);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                let relative = validated_relative_artifact_path(tracked_path).with_context(|| {
                    format!(
                        "native tracked source hash path contains invalid components: {tracked_path}"
                    )
                })?;
                let absolute = workspace_root.join(&relative);
                ensure_path_within_root(
                    workspace_root,
                    &absolute,
                    "native tracked source hash path",
                )?;
                absolute
            }
        };
        canonical_sources.push((canonical_key, tracked_path.clone(), absolute));
    }
    canonical_sources.sort_by(|left, right| left.0.cmp(&right.0));

    for (canonical_key, original_key, absolute) in canonical_sources {
        let metadata = fs::metadata(&absolute)
            .with_context(|| format!("failed to stat {}", absolute.display()))?;
        if !metadata.is_file() {
            bail!("tracked source is not a file: {}", absolute.display());
        }
        let size_bytes = metadata.len();
        let modified_unix_ms = metadata_modified_unix_ms(&metadata)?;
        let cached_entry = index
            .entries
            .get(&canonical_key)
            .or_else(|| index.entries.get(&original_key));
        let file_hash = if let Some(cached) = cached_entry {
            let should_rehash_recent = now_ms.saturating_sub(modified_unix_ms) <= recheck_window_ms;
            if cached.size_bytes == size_bytes
                && cached.modified_unix_ms == modified_unix_ms
                && !should_rehash_recent
            {
                cached.blake3_hex.clone()
            } else {
                hash_fingerprint_source_file(&absolute)?
            }
        } else {
            hash_fingerprint_source_file(&absolute)?
        };
        if updated_entries.contains_key(&canonical_key) {
            bail!("duplicate canonical tracked source key: {canonical_key}");
        }
        updated_entries.insert(
            canonical_key.clone(),
            FingerprintIndexEntry {
                size_bytes,
                modified_unix_ms,
                blake3_hex: file_hash.clone(),
            },
        );
        hasher.update(canonical_key.as_bytes());
        hasher.update(b":");
        hasher.update(file_hash.as_bytes());
        hasher.update(b"\n");
    }

    if index.entries != updated_entries || index.schema_version != FINGERPRINT_INDEX_SCHEMA_VERSION
    {
        index.schema_version = FINGERPRINT_INDEX_SCHEMA_VERSION;
        index.entries = updated_entries;
        index.directories.clear();
        index.context_digest = None;
        index.last_fingerprint = None;
        store_fingerprint_index_cached(&index_path, &index);
        if let Err(err) = save_fingerprint_index(&index_path, &index) {
            tracing::warn!(
                path = %index_path.display(),
                error = %format!("{err:#}"),
                "failed to persist native source hash index"
            );
        }
    }

    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(feature = "native-compile")]
fn native_compile_session_image_path(workspace_root: &Path) -> Result<PathBuf> {
    let image_path = workspace_root.join(".uc/cache/native-session/session-image-v2.bin");
    ensure_path_within_root(
        workspace_root,
        &image_path,
        "native compile session image path",
    )?;
    Ok(image_path)
}

#[cfg(feature = "native-compile")]
fn native_compile_session_image_legacy_path(workspace_root: &Path) -> Result<PathBuf> {
    let image_path = workspace_root.join(".uc/cache/native-session/session-image-v1.json");
    ensure_path_within_root(
        workspace_root,
        &image_path,
        "native compile session legacy image path",
    )?;
    Ok(image_path)
}

#[cfg(feature = "native-compile")]
fn native_compile_session_image_snapshot_from_state(
    session: &NativeCompileSessionState,
) -> NativeCompileSessionImageSnapshot {
    NativeCompileSessionImageSnapshot {
        signature_hash: native_compile_session_signature_hash(&session.signature),
        source_root_modified_unix_ms: session.source_root_modified_unix_ms,
        tracked_sources: session.tracked_sources.clone(),
        tracked_source_bytes: session.tracked_source_bytes,
        tracked_sources_content_hash: session.tracked_sources_content_hash.clone(),
        contract_source_dependencies: session.contract_source_dependencies.clone(),
        contract_output_plans: session.contract_output_plans.clone(),
        journal_cursor_applied: session.journal_cursor_applied,
    }
}

#[cfg(feature = "native-compile")]
fn persist_native_compile_session_image_snapshot(
    workspace_root: &Path,
    snapshot: &NativeCompileSessionImageSnapshot,
) -> Result<()> {
    let image_path = native_compile_session_image_path(workspace_root)?;
    let parent = image_path
        .parent()
        .context("native compile session image path has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let image = NativeCompileSessionImageFile {
        schema_version: NATIVE_COMPILE_SESSION_IMAGE_SCHEMA_VERSION,
        signature_hash: snapshot.signature_hash.clone(),
        source_root_modified_unix_ms: snapshot.source_root_modified_unix_ms,
        tracked_sources: snapshot.tracked_sources.clone(),
        tracked_source_bytes: snapshot.tracked_source_bytes,
        tracked_sources_content_hash: snapshot.tracked_sources_content_hash.clone(),
        contract_source_dependencies: snapshot.contract_source_dependencies.clone(),
        contract_output_plans: snapshot.contract_output_plans.clone(),
        journal_cursor_applied: snapshot.journal_cursor_applied,
        generated_at_epoch_ms: epoch_ms_u64().unwrap_or_default(),
    };
    let bytes = postcard::to_allocvec(&image).context("failed to encode native session image")?;
    if bytes.len() as u64 > MAX_NATIVE_COMPILE_SESSION_IMAGE_BYTES {
        tracing::warn!(
            path = %image_path.display(),
            bytes = bytes.len(),
            max_bytes = MAX_NATIVE_COMPILE_SESSION_IMAGE_BYTES,
            "skipping native session image write: image exceeds size limit"
        );
        return Ok(());
    }
    atomic_write_bytes(&image_path, &bytes, "native session image")
}

#[cfg(feature = "native-compile")]
fn persist_native_compile_session_image_snapshot_best_effort(
    workspace_root: &Path,
    snapshot: &NativeCompileSessionImageSnapshot,
) {
    if let Err(err) = persist_native_compile_session_image_snapshot(workspace_root, snapshot) {
        tracing::warn!(
            workspace_root = %workspace_root.display(),
            error = %format!("{err:#}"),
            "failed to persist native session image"
        );
    }
}

#[cfg(feature = "native-compile")]
fn try_native_compile_session_image_restore(
    workspace_root: &Path,
    signature: &NativeCompileSessionSignature,
    tracked_sources_content_hash: &str,
) -> Option<NativeCompileSessionImageSnapshot> {
    let signature_hash = native_compile_session_signature_hash(signature);
    let candidate_paths = [
        (
            native_compile_session_image_path(workspace_root),
            "native session image",
            true,
        ),
        (
            native_compile_session_image_legacy_path(workspace_root),
            "legacy native session image",
            false,
        ),
    ];

    for (path_result, label, is_binary) in candidate_paths {
        let image_path = match path_result {
            Ok(path) => path,
            Err(err) => {
                tracing::warn!(
                    workspace_root = %workspace_root.display(),
                    error = %format!("{err:#}"),
                    "{label} path is invalid; ignoring persisted image"
                );
                continue;
            }
        };
        let metadata = match fs::metadata(&image_path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => {
                tracing::warn!(
                    path = %image_path.display(),
                    error = %err,
                    "failed to stat {label}; ignoring"
                );
                continue;
            }
        };
        if metadata.len() > MAX_NATIVE_COMPILE_SESSION_IMAGE_BYTES {
            tracing::warn!(
                path = %image_path.display(),
                bytes = metadata.len(),
                max_bytes = MAX_NATIVE_COMPILE_SESSION_IMAGE_BYTES,
                "ignoring oversized {label}"
            );
            let _ = fs::remove_file(&image_path);
            continue;
        }
        let bytes =
            match read_bytes_with_limit(&image_path, MAX_NATIVE_COMPILE_SESSION_IMAGE_BYTES, label)
            {
                Ok(bytes) => bytes,
                Err(err) => {
                    tracing::warn!(
                        path = %image_path.display(),
                        error = %format!("{err:#}"),
                        "failed to read {label}; ignoring"
                    );
                    continue;
                }
            };
        let decoded: NativeCompileSessionImageFile = if is_binary {
            match postcard::from_bytes::<NativeCompileSessionImageFile>(&bytes) {
                Ok(image)
                    if image.schema_version == NATIVE_COMPILE_SESSION_IMAGE_SCHEMA_VERSION =>
                {
                    image
                }
                Ok(_) => continue,
                Err(err) => {
                    tracing::warn!(
                        path = %image_path.display(),
                        error = %err,
                        "failed to decode {label}; ignoring"
                    );
                    continue;
                }
            }
        } else {
            match serde_json::from_slice::<NativeCompileSessionImageFile>(&bytes) {
                Ok(image) => image,
                Err(err) => {
                    tracing::warn!(
                        path = %image_path.display(),
                        error = %err,
                        "failed to decode {label}; ignoring"
                    );
                    continue;
                }
            }
        };
        if decoded.signature_hash != signature_hash {
            continue;
        }
        let (tracked_sources, tracked_sources_normalized) =
            match native_normalize_tracked_sources(workspace_root, decoded.tracked_sources) {
                Ok(value) => value,
                Err(err) => {
                    tracing::warn!(
                        path = %image_path.display(),
                        error = %format!("{err:#}"),
                        "{label} tracked source set is invalid; ignoring"
                    );
                    continue;
                }
            };
        let tracked_source_bytes = match native_tracked_sources_total_bytes(&tracked_sources) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(
                    path = %image_path.display(),
                    error = %format!("{err:#}"),
                    "{label} tracked source set is invalid; ignoring"
                );
                continue;
            }
        };
        let computed_content_hash =
            if tracked_sources_normalized || decoded.tracked_sources_content_hash.is_empty() {
                match native_tracked_sources_content_hash(workspace_root, &tracked_sources) {
                    Ok(hash) => hash,
                    Err(err) => {
                        tracing::warn!(
                            path = %image_path.display(),
                            error = %format!("{err:#}"),
                            "failed to compute content hash for {label}; ignoring"
                        );
                        continue;
                    }
                }
            } else {
                decoded.tracked_sources_content_hash.clone()
            };
        if computed_content_hash != tracked_sources_content_hash {
            continue;
        }
        if tracked_source_bytes != decoded.tracked_source_bytes {
            tracing::warn!(
                path = %image_path.display(),
                image_bytes = decoded.tracked_source_bytes,
                computed_bytes = tracked_source_bytes,
                "{label} tracked-source byte budget drift; using computed value"
            );
        }
        return Some(NativeCompileSessionImageSnapshot {
            signature_hash: signature_hash.clone(),
            source_root_modified_unix_ms: decoded.source_root_modified_unix_ms,
            tracked_sources,
            tracked_source_bytes,
            tracked_sources_content_hash: computed_content_hash,
            contract_source_dependencies: decoded.contract_source_dependencies,
            contract_output_plans: decoded.contract_output_plans,
            journal_cursor_applied: decoded.journal_cursor_applied,
        });
    }

    None
}

#[cfg(feature = "native-compile")]
fn native_source_journal_path(workspace_root: &Path) -> Result<PathBuf> {
    let path = workspace_root.join(".uc/cache/native-session/source-journal-v1.json");
    ensure_path_within_root(workspace_root, &path, "native source journal path")?;
    Ok(path)
}

#[cfg(feature = "native-compile")]
fn native_buildinfo_sidecar_path(workspace_root: &Path) -> Result<PathBuf> {
    let path = workspace_root.join(".uc/native-buildinfo-v2.bin");
    ensure_path_within_root(workspace_root, &path, "native buildinfo path")?;
    Ok(path)
}

#[cfg(feature = "native-compile")]
fn native_buildinfo_sidecar_legacy_path(workspace_root: &Path) -> Result<PathBuf> {
    let path = workspace_root.join(".uc/native-buildinfo.json");
    ensure_path_within_root(workspace_root, &path, "native legacy buildinfo path")?;
    Ok(path)
}

#[cfg(feature = "native-compile")]
#[cfg_attr(not(test), allow(dead_code))]
fn native_has_persisted_session_state_hints(workspace_root: &Path) -> bool {
    native_compile_session_image_path(workspace_root)
        .ok()
        .is_some_and(|path| path.is_file())
        || native_compile_session_image_legacy_path(workspace_root)
            .ok()
            .is_some_and(|path| path.is_file())
        || native_buildinfo_sidecar_path(workspace_root)
            .ok()
            .is_some_and(|path| path.is_file())
        || native_buildinfo_sidecar_legacy_path(workspace_root)
            .ok()
            .is_some_and(|path| path.is_file())
}

#[cfg(feature = "native-compile")]
fn native_tracked_sources_signature(
    tracked_sources: &BTreeMap<String, NativeTrackedFileState>,
) -> String {
    let mut hasher = Hasher::new();
    hasher.update(b"uc-native-tracked-sources-signature-v1");
    for (path, state) in tracked_sources {
        hasher.update(path.as_bytes());
        hasher.update(&state.size_bytes.to_le_bytes());
        hasher.update(&state.modified_unix_ms.to_le_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(feature = "native-compile")]
fn native_source_journal_file_from_state(
    state: &NativeSourceChangeJournal,
) -> NativeSourceJournalFile {
    NativeSourceJournalFile {
        schema_version: NATIVE_SOURCE_JOURNAL_SCHEMA_VERSION,
        changed_files: state.changed_files.iter().cloned().collect(),
        removed_files: state.removed_files.iter().cloned().collect(),
        overflowed: state.overflowed,
        cursor: state.cursor,
        applied_cursor: state.applied_cursor.min(state.cursor),
        updated_at_epoch_ms: epoch_ms_u64().unwrap_or_default(),
    }
}

#[cfg(feature = "native-compile")]
fn native_source_journal_state_from_file(
    decoded: NativeSourceJournalFile,
) -> NativeSourceChangeJournal {
    let changed_overflow = decoded.changed_files.len() > max_fingerprint_files();
    let removed_overflow = decoded.removed_files.len() > max_fingerprint_files();
    let changed_files = decoded
        .changed_files
        .into_iter()
        .take(max_fingerprint_files())
        .collect::<BTreeSet<_>>();
    let removed_files = decoded
        .removed_files
        .into_iter()
        .take(max_fingerprint_files())
        .collect::<BTreeSet<_>>();
    NativeSourceChangeJournal {
        changed_files,
        removed_files,
        overflowed: decoded.overflowed || changed_overflow || removed_overflow,
        cursor: decoded.cursor,
        applied_cursor: decoded.applied_cursor.min(decoded.cursor),
    }
}

#[cfg(feature = "native-compile")]
fn persist_native_source_change_journal(
    workspace_root: &Path,
    state: &NativeSourceChangeJournal,
) -> Result<()> {
    let journal_path = native_source_journal_path(workspace_root)?;
    let parent = journal_path
        .parent()
        .context("native source journal path has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let mut snapshot = state.clone();
    if snapshot.changed_files.len() > max_fingerprint_files()
        || snapshot.removed_files.len() > max_fingerprint_files()
    {
        snapshot.changed_files.clear();
        snapshot.removed_files.clear();
        snapshot.overflowed = true;
    }
    let file = native_source_journal_file_from_state(&snapshot);
    let bytes = serde_json::to_vec(&file).context("failed to encode native source journal")?;
    if bytes.len() as u64 > MAX_NATIVE_SOURCE_JOURNAL_BYTES {
        bail!(
            "native source journal exceeds size limit ({} bytes > {} bytes)",
            bytes.len(),
            MAX_NATIVE_SOURCE_JOURNAL_BYTES
        );
    }
    atomic_write_bytes(&journal_path, &bytes, "native source journal")
}

#[cfg(feature = "native-compile")]
fn persist_native_source_change_journal_best_effort(
    workspace_root: &Path,
    state: &NativeSourceChangeJournal,
) {
    if let Err(err) = persist_native_source_change_journal(workspace_root, state) {
        tracing::warn!(
            workspace_root = %workspace_root.display(),
            error = %format!("{err:#}"),
            "failed to persist native source journal"
        );
    }
}

#[cfg(feature = "native-compile")]
fn load_native_source_change_journal(workspace_root: &Path) -> NativeSourceChangeJournal {
    let journal_path = match native_source_journal_path(workspace_root) {
        Ok(path) => path,
        Err(err) => {
            tracing::warn!(
                workspace_root = %workspace_root.display(),
                error = %format!("{err:#}"),
                "native source journal path is invalid; ignoring persisted journal"
            );
            return NativeSourceChangeJournal::default();
        }
    };
    let metadata = match fs::metadata(&journal_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return NativeSourceChangeJournal::default()
        }
        Err(err) => {
            tracing::warn!(
                path = %journal_path.display(),
                error = %err,
                "failed to stat native source journal; ignoring"
            );
            return NativeSourceChangeJournal {
                overflowed: true,
                ..NativeSourceChangeJournal::default()
            };
        }
    };
    if metadata.len() > MAX_NATIVE_SOURCE_JOURNAL_BYTES {
        tracing::warn!(
            path = %journal_path.display(),
            bytes = metadata.len(),
            max_bytes = MAX_NATIVE_SOURCE_JOURNAL_BYTES,
            "ignoring oversized native source journal"
        );
        let _ = fs::remove_file(&journal_path);
        return NativeSourceChangeJournal {
            overflowed: true,
            ..NativeSourceChangeJournal::default()
        };
    }
    let bytes = match read_bytes_with_limit(
        &journal_path,
        MAX_NATIVE_SOURCE_JOURNAL_BYTES,
        "native source journal",
    ) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(
                path = %journal_path.display(),
                error = %format!("{err:#}"),
                "failed to read native source journal; ignoring"
            );
            return NativeSourceChangeJournal {
                overflowed: true,
                ..NativeSourceChangeJournal::default()
            };
        }
    };
    let decoded = match serde_json::from_slice::<NativeSourceJournalFile>(&bytes) {
        Ok(file) if file.schema_version == NATIVE_SOURCE_JOURNAL_SCHEMA_VERSION => file,
        Ok(_) => return NativeSourceChangeJournal::default(),
        Err(err) => {
            tracing::warn!(
                path = %journal_path.display(),
                error = %err,
                "failed to decode native source journal; treating as overflow fallback"
            );
            return NativeSourceChangeJournal {
                overflowed: true,
                ..NativeSourceChangeJournal::default()
            };
        }
    };
    native_source_journal_state_from_file(decoded)
}

#[cfg(feature = "native-compile")]
fn native_current_source_journal_cursor(workspace_root: &Path) -> u64 {
    let cache_key = native_compile_session_cache_key(workspace_root);
    if let Some(cursor) = native_source_change_watchers()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&cache_key)
        .map(|watcher| {
            watcher
                .journal
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .cursor
        })
    {
        return cursor;
    }
    load_native_source_change_journal(workspace_root).cursor
}

#[cfg(feature = "native-compile")]
fn native_buildinfo_file_from_snapshot(
    signature_hash: String,
    source_root_modified_unix_ms: u64,
    tracked_sources: BTreeMap<String, NativeTrackedFileState>,
    tracked_source_bytes: u64,
    tracked_sources_content_hash: String,
    contract_source_dependencies: BTreeMap<String, BTreeSet<String>>,
    contract_output_plans: Vec<NativeContractOutputPlan>,
    journal_cursor_applied: u64,
) -> NativeBuildInfoFile {
    let tracked_sources_signature = native_tracked_sources_signature(&tracked_sources);
    NativeBuildInfoFile {
        schema_version: NATIVE_BUILDINFO_SCHEMA_VERSION,
        signature_hash,
        source_root_modified_unix_ms,
        tracked_sources,
        tracked_source_bytes,
        tracked_sources_signature,
        tracked_sources_content_hash,
        contract_source_dependencies,
        contract_output_plans,
        journal_cursor_applied,
        generated_at_epoch_ms: epoch_ms_u64().unwrap_or_default(),
    }
}

#[cfg(feature = "native-compile")]
fn native_buildinfo_file_from_state(
    session: &NativeCompileSessionState,
    journal_cursor_applied: u64,
) -> NativeBuildInfoFile {
    native_buildinfo_file_from_snapshot(
        native_compile_session_signature_hash(&session.signature),
        session.source_root_modified_unix_ms,
        session.tracked_sources.clone(),
        session.tracked_source_bytes,
        session.tracked_sources_content_hash.clone(),
        session.contract_source_dependencies.clone(),
        session.contract_output_plans.clone(),
        journal_cursor_applied,
    )
}

#[cfg(feature = "native-compile")]
fn persist_native_buildinfo_sidecar(
    workspace_root: &Path,
    buildinfo: &NativeBuildInfoFile,
) -> Result<()> {
    let sidecar_path = native_buildinfo_sidecar_path(workspace_root)?;
    let parent = sidecar_path
        .parent()
        .context("native buildinfo path has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let bytes = postcard::to_allocvec(buildinfo).context("failed to encode native buildinfo")?;
    if bytes.len() as u64 > MAX_NATIVE_BUILDINFO_BYTES {
        tracing::warn!(
            path = %sidecar_path.display(),
            bytes = bytes.len(),
            max_bytes = MAX_NATIVE_BUILDINFO_BYTES,
            "skipping native buildinfo write: sidecar exceeds size limit"
        );
        return Ok(());
    }
    atomic_write_bytes(&sidecar_path, &bytes, "native buildinfo")
}

#[cfg(feature = "native-compile")]
fn persist_native_buildinfo_sidecar_best_effort(
    workspace_root: &Path,
    buildinfo: &NativeBuildInfoFile,
) {
    if let Err(err) = persist_native_buildinfo_sidecar(workspace_root, buildinfo) {
        tracing::warn!(
            workspace_root = %workspace_root.display(),
            error = %format!("{err:#}"),
            "failed to persist native buildinfo sidecar"
        );
    }
}

#[cfg(feature = "native-compile")]
fn load_native_buildinfo_sidecar_snapshot(
    workspace_root: &Path,
    signature: &NativeCompileSessionSignature,
) -> Option<(NativeBuildInfoSnapshot, u64)> {
    let signature_hash = native_compile_session_signature_hash(signature);
    let candidate_paths = [
        (
            native_buildinfo_sidecar_path(workspace_root),
            "native buildinfo sidecar",
            true,
        ),
        (
            native_buildinfo_sidecar_legacy_path(workspace_root),
            "legacy native buildinfo sidecar",
            false,
        ),
    ];

    for (path_result, label, is_binary) in candidate_paths {
        let sidecar_path = match path_result {
            Ok(path) => path,
            Err(err) => {
                tracing::warn!(
                    workspace_root = %workspace_root.display(),
                    error = %format!("{err:#}"),
                    "{label} path is invalid; ignoring sidecar"
                );
                continue;
            }
        };
        let metadata = match fs::metadata(&sidecar_path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => {
                tracing::warn!(
                    path = %sidecar_path.display(),
                    error = %err,
                    "failed to stat {label}; ignoring"
                );
                continue;
            }
        };
        if metadata.len() > MAX_NATIVE_BUILDINFO_BYTES {
            tracing::warn!(
                path = %sidecar_path.display(),
                bytes = metadata.len(),
                max_bytes = MAX_NATIVE_BUILDINFO_BYTES,
                "ignoring oversized {label}"
            );
            let _ = fs::remove_file(&sidecar_path);
            continue;
        }
        let bytes = match read_bytes_with_limit(&sidecar_path, MAX_NATIVE_BUILDINFO_BYTES, label) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(
                    path = %sidecar_path.display(),
                    error = %format!("{err:#}"),
                    "failed to read {label}; ignoring"
                );
                continue;
            }
        };
        let decoded = if is_binary {
            match postcard::from_bytes::<NativeBuildInfoFile>(&bytes) {
                Ok(file) if file.schema_version == NATIVE_BUILDINFO_SCHEMA_VERSION => file,
                Ok(_) => continue,
                Err(err) => {
                    tracing::warn!(
                        path = %sidecar_path.display(),
                        error = %err,
                        "failed to decode {label}; ignoring"
                    );
                    continue;
                }
            }
        } else {
            match serde_json::from_slice::<NativeBuildInfoFile>(&bytes) {
                Ok(file) => file,
                Err(err) => {
                    tracing::warn!(
                        path = %sidecar_path.display(),
                        error = %err,
                        "failed to decode {label}; ignoring"
                    );
                    continue;
                }
            }
        };
        if decoded.signature_hash != signature_hash {
            continue;
        }
        let legacy_signature = native_tracked_sources_signature(&decoded.tracked_sources);
        let (tracked_sources, tracked_sources_normalized) =
            match native_normalize_tracked_sources(workspace_root, decoded.tracked_sources) {
                Ok(value) => value,
                Err(err) => {
                    tracing::warn!(
                        path = %sidecar_path.display(),
                        error = %format!("{err:#}"),
                        "{label} tracked source set is invalid; ignoring"
                    );
                    continue;
                }
            };
        let tracked_source_bytes = match native_tracked_sources_total_bytes(&tracked_sources) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(
                    path = %sidecar_path.display(),
                    error = %format!("{err:#}"),
                    "{label} tracked source set is invalid; ignoring"
                );
                continue;
            }
        };
        let tracked_sources_signature = native_tracked_sources_signature(&tracked_sources);
        if decoded.tracked_sources_signature != tracked_sources_signature {
            if !(tracked_sources_normalized
                && decoded.tracked_sources_signature == legacy_signature)
            {
                tracing::warn!(
                    path = %sidecar_path.display(),
                    "{label} tracked source signature mismatch; ignoring"
                );
                continue;
            }
            tracing::debug!(
                path = %sidecar_path.display(),
                "{label} tracked source signature normalized from legacy absolute keys"
            );
        }
        let tracked_sources_content_hash =
            if tracked_sources_normalized || decoded.tracked_sources_content_hash.is_empty() {
                match native_tracked_sources_content_hash(workspace_root, &tracked_sources) {
                    Ok(hash) => hash,
                    Err(err) => {
                        tracing::warn!(
                            path = %sidecar_path.display(),
                            error = %format!("{err:#}"),
                            "failed to compute content hash for {label}; ignoring"
                        );
                        continue;
                    }
                }
            } else {
                decoded.tracked_sources_content_hash.clone()
            };
        if tracked_source_bytes != decoded.tracked_source_bytes {
            tracing::warn!(
                path = %sidecar_path.display(),
                sidecar_bytes = decoded.tracked_source_bytes,
                computed_bytes = tracked_source_bytes,
                "{label} tracked-source byte budget drift; using computed value"
            );
        }
        return Some((
            NativeBuildInfoSnapshot {
                tracked_sources,
                tracked_source_bytes,
                tracked_sources_content_hash,
                contract_source_dependencies: decoded.contract_source_dependencies,
                contract_output_plans: decoded.contract_output_plans,
                journal_cursor_applied: decoded.journal_cursor_applied,
            },
            decoded.source_root_modified_unix_ms,
        ));
    }

    None
}

#[cfg(feature = "native-compile")]
fn try_native_buildinfo_sidecar_restore(
    workspace_root: &Path,
    signature: &NativeCompileSessionSignature,
    tracked_sources_content_hash: &str,
) -> Option<NativeBuildInfoSnapshot> {
    let (snapshot, _snapshot_source_root_modified_unix_ms) =
        load_native_buildinfo_sidecar_snapshot(workspace_root, signature)?;
    if snapshot.tracked_sources_content_hash != tracked_sources_content_hash {
        return None;
    }
    Some(snapshot)
}

#[cfg(feature = "native-compile")]
fn try_native_buildinfo_sidecar_restore_with_journal_replay(
    workspace_root: &Path,
    signature: &NativeCompileSessionSignature,
    tracked_sources_content_hash: &str,
) -> Option<NativeBuildInfoSnapshot> {
    let (snapshot, snapshot_source_root_modified_unix_ms) =
        load_native_buildinfo_sidecar_snapshot(workspace_root, signature)?;
    if snapshot.tracked_sources_content_hash == tracked_sources_content_hash {
        return None;
    }
    let journal = load_native_source_change_journal(workspace_root);
    if journal.overflowed {
        return None;
    }
    let journal_applied_cursor = journal.applied_cursor.min(journal.cursor);
    if snapshot.journal_cursor_applied > journal_applied_cursor {
        return None;
    }
    if journal.cursor <= snapshot.journal_cursor_applied {
        return None;
    }
    if journal.changed_files.is_empty() && journal.removed_files.is_empty() {
        return None;
    }
    tracing::debug!(
        workspace_root = %workspace_root.display(),
        sidecar_source_root_mtime = snapshot_source_root_modified_unix_ms,
        current_tracked_sources_content_hash = tracked_sources_content_hash,
        sidecar_journal_cursor = snapshot.journal_cursor_applied,
        journal_cursor = journal.cursor,
        changed_files = journal.changed_files.len(),
        removed_files = journal.removed_files.len(),
        "native buildinfo restore accepted via source-journal replay seed"
    );
    Some(snapshot)
}

#[cfg(feature = "native-compile")]
fn native_crate_cache_root_path(workspace_root: &Path) -> Result<PathBuf> {
    let path = workspace_root.join(".uc/cache/native-session/crate-cache-v1");
    ensure_path_within_root(workspace_root, &path, "native crate cache root")?;
    Ok(path)
}

#[cfg(feature = "native-compile")]
fn native_crate_cache_entry_hash(signature_hash: &str, crate_cache_key: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(b"uc-native-crate-cache-entry-v1");
    hasher.update(signature_hash.as_bytes());
    hasher.update(b"\x1F");
    hasher.update(crate_cache_key.as_bytes());
    hasher.finalize().to_hex().to_string()
}

#[cfg(feature = "native-compile")]
fn native_crate_cache_entry_paths(
    workspace_root: &Path,
    entry_hash: &str,
) -> Result<(PathBuf, PathBuf)> {
    let root = native_crate_cache_root_path(workspace_root)?;
    let blob_path = root.join(format!("{entry_hash}.bin"));
    let entry_path = root.join(format!("{entry_hash}.json"));
    ensure_path_within_root(workspace_root, &blob_path, "native crate cache blob path")?;
    ensure_path_within_root(
        workspace_root,
        &entry_path,
        "native crate cache metadata path",
    )?;
    Ok((blob_path, entry_path))
}

#[cfg(feature = "native-compile")]
fn native_crate_cache_root_fingerprint(root: &Path) -> Result<String> {
    let metadata = fs::metadata(root)
        .with_context(|| format!("failed to stat crate cache root {}", root.display()))?;
    if !metadata.is_dir() {
        bail!("crate cache root is not a directory: {}", root.display());
    }
    let mut files = Vec::<(String, u64, u64, u64)>::new();
    for entry in WalkDir::new(root).follow_links(false).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let is_cairo = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("cairo"));
        if !is_cairo {
            continue;
        }
        if files.len() >= max_fingerprint_files() {
            bail!(
                "native crate cache fingerprint found too many files (>{}) under {}",
                max_fingerprint_files(),
                root.display()
            );
        }
        let metadata = fs::metadata(path).with_context(|| {
            format!(
                "failed to stat crate cache fingerprint file {}",
                path.display()
            )
        })?;
        let modified_unix_ms = metadata_modified_unix_ms(&metadata)?;
        let change_unix_ms = metadata_change_unix_ms(&metadata).unwrap_or(modified_unix_ms);
        let relative = path.strip_prefix(root).unwrap_or(path);
        files.push((
            normalize_fingerprint_path(relative),
            metadata.len(),
            modified_unix_ms,
            change_unix_ms,
        ));
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hasher = Hasher::new();
    hasher.update(b"uc-native-crate-cache-root-fingerprint-v1");
    hasher.update(normalize_fingerprint_path(root).as_bytes());
    hasher.update(&(files.len() as u64).to_le_bytes());
    for (path, size_bytes, modified_unix_ms, change_unix_ms) in files {
        hasher.update(path.as_bytes());
        hasher.update(&size_bytes.to_le_bytes());
        hasher.update(&modified_unix_ms.to_le_bytes());
        hasher.update(&change_unix_ms.to_le_bytes());
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(feature = "native-compile")]
fn native_crate_cache_descriptor_for_crate(
    db: &dyn FilesGroup,
    crate_id: cairo_lang_filesystem::ids::CrateId<'_>,
) -> Option<NativeCrateCacheDescriptor> {
    let crate_config = db.crate_config(crate_id)?;
    let (root, root_fingerprint) = match &crate_config.root {
        Directory::Real(path) => (
            normalize_fingerprint_path(path),
            native_crate_cache_root_fingerprint(path).ok()?,
        ),
        Directory::Virtual { .. } => return None,
    };
    let (name, discriminator) = match crate_id.long(db) {
        CrateLongId::Real {
            name,
            discriminator,
        } => (
            name.to_string(db),
            discriminator.clone().unwrap_or_default(),
        ),
        CrateLongId::Virtual { .. } => return None,
    };
    let cache_key = format!("real:{name}:{discriminator}:{root}:{root_fingerprint}");
    let label = format!(
        "real:{name}{}@{root}",
        if discriminator.is_empty() {
            String::new()
        } else {
            format!("#{discriminator}")
        }
    );
    Some(NativeCrateCacheDescriptor { cache_key, label })
}

#[cfg(feature = "native-compile")]
fn native_restore_crate_cache_into_db(
    workspace_root: &Path,
    signature_hash: &str,
    db: &mut RootDatabase,
) -> NativeCrateCacheRestoreStats {
    if !native_crate_cache_enabled() {
        return NativeCrateCacheRestoreStats::default();
    }
    let crate_ids = db.crate_configs().keys().copied().collect::<Vec<_>>();
    if crate_ids.is_empty() {
        return NativeCrateCacheRestoreStats::default();
    }
    let db_ref: &dyn salsa::Database = db;
    let mut crate_configs = files_group_input(db_ref)
        .crate_configs(db_ref)
        .clone()
        .unwrap_or_default();
    let mut stats = NativeCrateCacheRestoreStats::default();
    let mut updated = false;
    for crate_id in crate_ids {
        let Some(descriptor) = native_crate_cache_descriptor_for_crate(db, crate_id) else {
            stats.skipped = stats.skipped.saturating_add(1);
            continue;
        };
        let entry_hash = native_crate_cache_entry_hash(signature_hash, &descriptor.cache_key);
        let (blob_path, entry_path) =
            match native_crate_cache_entry_paths(workspace_root, &entry_hash) {
                Ok(paths) => paths,
                Err(err) => {
                    tracing::warn!(
                        workspace_root = %workspace_root.display(),
                        error = %format!("{err:#}"),
                        crate_label = %descriptor.label,
                        "native crate cache paths invalid; skipping restore"
                    );
                    stats.rejected = stats.rejected.saturating_add(1);
                    continue;
                }
            };
        if !blob_path.is_file() || !entry_path.is_file() {
            stats.missing = stats.missing.saturating_add(1);
            continue;
        }
        let entry_bytes = match read_bytes_with_limit(
            &entry_path,
            MAX_NATIVE_CRATE_CACHE_ENTRY_BYTES,
            "native crate cache entry",
        ) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(
                    path = %entry_path.display(),
                    error = %format!("{err:#}"),
                    crate_label = %descriptor.label,
                    "failed to read native crate cache metadata; skipping restore"
                );
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            }
        };
        let entry = match serde_json::from_slice::<NativeCrateCacheEntryFile>(&entry_bytes) {
            Ok(entry) if entry.schema_version == NATIVE_CRATE_CACHE_ENTRY_SCHEMA_VERSION => entry,
            Ok(_) => {
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            }
            Err(err) => {
                tracing::warn!(
                    path = %entry_path.display(),
                    error = %err,
                    crate_label = %descriptor.label,
                    "failed to decode native crate cache metadata; skipping restore"
                );
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            }
        };
        if entry.signature_hash != signature_hash || entry.crate_cache_key != descriptor.cache_key {
            stats.rejected = stats.rejected.saturating_add(1);
            continue;
        }
        if entry.blob_size > MAX_NATIVE_CRATE_CACHE_BLOB_BYTES {
            stats.rejected = stats.rejected.saturating_add(1);
            continue;
        }
        let blob_bytes = match read_bytes_with_limit(
            &blob_path,
            MAX_NATIVE_CRATE_CACHE_BLOB_BYTES,
            "native crate cache blob",
        ) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(
                    path = %blob_path.display(),
                    error = %format!("{err:#}"),
                    crate_label = %descriptor.label,
                    "failed to read native crate cache blob; skipping restore"
                );
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            }
        };
        if blob_bytes.len() as u64 != entry.blob_size
            || blake3::hash(&blob_bytes).to_hex().to_string() != entry.blob_hash
        {
            stats.rejected = stats.rejected.saturating_add(1);
            continue;
        }
        let crate_input = crate_id.long(db).clone().into_crate_input(db);
        let Some(existing) = crate_configs.get(&crate_input).cloned() else {
            stats.skipped = stats.skipped.saturating_add(1);
            continue;
        };
        crate_configs.insert(
            crate_input,
            CrateConfigurationInput {
                root: existing.root,
                settings: existing.settings,
                cache_file: Some(BlobLongId::Virtual(blob_bytes)),
            },
        );
        stats.restored = stats.restored.saturating_add(1);
        updated = true;
    }
    if updated {
        set_crate_configs_input(db, crate_configs);
    }
    stats
}

#[cfg(feature = "native-compile")]
fn native_prune_crate_cache_files(workspace_root: &Path, max_bytes: u64) -> Result<()> {
    let root = native_crate_cache_root_path(workspace_root)?;
    if !root.is_dir() {
        return Ok(());
    }
    let mut files = Vec::new();
    let mut sizes_by_path = HashMap::<PathBuf, u64>::new();
    let mut total_bytes = 0_u64;
    for entry in WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let metadata =
            fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
        let modified = metadata_modified_unix_ms(&metadata).unwrap_or_default();
        let size = metadata.len();
        total_bytes = total_bytes.saturating_add(size);
        let path = path.to_path_buf();
        sizes_by_path.insert(path.clone(), size);
        files.push((modified, path));
    }
    if total_bytes <= max_bytes {
        return Ok(());
    }
    files.sort_by_key(|(modified, _)| *modified);
    let mut removed = HashSet::<PathBuf>::new();
    for (_modified, path) in files {
        if total_bytes <= max_bytes {
            break;
        }
        if removed.contains(&path) {
            continue;
        }
        ensure_path_within_root(workspace_root, &path, "native crate cache prune path")?;
        if fs::remove_file(&path).is_ok() {
            removed.insert(path.clone());
            if let Some(size) = sizes_by_path.get(&path) {
                total_bytes = total_bytes.saturating_sub(*size);
            }
            if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
                let companion_extension = match extension {
                    "bin" => Some("json"),
                    "json" => Some("bin"),
                    _ => None,
                };
                if let Some(companion_extension) = companion_extension {
                    let companion = path.with_extension(companion_extension);
                    ensure_path_within_root(
                        workspace_root,
                        &companion,
                        "native crate cache companion prune path",
                    )?;
                    if fs::remove_file(&companion).is_ok() {
                        removed.insert(companion.clone());
                        if let Some(size) = sizes_by_path.get(&companion) {
                            total_bytes = total_bytes.saturating_sub(*size);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(feature = "native-compile")]
fn native_persist_crate_cache_entries(
    workspace_root: &Path,
    signature_hash: &str,
    db: &RootDatabase,
    crate_ids: Vec<cairo_lang_filesystem::ids::CrateId<'_>>,
) -> Result<NativeCrateCachePersistStats> {
    let mut stats = NativeCrateCachePersistStats::default();
    if !native_crate_cache_enabled() || crate_ids.is_empty() {
        return Ok(stats);
    }
    let root = native_crate_cache_root_path(workspace_root)?;
    fs::create_dir_all(&root).with_context(|| format!("failed to create {}", root.display()))?;
    let mut seen = HashSet::new();
    for crate_id in crate_ids {
        let Some(descriptor) = native_crate_cache_descriptor_for_crate(db, crate_id) else {
            stats.skipped = stats.skipped.saturating_add(1);
            continue;
        };
        if !seen.insert(descriptor.cache_key.clone()) {
            continue;
        }
        let blob = match generate_crate_cache(db, crate_id) {
            Ok(blob) => blob,
            Err(err) => {
                tracing::debug!(
                    crate_label = %descriptor.label,
                    error = %err,
                    "native crate cache generation failed for crate"
                );
                stats.failed = stats.failed.saturating_add(1);
                continue;
            }
        };
        if blob.len() as u64 > MAX_NATIVE_CRATE_CACHE_BLOB_BYTES {
            tracing::warn!(
                crate_label = %descriptor.label,
                bytes = blob.len(),
                max_bytes = MAX_NATIVE_CRATE_CACHE_BLOB_BYTES,
                "skipping native crate cache blob: entry exceeds size limit"
            );
            stats.skipped = stats.skipped.saturating_add(1);
            continue;
        }
        let entry_hash = native_crate_cache_entry_hash(signature_hash, &descriptor.cache_key);
        let (blob_path, entry_path) = native_crate_cache_entry_paths(workspace_root, &entry_hash)?;
        let blob_hash = blake3::hash(&blob).to_hex().to_string();
        let entry = NativeCrateCacheEntryFile {
            schema_version: NATIVE_CRATE_CACHE_ENTRY_SCHEMA_VERSION,
            signature_hash: signature_hash.to_string(),
            crate_cache_key: descriptor.cache_key.clone(),
            blob_hash,
            blob_size: blob.len() as u64,
            generated_at_epoch_ms: epoch_ms_u64().unwrap_or_default(),
        };
        let entry_bytes =
            serde_json::to_vec(&entry).context("failed to encode native crate cache metadata")?;
        if entry_bytes.len() as u64 > MAX_NATIVE_CRATE_CACHE_ENTRY_BYTES {
            tracing::warn!(
                crate_label = %descriptor.label,
                bytes = entry_bytes.len(),
                max_bytes = MAX_NATIVE_CRATE_CACHE_ENTRY_BYTES,
                "skipping native crate cache metadata write: entry exceeds size limit"
            );
            stats.skipped = stats.skipped.saturating_add(1);
            continue;
        }
        atomic_write_bytes(&entry_path, &entry_bytes, "native crate cache metadata")?;
        atomic_write_bytes(&blob_path, &blob, "native crate cache blob")?;
        stats.saved = stats.saved.saturating_add(1);
        stats.bytes_written = stats.bytes_written.saturating_add(blob.len() as u64);
    }
    native_prune_crate_cache_files(workspace_root, native_crate_cache_max_bytes())?;
    Ok(stats)
}

#[cfg(feature = "native-compile")]
fn native_should_persist_crate_cache_after_build(
    daemon_context: bool,
    changed_files_count: u64,
    removed_files_count: u64,
    compiled_contracts: u64,
) -> bool {
    daemon_context
        && native_crate_cache_enabled()
        && compiled_contracts != 0
        && changed_files_count == 0
        && removed_files_count == 0
}

#[cfg(feature = "native-compile")]
fn native_persist_crate_cache_after_build_best_effort(
    workspace_root: &Path,
    signature: &NativeCompileSessionSignature,
    daemon_context: bool,
    changed_files_count: u64,
    removed_files_count: u64,
    compiled_contracts: u64,
) {
    if !native_should_persist_crate_cache_after_build(
        daemon_context,
        changed_files_count,
        removed_files_count,
        compiled_contracts,
    ) {
        // Non-daemon invocations should minimize synchronous cold-path overhead.
        // Keeping crate-cache persistence daemon-scoped preserves warm worker use
        // while avoiding heavy per-invocation writes in CLI mode.
        return;
    }
    let signature_hash = native_compile_session_signature_hash(signature);
    let session_handle = match native_compile_session_handle(workspace_root, signature) {
        Ok((handle, _session_cache_hit)) => handle,
        Err(err) => {
            tracing::warn!(
                workspace_root = %workspace_root.display(),
                error = %format!("{err:#}"),
                "failed to access native compile session for crate-cache persist"
            );
            return;
        }
    };
    let (db_snapshot, mut crate_inputs) = {
        let session = session_handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (session.db.snapshot(), session.main_crate_inputs.clone())
    };
    let core_input = CrateLongId::core(&db_snapshot).into_crate_input(&db_snapshot);
    if !crate_inputs.iter().any(|input| input == &core_input) {
        crate_inputs.push(core_input);
    }
    let crate_ids = CrateInput::into_crate_ids(&db_snapshot, crate_inputs.clone());
    match native_persist_crate_cache_entries(
        workspace_root,
        &signature_hash,
        &db_snapshot,
        crate_ids,
    ) {
        Ok(stats) => {
            tracing::debug!(
                workspace_root = %workspace_root.display(),
                signature_hash = %signature_hash,
                saved = stats.saved,
                skipped = stats.skipped,
                failed = stats.failed,
                bytes_written = stats.bytes_written,
                "native crate cache persisted"
            );
        }
        Err(err) => {
            tracing::warn!(
                workspace_root = %workspace_root.display(),
                error = %format!("{err:#}"),
                "failed to persist native crate cache"
            );
        }
    }
}

#[cfg(feature = "native-compile")]
fn native_setup_project(db: &mut RootDatabase, path: &Path) -> Result<Vec<CrateInput>> {
    native_progress_log(format!("native setup_project start ({})", path.display()));
    let started = Instant::now();
    let main_crate_inputs = setup_project(db, path)
        .with_context(|| format!("failed to setup native cairo project {}", path.display()))?;
    native_progress_log(format!(
        "native setup_project finished (main_crates={}, elapsed_ms={:.1})",
        main_crate_inputs.len(),
        started.elapsed().as_secs_f64() * 1000.0
    ));
    Ok(main_crate_inputs)
}

#[cfg(feature = "native-compile")]
fn native_seeded_root_database(corelib_src: &Path) -> Result<RootDatabase> {
    let mut db = RootDatabase::builder()
        .with_optimizations(Optimizations::enabled_with_default_movable_functions(
            InliningStrategy::Default,
        ))
        .with_default_plugin_suite(starknet_plugin_suite())
        .build()
        .context("failed to initialize native cairo compiler database")?;
    init_dev_corelib(&mut db, corelib_src.to_path_buf());
    tracing::debug!(
        corelib_src = %corelib_src.display(),
        "native root database initialized"
    );
    Ok(db)
}

#[cfg(feature = "native-compile")]
fn build_native_compile_session_state(
    workspace_root: &Path,
    signature: NativeCompileSessionSignature,
) -> Result<NativeCompileSessionState> {
    let source_roots = native_compile_source_roots(&signature.context);
    let signature_hash = native_compile_session_signature_hash(&signature);
    native_progress_log(format!(
        "native session-state build start (workspace={}, package={})",
        workspace_root.display(),
        signature.context.package_name
    ));
    let _heartbeat = NativeProgressHeartbeat::start("native session-state build");
    let db_start = Instant::now();
    let mut db = native_seeded_root_database(&signature.context.corelib_src)?;
    let db_init_ms = db_start.elapsed().as_secs_f64() * 1000.0;
    native_progress_log(format!(
        "native session-state db init finished in {:.1}ms",
        db_init_ms
    ));
    native_progress_log(format!(
        "native session-state setup_project start ({})",
        signature.context.cairo_project_dir.display()
    ));
    let setup_start = Instant::now();
    let _setup_heartbeat = NativeProgressHeartbeat::start("native session-state setup_project");
    let main_crate_inputs = native_setup_project(&mut db, &signature.context.cairo_project_dir)?;
    let setup_project_ms = setup_start.elapsed().as_secs_f64() * 1000.0;
    native_progress_log(format!(
        "native session-state setup_project finished in {:.1}ms",
        setup_project_ms
    ));
    let crate_cache_restore_start = Instant::now();
    let crate_cache_restore_stats =
        native_restore_crate_cache_into_db(workspace_root, &signature_hash, &mut db);
    let crate_cache_restore_ms = crate_cache_restore_start.elapsed().as_secs_f64() * 1000.0;
    native_progress_log(format!(
        "native session-state crate-cache restore finished in {:.1}ms (restored={}, missing={}, rejected={})",
        crate_cache_restore_ms,
        crate_cache_restore_stats.restored,
        crate_cache_restore_stats.missing,
        crate_cache_restore_stats.rejected
    ));
    native_progress_log("native session-state source scan start");
    let scan_start = Instant::now();
    let _scan_heartbeat = NativeProgressHeartbeat::start("native session-state source scan");
    let (
        mut precollected_tracked_sources,
        source_root_modified_unix_ms,
        current_tracked_sources_content_hash,
    ) = {
        let (tracked_sources, tracked_source_bytes, latest_source_root_modified_unix_ms) =
            native_collect_tracked_sources_with_source_root_mtime(workspace_root, &source_roots)?;
        let tracked_sources_content_hash =
            native_tracked_sources_content_hash(workspace_root, &tracked_sources)?;
        (
            Some((
                tracked_sources,
                tracked_source_bytes,
                tracked_sources_content_hash.clone(),
            )),
            latest_source_root_modified_unix_ms,
            tracked_sources_content_hash,
        )
    };
    let restored_image = try_native_compile_session_image_restore(
        workspace_root,
        &signature,
        &current_tracked_sources_content_hash,
    );
    let restored_buildinfo = if restored_image.is_none() {
        try_native_buildinfo_sidecar_restore(
            workspace_root,
            &signature,
            &current_tracked_sources_content_hash,
        )
    } else {
        None
    };
    let replayed_buildinfo = if restored_image.is_none() && restored_buildinfo.is_none() {
        try_native_buildinfo_sidecar_restore_with_journal_replay(
            workspace_root,
            &signature,
            &current_tracked_sources_content_hash,
        )
    } else {
        None
    };
    let (
        tracked_sources,
        tracked_source_bytes,
        tracked_sources_content_hash,
        contract_source_dependencies,
        contract_output_plans,
        session_image_hit,
        buildinfo_hit,
        buildinfo_replay_hit,
        journal_cursor_applied,
    ) = if let Some(image) = restored_image {
        (
            image.tracked_sources,
            image.tracked_source_bytes,
            image.tracked_sources_content_hash,
            image.contract_source_dependencies,
            image.contract_output_plans,
            true,
            false,
            false,
            image.journal_cursor_applied,
        )
    } else if let Some(buildinfo) = restored_buildinfo {
        (
            buildinfo.tracked_sources,
            buildinfo.tracked_source_bytes,
            buildinfo.tracked_sources_content_hash,
            buildinfo.contract_source_dependencies,
            buildinfo.contract_output_plans,
            false,
            true,
            false,
            buildinfo.journal_cursor_applied,
        )
    } else if let Some(buildinfo) = replayed_buildinfo {
        (
            buildinfo.tracked_sources,
            buildinfo.tracked_source_bytes,
            buildinfo.tracked_sources_content_hash,
            buildinfo.contract_source_dependencies,
            buildinfo.contract_output_plans,
            false,
            false,
            true,
            buildinfo.journal_cursor_applied,
        )
    } else {
        let (tracked_sources, tracked_source_bytes, tracked_sources_content_hash) =
            if let Some((tracked_sources, tracked_source_bytes, tracked_sources_content_hash)) =
                precollected_tracked_sources.take()
            {
                (
                    tracked_sources,
                    tracked_source_bytes,
                    tracked_sources_content_hash,
                )
            } else {
                let (tracked_sources, tracked_source_bytes) =
                    native_collect_tracked_sources(workspace_root, &source_roots)?;
                let tracked_sources_content_hash =
                    native_tracked_sources_content_hash(workspace_root, &tracked_sources)?;
                (
                    tracked_sources,
                    tracked_source_bytes,
                    tracked_sources_content_hash,
                )
            };
        (
            tracked_sources,
            tracked_source_bytes,
            tracked_sources_content_hash,
            BTreeMap::new(),
            Vec::new(),
            false,
            false,
            false,
            native_current_source_journal_cursor(workspace_root),
        )
    };
    let source_scan_ms = scan_start.elapsed().as_secs_f64() * 1000.0;
    native_progress_log(format!(
        "native session-state source scan finished in {:.1}ms (tracked={}, bytes={}, image_hit={}, buildinfo_hit={}, replay_hit={})",
        source_scan_ms,
        tracked_sources.len(),
        tracked_source_bytes,
        session_image_hit,
        buildinfo_hit,
        buildinfo_replay_hit
    ));
    tracing::debug!(
        workspace_root = %workspace_root.display(),
        db_init_ms,
        setup_project_ms,
        crate_cache_restore_ms,
        crate_cache_restored = crate_cache_restore_stats.restored,
        crate_cache_missing = crate_cache_restore_stats.missing,
        crate_cache_rejected = crate_cache_restore_stats.rejected,
        crate_cache_skipped = crate_cache_restore_stats.skipped,
        source_scan_ms,
        session_image_hit,
        buildinfo_hit,
        buildinfo_replay_hit,
        tracked_sources = tracked_sources.len(),
        source_roots = ?source_roots
            .iter()
            .map(|root| root.display().to_string())
            .collect::<Vec<_>>(),
        "native session state built"
    );
    if native_eager_keyed_slot_prime_enabled() {
        // Optional eager priming for deployments that prefer paying startup work once.
        let tracked_inputs = tracked_sources
            .keys()
            .map(|relative| {
                let absolute_path = workspace_root.join(relative);
                let file_id = FileId::new(&db, FileLongId::OnDisk(absolute_path));
                db.file_input(file_id).clone()
            })
            .collect::<Vec<_>>();
        if !tracked_inputs.is_empty() {
            let inserted = ensure_keyed_file_override_slots(&mut db, tracked_inputs);
            tracing::debug!(
                inserted_slots = inserted,
                tracked_sources = tracked_sources.len(),
                "native session primed keyed file-override slots"
            );
        }
    } else {
        tracing::debug!(
            tracked_sources = tracked_sources.len(),
            "native session skipped eager keyed file-override slot priming"
        );
    }
    let state = NativeCompileSessionState {
        signature,
        db,
        main_crate_inputs,
        tracked_sources,
        tracked_source_bytes,
        tracked_sources_content_hash,
        journal_cursor_applied,
        source_root_modified_unix_ms,
        contract_source_dependencies,
        contract_output_plans,
    };
    if !session_image_hit {
        let snapshot = native_compile_session_image_snapshot_from_state(&state);
        persist_native_compile_session_image_snapshot_best_effort(workspace_root, &snapshot);
    }
    if !buildinfo_hit || !session_image_hit {
        let buildinfo = native_buildinfo_file_from_state(&state, state.journal_cursor_applied);
        persist_native_buildinfo_sidecar_best_effort(workspace_root, &buildinfo);
    }
    native_progress_log(format!(
        "native session-state build finished (tracked={}, journal_cursor={})",
        state.tracked_sources.len(),
        state.journal_cursor_applied
    ));
    Ok(state)
}

#[cfg(feature = "native-compile")]
fn native_compile_session_build_locks() -> &'static Mutex<HashMap<String, Arc<Mutex<()>>>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(feature = "native-compile")]
fn native_compile_session_build_lock(cache_key: &str) -> Arc<Mutex<()>> {
    let mut locks = native_compile_session_build_locks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks
        .entry(cache_key.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

#[cfg(feature = "native-compile")]
fn release_native_compile_session_build_lock(cache_key: &str, build_lock: &Arc<Mutex<()>>) {
    let mut locks = native_compile_session_build_locks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing_lock) = locks.get(cache_key) {
        if Arc::ptr_eq(existing_lock, build_lock) && Arc::strong_count(existing_lock) == 2 {
            locks.remove(cache_key);
        }
    }
}

#[cfg(feature = "native-compile")]
fn native_compile_session_handle(
    workspace_root: &Path,
    signature: &NativeCompileSessionSignature,
) -> Result<(Arc<Mutex<NativeCompileSessionState>>, bool)> {
    let cache_key = native_compile_session_cache_key(workspace_root);
    let now_ms = epoch_ms_u64().unwrap_or_default();
    {
        let mut cache = native_compile_session_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        evict_expired_native_compile_session_cache_entries(
            &mut cache,
            now_ms,
            native_compile_session_cache_ttl_ms(),
        );
        if let Some(entry) = cache.get_mut(&cache_key) {
            entry.last_access_epoch_ms = now_ms;
            return Ok((entry.session.clone(), true));
        }
    }

    // Avoid holding the global cache lock while building RootDatabase. Serialize
    // cache misses per workspace key with a dedicated lock instead.
    let build_lock = native_compile_session_build_lock(&cache_key);
    let build_guard = build_lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let build_result = (|| -> Result<(Arc<Mutex<NativeCompileSessionState>>, bool)> {
        let now_ms = epoch_ms_u64().unwrap_or_default();
        {
            let mut cache = native_compile_session_cache()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            evict_expired_native_compile_session_cache_entries(
                &mut cache,
                now_ms,
                native_compile_session_cache_ttl_ms(),
            );
            if let Some(entry) = cache.get_mut(&cache_key) {
                entry.last_access_epoch_ms = now_ms;
                return Ok((entry.session.clone(), true));
            }
        }

        let state = build_native_compile_session_state(workspace_root, signature.clone())?;
        let estimated_bytes = native_compile_session_state_estimated_bytes(&state);
        let session = Arc::new(Mutex::new(state));
        {
            let mut cache = native_compile_session_cache()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            cache.insert(
                cache_key.clone(),
                NativeCompileSessionCacheEntry {
                    session: session.clone(),
                    last_access_epoch_ms: now_ms,
                    estimated_bytes,
                },
            );
            evict_expired_native_compile_session_cache_entries(
                &mut cache,
                now_ms,
                native_compile_session_cache_ttl_ms(),
            );
            evict_oldest_native_compile_session_cache_entries(
                &mut cache,
                native_compile_session_cache_max_entries(),
                native_compile_session_cache_max_bytes(),
            );
        }
        Ok((session, false))
    })();
    drop(build_guard);
    release_native_compile_session_build_lock(&cache_key, &build_lock);
    build_result
}

#[cfg(feature = "native-compile")]
fn clear_native_compile_session(workspace_root: &Path) {
    let cache_key = native_compile_session_cache_key(workspace_root);
    let mut cache = native_compile_session_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.remove(&cache_key);
    clear_native_source_change_watcher(workspace_root);
}

#[cfg(feature = "native-compile")]
fn native_source_change_watchers() -> &'static Mutex<HashMap<String, NativeSourceChangeWatcher>> {
    static VALUE: OnceLock<Mutex<HashMap<String, NativeSourceChangeWatcher>>> = OnceLock::new();
    VALUE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(feature = "native-compile")]
fn clear_native_source_change_watcher(workspace_root: &Path) {
    let cache_key = native_compile_session_cache_key(workspace_root);
    let mut watchers = native_source_change_watchers()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    watchers.remove(&cache_key);
}

#[cfg(feature = "native-compile")]
fn native_workspace_relative_cairo_path(workspace_root: &Path, path: &Path) -> Option<String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    let tracked_path = absolute
        .strip_prefix(workspace_root)
        .ok()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| absolute.clone());
    if tracked_path.is_relative()
        && tracked_path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return None;
    }
    let contains_src_segment = tracked_path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|segment| segment == "src")
    });
    if !contains_src_segment {
        return None;
    }
    let is_cairo = tracked_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cairo"));
    if !is_cairo {
        return None;
    }
    Some(normalize_fingerprint_path(&tracked_path))
}

#[cfg(feature = "native-compile")]
fn native_record_source_changes(
    workspace_root: &Path,
    journal: &Arc<Mutex<NativeSourceChangeJournal>>,
    changes: impl IntoIterator<Item = (String, bool)>,
) {
    let mut change_count = 0_usize;
    let snapshot = {
        let mut state = journal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for (relative_path, removed) in changes {
            if removed {
                state.changed_files.remove(&relative_path);
                state.removed_files.insert(relative_path);
            } else {
                state.removed_files.remove(&relative_path);
                state.changed_files.insert(relative_path);
            }
            state.cursor = state.cursor.saturating_add(1);
            change_count = change_count.saturating_add(1);
        }
        if change_count == 0 {
            return;
        }
        state.clone()
    };
    persist_native_source_change_journal_best_effort(workspace_root, &snapshot);
}

#[cfg(feature = "native-compile")]
fn native_record_source_change_event(
    workspace_root: &Path,
    journal: &Arc<Mutex<NativeSourceChangeJournal>>,
    event_kind: &NotifyEventKind,
    paths: &[PathBuf],
) {
    let as_relative = |path: &Path| native_workspace_relative_cairo_path(workspace_root, path);
    let mut changes = Vec::<(String, bool)>::new();
    match event_kind {
        NotifyEventKind::Remove(_)
        | NotifyEventKind::Modify(NotifyModifyKind::Name(NotifyRenameMode::From)) => {
            for path in paths {
                if let Some(relative) = as_relative(path) {
                    changes.push((relative, true));
                }
            }
        }
        NotifyEventKind::Modify(NotifyModifyKind::Name(NotifyRenameMode::Both))
            if paths.len() >= 2 =>
        {
            if let Some(relative) = as_relative(&paths[0]) {
                changes.push((relative, true));
            }
            if let Some(relative) = as_relative(&paths[1]) {
                changes.push((relative, false));
            }
        }
        NotifyEventKind::Modify(NotifyModifyKind::Name(NotifyRenameMode::To))
        | NotifyEventKind::Create(_)
        | NotifyEventKind::Modify(_)
        | NotifyEventKind::Any => {
            for path in paths {
                if let Some(relative) = as_relative(path) {
                    changes.push((relative, false));
                }
            }
        }
        _ => {}
    }
    native_record_source_changes(workspace_root, journal, changes);
}

#[cfg(feature = "native-compile")]
fn ensure_native_source_change_watcher(
    workspace_root: &Path,
    source_roots: &[PathBuf],
) -> Result<Arc<Mutex<NativeSourceChangeJournal>>> {
    let cache_key = native_compile_session_cache_key(workspace_root);
    let mut normalized_roots = source_roots
        .iter()
        .map(|path| normalize_fingerprint_path(path))
        .collect::<Vec<_>>();
    normalized_roots.sort();
    normalized_roots.dedup();
    {
        let mut watchers = native_source_change_watchers()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = watchers.get(&cache_key) {
            if existing.watched_roots == normalized_roots {
                return Ok(existing.journal.clone());
            }
        }
        watchers.remove(&cache_key);
    }

    let mut watched_roots = Vec::new();
    for root in source_roots {
        if root.is_dir() {
            watched_roots.push(root.to_path_buf());
        }
    }
    let journal = Arc::new(Mutex::new(load_native_source_change_journal(
        workspace_root,
    )));
    if watched_roots.is_empty() {
        let mut watchers = native_source_change_watchers()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = watchers.get(&cache_key) {
            return Ok(existing.journal.clone());
        }
        watchers.insert(
            cache_key,
            NativeSourceChangeWatcher {
                _watcher: None,
                journal: journal.clone(),
                watched_roots: normalized_roots,
            },
        );
        return Ok(journal);
    }

    let journal_for_events = journal.clone();
    let workspace_root_for_events = workspace_root.to_path_buf();
    let mut watcher = RecommendedWatcher::new(
        move |result: Result<notify::Event, notify::Error>| match result {
            Ok(event) => native_record_source_change_event(
                &workspace_root_for_events,
                &journal_for_events,
                &event.kind,
                &event.paths,
            ),
            Err(err) => {
                let snapshot = {
                    let mut state = journal_for_events
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    state.overflowed = true;
                    state.cursor = state.cursor.saturating_add(1);
                    state.clone()
                };
                persist_native_source_change_journal_best_effort(
                    &workspace_root_for_events,
                    &snapshot,
                );
                tracing::warn!(error = %err, "native source watcher received an error event");
            }
        },
        notify::Config::default(),
    )
    .context("failed to create native source watcher")?;
    for source_root in &watched_roots {
        watcher
            .watch(source_root, RecursiveMode::Recursive)
            .with_context(|| {
                format!(
                    "failed to watch native source root {}",
                    source_root.display()
                )
            })?;
    }
    let mut watchers = native_source_change_watchers()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing) = watchers.get(&cache_key) {
        if existing.watched_roots == normalized_roots {
            return Ok(existing.journal.clone());
        }
    }
    watchers.insert(
        cache_key,
        NativeSourceChangeWatcher {
            _watcher: Some(watcher),
            journal: journal.clone(),
            watched_roots: normalized_roots,
        },
    );
    Ok(journal)
}

#[cfg(feature = "native-compile")]
enum NativeSourceJournalDelta {
    NoChanges {
        commit: NativeSourceJournalCommit,
    },
    Changed {
        changed_files: Vec<String>,
        removed_files: Vec<String>,
        commit: NativeSourceJournalCommit,
    },
    FallbackFullScan {
        commit: NativeSourceJournalCommit,
    },
}

#[cfg(feature = "native-compile")]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct NativeSourceJournalCommit {
    apply_cursor: Option<u64>,
    clear_changed_sets: bool,
    clear_overflow: bool,
}

#[cfg(feature = "native-compile")]
fn native_take_source_journal_delta(
    workspace_root: &Path,
    source_roots: &[PathBuf],
    session_applied_cursor: u64,
) -> NativeSourceJournalDelta {
    let journal = match ensure_native_source_change_watcher(workspace_root, source_roots) {
        Ok(journal) => journal,
        Err(err) => {
            tracing::warn!(
                workspace_root = %workspace_root.display(),
                error = %format!("{err:#}"),
                "native source watcher unavailable; falling back to full source scan"
            );
            return NativeSourceJournalDelta::FallbackFullScan {
                commit: NativeSourceJournalCommit {
                    apply_cursor: None,
                    clear_changed_sets: false,
                    clear_overflow: false,
                },
            };
        }
    };
    let state = journal
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let journal_applied_cursor = state.applied_cursor.min(state.cursor);
    if session_applied_cursor > journal_applied_cursor {
        // We only persist aggregate changed/removed sets, not per-cursor history. If the
        // caller's applied cursor is ahead of the journal's applied cursor, replay from that
        // boundary is ambiguous after journal resets. Force a conservative full scan once and
        // resume journal mode afterwards.
        return NativeSourceJournalDelta::FallbackFullScan {
            commit: NativeSourceJournalCommit {
                apply_cursor: Some(state.cursor),
                clear_changed_sets: true,
                clear_overflow: true,
            },
        };
    }
    if state.overflowed {
        tracing::warn!(
            workspace_root = %workspace_root.display(),
            "native source watcher overflowed; falling back to full source scan"
        );
        return NativeSourceJournalDelta::FallbackFullScan {
            commit: NativeSourceJournalCommit {
                apply_cursor: Some(state.cursor),
                clear_changed_sets: true,
                clear_overflow: true,
            },
        };
    }
    if state.changed_files.is_empty() && state.removed_files.is_empty() {
        return NativeSourceJournalDelta::NoChanges {
            commit: NativeSourceJournalCommit {
                apply_cursor: (journal_applied_cursor != state.cursor).then_some(state.cursor),
                clear_changed_sets: false,
                clear_overflow: false,
            },
        };
    }
    let changed_files = state.changed_files.iter().cloned().collect::<Vec<_>>();
    let removed_files = state.removed_files.iter().cloned().collect::<Vec<_>>();
    NativeSourceJournalDelta::Changed {
        changed_files,
        removed_files,
        commit: NativeSourceJournalCommit {
            apply_cursor: Some(state.cursor),
            clear_changed_sets: true,
            clear_overflow: false,
        },
    }
}

#[cfg(feature = "native-compile")]
fn native_commit_source_journal_delta(
    workspace_root: &Path,
    source_roots: &[PathBuf],
    commit: NativeSourceJournalCommit,
) {
    if commit.apply_cursor.is_none() && !commit.clear_changed_sets && !commit.clear_overflow {
        return;
    }
    let journal = match ensure_native_source_change_watcher(workspace_root, source_roots) {
        Ok(journal) => journal,
        Err(err) => {
            tracing::warn!(
                workspace_root = %workspace_root.display(),
                error = %format!("{err:#}"),
                "native source watcher unavailable while committing journal cursor"
            );
            return;
        }
    };
    let mut state = journal
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let can_clear_for_commit = commit
        .apply_cursor
        .is_some_and(|applied_cursor| applied_cursor >= state.cursor);
    if commit.clear_changed_sets && can_clear_for_commit {
        state.changed_files.clear();
        state.removed_files.clear();
    }
    if commit.clear_overflow && can_clear_for_commit {
        state.overflowed = false;
    }
    if let Some(applied_cursor) = commit.apply_cursor {
        state.applied_cursor = applied_cursor.min(state.cursor);
    } else if state.applied_cursor > state.cursor {
        state.applied_cursor = state.cursor;
    }
    let snapshot = state.clone();
    drop(state);
    persist_native_source_change_journal_best_effort(workspace_root, &snapshot);
}

#[cfg(feature = "native-compile")]
fn native_tracked_sources_total_bytes(
    tracked_sources: &BTreeMap<String, NativeTrackedFileState>,
) -> Result<u64> {
    if tracked_sources.len() > max_fingerprint_files() {
        bail!(
            "native source tracker found too many files (>{}); refusing to continue",
            max_fingerprint_files()
        );
    }
    let mut total_bytes = 0_u64;
    for state in tracked_sources.values() {
        total_bytes = total_bytes.saturating_add(state.size_bytes);
    }
    if total_bytes > max_fingerprint_total_bytes() {
        bail!(
            "native source tracker budget exceeded ({} bytes > {} bytes)",
            total_bytes,
            max_fingerprint_total_bytes()
        );
    }
    Ok(total_bytes)
}

#[cfg(feature = "native-compile")]
fn native_apply_source_change_journal_delta_in_place(
    workspace_root: &Path,
    tracked_sources: &mut BTreeMap<String, NativeTrackedFileState>,
    changed_files: &[String],
    removed_files: &[String],
) -> Result<(u64, Vec<String>, Vec<String>)> {
    let mut effective_changed = BTreeSet::new();
    let mut effective_removed = BTreeSet::new();
    for relative in removed_files {
        if tracked_sources.remove(relative).is_some() {
            effective_removed.insert(relative.clone());
            effective_changed.remove(relative);
        }
    }
    for relative in changed_files {
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            bail!("native source watcher reported invalid relative path: {relative}");
        }
        let absolute_path = workspace_root.join(relative_path);
        ensure_path_within_root(
            workspace_root,
            &absolute_path,
            "native source watcher changed file path",
        )?;
        let metadata = match fs::metadata(&absolute_path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                if tracked_sources.remove(relative).is_some() {
                    effective_removed.insert(relative.clone());
                    effective_changed.remove(relative);
                }
                continue;
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to stat {}", absolute_path.display()));
            }
        };
        let is_cairo = absolute_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("cairo"));
        if !metadata.is_file() || !is_cairo {
            if tracked_sources.remove(relative).is_some() {
                effective_removed.insert(relative.clone());
                effective_changed.remove(relative);
            }
            continue;
        }
        let size_bytes = metadata.len();
        if size_bytes > max_fingerprint_file_bytes() {
            bail!(
                "native source tracker file {} exceeds size limit ({} bytes > {} bytes)",
                absolute_path.display(),
                size_bytes,
                max_fingerprint_file_bytes()
            );
        }
        let next_state = NativeTrackedFileState {
            size_bytes,
            modified_unix_ms: metadata_modified_unix_ms(&metadata)?,
        };
        let previous = tracked_sources.insert(relative.clone(), next_state.clone());
        if previous.as_ref() == Some(&next_state) {
            // Watcher noise can emit modify events without semantic/metadata drift.
            effective_changed.remove(relative);
            effective_removed.remove(relative);
        } else {
            effective_removed.remove(relative);
            effective_changed.insert(relative.clone());
        }
    }
    let total_bytes = native_tracked_sources_total_bytes(tracked_sources)?;
    Ok((
        total_bytes,
        effective_changed.into_iter().collect(),
        effective_removed.into_iter().collect(),
    ))
}

#[cfg(all(feature = "native-compile", test))]
type NativeApplySourceChangeJournalDeltaResult = (
    BTreeMap<String, NativeTrackedFileState>,
    u64,
    Vec<String>,
    Vec<String>,
);

#[cfg(all(feature = "native-compile", test))]
fn native_apply_source_change_journal_delta(
    workspace_root: &Path,
    previous_sources: &BTreeMap<String, NativeTrackedFileState>,
    changed_files: &[String],
    removed_files: &[String],
) -> Result<NativeApplySourceChangeJournalDeltaResult> {
    let mut tracked_sources = previous_sources.clone();
    let (total_bytes, effective_changed, effective_removed) =
        native_apply_source_change_journal_delta_in_place(
            workspace_root,
            &mut tracked_sources,
            changed_files,
            removed_files,
        )?;
    Ok((
        tracked_sources,
        total_bytes,
        effective_changed,
        effective_removed,
    ))
}

#[cfg(feature = "native-compile")]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum NativeSessionRefreshAction {
    None,
    IncrementalChangedSet,
    FullRebuild,
}

#[cfg(feature = "native-compile")]
fn native_session_refresh_action(
    needs_full_rebuild: bool,
    changed_source_set_detected: bool,
    changed_files: usize,
    removed_files: usize,
) -> NativeSessionRefreshAction {
    if needs_full_rebuild {
        return NativeSessionRefreshAction::FullRebuild;
    }
    if !changed_source_set_detected {
        return NativeSessionRefreshAction::None;
    }
    let changed_total = changed_files.saturating_add(removed_files);
    if changed_total > native_incremental_max_changed_files() {
        return NativeSessionRefreshAction::FullRebuild;
    }
    NativeSessionRefreshAction::IncrementalChangedSet
}

#[cfg(feature = "native-compile")]
fn native_should_force_full_rebuild_on_empty_delta(
    rebuild_on_empty_delta: bool,
    session_cache_hit: bool,
    needs_full_rebuild: bool,
    changed_source_set_detected: bool,
) -> bool {
    rebuild_on_empty_delta
        && session_cache_hit
        && !needs_full_rebuild
        && !changed_source_set_detected
}

#[cfg(feature = "native-compile")]
fn native_should_try_cached_noop_reuse(
    changed_files: &[String],
    removed_files: &[String],
    journal_fallback_full_scan: bool,
) -> bool {
    changed_files.is_empty() && removed_files.is_empty() && !journal_fallback_full_scan
}

#[cfg(feature = "native-compile")]
fn native_impacted_subset_used(total_contracts: usize, compiled_contracts: usize) -> bool {
    compiled_contracts > 0 && compiled_contracts < total_contracts
}

#[cfg(feature = "native-compile")]
fn native_apply_file_keyed_session_updates(
    db: &mut RootDatabase,
    workspace_root: &Path,
    changed_files: &[String],
    removed_files: &[String],
) -> Result<bool> {
    let mut changed_updates = Vec::with_capacity(changed_files.len());
    let mut removed_updates = Vec::with_capacity(removed_files.len());
    for relative in changed_files {
        let relative_path = Path::new(relative);
        if relative_path.is_absolute() {
            bail!(
                "native changed-file path must be relative: {}",
                relative_path.display()
            );
        }
        let absolute_path = workspace_root.join(relative_path);
        ensure_path_within_root(
            workspace_root,
            &absolute_path,
            "native changed-file override path",
        )?;
        let content = fs::read_to_string(&absolute_path)
            .with_context(|| format!("failed to read {}", absolute_path.display()))?;
        let file_id = FileId::new(db, FileLongId::OnDisk(absolute_path));
        let file = db.file_input(file_id).clone();
        changed_updates.push((file, Some(Arc::<str>::from(content))));
    }
    for relative in removed_files {
        let relative_path = Path::new(relative);
        if relative_path.is_absolute() {
            bail!(
                "native removed-file path must be relative: {}",
                relative_path.display()
            );
        }
        let absolute_path = workspace_root.join(relative_path);
        ensure_path_within_root(
            workspace_root,
            &absolute_path,
            "native removed-file override path",
        )?;
        let file_id = FileId::new(db, FileLongId::OnDisk(absolute_path));
        let file = db.file_input(file_id).clone();
        let slot_exists = files_group_input(db)
            .keyed_file_overrides(db)
            .as_ref()
            .is_some_and(|overrides| overrides.contains_key(&file));
        if slot_exists {
            removed_updates.push((file, None));
        }
    }
    if !changed_updates.is_empty() {
        let _ = ensure_keyed_file_override_slots(
            db,
            changed_updates.iter().map(|(file, _)| file.clone()),
        );
    }
    let mut updates = changed_updates;
    updates.extend(removed_updates);
    let mut overrides_changed = false;
    for (file, content) in updates {
        if set_file_override_content_keyed(db, file, content) {
            overrides_changed = true;
        }
    }
    if !overrides_changed {
        return Ok(false);
    }
    Ok(true)
}

#[cfg(feature = "native-compile")]
fn with_native_compile_session<T>(
    workspace_root: &Path,
    signature: &NativeCompileSessionSignature,
    daemon_context: bool,
    rebuild_on_empty_delta: bool,
    f: impl FnOnce(&NativeCompileSessionSnapshot) -> Result<T>,
) -> Result<T> {
    let source_roots = native_compile_source_roots(&signature.context);
    let (session_handle, session_cache_hit) =
        native_compile_session_handle(workspace_root, signature)?;
    native_progress_log(format!(
        "native session refresh start (cache_hit={}, daemon_context={}, rebuild_on_empty_delta={})",
        session_cache_hit, daemon_context, rebuild_on_empty_delta
    ));
    let _heartbeat = NativeProgressHeartbeat::start("native session refresh");
    let (needs_full_rebuild, session_applied_cursor) = {
        let session = session_handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (
            session.signature != *signature,
            session.journal_cursor_applied,
        )
    };
    let snapshot_previous_sources = || {
        let session = session_handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        session.tracked_sources.clone()
    };
    let mut source_delta_applied_pre_refresh = false;
    let mut source_journal_commit: Option<NativeSourceJournalCommit> = None;
    let drift_scan_start = Instant::now();
    let (
        changed_files,
        removed_files,
        current_source_snapshot,
        source_root_mtime,
        mut journal_fallback_full_scan,
    ) = if needs_full_rebuild {
        (Vec::new(), Vec::new(), None, 0_u64, false)
    } else if daemon_context {
        match native_take_source_journal_delta(
            workspace_root,
            &source_roots,
            session_applied_cursor,
        ) {
            NativeSourceJournalDelta::NoChanges { commit } => {
                source_journal_commit = Some(commit);
                (Vec::new(), Vec::new(), None, 0_u64, false)
            }
            NativeSourceJournalDelta::Changed {
                changed_files,
                removed_files,
                commit,
            } => {
                source_journal_commit = Some(commit);
                let mut session = session_handle
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                match native_apply_source_change_journal_delta_in_place(
                    workspace_root,
                    &mut session.tracked_sources,
                    &changed_files,
                    &removed_files,
                ) {
                    Ok((
                        tracked_source_bytes,
                        effective_changed_files,
                        effective_removed_files,
                    )) => {
                        session.tracked_source_bytes = tracked_source_bytes;
                        source_delta_applied_pre_refresh = true;
                        (
                            effective_changed_files,
                            effective_removed_files,
                            None,
                            0_u64,
                            false,
                        )
                    }
                    Err(err) => {
                        drop(session);
                        tracing::warn!(
                            workspace_root = %workspace_root.display(),
                            error = %format!("{err:#}"),
                            "native source watcher delta rejected; falling back to full source scan"
                        );
                        let previous_sources = snapshot_previous_sources();
                        let (current_sources, current_source_bytes) =
                            native_collect_tracked_sources(workspace_root, &source_roots)?;
                        let current_sources_content_hash =
                            native_tracked_sources_content_hash(workspace_root, &current_sources)?;
                        let (changed_files, removed_files) =
                            native_diff_tracked_sources(&previous_sources, &current_sources);
                        let source_root_mtime =
                            native_source_roots_modified_unix_ms(workspace_root, &source_roots)?;
                        (
                            changed_files,
                            removed_files,
                            Some((
                                current_sources,
                                current_source_bytes,
                                current_sources_content_hash,
                            )),
                            source_root_mtime,
                            true,
                        )
                    }
                }
            }
            NativeSourceJournalDelta::FallbackFullScan { commit } => {
                source_journal_commit = Some(commit);
                let previous_sources = snapshot_previous_sources();
                let (current_sources, current_source_bytes) =
                    native_collect_tracked_sources(workspace_root, &source_roots)?;
                let current_sources_content_hash =
                    native_tracked_sources_content_hash(workspace_root, &current_sources)?;
                let (changed_files, removed_files) =
                    native_diff_tracked_sources(&previous_sources, &current_sources);
                let source_root_mtime =
                    native_source_roots_modified_unix_ms(workspace_root, &source_roots)?;
                (
                    changed_files,
                    removed_files,
                    Some((
                        current_sources,
                        current_source_bytes,
                        current_sources_content_hash,
                    )),
                    source_root_mtime,
                    true,
                )
            }
        }
    } else {
        let previous_sources = snapshot_previous_sources();
        let (current_sources, current_source_bytes) =
            native_collect_tracked_sources(workspace_root, &source_roots)?;
        let current_sources_content_hash =
            native_tracked_sources_content_hash(workspace_root, &current_sources)?;
        let (changed_files, removed_files) =
            native_diff_tracked_sources(&previous_sources, &current_sources);
        let source_root_mtime =
            native_source_roots_modified_unix_ms(workspace_root, &source_roots)?;
        (
            changed_files,
            removed_files,
            Some((
                current_sources,
                current_source_bytes,
                current_sources_content_hash,
            )),
            source_root_mtime,
            false,
        )
    };
    let drift_scan_ms = drift_scan_start.elapsed().as_secs_f64() * 1000.0;
    native_progress_log(format!(
        "native session refresh drift scan finished in {:.1}ms (changed={}, removed={}, journal_full_scan={})",
        drift_scan_ms,
        changed_files.len(),
        removed_files.len(),
        journal_fallback_full_scan
    ));

    let changed_source_set_detected = !changed_files.is_empty() || !removed_files.is_empty();
    let force_full_rebuild_on_empty_delta = native_should_force_full_rebuild_on_empty_delta(
        rebuild_on_empty_delta,
        session_cache_hit,
        needs_full_rebuild,
        changed_source_set_detected,
    );
    if force_full_rebuild_on_empty_delta {
        // Cache-miss compile paths should not trust an empty changed-file delta.
        // File watcher latency or coarse filesystem mtimes can transiently hide edits.
        journal_fallback_full_scan = true;
        tracing::debug!(
            changed_files = changed_files.len(),
            removed_files = removed_files.len(),
            "native compile forcing conservative full rebuild on empty changed-file delta"
        );
    }
    let refresh_action = if force_full_rebuild_on_empty_delta {
        NativeSessionRefreshAction::FullRebuild
    } else {
        native_session_refresh_action(
            needs_full_rebuild,
            changed_source_set_detected,
            changed_files.len(),
            removed_files.len(),
        )
    };
    native_progress_log(format!(
        "native session refresh action = {:?}",
        refresh_action
    ));
    if matches!(refresh_action, NativeSessionRefreshAction::FullRebuild)
        && changed_source_set_detected
    {
        tracing::debug!(
            changed_files = changed_files.len(),
            removed_files = removed_files.len(),
            max_incremental = native_incremental_max_changed_files(),
            "native session drift exceeded incremental threshold; forcing full rebuild"
        );
    }
    record_native_refresh_telemetry(refresh_action, changed_files.len(), removed_files.len());
    // Keep the worker state persistent, but refresh it only when a changed-file
    // set indicates source drift or when manifest/context signatures changed.
    let mut rebuilt_state = if matches!(refresh_action, NativeSessionRefreshAction::FullRebuild) {
        Some(build_native_compile_session_state(
            workspace_root,
            signature.clone(),
        )?)
    } else {
        None
    };
    let mut current_source_snapshot = current_source_snapshot;
    let estimated_bytes;
    let (snapshot, image_snapshot, buildinfo_snapshot) = {
        let mut session = session_handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut state_mutated = source_delta_applied_pre_refresh;
        if session.signature != *signature {
            if let Some(state) = rebuilt_state.take() {
                *session = state;
            } else {
                *session = build_native_compile_session_state(workspace_root, signature.clone())?;
            }
            state_mutated = true;
        } else if matches!(
            refresh_action,
            NativeSessionRefreshAction::IncrementalChangedSet
        ) {
            let (by_source, dependency_index_complete) = native_contract_source_index(
                &session.contract_output_plans,
                &session.contract_source_dependencies,
            );
            let (scoped_changed_files, scoped_removed_files) =
                native_filter_changed_files_to_contract_source_index(
                    &changed_files,
                    &removed_files,
                    &by_source,
                    dependency_index_complete,
                );
            if !scoped_changed_files.is_empty() || !scoped_removed_files.is_empty() {
                // Prefer file-keyed override updates for relevant source changes to avoid coarse
                // full-DB invalidation on every incremental refresh.
                match native_apply_file_keyed_session_updates(
                    &mut session.db,
                    workspace_root,
                    &scoped_changed_files,
                    &scoped_removed_files,
                ) {
                    Ok(applied_override_update) => {
                        if applied_override_update {
                            tracing::debug!(
                                scoped_changed_files = scoped_changed_files.len(),
                                scoped_removed_files = scoped_removed_files.len(),
                                changed_files = changed_files.len(),
                                removed_files = removed_files.len(),
                                drift_scan_ms,
                                "native session applied file-keyed incremental updates"
                            );
                        } else {
                            tracing::debug!(
                                scoped_changed_files = scoped_changed_files.len(),
                                scoped_removed_files = scoped_removed_files.len(),
                                changed_files = changed_files.len(),
                                removed_files = removed_files.len(),
                                drift_scan_ms,
                                "native session file-keyed incremental updates resolved to no-op"
                            );
                        }
                    }
                    Err(err) => {
                        // Keep the original (non-scoped) changed/removed sets on keyed-update
                        // failure so downstream compilation stays conservative.
                        tracing::warn!(
                            workspace_root = %workspace_root.display(),
                            changed_files = changed_files.len(),
                            removed_files = removed_files.len(),
                            error = %format!("{err:#}"),
                            "native file-keyed update failed; rebuilding native compile session state"
                        );
                        if let Some(state) = rebuilt_state.take() {
                            *session = state;
                        } else {
                            *session = build_native_compile_session_state(
                                workspace_root,
                                signature.clone(),
                            )?;
                        }
                        state_mutated = true;
                    }
                }
            } else {
                tracing::debug!(
                    changed_files = changed_files.len(),
                    removed_files = removed_files.len(),
                    drift_scan_ms,
                    "native changed-file set does not affect tracked contracts; skipping file-keyed session update"
                );
            }
            if let Some((tracked_sources, tracked_source_bytes, tracked_sources_content_hash)) =
                current_source_snapshot.take()
            {
                session.tracked_sources = tracked_sources;
                session.tracked_source_bytes = tracked_source_bytes;
                session.tracked_sources_content_hash = tracked_sources_content_hash;
                state_mutated = true;
            } else if source_delta_applied_pre_refresh {
                session.tracked_sources_content_hash =
                    native_tracked_sources_content_hash(workspace_root, &session.tracked_sources)?;
                state_mutated = true;
            }
            if source_root_mtime != 0 {
                session.source_root_modified_unix_ms = source_root_mtime;
                state_mutated = true;
            }
        } else if source_root_mtime != 0
            && session.source_root_modified_unix_ms != source_root_mtime
        {
            session.source_root_modified_unix_ms = source_root_mtime;
            state_mutated = true;
        }
        estimated_bytes = native_compile_session_state_estimated_bytes(&session);
        (
            NativeCompileSessionSnapshot {
                db: session.db.snapshot(),
                main_crate_inputs: session.main_crate_inputs.clone(),
                changed_files: changed_files.clone(),
                removed_files: removed_files.clone(),
                journal_fallback_full_scan,
                contract_source_dependencies: session.contract_source_dependencies.clone(),
                contract_output_plans: session.contract_output_plans.clone(),
            },
            state_mutated.then(|| native_compile_session_image_snapshot_from_state(&session)),
            state_mutated.then(|| {
                native_buildinfo_file_from_state(&session, session.journal_cursor_applied)
            }),
        )
    };
    update_native_compile_session_cached_estimated_bytes(workspace_root, estimated_bytes);
    if let Some(image_snapshot) = image_snapshot {
        persist_native_compile_session_image_snapshot_best_effort(workspace_root, &image_snapshot);
    }
    if let Some(buildinfo_snapshot) = buildinfo_snapshot {
        persist_native_buildinfo_sidecar_best_effort(workspace_root, &buildinfo_snapshot);
    }
    native_progress_log("native session refresh invoking build closure");
    let result = f(&snapshot);
    native_progress_log(format!(
        "native session refresh closure finished (ok={})",
        result.is_ok()
    ));
    if result.is_ok() && daemon_context {
        if let Some(commit) = source_journal_commit {
            native_commit_source_journal_delta(workspace_root, &source_roots, commit);
            if let Some(applied_cursor) = commit.apply_cursor {
                let (image_snapshot, buildinfo_snapshot) = {
                    let mut session = session_handle
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    let next_cursor =
                        applied_cursor.min(native_current_source_journal_cursor(workspace_root));
                    if session.journal_cursor_applied == next_cursor {
                        (None, None)
                    } else {
                        session.journal_cursor_applied = next_cursor;
                        (
                            Some(native_compile_session_image_snapshot_from_state(&session)),
                            Some(native_buildinfo_file_from_state(&session, next_cursor)),
                        )
                    }
                };
                if let Some(image_snapshot) = image_snapshot {
                    persist_native_compile_session_image_snapshot_best_effort(
                        workspace_root,
                        &image_snapshot,
                    );
                }
                if let Some(buildinfo_snapshot) = buildinfo_snapshot {
                    persist_native_buildinfo_sidecar_best_effort(
                        workspace_root,
                        &buildinfo_snapshot,
                    );
                }
            }
        }
    }
    result
}

#[cfg(feature = "native-compile")]
fn run_native_build(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    workspace_root: &Path,
    profile: &str,
    daemon_context: bool,
) -> Result<(CommandRun, NativeBuildPhaseTelemetry, Vec<String>)> {
    if daemon_context {
        ensure_native_daemon_backend_available()?;
    }
    let result = std::panic::catch_unwind(|| {
        run_native_build_inner(
            common,
            manifest_path,
            workspace_root,
            profile,
            daemon_context,
        )
    });
    match result {
        Ok(result) => result,
        Err(payload) => {
            clear_native_compile_session(workspace_root);
            if daemon_context {
                mark_native_daemon_backend_poisoned();
            }
            let message = if let Some(text) = payload.downcast_ref::<&str>() {
                (*text).to_string()
            } else if let Some(text) = payload.downcast_ref::<String>() {
                text.clone()
            } else {
                "unknown panic payload".to_string()
            };
            Err(native_fallback_eligible_error(format!(
                "native compiler panicked: {message}"
            )))
        }
    }
}

#[cfg(not(feature = "native-compile"))]
fn run_native_build(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    workspace_root: &Path,
    profile: &str,
    daemon_context: bool,
) -> Result<(CommandRun, NativeBuildPhaseTelemetry, Vec<String>)> {
    let _ = (
        common,
        manifest_path,
        workspace_root,
        profile,
        daemon_context,
    );
    Err(native_fallback_eligible_error(
        "native compile backend is disabled at build time; rebuild uc with `native-compile` feature",
    ))
}

#[cfg(feature = "native-compile")]
fn validate_native_profile_name(profile: &str) -> Result<()> {
    let mut components = Path::new(profile).components();
    let Some(first) = components.next() else {
        bail!("native build profile must not be empty");
    };
    if profile.chars().any(|ch| ch == '\0') {
        bail!("native build profile must not contain NUL bytes: {profile:?}");
    }
    if components.next().is_some() || !matches!(first, Component::Normal(_)) {
        bail!("native build profile contains invalid path component: {profile}");
    }
    Ok(())
}

#[cfg(feature = "native-compile")]
fn native_compiler_config<'a>(
    main_crate_inputs: &'a [CrateInput],
    profile: &str,
    capture_statement_locations: bool,
) -> CompilerConfig<'a> {
    let mut compiler_config = CompilerConfig::default();
    compiler_config.diagnostics_reporter = compiler_config
        .diagnostics_reporter
        .with_crates(main_crate_inputs)
        // Match Scarb UX: warnings should not fail `build`.
        .allow_warnings();
    // Scarb built-in profiles default to `sierra-replace-ids = true` for dev-like
    // profiles and `false` for release; mirror that behavior for parity.
    compiler_config.replace_ids = profile != "release";
    compiler_config.add_statements_code_locations = capture_statement_locations;
    compiler_config
}

#[cfg(feature = "native-compile")]
fn native_should_capture_statement_locations_with_flags(
    base_enabled: bool,
    capture_on_cold: bool,
    changed_files: &[String],
    removed_files: &[String],
    cold_compile: bool,
) -> bool {
    if !base_enabled {
        return false;
    }
    if cold_compile || (changed_files.is_empty() && removed_files.is_empty()) {
        return capture_on_cold;
    }
    true
}

#[cfg(feature = "native-compile")]
fn native_should_capture_statement_locations(
    changed_files: &[String],
    removed_files: &[String],
    cold_compile: bool,
) -> bool {
    native_should_capture_statement_locations_with_flags(
        native_capture_statement_locations(),
        native_capture_statement_locations_on_cold(),
        changed_files,
        removed_files,
        cold_compile,
    )
}

#[cfg(feature = "native-compile")]
fn native_target_dir(workspace_root: &Path, profile: &str) -> Result<PathBuf> {
    validate_native_profile_name(profile)?;
    let target_dir = workspace_root.join("target").join(profile);
    ensure_path_within_root(workspace_root, &target_dir, "native build target directory")?;
    Ok(target_dir)
}

#[cfg(feature = "native-compile")]
fn write_native_sierra_artifact(
    target_dir: &Path,
    package_name: &str,
    output_name: &str,
    sierra_program: &str,
) -> Result<()> {
    let output_path = target_dir.join(output_name);
    ensure_path_within_root(target_dir, &output_path, "native sierra artifact path")?;
    let mut keep_files = BTreeSet::new();
    keep_files.insert(output_name.to_string());
    fs::write(&output_path, sierra_program)
        .with_context(|| format!("failed to write native artifact {}", output_name))?;
    prune_native_target_outputs(target_dir, package_name, &keep_files)?;
    Ok(())
}

#[inline(never)]
#[cfg(feature = "native-compile")]
fn run_native_build_inner(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    workspace_root: &Path,
    profile: &str,
    daemon_context: bool,
) -> Result<(CommandRun, NativeBuildPhaseTelemetry, Vec<String>)> {
    let started = Instant::now();
    let context_start = Instant::now();
    let context = build_native_compile_context(common, manifest_path, workspace_root)?;
    let context_ms = context_start.elapsed().as_secs_f64() * 1000.0;
    let target_dir_start = Instant::now();
    let target_dir = native_target_dir(workspace_root, profile)?;
    fs::create_dir_all(&target_dir)
        .with_context(|| format!("failed to create {}", target_dir.display()))?;
    let target_dir_ms = target_dir_start.elapsed().as_secs_f64() * 1000.0;
    let signature = native_compile_session_signature(manifest_path, &context);
    let compile_start = Instant::now();
    let session_scope_start = Instant::now();
    let (
        artifact_count,
        frontend_compile_ms,
        casm_ms,
        artifact_write_ms,
        produced_paths,
        changed_files_count,
        removed_files_count,
        total_contracts,
        compiled_contracts,
        impacted_subset_used,
        journal_fallback_full_scan,
        dependency_updates,
        contract_output_plans,
    ) = with_native_compile_session(
        workspace_root,
        &signature,
        daemon_context,
        true,
        |session| {
            tracing::trace!(
                changed_files = session.changed_files.len(),
                removed_files = session.removed_files.len(),
                "native session snapshot delta"
            );
            // Keep incremental changed-file builds captured for dependency indexing, but let cold/noop
            // behavior follow the dedicated cold flag to control startup overhead.
            let capture_statement_locations = native_should_capture_statement_locations(
                &session.changed_files,
                &session.removed_files,
                session.contract_output_plans.is_empty(),
            );
            if native_should_try_cached_noop_reuse(
                &session.changed_files,
                &session.removed_files,
                session.journal_fallback_full_scan,
            ) {
                if let Some(keep_files) = native_cached_noop_keep_files(
                    &target_dir,
                    &context.package_name,
                    &session.contract_output_plans,
                )? {
                    let produced_paths = keep_files.iter().cloned().collect::<Vec<_>>();
                    tracing::debug!(
                        contracts = session.contract_output_plans.len(),
                        "native compile reused cached artifacts for unchanged source set"
                    );
                    return Ok((
                        keep_files.len(),
                        0.0,
                        0.0,
                        0.0,
                        produced_paths,
                        0_u64,
                        0_u64,
                        session.contract_output_plans.len() as u64,
                        0_u64,
                        false,
                        session.journal_fallback_full_scan,
                        Vec::new(),
                        None,
                    ));
                }
            }
            if !session.contract_output_plans.is_empty()
                && (!session.changed_files.is_empty() || !session.removed_files.is_empty())
            {
                if !native_changed_files_affect_tracked_contracts(
                    &session.changed_files,
                    &session.removed_files,
                    &session.contract_output_plans,
                    &session.contract_source_dependencies,
                ) {
                    if let Some(keep_files) = native_cached_noop_keep_files(
                        &target_dir,
                        &context.package_name,
                        &session.contract_output_plans,
                    )? {
                        let produced_paths = keep_files.iter().cloned().collect::<Vec<_>>();
                        tracing::debug!(
                            changed_files = session.changed_files.len(),
                            removed_files = session.removed_files.len(),
                            contracts = session.contract_output_plans.len(),
                            "native compile reused cached artifacts for changed files outside the tracked contract set"
                        );
                        return Ok((
                            keep_files.len(),
                            0.0,
                            0.0,
                            0.0,
                            produced_paths,
                            session.changed_files.len() as u64,
                            session.removed_files.len() as u64,
                            session.contract_output_plans.len() as u64,
                            0_u64,
                            false,
                            session.journal_fallback_full_scan,
                            Vec::new(),
                            None,
                        ));
                    }
                }
            }
            let crate_ids =
                CrateInput::into_crate_ids(&session.db, session.main_crate_inputs.iter().cloned());
            let (contracts, mut module_paths, mut contract_source_paths, mut all_plans) =
                if session.changed_files.is_empty()
                    && session.removed_files.is_empty()
                    && !session.contract_output_plans.is_empty()
                {
                    native_progress_log(format!(
                        "native contract discovery fast path start (persisted={})",
                        session.contract_output_plans.len()
                    ));
                    let resolve_started_at = Instant::now();
                    if let Some(contracts) = native_resolve_contracts_from_output_plans(
                        &session.db,
                        &crate_ids,
                        &session.contract_output_plans,
                    ) {
                        native_progress_log(format!(
                        "native contract discovery fast path finished in {:.1}ms (contracts={})",
                        resolve_started_at.elapsed().as_secs_f64() * 1000.0,
                        contracts.len()
                    ));
                        (
                            contracts,
                            session
                                .contract_output_plans
                                .iter()
                                .map(|plan| plan.module_path.clone())
                                .collect::<Vec<_>>(),
                            Vec::new(),
                            session.contract_output_plans.clone(),
                        )
                    } else {
                        native_progress_log(
                            "native contract discovery fast path missed; falling back to full scan",
                        );
                        native_progress_log(format!(
                            "native find_contracts start (changed={}, removed={})",
                            session.changed_files.len(),
                            session.removed_files.len()
                        ));
                        let find_contracts_started_at = Instant::now();
                        let contracts = find_contracts(&session.db, &crate_ids);
                        native_progress_log(format!(
                            "native find_contracts finished in {:.1}ms (contracts={})",
                            find_contracts_started_at.elapsed().as_secs_f64() * 1000.0,
                            contracts.len()
                        ));
                        (contracts, Vec::new(), Vec::new(), Vec::new())
                    }
                } else {
                    native_progress_log(format!(
                        "native find_contracts start (changed={}, removed={})",
                        session.changed_files.len(),
                        session.removed_files.len()
                    ));
                    let find_contracts_started_at = Instant::now();
                    let contracts = find_contracts(&session.db, &crate_ids);
                    native_progress_log(format!(
                        "native find_contracts finished in {:.1}ms (contracts={})",
                        find_contracts_started_at.elapsed().as_secs_f64() * 1000.0,
                        contracts.len()
                    ));
                    (contracts, Vec::new(), Vec::new(), Vec::new())
                };

            if contracts.is_empty() {
                native_progress_log("native cairo program compile start");
                let _heartbeat = NativeProgressHeartbeat::start("native cairo program compile");
                let frontend_compile_start = Instant::now();
                let program = compile_prepared_db_program(
                    &session.db,
                    crate_ids,
                    native_compiler_config(
                        &session.main_crate_inputs,
                        profile,
                        capture_statement_locations,
                    ),
                )
                .map_err(|err| {
                    mark_native_fallback_eligible_for_external_dependencies(
                        err.context("native cairo compile failed"),
                        &context,
                    )
                })?;
                let frontend_compile_ms = frontend_compile_start.elapsed().as_secs_f64() * 1000.0;
                native_progress_log(format!(
                    "native cairo program compile finished in {:.1}ms",
                    frontend_compile_ms
                ));
                let artifact_write_start = Instant::now();
                let output_name = format!("{}.sierra", context.package_name);
                write_native_sierra_artifact(
                    &target_dir,
                    &context.package_name,
                    &output_name,
                    &program.to_string(),
                )?;
                let artifact_write_ms = artifact_write_start.elapsed().as_secs_f64() * 1000.0;
                return Ok((
                    1,
                    frontend_compile_ms,
                    0.0,
                    artifact_write_ms,
                    vec![output_name],
                    session.changed_files.len() as u64,
                    session.removed_files.len() as u64,
                    0_u64,
                    0_u64,
                    false,
                    session.journal_fallback_full_scan,
                    Vec::new(),
                    Some(Vec::new()),
                ));
            }

            if module_paths.is_empty() {
                module_paths = contracts
                    .iter()
                    .map(|contract| contract.submodule_id.full_path(&session.db))
                    .collect();
            }
            if contract_source_paths.is_empty() {
                contract_source_paths = contracts
                    .iter()
                    .map(|contract| {
                        native_contract_source_relative_path(&session.db, workspace_root, contract)
                    })
                    .collect();
            }
            if all_plans.is_empty() {
                let contract_stems = native_contract_file_stems(&module_paths);
                all_plans.reserve(module_paths.len());
                for (module_path, contract_stem) in module_paths.iter().zip(contract_stems.iter()) {
                    let package_name = native_contract_package_name(module_path).to_string();
                    let contract_name = native_contract_name(module_path).to_string();
                    let artifact_id = native_starknet_artifact_id(&package_name, module_path);
                    let file_stem = format!(
                        "{}_{}",
                        context.package_name,
                        sanitize_artifact_component(contract_stem)
                    );
                    let artifact_file = format!("{file_stem}.contract_class.json");
                    let casm_file = context
                        .starknet_target
                        .casm
                        .then(|| format!("{file_stem}.compiled_contract_class.json"));
                    all_plans.push(NativeContractOutputPlan {
                        module_path: module_path.clone(),
                        artifact_id,
                        package_name,
                        contract_name,
                        artifact_file,
                        casm_file,
                    });
                }
            }
            if !all_plans.is_empty() {
                native_update_compile_session_post_build_state(
                    workspace_root,
                    &signature,
                    &[],
                    Some(&all_plans),
                );
            }

            let mut selected_indices: Vec<usize> = (0..contracts.len()).collect();
            let mut reused_contract_entries = Vec::new();
            let mut reused_keep_files = BTreeSet::new();
            if native_impacted_subset_enabled()
                && (!session.changed_files.is_empty() || !session.removed_files.is_empty())
            {
                if let Some(impacted_indices) = native_impacted_contract_indices(
                    &module_paths,
                    &contract_source_paths,
                    &session.changed_files,
                    &session.removed_files,
                    &session.contract_source_dependencies,
                ) {
                    if impacted_indices.len() < contracts.len() {
                        let impacted_set =
                            impacted_indices.iter().copied().collect::<BTreeSet<_>>();
                        if let Some((entries, keep_files)) =
                            native_reusable_unaffected_manifest_entries(
                                &target_dir,
                                &context.package_name,
                                &all_plans,
                                &impacted_set,
                            )?
                        {
                            tracing::debug!(
                                impacted_contracts = impacted_indices.len(),
                                total_contracts = contracts.len(),
                                changed_files = session.changed_files.len(),
                                removed_files = session.removed_files.len(),
                                "native compile selected impacted contract subset"
                            );
                            selected_indices = impacted_indices;
                            reused_contract_entries = entries;
                            reused_keep_files = keep_files;
                        }
                    }
                }
            }

            let (frontend_compile_ms, contract_classes) = if selected_indices.is_empty() {
                (0.0, Vec::new())
            } else {
                native_run_contract_compile_batches(
                    &all_plans,
                    &selected_indices,
                    native_progress_compile_batch_size(),
                    |batch_indices| {
                        let contract_refs: Vec<_> = batch_indices
                            .iter()
                            .map(|index| &contracts[*index])
                            .collect();
                        compile_starknet_prepared_db(
                            &session.db,
                            &contract_refs,
                            native_compiler_config(
                                &session.main_crate_inputs,
                                profile,
                                capture_statement_locations,
                            ),
                        )
                        .map_err(|err| {
                            mark_native_fallback_eligible_for_external_dependencies(
                                err.context("native starknet compile failed"),
                                &context,
                            )
                        })
                    },
                )?
            };
            #[cfg(debug_assertions)]
            {
                for (result_index, contract_index) in selected_indices.iter().copied().enumerate() {
                    let contract = &contracts[contract_index];
                    let mut single_class = compile_starknet_prepared_db(
                        &session.db,
                        &[contract],
                        native_compiler_config(
                            &session.main_crate_inputs,
                            profile,
                            capture_statement_locations,
                        ),
                    )
                    .with_context(|| {
                        format!(
                            "failed to validate native contract ordering for {}",
                            contract.submodule_id.full_path(&session.db)
                        )
                    })?;
                    let expected_class = single_class
                        .pop()
                        .context("single-contract native compile returned no output")?;
                    debug_assert_eq!(
                        expected_class, contract_classes[result_index],
                        "compile_starknet_prepared_db returned classes in unexpected order"
                    );
                }
            }
            let dependency_updates = if capture_statement_locations {
                native_collect_contract_dependency_updates(
                    workspace_root,
                    &all_plans,
                    &contract_source_paths,
                    &selected_indices,
                    &contract_classes,
                )
            } else {
                Vec::new()
            };
            let artifact_pipeline_start = Instant::now();
            let mut casm_ms = 0.0;
            let mut serialized_artifacts =
                Vec::with_capacity(contract_classes.len().saturating_mul(2));
            let mut keep_files = reused_keep_files;
            let mut contracts_manifest = reused_contract_entries;
            for (contract_index, contract_class) in selected_indices
                .iter()
                .copied()
                .zip(contract_classes.into_iter())
            {
                let plan = &all_plans[contract_index];
                let artifact_bytes = serde_json::to_vec(&contract_class)
                    .with_context(|| format!("failed to serialize {}", plan.artifact_file))?;
                serialized_artifacts.push((plan.artifact_file.clone(), artifact_bytes));
                keep_files.insert(plan.artifact_file.clone());

                let casm_file = if context.starknet_target.casm {
                    let casm_compile_start = Instant::now();
                    let casm_contract = compile_native_casm_contract(
                        contract_class,
                        native_max_casm_bytecode_size(),
                    )?;
                    casm_ms += casm_compile_start.elapsed().as_secs_f64() * 1000.0;
                    let file_name = plan.casm_file.clone().context(
                    "native contract output plan missing CASM file while CASM target is enabled",
                )?;
                    let casm_bytes = serde_json::to_vec(&casm_contract)
                        .with_context(|| format!("failed to serialize {}", file_name))?;
                    serialized_artifacts.push((file_name.clone(), casm_bytes));
                    keep_files.insert(file_name.clone());
                    Some(file_name)
                } else {
                    None
                };

                contracts_manifest.push(StarknetArtifactEntry {
                    id: plan.artifact_id.clone(),
                    package_name: plan.package_name.clone(),
                    contract_name: plan.contract_name.clone(),
                    module_path: plan.module_path.clone(),
                    artifacts: StarknetArtifactFiles {
                        sierra: plan.artifact_file.clone(),
                        casm: casm_file,
                    },
                });
            }
            contracts_manifest.sort_by(|left, right| left.id.cmp(&right.id));
            let manifest = StarknetArtifactsManifest {
                version: 1,
                contracts: contracts_manifest,
            };
            let manifest_name = format!("{}.starknet_artifacts.json", context.package_name);
            keep_files.insert(manifest_name.clone());
            for (file_name, bytes) in serialized_artifacts {
                let artifact_path = target_dir.join(&file_name);
                ensure_path_within_root(
                    &target_dir,
                    &artifact_path,
                    "native artifact output path",
                )?;
                fs::write(&artifact_path, bytes)
                    .with_context(|| format!("failed to write {}", artifact_path.display()))?;
            }
            let manifest_path = target_dir.join(&manifest_name);
            ensure_path_within_root(
                &target_dir,
                &manifest_path,
                "native starknet artifacts manifest path",
            )?;
            write_json_file_compact(&manifest_path, &manifest)
                .with_context(|| format!("failed to write {}", manifest_path.display()))?;
            prune_native_target_outputs(&target_dir, &context.package_name, &keep_files)?;
            let artifact_pipeline_ms = artifact_pipeline_start.elapsed().as_secs_f64() * 1000.0;
            let artifact_write_ms = (artifact_pipeline_ms - casm_ms).max(0.0);
            let produced_paths = keep_files.iter().cloned().collect::<Vec<_>>();
            let compiled_contracts = selected_indices.len() as u64;
            let total_contracts = contracts.len() as u64;
            let impacted_subset_used =
                native_impacted_subset_used(total_contracts as usize, compiled_contracts as usize);
            Ok((
                keep_files.len(),
                frontend_compile_ms,
                casm_ms,
                artifact_write_ms,
                produced_paths,
                session.changed_files.len() as u64,
                session.removed_files.len() as u64,
                total_contracts,
                compiled_contracts,
                impacted_subset_used,
                session.journal_fallback_full_scan,
                dependency_updates,
                Some(all_plans),
            ))
        },
    )?;
    native_update_compile_session_post_build_state(
        workspace_root,
        &signature,
        &dependency_updates,
        contract_output_plans.as_deref(),
    );
    native_persist_crate_cache_after_build_best_effort(
        workspace_root,
        &signature,
        daemon_context,
        changed_files_count,
        removed_files_count,
        compiled_contracts,
    );
    let compile_ms = compile_start.elapsed().as_secs_f64() * 1000.0;
    let session_scope_ms = session_scope_start.elapsed().as_secs_f64() * 1000.0;
    let session_prepare_ms =
        (session_scope_ms - frontend_compile_ms - casm_ms - artifact_write_ms).max(0.0);
    let native_phase_telemetry = NativeBuildPhaseTelemetry {
        context_ms,
        target_dir_ms,
        session_prepare_ms,
        frontend_compile_ms,
        casm_ms,
        artifact_write_ms,
        changed_files: changed_files_count,
        removed_files: removed_files_count,
        total_contracts,
        compiled_contracts,
        impacted_subset_used,
        journal_fallback_full_scan,
    };
    let run = CommandRun {
        command: vec![
            "uc-native".to_string(),
            "build".to_string(),
            "--manifest-path".to_string(),
            manifest_path.display().to_string(),
            "--crate".to_string(),
            context.crate_name,
        ],
        exit_code: 0,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        stdout: format!("uc: native compile produced {} artifacts\n", artifact_count),
        stderr: String::new(),
    };
    tracing::debug!(
        manifest_path = %manifest_path.display(),
        workspace_root = %workspace_root.display(),
        profile,
        context_ms,
        target_dir_ms,
        session_prepare_ms,
        frontend_compile_ms,
        casm_ms,
        artifact_write_ms,
        compile_ms,
        total_ms = run.elapsed_ms,
        artifact_count,
        changed_files = changed_files_count,
        removed_files = removed_files_count,
        total_contracts,
        compiled_contracts,
        impacted_subset_used,
        journal_fallback_full_scan,
        "native build cold-path telemetry"
    );
    Ok((run, native_phase_telemetry, produced_paths))
}

#[cfg(feature = "native-compile")]
fn write_json_file_compact<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let file =
        File::create(path).with_context(|| format!("failed to create {}", path.display()))?;
    let mut writer = io::BufWriter::new(file);
    serde_json::to_writer(&mut writer, value)
        .with_context(|| format!("failed to serialize JSON to {}", path.display()))?;
    writer
        .flush()
        .with_context(|| format!("failed to flush {}", path.display()))?;
    Ok(())
}

fn scarb_build_command(common: &BuildCommonArgs, manifest_path: &Path) -> (Command, Vec<String>) {
    let mut command = Command::new("scarb");
    let mut command_vec = vec!["scarb".to_string()];
    let disable_artifacts_fingerprint = parse_env_bool(
        "UC_DISABLE_SCARB_ARTIFACTS_FINGERPRINT",
        DEFAULT_UC_DISABLE_SCARB_ARTIFACTS_FINGERPRINT,
    );
    command.env(
        "SCARB_ARTIFACTS_FINGERPRINT",
        if disable_artifacts_fingerprint {
            "0"
        } else {
            "1"
        },
    );

    command.arg("--manifest-path").arg(manifest_path);
    command_vec.push("--manifest-path".to_string());
    command_vec.push(manifest_path.display().to_string());

    if common.offline {
        command.arg("--offline");
        command_vec.push("--offline".to_string());
    }

    if common.release {
        command.arg("--release");
        command_vec.push("--release".to_string());
    }

    if let Some(profile) = &common.profile {
        command.arg("--profile").arg(profile);
        command_vec.push("--profile".to_string());
        command_vec.push(profile.clone());
    }

    command.arg("build");
    command_vec.push("build".to_string());

    if let Some(package) = &common.package {
        command.arg("--package").arg(package);
        command_vec.push("--package".to_string());
        command_vec.push(package.clone());
    }

    if common.workspace {
        command.arg("--workspace");
        command_vec.push("--workspace".to_string());
    }

    if !common.features.is_empty() {
        let features = common.features.join(",");
        command.arg("--features").arg(&features);
        command_vec.push("--features".to_string());
        command_vec.push(features);
    }

    (command, command_vec)
}

fn scarb_metadata_command(args: &MetadataArgs, manifest_path: &Path) -> (Command, Vec<String>) {
    let mut command = Command::new("scarb");
    let mut command_vec = vec!["scarb".to_string()];

    command.arg("--manifest-path").arg(manifest_path);
    command_vec.push("--manifest-path".to_string());
    command_vec.push(manifest_path.display().to_string());

    if args.offline {
        command.arg("--offline");
        command_vec.push("--offline".to_string());
    }

    if let Some(cache_dir) = &args.global_cache_dir {
        command.arg("--global-cache-dir").arg(cache_dir);
        command_vec.push("--global-cache-dir".to_string());
        command_vec.push(cache_dir.display().to_string());
    }

    command.arg("metadata");
    command
        .arg("--format-version")
        .arg(args.format_version.to_string());

    command_vec.push("metadata".to_string());
    command_vec.push("--format-version".to_string());
    command_vec.push(args.format_version.to_string());

    (command, command_vec)
}

fn run_uc_build_subprocess(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    engine: EngineArg,
) -> Result<CommandRun> {
    let exe = std::env::current_exe().context("failed to resolve current uc binary path")?;
    let mut command = Command::new(&exe);
    let mut command_vec = vec![exe.display().to_string(), "build".to_string()];

    command.arg("build");

    command.arg("--manifest-path").arg(manifest_path);
    command_vec.push("--manifest-path".to_string());
    command_vec.push(manifest_path.display().to_string());

    command.arg("--engine").arg(engine.as_str());
    command_vec.push("--engine".to_string());
    command_vec.push(engine.as_str().to_string());

    // compare-build must evaluate direct local behavior rather than daemon state.
    command.arg("--daemon-mode").arg("off");
    command_vec.push("--daemon-mode".to_string());
    command_vec.push("off".to_string());

    if common.offline {
        command.arg("--offline");
        command_vec.push("--offline".to_string());
    }

    if common.release {
        command.arg("--release");
        command_vec.push("--release".to_string());
    }

    if let Some(profile) = &common.profile {
        command.arg("--profile").arg(profile);
        command_vec.push("--profile".to_string());
        command_vec.push(profile.clone());
    }

    if let Some(package) = &common.package {
        command.arg("--package").arg(package);
        command_vec.push("--package".to_string());
        command_vec.push(package.clone());
    }

    if common.workspace {
        command.arg("--workspace");
        command_vec.push("--workspace".to_string());
    }

    if !common.features.is_empty() {
        let features = common.features.join(",");
        command.arg("--features").arg(&features);
        command_vec.push("--features".to_string());
        command_vec.push(features);
    }

    run_command_capture(command, command_vec)
}

fn run_command_capture(mut command: Command, command_vec: Vec<String>) -> Result<CommandRun> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let start = Instant::now();
    let mut child = command.spawn().context("failed to run command")?;
    let stdout = child
        .stdout
        .take()
        .context("failed to capture command stdout")?;
    let stderr = child
        .stderr
        .take()
        .context("failed to capture command stderr")?;

    let stdout_limit = max_capture_stdout_bytes();
    let stderr_limit = max_capture_stderr_bytes();
    let stdout_thread =
        thread::spawn(move || read_stream_with_limit(stdout, stdout_limit, "stdout"));
    let stderr_thread =
        thread::spawn(move || read_stream_with_limit(stderr, stderr_limit, "stderr"));

    let status = child.wait().context("failed waiting for command")?;
    let stdout_bytes = join_stream_thread(stdout_thread, "stdout")?;
    let stderr_bytes = join_stream_thread(stderr_thread, "stderr")?;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

    Ok(CommandRun {
        command: command_vec,
        exit_code: exit_code_from_status(&status),
        elapsed_ms,
        stdout: String::from_utf8_lossy(&stdout_bytes).to_string(),
        stderr: String::from_utf8_lossy(&stderr_bytes).to_string(),
    })
}

fn read_stream_with_limit<R: Read>(mut reader: R, max_bytes: u64, label: &str) -> Result<Vec<u8>> {
    let mut limited = (&mut reader).take(max_bytes + 1);
    let mut bytes = Vec::new();
    limited
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read command {label}"))?;
    if bytes.len() as u64 > max_bytes {
        bail!("command {label} exceeded capture limit of {max_bytes} bytes");
    }
    Ok(bytes)
}

fn join_stream_thread(handle: thread::JoinHandle<Result<Vec<u8>>>, label: &str) -> Result<Vec<u8>> {
    match handle.join() {
        Ok(result) => result,
        Err(_) => bail!("command {label} reader thread panicked"),
    }
}

fn run_command_status(mut command: Command, command_vec: Vec<String>) -> Result<CommandRun> {
    let start = Instant::now();
    command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    let status = command.status().context("failed to run command")?;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok(CommandRun {
        command: command_vec,
        exit_code: exit_code_from_status(&status),
        elapsed_ms,
        stdout: String::new(),
        stderr: String::new(),
    })
}

fn run_command_status_silent(mut command: Command, command_vec: Vec<String>) -> Result<CommandRun> {
    let start = Instant::now();
    command.stdout(Stdio::null()).stderr(Stdio::null());
    let status = command.status().context("failed to run command")?;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok(CommandRun {
        command: command_vec,
        exit_code: exit_code_from_status(&status),
        elapsed_ms,
        stdout: String::new(),
        stderr: String::new(),
    })
}

fn run_command(
    command: Command,
    command_vec: Vec<String>,
    capture_output: bool,
) -> Result<CommandRun> {
    if capture_output {
        return run_command_capture(command, command_vec);
    }
    run_command_status(command, command_vec)
}

fn collect_profile_artifacts(workspace_root: &Path, profile: &str) -> Result<Vec<ArtifactDigest>> {
    let target_dir = workspace_root.join("target").join(profile);
    collect_artifact_digests(&target_dir)
}

#[cfg(test)]
fn collect_cached_artifacts_for_entry(
    workspace_root: &Path,
    profile: &str,
    cache_root: &Path,
    objects_dir: &Path,
) -> Result<Vec<CachedArtifact>> {
    collect_cached_artifacts_for_entry_with_paths(
        workspace_root,
        profile,
        cache_root,
        objects_dir,
        None,
    )
}

fn collect_cached_artifacts_for_entry_with_paths(
    workspace_root: &Path,
    profile: &str,
    cache_root: &Path,
    objects_dir: &Path,
    artifact_relative_paths: Option<&[String]>,
) -> Result<Vec<CachedArtifact>> {
    let target_root = workspace_root.join("target").join(profile);
    if !target_root.exists() {
        return Ok(Vec::new());
    }

    let index_path = cache_root.join("artifact-index-v1.json");
    let mut index = load_artifact_index_cached(&index_path)?;
    let mut updated_index_entries: BTreeMap<String, ArtifactIndexEntry> =
        if artifact_relative_paths.is_some() {
            index.entries.clone()
        } else {
            BTreeMap::new()
        };
    let mut cached_artifacts = Vec::new();
    let now_ms = epoch_ms_u64().unwrap_or_default();
    let mtime_recheck_window_ms = fingerprint_mtime_recheck_window_ms();
    let mut process_cacheable_artifact = |path: &Path, strict: bool| -> Result<()> {
        let metadata =
            fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
        if !metadata.is_file() {
            if strict {
                bail!("expected artifact path is not a file: {}", path.display());
            }
            return Ok(());
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            if strict {
                bail!("artifact path is not valid UTF-8: {}", path.display());
            }
            return Ok(());
        };
        if !CACHEABLE_ARTIFACT_SUFFIXES
            .iter()
            .any(|suffix| name.ends_with(suffix))
        {
            if strict {
                bail!("artifact path is not cacheable: {}", path.display());
            }
            return Ok(());
        }
        if metadata.len() > MAX_CACHEABLE_ARTIFACT_BYTES {
            bail!(
                "cacheable artifact {} exceeds size limit ({} bytes > {} bytes)",
                path.display(),
                metadata.len(),
                MAX_CACHEABLE_ARTIFACT_BYTES
            );
        }
        let relative_path = path
            .strip_prefix(&target_root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let modified_unix_ms = metadata_modified_unix_ms(&metadata)?;
        let canonical_hash = if let Some(cached) = index.entries.get(&relative_path) {
            let should_rehash_recent =
                now_ms.saturating_sub(modified_unix_ms) <= mtime_recheck_window_ms;
            if cached.size_bytes == metadata.len()
                && cached.modified_unix_ms == modified_unix_ms
                && !should_rehash_recent
            {
                cached.blake3_hex.clone()
            } else {
                hash_file_blake3(path)?
            }
        } else {
            hash_file_blake3(path)?
        }
        .to_ascii_lowercase();
        validate_hex_digest("artifact blake3 hash", &canonical_hash, MIN_HASH_LEN)?;
        let object_rel_path = format!("{}/{}.bin", &canonical_hash[0..2], canonical_hash);
        let object_path = objects_dir.join(&object_rel_path);
        if !cache_object_matches_expected(&object_path, &canonical_hash, metadata.len())? {
            let _ = fs::remove_file(&object_path);
            if let Some(parent) = object_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            persist_artifact_object(path, &object_path)?;
        }
        cached_artifacts.push(CachedArtifact {
            relative_path: relative_path.clone(),
            blake3_hex: canonical_hash.clone(),
            size_bytes: metadata.len(),
            object_rel_path,
        });
        updated_index_entries.insert(
            relative_path,
            ArtifactIndexEntry {
                size_bytes: metadata.len(),
                modified_unix_ms,
                blake3_hex: canonical_hash,
            },
        );
        Ok(())
    };

    if let Some(relative_paths) = artifact_relative_paths {
        let mut unique_relative_paths = BTreeSet::new();
        for relative_path in relative_paths {
            let sanitized = validated_relative_artifact_path(relative_path)?;
            let normalized = normalize_fingerprint_path(&sanitized);
            if !unique_relative_paths.insert(normalized.clone()) {
                continue;
            }
            let absolute_path = target_root.join(&sanitized);
            ensure_path_within_root(&target_root, &absolute_path, "cache collection path")?;
            process_cacheable_artifact(&absolute_path, true)?;
        }
    } else {
        for entry in WalkDir::new(&target_root).follow_links(false).into_iter() {
            let entry = entry.with_context(|| {
                format!(
                    "failed to traverse artifact tree under {}",
                    target_root.display()
                )
            })?;
            if !entry.file_type().is_file() {
                continue;
            }
            process_cacheable_artifact(entry.path(), false)?;
        }
    }

    cached_artifacts.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    index.schema_version = ARTIFACT_INDEX_SCHEMA_VERSION;
    index.entries = updated_index_entries;
    store_artifact_index_cached(&index_path, &index);
    save_artifact_index(&index_path, &index)
        .with_context(|| format!("failed to update artifact index {}", index_path.display()))?;
    Ok(cached_artifacts)
}

fn persist_artifact_object(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        return Ok(());
    }
    if let Err(reflink_err) = try_reflink_file(source, destination) {
        if reflink_err.kind() == io::ErrorKind::AlreadyExists {
            return Ok(());
        }
        let _ = fs::remove_file(destination);
        match fs::hard_link(source, destination) {
            Ok(()) => return Ok(()),
            Err(err) => {
                if err.kind() == io::ErrorKind::AlreadyExists {
                    return Ok(());
                }
                fs::copy(source, destination).with_context(|| {
                    format!(
                        "failed to copy artifact {} to {} after reflink ({}) and hard-link ({}) fallbacks",
                        source.display(),
                        destination.display(),
                        reflink_err,
                        err
                    )
                })?;
            }
        }
    }
    Ok(())
}

fn replay_output(stdout: &str, stderr: &str) -> Result<()> {
    io::stdout().write_all(stdout.as_bytes())?;
    io::stderr().write_all(stderr.as_bytes())?;
    Ok(())
}

fn remove_build_outputs(workspace_root: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(workspace_root)
        .with_context(|| format!("failed to stat workspace root {}", workspace_root.display()))?;
    if meta.file_type().is_symlink() {
        bail!(
            "workspace root {} must not be a symlink",
            workspace_root.display()
        );
    }
    let canonical_root = workspace_root.canonicalize().with_context(|| {
        format!(
            "failed to resolve workspace root {}",
            workspace_root.display()
        )
    })?;
    if canonical_root == Path::new("/") {
        bail!(
            "workspace root {} is filesystem root; refusing cleanup",
            canonical_root.display()
        );
    }
    if !canonical_root.join("Scarb.toml").is_file() {
        bail!(
            "workspace root {} has no Scarb.toml marker; refusing cleanup",
            canonical_root.display()
        );
    }
    if !canonical_root.join("Scarb.lock").exists()
        && !canonical_root.join("src").is_dir()
        && !canonical_root.join("crates").is_dir()
    {
        bail!(
            "workspace root {} is missing expected project markers; refusing cleanup",
            canonical_root.display()
        );
    }

    let target = canonical_root.join("target");
    let scarb_dir = canonical_root.join(".scarb");
    let uc_dir = canonical_root.join(".uc");

    if target.exists() {
        fs::remove_dir_all(&target)
            .with_context(|| format!("failed to remove {}", target.display()))?;
    }

    if scarb_dir.exists() {
        fs::remove_dir_all(&scarb_dir)
            .with_context(|| format!("failed to remove {}", scarb_dir.display()))?;
    }

    if uc_dir.exists() {
        fs::remove_dir_all(&uc_dir)
            .with_context(|| format!("failed to remove {}", uc_dir.display()))?;
    }

    Ok(())
}

fn resolve_manifest_path(manifest_path: &Option<PathBuf>) -> Result<PathBuf> {
    let requested = manifest_path
        .as_ref()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("Scarb.toml"));
    let resolved = if requested.is_absolute() {
        requested
            .canonicalize()
            .with_context(|| format!("failed to resolve manifest path {}", requested.display()))?
    } else {
        let cwd = std::env::current_dir()?.canonicalize()?;
        let candidate = cwd.join(&requested);
        let resolved = candidate
            .canonicalize()
            .with_context(|| format!("failed to resolve manifest path {}", candidate.display()))?;
        if !resolved.starts_with(&cwd) {
            bail!(
                "manifest path escapes current working directory: {}",
                requested.display()
            );
        }
        resolved
    };

    if resolved.file_name().and_then(|s| s.to_str()) != Some("Scarb.toml") {
        bail!(
            "manifest path must reference Scarb.toml, got {}",
            resolved.display()
        );
    }

    Ok(resolved)
}

#[cfg(test)]
fn validate_manifest_dependency_sanity(manifest_path: &Path) -> Result<()> {
    let manifest_text = read_text_file_with_limit(manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
    let manifest = parse_manifest_toml(
        &manifest_text,
        manifest_path,
        "failed to parse manifest dependency tables",
    )?;
    validate_manifest_dependency_sanity_from_manifest(manifest_path, &manifest)
}

fn validate_manifest_dependency_sanity_from_manifest(
    manifest_path: &Path,
    manifest: &TomlValue,
) -> Result<()> {
    let package_name = manifest
        .get("package")
        .and_then(TomlValue::as_table)
        .and_then(|tbl| tbl.get("name"))
        .and_then(TomlValue::as_str)
        .map(str::to_string);

    let Some(package_name) = package_name else {
        return Ok(());
    };

    for section_name in ["dependencies", "dev-dependencies"] {
        let Some(table) = manifest.get(section_name).and_then(TomlValue::as_table) else {
            continue;
        };
        if table.contains_key(&package_name) {
            bail!(
                "manifest {} contains self-dependency `{}` in [{}]",
                manifest_path.display(),
                package_name,
                section_name
            );
        }
    }
    Ok(())
}

fn effective_profile(common: &BuildCommonArgs) -> String {
    if common.release {
        return "release".to_string();
    }

    common
        .profile
        .as_ref()
        .cloned()
        .unwrap_or_else(|| "dev".to_string())
}

fn scarb_version_line() -> Result<String> {
    static SCARB_VERSION_CACHE: OnceLock<String> = OnceLock::new();
    if let Ok(override_version) = std::env::var("UC_SCARB_VERSION_LINE") {
        let trimmed = override_version.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    if let Some(cached) = SCARB_VERSION_CACHE.get() {
        return Ok(cached.clone());
    }
    let output = Command::new("scarb")
        .arg("--version")
        .output()
        .context("failed to execute `scarb --version`")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("`scarb --version` failed: {}", stderr.trim());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let first = stdout.lines().next().unwrap_or("scarb unknown").trim();
    if first.is_empty() || first == "scarb unknown" {
        bail!("failed to parse `scarb --version` output");
    }
    let version = first.to_string();
    let _ = SCARB_VERSION_CACHE.set(version.clone());
    Ok(version)
}

fn session_input_cache() -> &'static Mutex<HashMap<String, SessionInputCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, SessionInputCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn evict_oldest_session_input_cache_entries(
    cache: &mut HashMap<String, SessionInputCacheEntry>,
    max_entries: usize,
) {
    while cache.len() > max_entries {
        let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_access_epoch_ms)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        cache.remove(&oldest_key);
    }
}

fn session_input_cache_key(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    profile: &str,
    compiler_version: &str,
    build_env_fingerprint: &str,
) -> String {
    let mut hasher = Hasher::new();
    hasher.update(b"uc-session-input-cache-v1");
    hasher.update(normalize_fingerprint_path(manifest_path).as_bytes());
    hasher.update(compiler_version.as_bytes());
    hasher.update(build_env_fingerprint.as_bytes());
    hasher.update(profile.as_bytes());
    hasher.update(common.package.as_deref().unwrap_or("*").as_bytes());
    hasher.update(if common.workspace {
        b"workspace"
    } else {
        b"package"
    });
    hasher.update(if common.offline {
        b"offline"
    } else {
        b"online"
    });
    hasher.update(if common.release { b"release" } else { b"dev" });
    let mut features = common.features.clone();
    features.sort_unstable();
    features.dedup();
    for feature in features {
        hasher.update(feature.as_bytes());
        hasher.update(b",");
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
fn build_session_input(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    profile: &str,
) -> Result<SessionInput> {
    let compiler_version = scarb_version_line()?;
    build_session_input_with_compiler_version(common, manifest_path, profile, &compiler_version)
}

fn build_session_input_with_compiler_version(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    profile: &str,
    compiler_version: &str,
) -> Result<SessionInput> {
    let build_env_fingerprint = current_build_env_fingerprint();
    let manifest_metadata = fs::metadata(manifest_path)
        .with_context(|| format!("failed to stat {}", manifest_path.display()))?;
    let manifest_size_bytes = manifest_metadata.len();
    let manifest_modified_unix_ms = metadata_modified_unix_ms(&manifest_metadata)?;
    let cache_key = session_input_cache_key(
        common,
        manifest_path,
        profile,
        compiler_version,
        &build_env_fingerprint,
    );
    let cache_now_ms = epoch_ms_u64().unwrap_or_default();
    {
        let mut cache = session_input_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = cache.get_mut(&cache_key) {
            entry.last_access_epoch_ms = cache_now_ms;
            if entry.manifest_size_bytes == manifest_size_bytes
                && entry.manifest_modified_unix_ms == manifest_modified_unix_ms
            {
                return Ok(entry.input.clone());
            }
        }
    }

    let manifest_text = read_text_file_with_limit(manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
    let manifest = parse_manifest_toml(
        &manifest_text,
        manifest_path,
        "failed to parse manifest for session key",
    )?;
    validate_manifest_dependency_sanity_from_manifest(manifest_path, &manifest)?;
    let manifest_content_hash = compute_manifest_content_hash_bytes(manifest_text.as_bytes());
    let (cairo_edition, cairo_lang_version) =
        resolve_manifest_cairo_settings_from_manifest(&manifest);
    let mut cfg_set = build_session_cfg_set_from_manifest(&manifest)?;
    cfg_set.push(format!("workspace:{}", common.workspace));
    cfg_set.push(format!("release:{}", common.release));
    let input = SessionInput {
        compiler_version: compiler_version.to_string(),
        profile: profile.to_string(),
        offline: common.offline,
        package: common.package.clone(),
        features: common.features.clone(),
        cfg_set,
        manifest_content_hash,
        target_family: if common.workspace {
            "workspace".to_string()
        } else {
            "package".to_string()
        },
        cairo_edition,
        cairo_lang_version,
        build_env_fingerprint,
    };
    {
        let mut cache = session_input_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.insert(
            cache_key,
            SessionInputCacheEntry {
                manifest_size_bytes,
                manifest_modified_unix_ms,
                input: input.clone(),
                last_access_epoch_ms: cache_now_ms,
            },
        );
        evict_oldest_session_input_cache_entries(&mut cache, session_input_cache_max_entries());
    }
    Ok(input)
}

fn daemon_build_plan_cache() -> &'static Mutex<HashMap<String, DaemonBuildPlanCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, DaemonBuildPlanCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn evict_oldest_daemon_build_plan_cache_entries(
    cache: &mut HashMap<String, DaemonBuildPlanCacheEntry>,
    max_entries: usize,
) {
    while cache.len() > max_entries {
        let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_access_epoch_ms)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        cache.remove(&oldest_key);
    }
}

fn daemon_lock_hash_cache() -> &'static Mutex<HashMap<String, LockfileHashCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, LockfileHashCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn metadata_result_cache() -> &'static Mutex<HashMap<String, MetadataResultCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, MetadataResultCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(feature = "native-compile")]
fn native_compile_session_cache() -> &'static Mutex<HashMap<String, NativeCompileSessionCacheEntry>>
{
    static CACHE: OnceLock<Mutex<HashMap<String, NativeCompileSessionCacheEntry>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(feature = "native-compile")]
fn native_compile_context_cache() -> &'static Mutex<HashMap<String, NativeCompileContextCacheEntry>>
{
    static CACHE: OnceLock<Mutex<HashMap<String, NativeCompileContextCacheEntry>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(feature = "native-compile")]
fn native_compile_context_cache_key(
    manifest_path: &Path,
    workspace_root: &Path,
    corelib_override: Option<&str>,
    home_dir: Option<&str>,
) -> String {
    let mut hasher = Hasher::new();
    hasher.update(b"uc-native-context-cache-v1");
    hasher.update(normalize_fingerprint_path(manifest_path).as_bytes());
    hasher.update(normalize_fingerprint_path(workspace_root).as_bytes());
    hasher.update(corelib_override.unwrap_or("*").as_bytes());
    hasher.update(home_dir.unwrap_or("*").as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn evict_oldest_daemon_lock_hash_cache_entries(
    cache: &mut HashMap<String, LockfileHashCacheEntry>,
    max_entries: usize,
) {
    while cache.len() > max_entries {
        let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_access_epoch_ms)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        cache.remove(&oldest_key);
    }
}

fn evict_oldest_metadata_result_cache_entries(
    cache: &mut HashMap<String, MetadataResultCacheEntry>,
    max_entries: usize,
    max_bytes: u64,
) {
    loop {
        let total_bytes = cache
            .values()
            .map(|entry| entry.estimated_bytes)
            .fold(0_u64, u64::saturating_add);
        let over_entry_budget = cache.len() > max_entries;
        let over_byte_budget = max_bytes > 0 && total_bytes > max_bytes;
        if !over_entry_budget && !over_byte_budget {
            break;
        }
        let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_access_epoch_ms)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        cache.remove(&oldest_key);
    }
}

#[cfg(feature = "native-compile")]
fn evict_oldest_native_compile_session_cache_entries(
    cache: &mut HashMap<String, NativeCompileSessionCacheEntry>,
    max_entries: usize,
    max_bytes: u64,
) {
    loop {
        let total_bytes = cache
            .values()
            .map(|entry| entry.estimated_bytes)
            .fold(0_u64, u64::saturating_add);
        let over_entry_budget = cache.len() > max_entries;
        let over_byte_budget = max_bytes > 0 && total_bytes > max_bytes;
        if !over_entry_budget && !over_byte_budget {
            break;
        }
        let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_access_epoch_ms)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        cache.remove(&oldest_key);
    }
}

#[cfg(feature = "native-compile")]
fn evict_oldest_native_compile_context_cache_entries(
    cache: &mut HashMap<String, NativeCompileContextCacheEntry>,
    max_entries: usize,
    max_bytes: u64,
) {
    loop {
        let total_bytes = cache
            .values()
            .map(|entry| entry.estimated_bytes)
            .fold(0_u64, u64::saturating_add);
        let over_entry_budget = cache.len() > max_entries;
        let over_byte_budget = max_bytes > 0 && total_bytes > max_bytes;
        if !over_entry_budget && !over_byte_budget {
            break;
        }
        let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_access_epoch_ms)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        cache.remove(&oldest_key);
    }
}

fn metadata_change_unix_ms(metadata: &fs::Metadata) -> Option<u64> {
    #[cfg(unix)]
    {
        let secs = u64::try_from(metadata.ctime()).ok()?;
        let nanos = u64::try_from(metadata.ctime_nsec()).ok()?;
        Some(secs.saturating_mul(1000).saturating_add(nanos / 1_000_000))
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

fn metadata_cache_entry_path(workspace_root: &Path, cache_key: &str) -> PathBuf {
    workspace_root
        .join(".uc/cache/metadata")
        .join(format!("{cache_key}.json"))
}

fn metadata_result_cache_key(
    args: &MetadataArgs,
    manifest_path: &Path,
    scarb_version: &str,
    build_env_fingerprint: &str,
) -> String {
    let mut hasher = Hasher::new();
    hasher.update(b"uc-metadata-result-cache-v2");
    hasher.update(normalize_fingerprint_path(manifest_path).as_bytes());
    hasher.update(args.format_version.to_string().as_bytes());
    hasher.update(
        args.global_cache_dir
            .as_ref()
            .map(|path| normalize_fingerprint_path(path))
            .unwrap_or_else(|| "*".to_string())
            .as_bytes(),
    );
    hasher.update(scarb_version.as_bytes());
    hasher.update(build_env_fingerprint.as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn metadata_run_estimated_bytes(run: &CommandRun) -> u64 {
    run.stdout.len() as u64
        + run.stderr.len() as u64
        + run
            .command
            .iter()
            .map(|part| part.len() as u64)
            .sum::<u64>()
        + 256
}

fn metadata_workspace_manifests_hash(workspace_root: &Path) -> Result<String> {
    let mut manifests = Vec::new();
    let walker = WalkDir::new(workspace_root)
        .follow_links(false)
        .max_depth(MAX_FINGERPRINT_DEPTH)
        .into_iter()
        .filter_entry(|entry| !is_ignored_entry(workspace_root, entry.path()));
    for entry in walker.filter_map(|entry| entry.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.file_name() == "Scarb.toml" {
            manifests.push(entry.into_path());
        }
    }
    manifests.sort();
    let mut hasher = Hasher::new();
    hasher.update(b"uc-metadata-workspace-manifests-v1");
    for manifest in manifests {
        let rel = manifest
            .strip_prefix(workspace_root)
            .map(normalize_fingerprint_path)
            .unwrap_or_else(|_| normalize_fingerprint_path(&manifest));
        let metadata = fs::metadata(&manifest)
            .with_context(|| format!("failed to stat {}", manifest.display()))?;
        hasher.update(rel.as_bytes());
        hasher.update(b"\n");
        hasher.update(metadata.len().to_string().as_bytes());
        hasher.update(b"\n");
        hasher.update(metadata_modified_unix_ms(&metadata)?.to_string().as_bytes());
        hasher.update(b"\n");
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn metadata_cache_entry_matches(
    entry: &MetadataResultCacheEntry,
    manifest_size_bytes: u64,
    manifest_modified_unix_ms: u64,
    lock_hash: &str,
    workspace_manifests_hash: &str,
) -> bool {
    entry.manifest_size_bytes == manifest_size_bytes
        && entry.manifest_modified_unix_ms == manifest_modified_unix_ms
        && entry.lock_hash == lock_hash
        && entry.workspace_manifests_hash == workspace_manifests_hash
}

fn metadata_cache_file_matches(
    entry: &MetadataResultCacheFile,
    manifest_size_bytes: u64,
    manifest_modified_unix_ms: u64,
    lock_hash: &str,
    workspace_manifests_hash: &str,
) -> bool {
    entry.manifest_size_bytes == manifest_size_bytes
        && entry.manifest_modified_unix_ms == manifest_modified_unix_ms
        && entry.lock_hash == lock_hash
        && entry.workspace_manifests_hash == workspace_manifests_hash
}

fn daemon_build_plan_cache_key(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    profile: &str,
    compile_backend: BuildCompileBackend,
    compiler_version: &str,
    build_env_fingerprint: &str,
) -> String {
    let mut hasher = Hasher::new();
    hasher.update(b"uc-daemon-build-plan-cache-v1");
    hasher.update(normalize_fingerprint_path(manifest_path).as_bytes());
    hasher.update(match compile_backend {
        BuildCompileBackend::Scarb => b"scarb" as &[u8],
        BuildCompileBackend::Native => b"native" as &[u8],
    });
    hasher.update(compiler_version.as_bytes());
    hasher.update(build_env_fingerprint.as_bytes());
    hasher.update(profile.as_bytes());
    hasher.update(common.package.as_deref().unwrap_or("*").as_bytes());
    hasher.update(if common.workspace {
        b"workspace"
    } else {
        b"package"
    });
    hasher.update(if common.offline {
        b"offline"
    } else {
        b"online"
    });
    hasher.update(if common.release { b"release" } else { b"dev" });
    let mut features = common.features.clone();
    features.sort_unstable();
    features.dedup();
    for feature in features {
        hasher.update(feature.as_bytes());
        hasher.update(b",");
    }
    hasher.finalize().to_hex().to_string()
}

fn daemon_lock_state(manifest_path: &Path) -> Result<(Option<u64>, Option<u64>, String)> {
    let lock_path = manifest_path
        .parent()
        .context("manifest path has no parent")?
        .join("Scarb.lock");
    let metadata = match fs::metadata(&lock_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok((None, None, "lock:none".to_string()));
        }
        Err(err) => {
            return Err(err).with_context(|| format!("failed to stat {}", lock_path.display()));
        }
    };
    if !metadata.is_file() {
        bail!("Scarb.lock path is not a file: {}", lock_path.display());
    }
    let lock_size_bytes = metadata.len();
    let lock_modified_unix_ms = metadata_modified_unix_ms(&metadata)?;
    let lock_change_unix_ms = metadata_change_unix_ms(&metadata);
    let cache_key = normalize_fingerprint_path(&lock_path);
    let cache_now_ms = epoch_ms_u64().unwrap_or_default();
    {
        let mut cache = daemon_lock_hash_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = cache.get_mut(&cache_key) {
            entry.last_access_epoch_ms = cache_now_ms;
            if entry.size_bytes == lock_size_bytes
                && entry.modified_unix_ms == lock_modified_unix_ms
                && entry.change_unix_ms == lock_change_unix_ms
            {
                return Ok((
                    Some(lock_size_bytes),
                    Some(lock_modified_unix_ms),
                    entry.lock_hash.clone(),
                ));
            }
        }
    }

    let bytes = read_bytes_with_limit(&lock_path, MAX_LOCKFILE_BYTES, "Scarb.lock")?;
    let mut hasher = Hasher::new();
    hasher.update(b"uc-scarb-lock-v1");
    hasher.update(&bytes);
    let lock_hash = format!("lock-blake3:{}", hasher.finalize().to_hex());
    {
        let mut cache = daemon_lock_hash_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.insert(
            cache_key,
            LockfileHashCacheEntry {
                size_bytes: lock_size_bytes,
                modified_unix_ms: lock_modified_unix_ms,
                change_unix_ms: lock_change_unix_ms,
                lock_hash: lock_hash.clone(),
                last_access_epoch_ms: cache_now_ms,
            },
        );
        evict_oldest_daemon_lock_hash_cache_entries(
            &mut cache,
            daemon_lock_hash_cache_max_entries(),
        );
    }
    Ok((
        Some(lock_size_bytes),
        Some(lock_modified_unix_ms),
        lock_hash,
    ))
}

fn daemon_lock_metadata_state(manifest_path: &Path) -> Result<(Option<u64>, Option<u64>, String)> {
    let lock_path = manifest_path
        .parent()
        .context("manifest path has no parent")?
        .join("Scarb.lock");
    let metadata = match fs::metadata(&lock_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok((None, None, "lock:none".to_string()));
        }
        Err(err) => {
            return Err(err).with_context(|| format!("failed to stat {}", lock_path.display()));
        }
    };
    if !metadata.is_file() {
        bail!("Scarb.lock path is not a file: {}", lock_path.display());
    }
    let lock_size_bytes = metadata.len();
    let lock_modified_unix_ms = metadata_modified_unix_ms(&metadata)?;
    let lock_change_unix_ms = metadata_change_unix_ms(&metadata);

    let mut hasher = Hasher::new();
    hasher.update(b"uc-scarb-lock-meta-v1");
    hasher.update(&lock_size_bytes.to_le_bytes());
    hasher.update(&lock_modified_unix_ms.to_le_bytes());
    match lock_change_unix_ms {
        Some(change_unix_ms) => {
            hasher.update(b"change");
            hasher.update(&change_unix_ms.to_le_bytes());
        }
        None => {
            hasher.update(b"change:none");
        }
    }
    let lock_meta_key = format!("lock-meta:{}", hasher.finalize().to_hex());

    Ok((
        Some(lock_size_bytes),
        Some(lock_modified_unix_ms),
        lock_meta_key,
    ))
}

fn daemon_build_plan_invalidation_key(session_input: &SessionInput, lock_hash: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(b"uc-daemon-build-plan-invalidation-v1");
    hasher.update(session_input.compiler_version.as_bytes());
    hasher.update(session_input.build_env_fingerprint.as_bytes());
    hasher.update(session_input.manifest_content_hash.as_bytes());
    hasher.update(session_input.profile.as_bytes());
    hasher.update(session_input.target_family.as_bytes());
    hasher.update(if session_input.offline {
        b"offline"
    } else {
        b"online"
    });
    hasher.update(session_input.package.as_deref().unwrap_or("*").as_bytes());
    let mut features = session_input.features.clone();
    features.sort_unstable();
    features.dedup();
    for feature in features {
        hasher.update(feature.as_bytes());
        hasher.update(b",");
    }
    hasher.update(lock_hash.as_bytes());
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
fn prepare_daemon_build_plan(
    common: &BuildCommonArgs,
    manifest_path: &Path,
) -> Result<(DaemonBuildPlan, bool)> {
    let compiler_version = scarb_version_line()?;
    prepare_daemon_build_plan_with_compiler_version(
        common,
        manifest_path,
        BuildCompileBackend::Scarb,
        &compiler_version,
    )
}

fn prepare_daemon_build_plan_with_compiler_version(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    compile_backend: BuildCompileBackend,
    compiler_version: &str,
) -> Result<(DaemonBuildPlan, bool)> {
    let profile = effective_profile(common);
    let build_env_fingerprint = current_build_env_fingerprint();
    let cache_key = daemon_build_plan_cache_key(
        common,
        manifest_path,
        &profile,
        compile_backend,
        compiler_version,
        &build_env_fingerprint,
    );

    let manifest_metadata = fs::metadata(manifest_path)
        .with_context(|| format!("failed to stat {}", manifest_path.display()))?;
    let manifest_size_bytes = manifest_metadata.len();
    let manifest_modified_unix_ms = metadata_modified_unix_ms(&manifest_metadata)?;
    // Daemon build-plan invalidation only needs lockfile change detection (not semantic lock hashing);
    // use metadata-derived state to avoid lockfile-byte reads on every daemon request.
    let (lock_size_bytes, lock_modified_unix_ms, lock_hash) =
        daemon_lock_metadata_state(manifest_path)?;
    let cache_now_ms = epoch_ms_u64().unwrap_or_default();

    {
        let mut cache = daemon_build_plan_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = cache.get_mut(&cache_key) {
            entry.last_access_epoch_ms = cache_now_ms;
            if entry.manifest_size_bytes == manifest_size_bytes
                && entry.manifest_modified_unix_ms == manifest_modified_unix_ms
                && entry.lock_size_bytes == lock_size_bytes
                && entry.lock_modified_unix_ms == lock_modified_unix_ms
                && entry.lock_hash == lock_hash
            {
                return Ok((entry.plan.clone(), true));
            }
        }
    }

    let workspace_root = manifest_path
        .parent()
        .context("manifest path has no parent")?
        .to_path_buf();
    let session_input = build_session_input_with_compiler_version(
        common,
        manifest_path,
        &profile,
        compiler_version,
    )?;
    let session_key = session_input.deterministic_key_hex();
    let strict_invalidation_key = daemon_build_plan_invalidation_key(&session_input, &lock_hash);
    let plan = DaemonBuildPlan {
        manifest_path: manifest_path.to_path_buf(),
        workspace_root,
        profile,
        session_key,
        strict_invalidation_key,
    };
    {
        let mut cache = daemon_build_plan_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.insert(
            cache_key,
            DaemonBuildPlanCacheEntry {
                manifest_size_bytes,
                manifest_modified_unix_ms,
                lock_size_bytes,
                lock_modified_unix_ms,
                lock_hash: lock_hash.clone(),
                plan: plan.clone(),
                last_access_epoch_ms: cache_now_ms,
            },
        );
        evict_oldest_daemon_build_plan_cache_entries(
            &mut cache,
            daemon_build_plan_cache_max_entries(),
        );
    }
    Ok((plan, false))
}

fn daemon_shared_cache_base_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("UC_DAEMON_SHARED_CACHE_DIR") {
        let path = PathBuf::from(path);
        if !path.as_os_str().is_empty() {
            return path;
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".uc/daemon-cache");
    }
    std::env::temp_dir().join("uc-daemon-cache")
}

fn daemon_shared_cache_root(workspace_root: &Path) -> PathBuf {
    let mut hasher = Hasher::new();
    hasher.update(b"uc-daemon-shared-cache-root-v1");
    hasher.update(normalize_fingerprint_path(workspace_root).as_bytes());
    let digest = hasher.finalize().to_hex().to_string();
    daemon_shared_cache_base_dir()
        .join(&digest[0..2])
        .join(digest)
}

fn daemon_shared_cache_entry_path(shared_cache_root: &Path, session_key: &str) -> PathBuf {
    shared_cache_root
        .join("build")
        .join(format!("{session_key}.json"))
}

fn daemon_shared_cache_entry_exists(workspace_root: &Path, session_key: &str) -> Result<bool> {
    if !daemon_shared_cache_enabled() {
        return Ok(false);
    }
    let shared_cache_root = daemon_shared_cache_root(workspace_root);
    let shared_entry_path = daemon_shared_cache_entry_path(&shared_cache_root, session_key);
    match fs::metadata(&shared_entry_path) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => {
            Err(err).with_context(|| format!("failed to stat {}", shared_entry_path.display()))
        }
    }
}

fn try_restore_daemon_shared_cache(
    workspace_root: &Path,
    profile: &str,
    session_key: &str,
    fingerprint: &str,
) -> Result<Option<usize>> {
    if !daemon_shared_cache_enabled() {
        return Ok(None);
    }
    let shared_cache_root = daemon_shared_cache_root(workspace_root);
    if !shared_cache_root.exists() {
        return Ok(None);
    }
    let shared_objects_dir = shared_cache_root.join("objects");
    let shared_entry_path = daemon_shared_cache_entry_path(&shared_cache_root, session_key);
    let matching_entry = {
        if let Some(entry) = load_cache_entry_cached(&shared_entry_path)? {
            if entry.schema_version == BUILD_CACHE_SCHEMA_VERSION
                && entry.profile == profile
                && entry.fingerprint == fingerprint
            {
                Some(entry)
            } else {
                None
            }
        } else {
            None
        }
    };
    let Some(entry) = matching_entry else {
        return Ok(None);
    };
    if !(cached_artifacts_already_materialized(
        workspace_root,
        profile,
        &shared_cache_root,
        &entry.artifacts,
    )? || restore_cached_artifacts(
        workspace_root,
        profile,
        &shared_cache_root,
        &shared_objects_dir,
        &entry.artifacts,
    )?) {
        return Ok(None);
    }
    Ok(Some(entry.artifacts.len()))
}

fn persist_daemon_shared_cache_entry_with_artifacts(
    workspace_root: &Path,
    profile: &str,
    session_key: &str,
    fingerprint: &str,
    source_objects_dir: &Path,
    cached_artifacts: &[CachedArtifact],
) -> Result<()> {
    if !daemon_shared_cache_enabled() {
        return Ok(());
    }
    let shared_cache_root = daemon_shared_cache_root(workspace_root);
    let shared_objects_dir = shared_cache_root.join("objects");
    let shared_entry_path = daemon_shared_cache_entry_path(&shared_cache_root, session_key);

    let _cache_lock = acquire_cache_lock(&shared_cache_root)?;
    for artifact in cached_artifacts {
        let source_object_path = source_objects_dir.join(&artifact.object_rel_path);
        let target_object_path = shared_objects_dir.join(&artifact.object_rel_path);
        if !cache_object_matches_expected(
            &target_object_path,
            &artifact.blake3_hex,
            artifact.size_bytes,
        )? {
            let _ = fs::remove_file(&target_object_path);
            if let Some(parent) = target_object_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            persist_artifact_object(&source_object_path, &target_object_path).with_context(
                || {
                    format!(
                        "failed to mirror cached artifact object {} to daemon shared cache {}",
                        source_object_path.display(),
                        target_object_path.display()
                    )
                },
            )?;
        }
    }

    persist_cache_entry(profile, fingerprint, cached_artifacts, &shared_entry_path)?;
    let max_bytes = daemon_shared_cache_max_bytes();
    if max_bytes > 0 {
        enforce_cache_size_budget_with_budget(&shared_cache_root, max_bytes)?;
    }
    Ok(())
}

#[cfg(feature = "native-compile")]
fn resolve_manifest_native_starknet_target_props(
    manifest: &TomlValue,
) -> Result<NativeStarknetTargetProps> {
    let mut props = NativeStarknetTargetProps {
        sierra: true,
        // Match Scarb's default for empty `[[target.starknet-contract]]` entries.
        casm: false,
    };
    let Some(target_table) = manifest.get("target").and_then(TomlValue::as_table) else {
        return Ok(props);
    };
    let Some(starknet_target) = target_table.get("starknet-contract") else {
        return Ok(props);
    };
    let target_props = match starknet_target {
        TomlValue::Table(table) => table,
        TomlValue::Array(entries) => {
            if entries.len() != 1 {
                bail!("native compile supports a single [[target.starknet-contract]] entry");
            }
            entries[0].as_table().context(
                "native compile expects [[target.starknet-contract]] entries to be tables",
            )?
        }
        _ => {
            bail!(
                "native compile expects [target.starknet-contract] or [[target.starknet-contract]]"
            );
        }
    };
    if let Some(value) = target_props.get("sierra") {
        props.sierra = value
            .as_bool()
            .context("native compile expects [target.starknet-contract].sierra to be a boolean")?;
    }
    if let Some(value) = target_props.get("casm") {
        props.casm = value
            .as_bool()
            .context("native compile expects [target.starknet-contract].casm to be a boolean")?;
    }
    Ok(props)
}

fn resolve_manifest_cairo_settings_from_manifest(
    manifest: &TomlValue,
) -> (Option<String>, Option<String>) {
    let edition_from_manifest = manifest
        .get("package")
        .and_then(TomlValue::as_table)
        .and_then(|tbl| tbl.get("edition"))
        .and_then(TomlValue::as_str)
        .map(str::to_string);

    let cairo_lang_from_manifest = manifest
        .get("cairo")
        .and_then(TomlValue::as_table)
        .and_then(|tbl| tbl.get("language-version").or_else(|| tbl.get("version")))
        .and_then(TomlValue::as_str)
        .map(str::to_string);

    let edition = std::env::var("CAIRO_EDITION")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or(edition_from_manifest);

    let cairo_lang_version = std::env::var("CAIRO_LANG_VERSION")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or(cairo_lang_from_manifest);

    (edition, cairo_lang_version)
}

fn build_env_prefixes() -> Vec<String> {
    if let Ok(prefixes_override) = std::env::var("UC_BUILD_ENV_PREFIXES") {
        return prefixes_override
            .split(',')
            .map(str::trim)
            .filter(|prefix| !prefix.is_empty())
            .map(ToString::to_string)
            .collect();
    }
    const BUILD_ENV_PREFIXES: [&str; 3] = ["CAIRO_", "SCARB_", "STARKNET_"];
    let mut prefixes: Vec<String> = BUILD_ENV_PREFIXES
        .iter()
        .map(|prefix| (*prefix).to_string())
        .collect();
    if let Ok(extra_prefixes) = std::env::var("UC_BUILD_ENV_PREFIXES_EXTRA") {
        for prefix in extra_prefixes
            .split(',')
            .map(str::trim)
            .filter(|prefix| !prefix.is_empty())
        {
            if !prefixes.iter().any(|existing| existing == prefix) {
                prefixes.push(prefix.to_string());
            }
        }
    }
    prefixes
}

fn compute_build_env_fingerprint() -> String {
    let prefixes = build_env_prefixes();
    let mut entries: Vec<(String, String)> = std::env::vars()
        .filter(|(key, _)| prefixes.iter().any(|prefix| key.starts_with(prefix)))
        .collect();
    entries.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

    let mut hasher = Hasher::new();
    hasher.update(b"uc-build-env-v1");
    for (key, value) in entries {
        hasher.update(key.as_bytes());
        hasher.update(b"=");
        hasher.update(value.as_bytes());
        hasher.update(b"\n");
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
fn current_build_env_fingerprint() -> String {
    compute_build_env_fingerprint()
}

#[cfg(not(test))]
fn current_build_env_fingerprint() -> String {
    static VALUE: OnceLock<String> = OnceLock::new();
    VALUE.get_or_init(compute_build_env_fingerprint).clone()
}

#[cfg(test)]
fn build_session_cfg_set(manifest_path: &Path) -> Result<Vec<String>> {
    let manifest_text = read_text_file_with_limit(manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
    let manifest = parse_manifest_toml(
        &manifest_text,
        manifest_path,
        "failed to parse manifest for session key",
    )?;
    build_session_cfg_set_from_manifest(&manifest)
}

fn build_session_cfg_set_from_manifest(manifest: &TomlValue) -> Result<Vec<String>> {
    let mut cfg_set = Vec::new();
    if let Some(cairo) = manifest.get("cairo") {
        cfg_set.push(format!(
            "manifest:cairo={}",
            stable_toml_fragment_hash(cairo)?
        ));
    }
    if let Some(target) = manifest.get("target") {
        cfg_set.push(format!(
            "manifest:target={}",
            stable_toml_fragment_hash(target)?
        ));
    }
    if let Some(tool) = manifest.get("tool") {
        cfg_set.push(format!(
            "manifest:tool={}",
            stable_toml_fragment_hash(tool)?
        ));
    }
    Ok(cfg_set)
}

fn parse_manifest_toml(
    manifest_text: &str,
    manifest_path: &Path,
    context: &str,
) -> Result<TomlValue> {
    manifest_text
        .parse::<TomlValue>()
        .with_context(|| format!("{context} {}", manifest_path.display()))
}

fn stable_toml_fragment_hash(value: &TomlValue) -> Result<String> {
    let json_value =
        serde_json::to_value(value).context("failed to serialize TOML fragment for session key")?;
    let canonical_json = canonicalize_json_value(&json_value);
    let canonical_bytes = serde_json::to_vec(&canonical_json)
        .context("failed to encode canonical TOML fragment for session key")?;
    let mut hasher = Hasher::new();
    hasher.update(&canonical_bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

fn canonicalize_json_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let mut canonical = serde_json::Map::new();
            for (key, item) in entries {
                canonical.insert(key.clone(), canonicalize_json_value(item));
            }
            serde_json::Value::Object(canonical)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonicalize_json_value).collect())
        }
        _ => value.clone(),
    }
}

fn exit_code_from_status(status: &ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    -1
}

fn compute_manifest_content_hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Hasher::new();
    hasher.update(bytes);
    format!("manifest-blake3:{}", hasher.finalize().to_hex())
}

fn validate_hex_digest(label: &str, digest: &str, min_len: usize) -> Result<()> {
    if digest.len() < min_len {
        bail!(
            "{label} must be at least {min_len} hex chars, got {}",
            digest.len()
        );
    }
    if !digest.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("{label} must contain only hex characters");
    }
    Ok(())
}

fn validate_cache_object_rel_path(path: &str) -> Result<()> {
    let rel = Path::new(path);
    if rel.is_absolute() {
        bail!("cache object path must be relative");
    }
    for component in rel.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => bail!("cache object path contains invalid component"),
        }
    }
    Ok(())
}

fn validated_relative_artifact_path(path: &str) -> Result<PathBuf> {
    let rel = Path::new(path);
    if rel.is_absolute() {
        bail!("cached artifact path must be relative");
    }
    let mut sanitized = PathBuf::new();
    for component in rel.components() {
        match component {
            Component::Normal(value) => sanitized.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("cached artifact path contains invalid component")
            }
        }
    }
    if sanitized.as_os_str().is_empty() {
        bail!("cached artifact path must not be empty");
    }
    Ok(sanitized)
}

fn ensure_path_within_root(root: &Path, path: &Path, label: &str) -> Result<()> {
    if !path.starts_with(root) {
        bail!(
            "{label} escapes workspace root: {} not under {}",
            path.display(),
            root.display()
        );
    }
    Ok(())
}

fn acquire_cache_lock(cache_root: &Path) -> Result<CacheLockGuard> {
    fs::create_dir_all(cache_root)
        .with_context(|| format!("failed to create cache root {}", cache_root.display()))?;
    let lock_path = cache_root.join(".lock");
    let deadline = Instant::now() + Duration::from_secs(10);

    loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut file) => {
                writeln!(file, "pid={}", std::process::id()).with_context(|| {
                    format!("failed to write lock file {}", lock_path.display())
                })?;
                return Ok(CacheLockGuard { path: lock_path });
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                if maybe_cleanup_stale_lock(&lock_path)? {
                    continue;
                }
                if Instant::now() >= deadline {
                    bail!("timed out waiting for cache lock {}", lock_path.display());
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("failed to acquire cache lock {}", lock_path.display())
                });
            }
        }
    }
}

fn maybe_cleanup_stale_lock(lock_path: &Path) -> Result<bool> {
    let metadata = match fs::metadata(lock_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed to stat cache lock {}", lock_path.display()));
        }
    };

    if let Ok(contents) = fs::read_to_string(lock_path) {
        if let Some(pid) = lock_file_pid(&contents) {
            if is_process_alive(pid) {
                return Ok(false);
            }
            fs::remove_file(lock_path)
                .with_context(|| format!("failed to remove stale lock {}", lock_path.display()))?;
            return Ok(true);
        }
        if lock_file_has_pid_marker(&contents) {
            return Ok(false);
        }
    }

    let modified = match metadata.modified() {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let age = match SystemTime::now().duration_since(modified) {
        Ok(duration) => duration,
        Err(_) => return Ok(false),
    };
    if should_cleanup_stale_lock_by_age(false, age) {
        fs::remove_file(lock_path)
            .with_context(|| format!("failed to remove stale lock {}", lock_path.display()))?;
        return Ok(true);
    }
    Ok(false)
}

fn should_cleanup_stale_lock_by_age(has_live_pid: bool, age: Duration) -> bool {
    if has_live_pid {
        return false;
    }
    age > Duration::from_secs(CACHE_LOCK_STALE_AFTER_SECONDS)
}

fn lock_file_pid(contents: &str) -> Option<u32> {
    contents.lines().find_map(|line| {
        let value = line.strip_prefix("pid=")?;
        value.trim().parse::<u32>().ok()
    })
}

fn lock_file_has_pid_marker(contents: &str) -> bool {
    contents
        .lines()
        .any(|line| line.trim_start().starts_with("pid="))
}

#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return true;
    }
    matches!(
        io::Error::last_os_error().raw_os_error(),
        Some(code) if code == libc::EPERM
    )
}

#[cfg(not(unix))]
fn is_process_alive(pid: u32) -> bool {
    use sysinfo::{Pid, ProcessesToUpdate, System};

    let mut system = System::new();
    let target = Pid::from_u32(pid);
    let _ = system.refresh_processes(ProcessesToUpdate::Some(&[target]));
    system.process(target).is_some()
}

fn read_bytes_with_limit(path: &Path, max_bytes: u64, label: &str) -> Result<Vec<u8>> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    if metadata.len() > max_bytes {
        bail!(
            "{label} {} exceeds size limit ({} bytes > {} bytes)",
            path.display(),
            metadata.len(),
            max_bytes
        );
    }
    fs::read(path).with_context(|| format!("failed to read {}", path.display()))
}

fn read_text_file_with_limit(path: &Path, max_bytes: u64, label: &str) -> Result<String> {
    let bytes = read_bytes_with_limit(path, max_bytes, label)?;
    String::from_utf8(bytes)
        .with_context(|| format!("{} is not valid UTF-8: {}", label, path.display()))
}

fn write_uc_toml(
    path: &Path,
    source_manifest: &Path,
    package_name: Option<&str>,
    package_version: Option<&str>,
    edition: Option<&str>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    let name = package_name.unwrap_or("unknown-package");
    let version = package_version.unwrap_or("0.1.0");
    let edition = edition.unwrap_or(DEFAULT_CAIRO_EDITION);

    let mut project = toml::map::Map::new();
    project.insert("name".to_string(), TomlValue::String(name.to_string()));
    project.insert(
        "version".to_string(),
        TomlValue::String(version.to_string()),
    );
    project.insert(
        "edition".to_string(),
        TomlValue::String(edition.to_string()),
    );

    let mut source = toml::map::Map::new();
    source.insert(
        "scarb_manifest".to_string(),
        TomlValue::String(source_manifest.to_string_lossy().replace('\\', "/")),
    );

    let mut root = toml::map::Map::new();
    root.insert("project".to_string(), TomlValue::Table(project));
    root.insert("source".to_string(), TomlValue::Table(source));

    let mut body = toml::to_string_pretty(&TomlValue::Table(root))
        .context("failed to encode Uc.toml contents")?;
    if !body.ends_with('\n') {
        body.push('\n');
    }

    fs::write(path, body).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn write_json_report<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create report directory {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn default_compare_output_path() -> Result<PathBuf> {
    let root = workspace_root()?;
    let stamp = epoch_ms()?;
    Ok(root
        .join("benchmarks/results")
        .join(format!("compare-build-{stamp}.json")))
}

fn epoch_ms() -> Result<u128> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())
}

fn epoch_ms_u64() -> Result<u64> {
    let value = epoch_ms()?;
    u64::try_from(value).context("epoch milliseconds overflowed u64")
}

fn workspace_root() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("UC_WORKSPACE_ROOT") {
        return PathBuf::from(root)
            .canonicalize()
            .context("failed to canonicalize UC_WORKSPACE_ROOT");
    }
    let cwd = std::env::current_dir()?.canonicalize()?;
    for candidate in cwd.ancestors() {
        let root = candidate.to_path_buf();
        if root.join("Scarb.toml").is_file() {
            return Ok(root);
        }
    }
    Ok(cwd)
}

fn parse_semver_triplet(value: &str) -> Result<(u64, u64, u64)> {
    let mut parts = value.trim().split('.');
    let major = parts
        .next()
        .context("missing major version")?
        .parse::<u64>()
        .context("invalid major version")?;
    let minor = parts
        .next()
        .context("missing minor version")?
        .parse::<u64>()
        .context("invalid minor version")?;
    let patch_raw = parts.next().context("missing patch version")?;
    let patch_text = patch_raw
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>();
    let patch = patch_text.parse::<u64>().context("invalid patch version")?;
    Ok((major, minor, patch))
}

fn parse_scarb_semver(version_line: &str) -> Result<(u64, u64, u64)> {
    let mut parts = version_line.split_whitespace();
    let tool = parts.next().unwrap_or_default();
    if !tool.eq_ignore_ascii_case("scarb") {
        bail!("unexpected `scarb --version` output: {version_line}");
    }
    let semver = parts
        .next()
        .context("missing scarb semantic version token")?;
    parse_semver_triplet(semver)
}

fn min_scarb_version() -> String {
    static VALUE: OnceLock<String> = OnceLock::new();
    VALUE
        .get_or_init(|| {
            std::env::var("UC_MIN_SCARB_VERSION")
                .ok()
                .map(|raw| raw.trim().to_string())
                .filter(|raw| !raw.is_empty())
                .unwrap_or_else(|| DEFAULT_MIN_SCARB_VERSION.to_string())
        })
        .clone()
}

fn scarb_toolchain_cache_ttl_ms() -> u64 {
    parse_env_u64(
        "UC_SCARB_TOOLCHAIN_CACHE_TTL_MS",
        DEFAULT_SCARB_TOOLCHAIN_CACHE_TTL_MS,
    )
}

fn scarb_toolchain_cache_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("UC_SCARB_TOOLCHAIN_CACHE_PATH") {
        let path = PathBuf::from(path);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".uc/toolchain-check-v1.json"))
}

fn load_cached_scarb_toolchain_version_line() -> Option<String> {
    let path = scarb_toolchain_cache_path()?;
    if !path.exists() {
        return None;
    }
    let bytes = read_bytes_with_limit(
        &path,
        MAX_TOOLCHAIN_CHECK_CACHE_BYTES,
        "scarb toolchain cache",
    )
    .ok()?;
    let cache: ToolchainCheckCacheEntry = serde_json::from_slice(&bytes).ok()?;
    if cache.schema_version != TOOLCHAIN_CHECK_CACHE_SCHEMA_VERSION {
        return None;
    }
    let now_ms = epoch_ms_u64().ok()?;
    if now_ms.saturating_sub(cache.checked_epoch_ms) > scarb_toolchain_cache_ttl_ms() {
        return None;
    }
    let version_line = cache.version_line.trim();
    if version_line.is_empty() {
        return None;
    }
    Some(version_line.to_string())
}

fn store_cached_scarb_toolchain_version_line(version_line: &str) {
    let Some(path) = scarb_toolchain_cache_path() else {
        return;
    };
    let checked_epoch_ms = match epoch_ms_u64() {
        Ok(value) => value,
        Err(_) => return,
    };
    let cache = ToolchainCheckCacheEntry {
        schema_version: TOOLCHAIN_CHECK_CACHE_SCHEMA_VERSION,
        checked_epoch_ms,
        version_line: version_line.to_string(),
    };
    let bytes = match serde_json::to_vec(&cache) {
        Ok(bytes) => bytes,
        Err(_) => return,
    };
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let _ = atomic_write_bytes(&path, &bytes, "scarb toolchain cache");
}

fn validate_scarb_version_constraints(version: &str) -> Result<()> {
    let current = parse_scarb_semver(version)
        .with_context(|| format!("failed to parse scarb semantic version from `{version}`"))?;
    let minimum_text = min_scarb_version();
    let minimum = parse_semver_triplet(&minimum_text).with_context(|| {
        format!("invalid UC_MIN_SCARB_VERSION `{minimum_text}` (expected `major.minor.patch`)")
    })?;
    if current < minimum {
        bail!(
            "scarb version {} is below minimum required {}",
            version,
            minimum_text
        );
    }
    if let Ok(expected) = std::env::var("UC_EXPECT_SCARB_VERSION") {
        if !version.contains(&expected) {
            bail!(
                "scarb version mismatch: expected token `{expected}` in `{version}` (set by UC_EXPECT_SCARB_VERSION)"
            );
        }
    }
    Ok(())
}

fn validate_scarb_toolchain() -> Result<()> {
    static VALIDATED_THIS_PROCESS: OnceLock<()> = OnceLock::new();
    if VALIDATED_THIS_PROCESS.get().is_some() {
        return Ok(());
    }
    if parse_env_bool("UC_SKIP_SCARB_TOOLCHAIN_CHECK", false) {
        let _ = VALIDATED_THIS_PROCESS.set(());
        return Ok(());
    }
    if let Some(cached) = load_cached_scarb_toolchain_version_line() {
        validate_scarb_version_constraints(&cached)?;
        let _ = VALIDATED_THIS_PROCESS.set(());
        return Ok(());
    }

    let version = scarb_version_line()?;
    validate_scarb_version_constraints(&version)?;
    store_cached_scarb_toolchain_version_line(&version);
    let _ = VALIDATED_THIS_PROCESS.set(());
    Ok(())
}

#[cfg(test)]
mod main_tests;
