//! The commands Hof's program has on any machine, the agent's own among
//! them: what `hof` and the lighter `hof-agent` both take. A machine Hof
//! copied `hof-agent` to as `hof` answers the same commands as one with
//! the whole of Hof.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Subcommand;

use crate::tty;

#[derive(Debug, Subcommand)]
pub enum Here {
    /// Put TEXT, or what comes on stdin, on the clipboard of the terminal
    /// this runs in, on whichever machine it runs (OSC 52).
    Copy {
        /// The selection, which the middle button pastes, not the clipboard.
        #[arg(short, long)]
        selection: bool,
        text: Vec<String>,
    },
    /// Print what is on the clipboard of the terminal this runs in. Hof
    /// asks the user first, unless `clipboard-read` says otherwise.
    Paste {
        #[arg(short, long)]
        selection: bool,
        /// Seconds to wait for the terminal, and the user, to answer.
        #[arg(short, long, default_value_t = 60.0)]
        timeout: f64,
    },
    /// Open a URL, or a file, with what the desktop Hof runs on opens it
    /// with, from whichever machine this runs on.
    Open { target: String },
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
        /// Have the agent running there become this program, newer,
        /// keeping its panes' programs running. It does so by itself too,
        /// when its own program is installed anew and a client connects.
        #[arg(long, conflicts_with_all = ["stdio", "detach"])]
        upgrade: bool,
        /// Take on the agent an older program handed over at this
        /// descriptor: how an upgraded agent starts.
        #[arg(long, value_name = "FD", hide = true, conflicts_with_all = ["stdio", "detach", "upgrade"])]
        resume: Option<i32>,
        /// Print the version of the handover this program takes on, and
        /// nothing else: what an agent asks before it upgrades to it.
        #[arg(long, hide = true)]
        resume_format: bool,
    },
}

/// Does what `here` says.
pub fn run(here: Here) -> Result<()> {
    match here {
        Here::Copy { selection, text } => {
            let text = if text.is_empty() {
                let mut all = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut all)
                    .context("reading stdin")?;
                all
            } else {
                text.join(" ")
            };
            Ok(tty::copy(&text, clipboard(selection))?)
        }
        Here::Paste { selection, timeout } => {
            let wait = Duration::try_from_secs_f64(timeout).context("the timeout")?;
            let text = tty::paste(clipboard(selection), wait)?;
            print!("{text}");
            Ok(())
        }
        Here::Open { target } => Ok(tty::open(&target)?),
        Here::Agent {
            resume_format: true,
            ..
        } => {
            println!("{}", crate::RESUME_FORMAT);
            Ok(())
        }
        Here::Agent {
            upgrade: true,
            socket,
            ..
        } => {
            let path = socket.unwrap_or_else(crate::daemon::socket_path);
            let program = std::env::current_exe().context("where this program is")?;
            crate::daemon::upgrade(&path, Some(&program))
                .with_context(|| format!("upgrading the agent at {}", path.display()))?;
            println!(
                "The agent at {} runs {} now.",
                path.display(),
                program.display()
            );
            Ok(())
        }
        Here::Agent {
            resume: Some(fd),
            socket,
            ..
        } => {
            let path = socket.unwrap_or_else(crate::daemon::socket_path);
            let (agent, listener) = crate::Agent::resume(crate::Options::default(), fd)
                .context("taking on the agent handed over")?;
            eprintln!("hof agent: upgraded, its panes taken on");
            crate::daemon::serve_on(&agent, listener, &path, crate::daemon::LINGER)
                .with_context(|| format!("the agent at {}", path.display()))
        }
        Here::Agent {
            stdio,
            socket,
            detach,
            ..
        } => agent(stdio, socket, detach),
    }
}

fn clipboard(selection: bool) -> hof_vt::Clipboard {
    if selection {
        hof_vt::Clipboard::Selection
    } else {
        hof_vt::Clipboard::Standard
    }
}

fn agent(stdio: bool, socket: Option<PathBuf>, detach: bool) -> Result<()> {
    let path = socket.unwrap_or_else(crate::daemon::socket_path);
    if stdio {
        // The agent it starts is this same program, whichever it is, apart
        // from the ssh login that starts it as the desktop's is apart from
        // the app: that login's end ends nothing of it.
        let program = std::env::current_exe().context("where this program is")?;
        return crate::daemon::bridge_stdio(&path, || crate::daemon::start_apart(&program, &path))
            .with_context(|| format!("the agent at {}", path.display()));
    }
    if detach {
        crate::daemon::detach().context("leaving the session")?;
    }
    let agent = crate::Agent::new(crate::Options::default());
    crate::daemon::serve(&agent, &path, crate::daemon::LINGER)
        .with_context(|| format!("the agent at {}", path.display()))
}
