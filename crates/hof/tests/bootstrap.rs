//! A machine with no Hof, reached over ssh: Hof copies its agent there and
//! runs a pane on it. Needs a machine to reach, so it is ignored unless
//! asked for; `mise run remote-test` starts one in a container and runs it.
//!
//! `HOF_TEST_SSH` is the ssh command that reaches it, without the command
//! to run there, its words split on spaces. `HOF_AGENT_DIR` is where the
//! agent to copy is, as `mise run agents` builds it.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use hof_remote::bootstrap::{self, Bootstrapping, Ssh};
use hof_remote::client::{self, Host, Screen, Start, State};

/// The copy is a few megabytes, and the first connection fails before it.
const PATIENCE: Duration = Duration::from_secs(90);

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
    fn wait_for(&self, text: &str) -> String {
        let deadline = Instant::now() + PATIENCE;
        let mut seen = self.inner.lock().unwrap();
        loop {
            let shown = String::from_utf8_lossy(&seen.0).into_owned();
            if shown.contains(text) {
                return shown;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "waiting for {text}: {:?}", seen.1);
            seen = self.changed.wait_timeout(seen, left).unwrap().0;
        }
    }

    fn states(&self) -> Vec<State> {
        self.inner.lock().unwrap().1.clone()
    }
}

fn ssh() -> Ssh {
    let argv = std::env::var("HOF_TEST_SSH").expect("HOF_TEST_SSH: the ssh command");
    Ssh {
        argv: argv
            .split(' ')
            .filter(|w| !w.is_empty())
            .map(String::from)
            .collect(),
        hof: "hof".into(),
    }
}

fn host() -> Host {
    let options = client::Options {
        retry_longest: Duration::from_secs(2),
        ..client::Options::default()
    };
    Host::new(Bootstrapping::new(ssh(), bootstrap::agent_dirs()), options)
}

#[test]
#[ignore = "needs a machine without Hof to reach over ssh: mise run remote-test"]
fn a_machine_without_hof_gets_the_agent_and_runs_the_pane() {
    let seen = Arc::new(Seen::default());
    let pane = host().open(
        Start {
            command: vec![
                "/bin/sh".into(),
                "-c".into(),
                "echo found:$(command -v hof); hof --version; read x; echo got:$x".into(),
            ],
            cols: 80,
            rows: 10,
            ..Start::default()
        },
        seen.clone(),
    );
    let shown = seen.wait_for("hof 0.");
    let dir = format!("/hof/{}/hof", bootstrap::version_dir());
    assert!(shown.contains(&dir), "the pane finds the copy: {shown}");
    let states = seen.states();
    assert!(
        states.iter().any(
            |s| matches!(s, State::Down(why) if why.starts_with("Hof is not on this machine"))
        ),
        "the first connection found no Hof: {states:?}"
    );
    pane.input(b"there\r");
    seen.wait_for("got:there");
    pane.close();

    // The next Hof finds the copy and has nothing to copy. Its pane is a
    // login shell, whose profile may set a `PATH` of its own; where that
    // profile puts `~/.local/bin` on it, the link there finds the copy.
    let again = Arc::new(Seen::default());
    let pane = host().open(
        Start {
            command: vec![
                "/bin/sh".into(),
                "-lc".into(),
                "grep -qs .local/bin ~/.profile && echo profile:local; \
                 echo login:$(command -v hof); echo again; read x"
                    .into(),
            ],
            cols: 80,
            rows: 10,
            ..Start::default()
        },
        again.clone(),
    );
    let shown = again.wait_for("again");
    if shown.contains("profile:local") {
        assert!(shown.contains("login:/home/"), "{shown}");
    }
    let states = again.states();
    assert!(
        !states.iter().any(|s| matches!(s, State::Down(_))),
        "{states:?}"
    );
    pane.close();
}
