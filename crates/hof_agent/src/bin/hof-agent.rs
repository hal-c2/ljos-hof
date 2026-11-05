//! `hof-agent`: the part of Hof a remote machine needs, without the window
//! and the libraries it links. Hof copies it to a machine that has no Hof,
//! as `hof`, and runs `hof agent --stdio` there as it would anywhere.

use clap::Parser;

/// Ljós Hof's agent: remote panes' programs, and the commands they use to
/// reach the terminal.
#[derive(Debug, Parser)]
#[command(name = "hof", version, about)]
struct Cli {
    #[command(subcommand)]
    here: hof_agent::cli::Here,
}

fn main() -> anyhow::Result<()> {
    hof_agent::daemon::settle_runtime_dir();
    hof_agent::cli::run(Cli::parse().here)
}
