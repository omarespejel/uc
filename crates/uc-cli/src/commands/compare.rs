use super::*;

pub(crate) fn run_compare_build(args: CompareBuildArgs) -> Result<()> {
    validate_scarb_toolchain()?;
    let common = args.common;
    let manifest_path = resolve_manifest_path(&common.manifest_path)?;
    let workspace_root = metadata_cache_workspace_root(&manifest_path)?;
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
