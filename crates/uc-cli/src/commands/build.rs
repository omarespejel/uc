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
            let local_session_key =
                build_session_input(&common, &manifest_path, &profile)?.deterministic_key_hex();
            let run_local =
                |session_key: &str| -> Result<(CommandRun, bool, String, BuildPhaseTelemetry)> {
                    // Local UC builds execute Scarb directly in-process and must enforce the toolchain gate.
                    validate_scarb_toolchain()?;
                    let (run, cache_hit, fingerprint, telemetry) = run_build_with_uc_cache(
                        &common,
                        &manifest_path,
                        &workspace_root,
                        &profile,
                        session_key,
                        BuildRunOptions {
                            capture_output: false,
                            inherit_output_when_uncaptured: true,
                            async_cache_persist: false,
                            use_daemon_shared_cache: false,
                        },
                    )?;
                    Ok((run, cache_hit, fingerprint, telemetry))
                };

            match daemon_mode {
                DaemonModeArg::Off => {
                    let (run, cache_hit, fingerprint, telemetry) = run_local(&local_session_key)?;
                    session_key = local_session_key.clone();
                    phase_telemetry = Some(telemetry);
                    (run, cache_hit, fingerprint)
                }
                DaemonModeArg::Auto => {
                    if let Some((run, fingerprint, telemetry)) = try_local_uc_cache_hit(
                        &common,
                        &manifest_path,
                        &workspace_root,
                        &profile,
                        &local_session_key,
                    )? {
                        session_key = local_session_key.clone();
                        phase_telemetry = Some(telemetry);
                        (run, true, fingerprint)
                    } else if let Some(response) =
                        try_uc_build_via_daemon(&common, &manifest_path, true)?
                    {
                        daemon_used = true;
                        session_key = response.session_key;
                        phase_telemetry = Some(response.telemetry);
                        (response.run, response.cache_hit, response.fingerprint)
                    } else {
                        let (run, cache_hit, fingerprint, telemetry) =
                            run_local(&local_session_key)?;
                        session_key = local_session_key.clone();
                        phase_telemetry = Some(telemetry);
                        (run, cache_hit, fingerprint)
                    }
                }
                DaemonModeArg::Require => {
                    if let Some((run, fingerprint, telemetry)) = try_local_uc_cache_hit(
                        &common,
                        &manifest_path,
                        &workspace_root,
                        &profile,
                        &local_session_key,
                    )? {
                        session_key = local_session_key.clone();
                        phase_telemetry = Some(telemetry);
                        (run, true, fingerprint)
                    } else {
                        let response = try_uc_build_via_daemon(&common, &manifest_path, false)?
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
