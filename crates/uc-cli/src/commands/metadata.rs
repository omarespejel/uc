use super::*;

pub(crate) fn run_metadata(args: MetadataArgs) -> Result<()> {
    validate_metadata_format_version(args.format_version)?;
    let manifest_path = resolve_manifest_path(&args.manifest_path)?;
    // Always capture daemon/local metadata output so local cache hits can replay
    // deterministically and reports keep full stdout/stderr content.
    let capture_output = true;

    let run = match args.daemon_mode {
        DaemonModeArg::Off => {
            run_scarb_metadata_with_uc_cache(&args, &manifest_path, capture_output)?
        }
        DaemonModeArg::Auto => {
            if let Some(run) =
                try_uc_metadata_via_daemon(&args, &manifest_path, capture_output, true)?
            {
                run
            } else {
                run_scarb_metadata_with_uc_cache(&args, &manifest_path, capture_output)?
            }
        }
        DaemonModeArg::Require => {
            try_uc_metadata_via_daemon(&args, &manifest_path, capture_output, false)?
                .context("daemon mode is require but daemon is unavailable")?
        }
    };

    replay_output(&run.stdout, &run.stderr)?;

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
