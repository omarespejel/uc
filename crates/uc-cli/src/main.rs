use anyhow::{bail, Context, Result};
use blake3::Hasher;
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use toml::Value as TomlValue;
use uc_core::artifacts::{
    collect_artifact_digests, compare_artifact_sets, ArtifactDigest, ArtifactMismatch,
};
use uc_core::compare::{compare_diagnostics, extract_diagnostic_lines, DiagnosticsComparison};
use uc_core::session::SessionInput;
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(name = "uc")]
#[command(about = "uc: Cairo package manager and build/prove engine", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Benchmark(BenchmarkArgs),
    SessionKey(SessionKeyArgs),
    Build(BuildArgs),
    Metadata(MetadataArgs),
    CompareBuild(CompareBuildArgs),
    Migrate(MigrateArgs),
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

    #[arg(long, default_value = "/Users/espejelomar/StarkNet/compiler-starknet")]
    workspace_root: String,
}

#[derive(Args, Debug)]
struct SessionKeyArgs {
    #[arg(long)]
    workspace_root: String,

    #[arg(long)]
    compiler_version: String,

    #[arg(long)]
    profile: String,

    #[arg(long, value_delimiter = ',')]
    features: Vec<String>,

    #[arg(long = "cfg", value_delimiter = ',')]
    cfg_set: Vec<String>,

    #[arg(long)]
    plugin_signature: String,

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

#[derive(Debug)]
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

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Benchmark(args) => run_benchmark(args),
        Commands::SessionKey(args) => run_session_key(args),
        Commands::Build(args) => run_build(args),
        Commands::Metadata(args) => run_metadata(args),
        Commands::CompareBuild(args) => run_compare_build(args),
        Commands::Migrate(args) => run_migrate(args),
    }
}

fn run_benchmark(args: BenchmarkArgs) -> Result<()> {
    let root = workspace_root().context("failed to resolve workspace root")?;
    let script = root.join("benchmarks/scripts/run_local_benchmarks.sh");

    if !script.exists() {
        bail!("benchmark script not found at {}", script.display());
    }

    let status = Command::new(&script)
        .arg("--matrix")
        .arg(args.matrix.as_str())
        .arg("--tool")
        .arg(args.tool.as_str())
        .arg("--runs")
        .arg(args.runs.to_string())
        .arg("--cold-runs")
        .arg(args.cold_runs.to_string())
        .arg("--workspace-root")
        .arg(args.workspace_root)
        .status()
        .context("failed to execute benchmark script")?;

    if !status.success() {
        bail!("benchmark script exited with status {status}");
    }

    Ok(())
}

fn run_session_key(args: SessionKeyArgs) -> Result<()> {
    let input = SessionInput {
        workspace_root: args.workspace_root,
        compiler_version: args.compiler_version,
        profile: args.profile,
        features: args.features,
        cfg_set: args.cfg_set,
        plugin_signature: args.plugin_signature,
        target_family: args.target_family,
    };

    println!("{}", input.deterministic_key_hex());
    Ok(())
}

fn run_build(args: BuildArgs) -> Result<()> {
    let common = args.common;
    let engine = args.engine;
    let manifest_path = resolve_manifest_path(&common.manifest_path)?;
    let workspace_root = manifest_path
        .parent()
        .context("manifest path has no parent")?
        .to_path_buf();
    let profile = effective_profile(&common);

    let scarb_version = scarb_version_line()?;
    let session_input = SessionInput {
        workspace_root: workspace_root.display().to_string(),
        compiler_version: scarb_version,
        profile: profile.clone(),
        features: common.features.clone(),
        cfg_set: Vec::new(),
        plugin_signature: "unknown".to_string(),
        target_family: if common.workspace {
            "workspace".to_string()
        } else {
            "package".to_string()
        },
    };

    let session_key = session_input.deterministic_key_hex();
    let (run, cache_hit, fingerprint) = match engine {
        EngineArg::Scarb => {
            let (command, command_vec) = scarb_build_command(&common, &manifest_path);
            let run = run_command_capture(command, command_vec)?;
            let fingerprint =
                compute_build_fingerprint(&workspace_root, &manifest_path, &common, &profile)?;
            (run, false, fingerprint)
        }
        EngineArg::Uc => run_build_with_uc_cache(
            &common,
            &manifest_path,
            &workspace_root,
            &profile,
            &session_key,
        )?,
    };
    replay_output(&run.stdout, &run.stderr)?;

    let artifacts = collect_profile_artifacts(&workspace_root, &profile)?;

    if let Some(path) = args.report_path {
        let report = BuildReport {
            generated_at_epoch_ms: epoch_ms()?,
            engine: engine.as_str().to_string(),
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

fn run_metadata(args: MetadataArgs) -> Result<()> {
    let manifest_path = resolve_manifest_path(&args.manifest_path)?;
    let (command, command_vec) = scarb_metadata_command(&args, &manifest_path);

    let run = run_command_capture(command, command_vec)?;
    replay_output(&run.stdout, &run.stderr)?;

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
    let raw = fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
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

    let report = CompareBuildReport {
        generated_at_epoch_ms: epoch_ms()?,
        manifest_path: manifest_path.display().to_string(),
        workspace_root: workspace_root.display().to_string(),
        clean_before_each: args.clean_before_each,
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
            && compare_artifact_sets(&baseline_artifacts, &candidate_artifacts).is_empty(),
    };

    let output_path = args.output_path.unwrap_or_else(|| {
        default_compare_output_path().unwrap_or_else(|_| PathBuf::from("compare-build-report.json"))
    });

    write_json_report(&output_path, &report)?;

    println!("Compare report: {}", output_path.display());
    println!(
        "Artifact mismatches: {} | Diagnostics similarity: {:.2}%",
        report.artifact_mismatch_count, report.diagnostics.similarity_percent
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
) -> Result<(CommandRun, bool, String)> {
    let fingerprint = compute_build_fingerprint(workspace_root, manifest_path, common, profile)?;
    let cache_root = workspace_root.join(".uc/cache");
    let objects_dir = cache_root.join("objects");
    let entry_path = cache_root.join("build").join(format!("{session_key}.json"));

    let restore_start = Instant::now();
    if let Some(entry) = load_cache_entry(&entry_path)? {
        if entry.schema_version == 1
            && entry.profile == profile
            && entry.fingerprint == fingerprint
            && restore_cached_artifacts(workspace_root, profile, &objects_dir, &entry.artifacts)?
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

    let (command, command_vec) = scarb_build_command(common, manifest_path);
    let run = run_command_capture(command, command_vec)?;

    if run.exit_code == 0 {
        let artifacts = collect_profile_artifacts(workspace_root, profile)?;
        persist_cache_entry(
            workspace_root,
            profile,
            &fingerprint,
            &artifacts,
            &objects_dir,
            &entry_path,
        )?;
    }

    Ok((run, false, fingerprint))
}

fn persist_cache_entry(
    workspace_root: &Path,
    profile: &str,
    fingerprint: &str,
    artifacts: &[ArtifactDigest],
    objects_dir: &Path,
    entry_path: &Path,
) -> Result<()> {
    let target_root = workspace_root.join("target").join(profile);
    let mut cached_artifacts = Vec::with_capacity(artifacts.len());

    for artifact in artifacts {
        let src = target_root.join(&artifact.relative_path);
        if !src.exists() {
            continue;
        }

        let object_rel_path = format!(
            "{}/{}.bin",
            &artifact.blake3_hex[0..2.min(artifact.blake3_hex.len())],
            artifact.blake3_hex
        );
        let object_path = objects_dir.join(&object_rel_path);
        if !object_path.exists() {
            if let Some(parent) = object_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::copy(&src, &object_path).with_context(|| {
                format!(
                    "failed to copy artifact {} to {}",
                    src.display(),
                    object_path.display()
                )
            })?;
        }

        cached_artifacts.push(CachedArtifact {
            relative_path: artifact.relative_path.clone(),
            blake3_hex: artifact.blake3_hex.clone(),
            size_bytes: artifact.size_bytes,
            object_rel_path,
        });
    }

    let entry = BuildCacheEntry {
        schema_version: 1,
        fingerprint: fingerprint.to_string(),
        profile: profile.to_string(),
        artifacts: cached_artifacts,
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

    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let parsed: BuildCacheEntry = match serde_json::from_slice(&bytes) {
        Ok(entry) => entry,
        Err(_) => return Ok(None),
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
        let object_path = objects_dir.join(&artifact.object_rel_path);
        if !object_path.exists() {
            return Ok(false);
        }
    }

    let target_root = workspace_root.join("target").join(profile);
    for artifact in artifacts {
        let expected_hash = &artifact.blake3_hex;
        let out_path = target_root.join(&artifact.relative_path);

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
    let restored = collect_artifact_digests(&target_root)?;
    Ok(compare_artifact_sets(&expected, &restored).is_empty())
}

fn hash_file_blake3(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut hasher = Hasher::new();
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

fn compute_build_fingerprint(
    workspace_root: &Path,
    manifest_path: &Path,
    common: &BuildCommonArgs,
    profile: &str,
) -> Result<String> {
    let mut hasher = Hasher::new();
    hasher.update(b"uc-build-fingerprint-v1");
    hasher.update(manifest_path.display().to_string().as_bytes());
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
    features.sort();
    for feature in features {
        hasher.update(feature.as_bytes());
        hasher.update(b",");
    }

    let mut files = Vec::new();
    let walker = WalkDir::new(workspace_root)
        .into_iter()
        .filter_entry(|entry| !is_ignored_entry(workspace_root, entry.path()));

    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if should_include_fingerprint_file(path) {
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
        let file_hash = hash_file_blake3(&path)?;
        hasher.update(rel.as_bytes());
        hasher.update(b":");
        hasher.update(file_hash.as_bytes());
        hasher.update(b"\n");
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
        exit_code: output.status.code().unwrap_or(-1),
        elapsed_ms,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn collect_profile_artifacts(workspace_root: &Path, profile: &str) -> Result<Vec<ArtifactDigest>> {
    let target_dir = workspace_root.join("target").join(profile);
    collect_artifact_digests(&target_dir)
}

fn replay_output(stdout: &str, stderr: &str) -> Result<()> {
    io::stdout().write_all(stdout.as_bytes())?;
    io::stderr().write_all(stderr.as_bytes())?;
    Ok(())
}

fn remove_build_outputs(workspace_root: &Path) -> Result<()> {
    let target = workspace_root.join("target");
    let scarb_dir = workspace_root.join(".scarb");

    if target.exists() {
        fs::remove_dir_all(&target)
            .with_context(|| format!("failed to remove {}", target.display()))?;
    }

    if scarb_dir.exists() {
        fs::remove_dir_all(&scarb_dir)
            .with_context(|| format!("failed to remove {}", scarb_dir.display()))?;
    }

    Ok(())
}

fn resolve_manifest_path(manifest_path: &Option<PathBuf>) -> Result<PathBuf> {
    let path = manifest_path
        .as_ref()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("Scarb.toml"));

    if path.is_absolute() {
        return Ok(path);
    }

    Ok(std::env::current_dir()?.join(path))
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

fn workspace_root() -> Result<PathBuf> {
    let dir = std::env::current_dir()?;
    Ok(dir)
}
