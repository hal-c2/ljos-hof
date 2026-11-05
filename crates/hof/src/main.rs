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
    /// Open a session as it was left: its tabs, its splits, and the
    /// programs that ran on in them. A name not saved yet starts one. With
    /// no name, the session saved last that no Hof has open.
    Attach {
        /// Letters, digits, '.', '_' and '-'; `default` is a plain `hof`'s.
        name: Option<String>,
    },
    /// List the saved sessions: whether a Hof has each open, and what it
    /// holds.
    Sessions,
    /// End the programs a session kept running, here and on its hosts, and
    /// forget it. Not one a Hof has open: close its panes there.
    Forget { name: String },
    #[command(flatten)]
    Here(hof_agent::cli::Here),
}

fn main() -> Result<()> {
    let mut cli = Cli::parse();
    let attach = match cli.action.take() {
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
        Some(Action::Here(here)) => return hof_agent::cli::run(here),
        Some(Action::Sessions) => {
            print!(
                "{}",
                listing(&hof::held::list(), std::time::SystemTime::now())
            );
            return Ok(());
        }
        Some(Action::Forget { name }) => {
            let config = cli.config.unwrap_or_else(hof_config::path);
            let ended = hof::held::forget(&name, &config, &hof_agent::daemon::socket_path())?;
            if ended > 0 {
                println!("ended {ended} program{}", if ended == 1 { "" } else { "s" });
            }
            return Ok(());
        }
        Some(Action::Attach { name }) => Some(name),
        None => None,
    };
    if attach.is_some() && (cli.dropdown || cli.toggle || cli.tab || !cli.command.is_empty()) {
        anyhow::bail!(
            "`hof attach` opens a window of its own: not with --dropdown, --toggle, --tab or -e"
        );
    }
    // Held for as long as Hof runs: no other Hof keeps this session too.
    let (held, restore) = session(attach, &cli)?;
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
        session_name: held.as_ref().map(|h| h.name.clone()),
        restore,
        // This same program, which is `hof agent` too; the socket a Hof
        // elsewhere reaches it at, so it can attach these panes as well.
        agent: std::env::current_exe().ok().map(|program| hof::AgentHere {
            program,
            socket: hof_agent::daemon::socket_path(),
        }),
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

    // Ctrl+C, SIGTERM or SIGHUP closes the window as Quit does, so the
    // session is kept.
    if let Err(e) = hof::catch_signals() {
        eprintln!("hof: signals end Hof without saving its session: {e}");
    }
    let ran = hof::app().run().context("the window");
    // Before the process goes: the remote panes' agents are told whether
    // to keep their programs, or the programs would outlive their panes.
    hof_qml::remote::finish(std::time::Duration::from_secs(2));
    drop(held);
    ran?;
    Ok(())
}

/// The session this window keeps and holds, and whether to restore it in
/// place of what the config says. `hof attach` opens the one it names, or
/// the one saved last that is free; a plain `hof` the first free of
/// `default`, `2`, `3` and on. A dropdown keeps none, nor does a window for
/// a command, so it never saves over a session another keeps.
fn session(
    attach: Option<Option<String>>,
    cli: &Cli,
) -> Result<(Option<hof::held::Held>, Option<bool>)> {
    if let Some(name) = attach {
        let name = match name {
            Some(name) => name,
            None => hof::held::last_free().context(
                "no saved session is free to attach to (`hof sessions` lists them; \
                 `hof attach NAME` starts one)",
            )?,
        };
        let held = hof::held::hold(&name)
            .with_context(|| format!("the session {name}"))?
            .with_context(|| format!("another Hof has the session {name} open"))?;
        return Ok((Some(held), Some(true)));
    }
    if cli.dropdown || cli.toggle || cli.tab {
        return Ok((None, None));
    }
    if !cli.command.is_empty() {
        return Ok((None, Some(false)));
    }
    match hof::held::hold_first_free() {
        Ok(held) => Ok((Some(held), None)),
        Err(e) => {
            eprintln!("hof: this window keeps no session: {e}");
            Ok((None, Some(false)))
        }
    }
}

/// `hof sessions`: one line each, as of `now`.
fn listing(sessions: &[hof::held::Listed], now: std::time::SystemTime) -> String {
    if sessions.is_empty() {
        return "no saved sessions\n".to_owned();
    }
    let width = sessions.iter().map(|s| s.name.len()).max().unwrap_or(0);
    let mut out = String::new();
    for s in sessions {
        let held = match s.held {
            Some((tabs, panes)) => format!(
                "{tabs} tab{}, {panes} pane{}",
                if tabs == 1 { "" } else { "s" },
                if panes == 1 { "" } else { "s" }
            ),
            None => "unreadable".to_owned(),
        };
        let ago = now.duration_since(s.saved).unwrap_or_default().as_secs();
        let ago = match ago {
            0..60 => format!("{ago}s"),
            60..3600 => format!("{}m", ago / 60),
            3600..86400 => format!("{}h", ago / 3600),
            _ => format!("{}d", ago / 86400),
        };
        out.push_str(&format!(
            "{:width$}  {:8}  {held:16}  saved {ago} ago\n",
            s.name,
            if s.open { "open" } else { "detached" },
        ));
    }
    out
}

/// Where the running Hof listens for agents, as its config says.
fn mcp_socket(config: Option<PathBuf>) -> PathBuf {
    let config = config.unwrap_or_else(hof_config::path);
    hof_config::load(&config)
        .ok()
        .and_then(|l| l.config.mcp.socket)
        .unwrap_or_else(hof_mcp::socket_path)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use hof::held::Listed;

    #[test]
    fn the_sessions_are_listed_one_a_line_and_lined_up() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100_000);
        let listed = [
            Listed {
                name: "default".into(),
                open: true,
                held: Some((2, 3)),
                saved: now - Duration::from_secs(5),
            },
            Listed {
                name: "work".into(),
                open: false,
                held: Some((1, 1)),
                saved: now - Duration::from_secs(7_300),
            },
            Listed {
                name: "x".into(),
                open: false,
                held: None,
                saved: now - Duration::from_secs(90_000),
            },
        ];
        assert_eq!(
            super::listing(&listed, now),
            "default  open      2 tabs, 3 panes   saved 5s ago\n\
             work     detached  1 tab, 1 pane     saved 2h ago\n\
             x        detached  unreadable        saved 1d ago\n"
        );
        assert_eq!(super::listing(&[], now), "no saved sessions\n");
    }
}
