use super::*;

pub(crate) fn run_metadata(args: MetadataArgs) -> Result<()> {
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
