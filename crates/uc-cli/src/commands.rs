use super::*;

pub(super) fn run_build(args: BuildArgs) -> Result<()> {
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
            let run_local =
                || -> Result<(CommandRun, bool, String, String, BuildPhaseTelemetry)> {
                    // Local UC builds execute Scarb directly in-process and must enforce the toolchain gate.
                    validate_scarb_toolchain()?;
                    let local_session_key = build_session_input(&common, &manifest_path, &profile)?
                        .deterministic_key_hex();
                    let (run, cache_hit, fingerprint, telemetry) = run_build_with_uc_cache(
                        &common,
                        &manifest_path,
                        &workspace_root,
                        &profile,
                        &local_session_key,
                        BuildRunOptions {
                            capture_output: false,
                            inherit_output_when_uncaptured: true,
                            async_cache_persist: false,
                        },
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

pub(super) fn run_metadata(args: MetadataArgs) -> Result<()> {
    validate_metadata_format_version(args.format_version)?;
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

pub(super) fn run_migrate(args: MigrateArgs) -> Result<()> {
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

pub(super) fn run_compare_build(args: CompareBuildArgs) -> Result<()> {
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
