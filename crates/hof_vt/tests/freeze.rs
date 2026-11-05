//! Freezing the readers: what a program writes meanwhile waits, unread,
//! for whoever reads its pty next. On its own, as a freeze stops every
//! session in the process.

use std::time::{Duration, Instant};

use hof_vt::{Options, Session, Size, Spawn};

fn wait_for(session: &Session, rx: &std::sync::mpsc::Receiver<()>, text: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !session.text().contains(text) {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "waiting for {text}:\n{}", session.text());
        let _ = rx.recv_timeout(left);
        session.take_changes();
    }
}

#[test]
fn output_written_while_frozen_is_read_once_thawed() {
    let (tx, rx) = std::sync::mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let spawn = Spawn::command("/bin/sh", ["-c", "echo ready; read x; echo got:$x; read y"]);
    let session = Session::spawn(
        &spawn,
        Options {
            size: Size::new(30, 5),
            ..Options::default()
        },
        move || {
            let _ = tx.lock().unwrap().send(());
        },
    )
    .unwrap();
    wait_for(&session, &rx, "ready");

    let frozen = hof_vt::freeze(Duration::from_secs(5)).unwrap();
    assert!(
        hof_vt::freeze(Duration::from_secs(1)).is_err(),
        "one freeze at a time"
    );
    session.write(b"a\r");
    std::thread::sleep(Duration::from_millis(300));
    assert!(!session.text().contains("got:a"), "{}", session.text());

    drop(frozen);
    wait_for(&session, &rx, "got:a");
}
