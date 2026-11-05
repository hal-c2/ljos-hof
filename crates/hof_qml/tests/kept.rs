//! A pane of this machine run by the agent here (`[session] keep-running`):
//! its program outlives the terminal, and Hof, and a terminal made later
//! attaches to it again. One test: where panes run is the process's to say.

mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use hof_agent::{Agent, daemon};
use hof_qml::{Terminal, remote};
use support::{PATIENCE, shown};

/// The agent here, on a socket of the test's, started the first time a
/// pane wants it: counted, to tell it is started once.
fn here(socket: PathBuf, agent: Agent, started: Arc<AtomicUsize>) -> remote::Here {
    let at = socket.clone();
    remote::Here {
        socket,
        start: Arc::new(move || {
            started.fetch_add(1, Ordering::SeqCst);
            let (agent, at) = (agent.clone(), at.clone());
            thread::spawn(move || daemon::serve(&agent, &at, Duration::from_secs(60)));
            Ok(())
        }),
    }
}

#[test]
fn a_program_here_outlives_hof_and_the_next_one_attaches_to_it() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("agent.sock");
    let agent = Agent::new(hof_agent::Options {
        name: "here".into(),
        ..hof_agent::Options::default()
    });
    let started = Arc::new(AtomicUsize::new(0));
    remote::keep_here(Some(here(socket.clone(), agent.clone(), started.clone())));

    // Where Hof is, not the agent's home: as for a program Hof starts.
    let cwd = std::env::current_dir().unwrap();
    let script = format!(
        "[ \"$PWD\" = '{}' ] && echo in-hofs-directory; echo ready; \
         while read x; do echo got:$x; done",
        cwd.display()
    );
    let mut s = shown("", &script, -1);
    s.until_text("ready");
    assert!(s.text().contains("in-hofs-directory"), "{:?}", s.text());
    assert_eq!(s.connection(), "up");
    let pane = s.remote_pane();
    assert!(pane >= 0, "run by the agent");
    s.type_text("one\r");
    s.until_text("got:one");

    // Hof quits with the session kept: the program runs on.
    remote::keep_programs(true);
    remote::finish(Duration::from_secs(2));
    drop(s);
    thread::sleep(Duration::from_millis(100));
    assert!(!agent.is_idle(), "the agent still has the pane");

    // The next Hof: a new connection to the same agent, attaching.
    remote::keep_here(None);
    remote::keep_here(Some(here(socket, agent.clone(), started.clone())));
    remote::keep_programs(false);
    let mut again = shown("", "echo not-run", pane);
    again.until_text("got:one");
    assert_eq!(again.remote_pane(), pane, "the same program");
    assert!(!again.text().contains("not-run"));
    again.type_text("two\r");
    again.until_text("got:two");
    assert!(again.engine.get(again.t, Terminal::RUNNING).unwrap());
    assert_eq!(started.load(Ordering::SeqCst), 1, "the agent started once");

    // Closed now, not kept: the agent has nothing left.
    drop(again);
    let deadline = Instant::now() + PATIENCE;
    while !agent.is_idle() {
        assert!(Instant::now() < deadline, "the agent kept the pane");
        thread::sleep(Duration::from_millis(10));
    }
}
