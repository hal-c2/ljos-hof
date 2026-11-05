//! `hof forget`: the programs a session kept running are ended though no
//! window shows them. One test: where panes run is the process's to say.

mod support;

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use hof_agent::{Agent, daemon};
use hof_qml::remote;
use support::{PATIENCE, shown};

#[test]
fn a_program_no_window_shows_is_ended_and_one_gone_is_not_counted() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("agent.sock");
    let agent = Agent::new(hof_agent::Options {
        name: "here".into(),
        ..hof_agent::Options::default()
    });
    {
        let (agent, socket) = (agent.clone(), socket.clone());
        thread::spawn(move || daemon::serve(&agent, &socket, Duration::from_secs(60)));
    }
    let here = |socket| remote::Here {
        socket,
        start: Arc::new(|| Err(std::io::ErrorKind::NotFound.into())),
    };
    remote::keep_here(Some(here(socket.clone())));

    let mut s = shown("", "echo ready; while read x; do :; done", -1);
    s.until_text("ready");
    let pane = u32::try_from(s.remote_pane()).expect("run by the agent");
    remote::keep_programs(true);
    remote::finish(Duration::from_secs(2));
    drop(s);
    remote::keep_programs(false);
    assert!(!agent.is_idle(), "it runs on, shown by none");

    // A new connection, as `hof forget` makes.
    remote::keep_here(None);
    remote::keep_here(Some(here(socket)));
    let gone = pane + 7;
    let ended = remote::end_unseen(&[(String::new(), pane), (String::new(), gone)], PATIENCE);
    assert_eq!(ended, 1, "the one it had, not the one it never had");
    let deadline = Instant::now() + PATIENCE;
    while !agent.is_idle() {
        assert!(Instant::now() < deadline, "the program was left running");
        thread::sleep(Duration::from_millis(10));
    }

    // No agent answers: nothing is ended, and it is said soon.
    remote::keep_here(None);
    remote::keep_here(Some(here(dir.path().join("none.sock"))));
    let asked = Instant::now();
    assert_eq!(remote::end_unseen(&[(String::new(), 5)], PATIENCE), 0);
    assert!(
        asked.elapsed() < PATIENCE,
        "it waited for an agent that is not there"
    );
}
