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

#[test]
fn what_an_agent_on_a_host_says_of_itself_is_shown_here() {
    let (_agent, _) = host("says");
    let mut s = shown(
        "says",
        "echo ready; printf '\\033]7701;waiting;codex\\007'; read x",
        -1,
    );
    s.until("its report", |s| {
        s.engine.get(s.t, Terminal::AGENT_STATE).unwrap() == "waiting"
    });
    assert_eq!(s.engine.get(s.t, Terminal::AGENT).unwrap(), "codex");
}

#[test]
fn an_agent_on_a_host_is_told_by_its_foreground_and_screen_there() {
    // A name no other test's program has: the tables are the process's.
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fake-agent");
    std::fs::write(&script, "printf 'Proceed?\\n'; read x\n").unwrap();
    hof_qml::agents::set_tables(vec![hof_qml::agents::Table {
        name: "fake".into(),
        processes: vec!["fake-agent".into()],
        waiting: vec!["Proceed?".into()],
        working: Vec::new(),
    }]);
    let (_agent, _) = host("tables");
    let mut s = shown("tables", &format!("exec sh '{}'", script.display()), -1);
    // Looks run on the engine's clock, which a test window moves itself.
    let deadline = Instant::now() + PATIENCE;
    while s.engine.get(s.t, Terminal::AGENT_STATE).unwrap() != "waiting" {
        assert!(Instant::now() < deadline, "waiting for it: {:?}", s.text());
        thread::sleep(Duration::from_millis(5));
        s.window.advance(100).unwrap();
    }
    assert_eq!(s.engine.get(s.t, Terminal::AGENT).unwrap(), "fake");
}

#[test]
fn a_program_on_a_host_reads_and_writes_the_clipboard_here() {
    use ljos::prelude::*;
    let (_agent, _) = host("clipboard");
    // Raw, so the answer, with no newline, reaches it at once; the ESCs
    // taken out, so what it read shows as text.
    let mut s = shown(
        "clipboard",
        "stty raw -echo; printf '\\033]52;c;?\\007'; head -c 13 | tr -d '\\033'; \
         printf ' \\033]52;c;bGrDs3M=\\007done'; read y",
        -1,
    );
    s.engine
        .set(s.t, Terminal::CLIPBOARD_READ, "allow".to_owned())
        .unwrap();
    Platform::of(&s.engine)
        .clipboard()
        .set_text(ClipboardMode::Clipboard, "hi")
        .unwrap();
    s.until_text("]52;c;aGk=\\ done");
    assert_eq!(
        Platform::of(&s.engine)
            .clipboard()
            .text(ClipboardMode::Clipboard)
            .as_deref(),
        Some("ljós")
    );
}

#[test]
fn a_remote_program_cannot_open_a_local_file() {
    use ljos::prelude::*;
    let (_agent, _) = host("opens");
    let mut s = shown("opens", "read x", -1);
    let urls = RecordedUrls::default();
    Platform::of(&s.engine).set_open_url(std::rc::Rc::new(urls.clone()));
    s.engine
        .set(s.t, Terminal::OPEN_URLS, "allow".to_owned())
        .unwrap();
    for url in [
        "/etc/passwd",
        "relative.desktop",
        "file:///etc/passwd",
        "FILE:///etc/passwd",
        "File:/etc/passwd",
    ] {
        let request = String::from_utf8(hof_vt::open_request(url)).unwrap();
        s.engine
            .invoke_by_name(s.t, "write", &[Value::from(request.as_str())])
            .unwrap();
    }
    s.window.settle().unwrap();
    assert!(urls.urls().is_empty(), "{:?}", urls.urls());

    let request = String::from_utf8(hof_vt::open_request("https://a.example")).unwrap();
    s.engine
        .invoke_by_name(s.t, "write", &[Value::from(request.as_str())])
        .unwrap();
    s.window.settle().unwrap();
    assert_eq!(urls.urls(), ["https://a.example"]);
}
