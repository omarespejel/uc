use super::*;

const DAEMON_LOCAL_PROBE_HINT_SUFFIX: &str = ".fallback-session-key";
const DAEMON_LOCAL_NATIVE_SUPPORTED_HINT_SUFFIX: &str = ".native-supported";
const DAEMON_LOCAL_PROBE_HINT_MAX_ENTRIES: usize = 256;
const DAEMON_LOCAL_PROBE_HINT_MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60;

fn daemon_local_probe_hint_path(
    workspace_root: &Path,
    primary_session_key: &str,
) -> Result<PathBuf> {
    validate_hex_digest(
        "daemon local probe primary session key",
        primary_session_key,
        SESSION_KEY_LEN,
    )?;
    let hint_dir = workspace_root.join(".uc/cache/probe-hints");
    ensure_path_within_root(
        workspace_root,
        &hint_dir,
        "daemon local probe hint directory",
    )?;
    let hint_path = hint_dir.join(format!(
        "{primary_session_key}{DAEMON_LOCAL_PROBE_HINT_SUFFIX}"
    ));
    ensure_path_within_root(workspace_root, &hint_path, "daemon local probe hint path")?;
    Ok(hint_path)
}

fn daemon_local_native_supported_hint_path(
    workspace_root: &Path,
    primary_session_key: &str,
) -> Result<PathBuf> {
    validate_hex_digest(
        "daemon local probe primary session key",
        primary_session_key,
        SESSION_KEY_LEN,
    )?;
    let hint_dir = workspace_root.join(".uc/cache/probe-hints");
    ensure_path_within_root(
        workspace_root,
        &hint_dir,
        "daemon local probe hint directory",
    )?;
    let hint_path = hint_dir.join(format!(
        "{primary_session_key}{DAEMON_LOCAL_NATIVE_SUPPORTED_HINT_SUFFIX}"
    ));
    ensure_path_within_root(workspace_root, &hint_path, "daemon local probe hint path")?;
    Ok(hint_path)
}

fn load_daemon_local_probe_hint(
    workspace_root: &Path,
    primary_session_key: &str,
) -> Result<Option<String>> {
    let hint_path = daemon_local_probe_hint_path(workspace_root, primary_session_key)?;
    let contents = match fs::read_to_string(&hint_path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            tracing::warn!(
                error = %err,
                hint_path = %hint_path.display(),
                "failed to read daemon probe hint; treating as cache miss"
            );
            return Ok(None);
        }
    };
    let hinted_session_key = contents.trim();
    if hinted_session_key.is_empty() || hinted_session_key == primary_session_key {
        return Ok(None);
    }
    if let Err(err) = validate_hex_digest(
        "daemon local probe hinted session key",
        hinted_session_key,
        SESSION_KEY_LEN,
    ) {
        tracing::warn!(
            error = %format!("{err:#}"),
            hint_path = %hint_path.display(),
            "invalid daemon probe hint value; ignoring"
        );
        return Ok(None);
    }
    Ok(Some(hinted_session_key.to_string()))
}

fn prune_daemon_local_probe_hints(hint_dir: &Path) -> Result<()> {
    if !hint_dir.exists() {
        return Ok(());
    }
    let now = SystemTime::now();
    let max_age = Duration::from_secs(DAEMON_LOCAL_PROBE_HINT_MAX_AGE_SECS);
    let mut entries = Vec::new();
    for entry in
        fs::read_dir(hint_dir).with_context(|| format!("failed to read {}", hint_dir.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read {}", hint_dir.display()))?;
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.ends_with(DAEMON_LOCAL_PROBE_HINT_SUFFIX)
            && !file_name.ends_with(DAEMON_LOCAL_NATIVE_SUPPORTED_HINT_SUFFIX)
        {
            continue;
        }
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    path = %path.display(),
                    "failed to stat hint file during pruning; skipping"
                );
                continue;
            }
        };
        let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
        if now.duration_since(modified).unwrap_or_default() > max_age {
            let _ = fs::remove_file(&path);
            continue;
        }
        entries.push((path, modified));
    }
    entries.sort_by_key(|(_, modified)| *modified);
    let stale_count = entries
        .len()
        .saturating_sub(DAEMON_LOCAL_PROBE_HINT_MAX_ENTRIES);
    for (path, _) in entries.into_iter().take(stale_count) {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

fn persist_daemon_local_probe_hint(
    workspace_root: &Path,
    primary_session_key: &str,
    hinted_session_key: &str,
) -> Result<()> {
    if hinted_session_key == primary_session_key {
        return Ok(());
    }
    validate_hex_digest(
        "daemon local probe hinted session key",
        hinted_session_key,
        SESSION_KEY_LEN,
    )?;
    let hint_path = daemon_local_probe_hint_path(workspace_root, primary_session_key)?;
    let parent = hint_path
        .parent()
        .context("daemon local probe hint path has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create daemon probe hint dir {}",
            parent.display()
        )
    })?;
    fs::write(&hint_path, format!("{hinted_session_key}\n"))
        .with_context(|| format!("failed to write daemon probe hint {}", hint_path.display()))?;
    if let Err(err) = prune_daemon_local_probe_hints(parent) {
        tracing::warn!(
            error = %format!("{err:#}"),
            hint_dir = %parent.display(),
            "failed to prune daemon probe hints"
        );
    }
    Ok(())
}

fn clear_daemon_local_probe_hint(workspace_root: &Path, primary_session_key: &str) -> Result<()> {
    let hint_path = daemon_local_probe_hint_path(workspace_root, primary_session_key)?;
    match fs::remove_file(&hint_path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err)
            .with_context(|| format!("failed to remove daemon probe hint {}", hint_path.display())),
    }
}

fn daemon_local_native_supported_hint(
    workspace_root: &Path,
    primary_session_key: &str,
) -> Result<bool> {
    let hint_path = daemon_local_native_supported_hint_path(workspace_root, primary_session_key)?;
    let contents = match fs::read_to_string(&hint_path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            tracing::warn!(
                error = %err,
                hint_path = %hint_path.display(),
                "failed to read native-supported hint; treating as unknown support"
            );
            return Ok(false);
        }
    };
    let value = contents.trim().to_ascii_lowercase();
    Ok(!value.is_empty() && !matches!(value.as_str(), "0" | "false" | "no" | "off"))
}

fn persist_daemon_local_native_supported_hint(
    workspace_root: &Path,
    primary_session_key: &str,
) -> Result<()> {
    let hint_path = daemon_local_native_supported_hint_path(workspace_root, primary_session_key)?;
    let parent = hint_path
        .parent()
        .context("native-supported hint path has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create native-supported hint dir {}",
            parent.display()
        )
    })?;
    fs::write(&hint_path, "1\n").with_context(|| {
        format!(
            "failed to write native-supported hint {}",
            hint_path.display()
        )
    })?;
    if let Err(err) = prune_daemon_local_probe_hints(parent) {
        tracing::warn!(
            error = %format!("{err:#}"),
            hint_dir = %parent.display(),
            "failed to prune daemon probe hints"
        );
    }
    Ok(())
}

fn clear_daemon_local_native_supported_hint(
    workspace_root: &Path,
    primary_session_key: &str,
) -> Result<()> {
    let hint_path = daemon_local_native_supported_hint_path(workspace_root, primary_session_key)?;
    match fs::remove_file(&hint_path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| {
            format!(
                "failed to remove native-supported hint {}",
                hint_path.display()
            )
        }),
    }
}

fn should_probe_local_before_daemon(
    daemon_mode: DaemonModeArg,
    daemon_socket_available: bool,
) -> bool {
    match daemon_mode {
        DaemonModeArg::Off => false,
        DaemonModeArg::Auto => daemon_socket_available,
        DaemonModeArg::Require => true,
    }
}

fn effective_native_mode(
    configured_mode: NativeBuildMode,
    native_auto_eligible: bool,
) -> NativeBuildMode {
    if configured_mode == NativeBuildMode::Auto && !native_auto_eligible {
        NativeBuildMode::Off
    } else {
        configured_mode
    }
}

fn daemon_backend_policy(
    configured_mode: NativeBuildMode,
    native_auto_eligible: bool,
) -> (BuildCompileBackend, bool) {
    match effective_native_mode(configured_mode, native_auto_eligible) {
        NativeBuildMode::Off => (BuildCompileBackend::Scarb, false),
        NativeBuildMode::Auto => (BuildCompileBackend::Native, true),
        NativeBuildMode::Require => (BuildCompileBackend::Native, false),
    }
}

fn daemon_socket_available_for_client() -> Result<bool> {
    #[cfg(not(unix))]
    {
        Ok(false)
    }
    #[cfg(unix)]
    {
        let socket_path = daemon_socket_path(None)?;
        Ok(socket_path.exists())
    }
}

pub(crate) fn run_build(args: BuildArgs) -> Result<()> {
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
            validate_scarb_toolchain()?;
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
            let configured_native_mode = native_build_mode();
            let native_auto_preflight_error = if configured_native_mode == NativeBuildMode::Auto {
                match native_compile_preflight(&common, &manifest_path, &workspace_root) {
                    Ok(()) => None,
                    Err(err) => {
                        if native_error_allows_scarb_fallback(&err) {
                            Some(format!("{err:#}"))
                        } else {
                            return Err(err);
                        }
                    }
                }
            } else {
                None
            };
            let native_auto_eligible = native_auto_preflight_error.is_none();
            let native_mode = effective_native_mode(configured_native_mode, native_auto_eligible);
            if let Some(reason) = native_auto_preflight_error.as_deref() {
                tracing::debug!(
                    manifest_path = %manifest_path.display(),
                    reason,
                    "native auto preflight ineligible; using scarb backend"
                );
                if configured_native_mode == NativeBuildMode::Auto {
                    if matches!(daemon_mode, DaemonModeArg::Off) {
                        eprintln!(
                            "uc: native compile unavailable ({reason}), falling back to scarb backend"
                        );
                    } else {
                        eprintln!(
                            "uc: daemon native build failed; daemon fell back to scarb backend ({reason})"
                        );
                    }
                }
            }
            let run_local_with_backend =
                |session_key: &str,
                 compiler_version: &str,
                 backend: BuildCompileBackend|
                 -> Result<(CommandRun, bool, String, BuildPhaseTelemetry)> {
                    if backend == BuildCompileBackend::Scarb {
                        // Local Scarb-backed UC builds execute Scarb directly in-process and must enforce the toolchain gate.
                        validate_scarb_toolchain()?;
                    }
                    let (run, cache_hit, fingerprint, telemetry) = run_build_with_uc_cache(
                        &common,
                        BuildCacheRunContext {
                            manifest_path: &manifest_path,
                            workspace_root: &workspace_root,
                            profile: &profile,
                            session_key,
                            compiler_version,
                            compile_backend: backend,
                            options: BuildRunOptions {
                                capture_output: false,
                                inherit_output_when_uncaptured: true,
                                async_cache_persist: false,
                                use_daemon_shared_cache: false,
                            },
                        },
                    )?;
                    Ok((run, cache_hit, fingerprint, telemetry))
                };

            let run_local_for_native_mode = |mode: NativeBuildMode| -> Result<(
                CommandRun,
                bool,
                String,
                String,
                BuildPhaseTelemetry,
            )> {
                match mode {
                    NativeBuildMode::Off => {
                        let compiler_version = scarb_version_line()?;
                        let local_session_key = build_session_input_with_compiler_version(
                            &common,
                            &manifest_path,
                            &profile,
                            &compiler_version,
                        )?
                        .deterministic_key_hex();
                        let (run, cache_hit, fingerprint, telemetry) = run_local_with_backend(
                            &local_session_key,
                            &compiler_version,
                            BuildCompileBackend::Scarb,
                        )?;
                        Ok((run, cache_hit, fingerprint, local_session_key, telemetry))
                    }
                    NativeBuildMode::Auto => {
                        let native_compiler_version = native_compiler_version_line();
                        let native_session_key = build_session_input_with_compiler_version(
                            &common,
                            &manifest_path,
                            &profile,
                            &native_compiler_version,
                        )?
                        .deterministic_key_hex();
                        let build_scarb_fallback_context = || -> Result<(String, String)> {
                            let compiler_version = scarb_version_line()?;
                            let session_key = build_session_input_with_compiler_version(
                                &common,
                                &manifest_path,
                                &profile,
                                &compiler_version,
                            )?
                            .deterministic_key_hex();
                            Ok((compiler_version, session_key))
                        };
                        if let Some(hinted_session_key) =
                            load_daemon_local_probe_hint(&workspace_root, &native_session_key)?
                        {
                            let (compiler_version, local_session_key) =
                                build_scarb_fallback_context()?;
                            let (run, cache_hit, fingerprint, telemetry) = run_local_with_backend(
                                &local_session_key,
                                &compiler_version,
                                BuildCompileBackend::Scarb,
                            )?;
                            if hinted_session_key != local_session_key {
                                let _ = persist_daemon_local_probe_hint(
                                    &workspace_root,
                                    &native_session_key,
                                    &local_session_key,
                                );
                            }
                            let _ = clear_daemon_local_native_supported_hint(
                                &workspace_root,
                                &native_session_key,
                            );
                            return Ok((run, cache_hit, fingerprint, local_session_key, telemetry));
                        }
                        match run_local_with_backend(
                            &native_session_key,
                            &native_compiler_version,
                            BuildCompileBackend::Native,
                        ) {
                            Ok((run, cache_hit, fingerprint, telemetry)) => {
                                let _ = clear_daemon_local_probe_hint(
                                    &workspace_root,
                                    &native_session_key,
                                );
                                let _ = persist_daemon_local_native_supported_hint(
                                    &workspace_root,
                                    &native_session_key,
                                );
                                Ok((run, cache_hit, fingerprint, native_session_key, telemetry))
                            }
                            Err(native_err) => {
                                if !native_error_allows_scarb_fallback(&native_err) {
                                    return Err(native_err);
                                }
                                eprintln!(
                                    "uc: native compile unavailable ({:#}), falling back to scarb backend",
                                    native_err
                                );
                                let (compiler_version, local_session_key) =
                                    build_scarb_fallback_context()?;
                                let (run, cache_hit, fingerprint, telemetry) =
                                    run_local_with_backend(
                                        &local_session_key,
                                        &compiler_version,
                                        BuildCompileBackend::Scarb,
                                    )?;
                                let _ = persist_daemon_local_probe_hint(
                                    &workspace_root,
                                    &native_session_key,
                                    &local_session_key,
                                );
                                let _ = clear_daemon_local_native_supported_hint(
                                    &workspace_root,
                                    &native_session_key,
                                );
                                Ok((run, cache_hit, fingerprint, local_session_key, telemetry))
                            }
                        }
                    }
                    NativeBuildMode::Require => {
                        let native_compiler_version = native_compiler_version_line();
                        let native_session_key = build_session_input_with_compiler_version(
                            &common,
                            &manifest_path,
                            &profile,
                            &native_compiler_version,
                        )?
                        .deterministic_key_hex();
                        let (run, cache_hit, fingerprint, telemetry) = run_local_with_backend(
                            &native_session_key,
                            &native_compiler_version,
                            BuildCompileBackend::Native,
                        )
                        .context("native compile mode is require but native backend failed")?;
                        Ok((run, cache_hit, fingerprint, native_session_key, telemetry))
                    }
                }
            };
            let build_session_key_for_compiler = |compiler_version: &str| -> Result<String> {
                Ok(build_session_input_with_compiler_version(
                    &common,
                    &manifest_path,
                    &profile,
                    compiler_version,
                )?
                .deterministic_key_hex())
            };
            let resolve_daemon_backend_context =
                || -> Result<(BuildCompileBackend, bool, String, String)> {
                    let (daemon_compile_backend, daemon_native_fallback_to_scarb) =
                        daemon_backend_policy(configured_native_mode, native_auto_eligible);
                    let compiler_version = if daemon_compile_backend == BuildCompileBackend::Scarb {
                        scarb_version_line()?
                    } else {
                        native_compiler_version_line()
                    };
                    let local_session_key = build_session_key_for_compiler(&compiler_version)?;
                    Ok((
                        daemon_compile_backend,
                        daemon_native_fallback_to_scarb,
                        compiler_version,
                        local_session_key,
                    ))
                };
            let try_daemon_local_probe = |primary_session_key: &str,
                                          primary_compiler_version: &str,
                                          include_scarb_fallback_probe: bool|
             -> Result<
                Option<(CommandRun, String, BuildPhaseTelemetry, String)>,
            > {
                let mut scarb_probe_context: Option<(String, String)> = None;
                let mut resolve_scarb_probe_context = || -> Result<(String, String)> {
                    if let Some((compiler_version, session_key)) = scarb_probe_context.as_ref() {
                        return Ok((compiler_version.clone(), session_key.clone()));
                    }
                    let compiler_version = scarb_version_line()?;
                    let session_key = build_session_key_for_compiler(&compiler_version)?;
                    scarb_probe_context = Some((compiler_version.clone(), session_key.clone()));
                    Ok((compiler_version, session_key))
                };
                let native_supported_hint = if include_scarb_fallback_probe {
                    daemon_local_native_supported_hint(&workspace_root, primary_session_key)?
                } else {
                    false
                };
                let mut tried_current_scarb_probe = false;
                if include_scarb_fallback_probe && !native_supported_hint {
                    if let Some(hinted_session_key) =
                        load_daemon_local_probe_hint(&workspace_root, primary_session_key)?
                    {
                        let (scarb_compiler_version, scarb_session_key) =
                            resolve_scarb_probe_context()?;
                        if hinted_session_key != scarb_session_key {
                            let _ = persist_daemon_local_probe_hint(
                                &workspace_root,
                                primary_session_key,
                                &scarb_session_key,
                            );
                        } else {
                            tried_current_scarb_probe = true;
                            if let Some((run, fingerprint, telemetry)) = try_local_uc_cache_hit(
                                &common,
                                &manifest_path,
                                &workspace_root,
                                &profile,
                                &scarb_session_key,
                                &scarb_compiler_version,
                            )? {
                                return Ok(Some((run, fingerprint, telemetry, scarb_session_key)));
                            }
                        }
                    }
                }
                if let Some((run, fingerprint, telemetry)) = try_local_uc_cache_hit(
                    &common,
                    &manifest_path,
                    &workspace_root,
                    &profile,
                    primary_session_key,
                    primary_compiler_version,
                )? {
                    if include_scarb_fallback_probe {
                        let _ = clear_daemon_local_probe_hint(&workspace_root, primary_session_key);
                        let _ = persist_daemon_local_native_supported_hint(
                            &workspace_root,
                            primary_session_key,
                        );
                    }
                    return Ok(Some((
                        run,
                        fingerprint,
                        telemetry,
                        primary_session_key.to_string(),
                    )));
                }
                if include_scarb_fallback_probe
                    && !native_supported_hint
                    && !tried_current_scarb_probe
                {
                    let (scarb_compiler_version, scarb_session_key) =
                        resolve_scarb_probe_context()?;
                    if scarb_session_key != primary_session_key {
                        if let Some((run, fingerprint, telemetry)) = try_local_uc_cache_hit(
                            &common,
                            &manifest_path,
                            &workspace_root,
                            &profile,
                            &scarb_session_key,
                            &scarb_compiler_version,
                        )? {
                            let _ = persist_daemon_local_probe_hint(
                                &workspace_root,
                                primary_session_key,
                                &scarb_session_key,
                            );
                            let _ = clear_daemon_local_native_supported_hint(
                                &workspace_root,
                                primary_session_key,
                            );
                            return Ok(Some((run, fingerprint, telemetry, scarb_session_key)));
                        }
                    }
                }
                Ok(None)
            };

            match daemon_mode {
                DaemonModeArg::Off => {
                    let (run, cache_hit, fingerprint, local_session_key, telemetry) =
                        run_local_for_native_mode(native_mode)?;
                    session_key = local_session_key;
                    phase_telemetry = Some(telemetry);
                    (run, cache_hit, fingerprint)
                }
                DaemonModeArg::Auto => {
                    let daemon_socket_available = daemon_socket_available_for_client()?;
                    if !should_probe_local_before_daemon(
                        DaemonModeArg::Auto,
                        daemon_socket_available,
                    ) {
                        let (run, cache_hit, fingerprint, fallback_session_key, telemetry) =
                            run_local_for_native_mode(native_mode)?;
                        session_key = fallback_session_key;
                        phase_telemetry = Some(telemetry);
                        (run, cache_hit, fingerprint)
                    } else {
                        let (
                            daemon_compile_backend,
                            daemon_native_fallback_to_scarb,
                            compiler_version,
                            local_session_key,
                        ) = resolve_daemon_backend_context()?;
                        if let Some((run, fingerprint, telemetry, hit_session_key)) =
                            try_daemon_local_probe(
                                &local_session_key,
                                &compiler_version,
                                daemon_native_fallback_to_scarb,
                            )?
                        {
                            session_key = hit_session_key;
                            phase_telemetry = Some(telemetry);
                            (run, true, fingerprint)
                        } else if let Some(response) = try_uc_build_via_daemon(
                            &common,
                            &manifest_path,
                            true,
                            daemon_compile_backend,
                            daemon_native_fallback_to_scarb,
                        )? {
                            if daemon_native_fallback_to_scarb
                                && response.compile_backend == DaemonBuildBackend::Scarb
                                && response.session_key != local_session_key
                            {
                                let _ = persist_daemon_local_probe_hint(
                                    &workspace_root,
                                    &local_session_key,
                                    &response.session_key,
                                );
                                let _ = clear_daemon_local_native_supported_hint(
                                    &workspace_root,
                                    &local_session_key,
                                );
                            } else if response.session_key == local_session_key {
                                let _ = clear_daemon_local_probe_hint(
                                    &workspace_root,
                                    &local_session_key,
                                );
                                if response.compile_backend == DaemonBuildBackend::Native {
                                    let _ = persist_daemon_local_native_supported_hint(
                                        &workspace_root,
                                        &local_session_key,
                                    );
                                }
                            }
                            daemon_used = true;
                            session_key = response.session_key;
                            phase_telemetry = Some(response.telemetry);
                            (response.run, response.cache_hit, response.fingerprint)
                        } else {
                            let (run, cache_hit, fingerprint, fallback_session_key, telemetry) =
                                run_local_for_native_mode(native_mode)?;
                            session_key = fallback_session_key;
                            phase_telemetry = Some(telemetry);
                            (run, cache_hit, fingerprint)
                        }
                    }
                }
                DaemonModeArg::Require => {
                    let (
                        daemon_compile_backend,
                        daemon_native_fallback_to_scarb,
                        compiler_version,
                        local_session_key,
                    ) = resolve_daemon_backend_context()?;
                    if let Some((run, fingerprint, telemetry, hit_session_key)) =
                        try_daemon_local_probe(
                            &local_session_key,
                            &compiler_version,
                            daemon_native_fallback_to_scarb,
                        )?
                    {
                        session_key = hit_session_key;
                        phase_telemetry = Some(telemetry);
                        (run, true, fingerprint)
                    } else {
                        let response = try_uc_build_via_daemon(
                            &common,
                            &manifest_path,
                            false,
                            daemon_compile_backend,
                            daemon_native_fallback_to_scarb,
                        )?
                        .context("daemon mode is require but daemon is unavailable")?;
                        if daemon_native_fallback_to_scarb
                            && response.compile_backend == DaemonBuildBackend::Scarb
                            && response.session_key != local_session_key
                        {
                            let _ = persist_daemon_local_probe_hint(
                                &workspace_root,
                                &local_session_key,
                                &response.session_key,
                            );
                            let _ = clear_daemon_local_native_supported_hint(
                                &workspace_root,
                                &local_session_key,
                            );
                        } else if response.session_key == local_session_key {
                            let _ =
                                clear_daemon_local_probe_hint(&workspace_root, &local_session_key);
                            if response.compile_backend == DaemonBuildBackend::Native {
                                let _ = persist_daemon_local_native_supported_hint(
                                    &workspace_root,
                                    &local_session_key,
                                );
                            }
                        }
                        daemon_used = true;
                        session_key = response.session_key;
                        phase_telemetry = Some(response.telemetry);
                        (response.run, response.cache_hit, response.fingerprint)
                    }
                }
            }
        }
    };
    replay_output(&run.stdout, &run.stderr)?;
    if should_log_phase_telemetry() {
        if let Some(telemetry) = phase_telemetry.as_ref() {
            eprintln!(
                "uc: phase timings (ms) fingerprint={:.3} cache_lookup={:.3} cache_restore={:.3} compile={:.3} cache_persist={:.3} async={} scheduled={} daemon_used={} cache_hit={} native_context={:.3} native_target_dir={:.3} native_session_prepare={:.3} native_frontend_compile={:.3} native_casm={:.3} native_artifact_write={:.3}",
                telemetry.fingerprint_ms,
                telemetry.cache_lookup_ms,
                telemetry.cache_restore_ms,
                telemetry.compile_ms,
                telemetry.cache_persist_ms,
                telemetry.cache_persist_async,
                telemetry.cache_persist_scheduled,
                daemon_used,
                cache_hit,
                telemetry.native_context_ms,
                telemetry.native_target_dir_ms,
                telemetry.native_session_prepare_ms,
                telemetry.native_frontend_compile_ms,
                telemetry.native_casm_ms,
                telemetry.native_artifact_write_ms
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

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_test_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).expect("failed to create test directory");
        dir
    }

    #[test]
    fn daemon_local_probe_hint_roundtrip_and_clear() {
        let workspace = unique_test_dir("uc-daemon-probe-hint");
        let primary_session_key = "a".repeat(SESSION_KEY_LEN);
        let hinted_session_key = "b".repeat(SESSION_KEY_LEN);

        persist_daemon_local_probe_hint(&workspace, &primary_session_key, &hinted_session_key)
            .expect("failed to persist daemon local probe hint");
        let loaded = load_daemon_local_probe_hint(&workspace, &primary_session_key)
            .expect("failed to load daemon local probe hint");
        assert_eq!(
            loaded,
            Some(hinted_session_key.clone()),
            "persisted probe hint should be readable"
        );

        clear_daemon_local_probe_hint(&workspace, &primary_session_key)
            .expect("failed to clear daemon local probe hint");
        let cleared = load_daemon_local_probe_hint(&workspace, &primary_session_key)
            .expect("failed to load daemon local probe hint after clear");
        assert!(
            cleared.is_none(),
            "probe hint should be removed after clear operation"
        );

        fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn daemon_local_native_supported_hint_roundtrip_and_clear() {
        let workspace = unique_test_dir("uc-daemon-native-supported-hint");
        let primary_session_key = "c".repeat(SESSION_KEY_LEN);

        persist_daemon_local_native_supported_hint(&workspace, &primary_session_key)
            .expect("failed to persist native-supported hint");
        assert!(
            daemon_local_native_supported_hint(&workspace, &primary_session_key)
                .expect("failed to read native-supported hint"),
            "persisted native-supported hint should be readable"
        );

        clear_daemon_local_native_supported_hint(&workspace, &primary_session_key)
            .expect("failed to clear native-supported hint");
        assert!(
            !daemon_local_native_supported_hint(&workspace, &primary_session_key)
                .expect("failed to read native-supported hint after clear"),
            "native-supported hint should be removed after clear operation"
        );

        fs::remove_dir_all(&workspace).ok();
    }

    #[cfg(unix)]
    #[test]
    fn prune_daemon_local_probe_hints_skips_dangling_entries() {
        let workspace = unique_test_dir("uc-daemon-probe-hint-prune");
        let hint_dir = workspace.join(".uc/cache/probe-hints");
        fs::create_dir_all(&hint_dir).expect("failed to create hint directory");

        let valid_key = "d".repeat(SESSION_KEY_LEN);
        let valid_hint = hint_dir.join(format!("{valid_key}{DAEMON_LOCAL_PROBE_HINT_SUFFIX}"));
        fs::write(&valid_hint, format!("{}\n", "e".repeat(SESSION_KEY_LEN)))
            .expect("failed to write valid hint");

        let dangling_key = "f".repeat(SESSION_KEY_LEN);
        let dangling_hint =
            hint_dir.join(format!("{dangling_key}{DAEMON_LOCAL_PROBE_HINT_SUFFIX}"));
        let missing_target = hint_dir.join("missing-target");
        std::os::unix::fs::symlink(&missing_target, &dangling_hint)
            .expect("failed to create dangling hint symlink");

        prune_daemon_local_probe_hints(&hint_dir)
            .expect("prune should ignore dangling hint entries");
        assert!(
            valid_hint.exists(),
            "valid hint should remain after pruning dangling entries"
        );

        fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn should_probe_local_before_daemon_follows_mode_policy() {
        assert!(
            !should_probe_local_before_daemon(DaemonModeArg::Off, false),
            "daemon off should never probe local pre-daemon path"
        );
        assert!(
            !should_probe_local_before_daemon(DaemonModeArg::Off, true),
            "daemon off should never probe local pre-daemon path"
        );

        assert!(
            !should_probe_local_before_daemon(DaemonModeArg::Auto, false),
            "daemon auto should skip pre-daemon probe when daemon socket is unavailable"
        );
        assert!(
            should_probe_local_before_daemon(DaemonModeArg::Auto, true),
            "daemon auto should probe local cache when daemon socket is available"
        );

        assert!(
            should_probe_local_before_daemon(DaemonModeArg::Require, false),
            "daemon require should still probe local cache first to preserve cache-hit fast path"
        );
        assert!(
            should_probe_local_before_daemon(DaemonModeArg::Require, true),
            "daemon require should probe local cache when daemon is available"
        );
    }

    #[test]
    fn effective_native_mode_downgrades_auto_when_preflight_is_ineligible() {
        assert_eq!(
            effective_native_mode(NativeBuildMode::Auto, true),
            NativeBuildMode::Auto
        );
        assert_eq!(
            effective_native_mode(NativeBuildMode::Auto, false),
            NativeBuildMode::Off
        );
        assert_eq!(
            effective_native_mode(NativeBuildMode::Off, false),
            NativeBuildMode::Off
        );
        assert_eq!(
            effective_native_mode(NativeBuildMode::Require, false),
            NativeBuildMode::Require
        );
    }

    #[test]
    fn daemon_backend_policy_matches_effective_native_mode() {
        assert_eq!(
            daemon_backend_policy(NativeBuildMode::Off, false),
            (BuildCompileBackend::Scarb, false)
        );
        assert_eq!(
            daemon_backend_policy(NativeBuildMode::Auto, true),
            (BuildCompileBackend::Native, true)
        );
        assert_eq!(
            daemon_backend_policy(NativeBuildMode::Auto, false),
            (BuildCompileBackend::Scarb, false)
        );
        assert_eq!(
            daemon_backend_policy(NativeBuildMode::Require, false),
            (BuildCompileBackend::Native, false)
        );
    }
}
