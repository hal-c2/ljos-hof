//! Closing a session ends its program however Hof was started. A test
//! binary of its own, because it ignores SIGHUP for the whole process, as
//! `nohup`, a service manager or an agent harness may for Hof's.

use std::sync::Mutex;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use hof_vt::{Options, Session, Spawn};

#[test]
fn closing_ends_the_program_when_hof_was_started_ignoring_hangups() {
    // SAFETY: setting a disposition to SIG_IGN, before any session's thread.
    let before = unsafe { libc::signal(libc::SIGHUP, libc::SIG_IGN) };
    assert_ne!(before, libc::SIG_ERR);

    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let spawn = Spawn::command("/bin/sh", ["-c", "echo ready; read x"]);
    let session = Session::spawn(&spawn, Options::default(), move || {
        let _ = tx.lock().unwrap().send(());
    })
    .unwrap();
    let pid = session.pid().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !session.text().contains("ready") {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "never ready; screen:\n{}", session.text());
        let _ = rx.recv_timeout(left);
        session.take_changes();
    }

    drop(session);
    let pid = libc::pid_t::try_from(pid).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    // Gone once the session's reaper has waited for it: kill(pid, 0) then
    // fails with ESRCH.
    // SAFETY: signal 0 only asks whether the process exists.
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(
            Instant::now() < deadline,
            "the program outlived its session: pid {pid}, {}",
            std::fs::read_to_string(format!("/proc/{pid}/status"))
                .ok()
                .and_then(|s| s
                    .lines()
                    .filter(|l| l.starts_with("State") || l.starts_with("SigIgn"))
                    .map(str::to_owned)
                    .reduce(|a, b| format!("{a}, {b}")))
                .unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
