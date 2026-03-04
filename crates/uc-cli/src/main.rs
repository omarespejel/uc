use anyhow::{bail, Context, Result};
use blake3::Hasher;
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
const DAEMON_REQUEST_SIZE_LIMIT_BYTES: usize = 1024 * 1024;
const DAEMON_RATE_WINDOW_SECONDS: u64 = 1;
const DAEMON_MAX_REQUESTS_PER_WINDOW: usize = 32;
const DAEMON_LOG_ROTATE_BYTES: u64 = 10 * 1024 * 1024;
const DAEMON_UNHEALTHY_RECOVERY_SECONDS: u64 = 5;
const DEFAULT_DAEMON_CLIENT_READ_TIMEOUT_SECS: u64 = 120;
const DEFAULT_DAEMON_BUILD_READ_TIMEOUT_SECS: u64 = 0;
const DEFAULT_DAEMON_CLIENT_WRITE_TIMEOUT_SECS: u64 = 30;
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
const DEFAULT_DAEMON_SHARED_CACHE_ENABLED: bool = true;
const DEFAULT_DAEMON_SHARED_CACHE_MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const DEFAULT_UC_DISABLE_SCARB_ARTIFACTS_FINGERPRINT: bool = false;
const TOOLCHAIN_CHECK_CACHE_SCHEMA_VERSION: u32 = 1;
const MAX_TOOLCHAIN_CHECK_CACHE_BYTES: u64 = 64 * 1024;
const DEFAULT_SCARB_TOOLCHAIN_CACHE_TTL_MS: u64 = 5 * 60 * 1000;
const DAEMON_PROTOCOL_VERSION: &str = env!("CARGO_PKG_VERSION");
const CACHEABLE_ARTIFACT_SUFFIXES: [&str; 5] = [
    ".sierra.json",
    ".sierra",
    ".casm",
    ".contract_class.json",
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

    #[arg(long, value_enum, default_value_t = DaemonModeArg::Off)]
    daemon_mode: DaemonModeArg,

    #[arg(long)]
    report_path: Option<PathBuf>,
}

#[derive(Args, Debug)]
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
}

#[derive(Copy, Clone)]
struct BuildRunOptions {
    capture_output: bool,
    inherit_output_when_uncaptured: bool,
    async_cache_persist: bool,
    use_daemon_shared_cache: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolchainCheckCacheEntry {
    schema_version: u32,
    checked_epoch_ms: u64,
    version_line: String,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonBuildResponse {
    run: CommandRun,
    cache_hit: bool,
    fingerprint: String,
    session_key: String,
    #[serde(default)]
    telemetry: BuildPhaseTelemetry,
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
    Build(DaemonBuildRequest),
    Metadata(DaemonMetadataRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DaemonResponse {
    Pong(DaemonStatusPayload),
    Ack,
    Build(DaemonBuildResponse),
    Metadata(DaemonMetadataResponse),
    Error { message: String },
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

fn daemon_request_read_timeout(request: &DaemonRequest) -> Option<Duration> {
    match request {
        DaemonRequest::Build(_) => daemon_build_read_timeout(),
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
            "uc daemon running (pid={}, started_at_epoch_ms={}, socket={}, protocol={}, healthy={}, total_requests={}, failed_requests={}, rate_limited_requests={}, last_error={})",
            status.pid,
            status.started_at_epoch_ms,
            status.socket_path,
            status.protocol_version,
            status.healthy,
            status.total_requests,
            status.failed_requests,
            status.rate_limited_requests,
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
                "healthy (pid={}, total_requests={}, failed_requests={}, rate_limited_requests={})",
                status.pid,
                status.total_requests,
                status.failed_requests,
                status.rate_limited_requests
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
        });
        let health = Arc::new(Mutex::new(DaemonHealth::default()));
        let rate_limiter = Arc::new(Mutex::new(DaemonRateLimiter::new()));
        let should_shutdown = Arc::new(AtomicBool::new(false));

        loop {
            if should_shutdown.load(Ordering::Relaxed) {
                break;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    let status = Arc::clone(&status);
                    let health = Arc::clone(&health);
                    let should_shutdown = Arc::clone(&should_shutdown);
                    let rate_limiter = Arc::clone(&rate_limiter);
                    thread::spawn(move || {
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
                    if should_shutdown.load(Ordering::Relaxed) {
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
) -> Result<Option<DaemonBuildResponse>> {
    #[cfg(not(unix))]
    {
        let _ = (common, manifest_path, fallback_to_local);
        return Ok(None);
    }
    #[cfg(unix)]
    {
        let socket_path = daemon_socket_path(None)?;
        if !socket_path.exists() {
            return Ok(None);
        }

        let request = DaemonRequest::Build(daemon_build_request_from_common(
            common,
            manifest_path,
            daemon_async_cache_persist_enabled(),
            daemon_capture_output_enabled(),
        ));
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
            DaemonResponse::Build(result) => Ok(Some(result)),
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

        let request = DaemonRequest::Metadata(daemon_metadata_request_from_args(
            args,
            manifest_path,
            capture_output,
        ));
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
            DaemonResponse::Metadata(result) => Ok(Some(result.run)),
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

fn execute_daemon_build(request: DaemonBuildRequest) -> Result<DaemonBuildResponse> {
    validate_daemon_protocol_version(&request.protocol_version)
        .context("daemon build request protocol mismatch")?;
    // Daemon mode validates the Scarb toolchain in the daemon process so clients avoid repeated
    // `scarb --version` subprocess overhead per request.
    validate_scarb_toolchain()?;
    let common = common_args_from_daemon_request(&request);
    let manifest_path = resolve_manifest_path(&common.manifest_path)?;
    let (plan, plan_cache_hit) = prepare_daemon_build_plan(&common, &manifest_path)?;
    if plan_cache_hit {
        tracing::debug!(
            manifest_path = %plan.manifest_path.display(),
            invalidation_key = %plan.strict_invalidation_key,
            "uc daemon build plan cache hit"
        );
    } else {
        tracing::debug!(
            manifest_path = %plan.manifest_path.display(),
            invalidation_key = %plan.strict_invalidation_key,
            "uc daemon build plan cache miss"
        );
    }

    let (run, cache_hit, fingerprint, telemetry) = run_build_with_uc_cache(
        &common,
        &plan.manifest_path,
        &plan.workspace_root,
        &plan.profile,
        &plan.session_key,
        BuildRunOptions {
            capture_output: request.capture_output,
            inherit_output_when_uncaptured: request.capture_output,
            async_cache_persist: request.async_cache_persist,
            use_daemon_shared_cache: true,
        },
    )?;

    Ok(DaemonBuildResponse {
        run,
        cache_hit,
        fingerprint,
        session_key: plan.session_key,
        telemetry,
    })
}

fn execute_daemon_metadata(request: DaemonMetadataRequest) -> Result<DaemonMetadataResponse> {
    validate_daemon_protocol_version(&request.protocol_version)
        .context("daemon metadata request protocol mismatch")?;
    validate_metadata_format_version(request.format_version)?;
    let args = metadata_args_from_daemon_request(&request);
    let manifest_path = resolve_manifest_path(&args.manifest_path)?;
    let (command, command_vec) = scarb_metadata_command(&args, &manifest_path);
    let run = run_command(command, command_vec, request.capture_output)?;
    Ok(DaemonMetadataResponse { run })
}

fn try_local_uc_cache_hit(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    workspace_root: &Path,
    profile: &str,
    session_key: &str,
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

    let fingerprint_start = Instant::now();
    let fingerprint = compute_build_fingerprint(
        &canonical_workspace_root,
        manifest_path,
        common,
        profile,
        Some(&cache_root),
    )?;
    telemetry.fingerprint_ms = fingerprint_start.elapsed().as_secs_f64() * 1000.0;

    let cache_lookup_start = Instant::now();
    let cached_entry = if let Some(entry) = load_cache_entry_cached(&entry_path)? {
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
    };
    telemetry.cache_lookup_ms = cache_lookup_start.elapsed().as_secs_f64() * 1000.0;

    let Some(entry) = cached_entry else {
        return Ok(None);
    };

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
    manifest_path: &Path,
    workspace_root: &Path,
    profile: &str,
    session_key: &str,
    options: BuildRunOptions,
) -> Result<(CommandRun, bool, String, BuildPhaseTelemetry)> {
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
    let fingerprint_start = Instant::now();
    let fingerprint = compute_build_fingerprint(
        &canonical_workspace_root,
        manifest_path,
        common,
        profile,
        Some(&cache_root),
    )?;
    telemetry.fingerprint_ms = fingerprint_start.elapsed().as_secs_f64() * 1000.0;

    let cache_lookup_start = Instant::now();
    let cached_entry = if let Some(entry) = load_cache_entry_cached(&entry_path)? {
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
    };
    telemetry.cache_lookup_ms = cache_lookup_start.elapsed().as_secs_f64() * 1000.0;

    if let Some(entry) = cached_entry {
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
            let total_elapsed_ms =
                telemetry.fingerprint_ms + telemetry.cache_lookup_ms + telemetry.cache_restore_ms;
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
            return Ok((run, true, fingerprint, telemetry));
        }
        telemetry.cache_restore_ms = restore_start.elapsed().as_secs_f64() * 1000.0;
    }

    if options.use_daemon_shared_cache {
        let shared_lookup_start = Instant::now();
        let shared_restore = try_restore_daemon_shared_cache(
            &canonical_workspace_root,
            profile,
            session_key,
            &fingerprint,
        )?;
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
            return Ok((run, true, fingerprint, telemetry));
        }
    }

    let (command, command_vec) = scarb_build_command(common, manifest_path);
    let run = if options.capture_output {
        run_command_capture(command, command_vec)?
    } else if options.inherit_output_when_uncaptured {
        run_command_status(command, command_vec)?
    } else {
        run_command_status_silent(command, command_vec)?
    };
    telemetry.compile_ms = run.elapsed_ms;

    if run.exit_code == 0 {
        if options.async_cache_persist {
            telemetry.cache_persist_async = true;
            let persist_scope_key = async_persist_scope_key(&canonical_workspace_root, profile);
            if try_mark_async_persist_in_flight(&persist_scope_key) {
                telemetry.cache_persist_scheduled = true;
                let task = AsyncPersistTask {
                    scope_key: persist_scope_key.clone(),
                    workspace_root: canonical_workspace_root.clone(),
                    profile: profile.to_string(),
                    fingerprint: fingerprint.clone(),
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
            if options.use_daemon_shared_cache && !local_cache_preexisted {
                let shared_persist_start = Instant::now();
                if let Err(err) = persist_daemon_shared_cache_entry_for_build(
                    &canonical_workspace_root,
                    profile,
                    session_key,
                    &fingerprint,
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
            }
        } else {
            let persist_start = Instant::now();
            let cached_artifacts = persist_cache_entry_for_build_with_artifacts(
                &canonical_workspace_root,
                profile,
                &fingerprint,
                &cache_root,
                &objects_dir,
                &entry_path,
            )?;
            if options.use_daemon_shared_cache && !local_cache_preexisted {
                persist_daemon_shared_cache_entry_with_artifacts(
                    &canonical_workspace_root,
                    profile,
                    session_key,
                    &fingerprint,
                    &objects_dir,
                    &cached_artifacts,
                )?;
            }
            telemetry.cache_persist_ms = persist_start.elapsed().as_secs_f64() * 1000.0;
        }
    }

    Ok((run, false, fingerprint, telemetry))
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

fn collect_cached_artifacts_for_entry(
    workspace_root: &Path,
    profile: &str,
    cache_root: &Path,
    objects_dir: &Path,
) -> Result<Vec<CachedArtifact>> {
    let target_root = workspace_root.join("target").join(profile);
    if !target_root.exists() {
        return Ok(Vec::new());
    }

    let index_path = cache_root.join("artifact-index-v1.json");
    let mut index = load_artifact_index_cached(&index_path)?;
    let mut updated_index_entries: BTreeMap<String, ArtifactIndexEntry> = BTreeMap::new();
    let mut cached_artifacts = Vec::new();
    let now_ms = epoch_ms_u64().unwrap_or_default();
    let mtime_recheck_window_ms = fingerprint_mtime_recheck_window_ms();

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
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !CACHEABLE_ARTIFACT_SUFFIXES
            .iter()
            .any(|suffix| name.ends_with(suffix))
        {
            continue;
        }

        let metadata =
            fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
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
    scarb_version: &str,
    build_env_fingerprint: &str,
) -> String {
    let mut hasher = Hasher::new();
    hasher.update(b"uc-session-input-cache-v1");
    hasher.update(normalize_fingerprint_path(manifest_path).as_bytes());
    hasher.update(scarb_version.as_bytes());
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

fn build_session_input(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    profile: &str,
) -> Result<SessionInput> {
    let scarb_version = scarb_version_line()?;
    let build_env_fingerprint = current_build_env_fingerprint();
    let manifest_metadata = fs::metadata(manifest_path)
        .with_context(|| format!("failed to stat {}", manifest_path.display()))?;
    let manifest_size_bytes = manifest_metadata.len();
    let manifest_modified_unix_ms = metadata_modified_unix_ms(&manifest_metadata)?;
    let cache_key = session_input_cache_key(
        common,
        manifest_path,
        profile,
        &scarb_version,
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
        compiler_version: scarb_version,
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

fn daemon_build_plan_cache_key(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    profile: &str,
    scarb_version: &str,
    build_env_fingerprint: &str,
) -> String {
    let mut hasher = Hasher::new();
    hasher.update(b"uc-daemon-build-plan-cache-v1");
    hasher.update(normalize_fingerprint_path(manifest_path).as_bytes());
    hasher.update(scarb_version.as_bytes());
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

fn prepare_daemon_build_plan(
    common: &BuildCommonArgs,
    manifest_path: &Path,
) -> Result<(DaemonBuildPlan, bool)> {
    let profile = effective_profile(common);
    let scarb_version = scarb_version_line()?;
    let build_env_fingerprint = current_build_env_fingerprint();
    let cache_key = daemon_build_plan_cache_key(
        common,
        manifest_path,
        &profile,
        &scarb_version,
        &build_env_fingerprint,
    );

    let manifest_metadata = fs::metadata(manifest_path)
        .with_context(|| format!("failed to stat {}", manifest_path.display()))?;
    let manifest_size_bytes = manifest_metadata.len();
    let manifest_modified_unix_ms = metadata_modified_unix_ms(&manifest_metadata)?;
    let (lock_size_bytes, lock_modified_unix_ms, lock_hash) = daemon_lock_state(manifest_path)?;
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
    let session_input = build_session_input(common, manifest_path, &profile)?;
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

fn persist_daemon_shared_cache_entry_for_build(
    workspace_root: &Path,
    profile: &str,
    session_key: &str,
    fingerprint: &str,
) -> Result<()> {
    if !daemon_shared_cache_enabled() {
        return Ok(());
    }
    let shared_cache_root = daemon_shared_cache_root(workspace_root);
    let shared_objects_dir = shared_cache_root.join("objects");
    let shared_entry_path = daemon_shared_cache_entry_path(&shared_cache_root, session_key);
    let cached_artifacts = collect_cached_artifacts_for_entry(
        workspace_root,
        profile,
        &shared_cache_root,
        &shared_objects_dir,
    )?;
    let _cache_lock = acquire_cache_lock(&shared_cache_root)?;
    persist_cache_entry(profile, fingerprint, &cached_artifacts, &shared_entry_path)?;
    let max_bytes = daemon_shared_cache_max_bytes();
    if max_bytes > 0 {
        enforce_cache_size_budget_with_budget(&shared_cache_root, max_bytes)?;
    }
    Ok(())
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
    let _ = system.refresh_processes(ProcessesToUpdate::Some(&[target]), true);
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
    let edition = edition.unwrap_or("2024_07");

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
