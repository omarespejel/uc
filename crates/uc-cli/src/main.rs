use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::process::Command;
use uc_core::session::SessionInput;

#[derive(Parser, Debug)]
#[command(name = "uc")]
#[command(about = "uc: Cairo package manager and build/prove engine", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Benchmark(BenchmarkArgs),
    SessionKey(SessionKeyArgs),
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum MatrixArg {
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

#[derive(Args, Debug)]
struct BenchmarkArgs {
    #[arg(long, value_enum, default_value_t = MatrixArg::Research)]
    matrix: MatrixArg,

    #[arg(long, default_value_t = 5)]
    runs: u32,

    #[arg(long, default_value_t = 3)]
    cold_runs: u32,

    #[arg(long, default_value = "/Users/espejelomar/StarkNet/compiler-starknet")]
    workspace_root: String,
}

#[derive(Args, Debug)]
struct SessionKeyArgs {
    #[arg(long)]
    workspace_root: String,

    #[arg(long)]
    compiler_version: String,

    #[arg(long)]
    profile: String,

    #[arg(long, value_delimiter = ',')]
    features: Vec<String>,

    #[arg(long = "cfg", value_delimiter = ',')]
    cfg_set: Vec<String>,

    #[arg(long)]
    plugin_signature: String,

    #[arg(long)]
    target_family: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Benchmark(args) => run_benchmark(args),
        Commands::SessionKey(args) => run_session_key(args),
    }
}

fn run_benchmark(args: BenchmarkArgs) -> Result<()> {
    let root = workspace_root().context("failed to resolve workspace root")?;
    let script = root.join("benchmarks/scripts/run_local_benchmarks.sh");

    if !script.exists() {
        bail!("benchmark script not found at {}", script.display());
    }

    let status = Command::new(&script)
        .arg("--matrix")
        .arg(args.matrix.as_str())
        .arg("--runs")
        .arg(args.runs.to_string())
        .arg("--cold-runs")
        .arg(args.cold_runs.to_string())
        .arg("--workspace-root")
        .arg(args.workspace_root)
        .status()
        .context("failed to execute benchmark script")?;

    if !status.success() {
        bail!("benchmark script exited with status {status}");
    }

    Ok(())
}

fn run_session_key(args: SessionKeyArgs) -> Result<()> {
    let input = SessionInput {
        workspace_root: args.workspace_root,
        compiler_version: args.compiler_version,
        profile: args.profile,
        features: args.features,
        cfg_set: args.cfg_set,
        plugin_signature: args.plugin_signature,
        target_family: args.target_family,
    };

    println!("{}", input.deterministic_key_hex());
    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    let dir = std::env::current_dir()?;
    Ok(dir)
}
