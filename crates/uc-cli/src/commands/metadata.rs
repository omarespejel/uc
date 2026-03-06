use super::*;

fn should_capture_metadata_output_for_local_run(args: &MetadataArgs) -> bool {
    args.report_path.is_some()
}

pub(crate) fn run_metadata(args: MetadataArgs) -> Result<()> {
    validate_metadata_format_version(args.format_version)?;
    let manifest_path = resolve_manifest_path(&args.manifest_path)?;
    // Keep local fallback streaming by default; capture only when reporting requires buffered IO.
    let capture_local_output = should_capture_metadata_output_for_local_run(&args);

    let run = match args.daemon_mode {
        DaemonModeArg::Off => {
            run_scarb_metadata_with_uc_cache(&args, &manifest_path, capture_local_output)?
        }
        DaemonModeArg::Auto => {
            if let Some(run) = try_uc_metadata_via_daemon(&args, &manifest_path, true, true)? {
                run
            } else {
                run_scarb_metadata_with_uc_cache(&args, &manifest_path, capture_local_output)?
            }
        }
        DaemonModeArg::Require => try_uc_metadata_via_daemon(&args, &manifest_path, true, false)?
            .context("daemon mode is require but daemon is unavailable")?,
    };

    if !run.stdout.is_empty() || !run.stderr.is_empty() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_local_capture_output_mode_only_depends_on_report_generation() {
        let base = MetadataArgs {
            manifest_path: Some(PathBuf::from("/tmp/workspace/Scarb.toml")),
            format_version: 1,
            daemon_mode: DaemonModeArg::Off,
            offline: false,
            global_cache_dir: None,
            report_path: None,
        };
        assert!(
            !should_capture_metadata_output_for_local_run(&base),
            "daemon=off without report should stream output"
        );

        let mut with_report = base.clone();
        with_report.report_path = Some(PathBuf::from("/tmp/report.json"));
        assert!(
            should_capture_metadata_output_for_local_run(&with_report),
            "report generation requires buffered output capture"
        );

        let mut daemon_auto = base;
        daemon_auto.daemon_mode = DaemonModeArg::Auto;
        assert!(
            !should_capture_metadata_output_for_local_run(&daemon_auto),
            "daemon auto fallback to local should stream output when report generation is disabled"
        );
    }
}
