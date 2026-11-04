//! `hof`: opens a window, or speaks to the dropdown that is open.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

/// Ljós Hof, a terminal.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Run as a dropdown: frameless and on top, shown and hidden by
    /// `hof --toggle` (bind that to a key in your desktop).
    #[arg(long)]
    dropdown: bool,

    /// Show or hide the running dropdown; start one if none runs.
    #[arg(long, conflicts_with = "dropdown")]
    toggle: bool,

    /// Open a tab in the running dropdown, running COMMAND if given.
    #[arg(long, conflicts_with_all = ["dropdown", "toggle"])]
    tab: bool,

    /// Start in this directory.
    #[arg(short = 'd', long, value_name = "DIR")]
    working_directory: Option<PathBuf>,

    /// Read this config file in place of ~/.config/hof/hof.toml.
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// The dropdown's socket, in place of $XDG_RUNTIME_DIR/hof-dropdown.sock.
    #[arg(long, value_name = "PATH")]
    socket: Option<PathBuf>,

    /// Run this in place of the shell (everything after `-e`).
    #[arg(short = 'e', long = "command", num_args = 1.., trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<String>,

    #[command(subcommand)]
    action: Option<Action>,
}

#[derive(Debug, Subcommand)]
enum Action {
    /// Speak MCP on stdin and stdout for the running Hof: what an agent
    /// is given to start. Needs `[mcp] enabled = true`.
    Mcp {
        /// The server's socket, in place of the config's or the usual one.
        #[arg(long, value_name = "PATH")]
        socket: Option<PathBuf>,
    },
    /// Drive the running Hof from the shell: list, read and type into
    /// panes, open tabs and splits, run commands.
    Ctl {
        /// Hof's socket, in place of $HOF_SOCKET or the MCP server's.
        #[arg(long, value_name = "PATH")]
        socket: Option<PathBuf>,
        #[command(subcommand)]
        what: hof::ctl::Ctl,
    },
    /// Run remote panes' programs on this machine, for a Hof elsewhere
    /// that reaches it over ssh. Stays while it has panes or connections.
    Agent {
        /// Join stdin and stdout to the agent, starting one if none runs:
        /// what a Hof elsewhere runs through ssh.
        #[arg(long)]
        stdio: bool,
        /// The agent's socket, in place of $HOF_AGENT_SOCKET or
        /// $XDG_RUNTIME_DIR/hof-agent.sock.
        #[arg(long, value_name = "PATH")]
        socket: Option<PathBuf>,
        /// Leave the session it was started from, to outlive it.
        #[arg(long, conflicts_with = "stdio")]
        detach: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.action {
        Some(Action::Mcp { socket }) => {
            let path = socket.unwrap_or_else(|| mcp_socket(cli.config));
            return hof_mcp::bridge_stdio(&path)
                .with_context(|| format!("no Hof answers at {}", path.display()));
        }
        Some(Action::Ctl { socket, what }) => {
            let path = socket.unwrap_or_else(|| hof::ctl::socket(|| mcp_socket(cli.config)));
            let done = hof::ctl::run(&path, what)?;
            if !done.output.is_empty() {
                println!("{}", done.output);
            }
            std::process::exit(done.code);
        }
        Some(Action::Agent {
            stdio,
            socket,
            detach,
        }) => return agent(stdio, socket, detach),
        None => {}
    }
    let working_directory = match cli.working_directory {
        Some(d) => Some(std::path::absolute(&d).with_context(|| d.display().to_string())?),
        None => None,
    };
    hof::set_launch(hof::Launch {
        dropdown: cli.dropdown || cli.toggle,
        command: cli.command.clone(),
        working_directory: working_directory.clone(),
        config: cli.config,
        socket: cli.socket,
        watch_config: true,
        session: None,
    });

    if cli.toggle || cli.tab {
        let request = if cli.tab {
            hof::Request::Tab {
                directory: working_directory
                    .or_else(|| std::env::current_dir().ok())
                    .map(|d| d.display().to_string())
                    .unwrap_or_default(),
                command: cli.command,
            }
        } else {
            hof::Request::Toggle
        };
        let path = hof::socket_path();
        match hof::send_request(&path, &request) {
            Ok(()) => return Ok(()),
            // None runs: this one becomes the dropdown, shown.
            Err(_) if cli.toggle => {}
            Err(e) => {
                return Err(e).with_context(|| format!("no dropdown at {}", path.display()));
            }
        }
    }

    let ran = hof::app().run().context("the window");
    // Before the process goes: the remote panes' agents are told whether
    // to keep their programs, or the programs would outlive their panes.
    hof_qml::remote::finish(std::time::Duration::from_secs(2));
    ran?;
    Ok(())
}

fn agent(stdio: bool, socket: Option<PathBuf>, detach: bool) -> Result<()> {
    let path = socket.unwrap_or_else(hof_agent::daemon::socket_path);
    if stdio {
        let hof = std::env::current_exe().context("where hof is")?;
        return hof_agent::daemon::bridge_stdio(&path, || hof_agent::daemon::start(&hof, &path))
            .with_context(|| format!("the agent at {}", path.display()));
    }
    if detach {
        hof_agent::daemon::detach().context("leaving the session")?;
    }
    let agent = hof_agent::Agent::new(hof_agent::Options::default());
    hof_agent::daemon::serve(&agent, &path, hof_agent::daemon::LINGER)
        .with_context(|| format!("the agent at {}", path.display()))
}

/// Where the running Hof listens for agents, as its config says.
fn mcp_socket(config: Option<PathBuf>) -> PathBuf {
    let config = config.unwrap_or_else(hof_config::path);
    hof_config::load(&config)
        .ok()
        .and_then(|l| l.config.mcp.socket)
        .unwrap_or_else(hof_mcp::socket_path)
}
