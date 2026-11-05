//! An agent becoming a newer program of itself with its panes' programs
//! running on: asked to (`hof agent --upgrade`), and when its program is
//! installed anew.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use hof_remote::client::{self, Host, Local, Screen, Start, State};

const PATIENCE: Duration = Duration::from_secs(20);
const PROGRAM: &str = env!("CARGO_BIN_EXE_hof-agent");

#[derive(Default)]
struct Seen {
    inner: Mutex<Inner>,
    changed: Condvar,
}

#[derive(Default)]
struct Inner {
    output: Vec<u8>,
    states: Vec<State>,
    exited: Option<Option<i32>>,
}

impl Screen for Seen {
    fn output(&self, bytes: &[u8]) {
        self.inner.lock().unwrap().output.extend_from_slice(bytes);
        self.changed.notify_all();
    }
    fn replace(&self, snapshot: &[u8]) {
        self.inner.lock().unwrap().output = snapshot.to_vec();
        self.changed.notify_all();
    }
    fn exited(&self, code: Option<i32>) {
        self.inner.lock().unwrap().exited = Some(code);
        self.changed.notify_all();
    }
    fn state(&self, state: State) {
        self.inner.lock().unwrap().states.push(state);
        self.changed.notify_all();
    }
}

impl Seen {
    fn wait(&self, what: &str, done: impl Fn(&Inner) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        let mut seen = self.inner.lock().unwrap();
        while !done(&seen) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "waiting for {what}: {:?}, {:?}",
                String::from_utf8_lossy(&seen.output),
                seen.states
            );
            seen = self.changed.wait_timeout(seen, left).unwrap().0;
        }
    }

    fn wait_for(&self, text: &str) {
        self.wait(text, |s| String::from_utf8_lossy(&s.output).contains(text));
    }

    fn downs(&self) -> usize {
        let seen = self.inner.lock().unwrap();
        seen.states
            .iter()
            .filter(|s| matches!(s, State::Down(_)))
            .count()
    }
}

/// An agent of `program` at `socket`, its log beside it; ended with the
/// test.
struct Running {
    child: Child,
    socket: PathBuf,
}

impl Running {
    fn start(program: &Path, socket: &Path) -> Self {
        let log = std::fs::File::create(hof_agent::daemon::log_path(socket)).unwrap();
        let child = Command::new(program)
            .args(["agent", "--socket"])
            .arg(socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + PATIENCE;
        while std::os::unix::net::UnixStream::connect(socket).is_err() {
            assert!(Instant::now() < deadline, "the agent did not listen");
            std::thread::sleep(Duration::from_millis(20));
        }
        Self {
            child,
            socket: socket.to_owned(),
        }
    }

    fn still_running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    fn log(&self) -> String {
        std::fs::read_to_string(hof_agent::daemon::log_path(&self.socket)).unwrap()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn host(socket: &Path) -> Host {
    Host::new(
        Local {
            socket: socket.to_owned(),
            start: Box::new(|| Err(std::io::Error::other("the test's agent is gone"))),
        },
        client::Options {
            retry_first: Duration::from_millis(50),
            retry_longest: Duration::from_millis(200),
            ..client::Options::default()
        },
    )
}

fn open(socket: &Path) -> (client::Pane, Arc<Seen>) {
    let seen = Arc::new(Seen::default());
    let pane = host(socket).open(
        Start {
            command: vec![
                "/bin/sh".into(),
                "-c".into(),
                "echo ready:$$; read x; echo later:$x; exit 7".into(),
            ],
            cols: 40,
            rows: 5,
            ..Start::default()
        },
        seen.clone(),
    );
    seen.wait_for("ready:");
    (pane, seen)
}

/// The rest of the pane's life, after the upgrade: the same program,
/// still reading what is typed, and its end told.
fn carries_on(pane: &client::Pane, seen: &Seen) {
    seen.wait("the connection again", |s| {
        matches!(s.states.last(), Some(State::Up { .. }))
    });
    seen.wait_for("ready:");
    pane.input(b"x\r");
    seen.wait_for("later:x");
    seen.wait("its end", |s| s.exited == Some(Some(7)));
}

#[test]
fn asked_to_the_agent_becomes_the_program_asking_and_its_panes_run_on() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("agent.sock");
    let mut agent = Running::start(Path::new(PROGRAM), &socket);
    let (pane, seen) = open(&socket);

    let upgraded = Command::new(PROGRAM)
        .args(["agent", "--upgrade", "--socket"])
        .arg(&socket)
        .output()
        .unwrap();
    assert!(
        upgraded.status.success(),
        "{}{}",
        String::from_utf8_lossy(&upgraded.stdout),
        String::from_utf8_lossy(&upgraded.stderr)
    );
    // The same process: its programs are still its children.
    assert!(agent.still_running());

    seen.wait("the connection to drop", |s| {
        s.states.iter().any(|s| matches!(s, State::Down(_)))
    });
    carries_on(&pane, &seen);
    assert!(agent.still_running());
    let log = agent.log();
    assert!(log.contains("asked for an upgrade"), "{log}");
    assert!(log.contains("upgraded, its panes taken on"), "{log}");
    pane.close();
}

#[test]
fn installed_anew_the_agent_becomes_the_new_program_at_the_next_connection() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let installed = bin.join("hof-agent");
    std::fs::copy(PROGRAM, &installed).unwrap();
    let socket = dir.path().join("agent.sock");
    let mut agent = Running::start(&installed, &socket);
    let (pane, seen) = open(&socket);
    let pid = agent.child.id();
    let exe = || std::fs::read_link(format!("/proc/{pid}/exe")).unwrap();
    assert_eq!(exe(), installed);

    // Installed as a package manager does: a new file in the old one's place.
    let new = bin.join("hof-agent.new");
    std::fs::copy(PROGRAM, &new).unwrap();
    std::fs::rename(&new, &installed).unwrap();
    assert!(exe().to_string_lossy().ends_with("(deleted)"));
    assert_eq!(seen.downs(), 0);

    // Anyone connecting sets it off.
    drop(std::os::unix::net::UnixStream::connect(&socket).unwrap());
    seen.wait("the connection to drop", |s| {
        s.states.iter().any(|s| matches!(s, State::Down(_)))
    });
    carries_on(&pane, &seen);
    assert!(agent.still_running());
    assert_eq!(exe(), installed);
    let log = agent.log();
    assert!(log.contains("was installed anew"), "{log}");
    assert!(log.contains("upgraded, its panes taken on"), "{log}");
    pane.close();
}

#[test]
fn an_agent_does_not_become_a_program_that_cannot_take_over() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("agent.sock");
    let _agent = Running::start(Path::new(PROGRAM), &socket);
    let (pane, seen) = open(&socket);

    let error = hof_agent::daemon::upgrade(&socket, Some(Path::new("/bin/true"))).unwrap_err();
    assert!(error.to_string().contains("cannot take over"), "{error}");

    // Nothing happened to it.
    assert_eq!(seen.downs(), 0);
    pane.input(b"x\r");
    seen.wait_for("later:x");
    pane.close();
}
