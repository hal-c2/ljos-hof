//! How a terminal tells the AI agent it runs, and how the agent is.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use hof_qml::agents::{self, Table};
use hof_qml::{Terminal, register};
use ljos::prelude::*;
use ljos::testing::TestWindow;

struct Shown {
    engine: Engine,
    window: TestWindow,
    t: ObjectId,
    changes: Rc<RefCell<Vec<(String, String)>>>,
}

/// A terminal fed by `write()` when `script` is `None`, else running it.
fn shown(script: Option<&str>) -> Shown {
    let engine = ljos::engine();
    register(&engine);
    let mut window = TestWindow::new(&engine, 400.0, 200.0);
    let t = engine.create_by_name("Hof", "Terminal", None).unwrap();
    match script {
        None => engine.set(t, Terminal::EXTERNAL, true).unwrap(),
        Some(script) => {
            engine
                .set(t, Terminal::PROGRAM, "/bin/sh".to_owned())
                .unwrap();
            let args = vec!["-c".to_owned(), script.to_owned()];
            engine.set(t, Terminal::ARGUMENTS, args).unwrap();
        }
    }
    let changes = Rc::new(RefCell::new(Vec::new()));
    let into = Rc::clone(&changes);
    engine
        .connect_by_name(t, "agentStatus", move |_, a| {
            let s = |v: &Value| v.to_js_string().as_str().to_owned();
            into.borrow_mut().push((s(&a[0]), s(&a[1])));
        })
        .unwrap();
    engine.set(t, Item::WIDTH, 400.0).unwrap();
    engine.set(t, Item::HEIGHT, 200.0).unwrap();
    window.show(t).unwrap();
    window.settle().unwrap();
    Shown {
        engine,
        window,
        t,
        changes,
    }
}

impl Shown {
    fn agent(&self) -> (String, String) {
        (
            self.engine.get(self.t, Terminal::AGENT).unwrap(),
            self.engine.get(self.t, Terminal::AGENT_STATE).unwrap(),
        )
    }

    fn write(&mut self, text: &str) {
        self.engine
            .invoke_by_name(self.t, "write", &[Value::from(text)])
            .unwrap();
        self.window.settle().unwrap();
    }

    fn send(&mut self, text: &str) {
        self.engine
            .invoke_by_name(self.t, "send", &[Value::from(text)])
            .unwrap();
        self.window.settle().unwrap();
    }

    /// Lets the program run, and time pass for the terminal to look,
    /// until its agent is `name` and `state`.
    fn until(&mut self, name: &str, state: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.agent() != (name.to_owned(), state.to_owned()) {
            assert!(
                Instant::now() < deadline,
                "waiting for {name} {state}: {:?}",
                self.agent()
            );
            std::thread::sleep(Duration::from_millis(5));
            self.window.advance(100).unwrap();
        }
    }
}

#[test]
fn an_agent_that_says_how_it_is_is_shown_so() {
    let mut s = shown(None);
    assert_eq!(s.agent(), (String::new(), "none".to_owned()));
    s.write("thinking \x1b]7701;working;claude\x07");
    assert_eq!(s.agent(), ("claude".to_owned(), "working".to_owned()));
    s.write("\x1b]7701;waiting\x1b\\");
    assert_eq!(s.agent(), ("agent".to_owned(), "waiting".to_owned()));
    s.write("\x1b]7701;none\x07");
    assert_eq!(
        *s.changes.borrow(),
        [
            ("claude".to_owned(), "working".to_owned()),
            ("agent".to_owned(), "waiting".to_owned()),
            (String::new(), "none".to_owned()),
        ]
    );
}

#[test]
fn what_qml_reports_counts_as_what_the_agent_says() {
    let mut s = shown(None);
    let ok = s
        .engine
        .invoke_by_name(
            s.t,
            "reportAgent",
            &[Value::from("idle"), Value::from("codex")],
        )
        .unwrap();
    assert!(ok.to_boolean());
    assert_eq!(s.agent(), ("codex".to_owned(), "idle".to_owned()));
    let bad = s
        .engine
        .invoke_by_name(s.t, "reportAgent", &[Value::from("busy"), Value::from("")])
        .unwrap();
    assert!(!bad.to_boolean());
    assert_eq!(s.agent(), ("codex".to_owned(), "idle".to_owned()));
    // A state no one knows, in the sequence, changes nothing either.
    s.write("\x1b]7701;dreaming\x07");
    assert_eq!(s.agent(), ("codex".to_owned(), "idle".to_owned()));
}

#[test]
fn a_program_with_a_table_is_told_by_its_screen_and_gone_when_it_ends() {
    // The tables are the process's; this is the only test here that needs
    // one, and the others' terminals run nothing a table names.
    agents::set_tables(vec![Table {
        name: "fake".into(),
        processes: vec!["sh".into()],
        waiting: vec!["Proceed?".into()],
        working: vec!["thinking".into()],
    }]);
    let mut s = shown(Some(
        "printf 'thinking\\n'; read x; printf 'Proceed?\\n'; read y",
    ));
    s.until("fake", "working");
    s.send("\r");
    s.until("fake", "waiting");
    // Once the question scrolls off the bottom rows, it asks no more.
    s.write(&"\r\n".repeat(20));
    s.until("fake", "idle");
    s.send("\r");
    s.until("", "none");
    agents::set_tables(Vec::new());
}
