//! The agent on a socket, reached through the bridge as ssh would reach it.

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use hof_agent::{Agent, Options, daemon};
use hof_remote::wire::{Decoder, Message, VERSION};

const PATIENCE: Duration = Duration::from_secs(10);
/// Long enough for a probe to come and go, short enough to wait out.
const LINGER: Duration = Duration::from_millis(500);

/// The far end of a bridge: what ssh's stdio would be.
struct Ssh {
    stream: UnixStream,
    decoder: Decoder,
}

impl Ssh {
    fn send(&mut self, message: Message) {
        let mut frame = Vec::new();
        message.encode(&mut frame);
        self.stream.write_all(&frame).unwrap();
    }

    fn next(&mut self) -> Message {
        loop {
            if let Some(m) = self.decoder.next_message() {
                return m.unwrap();
            }
            let mut buf = [0; 4096];
            let n = self.stream.read(&mut buf).expect("the agent answers");
            assert!(n > 0, "the bridge hung up");
            self.decoder.push(&buf[..n]);
        }
    }

    fn hello(&mut self) -> String {
        self.send(Message::Hello {
            version: VERSION,
            name: "hof".into(),
            instance: 0,
        });
        match self.next() {
            Message::Hello { name, .. } => name,
            other => panic!("{other:?}"),
        }
    }
}

/// A bridge to `path` on a thread, starting an agent there with `start`.
fn bridge(path: &Path, start: impl FnOnce() + Send + 'static) -> Ssh {
    let (ours, theirs) = UnixStream::pair().unwrap();
    ours.set_read_timeout(Some(PATIENCE)).unwrap();
    let path = path.to_owned();
    thread::spawn(move || {
        let input = theirs.try_clone().unwrap();
        let _ = daemon::bridge(&path, input, theirs, || {
            start();
            Ok(())
        });
    });
    Ssh {
        stream: ours,
        decoder: Decoder::new(),
    }
}

/// Serves an agent named `name` at `path` on a thread; says when it stops.
fn serve(path: &Path, name: &str) -> mpsc::Receiver<std::io::Result<()>> {
    let (done, stopped) = mpsc::channel();
    let path: PathBuf = path.to_owned();
    let agent = Agent::new(Options {
        name: name.into(),
        ..Options::default()
    });
    thread::spawn(move || {
        let _ = done.send(daemon::serve(&agent, &path, LINGER));
    });
    stopped
}

fn wait_for_socket(path: &Path) {
    for _ in 0..250 {
        if UnixStream::connect(path).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("nothing listens at {}", path.display());
}

#[test]
fn the_bridge_starts_an_agent_when_none_answers_and_it_goes_when_unused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.sock");
    let (started, stopped) = mpsc::channel();
    let start_at = path.clone();
    let mut ssh = bridge(&path, move || {
        let _ = started.send(serve(&start_at, "started"));
    });
    assert_eq!(ssh.hello(), "started");
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "only its user reaches it");

    // ssh hangs up, and with no panes the agent has nothing to stay for.
    drop(ssh);
    let stopped = stopped.recv_timeout(PATIENCE).unwrap();
    stopped.recv_timeout(PATIENCE).unwrap().unwrap();
    assert!(!path.exists(), "it takes its socket with it");
}

#[test]
fn the_bridge_reaches_the_agent_that_runs_and_its_panes_outlive_the_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.sock");
    let stopped = serve(&path, "running");
    wait_for_socket(&path);

    let mut ssh = bridge(&path, || panic!("one runs already"));
    assert_eq!(ssh.hello(), "running");
    ssh.send(Message::Open {
        request: 1,
        command: vec!["/bin/sh".into(), "-c".into(), "read x".into()],
        directory: String::new(),
        environment: vec![],
        cols: 20,
        rows: 2,
    });
    let pane = match ssh.next() {
        Message::Opened { pane, .. } => pane,
        other => panic!("{other:?}"),
    };
    drop(ssh);
    // A pane is left: the agent stays.
    assert!(stopped.recv_timeout(Duration::from_millis(300)).is_err());

    let mut ssh = bridge(&path, || panic!("one runs still"));
    ssh.hello();
    ssh.send(Message::List);
    match ssh.next() {
        Message::Panes { panes } => assert_eq!(panes[0].pane, pane),
        other => panic!("{other:?}"),
    }
    ssh.send(Message::Close { pane });
    drop(ssh);
    stopped.recv_timeout(PATIENCE).unwrap().unwrap();
}

#[test]
fn a_second_agent_on_the_same_socket_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.sock");
    let _first = serve(&path, "first");
    wait_for_socket(&path);
    let second = serve(&path, "second");
    let error = second.recv_timeout(PATIENCE).unwrap().unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
    assert!(path.exists(), "the first keeps its socket");
}

#[test]
fn a_socket_left_by_an_agent_that_died_is_taken_over() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.sock");
    drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
    assert!(path.exists());
    let _agent = serve(&path, "new");
    wait_for_socket(&path);
    let mut ssh = bridge(&path, || panic!("one runs"));
    assert_eq!(ssh.hello(), "new");
}
