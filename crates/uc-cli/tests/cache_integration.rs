use serde::Deserialize;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

#[derive(Debug, Deserialize)]
struct BuildReport {
    cache_hit: bool,
    fingerprint: String,
    session_key: String,
    exit_code: i32,
}

struct TestWorkspace {
    root: PathBuf,
}

#[derive(Default)]
struct BuildEnvOverrides<'a> {
    path_override: Option<&'a Path>,
    scarb_version_override: Option<&'a str>,
    native_mode_override: Option<&'a str>,
    native_corelib_override: Option<&'a Path>,
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

fn scarb_version_line() -> String {
    let output = Command::new("scarb")
        .arg("--version")
        .output()
        .expect("failed to execute `scarb --version` in test");
    assert!(
        output.status.success(),
        "`scarb --version` should succeed in test setup"
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or("scarb unknown")
        .trim()
        .to_string()
}

fn run_uc_build_for_root(
    root: &Path,
    manifest_path: &Path,
    report_tag: &str,
    daemon_mode: &str,
    daemon_socket_path: Option<&Path>,
) -> (Output, BuildReport) {
    run_uc_build_for_root_with_path_override(
        root,
        manifest_path,
        report_tag,
        daemon_mode,
        daemon_socket_path,
        None,
    )
}

fn run_uc_build_for_root_with_path_override(
    root: &Path,
    manifest_path: &Path,
    report_tag: &str,
    daemon_mode: &str,
    daemon_socket_path: Option<&Path>,
    path_override: Option<&Path>,
) -> (Output, BuildReport) {
    run_uc_build_for_root_with_env_overrides(
        root,
        manifest_path,
        report_tag,
        daemon_mode,
        daemon_socket_path,
        BuildEnvOverrides {
            path_override,
            ..BuildEnvOverrides::default()
        },
    )
}

fn run_uc_build_for_root_with_env_overrides(
    root: &Path,
    manifest_path: &Path,
    report_tag: &str,
    daemon_mode: &str,
    daemon_socket_path: Option<&Path>,
    overrides: BuildEnvOverrides<'_>,
) -> (Output, BuildReport) {
    let report_path = root.join(".uc").join(format!("report-{report_tag}.json"));
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent).expect("failed to create report directory");
    }
    let mut command = Command::new(uc_bin());
    command
        .current_dir(root)
        .arg("build")
        .arg("--engine")
        .arg("uc")
        .arg("--daemon-mode")
        .arg(daemon_mode)
        .arg("--offline")
        .arg("--manifest-path")
        .arg(manifest_path)
        .arg("--report-path")
        .arg(&report_path);
    if let Some(socket_path) = daemon_socket_path {
        command.env("UC_DAEMON_SOCKET_PATH", socket_path);
    } else {
        command.env_remove("UC_DAEMON_SOCKET_PATH");
    }
    if let Some(path) = overrides.path_override {
        command.env("PATH", path);
    }
    if let Some(version) = overrides.scarb_version_override {
        command.env("UC_SCARB_VERSION_LINE", version);
    } else {
        command.env_remove("UC_SCARB_VERSION_LINE");
    }
    if let Some(mode) = overrides.native_mode_override {
        command.env("UC_NATIVE_BUILD_MODE", mode);
    } else {
        command.env_remove("UC_NATIVE_BUILD_MODE");
    }
    if let Some(corelib_src) = overrides.native_corelib_override {
        command.env("UC_NATIVE_CORELIB_SRC", corelib_src);
    } else {
        command.env_remove("UC_NATIVE_CORELIB_SRC");
    }
    let output = command.output().expect("failed to execute uc build");
    let report_bytes = fs::read(&report_path).unwrap_or_else(|err| {
        panic!(
            "missing build report at {}: {}\n{}",
            report_path.display(),
            err,
            output_to_utf8(&output)
        )
    });
    let report: BuildReport =
        serde_json::from_slice(&report_bytes).expect("failed to decode build report JSON");
    (output, report)
}

fn run_uc_build_output_only(
    root: &Path,
    manifest_path: &Path,
    daemon_mode: &str,
    daemon_socket_path: Option<&Path>,
) -> Output {
    run_uc_build_output_only_with_env_overrides(
        root,
        manifest_path,
        daemon_mode,
        daemon_socket_path,
        None,
        None,
    )
}

fn run_uc_build_output_only_with_env_overrides(
    root: &Path,
    manifest_path: &Path,
    daemon_mode: &str,
    daemon_socket_path: Option<&Path>,
    native_mode_override: Option<&str>,
    native_corelib_override: Option<&Path>,
) -> Output {
    let mut command = Command::new(uc_bin());
    command
        .current_dir(root)
        .arg("build")
        .arg("--engine")
        .arg("uc")
        .arg("--daemon-mode")
        .arg(daemon_mode)
        .arg("--offline")
        .arg("--manifest-path")
        .arg(manifest_path);
    if let Some(socket_path) = daemon_socket_path {
        command.env("UC_DAEMON_SOCKET_PATH", socket_path);
    } else {
        command.env_remove("UC_DAEMON_SOCKET_PATH");
    }
    if let Some(mode) = native_mode_override {
        command.env("UC_NATIVE_BUILD_MODE", mode);
    } else {
        command.env_remove("UC_NATIVE_BUILD_MODE");
    }
    if let Some(corelib_src) = native_corelib_override {
        command.env("UC_NATIVE_CORELIB_SRC", corelib_src);
    } else {
        command.env_remove("UC_NATIVE_CORELIB_SRC");
    }
    command.output().expect("failed to execute uc build")
}

fn run_uc_build(workspace: &TestWorkspace, report_tag: &str) -> (Output, BuildReport) {
    let manifest = workspace.root.join("Scarb.toml");
    run_uc_build_for_root(&workspace.root, &manifest, report_tag, "off", None)
}

fn run_uc_daemon_stop(socket_path: &Path) -> Output {
    Command::new(uc_bin())
        .arg("daemon")
        .arg("stop")
        .arg("--socket-path")
        .arg(socket_path)
        .output()
        .expect("failed to execute uc daemon stop")
}

fn run_uc_daemon_start(socket_path: &Path) -> Output {
    Command::new(uc_bin())
        .arg("daemon")
        .arg("start")
        .arg("--socket-path")
        .arg(socket_path)
        .output()
        .expect("failed to execute uc daemon start")
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
fn integration_contract_fixture_semantic_edit_invalidates_then_recovers_to_hit() {
    let _guard = serial_guard();
    if !scarb_available() {
        eprintln!(
            "skipping integration_contract_fixture_semantic_edit_invalidates_then_recovers_to_hit: scarb not available"
        );
        return;
    }
    let workspace = make_test_workspace("contract-semantic-edit");

    let (baseline_output, baseline_report) = run_uc_build(&workspace, "contract-baseline");
    assert_success(&baseline_output, "contract baseline build");
    assert!(
        !baseline_report.cache_hit,
        "contract baseline build should miss cache"
    );

    let contract_file = workspace.root.join("src/contract_patterns.cairo");
    let baseline = fs::read_to_string(&contract_file).expect("failed to read contract fixture");
    let edited = baseline.replacen(
        "let nonce = self.allowance_nonce.read((owner, spender)) + 1_u64;",
        "let nonce = self.allowance_nonce.read((owner, spender)) + 2_u64;",
        1,
    );
    assert_ne!(
        baseline, edited,
        "contract semantic edit marker should exist in fixture"
    );
    fs::write(&contract_file, edited).expect("failed to write contract semantic edit");

    let (edit_output, edit_report) = run_uc_build(&workspace, "contract-after-edit");
    assert_success(&edit_output, "contract post-edit build");
    assert!(
        !edit_report.cache_hit,
        "contract semantic edit must invalidate cache"
    );
    assert_ne!(
        baseline_report.fingerprint, edit_report.fingerprint,
        "fingerprint should change after contract semantic edit"
    );

    let (steady_output, steady_report) = run_uc_build(&workspace, "contract-after-edit-steady");
    assert_success(&steady_output, "contract steady post-edit build");
    assert!(
        steady_report.cache_hit,
        "contract steady build should hit cache"
    );
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

#[test]
fn integration_concurrent_builds_complete_and_cache_recovers_to_hit() {
    let _guard = serial_guard();
    if !scarb_available() {
        eprintln!(
            "skipping integration_concurrent_builds_complete_and_cache_recovers_to_hit: scarb not available"
        );
        return;
    }
    let workspace = make_test_workspace("concurrent-builds");

    let root_a = workspace.root.clone();
    let root_b = workspace.root.clone();

    let worker_a = thread::spawn(move || {
        run_uc_build_for_root(
            &root_a,
            &root_a.join("Scarb.toml"),
            "concurrent-a",
            "off",
            None,
        )
    });
    let worker_b = thread::spawn(move || {
        run_uc_build_for_root(
            &root_b,
            &root_b.join("Scarb.toml"),
            "concurrent-b",
            "off",
            None,
        )
    });

    let (output_a, report_a) = worker_a
        .join()
        .expect("worker A thread panicked during concurrent build");
    let (output_b, report_b) = worker_b
        .join()
        .expect("worker B thread panicked during concurrent build");

    assert_success(&output_a, "concurrent build A");
    assert_success(&output_b, "concurrent build B");
    assert_eq!(report_a.exit_code, 0);
    assert_eq!(report_b.exit_code, 0);

    let (stabilize_output, stabilize_report) = run_uc_build(&workspace, "concurrent-stabilize");
    assert_success(
        &stabilize_output,
        "stabilization build after concurrent runs",
    );
    assert!(
        stabilize_report.cache_hit,
        "cache should converge to a hit after concurrent runs complete"
    );
}

#[test]
fn integration_daemon_restart_preserves_cache_hit_correctness() {
    let _guard = serial_guard();
    if !scarb_available() {
        eprintln!(
            "skipping integration_daemon_restart_preserves_cache_hit_correctness: scarb not available"
        );
        return;
    }
    let workspace = make_test_workspace("daemon-restart");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let socket_path =
        std::env::temp_dir().join(format!("uc-it-daemon-{}-{nonce}.sock", std::process::id()));
    let _ = fs::remove_file(&socket_path);

    let start_output = run_uc_daemon_start(&socket_path);
    assert!(
        start_output.status.success(),
        "daemon start should succeed: {}",
        output_to_utf8(&start_output)
    );

    let manifest = workspace.root.join("Scarb.toml");
    let (first_output, first_report) = run_uc_build_for_root(
        &workspace.root,
        &manifest,
        "daemon-first",
        "require",
        Some(&socket_path),
    );
    assert_success(&first_output, "first daemon build");
    assert_eq!(first_report.exit_code, 0);
    assert!(
        !first_report.cache_hit,
        "first daemon build should compile and populate cache"
    );

    let (second_output, second_report) = run_uc_build_for_root(
        &workspace.root,
        &manifest,
        "daemon-second",
        "require",
        Some(&socket_path),
    );
    assert_success(&second_output, "second daemon build");
    assert!(
        second_report.cache_hit,
        "second daemon build should reuse cache"
    );

    let stop_output = run_uc_daemon_stop(&socket_path);
    assert!(
        stop_output.status.success(),
        "daemon stop should succeed before restart: {}",
        output_to_utf8(&stop_output)
    );

    let restart_output = run_uc_daemon_start(&socket_path);
    assert!(
        restart_output.status.success(),
        "daemon restart should succeed: {}",
        output_to_utf8(&restart_output)
    );

    let (after_restart_output, after_restart_report) = run_uc_build_for_root(
        &workspace.root,
        &manifest,
        "daemon-after-restart",
        "require",
        Some(&socket_path),
    );
    assert_success(&after_restart_output, "daemon build after restart");
    assert!(
        after_restart_report.cache_hit,
        "cache hit should persist across daemon restart"
    );

    let _ = run_uc_daemon_stop(&socket_path);
    let _ = fs::remove_file(&socket_path);
}

#[test]
fn integration_manifest_path_variants_preserve_fingerprint_determinism() {
    let _guard = serial_guard();
    if !scarb_available() {
        eprintln!(
            "skipping integration_manifest_path_variants_preserve_fingerprint_determinism: scarb not available"
        );
        return;
    }
    let workspace = make_test_workspace("manifest-path-determinism");
    let abs_manifest = workspace.root.join("Scarb.toml");
    let rel_manifest = PathBuf::from("./Scarb.toml");

    let (abs_output, abs_report) =
        run_uc_build_for_root(&workspace.root, &abs_manifest, "manifest-abs", "off", None);
    assert_success(&abs_output, "absolute manifest build");

    let (rel_output, rel_report) =
        run_uc_build_for_root(&workspace.root, &rel_manifest, "manifest-rel", "off", None);
    assert_success(&rel_output, "relative manifest build");
    assert!(
        rel_report.cache_hit,
        "relative path build should hit existing cache"
    );
    assert_eq!(
        abs_report.fingerprint, rel_report.fingerprint,
        "fingerprint must be stable across equivalent manifest path spellings"
    );
    assert_eq!(
        abs_report.session_key, rel_report.session_key,
        "session key must be stable across equivalent manifest path spellings"
    );
}

#[test]
fn integration_workspace_clones_preserve_fingerprint_and_session_key() {
    let _guard = serial_guard();
    if !scarb_available() {
        eprintln!(
            "skipping integration_workspace_clones_preserve_fingerprint_and_session_key: scarb not available"
        );
        return;
    }
    let workspace_a = make_test_workspace("clone-determinism-a");
    let workspace_b = make_test_workspace("clone-determinism-b");

    let (output_a, report_a) = run_uc_build(&workspace_a, "clone-a");
    let (output_b, report_b) = run_uc_build(&workspace_b, "clone-b");
    assert_success(&output_a, "clone A build");
    assert_success(&output_b, "clone B build");
    assert!(
        !report_a.cache_hit && !report_b.cache_hit,
        "first build in each clone should be a cache miss"
    );
    assert_eq!(
        report_a.fingerprint, report_b.fingerprint,
        "fingerprint should be path-portable across equivalent workspace clones"
    );
    assert_eq!(
        report_a.session_key, report_b.session_key,
        "session key should be path-portable across equivalent workspace clones"
    );
}

#[test]
fn integration_daemon_require_mode_fails_when_daemon_unavailable() {
    let _guard = serial_guard();
    if !scarb_available() {
        eprintln!(
            "skipping integration_daemon_require_mode_fails_when_daemon_unavailable: scarb not available"
        );
        return;
    }
    let workspace = make_test_workspace("daemon-require-missing");
    let socket_path = std::env::temp_dir().join(format!(
        "uc-it-missing-daemon-{}-{}.sock",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos()
    ));
    let _ = fs::remove_file(&socket_path);

    let output = run_uc_build_output_only(
        &workspace.root,
        &workspace.root.join("Scarb.toml"),
        "require",
        Some(&socket_path),
    );
    assert!(
        !output.status.success(),
        "daemon require mode should fail when daemon socket is unavailable"
    );
    let combined = output_to_utf8(&output);
    assert!(
        combined.contains("daemon mode is require but daemon is unavailable")
            || combined.contains("daemon build request failed"),
        "unexpected daemon require failure output: {combined}"
    );
}

#[test]
fn integration_daemon_auto_mode_local_hit_skips_daemon_and_missing_scarb() {
    let _guard = serial_guard();
    if !scarb_available() {
        eprintln!(
            "skipping integration_daemon_auto_mode_local_hit_skips_daemon_and_missing_scarb: scarb not available"
        );
        return;
    }
    let workspace = make_test_workspace("daemon-auto-local-hit");
    let manifest = workspace.root.join("Scarb.toml");
    let scarb_version = scarb_version_line();

    let (seed_output, seed_report) =
        run_uc_build_for_root(&workspace.root, &manifest, "auto-seed", "off", None);
    assert_success(&seed_output, "seed build for daemon auto local-hit");
    assert!(
        !seed_report.cache_hit,
        "seed build should miss before local-hit probe can be exercised"
    );

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let missing_socket = std::env::temp_dir().join(format!(
        "uc-it-auto-local-hit-missing-daemon-{}-{nonce}.sock",
        std::process::id()
    ));
    let no_scarb_path = std::env::temp_dir().join(format!(
        "uc-it-no-scarb-path-{}-{nonce}",
        std::process::id()
    ));
    let _ = fs::remove_file(&missing_socket);

    let (probe_output, probe_report) = run_uc_build_for_root_with_env_overrides(
        &workspace.root,
        &manifest,
        "auto-local-hit",
        "auto",
        Some(&missing_socket),
        BuildEnvOverrides {
            path_override: Some(&no_scarb_path),
            scarb_version_override: Some(&scarb_version),
            ..BuildEnvOverrides::default()
        },
    );
    assert_success(&probe_output, "daemon auto local-hit probe");
    assert!(
        probe_report.cache_hit,
        "daemon auto mode should hit local cache before daemon/local compile fallback"
    );
    let combined = output_to_utf8(&probe_output);
    assert!(
        !combined.contains("daemon request failed"),
        "local probe hit should avoid daemon request path: {combined}"
    );
}

#[test]
fn integration_daemon_require_mode_local_hit_skips_missing_daemon_and_scarb() {
    let _guard = serial_guard();
    if !scarb_available() {
        eprintln!(
            "skipping integration_daemon_require_mode_local_hit_skips_missing_daemon_and_scarb: scarb not available"
        );
        return;
    }
    let workspace = make_test_workspace("daemon-require-local-hit");
    let manifest = workspace.root.join("Scarb.toml");
    let scarb_version = scarb_version_line();

    let (seed_output, seed_report) =
        run_uc_build_for_root(&workspace.root, &manifest, "require-seed", "off", None);
    assert_success(&seed_output, "seed build for daemon require local-hit");
    assert!(
        !seed_report.cache_hit,
        "seed build should miss before local-hit probe can be exercised"
    );

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let missing_socket = std::env::temp_dir().join(format!(
        "uc-it-require-local-hit-missing-daemon-{}-{nonce}.sock",
        std::process::id()
    ));
    let no_scarb_path = std::env::temp_dir().join(format!(
        "uc-it-no-scarb-path-require-{}-{nonce}",
        std::process::id()
    ));
    let _ = fs::remove_file(&missing_socket);

    let (probe_output, probe_report) = run_uc_build_for_root_with_env_overrides(
        &workspace.root,
        &manifest,
        "require-local-hit",
        "require",
        Some(&missing_socket),
        BuildEnvOverrides {
            path_override: Some(&no_scarb_path),
            scarb_version_override: Some(&scarb_version),
            ..BuildEnvOverrides::default()
        },
    );
    assert_success(&probe_output, "daemon require local-hit probe");
    assert!(
        probe_report.cache_hit,
        "daemon require mode should use local hit when artifacts are already cached"
    );
    let combined = output_to_utf8(&probe_output);
    assert!(
        !combined.contains("daemon mode is require but daemon is unavailable"),
        "local probe hit should bypass daemon unavailability failure: {combined}"
    );
}

#[test]
fn integration_native_auto_mode_falls_back_when_native_backend_unavailable() {
    let _guard = serial_guard();
    if !scarb_available() {
        eprintln!(
            "skipping integration_native_auto_mode_falls_back_when_native_backend_unavailable: scarb not available"
        );
        return;
    }
    let workspace = make_test_workspace("native-auto-fallback");
    let manifest = workspace.root.join("Scarb.toml");
    let missing_corelib = std::env::temp_dir().join(format!(
        "uc-it-missing-corelib-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&missing_corelib);

    let (output, report) = run_uc_build_for_root_with_env_overrides(
        &workspace.root,
        &manifest,
        "native-auto-fallback",
        "off",
        None,
        BuildEnvOverrides {
            native_mode_override: Some("auto"),
            native_corelib_override: Some(&missing_corelib),
            ..BuildEnvOverrides::default()
        },
    );
    assert_success(&output, "native auto fallback build");
    assert_eq!(report.exit_code, 0);
    let combined = output_to_utf8(&output);
    assert!(
        combined.contains("native compile unavailable"),
        "native auto mode should log fallback reason: {combined}"
    );
}

#[test]
fn integration_native_require_mode_fails_when_native_backend_unavailable() {
    let _guard = serial_guard();
    if !scarb_available() {
        eprintln!(
            "skipping integration_native_require_mode_fails_when_native_backend_unavailable: scarb not available"
        );
        return;
    }
    let workspace = make_test_workspace("native-require-missing");
    let manifest = workspace.root.join("Scarb.toml");
    let missing_corelib = std::env::temp_dir().join(format!(
        "uc-it-missing-corelib-require-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&missing_corelib);

    let output = run_uc_build_output_only_with_env_overrides(
        &workspace.root,
        &manifest,
        "off",
        None,
        Some("require"),
        Some(&missing_corelib),
    );
    assert!(
        !output.status.success(),
        "native require mode should fail when native backend cannot initialize"
    );
    let combined = output_to_utf8(&output);
    assert!(
        combined.contains("native compile mode is require but native backend failed"),
        "unexpected native require failure output: {combined}"
    );
}
