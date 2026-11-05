//! A session whose program runs under an agent elsewhere, as the
//! connection writes into it.

use std::sync::{Arc, Mutex};

use hof_vt::{Feed, Options, Outlet, Session, Size};

struct Nowhere;

impl Outlet for Nowhere {
    fn input(&self, _: &[u8]) {}
    fn resize(&self, _: Size) {}
    fn closed(&self) {}
}

fn remote(cols: u16, rows: u16) -> (Session, Feed) {
    let fed = Arc::new(Mutex::new(None));
    let keep = fed.clone();
    let options = Options {
        size: Size::new(cols, rows),
        ..Options::default()
    };
    let session = Session::remote(
        options,
        || {},
        move |feed| {
            *keep.lock().unwrap() = Some(feed);
            Arc::new(Nowhere)
        },
    )
    .unwrap();
    let feed = fed.lock().unwrap().take().unwrap();
    (session, feed)
}

fn lines(prefix: &str, n: usize) -> String {
    (1..=n).map(|i| format!("{prefix}-{i}\r\n")).collect()
}

#[test]
fn a_snapshot_stands_for_everything_before_it_scrollback_too() {
    // The pane as a saved session left it, its scrollback and screen, and
    // the agent's snapshot of the program, which has the same history:
    // shown once, the snapshot's, and nothing of before.
    let (session, feed) = remote(20, 4);
    session.feed(lines("saved", 12).as_bytes());
    let (agent, _) = remote(20, 4);
    agent.feed(lines("there", 12).as_bytes());
    feed.replace(&agent.snapshot().unwrap());

    let text = session.text();
    assert!(!text.contains("saved"), "what was before goes: {text:?}");
    for i in 1..=12 {
        let line = format!("there-{i}");
        let times = text.lines().filter(|l| *l == line).count();
        assert_eq!(times, 1, "{line:?} once in {text:?}");
    }
}
