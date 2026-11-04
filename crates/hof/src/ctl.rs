//! `hof ctl`: the running Hof driven from the shell, a tool call a command.
//! Scripts in a pane need no arguments to find it: Hof gives each pane
//! `HOF_SOCKET` and `HOF_PANE_ID`.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use serde_json::{Value, json};

/// What `hof ctl` does. A pane left out is the one the command runs in.
#[derive(Debug, Clone, PartialEq, Subcommand)]
pub enum Ctl {
    /// The panes of every tab, as JSON.
    List,
    /// What a pane shows, as text.
    Screen {
        #[arg(short, long)]
        pane: Option<u64>,
    },
    /// A pane's scrollback and screen, as text.
    Scrollback {
        #[arg(short, long)]
        pane: Option<u64>,
        /// Only the last LINES lines.
        #[arg(short = 'n', long)]
        lines: Option<usize>,
    },
    /// Types TEXT into a pane, as if at the keyboard.
    Send {
        #[arg(short, long)]
        pane: Option<u64>,
        /// Press enter after it.
        #[arg(short, long)]
        enter: bool,
        text: String,
    },
    /// Runs COMMAND at a pane's prompt and waits for it: prints its output
    /// and exits with its code (124 when it outlasts the timeout). Needs a
    /// shell with shell integration.
    Run {
        #[arg(short, long)]
        pane: Option<u64>,
        /// Seconds to wait.
        #[arg(short, long, default_value_t = 30.0)]
        timeout: f64,
        #[arg(required = true, num_args = 1.., trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Opens a tab, running COMMAND in place of the shell if given; prints
    /// its pane.
    Tab {
        /// Start in this directory, in place of the focused pane's.
        #[arg(short, long, value_name = "DIR")]
        directory: Option<PathBuf>,
        /// Run it on this `[hosts]` machine.
        #[arg(long, value_name = "NAME")]
        host: Option<String>,
        #[arg(num_args = 0.., trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Splits a pane, running COMMAND in the new one if given; prints its
    /// pane.
    Split {
        #[arg(short, long)]
        pane: Option<u64>,
        /// Below, in place of beside it.
        #[arg(long)]
        down: bool,
        /// Run it on this `[hosts]` machine, in place of the pane's; empty
        /// for this one.
        #[arg(long, value_name = "NAME")]
        host: Option<String>,
        #[arg(num_args = 0.., trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Gives a pane the keys, showing its tab.
    Focus {
        #[arg(short, long)]
        pane: Option<u64>,
    },
    /// Closes a pane.
    Close {
        #[arg(short, long)]
        pane: Option<u64>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("which pane? Give --pane, or run this in a Hof pane")]
    NoPane,
    #[error("no Hof answers at {0}")]
    NotRunning(PathBuf, #[source] std::io::Error),
    /// The tool's own words.
    #[error("{0}")]
    Refused(String),
}

/// What to print, and the code to exit with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    pub output: String,
    pub code: i32,
}

/// The socket `hof ctl` talks to: `HOF_SOCKET` in a pane, else the MCP
/// server's, which runs only when the config asks.
pub fn socket(mcp_socket: impl FnOnce() -> PathBuf) -> PathBuf {
    std::env::var_os("HOF_SOCKET")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(mcp_socket)
}

/// The pane `hof ctl` acts on when none is given: the one it runs in.
fn here() -> Option<u64> {
    std::env::var("HOF_PANE_ID").ok()?.parse().ok()
}

/// Does `what` on the Hof at `socket`.
pub fn run(socket: &Path, what: Ctl) -> Result<Done, Error> {
    run_in(socket, what, here())
}

fn run_in(socket: &Path, what: Ctl, here: Option<u64>) -> Result<Done, Error> {
    let pane = |p: Option<u64>| p.or(here).ok_or(Error::NoPane);
    let (tool, arguments) = match what {
        Ctl::List => ("list_panes", json!({})),
        Ctl::Screen { pane: p } => ("read_screen", json!({ "pane": pane(p)? })),
        Ctl::Scrollback { pane: p, lines } => (
            "read_scrollback",
            json!({ "pane": pane(p)?, "lines": lines }),
        ),
        Ctl::Send {
            pane: p,
            enter,
            text,
        } => (
            "send_input",
            json!({ "pane": pane(p)?, "text": text, "submit": enter }),
        ),
        Ctl::Run {
            pane: p,
            timeout,
            command,
        } => (
            "run_command",
            json!({ "pane": pane(p)?, "command": command.join(" "), "timeout_seconds": timeout }),
        ),
        Ctl::Tab {
            directory,
            host,
            command,
        } => (
            "new_tab",
            json!({
                "directory": directory.map(|d| std::path::absolute(&d).unwrap_or(d)),
                "command": command,
                "host": host,
            }),
        ),
        Ctl::Split {
            pane: p,
            down,
            host,
            command,
        } => (
            "split_pane",
            json!({
                "pane": pane(p)?,
                "direction": if down { "down" } else { "right" },
                "command": command,
                "host": host,
            }),
        ),
        Ctl::Focus { pane: p } => ("focus_pane", json!({ "pane": pane(p)? })),
        Ctl::Close { pane: p } => ("close_pane", json!({ "pane": pane(p)? })),
    };
    let answer = hof_mcp::call(socket, tool, arguments)
        .map_err(|e| Error::NotRunning(socket.to_path_buf(), e))?;
    if answer.is_error {
        return Err(Error::Refused(answer.text));
    }
    Ok(match tool {
        "run_command" => ran(&answer.text),
        "new_tab" | "split_pane" => Done {
            output: new_pane(&answer.text),
            code: 0,
        },
        "send_input" | "focus_pane" | "close_pane" => Done {
            output: String::new(),
            code: 0,
        },
        _ => Done {
            output: answer.text,
            code: 0,
        },
    })
}

/// `run_command`'s answer, a status line and the output, as the command's
/// own: its output, and its code.
fn ran(text: &str) -> Done {
    let (status, output) = text.split_once('\n').unwrap_or((text, ""));
    let code = match status.strip_prefix("exit code ") {
        Some(code) => code.parse().unwrap_or(1),
        // The timeout(1) convention.
        None if status.starts_with("still running") => 124,
        None => 0,
    };
    Done {
        output: output.to_owned(),
        code,
    }
}

/// `{"pane": 3}` as `3`.
fn new_pane(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| v["pane"].as_u64())
        .map_or_else(|| text.to_owned(), |p| p.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_s_status_becomes_its_exit_code() {
        assert_eq!(
            ran("exit code 3\nnope"),
            Done {
                output: "nope".into(),
                code: 3
            }
        );
        assert_eq!(
            ran("still running after 1s in pane 2; output so far:\nx").code,
            124
        );
        assert_eq!(
            ran("finished; the shell did not report an exit code\n").code,
            0
        );
    }

    #[test]
    fn without_a_pane_or_one_to_be_in_there_is_nothing_to_act_on() {
        let nowhere = Path::new("/nonexistent/hof.sock");
        let error = run_in(nowhere, Ctl::Screen { pane: None }, None).unwrap_err();
        assert!(matches!(error, Error::NoPane));
        // With one, it goes as far as finding no Hof.
        let error = run_in(nowhere, Ctl::Screen { pane: None }, Some(2)).unwrap_err();
        assert!(matches!(error, Error::NotRunning(..)));
    }

    #[test]
    fn a_new_pane_prints_as_its_id() {
        assert_eq!(new_pane(r#"{"pane":3}"#), "3");
    }
}
