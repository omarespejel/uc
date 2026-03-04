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
        BuildRunOptions {
            capture_output: true,
            inherit_output_when_uncaptured: true,
            async_cache_persist: false,
        },
    )
}

#[test]
fn daemon_metadata_request_roundtrip_preserves_fields() {
    let args = MetadataArgs {
        manifest_path: Some(PathBuf::from("/tmp/workspace/Scarb.toml")),
        format_version: 2,
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
    assert_eq!(restored.format_version, 2);
    assert_eq!(request.protocol_version, DAEMON_PROTOCOL_VERSION);
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
    let request = daemon_build_request_from_common(
        &common,
        Path::new("/tmp/workspace/Scarb.toml"),
        true,
        false,
    );
    let restored = common_args_from_daemon_request(&request);

    assert!(request.async_cache_persist);
    assert!(!request.capture_output);
    assert_eq!(request.protocol_version, DAEMON_PROTOCOL_VERSION);
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
        protocol_version: DAEMON_PROTOCOL_VERSION.to_string(),
        manifest_path: "/tmp/workspace/Scarb.toml".to_string(),
        package: None,
        workspace: false,
        features: vec!["feature_a".to_string()],
        offline: false,
        release: false,
        profile: None,
        async_cache_persist: true,
        capture_output: true,
    });
    let json = serde_json::to_string(&request).expect("failed to encode request");
    assert!(json.contains("\"type\":\"build\""));
    assert!(json.contains("\"async_cache_persist\":true"));
    assert!(json.contains("\"capture_output\":true"));

    let decoded: DaemonRequest =
        serde_json::from_str(&json).expect("failed to decode daemon request");
    match decoded {
        DaemonRequest::Build(payload) => {
            assert!(payload.async_cache_persist);
            assert!(payload.capture_output);
            assert_eq!(payload.protocol_version, DAEMON_PROTOCOL_VERSION);
            assert_eq!(payload.manifest_path, "/tmp/workspace/Scarb.toml");
            assert_eq!(payload.features, vec!["feature_a".to_string()]);
        }
        _ => panic!("expected build request"),
    }
}

#[test]
fn daemon_build_request_defaults_capture_output_when_missing_from_wire() {
    let json = format!(
            "{{\"type\":\"build\",\"protocol_version\":\"{}\",\"manifest_path\":\"/tmp/workspace/Scarb.toml\",\"package\":null,\"workspace\":false,\"features\":[],\"offline\":false,\"release\":false,\"profile\":null,\"async_cache_persist\":false}}",
            DAEMON_PROTOCOL_VERSION
        );
    let decoded: DaemonRequest =
        serde_json::from_str(&json).expect("failed to decode daemon request");
    match decoded {
        DaemonRequest::Build(payload) => {
            assert!(payload.capture_output);
            assert_eq!(payload.protocol_version, DAEMON_PROTOCOL_VERSION);
        }
        _ => panic!("expected build request"),
    }
}

#[test]
fn daemon_metadata_request_serialization_supports_wire_format() {
    let request = DaemonRequest::Metadata(DaemonMetadataRequest {
        protocol_version: DAEMON_PROTOCOL_VERSION.to_string(),
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
            assert_eq!(payload.protocol_version, DAEMON_PROTOCOL_VERSION);
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
    let (_, command_vec) = scarb_metadata_command(&args, Path::new("/tmp/workspace/Scarb.toml"));
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
fn scarb_version_line_uses_env_override() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::set_var("UC_SCARB_VERSION_LINE", "scarb 9.9.9 (override)");
    let version = scarb_version_line().expect("override version should be accepted");
    assert_eq!(version, "scarb 9.9.9 (override)");
    std::env::remove_var("UC_SCARB_VERSION_LINE");
}

#[test]
fn validate_scarb_version_constraints_respects_minimum_and_expected() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::set_var("UC_MIN_SCARB_VERSION", "2.14.0");
    std::env::remove_var("UC_EXPECT_SCARB_VERSION");
    assert!(validate_scarb_version_constraints("scarb 2.14.1 (local)").is_ok());
    assert!(validate_scarb_version_constraints("scarb 2.13.9 (local)").is_err());

    std::env::set_var("UC_MIN_SCARB_VERSION", "2.14.0");
    std::env::set_var("UC_EXPECT_SCARB_VERSION", "2.14.1");
    assert!(validate_scarb_version_constraints("scarb 2.14.1 (local)").is_ok());
    std::env::set_var("UC_EXPECT_SCARB_VERSION", "2.14.9");
    assert!(validate_scarb_version_constraints("scarb 2.14.1 (local)").is_err());

    std::env::remove_var("UC_MIN_SCARB_VERSION");
    std::env::remove_var("UC_EXPECT_SCARB_VERSION");
}

#[test]
fn scarb_toolchain_cache_load_rejects_stale_entries() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = unique_test_dir("uc-toolchain-cache");
    let cache_path = dir.join("scarb-toolchain.json");
    std::env::set_var("UC_SCARB_TOOLCHAIN_CACHE_PATH", &cache_path);
    std::env::set_var("UC_SCARB_TOOLCHAIN_CACHE_TTL_MS", "600000");

    store_cached_scarb_toolchain_version_line("scarb 2.14.2 (cached)");
    let loaded = load_cached_scarb_toolchain_version_line();
    assert_eq!(loaded.as_deref(), Some("scarb 2.14.2 (cached)"));

    let stale = ToolchainCheckCacheEntry {
        schema_version: TOOLCHAIN_CHECK_CACHE_SCHEMA_VERSION,
        checked_epoch_ms: 0,
        version_line: "scarb 2.14.2 (cached)".to_string(),
    };
    fs::write(&cache_path, serde_json::to_vec(&stale).unwrap())
        .expect("failed to write stale toolchain cache");
    assert!(
        load_cached_scarb_toolchain_version_line().is_none(),
        "stale toolchain cache should be ignored"
    );

    std::env::remove_var("UC_SCARB_TOOLCHAIN_CACHE_PATH");
    std::env::remove_var("UC_SCARB_TOOLCHAIN_CACHE_TTL_MS");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn build_env_fingerprint_tracks_prefixed_vars_only() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::remove_var("UC_BUILD_ENV_PREFIXES_EXTRA");
    std::env::remove_var("SCARB_TEST_UC_FINGERPRINT");
    std::env::remove_var("UC_TEST_UNRELATED_FINGERPRINT");

    let baseline = compute_build_env_fingerprint();
    std::env::set_var("SCARB_TEST_UC_FINGERPRINT", "v1");
    let with_prefixed = compute_build_env_fingerprint();
    assert_ne!(baseline, with_prefixed);

    std::env::set_var("UC_TEST_UNRELATED_FINGERPRINT", "noise");
    let with_unrelated = compute_build_env_fingerprint();
    assert_eq!(with_prefixed, with_unrelated);

    std::env::set_var("SCARB_TEST_UC_FINGERPRINT", "v2");
    let with_prefixed_change = compute_build_env_fingerprint();
    assert_ne!(with_prefixed, with_prefixed_change);

    std::env::remove_var("SCARB_TEST_UC_FINGERPRINT");
    std::env::remove_var("UC_TEST_UNRELATED_FINGERPRINT");
}

#[test]
fn build_env_fingerprint_supports_extra_prefixes() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::remove_var("UC_BUILD_ENV_PREFIXES_EXTRA");
    std::env::set_var("XBUILD_TEST_FINGERPRINT", "v1");
    let without_extra = compute_build_env_fingerprint();

    std::env::set_var("UC_BUILD_ENV_PREFIXES_EXTRA", "XBUILD_");
    let with_extra = compute_build_env_fingerprint();
    assert_ne!(without_extra, with_extra);

    std::env::remove_var("XBUILD_TEST_FINGERPRINT");
    std::env::remove_var("UC_BUILD_ENV_PREFIXES_EXTRA");
}

#[test]
fn session_input_cache_key_is_order_independent_for_features() {
    let common_a = BuildCommonArgs {
        manifest_path: Some(PathBuf::from("/tmp/workspace/Scarb.toml")),
        package: Some("demo".to_string()),
        workspace: false,
        features: vec!["b".to_string(), "a".to_string()],
        offline: false,
        release: false,
        profile: None,
    };
    let common_b = BuildCommonArgs {
        manifest_path: Some(PathBuf::from("/tmp/workspace/Scarb.toml")),
        package: Some("demo".to_string()),
        workspace: false,
        features: vec!["a".to_string(), "b".to_string(), "a".to_string()],
        offline: false,
        release: false,
        profile: None,
    };

    let key_a = session_input_cache_key(
        &common_a,
        Path::new("/tmp/workspace/Scarb.toml"),
        "dev",
        "scarb 2.14.0",
        "env-a",
    );
    let key_b = session_input_cache_key(
        &common_b,
        Path::new("/tmp/workspace/Scarb.toml"),
        "dev",
        "scarb 2.14.0",
        "env-a",
    );
    assert_eq!(key_a, key_b);
}

#[test]
fn session_input_cache_key_changes_with_build_env_fingerprint() {
    let common = BuildCommonArgs {
        manifest_path: Some(PathBuf::from("/tmp/workspace/Scarb.toml")),
        package: Some("demo".to_string()),
        workspace: false,
        features: vec!["a".to_string()],
        offline: false,
        release: false,
        profile: None,
    };
    let key_a = session_input_cache_key(
        &common,
        Path::new("/tmp/workspace/Scarb.toml"),
        "dev",
        "scarb 2.14.0",
        "env-a",
    );
    let key_b = session_input_cache_key(
        &common,
        Path::new("/tmp/workspace/Scarb.toml"),
        "dev",
        "scarb 2.14.0",
        "env-b",
    );
    assert_ne!(key_a, key_b);
}

#[test]
fn cache_budget_enforcement_stride_triggers_every_nth_persist() {
    assert!(!should_enforce_cache_size_budget_for_persist_index(1, 8));
    assert!(!should_enforce_cache_size_budget_for_persist_index(7, 8));
    assert!(should_enforce_cache_size_budget_for_persist_index(8, 8));
    assert!(!should_enforce_cache_size_budget_for_persist_index(9, 8));
    assert!(should_enforce_cache_size_budget_for_persist_index(16, 8));
}

#[test]
fn cache_budget_enforcement_stride_one_triggers_every_persist() {
    for persist_index in 1..=8 {
        assert!(should_enforce_cache_size_budget_for_persist_index(
            persist_index,
            1
        ));
    }
}

#[test]
fn cache_budget_enforcement_state_respects_interval_and_first_arm() {
    assert!(!should_enforce_cache_size_budget_for_state(
        8, 8, 10_000, 0, 60_000
    ));
    assert!(!should_enforce_cache_size_budget_for_state(
        8, 8, 20_000, 10_000, 60_000
    ));
    assert!(should_enforce_cache_size_budget_for_state(
        16, 8, 80_000, 10_000, 60_000
    ));
}

#[test]
fn cache_budget_enforcement_state_interval_zero_uses_stride_only() {
    assert!(!should_enforce_cache_size_budget_for_state(
        7, 8, 1_000, 0, 0
    ));
    assert!(should_enforce_cache_size_budget_for_state(
        8, 8, 1_000, 0, 0
    ));
}

#[test]
fn parse_metadata_format_version_accepts_supported_values() {
    assert_eq!(parse_metadata_format_version("1").unwrap(), 1);
    assert_eq!(parse_metadata_format_version("2").unwrap(), 2);
    assert!(parse_metadata_format_version("3").is_err());
}

#[test]
fn validate_daemon_protocol_version_rejects_mismatch() {
    let err = validate_daemon_protocol_version("0.0.0").expect_err("expected mismatch");
    assert!(
        format!("{err:#}").contains("daemon protocol mismatch"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn persist_artifact_object_materializes_destination() {
    let dir = unique_test_dir("uc-persist-object");
    let source = dir.join("source.bin");
    let destination = dir.join("objects/aa/object.bin");
    fs::create_dir_all(
        destination
            .parent()
            .expect("destination should have parent directory"),
    )
    .expect("failed to create object directory");
    fs::write(&source, b"artifact-bytes").expect("failed to write source object");

    persist_artifact_object(&source, &destination).expect("persist should succeed");

    let restored = fs::read(&destination).expect("failed to read destination");
    assert_eq!(restored, b"artifact-bytes");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn restore_cache_object_overwrites_existing_file() {
    let dir = unique_test_dir("uc-restore-object");
    let source = dir.join("source.bin");
    let destination = dir.join("target/output.bin");
    fs::create_dir_all(
        destination
            .parent()
            .expect("destination should have parent directory"),
    )
    .expect("failed to create destination directory");
    fs::write(&source, b"fresh-object").expect("failed to write source object");
    fs::write(&destination, b"stale-object").expect("failed to write stale destination");

    restore_cache_object(&source, &destination).expect("restore should succeed");

    let restored = fs::read(&destination).expect("failed to read restored object");
    assert_eq!(restored, b"fresh-object");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn restore_cached_artifacts_skips_object_restore_when_index_matches() {
    let dir = unique_test_dir("uc-restore-index-hit");
    let workspace = dir.join("workspace");
    let target_root = workspace.join("target/dev");
    let cache_root = workspace.join(".uc/cache");
    let objects_dir = cache_root.join("objects");
    fs::create_dir_all(&target_root).expect("failed to create target root");
    fs::create_dir_all(&objects_dir).expect("failed to create objects root");

    let output = target_root.join("demo.sierra.json");
    fs::write(&output, b"cached-artifact").expect("failed to write target artifact");
    let output_metadata = fs::metadata(&output).expect("failed to stat target artifact");
    let expected_hash = hash_file_blake3(&output).expect("failed to hash target artifact");

    let mut artifact_index = ArtifactIndex::empty();
    artifact_index.entries.insert(
        "demo.sierra.json".to_string(),
        ArtifactIndexEntry {
            size_bytes: output_metadata.len(),
            modified_unix_ms: metadata_modified_unix_ms(&output_metadata)
                .expect("failed to read target mtime"),
            blake3_hex: expected_hash.clone(),
        },
    );
    save_artifact_index(&cache_root.join("artifact-index-v1.json"), &artifact_index)
        .expect("failed to write artifact index");

    let artifacts = vec![CachedArtifact {
        relative_path: "demo.sierra.json".to_string(),
        blake3_hex: expected_hash,
        size_bytes: output_metadata.len(),
        object_rel_path: "aa/missing-object.bin".to_string(),
    }];

    let restored =
        restore_cached_artifacts(&workspace, "dev", &cache_root, &objects_dir, &artifacts)
            .expect("restore should succeed");
    assert!(restored);
    assert_eq!(
        fs::read(&output).expect("failed to read target artifact"),
        b"cached-artifact"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn restore_cached_artifacts_restores_when_target_mismatch() {
    let dir = unique_test_dir("uc-restore-index-miss");
    let workspace = dir.join("workspace");
    let target_root = workspace.join("target/dev");
    let cache_root = workspace.join(".uc/cache");
    let objects_dir = cache_root.join("objects/aa");
    fs::create_dir_all(&target_root).expect("failed to create target root");
    fs::create_dir_all(&objects_dir).expect("failed to create objects root");

    let output = target_root.join("demo.sierra.json");
    fs::write(&output, b"stale-artifact").expect("failed to write stale artifact");
    let object = objects_dir.join("fresh-object.bin");
    fs::write(&object, b"fresh-artifact").expect("failed to write cache object");
    let expected_hash = hash_file_blake3(&object).expect("failed to hash cache object");
    let object_metadata = fs::metadata(&object).expect("failed to stat cache object");

    let artifacts = vec![CachedArtifact {
        relative_path: "demo.sierra.json".to_string(),
        blake3_hex: expected_hash,
        size_bytes: object_metadata.len(),
        object_rel_path: "aa/fresh-object.bin".to_string(),
    }];

    let restored = restore_cached_artifacts(
        &workspace,
        "dev",
        &cache_root,
        &cache_root.join("objects"),
        &artifacts,
    )
    .expect("restore should succeed");
    assert!(restored);
    assert_eq!(
        fs::read(&output).expect("failed to read restored artifact"),
        b"fresh-artifact"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn validate_manifest_dependency_sanity_rejects_self_dependency() {
    let dir = unique_test_dir("uc-self-dependency");
    let manifest_path = dir.join("Scarb.toml");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"

[dependencies]
demo = "1.0.0"
"#,
    )
    .expect("failed to write manifest");

    let result = validate_manifest_dependency_sanity(&manifest_path);
    assert!(result.is_err());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn normalize_fingerprint_path_normalizes_windows_prefix() {
    assert_eq!(
        normalize_fingerprint_path(Path::new(r"\\?\C:\tmp\demo\Scarb.toml")),
        "C:/tmp/demo/Scarb.toml"
    );
}

#[test]
fn hot_fingerprint_reuses_digest_when_tracked_metadata_is_unchanged() {
    let dir = unique_test_dir("uc-hot-fingerprint-hit");
    let src_dir = dir.join("src");
    fs::create_dir_all(&src_dir).expect("failed to create src dir");
    let source = src_dir.join("lib.cairo");
    fs::write(&source, b"fn main() -> felt252 { 1 }").expect("failed to write source file");

    let source_metadata = fs::metadata(&source).expect("failed to stat source");
    let src_dir_metadata = fs::metadata(&src_dir).expect("failed to stat src dir");
    let root_metadata = fs::metadata(&dir).expect("failed to stat workspace");

    let mut entries = BTreeMap::new();
    entries.insert(
        "src/lib.cairo".to_string(),
        FingerprintIndexEntry {
            size_bytes: source_metadata.len(),
            modified_unix_ms: metadata_modified_unix_ms(&source_metadata)
                .expect("failed to read source mtime"),
            blake3_hex: "abc123".to_string(),
        },
    );
    let mut directories = BTreeMap::new();
    directories.insert(
        ".".to_string(),
        metadata_modified_unix_ms(&root_metadata).expect("failed to read workspace mtime"),
    );
    directories.insert(
        "src".to_string(),
        metadata_modified_unix_ms(&src_dir_metadata).expect("failed to read src dir mtime"),
    );

    let index = FingerprintIndex {
        schema_version: FINGERPRINT_INDEX_SCHEMA_VERSION,
        entries,
        directories,
        context_digest: Some("ctx".to_string()),
        last_fingerprint: Some("fp-hot-hit".to_string()),
    };
    let reused = try_reuse_hot_fingerprint(&dir, &index, "ctx", u64::MAX, 0)
        .expect("hot-path check should succeed");
    assert_eq!(reused, Some("fp-hot-hit".to_string()));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn hot_fingerprint_invalidates_when_tracked_directory_mtime_changes() {
    let dir = unique_test_dir("uc-hot-fingerprint-dir-change");
    let src_dir = dir.join("src");
    fs::create_dir_all(&src_dir).expect("failed to create src dir");
    let source = src_dir.join("lib.cairo");
    fs::write(&source, b"fn main() -> felt252 { 1 }").expect("failed to write source file");

    let source_metadata = fs::metadata(&source).expect("failed to stat source");
    let src_dir_metadata = fs::metadata(&src_dir).expect("failed to stat src dir");
    let root_metadata = fs::metadata(&dir).expect("failed to stat workspace");

    let mut entries = BTreeMap::new();
    entries.insert(
        "src/lib.cairo".to_string(),
        FingerprintIndexEntry {
            size_bytes: source_metadata.len(),
            modified_unix_ms: metadata_modified_unix_ms(&source_metadata)
                .expect("failed to read source mtime"),
            blake3_hex: "abc123".to_string(),
        },
    );
    let mut directories = BTreeMap::new();
    directories.insert(
        ".".to_string(),
        metadata_modified_unix_ms(&root_metadata).expect("failed to read workspace mtime"),
    );
    directories.insert(
        "src".to_string(),
        metadata_modified_unix_ms(&src_dir_metadata).expect("failed to read src dir mtime"),
    );

    let index = FingerprintIndex {
        schema_version: FINGERPRINT_INDEX_SCHEMA_VERSION,
        entries,
        directories,
        context_digest: Some("ctx".to_string()),
        last_fingerprint: Some("fp-hot-hit".to_string()),
    };

    thread::sleep(Duration::from_millis(20));
    fs::write(src_dir.join("new.cairo"), b"fn extra() -> felt252 { 2 }")
        .expect("failed to add new source");

    let reused = try_reuse_hot_fingerprint(&dir, &index, "ctx", u64::MAX, 0)
        .expect("hot-path check should succeed");
    assert!(reused.is_none());
    fs::remove_dir_all(&dir).ok();
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
        updated.replace(
            "BENCH_EDIT_SEED_BIAS: felt252 = 0",
            "BENCH_EDIT_SEED_BIAS: felt252 = 1",
        ),
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
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = take_async_persist_errors();
    record_async_persist_error("err-a".to_string());
    record_async_persist_error("err-b".to_string());

    assert_eq!(
        take_async_persist_errors(),
        vec!["err-a".to_string(), "err-b".to_string()]
    );
    let _ = take_async_persist_errors();
}

#[test]
fn async_persist_error_queue_drops_oldest_when_over_capacity() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
    let _ = take_async_persist_errors();
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
