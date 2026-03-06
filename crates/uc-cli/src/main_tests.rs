use super::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsString;
use std::fs;
use std::sync::{Arc, Mutex, OnceLock as TestOnceLock};
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

#[cfg(unix)]
fn unique_unix_socket_path(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before UNIX_EPOCH")
        .as_nanos();
    PathBuf::from("/tmp").join(format!(
        "{prefix}-{}-{}.sock",
        std::process::id(),
        nanos % 1_000_000
    ))
}

fn integration_env_lock() -> &'static Mutex<()> {
    static LOCK: TestOnceLock<Mutex<()>> = TestOnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct CurrentDirRestore {
    original: PathBuf,
}

impl CurrentDirRestore {
    fn capture() -> Self {
        Self {
            original: std::env::current_dir().expect("failed to read current directory"),
        }
    }
}

impl Drop for CurrentDirRestore {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}

struct ScopedEnvVar {
    key: &'static str,
    previous: Option<OsString>,
}

impl ScopedEnvVar {
    // All process-global env mutation in tests must run under `integration_env_lock()`
    // to avoid cross-test races when the harness executes tests in parallel.
    fn set_with_lock(
        _guard: &std::sync::MutexGuard<'_, ()>,
        key: &'static str,
        value: impl AsRef<std::ffi::OsStr>,
    ) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
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

fn create_mock_native_corelib(corelib_src: &Path) {
    fs::create_dir_all(corelib_src).expect("failed to create mock corelib src");
    fs::write(corelib_src.join("lib.cairo"), "mod prelude;\nmod ops;\n")
        .expect("failed to write mock corelib lib.cairo");
    fs::write(corelib_src.join("prelude.cairo"), "").expect("failed to write mock prelude");
    fs::write(corelib_src.join("ops.cairo"), "").expect("failed to write mock ops");
}

fn write_mock_native_corelib_manifest(corelib_src: &Path, version: &str) {
    let manifest_path = corelib_src
        .parent()
        .expect("corelib src should have parent")
        .join("Scarb.toml");
    fs::write(
        manifest_path,
        format!("[package]\nname = \"core\"\nversion = \"{version}\"\nedition = \"2024_07\"\n"),
    )
    .expect("failed to write mock corelib manifest");
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
    let compiler_version = scarb_version_line()?;
    run_build_with_uc_cache(
        common,
        BuildCacheRunContext {
            manifest_path,
            workspace_root,
            profile,
            session_key,
            compiler_version: &compiler_version,
            compile_backend: BuildCompileBackend::Scarb,
            options: BuildRunOptions {
                capture_output: true,
                inherit_output_when_uncaptured: true,
                async_cache_persist: false,
                use_daemon_shared_cache: false,
            },
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
        BuildCompileBackend::Native,
        true,
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
    assert_eq!(request.compile_backend, DaemonBuildBackend::Native);
    assert!(request.native_fallback_to_scarb);
}

#[cfg(feature = "native-compile")]
#[test]
fn native_compile_session_heap_estimate_scales_with_tracked_source_bytes() {
    let small = native_compile_session_estimated_heap_bytes(1024);
    let large = native_compile_session_estimated_heap_bytes(2 * 1024 * 1024);
    assert!(
        small >= 1024 + native_compile_session_memory_base_overhead_bytes(),
        "small estimate should include tracked bytes and base overhead"
    );
    assert!(
        large > small,
        "larger tracked source snapshots should consume larger estimated session memory"
    );
}

#[test]
fn daemon_build_request_serialization_supports_async_cache_persist_wire_field() {
    let request = DaemonRequest::Build {
        payload: DaemonBuildRequest {
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
            compile_backend: DaemonBuildBackend::Native,
            native_fallback_to_scarb: true,
        },
    };
    let json = serde_json::to_string(&request).expect("failed to encode request");
    assert!(json.contains("\"type\":\"build\""));
    assert!(json.contains("\"async_cache_persist\":true"));
    assert!(json.contains("\"capture_output\":true"));
    assert!(json.contains("\"compile_backend\":\"native\""));
    assert!(json.contains("\"native_fallback_to_scarb\":true"));

    let decoded: DaemonRequest =
        serde_json::from_str(&json).expect("failed to decode daemon request");
    match decoded {
        DaemonRequest::Build { payload } => {
            assert!(payload.async_cache_persist);
            assert!(payload.capture_output);
            assert_eq!(payload.protocol_version, DAEMON_PROTOCOL_VERSION);
            assert_eq!(payload.manifest_path, "/tmp/workspace/Scarb.toml");
            assert_eq!(payload.features, vec!["feature_a".to_string()]);
            assert_eq!(payload.compile_backend, DaemonBuildBackend::Native);
            assert!(payload.native_fallback_to_scarb);
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
        DaemonRequest::Build { payload } => {
            assert!(payload.capture_output);
            assert_eq!(payload.protocol_version, DAEMON_PROTOCOL_VERSION);
            assert_eq!(payload.compile_backend, DaemonBuildBackend::Scarb);
            assert!(!payload.native_fallback_to_scarb);
        }
        _ => panic!("expected build request"),
    }
}

#[test]
fn daemon_build_request_payload_wrapped_wire_format_is_decoded() {
    let json = format!(
        r#"{{
            "type":"build",
            "payload":{{
                "protocol_version":"{}",
                "manifest_path":"/tmp/workspace/Scarb.toml",
                "package":null,
                "workspace":false,
                "features":["feature_a"],
                "offline":false,
                "release":false,
                "profile":null,
                "async_cache_persist":true,
                "capture_output":true,
                "compile_backend":"native",
                "native_fallback_to_scarb":true
            }}
        }}"#,
        DAEMON_PROTOCOL_VERSION
    );
    let decoded = decode_daemon_request(&json).expect("failed to decode wrapped daemon request");
    match decoded {
        DaemonRequest::Build { payload } => {
            assert_eq!(payload.protocol_version, DAEMON_PROTOCOL_VERSION);
            assert_eq!(payload.features, vec!["feature_a".to_string()]);
            assert_eq!(payload.compile_backend, DaemonBuildBackend::Native);
            assert!(payload.async_cache_persist);
            assert!(payload.native_fallback_to_scarb);
        }
        _ => panic!("expected build request"),
    }
}

#[test]
fn daemon_build_request_payload_fields_override_hybrid_root_fields() {
    let json = format!(
        r#"{{
            "type":"build",
            "manifest_path":"/tmp/root-overridden/Scarb.toml",
            "payload":{{
                "protocol_version":"{}",
                "manifest_path":"/tmp/payload-authoritative/Scarb.toml",
                "package":null,
                "workspace":false,
                "features":[],
                "offline":false,
                "release":false,
                "profile":null,
                "async_cache_persist":false,
                "capture_output":true,
                "compile_backend":"native",
                "native_fallback_to_scarb":true
            }}
        }}"#,
        DAEMON_PROTOCOL_VERSION
    );
    let decoded = decode_daemon_request(&json).expect("failed to decode hybrid daemon request");
    match decoded {
        DaemonRequest::Build { payload } => {
            assert_eq!(
                payload.manifest_path,
                "/tmp/payload-authoritative/Scarb.toml"
            );
            assert_eq!(payload.compile_backend, DaemonBuildBackend::Native);
            assert!(payload.native_fallback_to_scarb);
        }
        _ => panic!("expected build request"),
    }
}

#[cfg(feature = "native-compile")]
#[test]
fn starknet_artifact_files_omits_casm_when_none() {
    let files = StarknetArtifactFiles {
        sierra: "demo.contract_class.json".to_string(),
        casm: None,
    };
    let json = serde_json::to_value(&files).expect("failed to encode artifact files");
    assert_eq!(
        json.get("sierra").and_then(serde_json::Value::as_str),
        Some("demo.contract_class.json")
    );
    assert!(
        json.get("casm").is_none(),
        "casm key should be omitted when not generated"
    );
}

#[test]
fn daemon_build_response_roundtrip_preserves_telemetry_fields() {
    let response = DaemonResponse::Build {
        payload: DaemonBuildResponse {
            run: CommandRun {
                command: vec!["scarb".to_string(), "build".to_string()],
                exit_code: 0,
                elapsed_ms: 123.4,
                stdout: "ok".to_string(),
                stderr: String::new(),
            },
            cache_hit: true,
            fingerprint: "abc123".to_string(),
            session_key: "session".to_string(),
            telemetry: BuildPhaseTelemetry {
                fingerprint_ms: 1.0,
                cache_lookup_ms: 2.0,
                cache_restore_ms: 3.0,
                compile_ms: 4.0,
                cache_persist_ms: 5.0,
                cache_persist_async: true,
                cache_persist_scheduled: false,
                ..BuildPhaseTelemetry::default()
            },
            compile_backend: DaemonBuildBackend::Native,
        },
    };

    let json = serde_json::to_string(&response).expect("failed to encode daemon response");
    let decoded = decode_daemon_response(&json).expect("failed to decode daemon response");
    match decoded {
        DaemonResponse::Build { payload } => {
            assert_eq!(payload.run.exit_code, 0);
            assert!(payload.cache_hit);
            assert_eq!(payload.fingerprint, "abc123");
            assert_eq!(payload.session_key, "session");
            assert_eq!(payload.telemetry.compile_ms, 4.0);
            assert!(payload.telemetry.cache_persist_async);
            assert!(!payload.telemetry.cache_persist_scheduled);
            assert_eq!(payload.compile_backend, DaemonBuildBackend::Native);
        }
        _ => panic!("expected build response"),
    }
}

#[test]
fn daemon_build_response_legacy_flat_format_is_decoded() {
    // Legacy daemon wire format (flat fields, no top-level `payload` wrapper).
    let json = r#"{
        "type":"build",
        "run":{
            "command":["scarb","build"],
            "exit_code":0,
            "elapsed_ms":12.5,
            "stdout":"",
            "stderr":""
        },
        "cache_hit":false,
        "fingerprint":"f",
        "session_key":"s",
        "compile_backend":"scarb",
        "telemetry":{
            "fingerprint_ms":0.1,
            "cache_lookup_ms":0.2,
            "cache_restore_ms":0.3,
            "compile_ms":10.0,
            "cache_persist_ms":0.4,
            "cache_persist_async":false,
            "cache_persist_scheduled":false
        }
    }"#;
    let decoded = decode_daemon_response(json).expect("failed to decode daemon response");
    match decoded {
        DaemonResponse::Build { payload } => {
            assert_eq!(payload.run.elapsed_ms, 12.5);
            assert_eq!(payload.telemetry.compile_ms, 10.0);
            assert_eq!(payload.telemetry.native_context_ms, 0.0);
            assert_eq!(payload.telemetry.native_target_dir_ms, 0.0);
            assert_eq!(payload.compile_backend, DaemonBuildBackend::Scarb);
        }
        _ => panic!("expected build response"),
    }
}

#[test]
fn daemon_build_response_payload_wrapped_wire_format_is_decoded() {
    let json = r#"{
        "type":"build",
        "payload":{
            "run":{
                "command":["scarb","build"],
                "exit_code":0,
                "elapsed_ms":11.5,
                "stdout":"",
                "stderr":""
            },
            "cache_hit":false,
            "fingerprint":"f",
            "session_key":"s",
            "compile_backend":"scarb",
            "telemetry":{
                "fingerprint_ms":0.1,
                "cache_lookup_ms":0.2,
                "cache_restore_ms":0.3,
                "compile_ms":9.0,
                "cache_persist_ms":0.4,
                "cache_persist_async":false,
                "cache_persist_scheduled":false
            }
        }
    }"#;
    let decoded = decode_daemon_response(json).expect("failed to decode wrapped daemon response");
    match decoded {
        DaemonResponse::Build { payload } => {
            assert_eq!(payload.run.elapsed_ms, 11.5);
            assert_eq!(payload.telemetry.compile_ms, 9.0);
            assert_eq!(payload.compile_backend, DaemonBuildBackend::Scarb);
        }
        _ => panic!("expected build response"),
    }
}

#[test]
fn daemon_build_response_payload_fields_override_hybrid_root_fields() {
    let json = r#"{
        "type":"build",
        "compile_backend":"scarb",
        "payload":{
            "run":{
                "command":["uc","build"],
                "exit_code":0,
                "elapsed_ms":7.5,
                "stdout":"",
                "stderr":""
            },
            "cache_hit":false,
            "fingerprint":"f",
            "session_key":"s",
            "compile_backend":"native",
            "telemetry":{
                "fingerprint_ms":0.1,
                "cache_lookup_ms":0.2,
                "cache_restore_ms":0.3,
                "compile_ms":9.0,
                "cache_persist_ms":0.4,
                "cache_persist_async":false,
                "cache_persist_scheduled":false
            }
        }
    }"#;
    let decoded = decode_daemon_response(json).expect("failed to decode hybrid daemon response");
    match decoded {
        DaemonResponse::Build { payload } => {
            assert_eq!(payload.compile_backend, DaemonBuildBackend::Native);
            assert_eq!(payload.run.elapsed_ms, 7.5);
        }
        _ => panic!("expected build response"),
    }
}

#[test]
fn daemon_build_response_deserializes_with_native_subphase_telemetry() {
    let json = r#"{
        "type":"build",
        "run":{
            "command":["uc","build"],
            "exit_code":0,
            "elapsed_ms":33.0,
            "stdout":"",
            "stderr":""
        },
        "cache_hit":false,
        "fingerprint":"f",
        "session_key":"s",
        "compile_backend":"native",
        "telemetry":{
            "fingerprint_ms":0.1,
            "cache_lookup_ms":0.2,
            "cache_restore_ms":0.3,
            "compile_ms":30.0,
            "cache_persist_ms":0.4,
            "cache_persist_async":false,
            "cache_persist_scheduled":false,
            "native_context_ms":1.1,
            "native_target_dir_ms":1.2,
            "native_session_prepare_ms":2.3,
            "native_frontend_compile_ms":22.4,
            "native_casm_ms":3.5,
            "native_artifact_write_ms":4.6
        }
    }"#;
    let decoded = decode_daemon_response(json).expect("failed to decode daemon response");
    match decoded {
        DaemonResponse::Build { payload } => {
            assert_eq!(payload.compile_backend, DaemonBuildBackend::Native);
            assert_eq!(payload.telemetry.native_context_ms, 1.1);
            assert_eq!(payload.telemetry.native_target_dir_ms, 1.2);
            assert_eq!(payload.telemetry.native_session_prepare_ms, 2.3);
            assert_eq!(payload.telemetry.native_frontend_compile_ms, 22.4);
            assert_eq!(payload.telemetry.native_casm_ms, 3.5);
            assert_eq!(payload.telemetry.native_artifact_write_ms, 4.6);
        }
        _ => panic!("expected build response"),
    }
}

#[test]
fn daemon_build_response_defaults_compile_backend_to_scarb_when_missing_from_wire() {
    let json = r#"{
        "type":"build",
        "run":{
            "command":["scarb","build"],
            "exit_code":0,
            "elapsed_ms":2.5,
            "stdout":"",
            "stderr":""
        },
        "cache_hit":true,
        "fingerprint":"f",
        "session_key":"s",
        "telemetry":{
            "fingerprint_ms":0.0,
            "cache_lookup_ms":0.0,
            "cache_restore_ms":0.0,
            "compile_ms":0.0,
            "cache_persist_ms":0.0,
            "cache_persist_async":false,
            "cache_persist_scheduled":false
        }
    }"#;
    let decoded = decode_daemon_response(json).expect("failed to decode daemon response");
    match decoded {
        DaemonResponse::Build { payload } => {
            assert_eq!(payload.compile_backend, DaemonBuildBackend::Scarb);
        }
        _ => panic!("expected build response"),
    }
}

#[test]
fn daemon_metadata_request_serialization_supports_wire_format() {
    let request = DaemonRequest::Metadata {
        payload: DaemonMetadataRequest {
            protocol_version: DAEMON_PROTOCOL_VERSION.to_string(),
            manifest_path: "/tmp/workspace/Scarb.toml".to_string(),
            format_version: 1,
            offline: false,
            global_cache_dir: None,
            capture_output: false,
        },
    };
    let json = serde_json::to_string(&request).expect("failed to encode request");
    assert!(json.contains("\"type\":\"metadata\""));

    let decoded: DaemonRequest =
        serde_json::from_str(&json).expect("failed to decode daemon request");
    match decoded {
        DaemonRequest::Metadata { payload } => {
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
fn daemon_request_protocol_validation_skips_ping_and_shutdown() {
    assert!(validate_daemon_request_protocol_version(&DaemonRequest::Ping).is_ok());
    assert!(validate_daemon_request_protocol_version(&DaemonRequest::Shutdown).is_ok());
}

#[test]
fn daemon_request_protocol_validation_rejects_mismatch_for_build_and_metadata() {
    let build = DaemonRequest::Build {
        payload: DaemonBuildRequest {
            protocol_version: "0.0.0".to_string(),
            manifest_path: "/tmp/workspace/Scarb.toml".to_string(),
            package: None,
            workspace: false,
            features: Vec::new(),
            offline: false,
            release: false,
            profile: None,
            async_cache_persist: false,
            capture_output: true,
            compile_backend: DaemonBuildBackend::Scarb,
            native_fallback_to_scarb: false,
        },
    };
    let metadata = DaemonRequest::Metadata {
        payload: DaemonMetadataRequest {
            protocol_version: "0.0.0".to_string(),
            manifest_path: "/tmp/workspace/Scarb.toml".to_string(),
            format_version: 1,
            offline: false,
            global_cache_dir: None,
            capture_output: false,
        },
    };

    let build_err = validate_daemon_request_protocol_version(&build)
        .expect_err("build request protocol mismatch should fail");
    assert!(
        format!("{build_err:#}").contains("daemon protocol mismatch"),
        "unexpected build mismatch error: {build_err:#}"
    );

    let metadata_err = validate_daemon_request_protocol_version(&metadata)
        .expect_err("metadata request protocol mismatch should fail");
    assert!(
        format!("{metadata_err:#}").contains("daemon protocol mismatch"),
        "unexpected metadata mismatch error: {metadata_err:#}"
    );
}

#[test]
fn daemon_request_protocol_validation_accepts_current_protocol() {
    let build = DaemonRequest::Build {
        payload: DaemonBuildRequest {
            protocol_version: DAEMON_PROTOCOL_VERSION.to_string(),
            manifest_path: "/tmp/workspace/Scarb.toml".to_string(),
            package: None,
            workspace: false,
            features: Vec::new(),
            offline: false,
            release: false,
            profile: None,
            async_cache_persist: false,
            capture_output: true,
            compile_backend: DaemonBuildBackend::Scarb,
            native_fallback_to_scarb: false,
        },
    };
    let metadata = DaemonRequest::Metadata {
        payload: DaemonMetadataRequest {
            protocol_version: DAEMON_PROTOCOL_VERSION.to_string(),
            manifest_path: "/tmp/workspace/Scarb.toml".to_string(),
            format_version: 1,
            offline: false,
            global_cache_dir: None,
            capture_output: false,
        },
    };

    assert!(validate_daemon_request_protocol_version(&build).is_ok());
    assert!(validate_daemon_request_protocol_version(&metadata).is_ok());
}

#[test]
fn resolve_manifest_path_accepts_absolute_manifest() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _cwd_guard = CurrentDirRestore::capture();
    let workspace = unique_test_dir("uc-resolve-manifest-abs");
    let manifest = workspace.join("Scarb.toml");
    fs::write(
        &manifest,
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .expect("failed to write manifest");
    let resolved =
        resolve_manifest_path(&Some(manifest.clone())).expect("absolute manifest should resolve");
    assert_eq!(
        resolved,
        manifest
            .canonicalize()
            .expect("failed to canonicalize manifest")
    );
}

#[test]
fn resolve_manifest_path_rejects_relative_escape() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _cwd_guard = CurrentDirRestore::capture();
    let root = unique_test_dir("uc-resolve-manifest-escape");
    let cwd = root.join("workspace");
    let outside = root.join("outside");
    fs::create_dir_all(&cwd).expect("failed to create test cwd");
    fs::create_dir_all(&outside).expect("failed to create outside directory");
    fs::write(
        outside.join("Scarb.toml"),
        "[package]\nname = \"escape\"\nversion = \"0.1.0\"\n",
    )
    .expect("failed to write outside manifest");
    std::env::set_current_dir(&cwd).expect("failed to set test cwd");
    let err = resolve_manifest_path(&Some(PathBuf::from("../outside/Scarb.toml")))
        .expect_err("relative manifest escape should fail");
    assert!(
        format!("{err:#}").contains("escapes current working directory"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn read_line_limited_reads_up_to_newline() {
    let data = b"hello-world\nnext";
    let mut reader = std::io::BufReader::new(std::io::Cursor::new(data.as_slice()));
    let line = read_line_limited(&mut reader, 64, "test line").expect("line read should succeed");
    assert_eq!(line, "hello-world");
}

#[test]
fn read_line_limited_rejects_oversized_line() {
    let payload = vec![b'a'; 32];
    let mut reader = std::io::BufReader::new(std::io::Cursor::new(payload));
    let err = read_line_limited(&mut reader, 8, "test line")
        .expect_err("oversized line should be rejected");
    assert!(
        format!("{err:#}").contains("exceeds size limit"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn read_line_limited_rejects_extra_bytes_after_exact_limit_without_newline() {
    let payload = [vec![b'a'; 8], vec![b'b']].concat();
    let mut reader = std::io::BufReader::with_capacity(8, std::io::Cursor::new(payload));
    let err = read_line_limited(&mut reader, 8, "test line")
        .expect_err("line should fail once bytes arrive after reaching the exact size limit");
    assert!(
        format!("{err:#}").contains("exceeds size limit"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn daemon_response_size_limit_exceeds_request_and_capture_budget() {
    let limit = daemon_response_size_limit_bytes();
    let minimum = max_capture_stdout_bytes()
        .saturating_add(max_capture_stderr_bytes())
        .saturating_add(DAEMON_RESPONSE_SIZE_OVERHEAD_BYTES as u64)
        .min(usize::MAX as u64) as usize;
    assert!(
        limit >= DAEMON_REQUEST_SIZE_LIMIT_BYTES,
        "daemon response limit must never be below request limit"
    );
    assert!(
        limit >= minimum,
        "daemon response limit must cover configured capture budgets"
    );
}

#[cfg(unix)]
#[test]
fn daemon_request_accepts_large_response_payload() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let socket_path = unique_unix_socket_path("uc-daemon-large-response");
    let _ = fs::remove_file(&socket_path);
    let listener =
        std::os::unix::net::UnixListener::bind(&socket_path).expect("failed to bind socket");
    let expected_len = 2 * 1024 * 1024;
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("failed to accept request");
        let mut line = String::new();
        {
            let mut reader = std::io::BufReader::new(
                stream
                    .try_clone()
                    .expect("failed to clone daemon test stream"),
            );
            let _ = std::io::BufRead::read_line(&mut reader, &mut line)
                .expect("failed to read daemon request");
        }
        assert!(
            !line.trim().is_empty(),
            "daemon request payload should not be empty"
        );
        let message = "x".repeat(expected_len);
        let payload = format!(r#"{{"type":"error","message":"{message}"}}"#);
        stream
            .write_all(payload.as_bytes())
            .expect("failed to write daemon response");
        stream
            .write_all(b"\n")
            .expect("failed to write daemon response delimiter");
        stream.flush().expect("failed to flush daemon response");
    });

    let response =
        daemon_request(&socket_path, &DaemonRequest::Ping).expect("daemon request should succeed");
    match response {
        DaemonResponse::Error { message } => {
            assert_eq!(
                message.len(),
                expected_len,
                "daemon client should accept responses larger than request size limit"
            );
        }
        other => panic!("expected error response, got {other:?}"),
    }

    server.join().expect("daemon test server panicked");
    let _ = fs::remove_file(&socket_path);
}

#[cfg(unix)]
#[test]
fn try_uc_build_via_daemon_auto_mode_falls_back_on_daemon_error() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let socket_path = unique_unix_socket_path("uc-daemon-auto-fallback");
    let _ = fs::remove_file(&socket_path);
    let listener = std::os::unix::net::UnixListener::bind(&socket_path)
        .expect("failed to bind test daemon socket");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("failed to accept daemon request");
        let mut line = String::new();
        {
            let mut reader = std::io::BufReader::new(
                stream
                    .try_clone()
                    .expect("failed to clone daemon test stream"),
            );
            let _ = std::io::BufRead::read_line(&mut reader, &mut line)
                .expect("failed to read daemon request");
        }
        assert!(
            !line.trim().is_empty(),
            "daemon request payload should not be empty"
        );
        stream
            .write_all(br#"{"type":"error","message":"simulated daemon failure"}"#)
            .expect("failed to write daemon response");
        stream
            .write_all(b"\n")
            .expect("failed to write daemon response delimiter");
        stream.flush().expect("failed to flush daemon response");
    });
    let _socket_env = ScopedEnvVar::set_with_lock(&_guard, "UC_DAEMON_SOCKET_PATH", &socket_path);

    let common = BuildCommonArgs {
        manifest_path: Some(PathBuf::from("/tmp/workspace/Scarb.toml")),
        package: None,
        workspace: false,
        features: Vec::new(),
        offline: false,
        release: false,
        profile: None,
    };
    let result = try_uc_build_via_daemon(
        &common,
        Path::new("/tmp/workspace/Scarb.toml"),
        true,
        BuildCompileBackend::Scarb,
        false,
    )
    .expect("auto mode should not fail hard when daemon returns error");
    assert!(result.is_none());

    server.join().expect("daemon test server panicked");
    let _ = fs::remove_file(&socket_path);
}

#[cfg(unix)]
#[test]
fn try_uc_build_via_daemon_require_mode_surfaces_daemon_error() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let socket_path = unique_unix_socket_path("uc-daemon-require-error");
    let _ = fs::remove_file(&socket_path);
    let listener = std::os::unix::net::UnixListener::bind(&socket_path)
        .expect("failed to bind test daemon socket");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("failed to accept daemon request");
        let mut line = String::new();
        {
            let mut reader = std::io::BufReader::new(
                stream
                    .try_clone()
                    .expect("failed to clone daemon test stream"),
            );
            let _ = std::io::BufRead::read_line(&mut reader, &mut line)
                .expect("failed to read daemon request");
        }
        assert!(
            !line.trim().is_empty(),
            "daemon request payload should not be empty"
        );
        stream
            .write_all(br#"{"type":"error","message":"simulated daemon failure"}"#)
            .expect("failed to write daemon response");
        stream
            .write_all(b"\n")
            .expect("failed to write daemon response delimiter");
        stream.flush().expect("failed to flush daemon response");
    });
    let _socket_env = ScopedEnvVar::set_with_lock(&_guard, "UC_DAEMON_SOCKET_PATH", &socket_path);

    let common = BuildCommonArgs {
        manifest_path: Some(PathBuf::from("/tmp/workspace/Scarb.toml")),
        package: None,
        workspace: false,
        features: Vec::new(),
        offline: false,
        release: false,
        profile: None,
    };
    let err = try_uc_build_via_daemon(
        &common,
        Path::new("/tmp/workspace/Scarb.toml"),
        false,
        BuildCompileBackend::Scarb,
        false,
    )
    .expect_err("require mode should fail when daemon returns an error");
    let text = format!("{err:#}");
    assert!(
        text.contains("daemon build request failed: simulated daemon failure"),
        "unexpected error: {text}"
    );

    server.join().expect("daemon test server panicked");
    let _ = fs::remove_file(&socket_path);
}

#[cfg(unix)]
#[test]
fn try_uc_build_via_daemon_require_mode_rejects_backend_mismatch() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let socket_path = unique_unix_socket_path("uc-daemon-require-backend-mismatch");
    let _ = fs::remove_file(&socket_path);
    let listener = std::os::unix::net::UnixListener::bind(&socket_path)
        .expect("failed to bind test daemon socket");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("failed to accept daemon request");
        let mut line = String::new();
        {
            let mut reader = std::io::BufReader::new(
                stream
                    .try_clone()
                    .expect("failed to clone daemon test stream"),
            );
            let _ = std::io::BufRead::read_line(&mut reader, &mut line)
                .expect("failed to read daemon request");
        }
        assert!(
            !line.trim().is_empty(),
            "daemon request payload should not be empty"
        );
        stream
            .write_all(
                br#"{"type":"build","run":{"command":["uc","build"],"exit_code":0,"elapsed_ms":1.0,"stdout":"","stderr":""},"cache_hit":false,"fingerprint":"fp","session_key":"abc","telemetry":{"fingerprint_ms":0.0,"cache_lookup_ms":0.0,"cache_restore_ms":0.0,"compile_ms":0.0,"cache_persist_ms":0.0,"cache_persist_async":false,"cache_persist_scheduled":false}}"#,
            )
            .expect("failed to write daemon response");
        stream
            .write_all(b"\n")
            .expect("failed to write daemon response delimiter");
        stream.flush().expect("failed to flush daemon response");
    });
    let _socket_env = ScopedEnvVar::set_with_lock(&_guard, "UC_DAEMON_SOCKET_PATH", &socket_path);

    let common = BuildCommonArgs {
        manifest_path: Some(PathBuf::from("/tmp/workspace/Scarb.toml")),
        package: None,
        workspace: false,
        features: Vec::new(),
        offline: false,
        release: false,
        profile: None,
    };
    let err = try_uc_build_via_daemon(
        &common,
        Path::new("/tmp/workspace/Scarb.toml"),
        false,
        BuildCompileBackend::Native,
        false,
    )
    .expect_err("backend mismatch should fail in require mode");
    let text = format!("{err:#}");
    assert!(
        text.contains("daemon returned backend"),
        "unexpected backend mismatch error: {text}"
    );

    server.join().expect("daemon test server panicked");
    let _ = fs::remove_file(&socket_path);
}

#[cfg(unix)]
#[test]
fn remove_socket_if_exists_rejects_non_socket_file() {
    let dir = unique_test_dir("uc-remove-socket-guard");
    let path = dir.join("not-a-socket");
    fs::write(&path, b"file").expect("failed to write non-socket marker");
    let err = remove_socket_if_exists(&path).expect_err("non-socket path should be rejected");
    assert!(
        format!("{err:#}").contains("refusing to remove non-socket path"),
        "unexpected error: {err:#}"
    );
    assert!(path.exists(), "non-socket file should remain on disk");
    fs::remove_dir_all(&dir).ok();
}

#[cfg(unix)]
#[test]
fn remove_socket_if_exists_removes_socket_file() {
    let path = unique_unix_socket_path("uc-remove-socket-ok");
    let _ = fs::remove_file(&path);
    let listener =
        std::os::unix::net::UnixListener::bind(&path).expect("failed to create socket file");
    drop(listener);
    remove_socket_if_exists(&path).expect("socket path should be removable");
    assert!(!path.exists(), "socket file should be removed");
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
fn prewarm_daemon_compiler_version_cache_uses_override() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::set_var("UC_SCARB_VERSION_LINE", "scarb 8.8.8 (prewarm-test)");
    prewarm_daemon_compiler_version_cache();
    let version = scarb_version_line().expect("override version should remain accessible");
    assert_eq!(version, "scarb 8.8.8 (prewarm-test)");
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
fn build_env_fingerprint_prefix_override_can_disable_default_prefixes() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::remove_var("UC_BUILD_ENV_PREFIXES");
    std::env::set_var("SCARB_TEST_OVERRIDE_FINGERPRINT", "v1");
    let with_defaults = compute_build_env_fingerprint();

    std::env::set_var("UC_BUILD_ENV_PREFIXES", "");
    let without_defaults = compute_build_env_fingerprint();
    assert_ne!(with_defaults, without_defaults);

    std::env::remove_var("SCARB_TEST_OVERRIDE_FINGERPRINT");
    std::env::remove_var("UC_BUILD_ENV_PREFIXES");
}

#[test]
fn scarb_build_command_disables_artifacts_fingerprint_by_default() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::remove_var("UC_DISABLE_SCARB_ARTIFACTS_FINGERPRINT");
    let common = BuildCommonArgs {
        manifest_path: Some(PathBuf::from("/tmp/workspace/Scarb.toml")),
        package: None,
        workspace: false,
        features: Vec::new(),
        offline: false,
        release: false,
        profile: None,
    };
    let (command, _) = scarb_build_command(&common, Path::new("/tmp/workspace/Scarb.toml"));
    let configured = command
        .get_envs()
        .find(|(key, _)| *key == std::ffi::OsStr::new("SCARB_ARTIFACTS_FINGERPRINT"))
        .and_then(|(_, value)| value)
        .map(|value| value.to_string_lossy().to_string());
    assert_eq!(configured.as_deref(), Some("0"));
}

#[test]
fn scarb_build_command_can_reenable_artifacts_fingerprint() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::set_var("UC_DISABLE_SCARB_ARTIFACTS_FINGERPRINT", "0");
    let common = BuildCommonArgs {
        manifest_path: Some(PathBuf::from("/tmp/workspace/Scarb.toml")),
        package: None,
        workspace: false,
        features: Vec::new(),
        offline: false,
        release: false,
        profile: None,
    };
    let (command, _) = scarb_build_command(&common, Path::new("/tmp/workspace/Scarb.toml"));
    let configured = command
        .get_envs()
        .find(|(key, _)| *key == std::ffi::OsStr::new("SCARB_ARTIFACTS_FINGERPRINT"))
        .and_then(|(_, value)| value)
        .map(|value| value.to_string_lossy().to_string());
    assert_eq!(configured.as_deref(), Some("1"));
    std::env::remove_var("UC_DISABLE_SCARB_ARTIFACTS_FINGERPRINT");
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
fn session_input_cache_key_changes_with_compiler_version() {
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
        "uc-native 0.1.0",
        "env-a",
    );
    assert_ne!(key_a, key_b);
}

#[test]
fn native_build_mode_parses_expected_values() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::remove_var("UC_NATIVE_BUILD_MODE");
    assert_eq!(native_build_mode(), NativeBuildMode::Auto);

    std::env::set_var("UC_NATIVE_BUILD_MODE", "off");
    assert_eq!(native_build_mode(), NativeBuildMode::Off);

    std::env::set_var("UC_NATIVE_BUILD_MODE", "auto");
    assert_eq!(native_build_mode(), NativeBuildMode::Auto);

    std::env::set_var("UC_NATIVE_BUILD_MODE", "require");
    assert_eq!(native_build_mode(), NativeBuildMode::Require);

    std::env::set_var("UC_NATIVE_BUILD_MODE", "invalid-mode");
    assert_eq!(native_build_mode(), NativeBuildMode::Auto);
    std::env::remove_var("UC_NATIVE_BUILD_MODE");
}

#[test]
fn native_disallow_scarb_fallback_parses_expected_values() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::remove_var("UC_NATIVE_DISALLOW_SCARB_FALLBACK");
    assert!(!native_disallow_scarb_fallback());

    std::env::set_var("UC_NATIVE_DISALLOW_SCARB_FALLBACK", "1");
    assert!(native_disallow_scarb_fallback());

    std::env::set_var("UC_NATIVE_DISALLOW_SCARB_FALLBACK", "true");
    assert!(native_disallow_scarb_fallback());

    std::env::set_var("UC_NATIVE_DISALLOW_SCARB_FALLBACK", "0");
    assert!(!native_disallow_scarb_fallback());
    std::env::remove_var("UC_NATIVE_DISALLOW_SCARB_FALLBACK");
}

#[test]
fn parse_lockfile_dependency_version_extracts_target_package() {
    let lockfile = r#"
[[package]]
name = "foo"
version = "1.0.0"

[[package]]
name = "cairo-lang-compiler"
version = "2.16.0"
"#;
    assert_eq!(
        parse_lockfile_dependency_version(lockfile, "cairo-lang-compiler"),
        Some("2.16.0".to_string())
    );
    assert_eq!(
        parse_lockfile_dependency_version(lockfile, "missing-package"),
        None
    );
}

#[test]
fn native_lockfile_fallback_version_is_stable_and_not_unknown() {
    let lock_a = r#"
[[package]]
name = "foo"
version = "1.0.0"
"#;
    let lock_b = r#"
[[package]]
name = "foo"
version = "2.0.0"
"#;
    let fallback_a = native_lockfile_fallback_version(lock_a);
    let fallback_a_repeat = native_lockfile_fallback_version(lock_a);
    let fallback_b = native_lockfile_fallback_version(lock_b);

    assert_eq!(
        fallback_a, fallback_a_repeat,
        "lock-hash fallback should be stable for identical lockfile input"
    );
    assert_ne!(
        fallback_a, fallback_b,
        "lock-hash fallback should change when lockfile contents differ"
    );
    assert!(
        fallback_a.starts_with("lockhash-"),
        "fallback marker should be namespaced and explicit: {fallback_a}"
    );
}

#[test]
fn native_compiler_version_line_includes_cairo_lang_version() {
    let line = native_compiler_version_line();
    assert!(
        line.starts_with("uc-native "),
        "native compiler version should include uc prefix: {line}"
    );
    assert!(
        line.contains("cairo-lang"),
        "native compiler version should include cairo-lang marker: {line}"
    );
    assert!(
        line.contains(native_cairo_lang_compiler_version()),
        "native compiler version should include resolved cairo-lang version: {line}"
    );
}

#[test]
fn daemon_build_plan_cache_key_is_order_independent_for_features() {
    let common_a = BuildCommonArgs {
        manifest_path: Some(PathBuf::from("/tmp/workspace/Scarb.toml")),
        package: Some("demo".to_string()),
        workspace: false,
        features: vec!["b".to_string(), "a".to_string(), "a".to_string()],
        offline: true,
        release: false,
        profile: None,
    };
    let common_b = BuildCommonArgs {
        manifest_path: Some(PathBuf::from("/tmp/workspace/Scarb.toml")),
        package: Some("demo".to_string()),
        workspace: false,
        features: vec!["a".to_string(), "b".to_string()],
        offline: true,
        release: false,
        profile: None,
    };
    let key_a = daemon_build_plan_cache_key(
        &common_a,
        Path::new("/tmp/workspace/Scarb.toml"),
        "dev",
        BuildCompileBackend::Scarb,
        "scarb 2.14.0",
        "env-a",
    );
    let key_b = daemon_build_plan_cache_key(
        &common_b,
        Path::new("/tmp/workspace/Scarb.toml"),
        "dev",
        BuildCompileBackend::Scarb,
        "scarb 2.14.0",
        "env-a",
    );
    assert_eq!(
        key_a, key_b,
        "equivalent feature sets should produce identical daemon plan cache keys"
    );
}

#[test]
fn daemon_build_plan_cache_key_changes_with_compile_backend() {
    let common = BuildCommonArgs {
        manifest_path: Some(PathBuf::from("/tmp/workspace/Scarb.toml")),
        package: Some("demo".to_string()),
        workspace: false,
        features: vec!["feature_a".to_string()],
        offline: false,
        release: false,
        profile: None,
    };
    let scarb_key = daemon_build_plan_cache_key(
        &common,
        Path::new("/tmp/workspace/Scarb.toml"),
        "dev",
        BuildCompileBackend::Scarb,
        "scarb 2.16.0",
        "env-a",
    );
    let native_key = daemon_build_plan_cache_key(
        &common,
        Path::new("/tmp/workspace/Scarb.toml"),
        "dev",
        BuildCompileBackend::Native,
        "uc-native 0.1.0",
        "env-a",
    );
    assert_ne!(
        scarb_key, native_key,
        "daemon build plan cache key should partition by compile backend"
    );
}

#[test]
fn daemon_build_plan_cache_eviction_removes_oldest_entries() {
    let sample_plan = DaemonBuildPlan {
        manifest_path: PathBuf::from("/tmp/workspace/Scarb.toml"),
        workspace_root: PathBuf::from("/tmp/workspace"),
        profile: "dev".to_string(),
        session_key: "s".repeat(64),
        strict_invalidation_key: "k".repeat(64),
    };
    let mut cache = HashMap::new();
    cache.insert(
        "oldest".to_string(),
        DaemonBuildPlanCacheEntry {
            manifest_size_bytes: 1,
            manifest_modified_unix_ms: 1,
            lock_size_bytes: Some(1),
            lock_modified_unix_ms: Some(1),
            lock_hash: "lock-hash-a".to_string(),
            plan: sample_plan.clone(),
            last_access_epoch_ms: 1,
        },
    );
    cache.insert(
        "middle".to_string(),
        DaemonBuildPlanCacheEntry {
            manifest_size_bytes: 1,
            manifest_modified_unix_ms: 1,
            lock_size_bytes: Some(1),
            lock_modified_unix_ms: Some(1),
            lock_hash: "lock-hash-b".to_string(),
            plan: sample_plan.clone(),
            last_access_epoch_ms: 2,
        },
    );
    cache.insert(
        "newest".to_string(),
        DaemonBuildPlanCacheEntry {
            manifest_size_bytes: 1,
            manifest_modified_unix_ms: 1,
            lock_size_bytes: Some(1),
            lock_modified_unix_ms: Some(1),
            lock_hash: "lock-hash-c".to_string(),
            plan: sample_plan,
            last_access_epoch_ms: 3,
        },
    );

    evict_oldest_daemon_build_plan_cache_entries(&mut cache, 2);
    assert_eq!(cache.len(), 2);
    assert!(!cache.contains_key("oldest"));
    assert!(cache.contains_key("middle"));
    assert!(cache.contains_key("newest"));
}

#[test]
fn daemon_lock_hash_cache_eviction_removes_oldest_entries() {
    let mut cache = HashMap::new();
    cache.insert(
        "oldest".to_string(),
        LockfileHashCacheEntry {
            size_bytes: 10,
            modified_unix_ms: 10,
            change_unix_ms: Some(10),
            lock_hash: "a".repeat(64),
            last_access_epoch_ms: 1,
        },
    );
    cache.insert(
        "middle".to_string(),
        LockfileHashCacheEntry {
            size_bytes: 11,
            modified_unix_ms: 11,
            change_unix_ms: Some(11),
            lock_hash: "b".repeat(64),
            last_access_epoch_ms: 2,
        },
    );
    cache.insert(
        "newest".to_string(),
        LockfileHashCacheEntry {
            size_bytes: 12,
            modified_unix_ms: 12,
            change_unix_ms: Some(12),
            lock_hash: "c".repeat(64),
            last_access_epoch_ms: 3,
        },
    );

    evict_oldest_daemon_lock_hash_cache_entries(&mut cache, 2);
    assert_eq!(cache.len(), 2);
    assert!(!cache.contains_key("oldest"));
    assert!(cache.contains_key("middle"));
    assert!(cache.contains_key("newest"));
}

#[test]
fn metadata_result_cache_eviction_respects_entry_and_byte_budgets() {
    let run = |suffix: &str, size: usize| CommandRun {
        command: vec!["scarb".to_string(), "metadata".to_string()],
        exit_code: 0,
        elapsed_ms: 1.0,
        stdout: format!("{}{}", suffix, "x".repeat(size)),
        stderr: String::new(),
    };
    let mut cache = HashMap::new();
    cache.insert(
        "oldest".to_string(),
        MetadataResultCacheEntry {
            manifest_size_bytes: 1,
            manifest_modified_unix_ms: 1,
            lock_hash: "a".repeat(64),
            run: run("a", 1024),
            last_access_epoch_ms: 1,
            estimated_bytes: 2048,
        },
    );
    cache.insert(
        "middle".to_string(),
        MetadataResultCacheEntry {
            manifest_size_bytes: 1,
            manifest_modified_unix_ms: 1,
            lock_hash: "b".repeat(64),
            run: run("b", 1024),
            last_access_epoch_ms: 2,
            estimated_bytes: 2048,
        },
    );
    cache.insert(
        "newest".to_string(),
        MetadataResultCacheEntry {
            manifest_size_bytes: 1,
            manifest_modified_unix_ms: 1,
            lock_hash: "c".repeat(64),
            run: run("c", 1024),
            last_access_epoch_ms: 3,
            estimated_bytes: 2048,
        },
    );

    // Budget forces eviction by both max entries and max bytes.
    evict_oldest_metadata_result_cache_entries(&mut cache, 2, 4096);
    assert_eq!(cache.len(), 2);
    assert!(!cache.contains_key("oldest"));
    assert!(cache.contains_key("middle"));
    assert!(cache.contains_key("newest"));
}

#[cfg(feature = "native-compile")]
#[test]
fn native_compile_context_cache_ttl_eviction_prunes_stale_entries() {
    let context = NativeCompileContext {
        package_name: "demo".to_string(),
        crate_name: "demo".to_string(),
        workspace_mode_supported: false,
        cairo_project_dir: PathBuf::from("/tmp/demo/.uc/native-project"),
        corelib_src: PathBuf::from("/tmp/demo/corelib/src"),
        starknet_target: NativeStarknetTargetProps {
            sierra: true,
            casm: true,
        },
        manifest_content_hash: "manifest-blake3:demo".to_string(),
        external_non_starknet_dependencies: Vec::new(),
        path_dependency_roots: Vec::new(),
        crate_dependency_configs: Vec::new(),
    };
    let mut cache = HashMap::new();
    cache.insert(
        "stale".to_string(),
        NativeCompileContextCacheEntry {
            manifest_size_bytes: 1,
            manifest_modified_unix_ms: 1,
            manifest_change_unix_ms: Some(1),
            context: context.clone(),
            last_access_epoch_ms: 10,
            estimated_bytes: 1024,
        },
    );
    cache.insert(
        "fresh".to_string(),
        NativeCompileContextCacheEntry {
            manifest_size_bytes: 1,
            manifest_modified_unix_ms: 1,
            manifest_change_unix_ms: Some(1),
            context,
            last_access_epoch_ms: 120,
            estimated_bytes: 1024,
        },
    );

    evict_expired_native_compile_context_cache_entries(&mut cache, 150, 40);
    assert_eq!(cache.len(), 1);
    assert!(!cache.contains_key("stale"));
    assert!(cache.contains_key("fresh"));
}

#[cfg(feature = "native-compile")]
#[test]
fn native_refresh_telemetry_counters_accumulate_events() {
    let before = native_refresh_telemetry_snapshot();
    record_native_refresh_telemetry(NativeSessionRefreshAction::None, 0, 0);
    record_native_refresh_telemetry(NativeSessionRefreshAction::IncrementalChangedSet, 3, 1);
    record_native_refresh_telemetry(NativeSessionRefreshAction::FullRebuild, 2, 2);
    let after = native_refresh_telemetry_snapshot();

    assert!(after.0 > before.0, "none refresh counter should increase");
    assert!(
        after.1 > before.1,
        "incremental refresh counter should increase"
    );
    assert!(after.2 > before.2, "full rebuild counter should increase");
    assert!(
        after.3 >= before.3 + 5,
        "changed file counter should accumulate recorded deltas"
    );
    assert!(
        after.4 >= before.4 + 3,
        "removed file counter should accumulate recorded deltas"
    );
}

#[test]
fn native_fallback_telemetry_counters_accumulate_events() {
    let before = native_fallback_telemetry_snapshot();
    record_native_fallback(NativeFallbackReason::PreflightIneligible);
    record_native_fallback(NativeFallbackReason::LocalNativeError);
    record_native_fallback(NativeFallbackReason::DaemonNativeError);
    record_native_fallback(NativeFallbackReason::DaemonBackendDowngrade);
    let after = native_fallback_telemetry_snapshot();

    assert!(
        after.0 > before.0,
        "preflight fallback counter should increase"
    );
    assert!(
        after.1 > before.1,
        "local native fallback counter should increase"
    );
    assert!(
        after.2 > before.2,
        "daemon native fallback counter should increase"
    );
    assert!(
        after.3 > before.3,
        "daemon backend downgrade fallback counter should increase"
    );
}

#[test]
fn daemon_lock_state_reuses_cache_when_metadata_is_unchanged() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    daemon_lock_hash_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();

    let workspace = unique_test_dir("uc-daemon-lock-hash-cache");
    let manifest_path = workspace.join("Scarb.toml");
    let lock_path = workspace.join("Scarb.lock");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"
"#,
    )
    .expect("failed to write manifest");
    fs::write(&lock_path, "version = 1\n").expect("failed to write lock file");

    let (size_first, modified_first, hash_first) =
        daemon_lock_state(&manifest_path).expect("first lock state read should work");
    let (size_second, modified_second, hash_second) =
        daemon_lock_state(&manifest_path).expect("second lock state read should work");
    assert_eq!(size_first, size_second);
    assert_eq!(modified_first, modified_second);
    assert_eq!(hash_first, hash_second);

    let cache = daemon_lock_hash_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(cache.len(), 1, "lock hash cache should keep a single entry");
    let entry = cache
        .values()
        .next()
        .expect("expected one lock hash cache entry");
    assert_eq!(entry.lock_hash, hash_second);
    drop(cache);

    fs::remove_dir_all(&workspace).ok();
}

#[test]
fn daemon_lock_state_invalidates_cache_when_lockfile_changes() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    daemon_lock_hash_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();

    let workspace = unique_test_dir("uc-daemon-lock-hash-invalidate");
    let manifest_path = workspace.join("Scarb.toml");
    let lock_path = workspace.join("Scarb.lock");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"
"#,
    )
    .expect("failed to write manifest");
    fs::write(&lock_path, "state = \"aaaaaaaa\"\n").expect("failed to write lock file");

    let (_, _, hash_first) =
        daemon_lock_state(&manifest_path).expect("first lock state read should work");
    std::thread::sleep(std::time::Duration::from_millis(5));
    fs::write(&lock_path, "state = \"bbbbbbbb\"\n").expect("failed to mutate lock file");
    let (_, _, hash_second) =
        daemon_lock_state(&manifest_path).expect("second lock state read should work");
    assert_ne!(
        hash_first, hash_second,
        "lock hash cache must invalidate when lockfile content changes"
    );

    fs::remove_dir_all(&workspace).ok();
}

#[test]
fn daemon_lock_metadata_state_tracks_lockfile_changes_without_hash_cache() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    daemon_lock_hash_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();

    let workspace = unique_test_dir("uc-daemon-lock-metadata-state");
    let manifest_path = workspace.join("Scarb.toml");
    let lock_path = workspace.join("Scarb.lock");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"
"#,
    )
    .expect("failed to write manifest");
    fs::write(&lock_path, "state = \"aaaaaaaa\"\n").expect("failed to write lock file");

    let (_, _, key_first) = daemon_lock_metadata_state(&manifest_path)
        .expect("first lock metadata state read should work");
    std::thread::sleep(std::time::Duration::from_millis(5));
    fs::write(&lock_path, "state = \"bbbbbbbb\"\n").expect("failed to mutate lock file");
    let (_, _, key_second) = daemon_lock_metadata_state(&manifest_path)
        .expect("second lock metadata state read should work");
    assert_ne!(
        key_first, key_second,
        "lock metadata state key must change when Scarb.lock changes"
    );

    let hash_cache = daemon_lock_hash_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        hash_cache.is_empty(),
        "metadata-only lock state should not populate the semantic lock hash cache"
    );
    drop(hash_cache);

    fs::remove_dir_all(&workspace).ok();
}

#[test]
fn metadata_result_cache_key_changes_with_metadata_options() {
    let manifest_path = Path::new("/tmp/workspace/Scarb.toml");
    let scarb_version = "scarb 2.14.0 (cache-key-test)";
    let build_env = "env:fingerprint";

    let base = MetadataArgs {
        manifest_path: Some(manifest_path.to_path_buf()),
        format_version: 1,
        daemon_mode: DaemonModeArg::Off,
        offline: false,
        global_cache_dir: None,
        report_path: None,
    };
    let base_key = metadata_result_cache_key(&base, manifest_path, scarb_version, build_env);

    let mut offline = base.clone();
    offline.offline = true;
    let offline_key = metadata_result_cache_key(&offline, manifest_path, scarb_version, build_env);
    assert_eq!(
        base_key, offline_key,
        "metadata cache key should be shared between online/offline modes"
    );

    let mut format_v2 = base.clone();
    format_v2.format_version = 2;
    let format_v2_key =
        metadata_result_cache_key(&format_v2, manifest_path, scarb_version, build_env);
    assert_ne!(base_key, format_v2_key);

    let mut cache_dir = base.clone();
    cache_dir.global_cache_dir = Some(PathBuf::from("/tmp/cache-a"));
    let cache_dir_key =
        metadata_result_cache_key(&cache_dir, manifest_path, scarb_version, build_env);
    assert_ne!(base_key, cache_dir_key);
}

#[test]
fn metadata_result_cache_hit_ignores_lock_size_and_mtime_when_hash_matches() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    metadata_result_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    daemon_lock_hash_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();

    let workspace = unique_test_dir("uc-metadata-cache-lock-metadata-drift");
    let manifest_path = workspace.join("Scarb.toml");
    let lock_path = workspace.join("Scarb.lock");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"
"#,
    )
    .expect("failed to write manifest");
    fs::write(&lock_path, "version = 1\n").expect("failed to write lock file");

    let manifest_metadata = fs::metadata(&manifest_path).expect("failed to stat manifest");
    let manifest_size_bytes = manifest_metadata.len();
    let manifest_modified_unix_ms =
        metadata_modified_unix_ms(&manifest_metadata).expect("failed to read manifest mtime");
    let (_, _, lock_hash) =
        daemon_lock_state(&manifest_path).expect("failed to resolve lock state");

    let args = MetadataArgs {
        manifest_path: Some(manifest_path.clone()),
        format_version: 1,
        daemon_mode: DaemonModeArg::Off,
        offline: false,
        global_cache_dir: None,
        report_path: None,
    };
    let cache_key = metadata_result_cache_key(
        &args,
        &manifest_path,
        "scarb 2.14.0 (metadata-cache-test)",
        "env:fingerprint",
    );
    let cache_root = workspace.join(".uc/cache");
    let entry_path = metadata_cache_entry_path(&workspace, &cache_key);
    let run = CommandRun {
        command: vec!["scarb".to_string(), "metadata".to_string()],
        exit_code: 0,
        elapsed_ms: 42.0,
        stdout: "{\"packages\":[]}\n".to_string(),
        stderr: String::new(),
    };
    let write_context = MetadataResultCacheWriteContext {
        cache_key: &cache_key,
        cache_root: &cache_root,
        entry_path: &entry_path,
        manifest_size_bytes,
        manifest_modified_unix_ms,
        lock_hash: &lock_hash,
    };
    store_metadata_result_cache_entry(&write_context, &run)
        .expect("failed to store metadata cache entry");

    let hit = try_metadata_result_cache_hit(
        &cache_key,
        &entry_path,
        manifest_size_bytes,
        manifest_modified_unix_ms,
        &lock_hash,
    )
    .expect("cache lookup should succeed")
    .expect("cache entry should still hit when lock hash is unchanged");
    assert_eq!(hit.exit_code, 0);
    assert_eq!(hit.stdout, run.stdout);

    fs::remove_dir_all(&workspace).ok();
}

#[test]
fn metadata_result_cache_roundtrip_hits_when_inputs_match() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    metadata_result_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    daemon_lock_hash_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();

    let workspace = unique_test_dir("uc-metadata-cache-roundtrip");
    let manifest_path = workspace.join("Scarb.toml");
    let lock_path = workspace.join("Scarb.lock");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"
"#,
    )
    .expect("failed to write manifest");
    fs::write(&lock_path, "version = 1\n").expect("failed to write lock file");

    let manifest_metadata = fs::metadata(&manifest_path).expect("failed to stat manifest");
    let manifest_size_bytes = manifest_metadata.len();
    let manifest_modified_unix_ms =
        metadata_modified_unix_ms(&manifest_metadata).expect("failed to read manifest mtime");
    let (_, _, lock_hash) =
        daemon_lock_state(&manifest_path).expect("failed to resolve lock state");

    let args = MetadataArgs {
        manifest_path: Some(manifest_path.clone()),
        format_version: 1,
        daemon_mode: DaemonModeArg::Off,
        offline: true,
        global_cache_dir: Some(workspace.join(".scarb-cache")),
        report_path: None,
    };
    let cache_key = metadata_result_cache_key(
        &args,
        &manifest_path,
        "scarb 2.14.0 (metadata-cache-test)",
        "env:fingerprint",
    );
    let cache_root = workspace.join(".uc/cache");
    let entry_path = metadata_cache_entry_path(&workspace, &cache_key);
    let run = CommandRun {
        command: vec!["scarb".to_string(), "metadata".to_string()],
        exit_code: 0,
        elapsed_ms: 42.0,
        stdout: "{\"packages\":[]}\n".to_string(),
        stderr: String::new(),
    };

    let write_context = MetadataResultCacheWriteContext {
        cache_key: &cache_key,
        cache_root: &cache_root,
        entry_path: &entry_path,
        manifest_size_bytes,
        manifest_modified_unix_ms,
        lock_hash: &lock_hash,
    };
    store_metadata_result_cache_entry(&write_context, &run)
        .expect("failed to store metadata cache entry");
    assert!(entry_path.exists(), "cache entry should be persisted");

    let hit = try_metadata_result_cache_hit(
        &cache_key,
        &entry_path,
        manifest_size_bytes,
        manifest_modified_unix_ms,
        &lock_hash,
    )
    .expect("cache lookup should succeed")
    .expect("cache entry should hit");
    assert_eq!(hit.exit_code, 0);
    assert_eq!(hit.stdout, run.stdout);
    assert_eq!(hit.command, run.command);

    fs::remove_dir_all(&workspace).ok();
}

#[test]
fn metadata_result_cache_misses_when_lock_hash_changes() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    metadata_result_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    daemon_lock_hash_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();

    let workspace = unique_test_dir("uc-metadata-cache-lock-change");
    let manifest_path = workspace.join("Scarb.toml");
    let lock_path = workspace.join("Scarb.lock");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"
"#,
    )
    .expect("failed to write manifest");
    fs::write(&lock_path, "version = 1\n").expect("failed to write lock file");

    let manifest_metadata = fs::metadata(&manifest_path).expect("failed to stat manifest");
    let manifest_size_bytes = manifest_metadata.len();
    let manifest_modified_unix_ms =
        metadata_modified_unix_ms(&manifest_metadata).expect("failed to read manifest mtime");
    let (_, _, lock_hash) =
        daemon_lock_state(&manifest_path).expect("failed to resolve lock state");

    let args = MetadataArgs {
        manifest_path: Some(manifest_path.clone()),
        format_version: 1,
        daemon_mode: DaemonModeArg::Off,
        offline: false,
        global_cache_dir: None,
        report_path: None,
    };
    let cache_key = metadata_result_cache_key(
        &args,
        &manifest_path,
        "scarb 2.14.0 (metadata-cache-test)",
        "env:fingerprint",
    );
    let cache_root = workspace.join(".uc/cache");
    let entry_path = metadata_cache_entry_path(&workspace, &cache_key);
    let run = CommandRun {
        command: vec!["scarb".to_string(), "metadata".to_string()],
        exit_code: 0,
        elapsed_ms: 42.0,
        stdout: "{\"packages\":[]}\n".to_string(),
        stderr: String::new(),
    };
    let write_context = MetadataResultCacheWriteContext {
        cache_key: &cache_key,
        cache_root: &cache_root,
        entry_path: &entry_path,
        manifest_size_bytes,
        manifest_modified_unix_ms,
        lock_hash: &lock_hash,
    };
    store_metadata_result_cache_entry(&write_context, &run)
        .expect("failed to store metadata cache entry");

    std::thread::sleep(Duration::from_millis(5));
    fs::write(&lock_path, "version = 2\n").expect("failed to mutate lock file");
    let (_, _, new_lock_hash) =
        daemon_lock_state(&manifest_path).expect("failed to resolve mutated lock state");
    assert_ne!(lock_hash, new_lock_hash);

    let hit = try_metadata_result_cache_hit(
        &cache_key,
        &entry_path,
        manifest_size_bytes,
        manifest_modified_unix_ms,
        &new_lock_hash,
    )
    .expect("cache lookup should succeed");
    assert!(
        hit.is_none(),
        "cache must miss when lock hash changes for the same key"
    );

    fs::remove_dir_all(&workspace).ok();
}

#[test]
fn prepare_daemon_build_plan_reuses_when_inputs_unchanged_and_invalidates_on_lock_change() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::set_var("UC_SCARB_VERSION_LINE", "scarb 2.14.0 (plan-cache-test)");

    let workspace = unique_test_dir("uc-daemon-plan-cache");
    let manifest_path = workspace.join("Scarb.toml");
    let lock_path = workspace.join("Scarb.lock");
    let src_dir = workspace.join("src");
    fs::create_dir_all(&src_dir).expect("failed to create src dir");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"
"#,
    )
    .expect("failed to write manifest");
    fs::write(&lock_path, "version = 1\n").expect("failed to write lock file");
    fs::write(src_dir.join("lib.cairo"), "fn main() -> felt252 { 1 }\n")
        .expect("failed to write source file");

    let common = BuildCommonArgs {
        manifest_path: Some(manifest_path.clone()),
        package: None,
        workspace: false,
        features: vec!["feature_b".to_string(), "feature_a".to_string()],
        offline: false,
        release: false,
        profile: None,
    };

    let (plan_first, hit_first) =
        prepare_daemon_build_plan(&common, &manifest_path).expect("first plan build should work");
    let (plan_second, hit_second) =
        prepare_daemon_build_plan(&common, &manifest_path).expect("second plan build should work");

    assert!(!hit_first, "first plan build should be a miss");
    assert!(hit_second, "second plan build should reuse cached plan");
    assert_eq!(plan_first.session_key, plan_second.session_key);
    assert_eq!(
        plan_first.strict_invalidation_key, plan_second.strict_invalidation_key,
        "invalidation key should remain stable when inputs are unchanged"
    );

    fs::write(&lock_path, "version = 2\n[metadata]\nseed = \"changed\"\n")
        .expect("failed to mutate lock file");
    let (plan_third, hit_third) =
        prepare_daemon_build_plan(&common, &manifest_path).expect("third plan build should work");

    assert!(
        !hit_third,
        "lock changes must invalidate daemon build plan cache"
    );
    assert_ne!(
        plan_second.strict_invalidation_key, plan_third.strict_invalidation_key,
        "invalidation key should change when Scarb.lock changes"
    );

    std::env::remove_var("UC_SCARB_VERSION_LINE");
    fs::remove_dir_all(&workspace).ok();
}

#[test]
fn prepare_daemon_build_plan_reuses_for_equivalent_feature_sets() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::set_var(
        "UC_SCARB_VERSION_LINE",
        "scarb 2.14.0 (plan-cache-features)",
    );

    let workspace = unique_test_dir("uc-daemon-plan-cache-features");
    let manifest_path = workspace.join("Scarb.toml");
    let lock_path = workspace.join("Scarb.lock");
    let src_dir = workspace.join("src");
    fs::create_dir_all(&src_dir).expect("failed to create src dir");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"
"#,
    )
    .expect("failed to write manifest");
    fs::write(&lock_path, "version = 1\n").expect("failed to write lock file");
    fs::write(src_dir.join("lib.cairo"), "fn main() -> felt252 { 1 }\n")
        .expect("failed to write source file");

    let common_a = BuildCommonArgs {
        manifest_path: Some(manifest_path.clone()),
        package: None,
        workspace: false,
        features: vec!["feature_b".to_string(), "feature_a".to_string()],
        offline: false,
        release: false,
        profile: None,
    };
    let common_b = BuildCommonArgs {
        manifest_path: Some(manifest_path.clone()),
        package: None,
        workspace: false,
        features: vec![
            "feature_a".to_string(),
            "feature_b".to_string(),
            "feature_a".to_string(),
        ],
        offline: false,
        release: false,
        profile: None,
    };

    let (plan_first, hit_first) =
        prepare_daemon_build_plan(&common_a, &manifest_path).expect("first plan build should work");
    let (plan_second, hit_second) = prepare_daemon_build_plan(&common_b, &manifest_path)
        .expect("second plan build should reuse cache");

    assert!(!hit_first, "first plan build should miss");
    assert!(
        hit_second,
        "equivalent feature sets should reuse daemon plan cache entry"
    );
    assert_eq!(
        plan_first.session_key, plan_second.session_key,
        "session key should remain stable for equivalent feature sets"
    );

    std::env::remove_var("UC_SCARB_VERSION_LINE");
    fs::remove_dir_all(&workspace).ok();
}

#[test]
fn prepare_daemon_build_plan_invalidates_on_profile_env_and_toolchain_changes() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::set_var(
        "UC_SCARB_VERSION_LINE",
        "scarb 2.14.0 (plan-cache-toolchain-a)",
    );
    std::env::remove_var("SCARB_PLAN_CACHE_TEST_ENV");

    let workspace = unique_test_dir("uc-daemon-plan-cache-context");
    let manifest_path = workspace.join("Scarb.toml");
    let lock_path = workspace.join("Scarb.lock");
    let src_dir = workspace.join("src");
    fs::create_dir_all(&src_dir).expect("failed to create src dir");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"
"#,
    )
    .expect("failed to write manifest");
    fs::write(&lock_path, "version = 1\n").expect("failed to write lock file");
    fs::write(src_dir.join("lib.cairo"), "fn main() -> felt252 { 1 }\n")
        .expect("failed to write source file");

    let common_dev = BuildCommonArgs {
        manifest_path: Some(manifest_path.clone()),
        package: None,
        workspace: false,
        features: vec!["feature_a".to_string()],
        offline: false,
        release: false,
        profile: None,
    };
    let common_release = BuildCommonArgs {
        release: true,
        ..common_dev.clone()
    };

    let (dev_first, dev_first_hit) =
        prepare_daemon_build_plan(&common_dev, &manifest_path).expect("dev plan build should work");
    let (_, dev_second_hit) = prepare_daemon_build_plan(&common_dev, &manifest_path)
        .expect("second dev plan build should work");
    assert!(!dev_first_hit, "first dev request should miss");
    assert!(dev_second_hit, "second dev request should hit");

    let (release_plan, release_hit) = prepare_daemon_build_plan(&common_release, &manifest_path)
        .expect("release plan build should work");
    assert!(
        !release_hit,
        "changing profile/release must invalidate daemon plan cache"
    );
    assert_ne!(
        dev_first.session_key, release_plan.session_key,
        "session key must change between dev and release profiles"
    );

    std::env::set_var("SCARB_PLAN_CACHE_TEST_ENV", "v1");
    let (_, env_change_hit) = prepare_daemon_build_plan(&common_dev, &manifest_path)
        .expect("env-changed plan build should work");
    assert!(
        !env_change_hit,
        "build environment fingerprint changes must invalidate plan cache"
    );
    let (_, env_stable_hit) = prepare_daemon_build_plan(&common_dev, &manifest_path)
        .expect("env-stable plan build should work");
    assert!(env_stable_hit, "unchanged env fingerprint should hit");

    std::env::set_var(
        "UC_SCARB_VERSION_LINE",
        "scarb 2.14.0 (plan-cache-toolchain-b)",
    );
    let (_, toolchain_change_hit) = prepare_daemon_build_plan(&common_dev, &manifest_path)
        .expect("toolchain-changed plan build should work");
    assert!(
        !toolchain_change_hit,
        "scarb version changes must invalidate daemon plan cache"
    );

    std::env::remove_var("SCARB_PLAN_CACHE_TEST_ENV");
    std::env::remove_var("UC_SCARB_VERSION_LINE");
    fs::remove_dir_all(&workspace).ok();
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
fn cache_budget_enforcement_evicts_oldest_objects_before_metadata_files() {
    let dir = unique_test_dir("uc-cache-budget-evict");
    let cache_root = dir.join(".uc/cache");
    let objects_dir = cache_root.join("objects");
    let build_dir = cache_root.join("build");
    fs::create_dir_all(&objects_dir).expect("failed to create cache objects dir");
    fs::create_dir_all(&build_dir).expect("failed to create cache build dir");

    let old_object = objects_dir.join("aa/old.bin");
    let new_object = objects_dir.join("bb/new.bin");
    let build_entry = build_dir.join("entry.json");
    let lock_file = cache_root.join(".lock");

    fs::create_dir_all(
        old_object
            .parent()
            .expect("old object path should include parent"),
    )
    .expect("failed to create old object parent");
    fs::create_dir_all(
        new_object
            .parent()
            .expect("new object path should include parent"),
    )
    .expect("failed to create new object parent");

    fs::write(&old_object, vec![b'a'; 32]).expect("failed to write old object");
    thread::sleep(Duration::from_millis(15));
    fs::write(&new_object, vec![b'b'; 32]).expect("failed to write new object");
    thread::sleep(Duration::from_millis(15));
    fs::write(&build_entry, vec![b'c'; 32]).expect("failed to write build entry");
    fs::write(&lock_file, "pid=1234\n").expect("failed to write lock file");

    enforce_cache_size_budget_with_budget(&cache_root, 72).expect("cache eviction should succeed");

    assert!(
        !old_object.exists(),
        "oldest object should be evicted first when over budget"
    );
    assert!(
        new_object.exists(),
        "newer object should remain when one eviction is sufficient"
    );
    assert!(
        build_entry.exists(),
        "metadata/build files should remain when object eviction satisfies budget"
    );
    assert!(
        lock_file.exists(),
        "cache lock marker must never be removed by eviction"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn parse_metadata_format_version_accepts_supported_values() {
    assert_eq!(parse_metadata_format_version("1").unwrap(), 1);
    assert_eq!(parse_metadata_format_version("2").unwrap(), 2);
    assert!(parse_metadata_format_version("3").is_err());
}

#[test]
fn timeout_duration_from_secs_zero_disables_timeout() {
    assert_eq!(timeout_duration_from_secs(0), None);
    assert_eq!(timeout_duration_from_secs(9), Some(Duration::from_secs(9)));
}

#[test]
fn daemon_request_read_timeout_prefers_build_override_for_build_requests() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::set_var("UC_DAEMON_CLIENT_READ_TIMEOUT_SECS", "7");
    std::env::set_var("UC_DAEMON_BUILD_READ_TIMEOUT_SECS", "13");

    let build_request = DaemonRequest::Build {
        payload: DaemonBuildRequest {
            protocol_version: DAEMON_PROTOCOL_VERSION.to_string(),
            manifest_path: "/tmp/workspace/Scarb.toml".to_string(),
            package: None,
            workspace: false,
            features: Vec::new(),
            offline: true,
            release: false,
            profile: None,
            async_cache_persist: false,
            capture_output: true,
            compile_backend: DaemonBuildBackend::Scarb,
            native_fallback_to_scarb: false,
        },
    };

    assert_eq!(
        daemon_request_read_timeout(&DaemonRequest::Ping),
        Some(Duration::from_secs(7))
    );
    assert_eq!(
        daemon_request_read_timeout(&build_request),
        Some(Duration::from_secs(13))
    );

    std::env::remove_var("UC_DAEMON_CLIENT_READ_TIMEOUT_SECS");
    std::env::remove_var("UC_DAEMON_BUILD_READ_TIMEOUT_SECS");
}

#[test]
fn stale_lock_age_cleanup_never_removes_live_pid_lock() {
    let old_age = Duration::from_secs(CACHE_LOCK_STALE_AFTER_SECONDS + 60);
    assert!(!should_cleanup_stale_lock_by_age(true, old_age));
    assert!(!should_cleanup_stale_lock_by_age(
        false,
        Duration::from_secs(CACHE_LOCK_STALE_AFTER_SECONDS)
    ));
    assert!(should_cleanup_stale_lock_by_age(false, old_age));
}

#[test]
fn lock_file_has_pid_marker_detects_prefixed_lines() {
    assert!(lock_file_has_pid_marker("pid=1234\n"));
    assert!(lock_file_has_pid_marker("meta=1\n  pid=abc\n"));
    assert!(!lock_file_has_pid_marker("owner=1234\n"));
}

#[test]
fn session_input_cache_eviction_removes_oldest_entries() {
    let sample_input = SessionInput {
        compiler_version: "scarb 2.14.0".to_string(),
        profile: "dev".to_string(),
        offline: false,
        package: None,
        features: Vec::new(),
        cfg_set: Vec::new(),
        manifest_content_hash: "manifest-blake3:abc".to_string(),
        target_family: "workspace".to_string(),
        cairo_edition: None,
        cairo_lang_version: None,
        build_env_fingerprint: String::new(),
    };
    let mut cache = HashMap::new();
    cache.insert(
        "oldest".to_string(),
        SessionInputCacheEntry {
            manifest_size_bytes: 1,
            manifest_modified_unix_ms: 1,
            input: sample_input.clone(),
            last_access_epoch_ms: 1,
        },
    );
    cache.insert(
        "middle".to_string(),
        SessionInputCacheEntry {
            manifest_size_bytes: 1,
            manifest_modified_unix_ms: 1,
            input: sample_input.clone(),
            last_access_epoch_ms: 2,
        },
    );
    cache.insert(
        "newest".to_string(),
        SessionInputCacheEntry {
            manifest_size_bytes: 1,
            manifest_modified_unix_ms: 1,
            input: sample_input,
            last_access_epoch_ms: 3,
        },
    );

    evict_oldest_session_input_cache_entries(&mut cache, 2);
    assert_eq!(cache.len(), 2);
    assert!(!cache.contains_key("oldest"));
    assert!(cache.contains_key("middle"));
    assert!(cache.contains_key("newest"));
}

#[test]
fn fingerprint_index_cache_eviction_removes_oldest_entries() {
    let mut cache = HashMap::new();
    cache.insert(
        "oldest".to_string(),
        FingerprintIndexCacheEntry {
            index: FingerprintIndex::empty(),
            last_access_epoch_ms: 1,
        },
    );
    cache.insert(
        "middle".to_string(),
        FingerprintIndexCacheEntry {
            index: FingerprintIndex::empty(),
            last_access_epoch_ms: 2,
        },
    );
    cache.insert(
        "newest".to_string(),
        FingerprintIndexCacheEntry {
            index: FingerprintIndex::empty(),
            last_access_epoch_ms: 3,
        },
    );

    evict_oldest_fingerprint_index_cache_entries(&mut cache, 2);
    assert_eq!(cache.len(), 2);
    assert!(!cache.contains_key("oldest"));
    assert!(cache.contains_key("middle"));
    assert!(cache.contains_key("newest"));
}

#[test]
fn build_entry_cache_eviction_removes_oldest_entries() {
    let sample_entry = BuildCacheEntry {
        schema_version: BUILD_CACHE_SCHEMA_VERSION,
        fingerprint: "fp".to_string(),
        profile: "dev".to_string(),
        artifacts: Vec::new(),
    };
    let mut cache = HashMap::new();
    cache.insert(
        "oldest".to_string(),
        BuildEntryCacheEntry {
            file_size_bytes: 1,
            file_modified_unix_ms: 1,
            entry: sample_entry.clone(),
            last_access_epoch_ms: 1,
        },
    );
    cache.insert(
        "middle".to_string(),
        BuildEntryCacheEntry {
            file_size_bytes: 1,
            file_modified_unix_ms: 1,
            entry: sample_entry.clone(),
            last_access_epoch_ms: 2,
        },
    );
    cache.insert(
        "newest".to_string(),
        BuildEntryCacheEntry {
            file_size_bytes: 1,
            file_modified_unix_ms: 1,
            entry: sample_entry,
            last_access_epoch_ms: 3,
        },
    );

    evict_oldest_build_entry_cache_entries(&mut cache, 2);
    assert_eq!(cache.len(), 2);
    assert!(!cache.contains_key("oldest"));
    assert!(cache.contains_key("middle"));
    assert!(cache.contains_key("newest"));
}

#[test]
fn artifact_index_cache_eviction_removes_oldest_entries() {
    let mut cache = HashMap::new();
    cache.insert(
        "oldest".to_string(),
        ArtifactIndexCacheEntry {
            index: ArtifactIndex::empty(),
            last_access_epoch_ms: 1,
        },
    );
    cache.insert(
        "middle".to_string(),
        ArtifactIndexCacheEntry {
            index: ArtifactIndex::empty(),
            last_access_epoch_ms: 2,
        },
    );
    cache.insert(
        "newest".to_string(),
        ArtifactIndexCacheEntry {
            index: ArtifactIndex::empty(),
            last_access_epoch_ms: 3,
        },
    );

    evict_oldest_artifact_index_cache_entries(&mut cache, 2);
    assert_eq!(cache.len(), 2);
    assert!(!cache.contains_key("oldest"));
    assert!(cache.contains_key("middle"));
    assert!(cache.contains_key("newest"));
}

#[test]
fn cache_object_hash_memo_eviction_removes_oldest_entries() {
    let mut cache = HashMap::new();
    cache.insert(
        "oldest".to_string(),
        CacheObjectHashMemoEntry {
            size_bytes: 1,
            modified_unix_ms: 1,
            blake3_hex: "a".repeat(64),
            last_access_epoch_ms: 1,
        },
    );
    cache.insert(
        "middle".to_string(),
        CacheObjectHashMemoEntry {
            size_bytes: 1,
            modified_unix_ms: 1,
            blake3_hex: "b".repeat(64),
            last_access_epoch_ms: 2,
        },
    );
    cache.insert(
        "newest".to_string(),
        CacheObjectHashMemoEntry {
            size_bytes: 1,
            modified_unix_ms: 1,
            blake3_hex: "c".repeat(64),
            last_access_epoch_ms: 3,
        },
    );

    evict_oldest_cache_object_hash_memo_entries(&mut cache, 2);
    assert_eq!(cache.len(), 2);
    assert!(!cache.contains_key("oldest"));
    assert!(cache.contains_key("middle"));
    assert!(cache.contains_key("newest"));
}

#[test]
fn load_cache_entry_cached_invalidates_when_file_changes() {
    let dir = unique_test_dir("uc-build-entry-cache-invalidate");
    let entry_path = dir.join("cache/build/test.json");
    if let Some(parent) = entry_path.parent() {
        fs::create_dir_all(parent).expect("failed to create cache entry parent");
    }
    let first = BuildCacheEntry {
        schema_version: BUILD_CACHE_SCHEMA_VERSION,
        fingerprint: "fp-a".to_string(),
        profile: "dev".to_string(),
        artifacts: Vec::new(),
    };
    persist_cache_entry(
        &first.profile,
        &first.fingerprint,
        &first.artifacts,
        &entry_path,
    )
    .expect("failed to persist first cache entry");
    let loaded_first = load_cache_entry_cached(&entry_path)
        .expect("failed to load first cache entry")
        .expect("first cache entry should exist");
    assert_eq!(loaded_first.fingerprint, "fp-a");

    // Ensure file mtime changes for the metadata-based cache validator.
    thread::sleep(Duration::from_millis(5));
    let second = BuildCacheEntry {
        schema_version: BUILD_CACHE_SCHEMA_VERSION,
        fingerprint: "fp-b".to_string(),
        profile: "dev".to_string(),
        artifacts: Vec::new(),
    };
    persist_cache_entry(
        &second.profile,
        &second.fingerprint,
        &second.artifacts,
        &entry_path,
    )
    .expect("failed to persist second cache entry");
    let loaded_second = load_cache_entry_cached(&entry_path)
        .expect("failed to load second cache entry")
        .expect("second cache entry should exist");
    assert_eq!(loaded_second.fingerprint, "fp-b");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn cache_object_matches_expected_recomputes_when_object_changes() {
    let dir = unique_test_dir("uc-cache-object-hash-memo-refresh");
    let object = dir.join("objects/aa/object.bin");
    fs::create_dir_all(
        object
            .parent()
            .expect("cache object path should include parent directory"),
    )
    .expect("failed to create object parent directory");
    fs::write(&object, b"first-bytes").expect("failed to write initial object bytes");
    let metadata = fs::metadata(&object).expect("failed to stat initial object");
    let expected_size = metadata.len();
    let first_hash = hash_file_blake3(&object).expect("failed to hash initial object bytes");
    assert!(
        cache_object_matches_expected(&object, &first_hash, expected_size)
            .expect("expected object should match on first check"),
        "initial object hash should match"
    );

    thread::sleep(Duration::from_millis(10));
    fs::write(&object, b"secondbytes").expect("failed to rewrite object bytes");
    let second_hash = hash_file_blake3(&object).expect("failed to hash updated object bytes");
    assert_ne!(
        first_hash, second_hash,
        "sanity: updated object content should produce a different hash"
    );
    assert!(
        !cache_object_matches_expected(&object, &first_hash, expected_size)
            .expect("stale hash should fail after object rewrite"),
        "stale object hash should be rejected"
    );
    assert!(
        cache_object_matches_expected(&object, &second_hash, expected_size)
            .expect("updated hash should match after object rewrite"),
        "updated object hash should match"
    );

    fs::remove_dir_all(&dir).ok();
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
fn cached_artifacts_already_materialized_returns_true_when_index_matches() {
    let dir = unique_test_dir("uc-materialized-index-hit");
    let workspace = dir.join("workspace");
    let target_root = workspace.join("target/dev");
    let cache_root = workspace.join(".uc/cache");
    fs::create_dir_all(&target_root).expect("failed to create target root");

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

    let materialized =
        cached_artifacts_already_materialized(&workspace, "dev", &cache_root, &artifacts)
            .expect("materialized check should succeed");
    assert!(
        materialized,
        "index + target metadata should satisfy hot cache-hit check"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn restore_cached_artifacts_skips_large_existing_hash_fallback() {
    let dir = unique_test_dir("uc-restore-large-hash-guard");
    let workspace = dir.join("workspace");
    let target_root = workspace.join("target/dev");
    let cache_root = workspace.join(".uc/cache");
    let objects_dir = cache_root.join("objects");
    fs::create_dir_all(&target_root).expect("failed to create target root");
    fs::create_dir_all(&objects_dir).expect("failed to create objects root");

    let output = target_root.join("demo.sierra.json");
    let large_len = DEFAULT_MAX_RESTORE_EXISTING_HASH_BYTES as usize + 4096;
    fs::write(&output, vec![b'X'; large_len]).expect("failed to write large artifact");
    let output_metadata = fs::metadata(&output).expect("failed to stat large artifact");
    let expected_hash = hash_file_blake3(&output).expect("failed to hash large artifact");

    let artifacts = vec![CachedArtifact {
        relative_path: "demo.sierra.json".to_string(),
        blake3_hex: expected_hash,
        size_bytes: output_metadata.len(),
        object_rel_path: "aa/missing-object.bin".to_string(),
    }];

    let restored =
        restore_cached_artifacts(&workspace, "dev", &cache_root, &objects_dir, &artifacts)
            .expect("restore should return result");
    assert!(
        !restored,
        "large artifacts should avoid hash fallback and miss when object is absent"
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
fn restore_cached_artifacts_returns_false_when_cache_object_hash_mismatch() {
    let dir = unique_test_dir("uc-restore-object-hash-mismatch");
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
    let object_len = fs::metadata(&object)
        .expect("failed to stat cache object")
        .len();
    // Keep same size as the valid object to ensure integrity guard verifies hash, not only size.
    fs::write(&object, vec![b'X'; object_len as usize]).expect("failed to corrupt cache object");
    assert_eq!(
        fs::metadata(&object)
            .expect("failed to stat corrupted cache object")
            .len(),
        object_len
    );

    let artifacts = vec![CachedArtifact {
        relative_path: "demo.sierra.json".to_string(),
        blake3_hex: expected_hash,
        size_bytes: object_len,
        object_rel_path: "aa/fresh-object.bin".to_string(),
    }];

    let restored = restore_cached_artifacts(
        &workspace,
        "dev",
        &cache_root,
        &cache_root.join("objects"),
        &artifacts,
    )
    .expect("restore should return result");
    assert!(
        !restored,
        "corrupted cache object should force cache miss recovery path"
    );
    assert!(
        !object.exists(),
        "corrupted cache object should be evicted from cache"
    );
    assert_eq!(
        fs::read(&output).expect("failed to read target after cache miss"),
        b"stale-artifact"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn collect_cached_artifacts_for_entry_rebuilds_corrupted_existing_object() {
    let dir = unique_test_dir("uc-collect-repair-object");
    let workspace = dir.join("workspace");
    let target_root = workspace.join("target/dev");
    let cache_root = workspace.join(".uc/cache");
    let objects_root = cache_root.join("objects");
    fs::create_dir_all(&target_root).expect("failed to create target root");
    fs::create_dir_all(&objects_root).expect("failed to create object root");

    let artifact_path = target_root.join("demo.sierra.json");
    fs::write(&artifact_path, b"fresh-artifact").expect("failed to write source artifact");
    let expected_hash = hash_file_blake3(&artifact_path).expect("failed to hash source artifact");
    let object_path = objects_root.join(format!("{}/{}.bin", &expected_hash[0..2], expected_hash));
    fs::create_dir_all(
        object_path
            .parent()
            .expect("object path should have parent directory"),
    )
    .expect("failed to create object subdir");
    fs::write(&object_path, b"broken-artifact").expect("failed to write corrupted cache object");

    let cached = collect_cached_artifacts_for_entry(&workspace, "dev", &cache_root, &objects_root)
        .expect("failed to collect cached artifacts");
    assert_eq!(cached.len(), 1, "expected one cached artifact");
    assert_eq!(
        fs::read(&object_path).expect("failed to read repaired cache object"),
        b"fresh-artifact"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn collect_cached_artifacts_for_entry_with_paths_limits_scan_scope() {
    let dir = unique_test_dir("uc-collect-with-path-filter");
    let workspace = dir.join("workspace");
    let target_root = workspace.join("target/dev");
    let cache_root = workspace.join(".uc/cache");
    let objects_root = cache_root.join("objects");
    fs::create_dir_all(&target_root).expect("failed to create target root");
    fs::create_dir_all(&objects_root).expect("failed to create object root");

    let selected = target_root.join("demo.contract_class.json");
    let unselected = target_root.join("ignore.compiled_contract_class.json");
    fs::write(&selected, b"selected-artifact").expect("failed to write selected artifact");
    fs::write(&unselected, b"unselected-artifact").expect("failed to write unselected artifact");

    let filtered = vec!["demo.contract_class.json".to_string()];
    let cached = collect_cached_artifacts_for_entry_with_paths(
        &workspace,
        "dev",
        &cache_root,
        &objects_root,
        Some(&filtered),
    )
    .expect("failed to collect filtered cached artifacts");

    assert_eq!(cached.len(), 1, "expected one cached artifact");
    assert_eq!(cached[0].relative_path, "demo.contract_class.json");

    let selected_hash = hash_file_blake3(&selected).expect("failed to hash selected artifact");
    let unselected_hash =
        hash_file_blake3(&unselected).expect("failed to hash unselected artifact");
    let selected_object =
        objects_root.join(format!("{}/{}.bin", &selected_hash[0..2], selected_hash));
    let unselected_object = objects_root.join(format!(
        "{}/{}.bin",
        &unselected_hash[0..2],
        unselected_hash
    ));
    assert!(
        selected_object.exists(),
        "selected artifact object should be materialized in cache"
    );
    assert!(
        !unselected_object.exists(),
        "unselected artifact should not be persisted when filtered paths are provided"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn async_persist_worker_uses_precomputed_cached_artifacts() {
    let dir = unique_test_dir("uc-async-persist-precomputed");
    let workspace = dir.join("workspace");
    let cache_root = workspace.join(".uc/cache");
    let objects_root = cache_root.join("objects");
    let entry_path = cache_root.join("build/session.json");
    fs::create_dir_all(&objects_root).expect("failed to create objects root");
    let object_rel_path = "aa/object.bin".to_string();
    let object_path = objects_root.join(&object_rel_path);
    fs::create_dir_all(
        object_path
            .parent()
            .expect("object path should have parent directory"),
    )
    .expect("failed to create object parent");
    fs::write(&object_path, b"artifact-object").expect("failed to write object bytes");
    let object_hash = hash_file_blake3(&object_path).expect("failed to hash object");
    let object_len = fs::metadata(&object_path)
        .expect("failed to stat object")
        .len();

    let artifact = CachedArtifact {
        relative_path: "demo.sierra.json".to_string(),
        blake3_hex: object_hash,
        size_bytes: object_len,
        object_rel_path,
    };
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let worker = thread::spawn(move || run_async_persist_worker(receiver));
    sender
        .send(AsyncPersistTask {
            scope_key: "scope".to_string(),
            workspace_root: workspace,
            profile: "dev".to_string(),
            fingerprint: "fingerprint".to_string(),
            artifact_relative_paths: None,
            cached_artifacts: Some(vec![artifact]),
            cache_root: cache_root.clone(),
            objects_dir: objects_root,
            entry_path: entry_path.clone(),
        })
        .expect("failed to enqueue async persist task");
    drop(sender);
    worker
        .join()
        .expect("async persist worker should exit cleanly");

    let entry = load_cache_entry(&entry_path)
        .expect("failed to load async persisted cache entry")
        .expect("cache entry should be written by async worker");
    assert_eq!(entry.artifacts.len(), 1);
    assert_eq!(entry.artifacts[0].relative_path, "demo.sierra.json");

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn daemon_shared_cache_root_is_workspace_scoped_and_stable() {
    let root_a_1 = daemon_shared_cache_root(Path::new("/tmp/workspace-a"));
    let root_a_2 = daemon_shared_cache_root(Path::new("/tmp/workspace-a"));
    let root_b = daemon_shared_cache_root(Path::new("/tmp/workspace-b"));
    assert_eq!(root_a_1, root_a_2);
    assert_ne!(root_a_1, root_b);
}

#[test]
fn run_build_with_uc_cache_restores_from_daemon_shared_cache_when_local_cache_is_missing() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::set_var(
        "UC_SCARB_VERSION_LINE",
        "scarb 2.14.0 (daemon-shared-cache-test)",
    );

    let dir = unique_test_dir("uc-daemon-shared-cache-restore");
    let shared_cache_dir = dir.join("daemon-shared-cache");
    std::env::set_var("UC_DAEMON_SHARED_CACHE_DIR", &shared_cache_dir);
    let workspace = dir.join("workspace");
    let src_dir = workspace.join("src");
    fs::create_dir_all(&src_dir).expect("failed to create src directory");
    let manifest_path = workspace.join("Scarb.toml");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"
"#,
    )
    .expect("failed to write manifest");
    fs::write(workspace.join("Scarb.lock"), "version = 1\n").expect("failed to write lock file");
    fs::write(src_dir.join("lib.cairo"), "fn main() -> felt252 { 1 }\n")
        .expect("failed to write source file");

    let common = BuildCommonArgs {
        manifest_path: Some(manifest_path.clone()),
        package: None,
        workspace: false,
        features: Vec::new(),
        offline: false,
        release: false,
        profile: None,
    };
    let profile = "dev";
    let workspace_root = manifest_path
        .parent()
        .expect("manifest must have parent")
        .to_path_buf();
    let session_key = build_session_input(&common, &manifest_path, profile)
        .expect("failed to build session input")
        .deterministic_key_hex();
    let fingerprint = compute_build_fingerprint(
        &workspace_root,
        &manifest_path,
        &common,
        profile,
        Some(&workspace_root.join(".uc/cache")),
    )
    .expect("failed to compute fingerprint");

    let target_root = workspace_root.join("target").join(profile);
    fs::create_dir_all(&target_root).expect("failed to create target root");
    let artifact_rel = "demo.sierra.json";
    let artifact_path = target_root.join(artifact_rel);
    let artifact_bytes = b"{\"artifact\":\"daemon-shared\"}";
    fs::write(&artifact_path, artifact_bytes).expect("failed to write artifact");
    let artifact_hash = hash_file_blake3(&artifact_path).expect("failed to hash artifact");
    let artifact_size = fs::metadata(&artifact_path)
        .expect("failed to stat artifact")
        .len();

    let shared_cache_root = daemon_shared_cache_root(&workspace_root);
    let shared_objects_dir = shared_cache_root.join("objects");
    let object_rel_path = format!("{}/{}.bin", &artifact_hash[0..2], artifact_hash);
    let shared_object_path = shared_objects_dir.join(&object_rel_path);
    fs::create_dir_all(
        shared_object_path
            .parent()
            .expect("shared object should have parent"),
    )
    .expect("failed to create shared object parent");
    persist_artifact_object(&artifact_path, &shared_object_path)
        .expect("failed to persist shared object");

    let shared_entry_path = daemon_shared_cache_entry_path(&shared_cache_root, &session_key);
    persist_cache_entry(
        profile,
        &fingerprint,
        &[CachedArtifact {
            relative_path: artifact_rel.to_string(),
            blake3_hex: artifact_hash.clone(),
            size_bytes: artifact_size,
            object_rel_path: object_rel_path.clone(),
        }],
        &shared_entry_path,
    )
    .expect("failed to persist shared cache entry");

    fs::remove_dir_all(workspace_root.join("target")).expect("failed to remove target directory");
    fs::remove_dir_all(workspace_root.join(".uc")).ok();
    assert!(
        !artifact_path.exists(),
        "artifact must be removed before shared-restore test"
    );

    let (run, cache_hit, returned_fingerprint, telemetry) = run_build_with_uc_cache(
        &common,
        BuildCacheRunContext {
            manifest_path: &manifest_path,
            workspace_root: &workspace_root,
            profile,
            session_key: &session_key,
            compiler_version: &scarb_version_line().expect("failed to resolve compiler version"),
            compile_backend: BuildCompileBackend::Scarb,
            options: BuildRunOptions {
                capture_output: false,
                inherit_output_when_uncaptured: true,
                async_cache_persist: false,
                use_daemon_shared_cache: true,
            },
        },
    )
    .expect("shared cache restore should succeed");

    assert!(cache_hit, "daemon shared cache should provide cache hit");
    assert_eq!(returned_fingerprint, fingerprint);
    assert_eq!(
        fs::read(&artifact_path).expect("failed to read restored artifact"),
        artifact_bytes
    );
    assert!(
        run.command.iter().any(|arg| arg == "--daemon-shared-cache"),
        "cache-hit command marker should indicate daemon shared cache path"
    );
    assert_eq!(telemetry.compile_ms, 0.0);

    std::env::remove_var("UC_SCARB_VERSION_LINE");
    std::env::remove_var("UC_DAEMON_SHARED_CACHE_DIR");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn try_local_uc_cache_hit_skips_fingerprint_when_entry_is_missing() {
    let dir = unique_test_dir("uc-local-cache-probe-missing-entry");
    let workspace = dir.join("workspace");
    fs::create_dir_all(&workspace).expect("failed to create workspace");
    let manifest_path = workspace.join("Scarb.toml");
    let common = BuildCommonArgs {
        manifest_path: Some(manifest_path.clone()),
        package: None,
        workspace: false,
        features: Vec::new(),
        offline: false,
        release: false,
        profile: None,
    };
    let session_key = "a".repeat(SESSION_KEY_LEN);
    let probe = try_local_uc_cache_hit(
        &common,
        &manifest_path,
        &workspace,
        "dev",
        &session_key,
        "scarb 2.14.0 (local-cache-probe-test)",
    )
    .expect("missing cache entry should return None without fingerprint failure");
    assert!(
        probe.is_none(),
        "missing local cache entry should produce a clean cache miss"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn run_build_with_uc_cache_defers_fingerprint_when_entry_is_missing() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !scarb_available() {
        return;
    }
    let dir = unique_test_dir("uc-run-build-deferred-fingerprint");
    let workspace = dir.join("workspace");
    fs::create_dir_all(&workspace).expect("failed to create workspace");
    let manifest_path = workspace.join("Scarb.toml");
    let common = BuildCommonArgs {
        manifest_path: Some(manifest_path.clone()),
        package: None,
        workspace: false,
        features: Vec::new(),
        offline: false,
        release: false,
        profile: None,
    };
    let session_key = "b".repeat(SESSION_KEY_LEN);
    let compiler_version = scarb_version_line().expect("failed to resolve scarb version");
    let (run, cache_hit, fingerprint, telemetry) = run_build_with_uc_cache(
        &common,
        BuildCacheRunContext {
            manifest_path: &manifest_path,
            workspace_root: &workspace,
            profile: "dev",
            session_key: &session_key,
            compiler_version: &compiler_version,
            compile_backend: BuildCompileBackend::Scarb,
            options: BuildRunOptions {
                capture_output: true,
                inherit_output_when_uncaptured: true,
                async_cache_persist: false,
                use_daemon_shared_cache: false,
            },
        },
    )
    .expect("compile miss path should return command result, not fingerprint error");
    assert!(
        !cache_hit,
        "missing local cache entry with missing manifest must remain a cache miss"
    );
    assert_ne!(
        run.exit_code, 0,
        "missing manifest should fail the compile command"
    );
    assert!(
        fingerprint.is_empty(),
        "compile failures should not force fingerprint computation on startup miss path"
    );
    assert_eq!(
        telemetry.fingerprint_ms, 0.0,
        "deferred startup path should avoid fingerprint work when compile fails"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn run_build_with_uc_cache_defers_fingerprint_when_shared_entry_is_missing() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !scarb_available() {
        return;
    }
    let dir = unique_test_dir("uc-run-build-deferred-shared-fingerprint");
    let workspace = dir.join("workspace");
    let shared_cache_dir = dir.join("daemon-shared-cache");
    fs::create_dir_all(&workspace).expect("failed to create workspace");
    std::env::set_var("UC_DAEMON_SHARED_CACHE_DIR", &shared_cache_dir);
    let manifest_path = workspace.join("Scarb.toml");
    let common = BuildCommonArgs {
        manifest_path: Some(manifest_path.clone()),
        package: None,
        workspace: false,
        features: Vec::new(),
        offline: false,
        release: false,
        profile: None,
    };
    let session_key = "c".repeat(SESSION_KEY_LEN);
    let compiler_version = scarb_version_line().expect("failed to resolve scarb version");
    let (run, cache_hit, fingerprint, telemetry) = run_build_with_uc_cache(
        &common,
        BuildCacheRunContext {
            manifest_path: &manifest_path,
            workspace_root: &workspace,
            profile: "dev",
            session_key: &session_key,
            compiler_version: &compiler_version,
            compile_backend: BuildCompileBackend::Scarb,
            options: BuildRunOptions {
                capture_output: true,
                inherit_output_when_uncaptured: true,
                async_cache_persist: false,
                use_daemon_shared_cache: true,
            },
        },
    )
    .expect("compile miss path should return command result, not fingerprint error");
    assert!(
        !cache_hit,
        "missing shared cache entry with missing manifest must remain a cache miss"
    );
    assert_ne!(
        run.exit_code, 0,
        "missing manifest should fail the compile command"
    );
    assert!(
        fingerprint.is_empty(),
        "shared-cache probe should not force fingerprint work when shared entry is absent"
    );
    assert_eq!(
        telemetry.fingerprint_ms, 0.0,
        "shared-cache miss without entry should avoid fingerprint computation"
    );

    std::env::remove_var("UC_DAEMON_SHARED_CACHE_DIR");
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
fn collect_native_dependency_surface_collects_external_root_and_target_dependencies() {
    let manifest: TomlValue = toml::from_str(
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"

[dependencies]
starknet = "2.7.0"
alexandria = "0.9.0"

[target.'cfg(target_os = "linux")'.dependencies]
dojo = "1.2.3"
"#,
    )
    .expect("manifest should parse");
    let manifest_path = PathBuf::from("/tmp/uc-dependency-surface-root/Scarb.toml");
    let workspace_root = PathBuf::from("/tmp/uc-dependency-surface-root");
    let surface = collect_native_dependency_surface(&manifest, &manifest_path, &workspace_root);
    assert!(surface.path_dependency_roots.is_empty());
    assert_eq!(
        surface.external_non_starknet_dependencies,
        vec![
            "[dependencies].alexandria".to_string(),
            "[target.cfg(target_os = \"linux\").dependencies].dojo".to_string(),
        ]
    );
    assert!(
        surface
            .crate_dependency_configs
            .iter()
            .any(|config| config.crate_name == "demo" && config.dependencies.is_empty()),
        "root crate dependency config should be present even without local path dependencies"
    );
}

#[test]
fn collect_native_dependency_surface_collects_local_path_dependency_roots() {
    let dir = unique_test_dir("uc-native-dependency-surface-path-roots");
    let manifest_path = dir.join("Scarb.toml");
    let local_dep_src = dir.join("deps/local-dep/src");
    let shared_dep_src = dir.join("deps/shared-dep/src");
    fs::create_dir_all(&local_dep_src).expect("failed to create local dependency src");
    fs::create_dir_all(&shared_dep_src).expect("failed to create shared dependency src");
    fs::write(local_dep_src.join("lib.cairo"), "fn local() {}\n")
        .expect("failed to write local dependency lib.cairo");
    fs::write(shared_dep_src.join("lib.cairo"), "fn shared() {}\n")
        .expect("failed to write shared dependency lib.cairo");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"

[workspace.dependencies]
shared-dep = { path = "deps/shared-dep" }

[dependencies]
starknet = "2.7.0"
local-dep = { path = "deps/local-dep" }
shared-dep = { workspace = true }
alexandria = "0.9.0"
"#,
    )
    .expect("failed to write manifest");
    let manifest_text = fs::read_to_string(&manifest_path).expect("failed to read manifest");
    let manifest: TomlValue = toml::from_str(&manifest_text).expect("manifest should parse");
    let surface = collect_native_dependency_surface(&manifest, &manifest_path, &dir);

    assert_eq!(
        surface.external_non_starknet_dependencies,
        vec!["[dependencies].alexandria".to_string()],
        "only non-path non-starknet deps should remain external",
    );
    assert_eq!(
        surface.path_dependency_roots,
        vec![
            NativePathDependencyRoot {
                crate_name: "local_dep".to_string(),
                source_root: local_dep_src,
            },
            NativePathDependencyRoot {
                crate_name: "shared_dep".to_string(),
                source_root: shared_dep_src,
            },
        ],
        "path dependencies should be tracked as native crate roots",
    );
    assert!(
        surface.crate_dependency_configs.iter().any(|config| {
            config.crate_name == "demo"
                && config.dependencies
                    == vec!["local_dep".to_string(), "shared_dep".to_string()]
        }),
        "root crate should track direct local path dependencies for cairo_project dependency wiring"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn collect_native_dependency_surface_resolves_workspace_paths_from_workspace_root() {
    let dir = unique_test_dir("uc-native-dependency-surface-workspace-path-root");
    let package_dir = dir.join("packages/app");
    let manifest_path = package_dir.join("Scarb.toml");
    let shared_dep_src = dir.join("deps/shared-dep/src");
    fs::create_dir_all(&shared_dep_src).expect("failed to create shared dependency src");
    fs::create_dir_all(&package_dir).expect("failed to create package directory");
    fs::write(shared_dep_src.join("lib.cairo"), "fn shared() {}\n")
        .expect("failed to write shared dependency lib.cairo");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"

[workspace.dependencies]
shared-dep = { path = "deps/shared-dep" }

[dependencies]
shared-dep = { workspace = true }
"#,
    )
    .expect("failed to write package manifest");
    let manifest_text = fs::read_to_string(&manifest_path).expect("failed to read manifest");
    let manifest: TomlValue = toml::from_str(&manifest_text).expect("manifest should parse");
    let surface = collect_native_dependency_surface(&manifest, &manifest_path, &dir);

    assert!(
        surface.external_non_starknet_dependencies.is_empty(),
        "workspace path dependency should not be external"
    );
    assert_eq!(
        surface.path_dependency_roots,
        vec![NativePathDependencyRoot {
            crate_name: "shared_dep".to_string(),
            source_root: shared_dep_src,
        }],
        "workspace path dependencies should resolve from workspace root, not package manifest directory",
    );
    assert!(
        surface.crate_dependency_configs.iter().any(|config| {
            config.crate_name == "demo" && config.dependencies == vec!["shared_dep".to_string()]
        }),
        "root crate should wire workspace path dependency as a local Cairo dependency"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn collect_native_dependency_surface_tracks_transitive_local_dependencies_from_path_manifests() {
    let dir = unique_test_dir("uc-native-dependency-surface-transitive-path-deps");
    let manifest_path = dir.join("Scarb.toml");
    let local_dep_src = dir.join("deps/local-dep/src");
    let shared_dep_src = dir.join("deps/shared-dep/src");
    fs::create_dir_all(dir.join("src")).expect("failed to create root src");
    fs::create_dir_all(&local_dep_src).expect("failed to create local dependency src");
    fs::create_dir_all(&shared_dep_src).expect("failed to create shared dependency src");
    fs::write(dir.join("src/lib.cairo"), "fn root() {}\n").expect("failed to write root lib.cairo");
    fs::write(local_dep_src.join("lib.cairo"), "fn local() {}\n")
        .expect("failed to write local dependency lib.cairo");
    fs::write(shared_dep_src.join("lib.cairo"), "fn shared() {}\n")
        .expect("failed to write shared dependency lib.cairo");
    fs::write(
        local_dep_src
            .parent()
            .expect("local dependency source root should have a parent")
            .join("Scarb.toml"),
        r#"[package]
name = "local-dep"
version = "0.1.0"
edition = "2024_07"

[dependencies]
shared-dep = { workspace = true }
"#,
    )
    .expect("failed to write local dependency manifest");
    fs::write(
        shared_dep_src
            .parent()
            .expect("shared dependency source root should have a parent")
            .join("Scarb.toml"),
        r#"[package]
name = "shared-dep"
version = "0.1.0"
edition = "2024_07"
"#,
    )
    .expect("failed to write shared dependency manifest");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"

[workspace.dependencies]
shared-dep = { path = "deps/shared-dep" }

[dependencies]
local-dep = { path = "deps/local-dep" }
"#,
    )
    .expect("failed to write root manifest");

    let manifest_text = fs::read_to_string(&manifest_path).expect("failed to read root manifest");
    let manifest: TomlValue = toml::from_str(&manifest_text).expect("manifest should parse");
    let surface = collect_native_dependency_surface(&manifest, &manifest_path, &dir);

    assert!(
        surface.external_non_starknet_dependencies.is_empty(),
        "transitive workspace path dependencies should remain local-only"
    );
    assert_eq!(
        surface.path_dependency_roots,
        vec![
            NativePathDependencyRoot {
                crate_name: "local_dep".to_string(),
                source_root: local_dep_src,
            },
            NativePathDependencyRoot {
                crate_name: "shared_dep".to_string(),
                source_root: shared_dep_src,
            },
        ],
        "transitive local dependencies should extend crate roots beyond direct root manifest entries",
    );
    assert!(
        surface.crate_dependency_configs.iter().any(|config| {
            config.crate_name == "local_dep"
                && config.dependencies == vec!["shared_dep".to_string()]
        }),
        "path dependency manifests should wire their own local dependency edges into cairo_project overrides"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn native_target_dir_rejects_profile_traversal_escape() {
    let workspace_root = PathBuf::from("/tmp/uc-native-target-dir");
    let err = native_target_dir(&workspace_root, "../../escape")
        .expect_err("profile traversal should be rejected");
    let message = format!("{err:#}");
    assert!(
        message.contains("native build profile contains invalid path component"),
        "unexpected error: {message}"
    );
}

#[test]
fn native_target_dir_rejects_profile_nul_byte() {
    let workspace_root = PathBuf::from("/tmp/uc-native-target-dir");
    let err = native_target_dir(&workspace_root, "dev\0etc")
        .expect_err("profile containing NUL should be rejected");
    let message = format!("{err:#}");
    assert!(
        message.contains("native build profile must not contain NUL bytes"),
        "unexpected error: {message}"
    );
}

#[test]
fn build_native_compile_context_writes_cairo_project_and_normalizes_crate_name() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = unique_test_dir("uc-native-context-project");
    let manifest_path = dir.join("Scarb.toml");
    fs::create_dir_all(dir.join("src")).expect("failed to create src directory");
    fs::write(dir.join("src/lib.cairo"), "fn main() {}\n").expect("failed to write lib.cairo");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo-native"
version = "0.1.0"
edition = "2024_07"
"#,
    )
    .expect("failed to write manifest");

    let fake_corelib_src = dir.join("toolchain/corelib/src");
    create_mock_native_corelib(&fake_corelib_src);
    std::env::set_var("UC_NATIVE_CORELIB_SRC", &fake_corelib_src);

    let common = BuildCommonArgs {
        manifest_path: Some(manifest_path.clone()),
        package: None,
        workspace: false,
        features: Vec::new(),
        offline: true,
        release: false,
        profile: None,
    };
    let context =
        build_native_compile_context(&common, &manifest_path, &dir).expect("context should build");
    assert_eq!(context.crate_name, "demo_native");
    assert_eq!(
        context.starknet_target,
        NativeStarknetTargetProps {
            sierra: true,
            casm: true
        }
    );
    let cairo_project = fs::read_to_string(context.cairo_project_dir.join("cairo_project.toml"))
        .expect("failed to read cairo project");
    assert!(
        cairo_project.contains("demo_native"),
        "crate name should be normalized in cairo_project.toml: {cairo_project}"
    );
    assert!(
        cairo_project.contains("[config.global]\nedition = \"2024_07\""),
        "manifest edition should be propagated into cairo_project.toml: {cairo_project}"
    );
    assert!(
        !context.workspace_mode_supported,
        "plain package manifests should not be treated as --workspace-safe in native mode"
    );

    std::env::remove_var("UC_NATIVE_CORELIB_SRC");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn build_native_compile_context_allows_workspace_for_single_member_workspace_root() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = unique_test_dir("uc-native-context-workspace-single");
    let manifest_path = dir.join("Scarb.toml");
    fs::create_dir_all(dir.join("src")).expect("failed to create src directory");
    fs::write(dir.join("src/lib.cairo"), "fn main() {}\n").expect("failed to write lib.cairo");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo-native"
version = "0.1.0"
edition = "2024_07"

[workspace]
members = ["."]
"#,
    )
    .expect("failed to write manifest");

    let fake_corelib_src = dir.join("toolchain/corelib/src");
    create_mock_native_corelib(&fake_corelib_src);
    std::env::set_var("UC_NATIVE_CORELIB_SRC", &fake_corelib_src);

    let common = BuildCommonArgs {
        manifest_path: Some(manifest_path.clone()),
        package: None,
        workspace: true,
        features: Vec::new(),
        offline: true,
        release: false,
        profile: None,
    };
    let context =
        build_native_compile_context(&common, &manifest_path, &dir).expect("context should build");
    assert!(
        context.workspace_mode_supported,
        "single-package workspace roots should stay on native backend with --workspace"
    );

    std::env::remove_var("UC_NATIVE_CORELIB_SRC");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn build_native_compile_context_rejects_workspace_for_multi_member_workspace_root() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = unique_test_dir("uc-native-context-workspace-multi");
    let manifest_path = dir.join("Scarb.toml");
    fs::create_dir_all(dir.join("src")).expect("failed to create src directory");
    fs::write(dir.join("src/lib.cairo"), "fn main() {}\n").expect("failed to write lib.cairo");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo-native"
version = "0.1.0"
edition = "2024_07"

[workspace]
members = [".", "packages/other"]
"#,
    )
    .expect("failed to write manifest");

    let fake_corelib_src = dir.join("toolchain/corelib/src");
    create_mock_native_corelib(&fake_corelib_src);
    std::env::set_var("UC_NATIVE_CORELIB_SRC", &fake_corelib_src);

    let common = BuildCommonArgs {
        manifest_path: Some(manifest_path.clone()),
        package: None,
        workspace: true,
        features: Vec::new(),
        offline: true,
        release: false,
        profile: None,
    };
    let err = build_native_compile_context(&common, &manifest_path, &dir)
        .expect_err("multi-member workspace should remain fallback-eligible");
    assert!(
        format!("{err:#}").contains("does not support --workspace"),
        "unexpected error: {err:#}"
    );

    std::env::remove_var("UC_NATIVE_CORELIB_SRC");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn build_native_compile_context_cache_revalidates_requested_package_on_cache_hit() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = unique_test_dir("uc-native-context-cache-package");
    let manifest_path = dir.join("Scarb.toml");
    fs::create_dir_all(dir.join("src")).expect("failed to create src directory");
    fs::write(dir.join("src/lib.cairo"), "fn main() {}\n").expect("failed to write lib.cairo");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo-native"
version = "0.1.0"
edition = "2024_07"
"#,
    )
    .expect("failed to write manifest");

    let fake_corelib_src = dir.join("toolchain/corelib/src");
    create_mock_native_corelib(&fake_corelib_src);
    std::env::set_var("UC_NATIVE_CORELIB_SRC", &fake_corelib_src);

    let common_ok = BuildCommonArgs {
        manifest_path: Some(manifest_path.clone()),
        package: None,
        workspace: false,
        features: Vec::new(),
        offline: true,
        release: false,
        profile: None,
    };
    build_native_compile_context(&common_ok, &manifest_path, &dir)
        .expect("initial native context build should succeed");

    let common_mismatch = BuildCommonArgs {
        manifest_path: Some(manifest_path.clone()),
        package: Some("other-package".to_string()),
        workspace: false,
        features: Vec::new(),
        offline: true,
        release: false,
        profile: None,
    };
    let err = build_native_compile_context(&common_mismatch, &manifest_path, &dir)
        .expect_err("cache hit path should still enforce --package matching");
    assert!(
        format!("{err:#}").contains("native compile only supports the manifest package"),
        "unexpected error: {err:#}"
    );

    std::env::remove_var("UC_NATIVE_CORELIB_SRC");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn build_native_compile_context_cache_tracks_corelib_override_changes() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = unique_test_dir("uc-native-context-cache-corelib");
    let manifest_path = dir.join("Scarb.toml");
    fs::create_dir_all(dir.join("src")).expect("failed to create src directory");
    fs::write(dir.join("src/lib.cairo"), "fn main() {}\n").expect("failed to write lib.cairo");
    fs::write(
        &manifest_path,
        r#"[package]
name = "demo-native"
version = "0.1.0"
edition = "2024_07"
"#,
    )
    .expect("failed to write manifest");

    let corelib_a = dir.join("corelib-a/src");
    let corelib_b = dir.join("corelib-b/src");
    create_mock_native_corelib(&corelib_a);
    create_mock_native_corelib(&corelib_b);

    let common = BuildCommonArgs {
        manifest_path: Some(manifest_path.clone()),
        package: None,
        workspace: false,
        features: Vec::new(),
        offline: true,
        release: false,
        profile: None,
    };

    std::env::set_var("UC_NATIVE_CORELIB_SRC", &corelib_a);
    let context_a = build_native_compile_context(&common, &manifest_path, &dir)
        .expect("context A should build");

    std::env::set_var("UC_NATIVE_CORELIB_SRC", &corelib_b);
    let context_b = build_native_compile_context(&common, &manifest_path, &dir)
        .expect("context B should build");
    assert_ne!(
        context_a.corelib_src, context_b.corelib_src,
        "cache key should include corelib override so override changes invalidate cache"
    );

    std::env::remove_var("UC_NATIVE_CORELIB_SRC");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn native_cairo_project_toml_prefers_explicit_cairo_edition() {
    let rendered = native_cairo_project_toml(
        &[("demo_native".to_string(), "/tmp/demo/src".to_string())],
        &[],
        Some("2023_10"),
    );
    assert!(
        rendered.contains("[crate_roots]\ndemo_native = \"/tmp/demo/src\""),
        "crate roots stanza should be present"
    );
    assert!(
        rendered.contains("[config.global]\nedition = \"2023_10\""),
        "explicit cairo edition should be rendered"
    );
}

#[test]
fn native_cairo_project_toml_renders_override_dependencies_for_local_crates() {
    let rendered = native_cairo_project_toml(
        &[
            ("demo".to_string(), "/tmp/demo/src".to_string()),
            ("utils".to_string(), "/tmp/utils/src".to_string()),
        ],
        &[NativeCrateDependencyConfig {
            crate_name: "demo".to_string(),
            cairo_edition: Some("2024_07".to_string()),
            dependencies: vec!["utils".to_string()],
        }],
        Some("2024_07"),
    );
    assert!(
        rendered.contains("[config.override.demo]\nedition = \"2024_07\""),
        "crate override should preserve effective edition when wiring dependencies: {rendered}"
    );
    assert!(
        rendered.contains("[config.override.demo.dependencies]\nutils = { discriminator = \"utils\" }"),
        "crate override should emit dependency discriminator mapping for local crate roots: {rendered}"
    );
}

#[test]
fn native_starknet_artifact_id_matches_scarb_contract_id_shape() {
    assert_eq!(
        native_starknet_artifact_id("uc_smoke", "uc_smoke::contract_patterns::portfolio_router"),
        "3jvjjppd7e8d4"
    );
}

#[test]
fn native_contract_file_stems_expand_duplicate_contract_names() {
    let stems = native_contract_file_stems(&[
        "demo::alpha::Balance".to_string(),
        "demo::beta::Balance".to_string(),
        "demo::gamma::Vault".to_string(),
    ]);
    assert_eq!(
        stems,
        vec![
            "demo_alpha_Balance".to_string(),
            "demo_beta_Balance".to_string(),
            "Vault".to_string(),
        ]
    );
}

#[test]
fn native_contract_file_stems_disambiguate_non_injective_module_path_collisions() {
    let stems = native_contract_file_stems(&[
        "foo::bar::Transfer".to_string(),
        "foo_bar::Transfer".to_string(),
    ]);
    assert_eq!(stems.len(), 2);
    assert_ne!(
        stems[0], stems[1],
        "module path expansions must remain unique to prevent artifact overwrite"
    );
    assert!(
        stems
            .iter()
            .all(|stem| stem.starts_with("foo_bar_Transfer_")),
        "colliding stems should carry a deterministic disambiguation suffix: {stems:?}"
    );
}

#[cfg(feature = "native-compile")]
#[test]
fn write_native_sierra_artifact_does_not_prune_when_write_fails() {
    let dir = unique_test_dir("uc-native-write-before-prune");
    let target_dir = dir.join("target/dev");
    fs::create_dir_all(&target_dir).expect("failed to create target directory");
    let stale_artifacts = target_dir.join("demo.starknet_artifacts.json");
    fs::write(&stale_artifacts, "{}").expect("failed to seed stale artifact");
    fs::create_dir_all(target_dir.join("demo.sierra"))
        .expect("failed to create blocking output directory");

    let err = write_native_sierra_artifact(&target_dir, "demo", "demo.sierra", "fn main() {}\n")
        .expect_err("write should fail when the output path is a directory");
    assert!(
        format!("{err:#}").contains("failed to write native artifact demo.sierra"),
        "unexpected error: {err:#}"
    );
    assert!(
        stale_artifacts.exists(),
        "prune should not run when native sierra write fails"
    );

    fs::remove_dir_all(&dir).ok();
}

#[cfg(feature = "native-compile")]
#[test]
fn native_compile_session_build_lock_is_per_key_and_released_when_idle() {
    let key = format!("workspace-{}", epoch_ms_u64().unwrap_or_default());
    let other_key = format!("{key}-other");
    {
        let mut locks = native_compile_session_build_locks()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks.remove(&key);
        locks.remove(&other_key);
    }

    let first = native_compile_session_build_lock(&key);
    let second = native_compile_session_build_lock(&key);
    let other = native_compile_session_build_lock(&other_key);
    assert!(
        std::sync::Arc::ptr_eq(&first, &second),
        "same key should share one build lock"
    );
    assert!(
        !std::sync::Arc::ptr_eq(&first, &other),
        "different keys should not contend on one build lock"
    );

    release_native_compile_session_build_lock(&key, &first);
    {
        let locks = native_compile_session_build_locks()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            locks.contains_key(&key),
            "lock entry must remain while another waiter still holds a reference"
        );
    }
    drop(second);
    release_native_compile_session_build_lock(&key, &first);
    {
        let locks = native_compile_session_build_locks()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            !locks.contains_key(&key),
            "lock entry should be removed once it is idle"
        );
    }

    release_native_compile_session_build_lock(&other_key, &other);
    drop(other);
    {
        let locks = native_compile_session_build_locks()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            !locks.contains_key(&other_key),
            "independent key entry should also be removed once idle"
        );
    }
}

#[cfg(feature = "native-compile")]
fn native_test_compile_session_signature(
    workspace_root: &Path,
    manifest_hash: &str,
) -> NativeCompileSessionSignature {
    NativeCompileSessionSignature {
        manifest_path: workspace_root.join("Scarb.toml"),
        manifest_content_hash: manifest_hash.to_string(),
        context: NativeCompileContext {
            package_name: "demo".to_string(),
            crate_name: "demo".to_string(),
            workspace_mode_supported: true,
            cairo_project_dir: workspace_root.join(".uc/native-project"),
            corelib_src: workspace_root.join("toolchain/corelib/src"),
            starknet_target: NativeStarknetTargetProps {
                sierra: true,
                casm: true,
            },
            manifest_content_hash: manifest_hash.to_string(),
            external_non_starknet_dependencies: vec!["serde".to_string()],
            path_dependency_roots: vec![NativePathDependencyRoot {
                crate_name: "shared".to_string(),
                source_root: workspace_root.join("deps/shared/src"),
            }],
            crate_dependency_configs: vec![NativeCrateDependencyConfig {
                crate_name: "demo".to_string(),
                cairo_edition: Some("2024_07".to_string()),
                dependencies: vec!["starknet".to_string()],
            }],
        },
    }
}

#[cfg(feature = "native-compile")]
#[test]
fn native_compile_session_image_round_trip_restores_tracked_sources_and_dependency_index() {
    let dir = unique_test_dir("uc-native-session-image-roundtrip");
    let signature = native_test_compile_session_signature(&dir, "manifest-blake3:demo");
    let tracked_sources = BTreeMap::from([
        (
            "src/lib.cairo".to_string(),
            NativeTrackedFileState {
                size_bytes: 42,
                modified_unix_ms: 101,
            },
        ),
        (
            "src/math.cairo".to_string(),
            NativeTrackedFileState {
                size_bytes: 12,
                modified_unix_ms: 102,
            },
        ),
    ]);
    let tracked_source_bytes = native_tracked_sources_total_bytes(&tracked_sources)
        .expect("tracked source bytes should be computed");
    let dependencies = BTreeMap::from([(
        "demo::token".to_string(),
        BTreeSet::from(["src/lib.cairo".to_string(), "src/math.cairo".to_string()]),
    )]);
    let plans = vec![NativeContractOutputPlan {
        module_path: "demo::token".to_string(),
        artifact_id: "id-token".to_string(),
        package_name: "demo".to_string(),
        contract_name: "token".to_string(),
        artifact_file: "demo_token.contract_class.json".to_string(),
        casm_file: Some("demo_token.compiled_contract_class.json".to_string()),
    }];
    let snapshot = NativeCompileSessionImageSnapshot {
        signature_hash: native_compile_session_signature_hash(&signature),
        source_root_modified_unix_ms: 777,
        tracked_sources: tracked_sources.clone(),
        tracked_source_bytes,
        contract_source_dependencies: dependencies.clone(),
        contract_output_plans: plans.clone(),
    };
    persist_native_compile_session_image_snapshot(&dir, &snapshot)
        .expect("native session image should persist");

    let restored = try_native_compile_session_image_restore(&dir, &signature, 777)
        .expect("matching signature/root mtime should restore session image");
    assert_eq!(restored.tracked_sources, tracked_sources);
    assert_eq!(restored.tracked_source_bytes, tracked_source_bytes);
    assert_eq!(restored.contract_source_dependencies, dependencies);
    assert_eq!(restored.contract_output_plans, plans);

    fs::remove_dir_all(&dir).ok();
}

#[cfg(feature = "native-compile")]
#[test]
fn native_compile_session_image_restore_rejects_signature_and_root_mtime_mismatches() {
    let dir = unique_test_dir("uc-native-session-image-invalidations");
    let signature = native_test_compile_session_signature(&dir, "manifest-blake3:demo");
    let tracked_sources = BTreeMap::from([(
        "src/lib.cairo".to_string(),
        NativeTrackedFileState {
            size_bytes: 5,
            modified_unix_ms: 11,
        },
    )]);
    let tracked_source_bytes = native_tracked_sources_total_bytes(&tracked_sources)
        .expect("tracked source bytes should be computed");
    let snapshot = NativeCompileSessionImageSnapshot {
        signature_hash: native_compile_session_signature_hash(&signature),
        source_root_modified_unix_ms: 999,
        tracked_sources,
        tracked_source_bytes,
        contract_source_dependencies: BTreeMap::new(),
        contract_output_plans: Vec::new(),
    };
    persist_native_compile_session_image_snapshot(&dir, &snapshot)
        .expect("native session image should persist");

    let different_signature = native_test_compile_session_signature(&dir, "manifest-blake3:other");
    assert!(
        try_native_compile_session_image_restore(&dir, &different_signature, 999).is_none(),
        "signature mismatch must invalidate persisted session image"
    );
    assert!(
        try_native_compile_session_image_restore(&dir, &signature, 1_000).is_none(),
        "source-root mtime mismatch must invalidate persisted session image"
    );

    fs::remove_dir_all(&dir).ok();
}

#[cfg(feature = "native-compile")]
#[test]
fn native_session_refresh_action_prefers_incremental_for_changed_sets() {
    assert_eq!(
        native_session_refresh_action(false, false, 0, 0),
        NativeSessionRefreshAction::None
    );
    assert_eq!(
        native_session_refresh_action(false, true, 1, 0),
        NativeSessionRefreshAction::IncrementalChangedSet
    );
    assert_eq!(
        native_session_refresh_action(true, false, 0, 0),
        NativeSessionRefreshAction::FullRebuild
    );
    assert_eq!(
        native_session_refresh_action(true, true, 1, 1),
        NativeSessionRefreshAction::FullRebuild
    );
    assert_eq!(
        native_session_refresh_action(false, true, 10_000, 0),
        NativeSessionRefreshAction::FullRebuild,
        "large changed-file sets should force a full rebuild to keep daemon latency predictable"
    );
}

#[cfg(feature = "native-compile")]
#[test]
fn native_impacted_subset_used_requires_partial_compile() {
    assert!(
        !native_impacted_subset_used(0, 0),
        "empty contract sets should not be marked as subset compiles"
    );
    assert!(
        !native_impacted_subset_used(4, 0),
        "zero compiled contracts should not be marked as subset compiles"
    );
    assert!(
        !native_impacted_subset_used(4, 4),
        "full compiles should not be marked as subset compiles"
    );
    assert!(
        native_impacted_subset_used(4, 2),
        "partial compiles should be marked as impacted-subset compiles"
    );
}

#[cfg(feature = "native-compile")]
#[test]
fn native_impacted_source_index_requires_complete_dependency_index_for_unmatched_changes() {
    let by_source = HashMap::from([
        (
            "src/contract_patterns.cairo".to_string(),
            vec![0_usize, 1_usize],
        ),
        ("src/math.cairo".to_string(), vec![1_usize]),
    ]);

    let incomplete = native_impacted_contract_indices_from_source_index(
        &by_source,
        &[String::from("src/lib.cairo")],
        &[],
        false,
    );
    assert!(
        incomplete.is_none(),
        "unmatched changes must force full compile when dependency index is incomplete"
    );

    let complete = native_impacted_contract_indices_from_source_index(
        &by_source,
        &[String::from("src/lib.cairo")],
        &[],
        true,
    )
    .expect("complete index should return impacted subset decision");
    assert!(
        complete.is_empty(),
        "unmatched changes should be treated as no-op for contracts when dependency index is complete"
    );
}

#[cfg(feature = "native-compile")]
#[test]
fn native_changed_files_affect_tracked_contracts_skips_unrelated_changes_when_index_complete() {
    let plans = vec![
        NativeContractOutputPlan {
            module_path: "pkg::contract_a".to_string(),
            artifact_id: "a".to_string(),
            package_name: "pkg".to_string(),
            contract_name: "contract_a".to_string(),
            artifact_file: "pkg_contract_a.contract_class.json".to_string(),
            casm_file: Some("pkg_contract_a.compiled_contract_class.json".to_string()),
        },
        NativeContractOutputPlan {
            module_path: "pkg::contract_b".to_string(),
            artifact_id: "b".to_string(),
            package_name: "pkg".to_string(),
            contract_name: "contract_b".to_string(),
            artifact_file: "pkg_contract_b.contract_class.json".to_string(),
            casm_file: Some("pkg_contract_b.compiled_contract_class.json".to_string()),
        },
    ];
    let dependencies = BTreeMap::from([
        (
            "pkg::contract_a".to_string(),
            BTreeSet::from([
                "src/contract_a.cairo".to_string(),
                "src/shared_types.cairo".to_string(),
            ]),
        ),
        (
            "pkg::contract_b".to_string(),
            BTreeSet::from(["src/contract_b.cairo".to_string()]),
        ),
    ]);

    assert!(
        !native_changed_files_affect_tracked_contracts(
            &[String::from("src/math.cairo")],
            &[],
            &plans,
            &dependencies
        ),
        "complete dependency indexes should treat unrelated source edits as no-op for contracts"
    );
    assert!(
        native_changed_files_affect_tracked_contracts(
            &[String::from("src/shared_types.cairo")],
            &[],
            &plans,
            &dependencies
        ),
        "indexed changed files should still trigger contract refresh"
    );
    assert!(
        native_changed_files_affect_tracked_contracts(
            &[],
            &[String::from("src/contract_b.cairo")],
            &plans,
            &dependencies
        ),
        "indexed removed files should still trigger contract refresh"
    );
}

#[cfg(feature = "native-compile")]
#[test]
fn native_changed_files_affect_tracked_contracts_stays_conservative_when_index_incomplete() {
    let plans = vec![
        NativeContractOutputPlan {
            module_path: "pkg::contract_a".to_string(),
            artifact_id: "a".to_string(),
            package_name: "pkg".to_string(),
            contract_name: "contract_a".to_string(),
            artifact_file: "pkg_contract_a.contract_class.json".to_string(),
            casm_file: Some("pkg_contract_a.compiled_contract_class.json".to_string()),
        },
        NativeContractOutputPlan {
            module_path: "pkg::contract_b".to_string(),
            artifact_id: "b".to_string(),
            package_name: "pkg".to_string(),
            contract_name: "contract_b".to_string(),
            artifact_file: "pkg_contract_b.contract_class.json".to_string(),
            casm_file: Some("pkg_contract_b.compiled_contract_class.json".to_string()),
        },
    ];
    let dependencies = BTreeMap::from([(
        "pkg::contract_a".to_string(),
        BTreeSet::from(["src/contract_a.cairo".to_string()]),
    )]);

    assert!(
        native_changed_files_affect_tracked_contracts(
            &[String::from("src/math.cairo")],
            &[],
            &plans,
            &dependencies
        ),
        "incomplete dependency indexes should keep conservative contract refresh behavior"
    );
}

#[cfg(feature = "native-compile")]
#[test]
fn native_workspace_relative_cairo_path_from_debug_requires_workspace_cairo_paths() {
    let workspace = PathBuf::from("/tmp/uc-native-debug-paths");
    assert_eq!(
        native_workspace_relative_cairo_path_from_debug(
            &workspace,
            "/tmp/uc-native-debug-paths/src/lib.cairo"
        )
        .as_deref(),
        Some("src/lib.cairo")
    );
    assert!(
        native_workspace_relative_cairo_path_from_debug(
            &workspace,
            "/tmp/uc-native-debug-paths/src/lib.txt"
        )
        .is_none(),
        "non-cairo files should be ignored"
    );
    assert!(
        native_workspace_relative_cairo_path_from_debug(&workspace, "/tmp/elsewhere/src/lib.cairo")
            .is_none(),
        "files outside workspace root must be ignored"
    );
}

#[cfg(feature = "native-compile")]
#[test]
fn native_contract_dependency_paths_from_debug_info_extracts_workspace_cairo_sources() {
    let workspace = PathBuf::from("/tmp/uc-native-contract-deps");
    let class: ContractClass = serde_json::from_value(serde_json::json!({
        "sierra_program": [],
        "sierra_program_debug_info": {
            "type_names": [],
            "libfunc_names": [],
            "user_func_names": [],
            "annotations": {
                "github.com/software-mansion/cairo-coverage": {
                    "statements_code_locations": {
                        "0": [
                            ["/tmp/uc-native-contract-deps/src/lib.cairo", {"start":{"line":0,"col":0},"end":{"line":0,"col":1}}, false],
                            ["/tmp/uc-native-contract-deps/src/math.cairo", {"start":{"line":1,"col":0},"end":{"line":1,"col":1}}, false]
                        ],
                        "1": [
                            ["/tmp/uc-native-contract-deps/src/lib.cairo", {"start":{"line":2,"col":0},"end":{"line":2,"col":1}}, false]
                        ]
                    }
                }
            },
            "executables": {}
        },
        "contract_class_version": "0.1.0",
        "entry_points_by_type": {
            "EXTERNAL": [],
            "L1_HANDLER": [],
            "CONSTRUCTOR": []
        },
        "abi": null
    }))
    .expect("contract class debug info fixture should deserialize");
    let deps = native_contract_dependency_paths_from_debug_info(&workspace, &class);
    assert_eq!(
        deps,
        BTreeSet::from(["src/lib.cairo".to_string(), "src/math.cairo".to_string()])
    );
}

#[cfg(feature = "native-compile")]
#[test]
fn native_collect_contract_dependency_updates_preserves_contract_source_fallback() {
    let workspace = PathBuf::from("/tmp/uc-native-dependency-updates");
    let plans = vec![NativeContractOutputPlan {
        module_path: "pkg::token".to_string(),
        artifact_id: "id-token".to_string(),
        package_name: "pkg".to_string(),
        contract_name: "token".to_string(),
        artifact_file: "pkg_token.contract_class.json".to_string(),
        casm_file: Some("pkg_token.compiled_contract_class.json".to_string()),
    }];
    let classes = vec![ContractClass {
        sierra_program: Vec::new(),
        sierra_program_debug_info: None,
        contract_class_version: "0.1.0".to_string(),
        entry_points_by_type: Default::default(),
        abi: None,
    }];
    let updates = native_collect_contract_dependency_updates(
        &workspace,
        &plans,
        &[Some("src/contract_patterns.cairo".to_string())],
        &[0_usize],
        &classes,
    );
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].0, "pkg::token");
    assert!(
        updates[0].1.contains("src/contract_patterns.cairo"),
        "contract source path must be retained even when debug annotation map is empty"
    );
}

#[cfg(feature = "native-compile")]
#[test]
fn native_diff_tracked_sources_detects_changed_and_removed_files() {
    let previous = BTreeMap::from([
        (
            "src/lib.cairo".to_string(),
            NativeTrackedFileState {
                size_bytes: 10,
                modified_unix_ms: 100,
            },
        ),
        (
            "src/old.cairo".to_string(),
            NativeTrackedFileState {
                size_bytes: 4,
                modified_unix_ms: 50,
            },
        ),
    ]);
    let current = BTreeMap::from([
        (
            "src/lib.cairo".to_string(),
            NativeTrackedFileState {
                size_bytes: 12,
                modified_unix_ms: 101,
            },
        ),
        (
            "src/new.cairo".to_string(),
            NativeTrackedFileState {
                size_bytes: 2,
                modified_unix_ms: 77,
            },
        ),
    ]);

    let (changed, removed) = native_diff_tracked_sources(&previous, &current);
    assert_eq!(
        changed,
        vec!["src/lib.cairo".to_string(), "src/new.cairo".to_string()]
    );
    assert_eq!(removed, vec!["src/old.cairo".to_string()]);
}

#[cfg(feature = "native-compile")]
#[test]
fn native_collect_tracked_sources_tracks_only_cairo_files() {
    let dir = unique_test_dir("uc-native-track-sources");
    fs::create_dir_all(dir.join("src")).expect("failed to create src directory");
    fs::write(dir.join("src/lib.cairo"), "fn lib() {}\n").expect("failed to write cairo file");
    fs::write(dir.join("src/notes.txt"), "ignore me\n").expect("failed to write non-cairo file");
    fs::write(
        dir.join("Scarb.toml"),
        "[package]\nname=\"x\"\nversion=\"0.1.0\"\n",
    )
    .expect("failed to write manifest");

    let source_roots = vec![dir.join("src")];
    let (tracked, total_bytes) = native_collect_tracked_sources(&dir, &source_roots)
        .expect("source tracking should succeed");
    assert_eq!(tracked.len(), 1, "only cairo files should be tracked");
    assert!(
        tracked.contains_key("src/lib.cairo"),
        "tracked source set should include src/lib.cairo"
    );
    assert!(
        !tracked.contains_key("Scarb.toml"),
        "manifest changes are handled by session signature and should not be tracked as source files"
    );
    assert!(total_bytes > 0, "tracked source bytes should be non-zero");

    fs::remove_dir_all(&dir).ok();
}

#[cfg(feature = "native-compile")]
#[test]
fn native_collect_tracked_sources_limits_to_declared_roots() {
    let dir = unique_test_dir("uc-native-track-sources-roots");
    let root_src = dir.join("src");
    let dep_src = dir.join("deps/local-dep/src");
    let other_src = dir.join("packages/other/src");
    fs::create_dir_all(&root_src).expect("failed to create root src directory");
    fs::create_dir_all(&dep_src).expect("failed to create dependency src directory");
    fs::create_dir_all(&other_src).expect("failed to create other src directory");
    fs::write(root_src.join("lib.cairo"), "fn root() {}\n").expect("failed to write root cairo");
    fs::write(dep_src.join("lib.cairo"), "fn dep() {}\n")
        .expect("failed to write dependency cairo");
    fs::write(other_src.join("lib.cairo"), "fn other() {}\n")
        .expect("failed to write unrelated cairo");

    let source_roots = vec![root_src, dep_src];
    let (tracked, _total_bytes) = native_collect_tracked_sources(&dir, &source_roots)
        .expect("source tracking should succeed");
    assert!(
        tracked.contains_key("src/lib.cairo"),
        "tracked source set should include root crate source"
    );
    assert!(
        tracked.contains_key("deps/local-dep/src/lib.cairo"),
        "tracked source set should include dependency source roots"
    );
    assert!(
        !tracked.contains_key("packages/other/src/lib.cairo"),
        "source files outside declared roots must be ignored to avoid unrelated workspace churn"
    );
    fs::remove_dir_all(&dir).ok();
}

#[cfg(feature = "native-compile")]
#[test]
fn native_compile_source_roots_include_main_and_dependency_roots_without_duplicates() {
    let workspace_root = PathBuf::from("/tmp/uc-native-source-roots");
    let duplicate_dep_root = workspace_root.join("deps/shared/src");
    let context = NativeCompileContext {
        package_name: "demo".to_string(),
        crate_name: "demo".to_string(),
        workspace_mode_supported: true,
        cairo_project_dir: workspace_root.join(".uc/native-project"),
        corelib_src: workspace_root.join("toolchain/corelib/src"),
        starknet_target: NativeStarknetTargetProps {
            sierra: true,
            casm: true,
        },
        manifest_content_hash: "manifest-blake3:demo".to_string(),
        external_non_starknet_dependencies: Vec::new(),
        path_dependency_roots: vec![
            NativePathDependencyRoot {
                crate_name: "shared_a".to_string(),
                source_root: duplicate_dep_root.clone(),
            },
            NativePathDependencyRoot {
                crate_name: "shared_b".to_string(),
                source_root: duplicate_dep_root.clone(),
            },
        ],
        crate_dependency_configs: Vec::new(),
    };

    let roots = native_compile_source_roots(&workspace_root, &context);
    let normalized = roots
        .iter()
        .map(|path| normalize_fingerprint_path(path))
        .collect::<Vec<_>>();
    assert_eq!(
        normalized,
        vec![
            normalize_fingerprint_path(&workspace_root.join("deps/shared/src")),
            normalize_fingerprint_path(&workspace_root.join("src")),
        ],
        "source roots should include main + dependency roots and deduplicate identical paths"
    );
}

#[cfg(feature = "native-compile")]
#[test]
fn native_workspace_relative_cairo_path_accepts_src_cairo_only() {
    let dir = unique_test_dir("uc-native-source-relpath");
    fs::create_dir_all(dir.join("src")).expect("failed to create src directory");
    let cairo_path = dir.join("src/lib.cairo");
    let txt_path = dir.join("src/readme.txt");
    fs::write(&cairo_path, "fn main() {}\n").expect("failed to write cairo file");
    fs::write(&txt_path, "ignore me\n").expect("failed to write txt file");

    assert_eq!(
        native_workspace_relative_cairo_path(&dir, &cairo_path).as_deref(),
        Some("src/lib.cairo")
    );
    assert!(
        native_workspace_relative_cairo_path(&dir, &txt_path).is_none(),
        "non-cairo files must be ignored"
    );
    assert!(
        native_workspace_relative_cairo_path(&dir, &dir.join("../escape.cairo")).is_none(),
        "paths outside the workspace root must be rejected"
    );

    fs::remove_dir_all(&dir).ok();
}

#[cfg(feature = "native-compile")]
#[test]
fn native_record_source_change_event_tracks_create_rename_and_remove() {
    let dir = unique_test_dir("uc-native-source-journal-event");
    fs::create_dir_all(dir.join("src")).expect("failed to create src directory");
    let before = dir.join("src/lib.cairo");
    let after = dir.join("src/new_lib.cairo");
    let journal = Arc::new(Mutex::new(NativeSourceChangeJournal::default()));

    native_record_source_change_event(
        &dir,
        &journal,
        &NotifyEventKind::Any,
        std::slice::from_ref(&before),
    );
    {
        let state = journal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(state.changed_files.contains("src/lib.cairo"));
        assert!(state.removed_files.is_empty());
    }

    native_record_source_change_event(
        &dir,
        &journal,
        &NotifyEventKind::Modify(NotifyModifyKind::Name(NotifyRenameMode::Both)),
        &[before.clone(), after.clone()],
    );
    {
        let state = journal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(state.changed_files.contains("src/new_lib.cairo"));
        assert!(state.removed_files.contains("src/lib.cairo"));
    }

    native_record_source_change_event(
        &dir,
        &journal,
        &NotifyEventKind::Remove(notify::event::RemoveKind::File),
        std::slice::from_ref(&after),
    );
    {
        let state = journal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(state.changed_files.is_empty());
        assert!(state.removed_files.contains("src/new_lib.cairo"));
    }

    fs::remove_dir_all(&dir).ok();
}

#[cfg(feature = "native-compile")]
#[test]
fn native_apply_source_change_journal_delta_updates_snapshot() {
    let dir = unique_test_dir("uc-native-source-journal-apply");
    fs::create_dir_all(dir.join("src")).expect("failed to create src directory");
    fs::write(dir.join("src/lib.cairo"), "fn lib() {}\n").expect("failed to write lib.cairo");
    fs::write(dir.join("src/new.cairo"), "fn new_file() {}\n").expect("failed to write new.cairo");
    let previous = BTreeMap::from([
        (
            "src/lib.cairo".to_string(),
            NativeTrackedFileState {
                size_bytes: 4,
                modified_unix_ms: 1,
            },
        ),
        (
            "src/old.cairo".to_string(),
            NativeTrackedFileState {
                size_bytes: 3,
                modified_unix_ms: 1,
            },
        ),
    ]);
    let (updated, total_bytes) = native_apply_source_change_journal_delta(
        &dir,
        &previous,
        &["src/lib.cairo".to_string(), "src/new.cairo".to_string()],
        &["src/old.cairo".to_string()],
    )
    .expect("journal delta application should succeed");
    assert!(
        updated.contains_key("src/lib.cairo"),
        "updated snapshot must include changed file"
    );
    assert!(
        updated.contains_key("src/new.cairo"),
        "updated snapshot must include created file"
    );
    assert!(
        !updated.contains_key("src/old.cairo"),
        "updated snapshot must remove deleted file"
    );
    assert!(
        total_bytes > 0,
        "tracked source budget should stay non-zero"
    );
    fs::remove_dir_all(&dir).ok();
}

#[cfg(feature = "native-compile")]
#[test]
fn native_reusable_unaffected_manifest_entries_reuses_only_safe_entries() {
    let dir = unique_test_dir("uc-native-manifest-reuse");
    fs::create_dir_all(&dir).expect("failed to create target dir");
    let plans = vec![
        NativeContractOutputPlan {
            module_path: "pkg::token".to_string(),
            artifact_id: "id-token".to_string(),
            package_name: "pkg".to_string(),
            contract_name: "token".to_string(),
            artifact_file: "pkg_token.contract_class.json".to_string(),
            casm_file: Some("pkg_token.compiled_contract_class.json".to_string()),
        },
        NativeContractOutputPlan {
            module_path: "pkg::vault".to_string(),
            artifact_id: "id-vault".to_string(),
            package_name: "pkg".to_string(),
            contract_name: "vault".to_string(),
            artifact_file: "pkg_vault.contract_class.json".to_string(),
            casm_file: Some("pkg_vault.compiled_contract_class.json".to_string()),
        },
    ];
    let manifest = StarknetArtifactsManifest {
        version: 1,
        contracts: vec![
            StarknetArtifactEntry {
                id: plans[0].artifact_id.clone(),
                package_name: plans[0].package_name.clone(),
                contract_name: plans[0].contract_name.clone(),
                module_path: plans[0].module_path.clone(),
                artifacts: StarknetArtifactFiles {
                    sierra: plans[0].artifact_file.clone(),
                    casm: plans[0].casm_file.clone(),
                },
            },
            StarknetArtifactEntry {
                id: plans[1].artifact_id.clone(),
                package_name: plans[1].package_name.clone(),
                contract_name: plans[1].contract_name.clone(),
                module_path: plans[1].module_path.clone(),
                artifacts: StarknetArtifactFiles {
                    sierra: plans[1].artifact_file.clone(),
                    casm: plans[1].casm_file.clone(),
                },
            },
        ],
    };
    fs::write(
        dir.join("pkg.starknet_artifacts.json"),
        serde_json::to_vec(&manifest).expect("manifest should serialize"),
    )
    .expect("failed to write manifest");
    fs::write(dir.join(&plans[1].artifact_file), "{}\n")
        .expect("failed to write unaffected sierra");
    fs::write(
        dir.join(
            plans[1]
                .casm_file
                .as_ref()
                .expect("casm file should exist in plan"),
        ),
        "{}\n",
    )
    .expect("failed to write unaffected casm");
    let impacted = BTreeSet::from([0_usize]);
    let result = native_reusable_unaffected_manifest_entries(&dir, "pkg", &plans, &impacted)
        .expect("manifest reuse evaluation should succeed")
        .expect("unaffected entries should be reusable");
    assert_eq!(
        result.0.len(),
        1,
        "one unaffected contract should be reused"
    );
    assert_eq!(result.0[0].id, "id-vault");
    assert!(result.1.contains("pkg_vault.contract_class.json"));
    assert!(result.1.contains("pkg_vault.compiled_contract_class.json"));

    fs::remove_file(dir.join("pkg_vault.contract_class.json")).expect("failed to remove artifact");
    let missing = native_reusable_unaffected_manifest_entries(&dir, "pkg", &plans, &impacted)
        .expect("manifest reuse should still evaluate");
    assert!(
        missing.is_none(),
        "missing unaffected artifact should force full-compile fallback"
    );

    fs::remove_dir_all(&dir).ok();
}

#[cfg(feature = "native-compile")]
#[test]
fn native_cached_noop_keep_files_requires_complete_manifest_artifacts() {
    let dir = unique_test_dir("uc-native-noop-keep-files");
    fs::create_dir_all(&dir).expect("failed to create target dir");
    let plans = vec![NativeContractOutputPlan {
        module_path: "pkg::token".to_string(),
        artifact_id: "id-token".to_string(),
        package_name: "pkg".to_string(),
        contract_name: "token".to_string(),
        artifact_file: "pkg_token.contract_class.json".to_string(),
        casm_file: Some("pkg_token.compiled_contract_class.json".to_string()),
    }];
    let manifest = StarknetArtifactsManifest {
        version: 1,
        contracts: vec![StarknetArtifactEntry {
            id: plans[0].artifact_id.clone(),
            package_name: plans[0].package_name.clone(),
            contract_name: plans[0].contract_name.clone(),
            module_path: plans[0].module_path.clone(),
            artifacts: StarknetArtifactFiles {
                sierra: plans[0].artifact_file.clone(),
                casm: plans[0].casm_file.clone(),
            },
        }],
    };
    fs::write(
        dir.join("pkg.starknet_artifacts.json"),
        serde_json::to_vec(&manifest).expect("manifest should serialize"),
    )
    .expect("failed to write manifest");
    fs::write(dir.join(&plans[0].artifact_file), "{}\n").expect("failed to write sierra artifact");
    fs::write(
        dir.join(
            plans[0]
                .casm_file
                .as_ref()
                .expect("casm file should exist in plan"),
        ),
        "{}\n",
    )
    .expect("failed to write casm artifact");

    let keep_files = native_cached_noop_keep_files(&dir, "pkg", &plans)
        .expect("noop keep-file evaluation should succeed")
        .expect("valid manifest/artifacts should enable noop reuse");
    assert!(keep_files.contains("pkg_token.contract_class.json"));
    assert!(keep_files.contains("pkg_token.compiled_contract_class.json"));
    assert!(keep_files.contains("pkg.starknet_artifacts.json"));

    fs::remove_file(dir.join("pkg_token.contract_class.json")).expect("failed to remove sierra");
    let missing = native_cached_noop_keep_files(&dir, "pkg", &plans)
        .expect("noop keep-file evaluation should still succeed");
    assert!(
        missing.is_none(),
        "missing artifact should disable noop reuse"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn native_compiler_config_sets_replace_ids_by_profile() {
    let empty_inputs: Vec<CrateInput> = Vec::new();
    assert!(
        native_compiler_config(&empty_inputs, "dev").replace_ids,
        "dev profile should default to replace IDs"
    );
    assert!(
        !native_compiler_config(&empty_inputs, "release").replace_ids,
        "release profile should not replace IDs by default"
    );
}

#[test]
fn resolve_manifest_native_starknet_target_props_defaults_match_scarb() {
    let manifest: TomlValue = toml::from_str(
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"
"#,
    )
    .expect("manifest should parse");
    let props = resolve_manifest_native_starknet_target_props(&manifest)
        .expect("default target props should parse");
    assert_eq!(
        props,
        NativeStarknetTargetProps {
            sierra: true,
            casm: true
        }
    );
}

#[test]
fn resolve_manifest_native_starknet_target_props_respects_explicit_flags() {
    let manifest: TomlValue = toml::from_str(
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"

[target.starknet-contract]
sierra = true
casm = true
"#,
    )
    .expect("manifest should parse");
    let props = resolve_manifest_native_starknet_target_props(&manifest)
        .expect("explicit target props should parse");
    assert_eq!(
        props,
        NativeStarknetTargetProps {
            sierra: true,
            casm: true
        }
    );
}

#[test]
fn resolve_manifest_native_starknet_target_props_accepts_single_target_array_entry() {
    let manifest: TomlValue = toml::from_str(
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"

[[target.starknet-contract]]
"#,
    )
    .expect("manifest should parse");
    let props = resolve_manifest_native_starknet_target_props(&manifest)
        .expect("single target array should be supported");
    assert_eq!(
        props,
        NativeStarknetTargetProps {
            sierra: true,
            casm: true
        }
    );
}

#[test]
fn resolve_manifest_native_starknet_target_props_rejects_multiple_target_array_entries() {
    let manifest: TomlValue = toml::from_str(
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024_07"

[[target.starknet-contract]]

[[target.starknet-contract]]
"#,
    )
    .expect("manifest should parse");
    let err = resolve_manifest_native_starknet_target_props(&manifest)
        .expect_err("multiple target entries should be rejected");
    assert!(
        format!("{err:#}").contains("supports a single [[target.starknet-contract]] entry"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn compile_native_casm_contract_rejects_tiny_bytecode_limit() {
    let fixture_contract = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../benchmarks/fixtures/scarb_smoke/target/dev/uc_smoke_token.contract_class.json",
    );
    let fixture_bytes = fs::read(&fixture_contract).unwrap_or_else(|err| {
        panic!(
            "failed to read starknet contract fixture {}: {err}",
            fixture_contract.display()
        )
    });
    let contract_class: ContractClass =
        serde_json::from_slice(&fixture_bytes).expect("failed to decode contract class fixture");
    let err = compile_native_casm_contract(contract_class, 1)
        .expect_err("tiny CASM bytecode limit should fail for fixture contract");
    let message = format!("{err:#}");
    let normalized = message.to_ascii_lowercase();
    assert!(
        message.contains("failed to compile native CASM contract class"),
        "expected CASM compile failure context, got: {message}"
    );
    assert!(
        (normalized.contains("bytecode") || normalized.contains("code size"))
            && (normalized.contains("limit") || normalized.contains("exceed")),
        "expected CASM bytecode size-limit rejection, got: {message}"
    );
}

#[test]
fn normalize_package_name_for_cairo_crate_sanitizes_toml_bare_key_shape() {
    assert_eq!(
        normalize_package_name_for_cairo_crate("cairo.contracts-2"),
        "cairo_contracts_2"
    );
    assert_eq!(
        normalize_package_name_for_cairo_crate("9demo"),
        "_9demo",
        "crate keys should not start with a digit"
    );
}

#[test]
fn toml_escape_basic_string_escapes_backslashes_and_quotes() {
    assert_eq!(
        toml_escape_basic_string(r#"C:\tmp\project "v2""#),
        r#"C:\\tmp\\project \"v2\""#
    );
}

#[test]
fn toml_escape_basic_string_escapes_control_characters() {
    assert_eq!(
        toml_escape_basic_string("line1\nline2\t\u{0007}\u{007F}"),
        r#"line1\nline2\t\u0007\u007F"#
    );
}

#[test]
fn toml_escape_basic_string_preserves_non_bmp_scalars_without_corrupting_escape_state() {
    assert_eq!(
        toml_escape_basic_string("prefix\u{1F642}\u{0007}"),
        "prefix🙂\\u0007"
    );
}

#[test]
fn cacheable_artifact_suffixes_include_native_compiled_contract_class() {
    assert!(
        CACHEABLE_ARTIFACT_SUFFIXES.contains(&".compiled_contract_class.json"),
        "native CASM artifact suffix must be cacheable for warm restores"
    );
}

#[test]
fn cacheable_artifact_suffixes_include_starknet_manifest() {
    assert!(
        CACHEABLE_ARTIFACT_SUFFIXES.contains(&".starknet_artifacts.json"),
        "starknet artifacts manifest suffix must be cacheable for warm restores"
    );
}

#[test]
fn native_error_allows_scarb_fallback_only_when_marked() {
    let generic = anyhow::Error::msg("native compile failed with diagnostics");
    assert!(
        !native_error_allows_scarb_fallback(&generic),
        "plain errors must not trigger scarb fallback"
    );

    let eligible = native_fallback_eligible_error("native backend unavailable");
    assert!(
        native_error_allows_scarb_fallback(&eligible),
        "fallback-eligible errors should trigger scarb fallback in auto mode"
    );
}

#[cfg(feature = "native-compile")]
#[test]
fn mark_native_fallback_eligible_for_external_dependencies_only_marks_when_present() {
    let base_context = NativeCompileContext {
        package_name: "demo".to_string(),
        crate_name: "demo".to_string(),
        workspace_mode_supported: false,
        cairo_project_dir: PathBuf::from("/tmp/demo/.uc/native-project"),
        corelib_src: PathBuf::from("/tmp/demo/corelib/src"),
        starknet_target: NativeStarknetTargetProps {
            sierra: true,
            casm: true,
        },
        manifest_content_hash: "manifest-blake3:demo".to_string(),
        external_non_starknet_dependencies: Vec::new(),
        path_dependency_roots: Vec::new(),
        crate_dependency_configs: Vec::new(),
    };

    let plain = mark_native_fallback_eligible_for_external_dependencies(
        anyhow::Error::msg("native starknet compile failed"),
        &base_context,
    );
    assert!(
        !native_error_allows_scarb_fallback(&plain),
        "errors without external manifest deps must stay non-fallback"
    );

    let mut with_external = base_context.clone();
    with_external.external_non_starknet_dependencies =
        vec!["[dependencies].alexandria".to_string()];
    let eligible = mark_native_fallback_eligible_for_external_dependencies(
        anyhow::Error::msg("native starknet compile failed"),
        &with_external,
    );
    assert!(
        native_error_allows_scarb_fallback(&eligible),
        "errors with external manifest deps should be fallback-eligible"
    );
    assert!(
        format!("{eligible:#}").contains("non-starknet dependencies"),
        "fallback-eligible message should explain why scarb fallback is allowed"
    );
}

#[test]
fn ensure_native_daemon_backend_available_rejects_poisoned_state() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    set_native_daemon_backend_poisoned_for_test(false);
    mark_native_daemon_backend_poisoned();
    let err =
        ensure_native_daemon_backend_available().expect_err("poisoned daemon backend should fail");
    assert!(
        native_error_allows_scarb_fallback(&err),
        "poison rejection should be fallback-eligible in native auto mode"
    );
    set_native_daemon_backend_poisoned_for_test(false);
}

#[test]
fn resolve_native_corelib_src_prefers_explicit_env_override() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = unique_test_dir("uc-native-corelib-env-override");
    let workspace_root = dir.join("workspace");
    fs::create_dir_all(&workspace_root).expect("failed to create workspace root");

    let home = dir.join("home");
    let home_corelib = home.join(".cairo/corelib/src");
    create_mock_native_corelib(&home_corelib);

    let sibling_corelib = dir.join("cairo/corelib/src");
    create_mock_native_corelib(&sibling_corelib);

    let override_corelib = dir.join("override/corelib/src");
    create_mock_native_corelib(&override_corelib);

    let original_home = std::env::var_os("HOME");
    std::env::set_var("HOME", &home);
    std::env::set_var("UC_NATIVE_CORELIB_SRC", &override_corelib);

    let resolved = resolve_native_corelib_src(&workspace_root).expect("resolve should succeed");
    assert_eq!(
        resolved,
        override_corelib
            .canonicalize()
            .expect("failed to canonicalize override corelib")
    );

    std::env::remove_var("UC_NATIVE_CORELIB_SRC");
    if let Some(value) = original_home {
        std::env::set_var("HOME", value);
    } else {
        std::env::remove_var("HOME");
    }
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn resolve_native_corelib_src_rejects_incompatible_env_override() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = unique_test_dir("uc-native-corelib-bad-override");
    let workspace_root = dir.join("workspace");
    fs::create_dir_all(&workspace_root).expect("failed to create workspace root");

    let bad_override = dir.join("bad/corelib/src");
    fs::create_dir_all(&bad_override).expect("failed to create bad override directory");
    fs::write(bad_override.join("lib.cairo"), "fn main() {}\n")
        .expect("failed to write bad corelib lib.cairo");

    let original_home = std::env::var_os("HOME");
    std::env::set_var("HOME", dir.join("home"));
    std::env::set_var("UC_NATIVE_CORELIB_SRC", &bad_override);

    let err =
        resolve_native_corelib_src(&workspace_root).expect_err("incompatible override should fail");
    assert!(
        format!("{err:#}").contains("native corelib override"),
        "unexpected error: {err:#}"
    );

    std::env::remove_var("UC_NATIVE_CORELIB_SRC");
    if let Some(value) = original_home {
        std::env::set_var("HOME", value);
    } else {
        std::env::remove_var("HOME");
    }
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn resolve_native_corelib_src_skips_incompatible_home_and_uses_workspace_sibling() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = unique_test_dir("uc-native-corelib-fallback");
    let workspace_root = dir.join("workspace");
    fs::create_dir_all(&workspace_root).expect("failed to create workspace root");

    let home = dir.join("home");
    let home_corelib = home.join(".cairo/corelib/src");
    fs::create_dir_all(&home_corelib).expect("failed to create home corelib");
    fs::write(home_corelib.join("lib.cairo"), "fn main() {}\n")
        .expect("failed to write incompatible home corelib");

    let sibling_corelib = dir.join("cairo/corelib/src");
    create_mock_native_corelib(&sibling_corelib);

    let original_home = std::env::var_os("HOME");
    std::env::set_var("HOME", &home);
    std::env::remove_var("UC_NATIVE_CORELIB_SRC");

    let resolved = resolve_native_corelib_src(&workspace_root).expect("resolve should succeed");
    assert_eq!(
        resolved,
        sibling_corelib
            .canonicalize()
            .expect("failed to canonicalize sibling corelib")
    );

    if let Some(value) = original_home {
        std::env::set_var("HOME", value);
    } else {
        std::env::remove_var("HOME");
    }
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn resolve_native_corelib_src_skips_version_mismatched_home_candidate() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = unique_test_dir("uc-native-corelib-version-mismatch");
    let workspace_root = dir.join("workspace");
    fs::create_dir_all(&workspace_root).expect("failed to create workspace root");

    let home = dir.join("home");
    let home_corelib = home.join(".cairo/corelib/src");
    create_mock_native_corelib(&home_corelib);
    write_mock_native_corelib_manifest(&home_corelib, "0.0.1");

    let sibling_corelib = dir.join("cairo/corelib/src");
    create_mock_native_corelib(&sibling_corelib);

    let original_home = std::env::var_os("HOME");
    std::env::set_var("HOME", &home);
    std::env::remove_var("UC_NATIVE_CORELIB_SRC");

    let resolved = resolve_native_corelib_src(&workspace_root).expect("resolve should succeed");
    assert_eq!(
        resolved,
        sibling_corelib
            .canonicalize()
            .expect("failed to canonicalize sibling corelib")
    );

    if let Some(value) = original_home {
        std::env::set_var("HOME", value);
    } else {
        std::env::remove_var("HOME");
    }
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
fn async_persist_error_log_rotation_rolls_when_threshold_exceeded() {
    let dir = unique_test_dir("uc-async-log-rotation");
    let log_path = dir.join("persist-errors.log");
    fs::write(&log_path, "0123456789").expect("failed to seed async log");

    maybe_rotate_async_persist_error_log(&log_path, 5).expect("rotation should succeed");

    let rotated = dir.join("persist-errors.log.1");
    assert!(
        rotated.exists(),
        "rotated log should exist after threshold rotation"
    );
    assert!(
        !log_path.exists(),
        "original log path should be moved to rotated path"
    );
    fs::remove_dir_all(&dir).ok();
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
fn compute_build_fingerprint_is_path_portable_across_workspace_clones() {
    let _guard = integration_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let workspace_a = prepare_smoke_workspace("uc-fingerprint-clone-a");
    let workspace_b = prepare_smoke_workspace("uc-fingerprint-clone-b");
    let manifest_a = workspace_a.join("Scarb.toml");
    let manifest_b = workspace_b.join("Scarb.toml");
    let common = smoke_common_args(&manifest_a);
    let profile = effective_profile(&common);

    let fingerprint_a = compute_build_fingerprint_with_scarb_version(
        &workspace_a,
        &manifest_a,
        &common,
        &profile,
        None,
        "scarb 2.14.0 (test)",
    )
    .expect("failed to compute fingerprint for clone A");
    let fingerprint_b = compute_build_fingerprint_with_scarb_version(
        &workspace_b,
        &manifest_b,
        &common,
        &profile,
        None,
        "scarb 2.14.0 (test)",
    )
    .expect("failed to compute fingerprint for clone B");
    assert_eq!(
        fingerprint_a, fingerprint_b,
        "fingerprint should be stable across equivalent workspace clone roots"
    );

    fs::remove_dir_all(&workspace_a).ok();
    fs::remove_dir_all(&workspace_b).ok();
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
