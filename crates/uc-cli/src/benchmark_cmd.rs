use anyhow::{bail, Context, Result};
use clap::{Args, ValueEnum};
use std::path::PathBuf;
use std::process::Command;

use crate::parse_env_bool;

#[derive(Copy, Clone, Debug, ValueEnum)]
pub(crate) enum MatrixArg {
    Research,
    Smoke,
}

impl MatrixArg {
    fn as_str(self) -> &'static str {
        match self {
            MatrixArg::Research => "research",
            MatrixArg::Smoke => "smoke",
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub(crate) enum BenchmarkToolArg {
    Scarb,
    Uc,
}

impl BenchmarkToolArg {
    fn as_str(self) -> &'static str {
        match self {
            BenchmarkToolArg::Scarb => "scarb",
            BenchmarkToolArg::Uc => "uc",
        }
    }
}

#[derive(Args, Debug)]
pub(crate) struct BenchmarkArgs {
    #[arg(long, value_enum, default_value_t = MatrixArg::Research)]
    pub(crate) matrix: MatrixArg,

    #[arg(long, value_enum, default_value_t = BenchmarkToolArg::Scarb)]
    pub(crate) tool: BenchmarkToolArg,

    #[arg(long, default_value_t = 5)]
    pub(crate) runs: u32,

    #[arg(long, default_value_t = 3)]
    pub(crate) cold_runs: u32,

    #[arg(long)]
    pub(crate) workspace_root: Option<String>,
}

pub(crate) fn run(args: BenchmarkArgs) -> Result<()> {
    if !parse_env_bool("UC_ENABLE_BENCHMARK_COMMAND", false) {
        bail!(
            "`uc benchmark` is a development-only command. Set UC_ENABLE_BENCHMARK_COMMAND=1 to enable it."
        );
    }
    let script = if let Some(path) = std::env::var_os("UC_BENCHMARK_SCRIPT") {
        PathBuf::from(path)
    } else if let Some(root) = std::env::var_os("UC_BENCHMARK_REPO_ROOT") {
        PathBuf::from(root).join("benchmarks/scripts/run_local_benchmarks.sh")
    } else {
        bail!("`uc benchmark` requires UC_BENCHMARK_SCRIPT or UC_BENCHMARK_REPO_ROOT to be set");
    };

    if !script.exists() {
        bail!(
            "benchmark script not found at {}. Set UC_BENCHMARK_SCRIPT or UC_BENCHMARK_REPO_ROOT.",
            script.display()
        );
    }

    let mut command = Command::new(&script);
    command
        .arg("--matrix")
        .arg(args.matrix.as_str())
        .arg("--tool")
        .arg(args.tool.as_str())
        .arg("--runs")
        .arg(args.runs.to_string())
        .arg("--cold-runs")
        .arg(args.cold_runs.to_string());
    if let Some(workspace_root) = args.workspace_root {
        command.arg("--workspace-root").arg(workspace_root);
    }
    let status = command
        .status()
        .context("failed to execute benchmark script")?;

    if !status.success() {
        bail!("benchmark script exited with status {status}");
    }

    Ok(())
}
