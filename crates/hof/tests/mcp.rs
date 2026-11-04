//! What an agent can do to Hof's window over MCP, through the host the
//! server is given. Each call is made from another thread, as the server
//! makes it, while this thread runs the window.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use hof_mcp::{Direction, Host, HostError, PaneId};
use ljos::prelude::*;
use ljos::testing::TestWindow;

/// A shell as small as can be that marks its prompts and commands the way
/// shell integration does (OSC 133), so the host can tell where a
/// command's output starts and ends.
const MARKING_SHELL: &str = r#"#!/bin/sh
while :; do
  printf '\033]133;A\007$ \033]133;B\007'
  IFS= read -r line || exit 0
  printf '\033]133;C\007'
  sh -c "$line"
  printf '\033]133;D;%d\007' "$?"
done
"#;

fn shell() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("mcp");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("marking-shell");
    std::fs::write(&path, MARKING_SHELL).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

struct Hof {
    engine: Engine,
    window: TestWindow,
    root: ObjectId,
    host: Arc<dyn Host>,
}

fn opened() -> Hof {
    hof::set_launch(hof::Launch {
        command: vec![shell().display().to_string()],
        config: Some(Path::new(env!("CARGO_TARGET_TMPDIR")).join("none/hof.toml")),
        ..hof::Launch::default()
    });
    let (engine, root) = hof::app().build().expect("the window builds");
    let mut window = TestWindow::new(&engine, 800.0, 400.0);
    window.show(root).expect("the window shows");
    window.settle().unwrap();
    let host = hof::mcp_host(&engine, root);
    Hof {
        engine,
        window,
        root,
        host,
    }
}

impl Hof {
    /// What `ask` returns, asked from another thread while this one runs
    /// the window.
    fn agent<T: Send + 'static>(&mut self, ask: impl FnOnce(&dyn Host) -> T + Send + 'static) -> T {
        let host = Arc::clone(&self.host);
        let asking = thread::spawn(move || ask(&*host));
        let deadline = Instant::now() + Duration::from_secs(20);
        while !asking.is_finished() {
            assert!(Instant::now() < deadline, "the agent got no answer");
            self.window.advance(16).unwrap();
            thread::sleep(Duration::from_millis(2));
        }
        asking.join().unwrap()
    }

    /// Runs the window until `done`.
    fn until(&mut self, what: &str, mut done: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(self) {
            assert!(Instant::now() < deadline, "never {what}");
            self.window.advance(16).unwrap();
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn first(&self) -> PaneId {
        PaneId(
            self.engine
                .read_by_name(self.root, "layout")
                .as_object()
                .map_or(0.0, |l| {
                    self.engine.read_by_name(l, "focusedPane").to_number()
                }) as u64,
        )
    }

    fn prompted(&mut self) {
        let root = self.root;
        self.until("prompted", |h| {
            h.engine
                .read_by_name(root, "current")
                .as_object()
                .is_some_and(|t| h.engine.read_by_name(t, "shellIntegration").to_boolean())
        });
    }
}

#[test]
fn an_agent_sees_the_panes_and_what_they_show() {
    let mut hof = opened();
    hof.prompted();
    let first = hof.first();

    let panes = hof.agent(|h| h.list_panes());
    assert_eq!(panes.len(), 1);
    assert_eq!(panes[0].id, first);
    assert!(panes[0].focused);
    assert!(panes[0].columns > 0 && panes[0].rows > 0);

    let screen = hof.agent(move |h| h.read_screen(first)).unwrap();
    assert_eq!(screen, "$");

    let missing = hof.agent(|h| h.read_screen(PaneId(99)));
    assert_eq!(missing, Err(HostError::NoSuchPane(PaneId(99))));
}

#[test]
fn an_agent_runs_a_command_and_gets_its_output_and_exit_code() {
    let mut hof = opened();
    hof.prompted();
    let first = hof.first();

    let ran = hof
        .agent(move |h| h.run_command(first, "echo zqxj; exit 3".into(), Duration::from_secs(10)))
        .unwrap();
    assert!(!ran.timed_out);
    assert_eq!(ran.output, "zqxj");
    assert_eq!(ran.exit_code, Some(3));

    // The pane is at its prompt again, ready for the next.
    let again = hof
        .agent(move |h| h.run_command(first, "true".into(), Duration::from_secs(10)))
        .unwrap();
    assert_eq!((again.output.as_str(), again.exit_code), ("", Some(0)));
}

#[test]
fn a_command_that_runs_too_long_comes_back_with_what_it_printed() {
    let mut hof = opened();
    hof.prompted();
    let first = hof.first();

    let ran = hof
        .agent(move |h| {
            h.run_command(
                first,
                "echo early; sleep 5".into(),
                Duration::from_millis(500),
            )
        })
        .unwrap();
    assert!(ran.timed_out);
    assert_eq!(ran.exit_code, None);
    assert_eq!(ran.output, "early");

    let busy = hof.agent(move |h| h.run_command(first, "true".into(), Duration::from_secs(1)));
    assert_eq!(busy.unwrap_err(), HostError::Busy(first), "it still runs");
}

#[test]
fn an_agent_splits_types_into_and_closes_panes() {
    let mut hof = opened();
    hof.prompted();
    let first = hof.first();

    let second = hof
        .agent(move |h| h.split(first, Direction::Right, vec!["cat".into()], None))
        .unwrap();
    assert_ne!(second, first);
    assert_eq!(hof.agent(|h| h.list_panes()).len(), 2);

    // `cat` marks nothing: a command cannot be told from its output there.
    let unmarked = hof.agent(move |h| h.run_command(second, "ls".into(), Duration::from_secs(1)));
    assert_eq!(unmarked, Err(HostError::NoShellIntegration(second)));

    hof.agent(move |h| h.send_input(second, "zqxj\r".into()))
        .unwrap();
    hof.until("echoed", |h| {
        let t = h
            .engine
            .invoke_by_name(h.root, "terminalOf", &[Value::from(second.0 as i32)])
            .unwrap()
            .as_object()
            .unwrap();
        h.engine
            .invoke_by_name(t, "screenText", &[])
            .unwrap()
            .to_js_string()
            .as_str()
            .contains("zqxj")
    });

    hof.agent(move |h| h.focus_pane(first)).unwrap();
    assert_eq!(hof.first(), first);

    hof.agent(move |h| h.close_pane(second)).unwrap();
    hof.window.settle().unwrap();
    let left: Vec<_> = hof.agent(|h| h.list_panes()).iter().map(|p| p.id).collect();
    assert_eq!(left, vec![first]);
    assert_eq!(
        hof.agent(move |h| h.close_pane(second)),
        Err(HostError::NoSuchPane(second))
    );
}
