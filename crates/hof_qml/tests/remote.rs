//! A `Terminal` whose program runs under `hof agent`, here an agent in
//! this process reached over socket pairs the test can cut.

mod support;

use std::thread;
use std::time::{Duration, Instant};

use hof_qml::Terminal;
use support::{PATIENCE, host, shown};

#[test]
fn a_program_on_a_host_shows_here_and_takes_what_is_typed() {
    let (agent, _) = host("typed");
    let mut s = shown("typed", "echo ready; read x; echo got:$x; exit 5", -1);
    s.until_text("ready");
    assert_eq!(s.connection(), "up");
    assert!(s.remote_pane() >= 0);
    assert!(s.engine.get(s.t, Terminal::RUNNING).unwrap());
    s.type_text("hi\r");
    s.until_text("got:hi");
    s.until("its end", |s| {
        !s.engine.get(s.t, Terminal::RUNNING).unwrap()
    });
    assert_eq!(s.engine.get(s.t, Terminal::EXIT_CODE).unwrap(), 5);
    assert_eq!(s.remote_pane(), -1, "its pane there is gone");
    drop(s);
    let deadline = Instant::now() + PATIENCE;
    while !agent.is_idle() {
        assert!(Instant::now() < deadline, "the agent kept the pane");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_dropped_connection_shows_as_down_and_the_screen_stays() {
    let (_agent, made) = host("dropped");
    let mut s = shown(
        "dropped",
        "echo ready; while read x; do echo got:$x; done",
        -1,
    );
    s.until_text("ready");
    for c in made.lock().unwrap().drain(..) {
        let _ = c.shutdown(std::net::Shutdown::Both);
    }
    s.until("it to notice", |s| s.connection() != "up");
    assert!(s.text().contains("ready"), "the screen is kept");
    s.type_text("again\r");
    s.until_text("got:again");
    assert_eq!(s.connection(), "up");
    assert_eq!(s.text().matches("ready").count(), 1);
}

#[test]
fn a_restored_pane_its_host_forgot_starts_its_program_again() {
    let (_agent, _) = host("forgot");
    let mut s = shown("forgot", "echo fresh; read x", 4);
    s.until_text("fresh");
    assert_eq!(s.connection(), "up");
    assert_ne!(s.remote_pane(), 4);
}

#[test]
fn a_host_no_one_named_is_said() {
    let s = shown("nowhere", "true", -1);
    let error = s.engine.get(s.t, Terminal::ERROR).unwrap();
    assert!(error.contains("nowhere"), "{error:?}");
}
