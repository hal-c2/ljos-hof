//! What the server asks of the app.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A pane, as the app numbers them. Stable for the pane's life, so an agent
/// can keep using one it got from [`Host::list_panes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PaneId(pub u64);

impl fmt::Display for PaneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// One pane, as an agent sees it in `list_panes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaneInfo {
    pub id: PaneId,
    /// The tab the pane is in. Panes that share it share a tab.
    pub tab: u64,
    pub title: String,
    /// From OSC 7, so `None` when the shell does not report it.
    pub working_directory: Option<String>,
    pub focused: bool,
    /// A command is running, as shell integration tells: `false` at a
    /// prompt, and also when the shell has no integration to tell.
    pub running: bool,
    pub columns: u16,
    pub rows: u16,
}

/// Where a split puts the new pane, next to the old one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Right,
    Down,
}

/// How a command started by [`Host::run_command`] ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    /// From the OSC 133 end-of-command mark; `None` when the shell sent
    /// the mark without one.
    pub exit_code: Option<i32>,
    /// What the command printed, without the prompt or the command line.
    pub output: String,
    /// The timeout passed first. The command is still running, and
    /// `output` is what it printed so far.
    pub timed_out: bool,
}

/// Why the app could not do what an agent asked. The message is shown to
/// the agent as is, so it should say what to do differently.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    #[error("there is no pane {0}; list_panes shows the ones there are")]
    NoSuchPane(PaneId),
    #[error(
        "the shell in pane {0} has no shell integration (OSC 133), so Hof cannot tell when a \
         command finishes; use send_input and read_screen instead"
    )]
    NoShellIntegration(PaneId),
    #[error("pane {0} is busy running a command; wait for it or interrupt it first")]
    Busy(PaneId),
    #[error("{0}")]
    Failed(String),
}

pub type HostResult<T> = Result<T, HostError>;

/// The app's side of the server: every request an agent can make.
///
/// The server calls these from its own threads, one per connection, and
/// waits for the answer, so an app whose terminals live on a UI thread
/// sends each call there and blocks on a channel for the reply. Nothing
/// here needs to be quick, but `run_command` blocks its caller until the
/// command ends or times out, so the app must not hold its UI thread for
/// that long: it should register the wait and reply when the mark arrives.
pub trait Host: Send + Sync {
    /// Every pane in every window, in an order that reads well: by tab,
    /// then by position in the tab.
    fn list_panes(&self) -> Vec<PaneInfo>;

    /// The visible screen as text, one line per row, trailing blanks
    /// trimmed.
    fn read_screen(&self, pane: PaneId) -> HostResult<String>;

    /// History followed by the visible screen, oldest first; only the last
    /// `lines` lines when given.
    fn read_scrollback(&self, pane: PaneId, lines: Option<usize>) -> HostResult<String>;

    /// Write `text` to the pane's terminal as typed keys, not as a
    /// bracketed paste, so control characters like `\r` and `\u{3}` act.
    fn send_input(&self, pane: PaneId, text: String) -> HostResult<()>;

    /// Open a tab running `command` (the user's shell when empty) in
    /// `directory` (the app's choice when `None`), on the machine `host`
    /// names (this one when `None`), and return its pane.
    fn new_tab(
        &self,
        directory: Option<String>,
        command: Vec<String>,
        host: Option<String>,
    ) -> HostResult<PaneId>;

    /// Split `pane`, putting a new pane running `command` (the user's shell
    /// when empty) to its right or below it, on the machine `host` names
    /// (`pane`'s when `None`, this one when empty), and return the new pane.
    fn split(
        &self,
        pane: PaneId,
        direction: Direction,
        command: Vec<String>,
        host: Option<String>,
    ) -> HostResult<PaneId>;

    /// Close the pane, ending what runs in it.
    fn close_pane(&self, pane: PaneId) -> HostResult<()>;

    /// Give the pane keyboard focus, switching to its tab if need be.
    fn focus_pane(&self, pane: PaneId) -> HostResult<()>;

    /// Type `command` and Enter at the pane's prompt, then wait for the
    /// shell's end-of-command mark or `timeout`, whichever comes first.
    ///
    /// The command is a single line; the server rejects any other.
    fn run_command(
        &self,
        pane: PaneId,
        command: String,
        timeout: Duration,
    ) -> HostResult<CommandResult>;
}
