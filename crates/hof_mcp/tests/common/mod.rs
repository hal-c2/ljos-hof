//! A Host that remembers what it was asked and answers from a script.

#![allow(dead_code)]

use std::sync::Mutex;
use std::time::Duration;

use hof_mcp::{CommandResult, Direction, Host, HostError, HostResult, PaneId, PaneInfo};

pub struct FakeHost {
    pub panes: Vec<PaneInfo>,
    pub calls: Mutex<Vec<String>>,
    pub command_result: CommandResult,
}

impl FakeHost {
    pub fn new() -> Self {
        Self {
            panes: vec![
                pane(1, 0, "zsh", true),
                PaneInfo {
                    running: true,
                    ..pane(2, 0, "cargo test", false)
                },
            ],
            calls: Mutex::new(Vec::new()),
            command_result: CommandResult {
                exit_code: Some(0),
                output: "hello\n".into(),
                timed_out: false,
            },
        }
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }

    fn known(&self, pane: PaneId) -> HostResult<()> {
        if self.panes.iter().any(|p| p.id == pane) {
            Ok(())
        } else {
            Err(HostError::NoSuchPane(pane))
        }
    }
}

pub fn pane(id: u64, tab: u64, title: &str, focused: bool) -> PaneInfo {
    PaneInfo {
        id: PaneId(id),
        tab,
        title: title.into(),
        working_directory: Some("/home/user".into()),
        focused,
        running: false,
        columns: 80,
        rows: 24,
    }
}

impl Host for FakeHost {
    fn list_panes(&self) -> Vec<PaneInfo> {
        self.record("list_panes".into());
        self.panes.clone()
    }

    fn read_screen(&self, pane: PaneId) -> HostResult<String> {
        self.known(pane)?;
        self.record(format!("read_screen {pane}"));
        Ok(format!("$ screen of pane {pane}"))
    }

    fn read_scrollback(&self, pane: PaneId, lines: Option<usize>) -> HostResult<String> {
        self.known(pane)?;
        self.record(format!("read_scrollback {pane} {lines:?}"));
        Ok("old line\n$ screen".into())
    }

    fn send_input(&self, pane: PaneId, text: String) -> HostResult<()> {
        self.known(pane)?;
        self.record(format!("send_input {pane} {text:?}"));
        Ok(())
    }

    fn new_tab(
        &self,
        directory: Option<String>,
        command: Vec<String>,
        host: Option<String>,
    ) -> HostResult<PaneId> {
        self.record(format!("new_tab {directory:?} {command:?}{}", on(host)));
        Ok(PaneId(10))
    }

    fn split(
        &self,
        pane: PaneId,
        direction: Direction,
        command: Vec<String>,
        host: Option<String>,
    ) -> HostResult<PaneId> {
        self.known(pane)?;
        self.record(format!(
            "split {pane} {direction:?} {command:?}{}",
            on(host)
        ));
        Ok(PaneId(11))
    }

    fn close_pane(&self, pane: PaneId) -> HostResult<()> {
        self.known(pane)?;
        self.record(format!("close_pane {pane}"));
        Ok(())
    }

    fn focus_pane(&self, pane: PaneId) -> HostResult<()> {
        self.known(pane)?;
        self.record(format!("focus_pane {pane}"));
        Ok(())
    }

    fn run_command(
        &self,
        pane: PaneId,
        command: String,
        timeout: Duration,
    ) -> HostResult<CommandResult> {
        self.known(pane)?;
        self.record(format!("run_command {pane} {command:?} {timeout:?}"));
        Ok(self.command_result.clone())
    }
}

/// How a call's host is recorded: nothing when it gave none.
fn on(host: Option<String>) -> String {
    host.map(|h| format!(" on {h:?}")).unwrap_or_default()
}
