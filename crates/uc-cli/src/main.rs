use anyhow::{bail, Context, Result};
use blake3::Hasher;
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
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
const MAX_CACHE_ENTRY_BYTES: u64 = 10 * 1024 * 1024;
const MAX_FINGERPRINT_INDEX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ARTIFACT_INDEX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_CAPTURE_STDOUT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CAPTURE_STDERR_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_MAX_CACHE_BYTES: u64 = 10 * 1024 * 1024 * 1024;
const FINGERPRINT_INDEX_SCHEMA_VERSION: u32 = 2;
const ARTIFACT_INDEX_SCHEMA_VERSION: u32 = 1;
const DEFAULT_DIAGNOSTICS_SIMILARITY_THRESHOLD: f64 = 99.5;
const DAEMON_REQUEST_SIZE_LIMIT_BYTES: usize = 1024 * 1024;
const DAEMON_RATE_WINDOW_SECONDS: u64 = 1;
const DAEMON_MAX_REQUESTS_PER_WINDOW: usize = 32;
const DAEMON_LOG_ROTATE_BYTES: u64 = 10 * 1024 * 1024;
const ASYNC_PERSIST_ERROR_QUEUE_LIMIT: usize = 32;
const CACHE_LOCK_STALE_AFTER_SECONDS: u64 = 300;
const DEFAULT_MIN_SCARB_VERSION: &str = "2.14.0";
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
    #[command(hide = true)]
    Benchmark(BenchmarkArgs),
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
enum MatrixArg {
    Research,
    Smoke,
}

impl MatrixArg {
    fn as_str(self) -> &'static str {
        match self {
            MatrixArg::Research => "research",
            MatrixArg::Smoke => "smoke",
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum BenchmarkToolArg {
    Scarb,
    Uc,
}

impl BenchmarkToolArg {
    fn as_str(self) -> &'static str {
        match self {
            BenchmarkToolArg::Scarb => "scarb",
            BenchmarkToolArg::Uc => "uc",
        }
    }
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
struct BenchmarkArgs {
    #[arg(long, value_enum, default_value_t = MatrixArg::Research)]
    matrix: MatrixArg,

    #[arg(long, value_enum, default_value_t = BenchmarkToolArg::Scarb)]
    tool: BenchmarkToolArg,

    #[arg(long, default_value_t = 5)]
    runs: u32,

    #[arg(long, default_value_t = 3)]
    cold_runs: u32,

    #[arg(long)]
    workspace_root: Option<String>,
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

    #[arg(long, default_value_t = 1)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonStatusPayload {
    pid: u32,
    started_at_epoch_ms: u64,
    socket_path: String,
    healthy: bool,
    total_requests: u64,
    failed_requests: u64,
    rate_limited_requests: u64,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonBuildRequest {
    manifest_path: String,
    package: Option<String>,
    workspace: bool,
    features: Vec<String>,
    offline: bool,
    release: bool,
    profile: Option<String>,
    #[serde(default)]
    async_cache_persist: bool,
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

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct FingerprintIndex {
    schema_version: u32,
    entries: BTreeMap<String, FingerprintIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        Commands::Benchmark(args) => run_benchmark(args),
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

fn fail_on_async_cache_error() -> bool {
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| parse_env_bool("UC_FAIL_ON_ASYNC_CACHE_ERROR", false))
}

fn daemon_async_cache_persist_enabled() -> bool {
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| parse_env_bool("UC_DAEMON_ASYNC_CACHE_PERSIST", false))
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
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .with_context(|| format!("failed to open daemon log {}", log_path.display()))?;
        let log_file_err = log_file
            .try_clone()
            .with_context(|| format!("failed to clone log file {}", log_path.display()))?;

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
            "uc daemon running (pid={}, started_at_epoch_ms={}, socket={}, healthy={}, total_requests={}, failed_requests={}, rate_limited_requests={}, last_error={})",
            status.pid,
            status.started_at_epoch_ms,
            status.socket_path,
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
        while Instant::now() < deadline {
            if daemon_ping(&socket_path).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        remove_socket_if_exists(&socket_path)?;
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
        let status = DaemonStatusPayload {
            pid: std::process::id(),
            started_at_epoch_ms: epoch_ms_u64()?,
            socket_path: socket_path.display().to_string(),
            healthy: true,
            total_requests: 0,
            failed_requests: 0,
            rate_limited_requests: 0,
            last_error: None,
        };
        let health = Arc::new(Mutex::new(DaemonHealth::default()));
        let mut rate_limiter = DaemonRateLimiter::new();

        let mut should_shutdown = false;
        for incoming in listener.incoming() {
            match incoming {
                Ok(stream) => {
                    if let Err(err) = handle_daemon_connection(
                        stream,
                        &status,
                        &health,
                        &mut should_shutdown,
                        &mut rate_limiter,
                    ) {
                        record_daemon_failure(&health, format!("{err:#}"));
                        tracing::error!(error = %format!("{err:#}"), "daemon request handling failed");
                        eprintln!("uc daemon: request handling failed: {err:#}");
                    }
                }
                Err(err) => {
                    tracing::error!(error = %err, "daemon socket accept failed");
                    eprintln!("uc daemon: socket accept failed: {err}");
                }
            }
            if should_shutdown {
                break;
            }
        }
        remove_socket_if_exists(&socket_path)?;
        Ok(())
    }
}

fn run_benchmark(args: BenchmarkArgs) -> Result<()> {
    if !parse_env_bool("UC_ENABLE_BENCHMARK_COMMAND", false) {
        bail!(
            "`uc benchmark` is a development-only command. Set UC_ENABLE_BENCHMARK_COMMAND=1 to enable it."
        );
    }
    let script = if let Some(path) = std::env::var_os("UC_BENCHMARK_SCRIPT") {
        PathBuf::from(path)
    } else if let Some(root) = std::env::var_os("UC_BENCHMARK_REPO_ROOT") {
        PathBuf::from(root).join("benchmarks/scripts/run_local_benchmarks.sh")
    } else {
        bail!("`uc benchmark` requires UC_BENCHMARK_SCRIPT or UC_BENCHMARK_REPO_ROOT to be set");
    };

    if !script.exists() {
        bail!(
            "benchmark script not found at {}. Set UC_BENCHMARK_SCRIPT or UC_BENCHMARK_REPO_ROOT.",
            script.display()
        );
    }

    let mut command = Command::new(&script);
    command
        .arg("--matrix")
        .arg(args.matrix.as_str())
        .arg("--tool")
        .arg(args.tool.as_str())
        .arg("--runs")
        .arg(args.runs.to_string())
        .arg("--cold-runs")
        .arg(args.cold_runs.to_string());
    if let Some(workspace_root) = args.workspace_root {
        command.arg("--workspace-root").arg(workspace_root);
    }
    let status = command
        .status()
        .context("failed to execute benchmark script")?;

    if !status.success() {
        bail!("benchmark script exited with status {status}");
    }

    Ok(())
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
    };

    println!("{}", input.deterministic_key_hex());
    Ok(())
}

fn run_build(args: BuildArgs) -> Result<()> {
    validate_scarb_toolchain()?;
    let common = args.common;
    let report_path = args.report_path;
    let write_report = report_path.is_some();
    let engine = args.engine;
    let daemon_mode = args.daemon_mode;
    let manifest_path = resolve_manifest_path(&common.manifest_path)?;
    let workspace_root = manifest_path
        .parent()
        .context("manifest path has no parent")?
        .to_path_buf();
    let profile = effective_profile(&common);

    let mut session_key = String::new();
    let mut daemon_used = false;
    let mut phase_telemetry: Option<BuildPhaseTelemetry> = None;
    let (run, cache_hit, fingerprint) = match engine {
        EngineArg::Scarb => {
            let (command, command_vec) = scarb_build_command(&common, &manifest_path);
            let run = run_command(command, command_vec, write_report)?;
            let fingerprint = if write_report {
                compute_build_fingerprint(&workspace_root, &manifest_path, &common, &profile, None)?
            } else {
                String::new()
            };
            (run, false, fingerprint)
        }
        EngineArg::Uc => {
            let run_local =
                || -> Result<(CommandRun, bool, String, String, BuildPhaseTelemetry)> {
                    let local_session_key = build_session_input(&common, &manifest_path, &profile)?
                        .deterministic_key_hex();
                    let (run, cache_hit, fingerprint, telemetry) = run_build_with_uc_cache(
                        &common,
                        &manifest_path,
                        &workspace_root,
                        &profile,
                        &local_session_key,
                        false,
                        false,
                    )?;
                    Ok((run, cache_hit, fingerprint, local_session_key, telemetry))
                };

            match daemon_mode {
                DaemonModeArg::Off => {
                    let (run, cache_hit, fingerprint, local_session_key, telemetry) = run_local()?;
                    session_key = local_session_key;
                    phase_telemetry = Some(telemetry);
                    (run, cache_hit, fingerprint)
                }
                DaemonModeArg::Auto => {
                    if let Some(response) = try_uc_build_via_daemon(&common, &manifest_path)? {
                        daemon_used = true;
                        session_key = response.session_key;
                        phase_telemetry = Some(response.telemetry);
                        (response.run, response.cache_hit, response.fingerprint)
                    } else {
                        let (run, cache_hit, fingerprint, local_session_key, telemetry) =
                            run_local()?;
                        session_key = local_session_key;
                        phase_telemetry = Some(telemetry);
                        (run, cache_hit, fingerprint)
                    }
                }
                DaemonModeArg::Require => {
                    let response = try_uc_build_via_daemon(&common, &manifest_path)?
                        .context("daemon mode is require but daemon is unavailable")?;
                    daemon_used = true;
                    session_key = response.session_key;
                    phase_telemetry = Some(response.telemetry);
                    (response.run, response.cache_hit, response.fingerprint)
                }
            }
        }
    };
    replay_output(&run.stdout, &run.stderr)?;
    if should_log_phase_telemetry() {
        if let Some(telemetry) = phase_telemetry.as_ref() {
            eprintln!(
                "uc: phase timings (ms) fingerprint={:.3} cache_lookup={:.3} cache_restore={:.3} compile={:.3} cache_persist={:.3} async={} scheduled={} daemon_used={} cache_hit={}",
                telemetry.fingerprint_ms,
                telemetry.cache_lookup_ms,
                telemetry.cache_restore_ms,
                telemetry.compile_ms,
                telemetry.cache_persist_ms,
                telemetry.cache_persist_async,
                telemetry.cache_persist_scheduled,
                daemon_used,
                cache_hit
            );
        }
    }
    if session_key.is_empty() {
        session_key = "n/a".to_string();
    }

    if let Some(path) = report_path {
        let artifacts = collect_profile_artifacts(&workspace_root, &profile)?;
        let report = BuildReport {
            generated_at_epoch_ms: epoch_ms()?,
            engine: engine.as_str().to_string(),
            daemon_used,
            manifest_path: manifest_path.display().to_string(),
            workspace_root: workspace_root.display().to_string(),
            profile,
            session_key,
            command: run.command.clone(),
            exit_code: run.exit_code,
            elapsed_ms: run.elapsed_ms,
            cache_hit,
            fingerprint,
            artifact_count: artifacts.len(),
            phase_telemetry,
        };
        write_json_report(&path, &report)?;
    }

    if run.exit_code != 0 {
        bail!("build failed with exit code {}", run.exit_code);
    }

    Ok(())
}

fn try_uc_build_via_daemon(
    common: &BuildCommonArgs,
    manifest_path: &Path,
) -> Result<Option<DaemonBuildResponse>> {
    #[cfg(not(unix))]
    {
        let _ = (common, manifest_path);
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
        ));
        let response = match daemon_request(&socket_path, &request) {
            Ok(response) => response,
            Err(err) => {
                eprintln!(
                    "uc: daemon request failed ({}), falling back to local engine",
                    err
                );
                return Ok(None);
            }
        };

        match response {
            DaemonResponse::Build(result) => Ok(Some(result)),
            DaemonResponse::Error { message } => {
                eprintln!(
                    "uc: daemon returned error ({}), falling back to local engine",
                    message
                );
                Ok(None)
            }
            _ => Ok(None),
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
                    eprintln!(
                        "uc: daemon returned error ({}), falling back to local metadata",
                        message
                    );
                    Ok(None)
                } else {
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
) -> DaemonBuildRequest {
    DaemonBuildRequest {
        manifest_path: manifest_path.display().to_string(),
        package: common.package.clone(),
        workspace: common.workspace,
        features: common.features.clone(),
        offline: common.offline,
        release: common.release,
        profile: common.profile.clone(),
        async_cache_persist,
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
    let common = common_args_from_daemon_request(&request);
    let manifest_path = resolve_manifest_path(&common.manifest_path)?;
    let workspace_root = manifest_path
        .parent()
        .context("manifest path has no parent")?
        .to_path_buf();
    let profile = effective_profile(&common);
    let session_input = build_session_input(&common, &manifest_path, &profile)?;
    let session_key = session_input.deterministic_key_hex();

    let (run, cache_hit, fingerprint, telemetry) = run_build_with_uc_cache(
        &common,
        &manifest_path,
        &workspace_root,
        &profile,
        &session_key,
        true,
        request.async_cache_persist,
    )?;

    Ok(DaemonBuildResponse {
        run,
        cache_hit,
        fingerprint,
        session_key,
        telemetry,
    })
}

fn execute_daemon_metadata(request: DaemonMetadataRequest) -> Result<DaemonMetadataResponse> {
    let args = metadata_args_from_daemon_request(&request);
    let manifest_path = resolve_manifest_path(&args.manifest_path)?;
    let (command, command_vec) = scarb_metadata_command(&args, &manifest_path);
    let run = run_command(command, command_vec, request.capture_output)?;
    Ok(DaemonMetadataResponse { run })
}

fn run_metadata(args: MetadataArgs) -> Result<()> {
    let manifest_path = resolve_manifest_path(&args.manifest_path)?;
    let write_report = args.report_path.is_some();

    let run = match args.daemon_mode {
        DaemonModeArg::Off => {
            let (command, command_vec) = scarb_metadata_command(&args, &manifest_path);
            run_command(command, command_vec, write_report)?
        }
        DaemonModeArg::Auto => {
            if let Some(run) =
                try_uc_metadata_via_daemon(&args, &manifest_path, write_report, true)?
            {
                run
            } else {
                let (command, command_vec) = scarb_metadata_command(&args, &manifest_path);
                run_command(command, command_vec, write_report)?
            }
        }
        DaemonModeArg::Require => {
            try_uc_metadata_via_daemon(&args, &manifest_path, write_report, false)?
                .context("daemon mode is require but daemon is unavailable")?
        }
    };

    if write_report {
        replay_output(&run.stdout, &run.stderr)?;
    }

    if let Some(path) = args.report_path {
        let report = MetadataReport {
            generated_at_epoch_ms: epoch_ms()?,
            manifest_path: manifest_path.display().to_string(),
            command: run.command.clone(),
            exit_code: run.exit_code,
            elapsed_ms: run.elapsed_ms,
        };
        write_json_report(&path, &report)?;
    }

    if run.exit_code != 0 {
        bail!("metadata failed with exit code {}", run.exit_code);
    }

    Ok(())
}

fn run_migrate(args: MigrateArgs) -> Result<()> {
    let manifest_path = resolve_manifest_path(&args.manifest_path)?;
    let workspace_root = manifest_path
        .parent()
        .context("manifest path has no parent")?
        .to_path_buf();
    let raw = read_text_file_with_limit(&manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
    let parsed: TomlValue = raw
        .parse()
        .with_context(|| format!("failed to parse TOML in {}", manifest_path.display()))?;

    let package = parsed.get("package").and_then(TomlValue::as_table);
    let package_name = package
        .and_then(|tbl| tbl.get("name"))
        .and_then(TomlValue::as_str)
        .map(str::to_string);
    let package_version = package
        .and_then(|tbl| tbl.get("version"))
        .and_then(TomlValue::as_str)
        .map(str::to_string);
    let edition = package
        .and_then(|tbl| tbl.get("edition"))
        .and_then(TomlValue::as_str)
        .map(str::to_string);

    let dependency_count = parsed
        .get("dependencies")
        .and_then(TomlValue::as_table)
        .map_or(0, |tbl| tbl.len());
    let dev_dependency_count = parsed
        .get("dev-dependencies")
        .and_then(TomlValue::as_table)
        .map_or(0, |tbl| tbl.len());

    let known_sections = [
        "package",
        "dependencies",
        "dev-dependencies",
        "workspace",
        "target",
        "scripts",
        "tool",
        "features",
        "patch",
        "cairo",
        "lib",
        "executable",
        "test",
    ];

    let unknown_sections = parsed
        .as_table()
        .map(|tbl| {
            let mut keys: Vec<String> = tbl
                .keys()
                .filter(|k| !known_sections.contains(&k.as_str()))
                .cloned()
                .collect();
            keys.sort();
            keys
        })
        .unwrap_or_default();

    let mut warnings = Vec::new();
    if package_name.is_none() {
        warnings.push("missing [package].name".to_string());
    }
    if edition.is_none() {
        warnings.push("missing [package].edition".to_string());
    }
    if !unknown_sections.is_empty() {
        warnings.push(format!(
            "unknown top-level sections detected: {}",
            unknown_sections.join(", ")
        ));
    }

    let report = MigrationReport {
        generated_at_epoch_ms: epoch_ms()?,
        manifest_path: manifest_path.display().to_string(),
        workspace_root: workspace_root.display().to_string(),
        package_name: package_name.clone(),
        package_version: package_version.clone(),
        edition: edition.clone(),
        dependency_count,
        dev_dependency_count,
        unknown_sections,
        warnings,
        suggested_next_steps: vec![
            "Run `uc compare-build` to establish artifact parity before migration.".to_string(),
            "Define migration owner and target milestone for this workspace.".to_string(),
            "Prepare `Uc.toml` and validate in CI shadow lane.".to_string(),
        ],
    };

    let report_path = args
        .report_path
        .unwrap_or_else(|| workspace_root.join("uc-migration-report.json"));
    write_json_report(&report_path, &report)?;
    println!("Migration report: {}", report_path.display());

    if let Some(uc_toml_path) = args.emit_uc_toml {
        write_uc_toml(
            &uc_toml_path,
            &manifest_path,
            package_name.as_deref(),
            package_version.as_deref(),
            edition.as_deref(),
        )?;
        println!("Generated Uc.toml scaffold: {}", uc_toml_path.display());
    }

    Ok(())
}

fn run_compare_build(args: CompareBuildArgs) -> Result<()> {
    validate_scarb_toolchain()?;
    let common = args.common;
    let manifest_path = resolve_manifest_path(&common.manifest_path)?;
    let workspace_root = manifest_path
        .parent()
        .context("manifest path has no parent")?
        .to_path_buf();
    let profile = effective_profile(&common);

    if args.clean_before_each {
        remove_build_outputs(&workspace_root)?;
    }

    let (baseline_command, baseline_vec) = scarb_build_command(&common, &manifest_path);
    let baseline_run = run_command_capture(baseline_command, baseline_vec)?;
    let baseline_artifacts = collect_profile_artifacts(&workspace_root, &profile)?;
    let baseline_diag = extract_diagnostic_lines(&baseline_run.stderr);

    if args.clean_before_each {
        remove_build_outputs(&workspace_root)?;
    }

    let candidate_run = run_uc_build_subprocess(&common, &manifest_path, EngineArg::Uc)?;
    let candidate_artifacts = collect_profile_artifacts(&workspace_root, &profile)?;
    let candidate_diag = extract_diagnostic_lines(&candidate_run.stderr);

    let mismatches = compare_artifact_sets(&baseline_artifacts, &candidate_artifacts);
    let diagnostics = compare_diagnostics(&baseline_diag, &candidate_diag);
    let artifacts_match = mismatches.is_empty();
    let diagnostics_threshold = resolve_diagnostics_threshold(args.diagnostics_threshold)?;
    let diagnostics_ok = diagnostics.similarity_percent >= diagnostics_threshold;

    let report = CompareBuildReport {
        generated_at_epoch_ms: epoch_ms()?,
        manifest_path: manifest_path.display().to_string(),
        workspace_root: workspace_root.display().to_string(),
        clean_before_each: args.clean_before_each,
        diagnostics_threshold,
        baseline: CompareRunSnapshot {
            label: "scarb-direct".to_string(),
            command: baseline_run.command.clone(),
            exit_code: baseline_run.exit_code,
            elapsed_ms: baseline_run.elapsed_ms,
            artifact_count: baseline_artifacts.len(),
            diagnostics: baseline_diag,
        },
        candidate: CompareRunSnapshot {
            label: "uc-engine".to_string(),
            command: candidate_run.command.clone(),
            exit_code: candidate_run.exit_code,
            elapsed_ms: candidate_run.elapsed_ms,
            artifact_count: candidate_artifacts.len(),
            diagnostics: candidate_diag,
        },
        diagnostics,
        artifact_mismatch_count: mismatches.len(),
        artifact_mismatches: mismatches,
        passed: baseline_run.exit_code == 0
            && candidate_run.exit_code == 0
            && artifacts_match
            && diagnostics_ok,
    };

    let output_path = args.output_path.unwrap_or_else(|| {
        default_compare_output_path().unwrap_or_else(|_| PathBuf::from("compare-build-report.json"))
    });

    write_json_report(&output_path, &report)?;

    println!("Compare report: {}", output_path.display());
    println!(
        "Artifact mismatches: {} | Diagnostics similarity: {:.2}% (threshold: {:.2}%)",
        report.artifact_mismatch_count,
        report.diagnostics.similarity_percent,
        diagnostics_threshold
    );

    if !report.passed {
        bail!("compare-build gate failed");
    }

    Ok(())
}

fn run_build_with_uc_cache(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    workspace_root: &Path,
    profile: &str,
    session_key: &str,
    capture_output: bool,
    async_cache_persist: bool,
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
    let canonical_workspace_root = workspace_root.canonicalize().with_context(|| {
        format!(
            "failed to resolve workspace root for cache path {}",
            workspace_root.display()
        )
    })?;
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
    let cached_entry = {
        let _cache_lock = acquire_cache_lock(&cache_root)?;
        if let Some(entry) = load_cache_entry(&entry_path)? {
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
    telemetry.cache_lookup_ms = cache_lookup_start.elapsed().as_secs_f64() * 1000.0;

    if let Some(entry) = cached_entry {
        let restore_start = Instant::now();
        if restore_cached_artifacts(
            &canonical_workspace_root,
            profile,
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

    let (command, command_vec) = scarb_build_command(common, manifest_path);
    let run = run_command(command, command_vec, capture_output)?;
    telemetry.compile_ms = run.elapsed_ms;

    if run.exit_code == 0 {
        if async_cache_persist {
            telemetry.cache_persist_async = true;
            let persist_scope_key = async_persist_scope_key(&canonical_workspace_root, profile);
            let workspace_root = canonical_workspace_root.clone();
            let profile = profile.to_string();
            let fingerprint = fingerprint.clone();
            let cache_root = cache_root.clone();
            let objects_dir = objects_dir.clone();
            let entry_path = entry_path.clone();
            if try_mark_async_persist_in_flight(&persist_scope_key) {
                telemetry.cache_persist_scheduled = true;
                thread::spawn(move || {
                    let _guard = AsyncPersistGuard::new(persist_scope_key);
                    if let Err(err) = persist_cache_entry_for_build(
                        &workspace_root,
                        &profile,
                        &fingerprint,
                        &cache_root,
                        &objects_dir,
                        &entry_path,
                    ) {
                        record_async_persist_error(err.to_string());
                        tracing::warn!(error = %format!("{err:#}"), "async cache persistence failed");
                        eprintln!("uc: warning: async cache persistence failed: {err:#}");
                    }
                });
            }
        } else {
            let persist_start = Instant::now();
            persist_cache_entry_for_build(
                &canonical_workspace_root,
                profile,
                &fingerprint,
                &cache_root,
                &objects_dir,
                &entry_path,
            )?;
            telemetry.cache_persist_ms = persist_start.elapsed().as_secs_f64() * 1000.0;
        }
    }

    Ok((run, false, fingerprint, telemetry))
}

fn async_persist_scope_key(workspace_root: &Path, profile: &str) -> String {
    format!("{}::{profile}", workspace_root.display())
}

fn async_persist_in_flight_set() -> &'static Mutex<HashSet<String>> {
    static IN_FLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

fn try_mark_async_persist_in_flight(scope_key: &str) -> bool {
    let mut in_flight = async_persist_in_flight_set()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    in_flight.insert(scope_key.to_string())
}

fn clear_async_persist_in_flight(scope_key: &str) {
    let mut in_flight = async_persist_in_flight_set()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    in_flight.remove(scope_key);
}

fn async_persist_error_slot() -> &'static Mutex<VecDeque<String>> {
    static SLOT: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn record_async_persist_error(error: String) {
    let mut slot = async_persist_error_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    slot.push_back(error);
    while slot.len() > ASYNC_PERSIST_ERROR_QUEUE_LIMIT {
        slot.pop_front();
    }
}

fn take_async_persist_errors() -> Vec<String> {
    let mut slot = async_persist_error_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    slot.drain(..).collect()
}

struct AsyncPersistGuard {
    scope_key: String,
}

impl AsyncPersistGuard {
    fn new(scope_key: String) -> Self {
        Self { scope_key }
    }
}

impl Drop for AsyncPersistGuard {
    fn drop(&mut self) {
        clear_async_persist_in_flight(&self.scope_key);
    }
}

fn persist_cache_entry(
    profile: &str,
    fingerprint: &str,
    cached_artifacts: &[CachedArtifact],
    entry_path: &Path,
) -> Result<()> {
    let entry = BuildCacheEntry {
        schema_version: BUILD_CACHE_SCHEMA_VERSION,
        fingerprint: fingerprint.to_string(),
        profile: profile.to_string(),
        artifacts: cached_artifacts.to_vec(),
    };

    let bytes = serde_json::to_vec(&entry)?;
    atomic_write_bytes(entry_path, &bytes, "cache entry")?;

    Ok(())
}

fn persist_cache_entry_for_build(
    workspace_root: &Path,
    profile: &str,
    fingerprint: &str,
    cache_root: &Path,
    objects_dir: &Path,
    entry_path: &Path,
) -> Result<()> {
    let cached_artifacts =
        collect_cached_artifacts_for_entry(workspace_root, profile, cache_root, objects_dir)?;
    let _cache_lock = acquire_cache_lock(cache_root)?;
    persist_cache_entry(profile, fingerprint, &cached_artifacts, entry_path)?;
    enforce_cache_size_budget(cache_root)
}

fn enforce_cache_size_budget(cache_root: &Path) -> Result<()> {
    let budget = max_cache_bytes();
    if budget == 0 || !cache_root.exists() {
        return Ok(());
    }

    #[derive(Clone)]
    struct CacheFile {
        path: PathBuf,
        size: u64,
        modified_ms: u64,
        is_object: bool,
    }

    let mut files = Vec::<CacheFile>::new();
    let mut total = 0_u64;
    for entry in WalkDir::new(cache_root).follow_links(false).into_iter() {
        let entry = entry.with_context(|| {
            format!(
                "failed to traverse cache tree under {}",
                cache_root.display()
            )
        })?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !is_removable_cache_file(path) {
            continue;
        }
        let metadata =
            fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
        let size = metadata.len();
        total = total.saturating_add(size);
        let modified_ms = metadata_modified_unix_ms(&metadata).unwrap_or_default();
        let is_object = path
            .components()
            .any(|c| matches!(c, Component::Normal(name) if name == "objects"));
        files.push(CacheFile {
            path: path.to_path_buf(),
            size,
            modified_ms,
            is_object,
        });
    }

    if total <= budget {
        if total > (budget.saturating_mul(9) / 10) {
            eprintln!("uc: cache usage is high: {total} / {budget} bytes");
        }
        return Ok(());
    }

    files.sort_by(|a, b| {
        a.is_object
            .cmp(&b.is_object)
            .reverse()
            .then_with(|| a.modified_ms.cmp(&b.modified_ms))
            .then_with(|| a.path.cmp(&b.path))
    });

    let mut removed = 0_u64;
    for file in files {
        if total <= budget {
            break;
        }
        match fs::remove_file(&file.path) {
            Ok(()) => {
                total = total.saturating_sub(file.size);
                removed = removed.saturating_add(file.size);
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                total = total.saturating_sub(file.size);
            }
            Err(err) => {
                eprintln!(
                    "uc: warning: failed to evict cache file {}: {err}",
                    file.path.display()
                );
            }
        }
    }

    if removed > 0 {
        eprintln!(
            "uc: cache eviction removed {} bytes (budget {} bytes)",
            removed, budget
        );
    }
    if total > budget {
        eprintln!(
            "uc: warning: cache remains over budget after eviction ({} > {} bytes)",
            total, budget
        );
    }
    Ok(())
}

fn is_removable_cache_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
        return false;
    };
    name != ".lock"
}

fn load_cache_entry(path: &Path) -> Result<Option<BuildCacheEntry>> {
    if !path.exists() {
        return Ok(None);
    }

    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    let max_bytes = max_cache_entry_bytes();
    if metadata.len() > max_bytes {
        eprintln!(
            "uc: warning: ignoring oversized cache entry {} ({} bytes > {} bytes)",
            path.display(),
            metadata.len(),
            max_bytes
        );
        return Ok(None);
    }
    let file = File::open(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = BufReader::new(file).take(max_bytes + 1);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.len() as u64 > max_bytes {
        eprintln!(
            "uc: warning: ignoring oversized cache entry {} (>{} bytes)",
            path.display(),
            max_bytes
        );
        return Ok(None);
    }
    let parsed: BuildCacheEntry = match serde_json::from_slice(&bytes) {
        Ok(entry) => entry,
        Err(err) => {
            eprintln!(
                "uc: warning: ignoring unreadable cache entry {}: {}",
                path.display(),
                err
            );
            return Ok(None);
        }
    };
    Ok(Some(parsed))
}

fn restore_cached_artifacts(
    workspace_root: &Path,
    profile: &str,
    objects_dir: &Path,
    artifacts: &[CachedArtifact],
) -> Result<bool> {
    if artifacts.is_empty() {
        return Ok(false);
    }

    for artifact in artifacts {
        validate_hex_digest(
            "cached artifact blake3 hash",
            &artifact.blake3_hex,
            MIN_HASH_LEN,
        )?;
        validate_cache_object_rel_path(&artifact.object_rel_path)?;
        let object_path = objects_dir.join(&artifact.object_rel_path);
        if !object_path.exists() {
            return Ok(false);
        }
    }

    let target_root = workspace_root.join("target").join(profile);
    for artifact in artifacts {
        let expected_hash = &artifact.blake3_hex;
        let relative_path = validated_relative_artifact_path(&artifact.relative_path)?;
        let out_path = target_root.join(relative_path);
        ensure_path_within_root(&target_root, &out_path, "cache restore path")?;

        // Cache validity is content-addressed; permissions/ownership are not normalized.
        if out_path.exists() && hash_file_blake3(&out_path)? == *expected_hash {
            continue;
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let object_path = objects_dir.join(&artifact.object_rel_path);
        fs::copy(&object_path, &out_path).with_context(|| {
            format!(
                "failed to restore cache object {} to {}",
                object_path.display(),
                out_path.display()
            )
        })?;
    }

    let expected: Vec<ArtifactDigest> = artifacts
        .iter()
        .map(|item| ArtifactDigest {
            relative_path: item.relative_path.clone(),
            blake3_hex: item.blake3_hex.clone(),
            size_bytes: item.size_bytes,
        })
        .collect();
    let restored = collect_artifact_digests_fast(&target_root)?;
    Ok(compare_artifact_sets(&expected, &restored).is_empty())
}

fn hash_file_blake3(path: &Path) -> Result<String> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    if metadata.len() > MAX_CACHEABLE_ARTIFACT_BYTES {
        bail!(
            "file {} exceeds hashing size limit ({} bytes > {} bytes)",
            path.display(),
            metadata.len(),
            MAX_CACHEABLE_ARTIFACT_BYTES
        );
    }
    let file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut buf = [0_u8; 8192];
    let mut hasher = Hasher::new();

    loop {
        let read = reader
            .read(&mut buf)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

fn hash_fingerprint_source_file(path: &Path) -> Result<String> {
    let is_cairo = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("cairo"))
        .unwrap_or(false);
    if is_cairo {
        return hash_cairo_source_semantic(path);
    }
    hash_file_blake3(path)
}

fn hash_cairo_source_semantic(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut normalized = strip_cairo_comments(&bytes);
    while matches!(normalized.last(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        normalized.pop();
    }
    let mut hasher = Hasher::new();
    hasher.update(b"uc-cairo-semantic-hash-v1");
    hasher.update(&normalized);
    Ok(hasher.finalize().to_hex().to_string())
}

fn strip_cairo_comments(input: &[u8]) -> Vec<u8> {
    #[derive(Clone, Copy)]
    enum Mode {
        Code,
        LineComment,
        BlockComment { depth: u32 },
        SingleQuote,
        DoubleQuote,
    }

    let mut mode = Mode::Code;
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0_usize;

    while i < input.len() {
        let b = input[i];
        let next = input.get(i + 1).copied();

        match mode {
            Mode::Code => {
                if b == b'/' && next == Some(b'/') {
                    mode = Mode::LineComment;
                    i += 2;
                    continue;
                }
                if b == b'/' && next == Some(b'*') {
                    mode = Mode::BlockComment { depth: 1 };
                    i += 2;
                    continue;
                }
                out.push(b);
                if b == b'\'' {
                    mode = Mode::SingleQuote;
                } else if b == b'"' {
                    mode = Mode::DoubleQuote;
                }
                i += 1;
            }
            Mode::LineComment => {
                if b == b'\n' {
                    out.push(b'\n');
                    mode = Mode::Code;
                }
                i += 1;
            }
            Mode::BlockComment { depth } => {
                if b == b'/' && next == Some(b'*') {
                    mode = Mode::BlockComment { depth: depth + 1 };
                    i += 2;
                    continue;
                }
                if b == b'*' && next == Some(b'/') {
                    if depth <= 1 {
                        mode = Mode::Code;
                    } else {
                        mode = Mode::BlockComment { depth: depth - 1 };
                    }
                    i += 2;
                    continue;
                }
                if b == b'\n' {
                    out.push(b'\n');
                }
                i += 1;
            }
            Mode::SingleQuote => {
                out.push(b);
                if b == b'\\' {
                    if let Some(escaped) = next {
                        out.push(escaped);
                        i += 2;
                        continue;
                    }
                }
                if b == b'\'' {
                    mode = Mode::Code;
                }
                i += 1;
            }
            Mode::DoubleQuote => {
                out.push(b);
                if b == b'\\' {
                    if let Some(escaped) = next {
                        out.push(escaped);
                        i += 2;
                        continue;
                    }
                }
                if b == b'"' {
                    mode = Mode::Code;
                }
                i += 1;
            }
        }
    }

    out
}

fn metadata_modified_unix_ms(metadata: &fs::Metadata) -> Result<u64> {
    let modified = metadata
        .modified()
        .context("failed to read file modification time")?;
    let since_epoch = modified.duration_since(UNIX_EPOCH).unwrap_or_default();
    u64::try_from(since_epoch.as_millis()).context("file modified time overflowed u64")
}

fn normalize_fingerprint_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let without_windows_prefix = raw.strip_prefix("\\\\?\\").unwrap_or(&raw);
    without_windows_prefix.replace('\\', "/")
}

fn atomic_write_bytes(path: &Path, bytes: &[u8], label: &str) -> Result<()> {
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);
    let parent = path
        .parent()
        .context("cannot atomically write file without parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let stem = path.file_name().and_then(|v| v.to_str()).unwrap_or("file");
    let temp_id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let thread_id = format!("{:?}", thread::current().id());
    let temp_path = parent.join(format!(
        ".{stem}.tmp.{}.{}.{}.{}",
        std::process::id(),
        thread_id,
        temp_id,
        epoch_ms_u64().unwrap_or_default()
    ));
    fs::write(&temp_path, bytes).with_context(|| {
        format!(
            "failed to write temporary {label} file {}",
            temp_path.display()
        )
    })?;
    if let Err(err) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(err).with_context(|| {
            format!(
                "failed to move temporary {label} {} to {}",
                temp_path.display(),
                path.display()
            )
        });
    }
    Ok(())
}

fn load_fingerprint_index(path: &Path) -> Result<FingerprintIndex> {
    if !path.exists() {
        return Ok(FingerprintIndex::empty());
    }
    let bytes = read_bytes_with_limit(path, max_fingerprint_index_bytes(), "fingerprint index")?;
    match serde_json::from_slice::<FingerprintIndex>(&bytes) {
        Ok(index) if index.schema_version == FINGERPRINT_INDEX_SCHEMA_VERSION => Ok(index),
        Ok(_) => Ok(FingerprintIndex::empty()),
        Err(err) => {
            eprintln!(
                "uc: warning: ignoring unreadable fingerprint index {}: {}",
                path.display(),
                err
            );
            Ok(FingerprintIndex::empty())
        }
    }
}

fn save_fingerprint_index(path: &Path, index: &FingerprintIndex) -> Result<()> {
    let bytes = serde_json::to_vec(index).context("failed to encode fingerprint index")?;
    atomic_write_bytes(path, &bytes, "fingerprint index")?;
    Ok(())
}

fn load_artifact_index(path: &Path) -> Result<ArtifactIndex> {
    if !path.exists() {
        return Ok(ArtifactIndex::empty());
    }
    let bytes = read_bytes_with_limit(path, max_artifact_index_bytes(), "artifact index")?;
    match serde_json::from_slice::<ArtifactIndex>(&bytes) {
        Ok(index) if index.schema_version == ARTIFACT_INDEX_SCHEMA_VERSION => Ok(index),
        Ok(_) => Ok(ArtifactIndex::empty()),
        Err(err) => {
            eprintln!(
                "uc: warning: ignoring unreadable artifact index {}: {}",
                path.display(),
                err
            );
            Ok(ArtifactIndex::empty())
        }
    }
}

fn save_artifact_index(path: &Path, index: &ArtifactIndex) -> Result<()> {
    let bytes = serde_json::to_vec(index).context("failed to encode artifact index")?;
    atomic_write_bytes(path, &bytes, "artifact index")?;
    Ok(())
}

fn compute_build_fingerprint(
    workspace_root: &Path,
    manifest_path: &Path,
    common: &BuildCommonArgs,
    profile: &str,
    cache_root: Option<&Path>,
) -> Result<String> {
    let scarb_version = scarb_version_line()?;
    compute_build_fingerprint_with_scarb_version(
        workspace_root,
        manifest_path,
        common,
        profile,
        cache_root,
        &scarb_version,
    )
}

fn compute_build_fingerprint_with_scarb_version(
    workspace_root: &Path,
    manifest_path: &Path,
    common: &BuildCommonArgs,
    profile: &str,
    cache_root: Option<&Path>,
    scarb_version: &str,
) -> Result<String> {
    let mut hasher = Hasher::new();
    hasher.update(b"uc-build-fingerprint-v1");
    hasher.update(scarb_version.as_bytes());
    let canonical_manifest = manifest_path
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", manifest_path.display()))?;
    hasher.update(normalize_fingerprint_path(&canonical_manifest).as_bytes());
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

    let mut features = common.features.clone();
    features.sort_unstable();
    features.dedup();
    for feature in features {
        hasher.update(feature.as_bytes());
        hasher.update(b",");
    }

    let (index_path, mut index) = if let Some(root) = cache_root {
        let path = root.join("fingerprint/index-v1.json");
        (Some(path.clone()), load_fingerprint_index(&path)?)
    } else {
        (None, FingerprintIndex::empty())
    };
    let max_files = max_fingerprint_files();
    let max_file_bytes = max_fingerprint_file_bytes();
    let max_total_bytes = max_fingerprint_total_bytes();
    let fingerprint_timeout = Duration::from_millis(fingerprint_timeout_ms());
    let fingerprint_started = Instant::now();
    let mtime_recheck_window_ms = fingerprint_mtime_recheck_window_ms();
    let now_ms = epoch_ms_u64().unwrap_or_default();
    let mut updated_entries: BTreeMap<String, FingerprintIndexEntry> = BTreeMap::new();

    let mut files = Vec::new();
    let walker = WalkDir::new(workspace_root)
        .follow_links(false)
        .max_depth(MAX_FINGERPRINT_DEPTH)
        .into_iter()
        .filter_entry(|entry| !is_ignored_entry(workspace_root, entry.path()));

    for entry in walker.filter_map(|e| e.ok()) {
        if fingerprint_started.elapsed() > fingerprint_timeout {
            bail!(
                "fingerprinting timed out after {} ms",
                fingerprint_timeout.as_millis()
            );
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if should_include_fingerprint_file(path) {
            if files.len() >= max_files {
                bail!(
                    "workspace has too many fingerprintable files (>{max_files}); refusing to hash more"
                );
            }
            files.push(path.to_path_buf());
        }
    }
    files.sort();
    let mut total_fingerprint_bytes = 0_u64;

    for path in files {
        if fingerprint_started.elapsed() > fingerprint_timeout {
            bail!(
                "fingerprinting timed out after {} ms",
                fingerprint_timeout.as_millis()
            );
        }
        let rel = path
            .strip_prefix(workspace_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let metadata =
            fs::metadata(&path).with_context(|| format!("failed to stat {}", path.display()))?;
        let file_size = metadata.len();
        if file_size > max_file_bytes {
            bail!(
                "fingerprint file {} exceeds size limit ({} bytes > {} bytes)",
                path.display(),
                file_size,
                max_file_bytes
            );
        }
        total_fingerprint_bytes = total_fingerprint_bytes.saturating_add(file_size);
        if total_fingerprint_bytes > max_total_bytes {
            bail!(
                "fingerprint source budget exceeded ({} bytes > {} bytes)",
                total_fingerprint_bytes,
                max_total_bytes
            );
        }
        let modified_unix_ms = metadata_modified_unix_ms(&metadata)?;
        let file_hash = if let Some(cached) = index.entries.get(&rel) {
            let should_rehash_recent =
                now_ms.saturating_sub(modified_unix_ms) <= mtime_recheck_window_ms;
            if cached.size_bytes == file_size
                && cached.modified_unix_ms == modified_unix_ms
                && !should_rehash_recent
            {
                cached.blake3_hex.clone()
            } else {
                hash_fingerprint_source_file(&path)?
            }
        } else {
            hash_fingerprint_source_file(&path)?
        };
        updated_entries.insert(
            rel.clone(),
            FingerprintIndexEntry {
                size_bytes: file_size,
                modified_unix_ms,
                blake3_hex: file_hash.clone(),
            },
        );
        hasher.update(rel.as_bytes());
        hasher.update(b":");
        hasher.update(file_hash.as_bytes());
        hasher.update(b"\n");
    }
    if let Some(path) = index_path {
        index.schema_version = FINGERPRINT_INDEX_SCHEMA_VERSION;
        index.entries = updated_entries;
        if let Err(err) = save_fingerprint_index(&path, &index) {
            eprintln!(
                "uc: warning: failed to update fingerprint index {}: {err:#}",
                path.display()
            );
        }
    }

    Ok(hasher.finalize().to_hex().to_string())
}

fn is_ignored_entry(root: &Path, path: &Path) -> bool {
    if path == root {
        return false;
    }
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    matches!(name, ".git" | "target" | ".scarb" | ".uc")
}

fn should_include_fingerprint_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };

    if matches!(name, "Scarb.toml" | "Scarb.lock" | "Uc.toml") {
        return true;
    }

    path.extension()
        .and_then(|s| s.to_str())
        .map(|ext| ext == "cairo")
        .unwrap_or(false)
}

fn scarb_build_command(common: &BuildCommonArgs, manifest_path: &Path) -> (Command, Vec<String>) {
    let mut command = Command::new("scarb");
    let mut command_vec = vec!["scarb".to_string()];

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
    let mut index = load_artifact_index(&index_path)?;
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
        if !object_path.exists() {
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
    save_artifact_index(&index_path, &index)
        .with_context(|| format!("failed to update artifact index {}", index_path.display()))?;
    Ok(cached_artifacts)
}

fn persist_artifact_object(source: &Path, destination: &Path) -> Result<()> {
    match fs::hard_link(source, destination) {
        Ok(()) => Ok(()),
        Err(err) => {
            if err.kind() == io::ErrorKind::AlreadyExists {
                return Ok(());
            }
            fs::copy(source, destination).with_context(|| {
                format!(
                    "failed to copy artifact {} to {}",
                    source.display(),
                    destination.display()
                )
            })?;
            Ok(())
        }
    }
}

fn collect_artifact_digests_fast(target_root: &Path) -> Result<Vec<ArtifactDigest>> {
    if !target_root.exists() {
        return Ok(Vec::new());
    }
    let mut digests = Vec::new();
    for entry in WalkDir::new(target_root).follow_links(false).into_iter() {
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
        let hash = hash_file_blake3(path)?;
        let relative_path = path
            .strip_prefix(target_root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        digests.push(ArtifactDigest {
            relative_path,
            blake3_hex: hash,
            size_bytes: metadata.len(),
        });
    }
    digests.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(digests)
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
    let cwd = std::env::current_dir()?.canonicalize()?;
    let requested = manifest_path
        .as_ref()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("Scarb.toml"));
    let candidate = if requested.is_absolute() {
        requested.clone()
    } else {
        cwd.join(&requested)
    };
    let resolved = candidate
        .canonicalize()
        .with_context(|| format!("failed to resolve manifest path {}", candidate.display()))?;

    if !requested.is_absolute() && !resolved.starts_with(&cwd) {
        bail!(
            "manifest path escapes current working directory: {}",
            requested.display()
        );
    }

    if resolved.file_name().and_then(|s| s.to_str()) != Some("Scarb.toml") {
        bail!(
            "manifest path must reference Scarb.toml, got {}",
            resolved.display()
        );
    }

    Ok(resolved)
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

fn build_session_input(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    profile: &str,
) -> Result<SessionInput> {
    let scarb_version = scarb_version_line()?;
    let manifest_content_hash = compute_manifest_content_hash(manifest_path)?;
    let mut cfg_set = build_session_cfg_set(manifest_path)?;
    cfg_set.push(format!("workspace:{}", common.workspace));
    cfg_set.push(format!("release:{}", common.release));
    Ok(SessionInput {
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
    })
}

fn build_session_cfg_set(manifest_path: &Path) -> Result<Vec<String>> {
    let manifest_text = read_text_file_with_limit(manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
    let manifest = manifest_text.parse::<TomlValue>().with_context(|| {
        format!(
            "failed to parse manifest for session key {}",
            manifest_path.display()
        )
    })?;
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

fn compute_manifest_content_hash(manifest_path: &Path) -> Result<String> {
    let bytes = read_bytes_with_limit(manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
    let mut hasher = Hasher::new();
    hasher.update(&bytes);
    Ok(format!("manifest-blake3:{}", hasher.finalize().to_hex()))
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
            if !is_process_alive(pid) {
                fs::remove_file(lock_path).with_context(|| {
                    format!("failed to remove stale lock {}", lock_path.display())
                })?;
                return Ok(true);
            }
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
    if age > Duration::from_secs(CACHE_LOCK_STALE_AFTER_SECONDS) {
        fs::remove_file(lock_path)
            .with_context(|| format!("failed to remove stale lock {}", lock_path.display()))?;
        return Ok(true);
    }
    Ok(false)
}

fn lock_file_pid(contents: &str) -> Option<u32> {
    contents.lines().find_map(|line| {
        let value = line.strip_prefix("pid=")?;
        value.trim().parse::<u32>().ok()
    })
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

fn daemon_socket_path(override_path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        return Ok(path);
    }
    if let Some(path) = std::env::var_os("UC_DAEMON_SOCKET_PATH") {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").context("HOME is not set; provide --socket-path")?;
    Ok(PathBuf::from(home).join(".uc/daemon/uc.sock"))
}

fn daemon_log_path(socket_path: &Path) -> PathBuf {
    socket_path.with_extension("log")
}

fn remove_socket_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("failed to remove {}", path.display())),
    }
}

fn rotate_daemon_log_if_needed(log_path: &Path) -> Result<()> {
    let metadata = match fs::metadata(log_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(err).with_context(|| format!("failed to stat {}", log_path.display()));
        }
    };
    if metadata.len() < DAEMON_LOG_ROTATE_BYTES {
        return Ok(());
    }
    let rotated = PathBuf::from(format!("{}.1", log_path.display()));
    if rotated.exists() {
        fs::remove_file(&rotated)
            .with_context(|| format!("failed to remove {}", rotated.display()))?;
    }
    fs::rename(log_path, &rotated).with_context(|| {
        format!(
            "failed to rotate daemon log {} to {}",
            log_path.display(),
            rotated.display()
        )
    })?;
    Ok(())
}

fn read_line_limited<R: BufRead>(reader: &mut R, max_bytes: usize, label: &str) -> Result<String> {
    let mut bytes = Vec::with_capacity(128);
    loop {
        let mut byte = [0_u8; 1];
        let read = reader
            .read(&mut byte)
            .with_context(|| format!("failed to read {label}"))?;
        if read == 0 {
            break;
        }
        if byte[0] == b'\n' {
            break;
        }
        bytes.push(byte[0]);
        if bytes.len() > max_bytes {
            bail!("{label} exceeds size limit ({max_bytes} bytes)");
        }
    }
    if bytes.is_empty() {
        return Ok(String::new());
    }
    String::from_utf8(bytes).with_context(|| format!("{label} is not valid UTF-8"))
}

#[cfg(unix)]
fn daemon_request(socket_path: &Path, request: &DaemonRequest) -> Result<DaemonResponse> {
    let mut stream = UnixStream::connect(socket_path)
        .with_context(|| format!("failed to connect daemon socket {}", socket_path.display()))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(120)))
        .with_context(|| format!("failed to set read timeout for {}", socket_path.display()))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .with_context(|| format!("failed to set write timeout for {}", socket_path.display()))?;

    let payload = serde_json::to_vec(request).context("failed to encode daemon request")?;
    stream
        .write_all(&payload)
        .context("failed to write daemon request payload")?;
    stream
        .write_all(b"\n")
        .context("failed to write daemon request newline")?;
    stream.flush().context("failed to flush daemon request")?;

    let response_line = {
        let mut reader = BufReader::new(&mut stream);
        read_line_limited(
            &mut reader,
            DAEMON_REQUEST_SIZE_LIMIT_BYTES,
            "daemon response",
        )?
    };
    if response_line.trim().is_empty() {
        bail!("daemon returned empty response");
    }
    serde_json::from_str(response_line.trim_end()).context("failed to decode daemon response")
}

#[cfg(unix)]
fn daemon_ping(socket_path: &Path) -> Result<DaemonStatusPayload> {
    match daemon_request(socket_path, &DaemonRequest::Ping)? {
        DaemonResponse::Pong(status) => Ok(status),
        DaemonResponse::Error { message } => bail!("daemon ping failed: {message}"),
        _ => bail!("unexpected daemon response to ping"),
    }
}

#[cfg(unix)]
fn daemon_status_snapshot(
    base: &DaemonStatusPayload,
    health: &Arc<Mutex<DaemonHealth>>,
) -> DaemonStatusPayload {
    let snapshot = health
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    DaemonStatusPayload {
        pid: base.pid,
        started_at_epoch_ms: base.started_at_epoch_ms,
        socket_path: base.socket_path.clone(),
        healthy: snapshot.consecutive_failures < 3,
        total_requests: snapshot.total_requests,
        failed_requests: snapshot.failed_requests,
        rate_limited_requests: snapshot.rate_limited_requests,
        last_error: snapshot.last_error,
    }
}

#[cfg(unix)]
fn record_daemon_success(health: &Arc<Mutex<DaemonHealth>>) {
    let mut state = health
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.total_requests = state.total_requests.saturating_add(1);
    state.consecutive_failures = 0;
    state.last_error = None;
}

#[cfg(unix)]
fn record_daemon_failure(health: &Arc<Mutex<DaemonHealth>>, error: String) {
    let mut state = health
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.total_requests = state.total_requests.saturating_add(1);
    state.failed_requests = state.failed_requests.saturating_add(1);
    state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    state.last_error = Some(error);
}

#[cfg(unix)]
fn record_daemon_rate_limit(health: &Arc<Mutex<DaemonHealth>>) {
    let mut state = health
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.total_requests = state.total_requests.saturating_add(1);
    state.failed_requests = state.failed_requests.saturating_add(1);
    state.rate_limited_requests = state.rate_limited_requests.saturating_add(1);
    state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    state.last_error = Some("daemon rate limit exceeded; retry shortly".to_string());
}

#[cfg(unix)]
fn handle_daemon_connection(
    mut stream: UnixStream,
    status: &DaemonStatusPayload,
    health: &Arc<Mutex<DaemonHealth>>,
    should_shutdown: &mut bool,
    rate_limiter: &mut DaemonRateLimiter,
) -> Result<()> {
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .context("failed to set daemon read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(120)))
        .context("failed to set daemon write timeout")?;

    let request_line = {
        let mut reader = BufReader::new(&mut stream);
        read_line_limited(
            &mut reader,
            DAEMON_REQUEST_SIZE_LIMIT_BYTES,
            "daemon request",
        )?
    };
    if request_line.trim().is_empty() {
        return Ok(());
    }

    if !rate_limiter.allow() {
        record_daemon_rate_limit(health);
        let response = DaemonResponse::Error {
            message: "daemon rate limit exceeded; retry shortly".to_string(),
        };
        let payload = serde_json::to_vec(&response).context("failed to encode daemon response")?;
        stream
            .write_all(&payload)
            .context("failed to write daemon response")?;
        stream
            .write_all(b"\n")
            .context("failed to write daemon response newline")?;
        stream.flush().context("failed to flush daemon response")?;
        return Ok(());
    }

    let request: DaemonRequest = match serde_json::from_str(request_line.trim_end()) {
        Ok(request) => request,
        Err(err) => {
            let message = format!("failed to parse daemon request: {err}");
            record_daemon_failure(health, message.clone());
            let response = DaemonResponse::Error { message };
            let payload =
                serde_json::to_vec(&response).context("failed to encode daemon response")?;
            stream
                .write_all(&payload)
                .context("failed to write daemon response")?;
            stream
                .write_all(b"\n")
                .context("failed to write daemon response newline")?;
            stream.flush().context("failed to flush daemon response")?;
            return Ok(());
        }
    };

    let response = match request {
        DaemonRequest::Ping => {
            record_daemon_success(health);
            DaemonResponse::Pong(daemon_status_snapshot(status, health))
        }
        DaemonRequest::Shutdown => {
            record_daemon_success(health);
            *should_shutdown = true;
            DaemonResponse::Ack
        }
        DaemonRequest::Build(request) => match execute_daemon_build(request) {
            Ok(result) => {
                record_daemon_success(health);
                DaemonResponse::Build(result)
            }
            Err(err) => {
                let message = format!("{err:#}");
                record_daemon_failure(health, message.clone());
                DaemonResponse::Error { message }
            }
        },
        DaemonRequest::Metadata(request) => match execute_daemon_metadata(request) {
            Ok(result) => {
                record_daemon_success(health);
                DaemonResponse::Metadata(result)
            }
            Err(err) => {
                let message = format!("{err:#}");
                record_daemon_failure(health, message.clone());
                DaemonResponse::Error { message }
            }
        },
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
    if tool.to_ascii_lowercase() != "scarb" {
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

fn validate_scarb_toolchain() -> Result<()> {
    let version = scarb_version_line()?;
    let current = parse_scarb_semver(&version)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Mutex, OnceLock as TestOnceLock};
    use std::thread;

    fn unique_test_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).expect("failed to create test directory");
        dir
    }

    fn integration_env_lock() -> &'static Mutex<()> {
        static LOCK: TestOnceLock<Mutex<()>> = TestOnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn scarb_available() -> bool {
        Command::new("scarb")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn smoke_fixture_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../benchmarks/fixtures/scarb_smoke")
            .canonicalize()
            .expect("failed to locate scarb_smoke fixture")
    }

    fn copy_dir_recursive(src: &Path, dst: &Path) {
        for entry in walkdir::WalkDir::new(src) {
            let entry = entry.expect("failed to traverse fixture directory");
            let rel = entry
                .path()
                .strip_prefix(src)
                .expect("failed to strip fixture prefix");
            let out = dst.join(rel);
            if entry.file_type().is_dir() {
                fs::create_dir_all(&out).expect("failed to create fixture output directory");
            } else {
                if let Some(parent) = out.parent() {
                    fs::create_dir_all(parent).expect("failed to create fixture parent");
                }
                fs::copy(entry.path(), &out).expect("failed to copy fixture file");
            }
        }
    }

    fn prepare_smoke_workspace(prefix: &str) -> PathBuf {
        let dir = unique_test_dir(prefix);
        copy_dir_recursive(&smoke_fixture_dir(), &dir);
        dir
    }

    fn smoke_common_args(manifest_path: &Path) -> BuildCommonArgs {
        BuildCommonArgs {
            manifest_path: Some(manifest_path.to_path_buf()),
            package: None,
            workspace: false,
            features: Vec::new(),
            offline: false,
            release: false,
            profile: None,
        }
    }

    fn run_smoke_cached_build(
        common: &BuildCommonArgs,
        manifest_path: &Path,
        workspace_root: &Path,
        profile: &str,
        session_key: &str,
    ) -> Result<(CommandRun, bool, String, BuildPhaseTelemetry)> {
        run_build_with_uc_cache(
            common,
            manifest_path,
            workspace_root,
            profile,
            session_key,
            true,
            false,
        )
    }

    #[test]
    fn daemon_metadata_request_roundtrip_preserves_fields() {
        let args = MetadataArgs {
            manifest_path: Some(PathBuf::from("/tmp/workspace/Scarb.toml")),
            format_version: 3,
            daemon_mode: DaemonModeArg::Auto,
            offline: true,
            global_cache_dir: Some(PathBuf::from("/tmp/scarb-cache")),
            report_path: None,
        };
        let request =
            daemon_metadata_request_from_args(&args, Path::new("/tmp/workspace/Scarb.toml"), true);
        let restored = metadata_args_from_daemon_request(&request);

        assert_eq!(
            restored
                .manifest_path
                .as_ref()
                .expect("manifest path missing"),
            Path::new("/tmp/workspace/Scarb.toml")
        );
        assert_eq!(restored.format_version, 3);
        assert!(restored.offline);
        assert_eq!(
            restored.global_cache_dir,
            Some(PathBuf::from("/tmp/scarb-cache"))
        );
        assert_eq!(restored.daemon_mode as u8, DaemonModeArg::Off as u8);
        assert!(request.capture_output);
    }

    #[test]
    fn daemon_build_request_roundtrip_preserves_async_cache_persist() {
        let common = BuildCommonArgs {
            manifest_path: Some(PathBuf::from("/tmp/workspace/Scarb.toml")),
            package: Some("demo".to_string()),
            workspace: true,
            features: vec!["feature_a".to_string(), "feature_b".to_string()],
            offline: true,
            release: false,
            profile: Some("dev".to_string()),
        };
        let request =
            daemon_build_request_from_common(&common, Path::new("/tmp/workspace/Scarb.toml"), true);
        let restored = common_args_from_daemon_request(&request);

        assert!(request.async_cache_persist);
        assert_eq!(restored.package, common.package);
        assert_eq!(restored.workspace, common.workspace);
        assert_eq!(restored.features, common.features);
        assert_eq!(restored.offline, common.offline);
        assert_eq!(restored.release, common.release);
        assert_eq!(restored.profile, common.profile);
    }

    #[test]
    fn daemon_build_request_serialization_supports_async_cache_persist_wire_field() {
        let request = DaemonRequest::Build(DaemonBuildRequest {
            manifest_path: "/tmp/workspace/Scarb.toml".to_string(),
            package: None,
            workspace: false,
            features: vec!["feature_a".to_string()],
            offline: false,
            release: false,
            profile: None,
            async_cache_persist: true,
        });
        let json = serde_json::to_string(&request).expect("failed to encode request");
        assert!(json.contains("\"type\":\"build\""));
        assert!(json.contains("\"async_cache_persist\":true"));

        let decoded: DaemonRequest =
            serde_json::from_str(&json).expect("failed to decode daemon request");
        match decoded {
            DaemonRequest::Build(payload) => {
                assert!(payload.async_cache_persist);
                assert_eq!(payload.manifest_path, "/tmp/workspace/Scarb.toml");
                assert_eq!(payload.features, vec!["feature_a".to_string()]);
            }
            _ => panic!("expected build request"),
        }
    }

    #[test]
    fn daemon_metadata_request_serialization_supports_wire_format() {
        let request = DaemonRequest::Metadata(DaemonMetadataRequest {
            manifest_path: "/tmp/workspace/Scarb.toml".to_string(),
            format_version: 1,
            offline: false,
            global_cache_dir: None,
            capture_output: false,
        });
        let json = serde_json::to_string(&request).expect("failed to encode request");
        assert!(json.contains("\"type\":\"metadata\""));

        let decoded: DaemonRequest =
            serde_json::from_str(&json).expect("failed to decode daemon request");
        match decoded {
            DaemonRequest::Metadata(payload) => {
                assert_eq!(payload.manifest_path, "/tmp/workspace/Scarb.toml");
                assert_eq!(payload.format_version, 1);
                assert!(!payload.offline);
                assert!(!payload.capture_output);
                assert!(payload.global_cache_dir.is_none());
            }
            _ => panic!("expected metadata request"),
        }
    }

    #[test]
    fn write_uc_toml_normalizes_windows_manifest_path() {
        let dir = unique_test_dir("uc-write-uc-toml");
        let output = dir.join("Uc.toml");

        write_uc_toml(
            &output,
            Path::new(r"C:\Users\foo\project\Scarb.toml"),
            Some("demo"),
            Some("0.1.0"),
            Some("2024_07"),
        )
        .expect("failed to write Uc.toml");

        let body = fs::read_to_string(&output).expect("failed to read Uc.toml");
        assert!(body.contains(r#"scarb_manifest = "C:/Users/foo/project/Scarb.toml""#));
        assert!(!body.contains(r#"C:\Users\foo\project\Scarb.toml"#));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scarb_metadata_command_includes_daemon_independent_flags() {
        let args = MetadataArgs {
            manifest_path: Some(PathBuf::from("/tmp/workspace/Scarb.toml")),
            format_version: 2,
            daemon_mode: DaemonModeArg::Require,
            offline: true,
            global_cache_dir: Some(PathBuf::from("/tmp/scarb-cache")),
            report_path: None,
        };
        let (_, command_vec) =
            scarb_metadata_command(&args, Path::new("/tmp/workspace/Scarb.toml"));
        assert_eq!(
            command_vec,
            vec![
                "scarb",
                "--manifest-path",
                "/tmp/workspace/Scarb.toml",
                "--offline",
                "--global-cache-dir",
                "/tmp/scarb-cache",
                "metadata",
                "--format-version",
                "2",
            ]
        );
    }

    #[test]
    fn parse_scarb_semver_extracts_triplet() {
        assert_eq!(
            parse_scarb_semver("scarb 2.14.0 (682b29e13 2025-11-25)").unwrap(),
            (2, 14, 0)
        );
        assert!(parse_scarb_semver("invalid-output").is_err());
    }

    #[test]
    fn strip_cairo_comments_preserves_literals() {
        let source = br#"fn demo() {
    let url = "http://localhost";
    let marker = '//';
    // remove this comment
    /* and this block */
}
"#;
        let stripped = strip_cairo_comments(source);
        let text = String::from_utf8(stripped).expect("stripped output should be utf-8");
        assert!(text.contains("http://localhost"));
        assert!(text.contains("let marker = '//';"));
        assert!(!text.contains("remove this comment"));
        assert!(!text.contains("and this block"));
    }

    #[test]
    fn fingerprint_ignores_cairo_comment_only_edits() {
        let workspace = prepare_smoke_workspace("uc-fingerprint-comments");
        let manifest_path = workspace.join("Scarb.toml");
        let common = smoke_common_args(&manifest_path);
        let profile = effective_profile(&common);
        let lib_path = workspace.join("src/lib.cairo");

        let original = compute_build_fingerprint_with_scarb_version(
            &workspace,
            &manifest_path,
            &common,
            &profile,
            None,
            "scarb 2.14.0 (test)",
        )
        .expect("failed to compute baseline fingerprint");

        fs::write(
            &lib_path,
            format!(
                "{}\n// comment-only change for fingerprint test\n",
                fs::read_to_string(&lib_path).expect("failed to read lib.cairo")
            ),
        )
        .expect("failed to append comment");

        let with_comment = compute_build_fingerprint_with_scarb_version(
            &workspace,
            &manifest_path,
            &common,
            &profile,
            None,
            "scarb 2.14.0 (test)",
        )
        .expect("failed to compute comment fingerprint");
        assert_eq!(original, with_comment);

        let updated = fs::read_to_string(&lib_path).expect("failed to read updated lib.cairo");
        fs::write(
            &lib_path,
            updated.replace("math::weighted_sum(seed)", "math::weighted_sum(seed + 1)"),
        )
        .expect("failed to write semantic change");

        let with_semantic_change = compute_build_fingerprint_with_scarb_version(
            &workspace,
            &manifest_path,
            &common,
            &profile,
            None,
            "scarb 2.14.0 (test)",
        )
        .expect("failed to compute semantic-change fingerprint");
        assert_ne!(original, with_semantic_change);

        fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn async_persist_error_queue_retains_multiple_failures() {
        let _ = take_async_persist_errors();
        record_async_persist_error("err-a".to_string());
        record_async_persist_error("err-b".to_string());

        assert_eq!(
            take_async_persist_errors(),
            vec!["err-a".to_string(), "err-b".to_string()]
        );
    }

    #[test]
    fn async_persist_error_queue_drops_oldest_when_over_capacity() {
        let _ = take_async_persist_errors();
        let overflow = ASYNC_PERSIST_ERROR_QUEUE_LIMIT + 3;
        for i in 0..overflow {
            record_async_persist_error(format!("err-{i}"));
        }

        let drained = take_async_persist_errors();
        assert_eq!(drained.len(), ASYNC_PERSIST_ERROR_QUEUE_LIMIT);
        assert_eq!(drained.first().map(String::as_str), Some("err-3"));
        assert_eq!(
            drained.last().map(String::as_str),
            Some(format!("err-{}", overflow - 1).as_str())
        );
    }

    #[test]
    fn compute_build_fingerprint_changes_when_scarb_version_changes() {
        let _guard = integration_env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let workspace = prepare_smoke_workspace("uc-fingerprint-version");
        let manifest_path = workspace.join("Scarb.toml");
        let common = smoke_common_args(&manifest_path);
        let profile = effective_profile(&common);

        let v1 = compute_build_fingerprint_with_scarb_version(
            &workspace,
            &manifest_path,
            &common,
            &profile,
            None,
            "scarb 2.14.0 (test)",
        )
        .expect("failed to compute fingerprint for v1");
        let v2 = compute_build_fingerprint_with_scarb_version(
            &workspace,
            &manifest_path,
            &common,
            &profile,
            None,
            "scarb 2.15.0 (test)",
        )
        .expect("failed to compute fingerprint for v2");
        assert_ne!(v1, v2);

        fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn run_build_with_uc_cache_hits_after_initial_compile() {
        let _guard = integration_env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !scarb_available() {
            return;
        }
        let workspace = prepare_smoke_workspace("uc-cache-hit");
        let manifest_path = workspace.join("Scarb.toml");
        let common = smoke_common_args(&manifest_path);
        let profile = effective_profile(&common);
        let session_key = build_session_input(&common, &manifest_path, &profile)
            .expect("failed to compute session input")
            .deterministic_key_hex();

        let (first_run, first_hit, first_fingerprint, _) =
            run_smoke_cached_build(&common, &manifest_path, &workspace, &profile, &session_key)
                .expect("first build should succeed");
        assert_eq!(first_run.exit_code, 0);
        assert!(!first_hit);

        let (second_run, second_hit, second_fingerprint, _) =
            run_smoke_cached_build(&common, &manifest_path, &workspace, &profile, &session_key)
                .expect("second build should succeed");
        assert_eq!(second_run.exit_code, 0);
        assert!(second_hit);
        assert_eq!(first_fingerprint, second_fingerprint);

        fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn run_build_with_uc_cache_recovers_from_corrupted_entry() {
        let _guard = integration_env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !scarb_available() {
            return;
        }
        let workspace = prepare_smoke_workspace("uc-cache-corruption");
        let manifest_path = workspace.join("Scarb.toml");
        let common = smoke_common_args(&manifest_path);
        let profile = effective_profile(&common);
        let session_key = build_session_input(&common, &manifest_path, &profile)
            .expect("failed to compute session input")
            .deterministic_key_hex();

        run_smoke_cached_build(&common, &manifest_path, &workspace, &profile, &session_key)
            .expect("initial build should succeed");

        let entry_path = workspace
            .join(".uc/cache/build")
            .join(format!("{session_key}.json"));
        fs::write(&entry_path, b"{not-json").expect("failed to corrupt cache entry");

        let (run, cache_hit, _, _) =
            run_smoke_cached_build(&common, &manifest_path, &workspace, &profile, &session_key)
                .expect("build should recover from corrupted cache entry");
        assert_eq!(run.exit_code, 0);
        assert!(!cache_hit);

        fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn run_build_with_uc_cache_allows_concurrent_builds() {
        let _guard = integration_env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !scarb_available() {
            return;
        }
        let workspace = prepare_smoke_workspace("uc-cache-concurrency");
        let manifest_path = workspace.join("Scarb.toml");
        let common = smoke_common_args(&manifest_path);
        let profile = effective_profile(&common);
        let session_key = build_session_input(&common, &manifest_path, &profile)
            .expect("failed to compute session input")
            .deterministic_key_hex();

        let workspace_a = workspace.clone();
        let manifest_a = manifest_path.clone();
        let profile_a = profile.clone();
        let session_a = session_key.clone();
        let common_a = common.clone();
        let worker_a = thread::spawn(move || {
            run_smoke_cached_build(&common_a, &manifest_a, &workspace_a, &profile_a, &session_a)
        });

        let workspace_b = workspace.clone();
        let manifest_b = manifest_path.clone();
        let profile_b = profile.clone();
        let session_b = session_key.clone();
        let common_b = common.clone();
        let worker_b = thread::spawn(move || {
            run_smoke_cached_build(&common_b, &manifest_b, &workspace_b, &profile_b, &session_b)
        });

        let (run_a, _, _, _) = worker_a
            .join()
            .expect("worker A panicked")
            .expect("worker A build failed");
        let (run_b, _, _, _) = worker_b
            .join()
            .expect("worker B panicked")
            .expect("worker B build failed");
        assert_eq!(run_a.exit_code, 0);
        assert_eq!(run_b.exit_code, 0);

        fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn build_session_cfg_set_changes_when_cairo_target_or_tool_changes() {
        let dir = unique_test_dir("uc-session-cfg");
        let manifest_path = dir.join("Scarb.toml");

        fs::write(
            &manifest_path,
            r#"[package]
name = "cfg_test"
version = "0.1.0"
edition = "2024_07"

[cairo]
allow-warnings = true

[target.starknet-contract]
sierra = true

[tool.uc]
mode = "fast"
"#,
        )
        .expect("failed to write manifest");

        let cfg_a = build_session_cfg_set(&manifest_path).expect("failed to compute cfg A");

        fs::write(
            &manifest_path,
            r#"[package]
name = "cfg_test"
version = "0.1.0"
edition = "2024_07"

[cairo]
allow-warnings = false

[target.starknet-contract]
sierra = true

[tool.uc]
mode = "safe"
"#,
        )
        .expect("failed to rewrite manifest");

        let cfg_b = build_session_cfg_set(&manifest_path).expect("failed to compute cfg B");
        assert_ne!(cfg_a, cfg_b);

        fs::remove_dir_all(&dir).ok();
    }
}
