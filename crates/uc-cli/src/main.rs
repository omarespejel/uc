use anyhow::{bail, Context, Result};
use blake3::Hasher;
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
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
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use toml::Value as TomlValue;
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
const MAX_CACHEABLE_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_CACHE_ENTRY_BYTES: u64 = 10 * 1024 * 1024;
const MAX_FINGERPRINT_INDEX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ARTIFACT_INDEX_BYTES: u64 = 32 * 1024 * 1024;
const FINGERPRINT_INDEX_SCHEMA_VERSION: u32 = 1;
const ARTIFACT_INDEX_SCHEMA_VERSION: u32 = 1;
const DEFAULT_DIAGNOSTICS_SIMILARITY_THRESHOLD: f64 = 99.5;
const DAEMON_REQUEST_SIZE_LIMIT_BYTES: usize = 1024 * 1024;
const DAEMON_RATE_WINDOW_SECONDS: u64 = 1;
const DAEMON_MAX_REQUESTS_PER_WINDOW: usize = 32;
const DAEMON_LOG_ROTATE_BYTES: u64 = 10 * 1024 * 1024;
const CACHE_LOCK_STALE_AFTER_SECONDS: u64 = 300;
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
    Benchmark(BenchmarkArgs),
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
    Stop(DaemonSocketArgs),
    #[command(hide = true)]
    Serve(DaemonSocketArgs),
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonBuildResponse {
    run: CommandRun,
    cache_hit: bool,
    fingerprint: String,
    session_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DaemonRequest {
    Ping,
    Shutdown,
    Build(DaemonBuildRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DaemonResponse {
    Pong(DaemonStatusPayload),
    Ack,
    Build(DaemonBuildResponse),
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
        let _ = fs::remove_file(&self.path);
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Daemon(args) => run_daemon(args),
        Commands::Benchmark(args) => run_benchmark(args),
        Commands::SessionKey(args) => run_session_key(args),
        Commands::Build(args) => run_build(args),
        Commands::Metadata(args) => run_metadata(args),
        Commands::CompareBuild(args) => run_compare_build(args),
        Commands::Migrate(args) => run_migrate(args),
    }
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
        DaemonCommand::Stop(socket) => run_daemon_stop(socket),
        DaemonCommand::Serve(socket) => run_daemon_serve(socket),
    }
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
            "uc daemon running (pid={}, started_at_epoch_ms={}, socket={})",
            status.pid, status.started_at_epoch_ms, status.socket_path
        );
        Ok(())
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
        };
        let daemon_root = std::env::current_dir()
            .context("failed to resolve daemon current directory")?
            .canonicalize()
            .context("failed to canonicalize daemon root")?;
        let mut rate_limiter = DaemonRateLimiter::new();

        let mut should_shutdown = false;
        for incoming in listener.incoming() {
            match incoming {
                Ok(stream) => {
                    if let Err(err) = handle_daemon_connection(
                        stream,
                        &status,
                        &mut should_shutdown,
                        &mut rate_limiter,
                        &daemon_root,
                    ) {
                        eprintln!("uc daemon: request handling failed: {err:#}");
                    }
                }
                Err(err) => {
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
    let root = workspace_root().context("failed to resolve workspace root")?;
    let script = root.join("benchmarks/scripts/run_local_benchmarks.sh");

    if !script.exists() {
        bail!("benchmark script not found at {}", script.display());
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
        features: args.features,
        cfg_set: args.cfg_set,
        manifest_content_hash: args.manifest_content_hash,
        target_family: args.target_family,
    };

    println!("{}", input.deterministic_key_hex());
    Ok(())
}

fn run_build(args: BuildArgs) -> Result<()> {
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

    let session_input = build_session_input(&common, &manifest_path, &profile)?;
    let mut session_key = session_input.deterministic_key_hex();
    let mut daemon_used = false;
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
        EngineArg::Uc => match daemon_mode {
            DaemonModeArg::Off => run_build_with_uc_cache(
                &common,
                &manifest_path,
                &workspace_root,
                &profile,
                &session_key,
                false,
            )?,
            DaemonModeArg::Auto => {
                if let Some(response) = try_uc_build_via_daemon(&common, &manifest_path)? {
                    daemon_used = true;
                    session_key = response.session_key;
                    (response.run, response.cache_hit, response.fingerprint)
                } else {
                    run_build_with_uc_cache(
                        &common,
                        &manifest_path,
                        &workspace_root,
                        &profile,
                        &session_key,
                        false,
                    )?
                }
            }
            DaemonModeArg::Require => {
                let response = try_uc_build_via_daemon(&common, &manifest_path)?
                    .context("daemon mode is require but daemon is unavailable")?;
                daemon_used = true;
                session_key = response.session_key;
                (response.run, response.cache_hit, response.fingerprint)
            }
        },
    };
    replay_output(&run.stdout, &run.stderr)?;

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

        let request = DaemonRequest::Build(daemon_build_request_from_common(common, manifest_path));
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

fn daemon_build_request_from_common(
    common: &BuildCommonArgs,
    manifest_path: &Path,
) -> DaemonBuildRequest {
    DaemonBuildRequest {
        manifest_path: manifest_path.display().to_string(),
        package: common.package.clone(),
        workspace: common.workspace,
        features: common.features.clone(),
        offline: common.offline,
        release: common.release,
        profile: common.profile.clone(),
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

fn execute_daemon_build(
    request: DaemonBuildRequest,
    daemon_root: &Path,
) -> Result<DaemonBuildResponse> {
    let common = common_args_from_daemon_request(&request);
    let manifest_path = resolve_manifest_path(&common.manifest_path)?;
    if !manifest_path.starts_with(daemon_root) {
        bail!(
            "daemon denied manifest outside allowed root: {} not under {}",
            manifest_path.display(),
            daemon_root.display()
        );
    }
    let workspace_root = manifest_path
        .parent()
        .context("manifest path has no parent")?
        .to_path_buf();
    let profile = effective_profile(&common);
    let session_input = build_session_input(&common, &manifest_path, &profile)?;
    let session_key = session_input.deterministic_key_hex();

    let (run, cache_hit, fingerprint) = run_build_with_uc_cache(
        &common,
        &manifest_path,
        &workspace_root,
        &profile,
        &session_key,
        true,
    )?;

    Ok(DaemonBuildResponse {
        run,
        cache_hit,
        fingerprint,
        session_key,
    })
}

fn run_metadata(args: MetadataArgs) -> Result<()> {
    let manifest_path = resolve_manifest_path(&args.manifest_path)?;
    let (command, command_vec) = scarb_metadata_command(&args, &manifest_path);
    let write_report = args.report_path.is_some();

    let run = run_command(command, command_vec, write_report)?;
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
) -> Result<(CommandRun, bool, String)> {
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
    let fingerprint = compute_build_fingerprint(
        &canonical_workspace_root,
        manifest_path,
        common,
        profile,
        Some(&cache_root),
    )?;

    let restore_start = Instant::now();
    {
        let _cache_lock = acquire_cache_lock(&cache_root)?;
        if let Some(entry) = load_cache_entry(&entry_path)? {
            if entry.schema_version == BUILD_CACHE_SCHEMA_VERSION
                && entry.profile == profile
                && entry.fingerprint == fingerprint
                && restore_cached_artifacts(
                    &canonical_workspace_root,
                    profile,
                    &objects_dir,
                    &entry.artifacts,
                )?
            {
                let run = CommandRun {
                    command: vec![
                        "uc".to_string(),
                        "build".to_string(),
                        "--engine".to_string(),
                        "uc".to_string(),
                        "--cache-hit".to_string(),
                    ],
                    exit_code: 0,
                    elapsed_ms: restore_start.elapsed().as_secs_f64() * 1000.0,
                    stdout: format!(
                        "uc: cache hit, restored {} artifacts\n",
                        entry.artifacts.len()
                    ),
                    stderr: String::new(),
                };
                return Ok((run, true, fingerprint));
            }
        }
    }

    let (command, command_vec) = scarb_build_command(common, manifest_path);
    let run = run_command(command, command_vec, capture_output)?;

    if run.exit_code == 0 {
        let cached_artifacts = collect_cached_artifacts_for_entry(
            &canonical_workspace_root,
            profile,
            &cache_root,
            &objects_dir,
        )?;
        let _cache_lock = acquire_cache_lock(&cache_root)?;
        persist_cache_entry(profile, &fingerprint, &cached_artifacts, &entry_path)?;
    }

    Ok((run, false, fingerprint))
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

    if let Some(parent) = entry_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let bytes = serde_json::to_vec_pretty(&entry)?;
    fs::write(entry_path, bytes)
        .with_context(|| format!("failed to write cache entry {}", entry_path.display()))?;

    Ok(())
}

fn load_cache_entry(path: &Path) -> Result<Option<BuildCacheEntry>> {
    if !path.exists() {
        return Ok(None);
    }

    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    if metadata.len() > MAX_CACHE_ENTRY_BYTES {
        eprintln!(
            "uc: warning: ignoring oversized cache entry {} ({} bytes > {} bytes)",
            path.display(),
            metadata.len(),
            MAX_CACHE_ENTRY_BYTES
        );
        return Ok(None);
    }
    let file = File::open(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = BufReader::new(file).take(MAX_CACHE_ENTRY_BYTES + 1);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.len() as u64 > MAX_CACHE_ENTRY_BYTES {
        eprintln!(
            "uc: warning: ignoring oversized cache entry {} (>{} bytes)",
            path.display(),
            MAX_CACHE_ENTRY_BYTES
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

fn load_fingerprint_index(path: &Path) -> Result<FingerprintIndex> {
    if !path.exists() {
        return Ok(FingerprintIndex::empty());
    }
    let bytes = read_bytes_with_limit(path, MAX_FINGERPRINT_INDEX_BYTES, "fingerprint index")?;
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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec(index).context("failed to encode fingerprint index")?;
    let temp_path = path.with_extension("tmp");
    fs::write(&temp_path, &bytes)
        .with_context(|| format!("failed to write {}", temp_path.display()))?;
    fs::rename(&temp_path, path).with_context(|| {
        format!(
            "failed to move fingerprint index {} to {}",
            temp_path.display(),
            path.display()
        )
    })?;
    Ok(())
}

fn load_artifact_index(path: &Path) -> Result<ArtifactIndex> {
    if !path.exists() {
        return Ok(ArtifactIndex::empty());
    }
    let bytes = read_bytes_with_limit(path, MAX_ARTIFACT_INDEX_BYTES, "artifact index")?;
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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec(index).context("failed to encode artifact index")?;
    let temp_path = path.with_extension("tmp");
    fs::write(&temp_path, &bytes)
        .with_context(|| format!("failed to write {}", temp_path.display()))?;
    fs::rename(&temp_path, path).with_context(|| {
        format!(
            "failed to move artifact index {} to {}",
            temp_path.display(),
            path.display()
        )
    })?;
    Ok(())
}

fn compute_build_fingerprint(
    workspace_root: &Path,
    manifest_path: &Path,
    common: &BuildCommonArgs,
    profile: &str,
    cache_root: Option<&Path>,
) -> Result<String> {
    let mut hasher = Hasher::new();
    hasher.update(b"uc-build-fingerprint-v1");
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
    let mut updated_entries: BTreeMap<String, FingerprintIndexEntry> = BTreeMap::new();

    let mut files = Vec::new();
    let walker = WalkDir::new(workspace_root)
        .max_depth(MAX_FINGERPRINT_DEPTH)
        .into_iter()
        .filter_entry(|entry| !is_ignored_entry(workspace_root, entry.path()));

    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if should_include_fingerprint_file(path) {
            if files.len() >= MAX_FINGERPRINT_FILES {
                bail!(
                    "workspace has too many fingerprintable files (>{MAX_FINGERPRINT_FILES}); refusing to hash more"
                );
            }
            files.push(path.to_path_buf());
        }
    }
    files.sort();

    for path in files {
        let rel = path
            .strip_prefix(workspace_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let metadata =
            fs::metadata(&path).with_context(|| format!("failed to stat {}", path.display()))?;
        let file_size = metadata.len();
        if file_size > MAX_FINGERPRINT_FILE_BYTES {
            bail!(
                "fingerprint file {} exceeds size limit ({} bytes > {} bytes)",
                path.display(),
                file_size,
                MAX_FINGERPRINT_FILE_BYTES
            );
        }
        let modified_unix_ms = metadata_modified_unix_ms(&metadata)?;
        let file_hash = if let Some(cached) = index.entries.get(&rel) {
            if cached.size_bytes == file_size && cached.modified_unix_ms == modified_unix_ms {
                cached.blake3_hex.clone()
            } else {
                hash_file_blake3(&path)?
            }
        } else {
            hash_file_blake3(&path)?
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
    let start = Instant::now();
    let output = command.output().context("failed to run command")?;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

    Ok(CommandRun {
        command: command_vec,
        exit_code: exit_code_from_status(&output.status),
        elapsed_ms,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
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
            if cached.size_bytes == metadata.len() && cached.modified_unix_ms == modified_unix_ms {
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
            fs::copy(path, &object_path).with_context(|| {
                format!(
                    "failed to copy artifact {} to {}",
                    path.display(),
                    object_path.display()
                )
            })?;
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
    if let Err(err) = save_artifact_index(&index_path, &index) {
        eprintln!(
            "uc: warning: failed to update artifact index {}: {err:#}",
            index_path.display()
        );
    }
    Ok(cached_artifacts)
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
    let output = Command::new("scarb")
        .arg("--version")
        .output()
        .context("failed to execute `scarb --version`")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let first = stdout.lines().next().unwrap_or("scarb unknown").trim();
    Ok(first.to_string())
}

fn build_session_input(
    common: &BuildCommonArgs,
    manifest_path: &Path,
    profile: &str,
) -> Result<SessionInput> {
    let scarb_version = scarb_version_line()?;
    let manifest_content_hash = compute_manifest_content_hash(manifest_path)?;
    Ok(SessionInput {
        compiler_version: scarb_version,
        profile: profile.to_string(),
        features: common.features.clone(),
        cfg_set: Vec::new(),
        manifest_content_hash,
        target_family: if common.workspace {
            "workspace".to_string()
        } else {
            "package".to_string()
        },
    })
}

fn exit_code_from_status(status: &ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        if let Some(signal) = status.signal() {
            return -signal;
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
fn is_process_alive(_pid: u32) -> bool {
    true
}

fn daemon_socket_path(override_path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        return Ok(path);
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
fn handle_daemon_connection(
    mut stream: UnixStream,
    status: &DaemonStatusPayload,
    should_shutdown: &mut bool,
    rate_limiter: &mut DaemonRateLimiter,
    daemon_root: &Path,
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

    let request: DaemonRequest =
        serde_json::from_str(request_line.trim_end()).context("failed to parse daemon request")?;
    let response = match request {
        DaemonRequest::Ping => DaemonResponse::Pong(status.clone()),
        DaemonRequest::Shutdown => {
            *should_shutdown = true;
            DaemonResponse::Ack
        }
        DaemonRequest::Build(request) => match execute_daemon_build(request, daemon_root) {
            Ok(result) => DaemonResponse::Build(result),
            Err(err) => DaemonResponse::Error {
                message: format!("{err:#}"),
            },
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

    let body = format!(
        "[project]\nname = \"{name}\"\nversion = \"{version}\"\nedition = \"{edition}\"\n\n[source]\nscarb_manifest = \"{}\"\n",
        source_manifest.display()
    );

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
    let cwd = std::env::current_dir()?.canonicalize()?;
    for candidate in cwd.ancestors() {
        let root = candidate.to_path_buf();
        let benchmarks_script = root.join("benchmarks/scripts/run_local_benchmarks.sh");
        let cargo_manifest = root.join("Cargo.toml");
        if benchmarks_script.is_file() && cargo_manifest.is_file() {
            return Ok(root);
        }
    }
    bail!(
        "failed to locate uc workspace root from {}; expected Cargo.toml and benchmarks/scripts/run_local_benchmarks.sh in an ancestor directory",
        cwd.display()
    )
}
