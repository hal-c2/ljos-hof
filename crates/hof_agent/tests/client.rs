//! Hof's client against a real agent, over connections the test can cut.

use std::io;
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hof_agent::{Agent, Options};
use hof_remote::client::{self, Connection, Host, Screen, Start, State, Transport};

const PATIENCE: Duration = Duration::from_secs(10);

/// A transport to an agent in this process. Each connection can be cut
/// from the test, as a network would.
struct Pipe {
    agent: Agent,
    /// The ends of the connections made, for cutting.
    made: mpsc::Sender<UnixStream>,
    /// Fails this many connects first.
    refuse: Arc<Mutex<u32>>,
}

impl Transport for Pipe {
    fn connect(&mut self) -> io::Result<Connection> {
        {
            let mut refuse = self.refuse.lock().unwrap();
            if *refuse > 0 {
                *refuse -= 1;
                return Err(io::Error::other("no route to host"));
            }
        }
        let (ours, theirs) = UnixStream::pair()?;
        let agent = self.agent.clone();
        thread::spawn(move || {
            let reader = theirs.try_clone().unwrap();
            let _ = agent.serve(reader, theirs);
        });
        let _ = self.made.send(ours.try_clone()?);
        let cut = ours.try_clone()?;
        Ok(Connection {
            reader: Box::new(ours.try_clone()?),
            writer: Box::new(ours),
            end: Box::new(move || {
                let _ = cut.shutdown(std::net::Shutdown::Both);
                None
            }),
        })
    }
}

/// A screen that only collects.
#[derive(Default)]
struct Seen {
    inner: Mutex<SeenInner>,
    changed: std::sync::Condvar,
}

#[derive(Default)]
struct SeenInner {
    text: Vec<u8>,
    snapshots: usize,
    exited: Option<Option<i32>>,
    states: Vec<State>,
}

impl Screen for Seen {
    fn output(&self, bytes: &[u8]) {
        self.inner.lock().unwrap().text.extend_from_slice(bytes);
        self.changed.notify_all();
    }
    fn replace(&self, snapshot: &[u8]) {
        let mut seen = self.inner.lock().unwrap();
        seen.text = snapshot.to_vec();
        seen.snapshots += 1;
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
    fn wait(&self, what: &str, done: impl Fn(&SeenInner) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        let mut seen = self.inner.lock().unwrap();
        while !done(&seen) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "waiting for {what}: {:?}, {:?}",
                String::from_utf8_lossy(&seen.text),
                seen.states
            );
            seen = self.changed.wait_timeout(seen, left).unwrap().0;
        }
    }

    fn wait_for_text(&self, text: &str) {
        self.wait(text, |s| String::from_utf8_lossy(&s.text).contains(text));
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.inner.lock().unwrap().text).into_owned()
    }

    fn up(&self) -> Option<u32> {
        self.inner
            .lock()
            .unwrap()
            .states
            .iter()
            .rev()
            .find_map(|s| match s {
                State::Up { pane } => Some(*pane),
                _ => None,
            })
    }
}

fn fast() -> client::Options {
    client::Options {
        retry_first: Duration::from_millis(10),
        retry_longest: Duration::from_millis(50),
        ..client::Options::default()
    }
}

fn host(agent: &Agent) -> (Host, mpsc::Receiver<UnixStream>, Arc<Mutex<u32>>) {
    let (made, connections) = mpsc::channel();
    let refuse = Arc::new(Mutex::new(0));
    let pipe = Pipe {
        agent: agent.clone(),
        made,
        refuse: refuse.clone(),
    };
    (Host::new(pipe, fast()), connections, refuse)
}

fn start(script: &str) -> Start {
    Start {
        command: vec!["/bin/sh".into(), "-c".into(), script.into()],
        cols: 40,
        rows: 5,
        ..Start::default()
    }
}

fn agent() -> Agent {
    Agent::new(Options {
        name: "test".into(),
        ..Options::default()
    })
}

#[test]
fn a_pane_opens_on_the_host_and_its_program_runs_there() {
    let agent = agent();
    let (host, _, _) = host(&agent);
    let seen = Arc::new(Seen::default());
    let pane = host.open(
        start("echo ready; read x; echo got:$x; exit 4"),
        seen.clone(),
    );
    seen.wait_for_text("ready");
    assert!(seen.up().is_some());
    pane.input(b"hi\r");
    seen.wait_for_text("got:hi");
    seen.wait("its end", |s| s.exited.is_some());
    assert_eq!(seen.inner.lock().unwrap().exited, Some(Some(4)));
    pane.close();
    let deadline = Instant::now() + PATIENCE;
    while !agent.is_idle() {
        assert!(Instant::now() < deadline, "the agent still has the pane");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_paste_larger_than_a_message_reaches_the_program_whole() {
    let agent = agent();
    let (host, _, _) = host(&agent);
    let seen = Arc::new(Seen::default());
    let n = 300 * 1024;
    let pane = host.open(
        start(&format!(
            "stty raw -echo; echo ready; head -c {n} | wc -c | tr -d ' '; echo done"
        )),
        seen.clone(),
    );
    seen.wait_for_text("ready");
    pane.input(&vec![b'a'; n]);
    seen.wait_for_text("done");
    assert!(seen.text().contains(&n.to_string()), "{}", seen.text());
    pane.close();
}

#[test]
fn a_dropped_connection_comes_back_missing_nothing_and_typing_nothing_twice() {
    let agent = agent();
    let (host, connections, _) = host(&agent);
    let seen = Arc::new(Seen::default());
    let pane = host.open(
        start("echo ready; while read x; do echo got:$x; done"),
        seen.clone(),
    );
    seen.wait_for_text("ready");
    let first = connections.recv_timeout(PATIENCE).unwrap();

    // Cut, and typed into while it is down.
    first.shutdown(std::net::Shutdown::Both).unwrap();
    seen.wait("it to notice", |s| {
        s.states.iter().any(|s| matches!(s, State::Down(_)))
    });
    pane.input(b"one\r");
    pane.input(b"two\r");
    seen.wait_for_text("got:two");
    let text = seen.text();
    assert_eq!(text.matches("got:one").count(), 1, "{text:?}");
    assert_eq!(text.matches("ready").count(), 1, "{text:?}");
    assert!(text.find("got:one") < text.find("got:two"), "{text:?}");
    pane.close();
}

#[test]
fn a_host_that_cannot_be_reached_is_tried_again() {
    let agent = agent();
    let (host, _, refuse) = host(&agent);
    *refuse.lock().unwrap() = 3;
    let seen = Arc::new(Seen::default());
    let pane = host.open(start("echo ready; read x"), seen.clone());
    seen.wait_for_text("ready");
    let states = seen.inner.lock().unwrap().states.clone();
    assert!(
        states.contains(&State::Down("no route to host".into())),
        "{states:?}"
    );
    pane.close();
}

#[test]
fn a_detached_pane_runs_on_and_is_attached_again_by_number() {
    let agent = agent();
    let (host, _, _) = host(&agent);
    let seen = Arc::new(Seen::default());
    let pane = host.open(
        start("echo ready; read x; echo later:$x; read y"),
        seen.clone(),
    );
    seen.wait_for_text("ready");
    let number = seen.up().unwrap();
    pane.detach();

    // Another window, or Hof started again from its saved session.
    let (other, _, _) = self::host(&agent);
    let again = Arc::new(Seen::default());
    let pane = other.attach(number, 40, 5, again.clone());
    again.wait_for_text("ready");
    // Drawn from a snapshot of its screen, which stands for all it showed:
    // what the window has of it already, saved, is not shown twice.
    assert_eq!(again.inner.lock().unwrap().snapshots, 1);
    pane.input(b"x\r");
    again.wait_for_text("later:x");
    assert_eq!(again.text().matches("ready").count(), 1, "{}", again.text());
    pane.close();
}

#[test]
fn a_pane_taken_by_another_window_is_gone_from_the_first() {
    let agent = agent();
    let (host, _, _) = host(&agent);
    let seen = Arc::new(Seen::default());
    let _pane = host.open(start("echo ready; read x"), seen.clone());
    seen.wait_for_text("ready");
    let number = seen.up().unwrap();

    let (other, _, _) = self::host(&agent);
    let again = Arc::new(Seen::default());
    let taken = other.attach(number, 40, 5, again.clone());
    again.wait_for_text("ready");
    seen.wait("it to go", |s| {
        s.states
            .iter()
            .any(|s| *s == State::Gone("opened in another window".into()))
    });
    taken.close();
}

#[test]
fn a_program_that_cannot_start_ends_its_pane() {
    let agent = agent();
    let (host, _, _) = host(&agent);
    let seen = Arc::new(Seen::default());
    let _pane = host.open(
        Start {
            command: vec!["/nonexistent/program".into()],
            cols: 10,
            rows: 2,
            ..Start::default()
        },
        seen.clone(),
    );
    seen.wait("it to end", |s| {
        s.exited.is_some_and(|c| c != Some(0))
            || s.states.iter().any(|s| matches!(s, State::Gone(_)))
    });
}

#[test]
fn attaching_to_a_pane_the_agent_does_not_have_says_so() {
    let agent = agent();
    let (host, _, _) = host(&agent);
    let seen = Arc::new(Seen::default());
    let _pane = host.attach(7, 40, 5, seen.clone());
    seen.wait("it to be gone", |s| {
        s.states.iter().any(|s| matches!(s, State::Gone(_)))
    });
}

#[test]
fn a_spawned_transport_says_why_it_failed() {
    let host = Host::new(
        client::Spawned {
            argv: vec![
                "/bin/sh".into(),
                "-c".into(),
                "echo 'ssh: Could not resolve hostname nowhere' >&2; exit 255".into(),
            ],
        },
        fast(),
    );
    let seen = Arc::new(Seen::default());
    let pane = host.open(start("true"), seen.clone());
    seen.wait("the reason", |s| {
        s.states
            .iter()
            .any(|s| matches!(s, State::Down(why) if why.contains("Could not resolve hostname")))
    });
    pane.close();
}

#[test]
fn what_a_program_writes_while_the_connection_is_down_comes_once_it_is_back() {
    let agent = agent();
    let (host, connections, refuse) = host(&agent);
    let seen = Arc::new(Seen::default());
    let pane = host.open(
        start("echo ready; sleep 0.3; echo while-away; read x"),
        seen.clone(),
    );
    seen.wait_for_text("ready");
    *refuse.lock().unwrap() = u32::MAX;
    connections
        .recv_timeout(PATIENCE)
        .unwrap()
        .shutdown(std::net::Shutdown::Both)
        .unwrap();
    thread::sleep(Duration::from_millis(600));
    assert!(!seen.text().contains("while-away"), "it was down");
    *refuse.lock().unwrap() = 0;
    seen.wait_for_text("while-away");
    let text = seen.text();
    assert_eq!(text.matches("ready").count(), 1, "{text:?}");
    assert_eq!(seen.inner.lock().unwrap().snapshots, 0);
    pane.close();
}

#[test]
fn finishing_closes_the_panes_still_open() {
    let agent = agent();
    let (host, _, _) = host(&agent);
    let seen = Arc::new(Seen::default());
    let _pane = host.open(start("echo ready; read x"), seen.clone());
    seen.wait_for_text("ready");
    host.finish(false, PATIENCE);
    let deadline = Instant::now() + PATIENCE;
    while !agent.is_idle() {
        assert!(Instant::now() < deadline, "the agent still has the pane");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn finishing_to_keep_leaves_the_programs_running() {
    let agent = agent();
    let (host, _, _) = host(&agent);
    let seen = Arc::new(Seen::default());
    let _pane = host.open(start("echo ready; read x; echo later:$x"), seen.clone());
    seen.wait_for_text("ready");
    let number = seen.up().unwrap();
    host.finish(true, PATIENCE);

    let (other, _, _) = self::host(&agent);
    let again = Arc::new(Seen::default());
    let pane = other.attach(number, 40, 5, again.clone());
    again.wait_for_text("ready");
    pane.input(b"x\r");
    again.wait_for_text("later:x");
    pane.close();
}
