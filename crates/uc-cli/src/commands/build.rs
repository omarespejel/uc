use super::*;

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
            let native_mode = native_build_mode();
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
                        match run_local_with_backend(
                            &native_session_key,
                            &native_compiler_version,
                            BuildCompileBackend::Native,
                        ) {
                            Ok((run, cache_hit, fingerprint, telemetry)) => {
                                Ok((run, cache_hit, fingerprint, native_session_key, telemetry))
                            }
                            Err(native_err) => {
                                eprintln!(
                                        "uc: native compile unavailable ({}), falling back to scarb backend",
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

            match daemon_mode {
                DaemonModeArg::Off => {
                    let (run, cache_hit, fingerprint, local_session_key, telemetry) =
                        run_local_for_native_mode(native_mode)?;
                    session_key = local_session_key;
                    phase_telemetry = Some(telemetry);
                    (run, cache_hit, fingerprint)
                }
                DaemonModeArg::Auto => {
                    let daemon_compile_backend = if native_mode == NativeBuildMode::Off {
                        BuildCompileBackend::Scarb
                    } else {
                        BuildCompileBackend::Native
                    };
                    let daemon_native_fallback_to_scarb = native_mode == NativeBuildMode::Auto;
                    let compiler_version = if daemon_compile_backend == BuildCompileBackend::Scarb {
                        scarb_version_line()?
                    } else {
                        native_compiler_version_line()
                    };
                    let local_session_key = build_session_input_with_compiler_version(
                        &common,
                        &manifest_path,
                        &profile,
                        &compiler_version,
                    )?
                    .deterministic_key_hex();
                    if let Some((run, fingerprint, telemetry)) = try_local_uc_cache_hit(
                        &common,
                        &manifest_path,
                        &workspace_root,
                        &profile,
                        &local_session_key,
                        &compiler_version,
                    )? {
                        session_key = local_session_key;
                        phase_telemetry = Some(telemetry);
                        (run, true, fingerprint)
                    } else if let Some(response) = try_uc_build_via_daemon(
                        &common,
                        &manifest_path,
                        true,
                        daemon_compile_backend,
                        daemon_native_fallback_to_scarb,
                    )? {
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
                DaemonModeArg::Require => {
                    let daemon_compile_backend = if native_mode == NativeBuildMode::Off {
                        BuildCompileBackend::Scarb
                    } else {
                        BuildCompileBackend::Native
                    };
                    let daemon_native_fallback_to_scarb = native_mode == NativeBuildMode::Auto;
                    let compiler_version = if daemon_compile_backend == BuildCompileBackend::Scarb {
                        scarb_version_line()?
                    } else {
                        native_compiler_version_line()
                    };
                    let local_session_key = build_session_input_with_compiler_version(
                        &common,
                        &manifest_path,
                        &profile,
                        &compiler_version,
                    )?
                    .deterministic_key_hex();
                    if let Some((run, fingerprint, telemetry)) = try_local_uc_cache_hit(
                        &common,
                        &manifest_path,
                        &workspace_root,
                        &profile,
                        &local_session_key,
                        &compiler_version,
                    )? {
                        session_key = local_session_key;
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
