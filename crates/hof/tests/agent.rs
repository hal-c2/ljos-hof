//! `hof agent --stdio`, the program ssh runs, reached as Hof reaches a
//! host: through a command whose stdin and stdout speak the protocol.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use hof_remote::client::{self, Host, Screen, Spawned, Start, State};

const PATIENCE: Duration = Duration::from_secs(20);

#[derive(Default)]
struct Seen {
    inner: Mutex<(Vec<u8>, Vec<State>)>,
    changed: Condvar,
}

impl Screen for Seen {
    fn output(&self, bytes: &[u8]) {
        self.inner.lock().unwrap().0.extend_from_slice(bytes);
        self.changed.notify_all();
    }
    fn replace(&self, snapshot: &[u8]) {
        self.inner.lock().unwrap().0 = snapshot.to_vec();
        self.changed.notify_all();
    }
    fn exited(&self, _: Option<i32>) {}
    fn state(&self, state: State) {
        self.inner.lock().unwrap().1.push(state);
        self.changed.notify_all();
    }
}

impl Seen {
    fn wait_for(&self, text: &str) {
        let deadline = Instant::now() + PATIENCE;
        let mut seen = self.inner.lock().unwrap();
        while !String::from_utf8_lossy(&seen.0).contains(text) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "waiting for {text}: {:?}", seen.1);
            seen = self.changed.wait_timeout(seen, left).unwrap().0;
        }
    }
}

fn host(socket: &std::path::Path) -> Host {
    let argv = [
        env!("CARGO_BIN_EXE_hof"),
        "agent",
        "--stdio",
        "--socket",
        socket.to_str().unwrap(),
    ];
    Host::new(
        Spawned {
            argv: argv.map(String::from).into(),
        },
        client::Options::default(),
    )
}

#[test]
fn a_pane_runs_under_the_agent_hof_starts_and_outlives_its_window() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("agent.sock");
    let seen = Arc::new(Seen::default());
    let pane = host(&socket).open(
        Start {
            command: vec![
                "/bin/sh".into(),
                "-c".into(),
                "echo ready; read x; echo later:$x".into(),
            ],
            cols: 40,
            rows: 5,
            ..Start::default()
        },
        seen.clone(),
    );
    seen.wait_for("ready");
    let number = seen
        .inner
        .lock()
        .unwrap()
        .1
        .iter()
        .find_map(|s| match s {
            State::Up { pane } => Some(*pane),
            _ => None,
        })
        .unwrap();
    // The window goes, keeping the program; another Hof picks it up.
    pane.detach();

    let again = Arc::new(Seen::default());
    let pane = host(&socket).attach(number, 40, 5, again.clone());
    again.wait_for("ready");
    pane.input(b"x\r");
    again.wait_for("later:x");
    pane.close();

    // With nothing left, the agent goes, and takes its socket with it.
    let deadline = Instant::now() + PATIENCE;
    while socket.exists() {
        assert!(Instant::now() < deadline, "the agent stayed");
        std::thread::sleep(Duration::from_millis(100));
    }

    // It said why its program ended, and why it did, where it is looked for
    // when a session comes back without them.
    let log = std::fs::read_to_string(hof_agent::daemon::log_path(&socket)).unwrap();
    assert!(log.contains("closed by connection"), "{log:?}");
    assert!(log.contains("no panes nor connections"), "{log:?}");
}
