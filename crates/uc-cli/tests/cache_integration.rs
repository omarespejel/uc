use serde::Deserialize;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

#[derive(Debug, Deserialize)]
struct BuildReport {
    cache_hit: bool,
    fingerprint: String,
    exit_code: i32,
}

struct TestWorkspace {
    root: PathBuf,
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn serial_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn fixture_source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/fixtures/scarb_smoke")
        .canonicalize()
        .expect("failed to resolve scarb_smoke fixture path")
}

fn make_test_workspace(name: &str) -> TestWorkspace {
    let source = fixture_source();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "uc-cli-integration-{name}-{}-{nonce}",
        std::process::id()
    ));
    copy_dir_recursive(&source, &root).expect("failed to create test workspace");
    let _ = fs::remove_dir_all(root.join(".uc"));
    let _ = fs::remove_dir_all(root.join("target"));
    let _ = fs::remove_dir_all(root.join(".scarb"));
    TestWorkspace { root }
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> std::io::Result<()> {
    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(source)
            .expect("invalid fixture relative path");
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(path, &target)?;
        }
    }
    Ok(())
}

fn uc_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_uc") {
        return PathBuf::from(path);
    }
    let fallback = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/uc");
    assert!(
        fallback.exists(),
        "missing uc test binary; expected {}",
        fallback.display()
    );
    fallback
}

fn scarb_available() -> bool {
    Command::new("scarb")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn run_uc_build(workspace: &TestWorkspace, report_tag: &str) -> (Output, BuildReport) {
    let manifest = workspace.root.join("Scarb.toml");
    let report_path = workspace
        .root
        .join(".uc")
        .join(format!("report-{report_tag}.json"));
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent).expect("failed to create report directory");
    }
    let output = Command::new(uc_bin())
        .current_dir(&workspace.root)
        .arg("build")
        .arg("--engine")
        .arg("uc")
        .arg("--daemon-mode")
        .arg("off")
        .arg("--offline")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--report-path")
        .arg(&report_path)
        .output()
        .expect("failed to execute uc build");
    let report_bytes = fs::read(&report_path).expect("missing build report");
    let report: BuildReport =
        serde_json::from_slice(&report_bytes).expect("failed to decode build report JSON");
    (output, report)
}

fn output_to_utf8(output: &Output) -> String {
    let mut message = String::new();
    message.push_str(&String::from_utf8_lossy(&output.stdout));
    message.push_str(&String::from_utf8_lossy(&output.stderr));
    message
}

fn assert_success(output: &Output, context: &str) {
    if output.status.success() {
        return;
    }
    panic!(
        "{context} failed (status: {:?})\n{}",
        output.status.code(),
        output_to_utf8(output)
    );
}

fn cache_entry_path(workspace: &TestWorkspace) -> PathBuf {
    let build_cache = workspace.root.join(".uc/cache/build");
    let mut entries: Vec<PathBuf> = fs::read_dir(&build_cache)
        .expect("cache build directory missing")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension() == Some(OsStr::new("json")))
        .collect();
    entries.sort();
    entries
        .into_iter()
        .next()
        .expect("expected at least one cache entry JSON file")
}

#[test]
fn integration_cache_hit_after_initial_build() {
    let _guard = serial_guard();
    if !scarb_available() {
        eprintln!("skipping integration_cache_hit_after_initial_build: scarb not available");
        return;
    }
    let workspace = make_test_workspace("cache-hit");

    let (first_output, first_report) = run_uc_build(&workspace, "first");
    assert_success(&first_output, "first build");
    assert_eq!(first_report.exit_code, 0);
    assert!(
        !first_report.cache_hit,
        "first build should be a cache miss"
    );

    let (second_output, second_report) = run_uc_build(&workspace, "second");
    assert_success(&second_output, "second build");
    assert_eq!(second_report.exit_code, 0);
    assert!(second_report.cache_hit, "second build should hit cache");
    assert_eq!(first_report.fingerprint, second_report.fingerprint);
}

#[test]
fn integration_semantic_edit_invalidates_then_recovers_to_hit() {
    let _guard = serial_guard();
    if !scarb_available() {
        eprintln!(
            "skipping integration_semantic_edit_invalidates_then_recovers_to_hit: scarb not available"
        );
        return;
    }
    let workspace = make_test_workspace("semantic-edit");

    let (baseline_output, baseline_report) = run_uc_build(&workspace, "baseline");
    assert_success(&baseline_output, "baseline build");
    assert!(
        !baseline_report.cache_hit,
        "baseline build should miss cache"
    );

    let lib_file = workspace.root.join("src/lib.cairo");
    let baseline = fs::read_to_string(&lib_file).expect("failed to read lib.cairo");
    let edited = baseline.replacen(
        "const BENCH_EDIT_SEED_BIAS: felt252 = 0;",
        "const BENCH_EDIT_SEED_BIAS: felt252 = 11;",
        1,
    );
    assert_ne!(
        baseline, edited,
        "semantic edit marker should exist in smoke fixture"
    );
    fs::write(&lib_file, edited).expect("failed to write semantic edit");

    let (edit_output, edit_report) = run_uc_build(&workspace, "after-edit");
    assert_success(&edit_output, "post-edit build");
    assert!(
        !edit_report.cache_hit,
        "semantic edit must invalidate cache"
    );
    assert_ne!(
        baseline_report.fingerprint, edit_report.fingerprint,
        "fingerprint should change after semantic edit"
    );

    let (steady_output, steady_report) = run_uc_build(&workspace, "after-edit-steady");
    assert_success(&steady_output, "steady post-edit build");
    assert!(steady_report.cache_hit, "steady build should hit cache");
    assert_eq!(edit_report.fingerprint, steady_report.fingerprint);
}

#[test]
fn integration_corrupted_cache_entry_recovers_without_crash() {
    let _guard = serial_guard();
    if !scarb_available() {
        eprintln!(
            "skipping integration_corrupted_cache_entry_recovers_without_crash: scarb not available"
        );
        return;
    }
    let workspace = make_test_workspace("corruption");

    let (seed_output, seed_report) = run_uc_build(&workspace, "seed");
    assert_success(&seed_output, "seed build");
    assert!(!seed_report.cache_hit, "seed build should miss cache");

    let entry_path = cache_entry_path(&workspace);
    fs::write(&entry_path, b"{not-json").expect("failed to corrupt cache entry JSON");

    let (recover_output, recover_report) = run_uc_build(&workspace, "recover");
    assert_success(&recover_output, "recovery build");
    assert!(
        !recover_report.cache_hit,
        "build after corruption should fall back to fresh compile"
    );

    let (stabilize_output, stabilize_report) = run_uc_build(&workspace, "stabilize");
    assert_success(&stabilize_output, "stabilization build");
    assert!(
        stabilize_report.cache_hit,
        "subsequent build should hit cache again after recovery"
    );
}
